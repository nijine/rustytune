//! TunerStudio `.msq` tune files: parse, serialize, apply-to-tune, and
//! diff-against-tune.
//!
//! The format (fileFormat 5.0, captured from a real TS save) is XML with
//! numbered `<page>` elements (0-based; INI page minus one) holding
//! `<constant>` entries: scalars as numbers **in user units under the
//! profile that saved them** (see `<settings>`), bits as the quoted combo
//! label, arrays as whitespace-separated values in memory order. Values
//! are matched by constant *name*, so files survive firmware version
//! drift; anything unmatched is reported, never silently dropped.

use std::collections::HashSet;

use indexmap::IndexMap;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::{ConstantClass, Tune, TuneError};
use ts_ini::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum MsqValue {
    Num(f64),
    /// Quoted text: a bits combo label or a string constant.
    Text(String),
    /// Array values in memory (offset) order.
    Array(Vec<f64>),
}

#[derive(Debug, Clone, Default)]
pub struct MsqFile {
    pub signature: Option<String>,
    pub firmware_info: Option<String>,
    pub author: Option<String>,
    pub write_date: Option<String>,
    /// Symbol names from `<settings>` (e.g. FAHRENHEIT, AFR).
    pub settings: Vec<String>,
    pub constants: IndexMap<String, MsqValue>,
    pub pc_variables: IndexMap<String, MsqValue>,
}

#[derive(Debug, thiserror::Error)]
pub enum MsqError {
    #[error("xml: {0}")]
    Xml(String),
}

/// `.msq` files declare ISO-8859-1; decode bytes losslessly.
pub fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

fn parse_body(text: &str) -> MsqValue {
    let trimmed = text.trim();
    if let Some(stripped) = trimmed.strip_prefix('"') {
        return MsqValue::Text(stripped.trim_end_matches('"').to_string());
    }
    let nums: Option<Vec<f64>> = trimmed
        .split_whitespace()
        .map(|tok| tok.parse::<f64>().ok())
        .collect();
    match nums {
        Some(nums) if nums.len() == 1 => MsqValue::Num(nums[0]),
        Some(nums) if !nums.is_empty() => MsqValue::Array(nums),
        _ => MsqValue::Text(trimmed.to_string()),
    }
}

pub fn parse(src: &str) -> Result<MsqFile, MsqError> {
    let mut reader = Reader::from_str(src);
    let mut file = MsqFile::default();
    let mut element: Option<(String, String)> = None; // (tag, name attr)
    let mut body = String::new();
    let mut in_numbered_page = false;

    let attr_of = |e: &quick_xml::events::BytesStart, key: &[u8]| -> Option<String> {
        e.attributes().flatten().find_map(|a| {
            (a.key.as_ref() == key)
                .then(|| a.unescape_value().map(|v| v.into_owned()).ok())
                .flatten()
        })
    };

    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) => match e.name().as_ref() {
                b"bibliography" => {
                    file.author = attr_of(&e, b"author");
                    file.write_date = attr_of(&e, b"writeDate");
                }
                b"versionInfo" => {
                    file.signature = attr_of(&e, b"signature");
                    file.firmware_info = attr_of(&e, b"firmwareInfo");
                }
                b"page" => in_numbered_page = attr_of(&e, b"number").is_some(),
                b"setting" => {
                    if let Some(name) = attr_of(&e, b"name") {
                        file.settings.push(name);
                    }
                }
                b"constant" | b"pcVariable" => {
                    if let Some(name) = attr_of(&e, b"name") {
                        let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                        element = Some((tag, name));
                        body.clear();
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if element.is_some() {
                    body.push_str(&t.unescape().map_err(|e| MsqError::Xml(e.to_string()))?);
                    body.push(' ');
                }
            }
            Ok(Event::End(e)) => {
                if matches!(e.name().as_ref(), b"constant" | b"pcVariable")
                    && let Some((tag, name)) = element.take()
                {
                    let value = parse_body(&body);
                    if tag == "pcVariable" {
                        file.pc_variables.insert(name, value);
                    } else if in_numbered_page {
                        file.constants.insert(name, value);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(MsqError::Xml(e.to_string())),
            _ => {}
        }
    }
    Ok(file)
}

// ----- diff -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum DiffKind {
    Scalar {
        ecu: f64,
        file: f64,
    },
    Bits {
        ecu: String,
        file: String,
    },
    /// Array with per-element differences (indices in memory order).
    Array {
        changed: Vec<usize>,
        len: usize,
    },
}

#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub name: String,
    pub page: Option<u8>,
    pub kind: DiffKind,
}

#[derive(Debug, Clone, Default)]
pub struct MsqDiff {
    pub entries: Vec<DiffEntry>,
    /// Constants in the file that this definition doesn't know (version
    /// drift, or the overlaid twin of a settings-dependent constant).
    pub only_in_file: Vec<String>,
    /// Constants that couldn't be compared (unresolvable scale, label
    /// missing from the combo list, length mismatch), with reasons.
    pub unresolved: Vec<(String, String)>,
}

fn bits_label(tune: &Tune, name: &str, index: i64) -> String {
    tune.def()
        .constants
        .get(name)
        .and_then(|c| c.labels.get(index as usize).cloned())
        .unwrap_or_else(|| index.to_string())
}

fn file_bits_index(tune: &Tune, name: &str, value: &MsqValue) -> Option<i64> {
    match value {
        MsqValue::Num(n) => Some(*n as i64),
        // "INVALID" is a placeholder shared by many combo slots — as a
        // label it's ambiguous (saves write those as numbers instead).
        MsqValue::Text(label) if label == "INVALID" => None,
        MsqValue::Text(label) => tune
            .def()
            .constants
            .get(name)?
            .labels
            .iter()
            .position(|l| l == label)
            .map(|i| i as i64),
        MsqValue::Array(_) => None,
    }
}

/// Compare a file against the tune's current (local) state. Comparison is
/// exact at the raw-byte level: file values are encoded through the same
/// scale/translate path as edits, so float formatting noise can't produce
/// phantom differences.
pub fn diff(file: &MsqFile, tune: &Tune) -> MsqDiff {
    let mut out = MsqDiff::default();

    for (name, value) in &file.constants {
        let Some(c) = tune.def().constants.get(name).cloned() else {
            out.only_in_file.push(name.clone());
            continue;
        };
        let unresolved = |reason: &str| (name.clone(), reason.to_string());
        match (c.class, value) {
            (ConstantClass::Scalar, MsqValue::Num(user_file)) => {
                let (Some(raw_cur), Ok(raw_file)) =
                    (tune.constant_raw(name), tune.encode_user(name, *user_file))
                else {
                    out.unresolved.push(unresolved("scale did not resolve"));
                    continue;
                };
                if raw_cur != raw_file {
                    let ecu = match tune.constant_value(name) {
                        Some(Value::Num(n)) => n,
                        _ => f64::NAN,
                    };
                    out.entries.push(DiffEntry {
                        name: name.clone(),
                        page: c.page,
                        kind: DiffKind::Scalar {
                            ecu,
                            file: *user_file,
                        },
                    });
                }
            }
            (ConstantClass::Bits, _) => {
                let (Some(raw_cur), Some(idx_file)) =
                    (tune.constant_raw(name), file_bits_index(tune, name, value))
                else {
                    out.unresolved.push(unresolved("label not in combo list"));
                    continue;
                };
                if raw_cur != idx_file {
                    out.entries.push(DiffEntry {
                        name: name.clone(),
                        page: c.page,
                        kind: DiffKind::Bits {
                            ecu: bits_label(tune, name, raw_cur),
                            file: bits_label(tune, name, idx_file),
                        },
                    });
                }
            }
            (ConstantClass::Array, MsqValue::Array(values)) => {
                let len = c.shape.map_or(0, |s| s.element_count()) as usize;
                if values.len() != len {
                    out.unresolved.push(unresolved("array length mismatch"));
                    continue;
                }
                let mut changed = Vec::new();
                for (i, user_file) in values.iter().enumerate() {
                    let (Some(raw_cur), Ok(raw_file)) =
                        (tune.array_raw(name, i), tune.encode_user(name, *user_file))
                    else {
                        changed.clear();
                        break;
                    };
                    if raw_cur != raw_file {
                        changed.push(i);
                    }
                }
                if !changed.is_empty() {
                    out.entries.push(DiffEntry {
                        name: name.clone(),
                        page: c.page,
                        kind: DiffKind::Array { changed, len },
                    });
                }
            }
            (ConstantClass::String, MsqValue::Text(text)) => {
                if let Some(Value::Str(current)) = tune.constant_value(name)
                    && current != *text
                {
                    out.entries.push(DiffEntry {
                        name: name.clone(),
                        page: c.page,
                        kind: DiffKind::Bits {
                            ecu: current,
                            file: text.clone(),
                        },
                    });
                }
            }
            _ => out.unresolved.push(unresolved("value shape mismatch")),
        }
    }
    out
}

// ----- apply ----------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    pub applied: usize,
    pub skipped: Vec<(String, String)>,
}

/// Write file values into the tune's local data (the flush loop then sends
/// them to the ECU). `names` restricts to a subset (selective send).
pub fn apply(file: &MsqFile, tune: &mut Tune, names: Option<&HashSet<String>>) -> ApplyReport {
    let mut report = ApplyReport::default();
    for (name, value) in &file.constants {
        if let Some(filter) = names
            && !filter.contains(name)
        {
            continue;
        }
        let Some(class) = tune.def().constants.get(name).map(|c| c.class) else {
            report
                .skipped
                .push((name.clone(), "not in definition".into()));
            continue;
        };
        let result: Result<(), TuneError> = match (class, value) {
            (ConstantClass::Scalar, MsqValue::Num(v)) => tune.set_constant_unclamped(name, *v),
            (ConstantClass::Bits, _) => match file_bits_index(tune, name, value) {
                Some(idx) => tune.set_constant_unclamped(name, idx as f64),
                None => Err(TuneError::NotEditable(name.clone(), "label not found")),
            },
            (ConstantClass::Array, MsqValue::Array(values)) => {
                let len = tune
                    .def()
                    .constants
                    .get(name)
                    .and_then(|c| c.shape)
                    .map_or(0, |s| s.element_count()) as usize;
                if values.len() != len {
                    Err(TuneError::NotEditable(
                        name.clone(),
                        "array length mismatch",
                    ))
                } else {
                    values
                        .iter()
                        .enumerate()
                        .try_for_each(|(i, v)| tune.set_array_element_unclamped(name, i, *v))
                }
            }
            (ConstantClass::String, _) => {
                Err(TuneError::NotEditable(name.clone(), "string constant"))
            }
            _ => Err(TuneError::NotEditable(name.clone(), "value shape mismatch")),
        };
        match result {
            Ok(()) => report.applied += 1,
            Err(e) => report.skipped.push((name.clone(), e.to_string())),
        }
    }
    report
}

// ----- save -----------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// TS-style number formatting: integers get one decimal ("28.0"), others
/// keep their precision without float tails.
fn fmt_num(v: f64) -> String {
    let rounded = (v * 1e6).round() / 1e6;
    if rounded == rounded.trunc() && rounded.abs() < 1e15 {
        format!("{rounded:.1}")
    } else {
        let s = format!("{rounded}");
        s
    }
}

/// Serialize the tune's local state as a TunerStudio-compatible `.msq`.
pub fn save(tune: &Tune, symbols: &[String], author: &str, write_date: &str) -> String {
    let def = tune.def();
    let mut out = String::with_capacity(128 * 1024);
    out.push_str("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n");
    out.push_str("<msq xmlns=\"http://www.msefi.com/:msq\">\n");
    out.push_str(&format!(
        "<bibliography author=\"{}\" tuneComment=\"\" writeDate=\"{}\"/>\n",
        xml_escape(author),
        xml_escape(write_date)
    ));
    out.push_str(&format!(
        "<versionInfo fileFormat=\"5.0\" firmwareInfo=\"{}\" nPages=\"{}\" signature=\"{}\"/>\n",
        xml_escape(&def.signature.replace(' ', "+")),
        tune.page_count(),
        xml_escape(&def.signature)
    ));

    for page_idx in 0..tune.page_count() {
        out.push_str(&format!("<page number=\"{page_idx}\">\n"));
        let page_num = (page_idx + 1) as u8;
        let names: Vec<String> = def
            .constants
            .iter()
            .filter(|(_, c)| c.page == Some(page_num) && !c.no_msq_save)
            .map(|(n, _)| n.clone())
            .collect();
        // Byte ranges already written by a value-owning constant. Overlaid
        // views (lambdaTable/afrTable via lastOffset, outputPin array vs
        // outputPin0.. scalars) are saved once, first definition wins —
        // matching TS files, which carry only the active view. Bits fields
        // are exempt: several legitimately share one byte.
        let mut owned: Vec<(u32, u32)> = Vec::new();
        for name in names {
            let c = &def.constants[&name];
            if c.class != ConstantClass::Bits {
                let start = c.offset.unwrap_or(0);
                let end = start + c.byte_size();
                if owned.iter().any(|&(s, e)| start < e && s < end) {
                    continue;
                }
                owned.push((start, end));
            }
            let digits = c
                .digits
                .as_ref()
                .and_then(|d| d.eval(tune).ok())
                .unwrap_or(0.0) as u8;
            let units = c.units.as_ref().and_then(|u| u.eval(tune).ok());
            let units_attr = units
                .map(|u| format!(" units=\"{}\"", xml_escape(u.trim_matches('"'))))
                .unwrap_or_default();
            match c.class {
                ConstantClass::Scalar => {
                    if let Some(Value::Num(v)) = tune.constant_value(&name) {
                        out.push_str(&format!(
                            "<constant digits=\"{digits}\" name=\"{name}\"{units_attr}>{}</constant>\n",
                            fmt_num(v)
                        ));
                    }
                }
                ConstantClass::Bits => {
                    if let Some(Value::Num(idx)) = tune.constant_value(&name) {
                        let idx = idx as i64;
                        // INVALID (or out-of-list) selections can't round-trip
                        // as labels — write the index as a number.
                        match c.labels.get(idx as usize) {
                            Some(label) if label != "INVALID" => out.push_str(&format!(
                                "<constant name=\"{name}\">&quot;{}&quot;</constant>\n",
                                xml_escape(label)
                            )),
                            _ => out
                                .push_str(&format!("<constant name=\"{name}\">{idx}</constant>\n")),
                        }
                    }
                }
                ConstantClass::Array => {
                    let Some(values) = tune.array_values(&name) else {
                        continue;
                    };
                    let (cols, rows) = match c.shape {
                        Some(ts_ini::Shape::Array2D { x, y }) => (x as usize, y as usize),
                        Some(ts_ini::Shape::Array1D(n)) => (1, n as usize),
                        _ => continue,
                    };
                    out.push_str(&format!(
                        "<constant cols=\"{cols}\" digits=\"{digits}\" name=\"{name}\" rows=\"{rows}\"{units_attr}>\n"
                    ));
                    for row in values.chunks(cols.max(1)) {
                        out.push_str("         ");
                        for v in row {
                            out.push_str(&fmt_num(*v));
                            out.push(' ');
                        }
                        out.push('\n');
                    }
                    out.push_str("      </constant>\n");
                }
                ConstantClass::String => {
                    if let Some(Value::Str(s)) = tune.constant_value(&name) {
                        out.push_str(&format!(
                            "<constant name=\"{name}\">&quot;{}&quot;</constant>\n",
                            xml_escape(&s)
                        ));
                    }
                }
            }
        }
        out.push_str("</page>\n");
    }

    out.push_str(
        "<settings Comment=\"These setting are only used if this msq is opened without a project.\">\n",
    );
    for symbol in symbols {
        out.push_str(&format!(
            "<setting name=\"{0}\" value=\"{0}\"/>\n",
            xml_escape(symbol)
        ));
    }
    out.push_str("</settings>\n</msq>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::loaded_tune;

    fn real_msq() -> MsqFile {
        let bytes = include_bytes!("../../../fixtures/CurrentTune.msq");
        parse(&decode_latin1(bytes)).unwrap()
    }

    #[test]
    fn parses_real_tunerstudio_file() {
        let file = real_msq();
        assert_eq!(file.signature.as_deref(), Some("speeduino 202501"));
        assert!(file.constants.len() > 700, "{}", file.constants.len());
        assert!(file.settings.iter().any(|s| s == "FAHRENHEIT"));

        assert_eq!(file.constants["aeColdPct"], MsqValue::Num(100.0));
        assert_eq!(file.constants["aeMode"], MsqValue::Text("TPS".into()));
        let MsqValue::Array(ve) = &file.constants["veTable"] else {
            panic!("veTable should be an array");
        };
        assert_eq!(ve.len(), 256);
        assert_eq!(ve[0], 28.0);

        assert_eq!(file.pc_variables["rpmhigh"], MsqValue::Num(8000.0));
    }

    #[test]
    fn save_then_diff_is_empty() {
        let tune = loaded_tune();
        let saved = save(&tune, &[], "rustytune test", "today");
        let file = parse(&saved).unwrap();
        assert_eq!(file.signature.as_deref(), Some("speeduino 202405-dev"));

        let d = diff(&file, &tune);
        assert!(
            d.entries.is_empty(),
            "round-trip must not drift: {:?}",
            &d.entries[..d.entries.len().min(5)]
        );
        assert!(d.only_in_file.is_empty(), "{:?}", d.only_in_file);
    }

    #[test]
    fn diff_pinpoints_changes() {
        let tune = loaded_tune();
        let saved = save(&tune, &[], "t", "d");
        let file = parse(&saved).unwrap();

        let mut edited = loaded_tune();
        edited.set_table_cell("veTable1Tbl", 3, 5, 99.0).unwrap();
        edited.set_constant("reqFuel", 12.6).unwrap();

        let d = diff(&file, &edited);
        let names: Vec<&str> = d.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"veTable"), "{names:?}");
        assert!(names.contains(&"reqFuel"), "{names:?}");
        assert_eq!(d.entries.len(), 2, "{names:?}");

        let ve = d.entries.iter().find(|e| e.name == "veTable").unwrap();
        let DiffKind::Array { changed, len } = &ve.kind else {
            panic!("array diff");
        };
        assert_eq!(*len, 256);
        assert_eq!(changed, &[3 * 16 + 5]);
    }

    #[test]
    fn apply_makes_diff_empty() {
        let mut tune = loaded_tune();
        let mut other = loaded_tune();
        other.set_table_cell("veTable1Tbl", 0, 0, 42.0).unwrap();
        other.set_constant("reqFuel", 11.0).unwrap();
        let file = parse(&save(&other, &[], "t", "d")).unwrap();

        let report = apply(&file, &mut tune, None);
        assert!(report.applied > 500, "{}", report.applied);
        let d = diff(&file, &tune);
        assert!(
            d.entries.is_empty(),
            "{:?}",
            &d.entries[..d.entries.len().min(5)]
        );
    }

    #[test]
    fn selective_apply() {
        let mut tune = loaded_tune();
        let mut other = loaded_tune();
        other.set_constant("reqFuel", 11.0).unwrap();
        other.set_table_cell("veTable1Tbl", 0, 0, 42.0).unwrap();
        let file = parse(&save(&other, &[], "t", "d")).unwrap();

        let names: HashSet<String> = ["reqFuel".to_string()].into();
        apply(&file, &mut tune, Some(&names));

        let d = diff(&file, &tune);
        let names: Vec<&str> = d.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(!names.contains(&"reqFuel"), "reqFuel was applied");
        assert!(names.contains(&"veTable"), "veTable must remain different");
    }
}
