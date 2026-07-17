//! Decoding a raw realtime (och) block into named channel values.
//!
//! Data channels decode little-endian bytes at their offset and apply
//! `raw * scale + translate` (OutputChannels semantics — distinct from
//! Constants' `(raw + translate) * scale`). Derived channels evaluate their
//! expression with this frame as the symbol source, so chains like
//! `lambda = { afr / stoich }` resolve `afr` from the block and `stoich`
//! from a caller-supplied fallback (tune constants, PcVariables, ...).

use crate::expr::{SymbolSource, Value};
use crate::model::{DataType, IniDef, OutputChannel};

pub struct Telemetry<'a> {
    def: &'a IniDef,
    block: &'a [u8],
    /// Resolves identifiers that aren't output channels: tune constants
    /// (`stoich`), PcVariables, `timeNow`, ...
    extra: Option<&'a dyn SymbolSource>,
}

impl<'a> Telemetry<'a> {
    pub fn new(def: &'a IniDef, block: &'a [u8]) -> Self {
        Telemetry {
            def,
            block,
            extra: None,
        }
    }

    pub fn with_extra(def: &'a IniDef, block: &'a [u8], extra: &'a dyn SymbolSource) -> Self {
        Telemetry {
            def,
            block,
            extra: Some(extra),
        }
    }

    /// Decode one channel to its user-facing value.
    pub fn channel(&self, name: &str) -> Option<Value> {
        match self.def.output_channels.get(name)? {
            OutputChannel::Scalar {
                ty,
                offset,
                scale,
                translate,
                ..
            } => {
                let raw = read_raw(self.block, *offset as usize, *ty)?;
                let scale = scale.eval(self).ok()?;
                let translate = translate.eval(self).ok()?;
                Some(Value::Num(raw * scale + translate))
            }
            OutputChannel::Bits { ty, offset, lo, hi } => {
                let raw = read_raw(self.block, *offset as usize, *ty)? as u64;
                let width = hi - lo + 1;
                let mask = (1u64 << width) - 1;
                Some(Value::Num(((raw >> lo) & mask) as f64))
            }
            OutputChannel::Derived { expr, .. } => expr.eval(self).ok(),
        }
    }
}

fn read_raw(block: &[u8], offset: usize, ty: DataType) -> Option<f64> {
    let size = ty.size() as usize;
    let bytes = block.get(offset..offset + size)?;
    let unsigned = match size {
        1 => bytes[0] as u64,
        2 => u16::from_le_bytes(bytes.try_into().unwrap()) as u64,
        4 => u32::from_le_bytes(bytes.try_into().unwrap()) as u64,
        _ => return None,
    };
    Some(if ty.signed() {
        match size {
            1 => unsigned as u8 as i8 as f64,
            2 => unsigned as u16 as i16 as f64,
            _ => unsigned as u32 as i32 as f64,
        }
    } else {
        unsigned as f64
    })
}

impl SymbolSource for Telemetry<'_> {
    fn value(&self, name: &str) -> Option<Value> {
        if self.def.output_channels.contains_key(name) {
            return self.channel(name);
        }
        self.extra.and_then(|e| e.value(name))
    }

    fn bit_string(&self, list: &str, index: usize) -> Option<String> {
        // Bits label lists live on constants/PcVariables.
        let def = self
            .def
            .constants
            .get(list)
            .or_else(|| self.def.pc_variables.get(list))?;
        def.labels.get(index).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_decode() {
        // S16 at offset 0: -300
        let block = (-300i16).to_le_bytes();
        assert_eq!(read_raw(&block, 0, DataType::S16), Some(-300.0));
        assert_eq!(read_raw(&[0x85], 0, DataType::S08), Some(-123.0));
        assert_eq!(read_raw(&[0x85], 0, DataType::U08), Some(133.0));
    }

    #[test]
    fn out_of_range_is_none() {
        assert_eq!(read_raw(&[1, 2], 1, DataType::U16), None);
    }
}
