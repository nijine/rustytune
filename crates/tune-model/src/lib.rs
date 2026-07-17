//! In-memory tune state.
//!
//! Pages are raw byte buffers sized from the INI; typed access to constants,
//! tables, and curves decodes through the ts-ini definition model. Each page
//! keeps three copies — local edits (`data`), the ECU-RAM shadow (`ecu`),
//! and the burned/EEPROM shadow (`burned`) — whose diffs drive the
//! unsent-write queue and the Burn indicator.
//!
//! Constant scale semantics (distinct from OutputChannels!):
//! `userValue = (raw + translate) * scale`, so `raw = user/scale - translate`.
//! Scales and limits may be `{expr}`s over other constants (e.g.
//! `fuelLoadBins` scale is `{fuelLoadRes}`), so the tune itself is the
//! expression symbol source.

pub mod msq;

use std::collections::HashMap;
use std::sync::Arc;

use ts_ini::{
    ConstantClass, ConstantDef, DataType, IniDef, OutputChannel, Shape, SymbolSource, TableDef,
    Value,
};

pub use msq::{ApplyReport, DiffEntry, DiffKind, MsqDiff, MsqFile, MsqValue};

#[derive(Debug, thiserror::Error)]
pub enum TuneError {
    #[error("unknown constant `{0}`")]
    UnknownConstant(String),
    #[error("unknown table `{0}`")]
    UnknownTable(String),
    #[error("`{0}` is not editable ({1})")]
    NotEditable(String, &'static str),
    #[error("index {index} out of range for `{name}` ({len} elements)")]
    IndexRange {
        name: String,
        index: usize,
        len: usize,
    },
    #[error("evaluating `{0}`: {1}")]
    Eval(String, String),
}

/// One tune page: desired bytes, what the ECU RAM has, what EEPROM has.
#[derive(Debug, Clone)]
pub struct PageState {
    pub data: Vec<u8>,
    pub ecu: Vec<u8>,
    pub burned: Vec<u8>,
}

/// A decoded table grid. `z[row][col]` with `row` indexing `y` (row 0 =
/// y[0], the first bin in memory) and `col` indexing `x`.
#[derive(Debug, Clone)]
pub struct TableData {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub z: Vec<Vec<f64>>,
    pub z_lo: f64,
    pub z_hi: f64,
    pub z_digits: u8,
}

pub struct Tune {
    def: Arc<IniDef>,
    pages: Vec<PageState>,
    /// PcVariable values (TS-side, not on any page), seeded from
    /// [DefaultValues].
    pc_values: HashMap<String, f64>,
    loaded: bool,
}

fn read_raw(bytes: &[u8], offset: usize, ty: DataType) -> Option<i64> {
    let size = ty.size() as usize;
    let b = bytes.get(offset..offset + size)?;
    let unsigned = match size {
        1 => b[0] as u64,
        2 => u16::from_le_bytes(b.try_into().unwrap()) as u64,
        4 => u32::from_le_bytes(b.try_into().unwrap()) as u64,
        _ => return None,
    };
    Some(if ty.signed() {
        match size {
            1 => unsigned as u8 as i8 as i64,
            2 => unsigned as u16 as i16 as i64,
            _ => unsigned as u32 as i32 as i64,
        }
    } else {
        unsigned as i64
    })
}

fn write_raw(bytes: &mut [u8], offset: usize, ty: DataType, raw: i64) -> bool {
    let size = ty.size() as usize;
    let Some(dst) = bytes.get_mut(offset..offset + size) else {
        return false;
    };
    match size {
        1 => dst[0] = raw as u8,
        2 => dst.copy_from_slice(&(raw as u16).to_le_bytes()),
        _ => dst.copy_from_slice(&(raw as u32).to_le_bytes()),
    }
    true
}

fn type_range(ty: DataType) -> (i64, i64) {
    match ty {
        DataType::U08 | DataType::Ascii => (0, u8::MAX as i64),
        DataType::S08 => (i8::MIN as i64, i8::MAX as i64),
        DataType::U16 => (0, u16::MAX as i64),
        DataType::S16 => (i16::MIN as i64, i16::MAX as i64),
        DataType::U32 => (0, u32::MAX as i64),
        DataType::S32 => (i32::MIN as i64, i32::MAX as i64),
    }
}

impl Tune {
    pub fn new(def: Arc<IniDef>) -> Self {
        let pages = def
            .header
            .page_sizes
            .iter()
            .map(|&size| PageState {
                data: vec![0; size as usize],
                ecu: vec![0; size as usize],
                burned: vec![0; size as usize],
            })
            .collect();
        let pc_values = def
            .default_values
            .iter()
            .filter_map(|(name, text)| Some((name.clone(), text.trim().parse::<f64>().ok()?)))
            .collect();
        Tune {
            def,
            pages,
            pc_values,
            loaded: false,
        }
    }

    pub fn def(&self) -> &IniDef {
        &self.def
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn set_loaded(&mut self, loaded: bool) {
        self.loaded = loaded;
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page(&self, page_idx: usize) -> Option<&PageState> {
        self.pages.get(page_idx)
    }

    /// Install a page downloaded from the ECU: all three copies agree.
    pub fn load_page(&mut self, page_idx: usize, bytes: &[u8]) {
        if let Some(page) = self.pages.get_mut(page_idx) {
            page.data = bytes.to_vec();
            page.ecu = bytes.to_vec();
            page.burned = bytes.to_vec();
        }
    }

    /// The ECU answered a `d` CRC that doesn't match what we think it has;
    /// resync our shadow (pending edits stay pending).
    pub fn resync_ecu(&mut self, page_idx: usize, bytes: &[u8]) {
        if let Some(page) = self.pages.get_mut(page_idx) {
            page.ecu = bytes.to_vec();
        }
    }

    fn constant(&self, name: &str) -> Option<&ConstantDef> {
        self.def.constants.get(name)
    }

    /// (page index, absolute offset) for a page constant.
    fn location(&self, c: &ConstantDef) -> Option<(usize, usize)> {
        let page = c.page? as usize;
        let offset = c.offset? as usize;
        Some((page.checked_sub(1)?, offset))
    }

    /// Decoded user value of a scalar/bits constant (bits: the raw field
    /// value, i.e. the combo index; string: the trimmed text).
    pub fn constant_value(&self, name: &str) -> Option<Value> {
        let c = self.constant(name)?;
        let (page_idx, offset) = self.location(c)?;
        let bytes = &self.pages.get(page_idx)?.data;
        match c.class {
            ConstantClass::Scalar => {
                let raw = read_raw(bytes, offset, c.ty)? as f64;
                let scale = c.scale.eval(self).ok()?;
                let translate = c.translate.eval(self).ok()?;
                Some(Value::Num((raw + translate) * scale))
            }
            ConstantClass::Bits => {
                let raw = read_raw(bytes, offset, c.ty)? as u64;
                let Some(Shape::Bits { lo, hi }) = c.shape else {
                    return None;
                };
                let mask = (1u64 << (hi - lo + 1)) - 1;
                Some(Value::Num(((raw >> lo) & mask) as f64))
            }
            ConstantClass::String => {
                let len = c.string_len? as usize;
                let text = bytes.get(offset..offset + len)?;
                Some(Value::Str(
                    String::from_utf8_lossy(text)
                        .trim_end_matches(['\0', ' '])
                        .to_string(),
                ))
            }
            ConstantClass::Array => None, // use array_element / table views
        }
    }

    /// One element of an array constant, decoded to its user value.
    pub fn array_element(&self, name: &str, index: usize) -> Option<f64> {
        let c = self.constant(name)?;
        if c.class != ConstantClass::Array {
            return None;
        }
        let count = c.shape?.element_count() as usize;
        if index >= count {
            return None;
        }
        let (page_idx, offset) = self.location(c)?;
        let bytes = &self.pages.get(page_idx)?.data;
        let raw = read_raw(bytes, offset + index * c.ty.size() as usize, c.ty)? as f64;
        let scale = c.scale.eval(self).ok()?;
        let translate = c.translate.eval(self).ok()?;
        Some((raw + translate) * scale)
    }

    pub fn array_values(&self, name: &str) -> Option<Vec<f64>> {
        let count = self.constant(name)?.shape?.element_count() as usize;
        (0..count).map(|i| self.array_element(name, i)).collect()
    }

    /// `clamp` applies the INI's lo/hi (interactive edits). The msq
    /// apply/diff paths pass `false`: a saved tune must restore and
    /// compare byte-faithfully even where values sit outside those UI
    /// limits.
    fn encode_impl(&self, c: &ConstantDef, user: f64, clamp: bool) -> Result<i64, TuneError> {
        let eval = |field: &ts_ini::NumOrExpr, what: &str| {
            field
                .eval(self)
                .map_err(|e| TuneError::Eval(format!("{} {what}", c.name), e.to_string()))
        };
        let scale = eval(&c.scale, "scale")?;
        let translate = eval(&c.translate, "translate")?;
        if scale == 0.0 {
            return Err(TuneError::Eval(c.name.clone(), "scale is zero".into()));
        }
        let mut user = user;
        if clamp {
            if let Some(lo) = &c.lo
                && let Ok(lo) = eval(lo, "lo")
            {
                user = user.max(lo);
            }
            if let Some(hi) = &c.hi
                && let Ok(hi) = eval(hi, "hi")
            {
                user = user.min(hi);
            }
        }
        let raw = (user / scale - translate).round() as i64;
        let (lo, hi) = type_range(c.ty);
        Ok(raw.clamp(lo, hi))
    }

    /// Write a scalar or bits constant into the local page data.
    pub fn set_constant(&mut self, name: &str, user: f64) -> Result<(), TuneError> {
        self.set_constant_impl(name, user, true)
    }

    pub(crate) fn set_constant_unclamped(
        &mut self,
        name: &str,
        user: f64,
    ) -> Result<(), TuneError> {
        self.set_constant_impl(name, user, false)
    }

    fn set_constant_impl(&mut self, name: &str, user: f64, clamp: bool) -> Result<(), TuneError> {
        let c = self
            .constant(name)
            .ok_or_else(|| TuneError::UnknownConstant(name.into()))?
            .clone();
        let (page_idx, offset) = self
            .location(&c)
            .ok_or_else(|| TuneError::NotEditable(name.into(), "not a page constant"))?;
        match c.class {
            ConstantClass::Scalar => {
                let raw = self.encode_impl(&c, user, clamp)?;
                let page = &mut self.pages[page_idx];
                write_raw(&mut page.data, offset, c.ty, raw);
                Ok(())
            }
            ConstantClass::Bits => {
                let Some(Shape::Bits { lo, hi }) = c.shape else {
                    return Err(TuneError::NotEditable(name.into(), "malformed bits shape"));
                };
                let page = &mut self.pages[page_idx];
                let current = read_raw(&page.data, offset, c.ty)
                    .ok_or_else(|| TuneError::NotEditable(name.into(), "out of page"))?
                    as u64;
                let width = hi - lo + 1;
                let mask = ((1u64 << width) - 1) << lo;
                let value = ((user.round().max(0.0) as u64) << lo) & mask;
                let raw = (current & !mask) | value;
                write_raw(&mut page.data, offset, c.ty, raw as i64);
                Ok(())
            }
            _ => Err(TuneError::NotEditable(
                name.into(),
                "use set_array_element for arrays",
            )),
        }
    }

    /// Write one element of an array constant.
    pub fn set_array_element(
        &mut self,
        name: &str,
        index: usize,
        user: f64,
    ) -> Result<(), TuneError> {
        self.set_array_element_impl(name, index, user, true)
    }

    pub(crate) fn set_array_element_unclamped(
        &mut self,
        name: &str,
        index: usize,
        user: f64,
    ) -> Result<(), TuneError> {
        self.set_array_element_impl(name, index, user, false)
    }

    fn set_array_element_impl(
        &mut self,
        name: &str,
        index: usize,
        user: f64,
        clamp: bool,
    ) -> Result<(), TuneError> {
        let c = self
            .constant(name)
            .ok_or_else(|| TuneError::UnknownConstant(name.into()))?
            .clone();
        if c.class != ConstantClass::Array {
            return Err(TuneError::NotEditable(name.into(), "not an array"));
        }
        let len = c.shape.map_or(0, |s| s.element_count()) as usize;
        if index >= len {
            return Err(TuneError::IndexRange {
                name: name.into(),
                index,
                len,
            });
        }
        let (page_idx, offset) = self
            .location(&c)
            .ok_or_else(|| TuneError::NotEditable(name.into(), "not a page constant"))?;
        let raw = self.encode_impl(&c, user, clamp)?;
        let page = &mut self.pages[page_idx];
        write_raw(
            &mut page.data,
            offset + index * c.ty.size() as usize,
            c.ty,
            raw,
        );
        Ok(())
    }

    pub fn requires_power_cycle(&self, name: &str) -> bool {
        self.def.requires_power_cycle.iter().any(|n| n == name)
    }

    // ----- raw access for the msq diff (same crate only) ------------------

    /// Raw field value of a scalar/bits constant (bits: the extracted
    /// index), for exact comparisons.
    pub(crate) fn constant_raw(&self, name: &str) -> Option<i64> {
        let c = self.constant(name)?;
        let (page_idx, offset) = self.location(c)?;
        let bytes = &self.pages.get(page_idx)?.data;
        let raw = read_raw(bytes, offset, c.ty)?;
        match c.class {
            ConstantClass::Bits => {
                let Some(Shape::Bits { lo, hi }) = c.shape else {
                    return None;
                };
                let mask = (1i64 << (hi - lo + 1)) - 1;
                Some((raw >> lo) & mask)
            }
            _ => Some(raw),
        }
    }

    pub(crate) fn array_raw(&self, name: &str, index: usize) -> Option<i64> {
        let c = self.constant(name)?;
        let (page_idx, offset) = self.location(c)?;
        let bytes = &self.pages.get(page_idx)?.data;
        read_raw(bytes, offset + index * c.ty.size() as usize, c.ty)
    }

    /// Encode a user value for byte-exact msq comparison (no lo/hi clamp).
    pub(crate) fn encode_user(&self, name: &str, user: f64) -> Result<i64, TuneError> {
        let c = self
            .constant(name)
            .ok_or_else(|| TuneError::UnknownConstant(name.into()))?;
        self.encode_impl(c, user, false)
    }

    // ----- tables ---------------------------------------------------------

    fn table_def(&self, id: &str) -> Option<&TableDef> {
        self.def.tables.get(id)
    }

    /// Grid dimensions of a table's zBins: (columns, rows).
    fn table_dims(&self, table: &TableDef) -> Option<(usize, usize)> {
        match self.constant(&table.z_bins)?.shape? {
            Shape::Array2D { x, y } => Some((x as usize, y as usize)),
            Shape::Array1D(n) => Some((n as usize, 1)),
            Shape::Bits { .. } => None,
        }
    }

    pub fn table(&self, id: &str) -> Option<TableData> {
        let table = self.table_def(id)?;
        let (nx, ny) = self.table_dims(table)?;
        let x = self.array_values(&table.x_bins.0)?;
        let y = self.array_values(&table.y_bins.0)?;
        let flat = self.array_values(&table.z_bins)?;
        let z: Vec<Vec<f64>> = (0..ny)
            .map(|row| flat[row * nx..(row + 1) * nx].to_vec())
            .collect();

        let z_def = self.constant(&table.z_bins)?;
        let z_lo = z_def
            .lo
            .as_ref()
            .and_then(|v| v.eval(self).ok())
            .unwrap_or(0.0);
        let z_hi = z_def
            .hi
            .as_ref()
            .and_then(|v| v.eval(self).ok())
            .unwrap_or(255.0);
        let z_digits = z_def
            .digits
            .as_ref()
            .and_then(|d| d.eval(self).ok())
            .unwrap_or(0.0) as u8;
        Some(TableData {
            x,
            y,
            z,
            z_lo,
            z_hi,
            z_digits,
        })
    }

    /// Set one z cell (row indexes y, col indexes x).
    pub fn set_table_cell(
        &mut self,
        id: &str,
        row: usize,
        col: usize,
        value: f64,
    ) -> Result<(), TuneError> {
        let table = self
            .table_def(id)
            .ok_or_else(|| TuneError::UnknownTable(id.into()))?;
        let z_bins = table.z_bins.clone();
        let (nx, ny) = self
            .table_dims(table)
            .ok_or_else(|| TuneError::UnknownTable(id.into()))?;
        if row >= ny || col >= nx {
            return Err(TuneError::IndexRange {
                name: z_bins,
                index: row * nx + col,
                len: nx * ny,
            });
        }
        self.set_array_element(&z_bins, row * nx + col, value)
    }

    // ----- dirty tracking -------------------------------------------------

    pub fn page_dirty(&self, page_idx: usize) -> bool {
        self.pages.get(page_idx).is_some_and(|p| p.data != p.ecu)
    }

    pub fn any_dirty(&self) -> bool {
        (0..self.pages.len()).any(|i| self.page_dirty(i))
    }

    pub fn page_burn_pending(&self, page_idx: usize) -> bool {
        self.pages.get(page_idx).is_some_and(|p| p.ecu != p.burned)
    }

    pub fn burn_pending(&self) -> bool {
        (0..self.pages.len()).any(|i| self.page_burn_pending(i))
    }

    /// Byte spans where local data differs from the ECU shadow, with gaps
    /// up to `max_gap` merged so nearby edits ride one `M` command.
    pub fn dirty_spans(&self, page_idx: usize, max_gap: usize) -> Vec<(usize, Vec<u8>)> {
        let Some(page) = self.pages.get(page_idx) else {
            return Vec::new();
        };
        let mut spans: Vec<(usize, usize)> = Vec::new(); // (start, end)
        for (i, (a, b)) in page.data.iter().zip(&page.ecu).enumerate() {
            if a == b {
                continue;
            }
            match spans.last_mut() {
                Some((_, end)) if i - *end <= max_gap => *end = i + 1,
                _ => spans.push((i, i + 1)),
            }
        }
        spans
            .into_iter()
            .map(|(start, end)| (start, page.data[start..end].to_vec()))
            .collect()
    }

    /// The span was written to the ECU successfully.
    pub fn mark_sent(&mut self, page_idx: usize, offset: usize, len: usize) {
        if let Some(page) = self.pages.get_mut(page_idx)
            && offset + len <= page.data.len()
        {
            let bytes = page.data[offset..offset + len].to_vec();
            page.ecu[offset..offset + len].copy_from_slice(&bytes);
        }
    }

    /// The page was burned: EEPROM now matches ECU RAM.
    pub fn mark_burned(&mut self, page_idx: usize) {
        if let Some(page) = self.pages.get_mut(page_idx) {
            page.burned = page.ecu.clone();
        }
    }
}

impl SymbolSource for Tune {
    fn value(&self, name: &str) -> Option<Value> {
        if self.def.constants.contains_key(name) {
            return self.constant_value(name);
        }
        if let Some(v) = self.pc_values.get(name) {
            return Some(Value::Num(*v));
        }
        // Derived output channels whose expressions only reference tune
        // state also resolve here (e.g. constant scales use
        // `fuelLoadRes = { algorithm == 0 ? 2.0 : 0.5 }`).
        if let Some(OutputChannel::Derived { expr, .. }) = self.def.output_channels.get(name) {
            return expr.eval(self).ok();
        }
        None
    }

    fn bit_string(&self, list: &str, index: usize) -> Option<String> {
        let def = self
            .def
            .constants
            .get(list)
            .or_else(|| self.def.pc_variables.get(list))?;
        def.labels.get(index).cloned()
    }

    fn array_value(&self, name: &str, index: usize) -> Option<f64> {
        self.array_element(name, index)
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub fn fixture() -> Arc<IniDef> {
        static DEF: std::sync::OnceLock<Arc<IniDef>> = std::sync::OnceLock::new();
        DEF.get_or_init(|| {
            let src = include_str!("../../../fixtures/speeduino202405_dev.ini");
            Arc::new(ts_ini::parse(src).unwrap())
        })
        .clone()
    }

    /// Pages filled with the fake ECU's deterministic pattern.
    pub fn loaded_tune() -> Tune {
        let def = fixture();
        let mut tune = Tune::new(def.clone());
        for (idx, &size) in def.header.page_sizes.clone().iter().enumerate() {
            let bytes: Vec<u8> = (0..size as usize)
                .map(|i| (((idx + 1) * 31 + i) & 0xFF) as u8)
                .collect();
            tune.load_page(idx, &bytes);
        }
        tune.set_loaded(true);
        tune
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::loaded_tune;
    use super::*;

    #[test]
    fn ve_table_view_and_edit() {
        let mut tune = loaded_tune();
        let table = tune.table("veTable1Tbl").expect("VE table decodes");
        assert_eq!(table.z.len(), 16);
        assert_eq!(table.z[0].len(), 16);
        assert_eq!(table.x.len(), 16);
        assert_eq!(table.y.len(), 16);

        // veTable is U08 scale 1.0 at page 2 offset 0: z[0][0] is byte 0 of
        // page 2's pattern; rpmBins scale 100 at offset 256.
        assert_eq!(table.z[0][0], ((2 * 31) & 0xFF) as f64);
        assert_eq!(table.x[0], (((2 * 31 + 256) & 0xFF) as f64) * 100.0);

        tune.set_table_cell("veTable1Tbl", 3, 5, 87.0).unwrap();
        let table = tune.table("veTable1Tbl").unwrap();
        assert_eq!(table.z[3][5], 87.0);

        // That cell is one byte at offset 3*16+5 on page 2 (index 1).
        assert!(tune.page_dirty(1));
        let spans = tune.dirty_spans(1, 4);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0], (3 * 16 + 5, vec![87]));
    }

    #[test]
    fn nearby_edits_coalesce_into_one_span() {
        let mut tune = loaded_tune();
        tune.set_table_cell("veTable1Tbl", 0, 0, 50.0).unwrap();
        tune.set_table_cell("veTable1Tbl", 0, 2, 60.0).unwrap();
        let spans = tune.dirty_spans(1, 4);
        assert_eq!(spans.len(), 1, "gap of 1 byte merges");
        assert_eq!(spans[0].0, 0);
        assert_eq!(spans[0].1.len(), 3);

        tune.mark_sent(1, 0, 3);
        assert!(!tune.page_dirty(1));
        assert!(tune.page_burn_pending(1));
        tune.mark_burned(1);
        assert!(!tune.burn_pending());
    }

    #[test]
    fn scalar_and_bits_round_trip() {
        let mut tune = loaded_tune();

        // reqFuel: scalar U08 scale 0.1 on page 1.
        tune.set_constant("reqFuel", 10.2).unwrap();
        let Value::Num(v) = tune.constant_value("reqFuel").unwrap() else {
            panic!("numeric")
        };
        assert!((v - 10.2).abs() < 1e-9, "{v}");

        // nCylinders: bits field.
        let before = tune.constant_value("nCylinders");
        tune.set_constant("nCylinders", 3.0).unwrap();
        let Value::Num(v) = tune.constant_value("nCylinders").unwrap() else {
            panic!("numeric")
        };
        assert_eq!(v, 3.0, "was {before:?}");
    }

    #[test]
    fn every_scalar_constant_encode_decode_identity() {
        // decode -> encode must reproduce the raw bytes for every scalar
        // constant whose scale resolves (guards the scale semantics).
        let mut tune = loaded_tune();
        let names: Vec<String> = tune
            .def
            .constants
            .iter()
            .filter(|(_, c)| c.class == ConstantClass::Scalar && c.page.is_some())
            .map(|(n, _)| n.clone())
            .collect();
        let mut checked = 0;
        for name in names {
            let Some(Value::Num(user)) = tune.constant_value(&name) else {
                continue;
            };
            let c = tune.constant(&name).unwrap();
            // The pattern data may violate the INI's lo/hi; encode clamps
            // to those, so identity only holds for in-range values.
            let in_range = |bound: &Option<ts_ini::NumOrExpr>, check: fn(f64, f64) -> bool| {
                bound
                    .as_ref()
                    .and_then(|b| b.eval(&tune).ok())
                    .is_none_or(|b| check(user, b))
            };
            if !in_range(&c.lo, |u, lo| u >= lo) || !in_range(&c.hi, |u, hi| u <= hi) {
                continue;
            }
            let (page_idx, offset) = tune.location(c).unwrap();
            let ty = c.ty;
            let before = read_raw(&tune.pages[page_idx].data, offset, ty).unwrap();
            if tune.set_constant(&name, user).is_err() {
                continue; // unresolvable {expr} scale — skip
            }
            let after = read_raw(&tune.pages[page_idx].data, offset, ty).unwrap();
            assert_eq!(
                before, after,
                "{name}: raw {before} -> {after} (user {user})"
            );
            checked += 1;
        }
        // With pattern page data ~150 scalars decode inside their lo/hi;
        // each of those must round-trip exactly.
        assert!(checked > 100, "only {checked} scalar constants checked");
    }

    #[test]
    fn expression_scale_resolves_from_tune() {
        // fuelLoadBins scale is {fuelLoadRes}, which depends on the
        // algorithm bits — both live in the tune itself.
        let tune = loaded_tune();
        let bins = tune.array_values("fuelLoadBins");
        assert!(bins.is_some(), "expression-scaled bins decode");
    }

    #[test]
    fn power_cycle_list() {
        let tune = loaded_tune();
        assert!(tune.requires_power_cycle("nCylinders"));
        assert!(!tune.requires_power_cycle("veTable"));
    }
}
