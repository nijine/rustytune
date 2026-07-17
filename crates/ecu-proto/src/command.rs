//! Builder for command byte strings from INI templates.
//!
//! The INI describes every serial command as a template string, e.g.
//! `pageReadCommand = "p%2i%2o%2c"`, `ochGetCommand = "r\$tsCanId\x30%2o%2c"`,
//! `pageIdentifier = "\$tsCanId\x01"`. Command bytes are always built from
//! these — never hardcoded — so a firmware INI change costs nothing.
//!
//! Recognized pieces:
//! - literal characters (sent as-is)
//! - `\xNN` — a literal byte in hex
//! - `\$name` — a runtime variable (today: `tsCanId`)
//! - `%2i` — the 2-byte page identifier (itself a template)
//! - `%2o` — offset, little-endian u16
//! - `%2c` — count, little-endian u16
//! - `%v`  — the value bytes being written

use crate::ProtoError;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Literal(Vec<u8>),
    Var(String),
    PageId,
    Offset,
    Count,
    Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

/// Runtime values substituted into a template.
#[derive(Debug, Clone, Default)]
pub struct Args<'a> {
    pub can_id: u8,
    /// Pre-built page identifier bytes (from the page's `pageIdentifier`).
    pub page_id: Option<&'a [u8]>,
    pub offset: Option<u16>,
    pub count: Option<u16>,
    pub value: Option<&'a [u8]>,
}

impl Template {
    pub fn parse(text: &str) -> Result<Self, ProtoError> {
        let mut parts = Vec::new();
        let mut literal = Vec::new();
        let mut chars = text.chars().peekable();

        fn flush(literal: &mut Vec<u8>, parts: &mut Vec<Part>) {
            if !literal.is_empty() {
                parts.push(Part::Literal(std::mem::take(literal)));
            }
        }

        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('x') => {
                        let hi = chars.next();
                        let lo = chars.next();
                        let (Some(hi), Some(lo)) = (hi, lo) else {
                            return Err(ProtoError::Template(format!(
                                "truncated \\x escape in `{text}`"
                            )));
                        };
                        let byte = u8::from_str_radix(&format!("{hi}{lo}"), 16).map_err(|_| {
                            ProtoError::Template(format!("bad \\x{hi}{lo} in `{text}`"))
                        })?;
                        literal.push(byte);
                    }
                    Some('$') => {
                        flush(&mut literal, &mut parts);
                        let mut name = String::new();
                        while let Some(&n) = chars.peek() {
                            if n.is_ascii_alphanumeric() || n == '_' {
                                name.push(n);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        parts.push(Part::Var(name));
                    }
                    Some(other) => literal.push(other as u8),
                    None => {
                        return Err(ProtoError::Template(format!(
                            "trailing backslash in `{text}`"
                        )));
                    }
                },
                '%' => {
                    flush(&mut literal, &mut parts);
                    let spec: String = [chars.next(), chars.peek().copied()]
                        .into_iter()
                        .flatten()
                        .collect();
                    match spec.as_str() {
                        s if s.starts_with('v') => parts.push(Part::Value),
                        "2i" => {
                            chars.next();
                            parts.push(Part::PageId);
                        }
                        "2o" => {
                            chars.next();
                            parts.push(Part::Offset);
                        }
                        "2c" => {
                            chars.next();
                            parts.push(Part::Count);
                        }
                        other => {
                            return Err(ProtoError::Template(format!(
                                "unknown %{other} in `{text}`"
                            )));
                        }
                    }
                }
                c => literal.push(c as u8),
            }
        }
        flush(&mut literal, &mut parts);
        Ok(Template { parts })
    }

    pub fn build(&self, args: &Args) -> Result<Vec<u8>, ProtoError> {
        let mut out = Vec::new();
        let missing = |what: &str| ProtoError::Template(format!("template needs {what}"));
        for part in &self.parts {
            match part {
                Part::Literal(bytes) => out.extend_from_slice(bytes),
                Part::Var(name) => match name.as_str() {
                    "tsCanId" => out.push(args.can_id),
                    other => return Err(missing(&format!("unknown variable ${other}"))),
                },
                Part::PageId => {
                    out.extend_from_slice(args.page_id.ok_or_else(|| missing("page id"))?);
                }
                Part::Offset => {
                    out.extend_from_slice(
                        &args.offset.ok_or_else(|| missing("offset"))?.to_le_bytes(),
                    );
                }
                Part::Count => {
                    out.extend_from_slice(
                        &args.count.ok_or_else(|| missing("count"))?.to_le_bytes(),
                    );
                }
                Part::Value => {
                    out.extend_from_slice(args.value.ok_or_else(|| missing("value"))?);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn och_get_command() {
        // "r\$tsCanId\x30%2o%2c" -> 'r' + canId + 0x30 + offset LE + count LE
        let t = Template::parse(r"r\$tsCanId\x30%2o%2c").unwrap();
        let bytes = t
            .build(&Args {
                can_id: 0,
                offset: Some(0),
                count: Some(127),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(bytes, [b'r', 0x00, 0x30, 0x00, 0x00, 127, 0x00]);
    }

    #[test]
    fn page_identifier_and_read() {
        let ident = Template::parse(r"\$tsCanId\x02").unwrap();
        let page_id = ident
            .build(&Args {
                can_id: 0,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page_id, [0x00, 0x02]);

        let read = Template::parse("p%2i%2o%2c").unwrap();
        let bytes = read
            .build(&Args {
                page_id: Some(&page_id),
                offset: Some(256),
                count: Some(32),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(bytes, [b'p', 0x00, 0x02, 0x00, 0x01, 32, 0x00]);
    }

    #[test]
    fn chunk_write_with_value() {
        let t = Template::parse("M%2i%2o%2c%v").unwrap();
        let bytes = t
            .build(&Args {
                page_id: Some(&[0x00, 0x01]),
                offset: Some(4),
                count: Some(2),
                value: Some(&[0xAB, 0xCD]),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            bytes,
            [b'M', 0x00, 0x01, 0x04, 0x00, 0x02, 0x00, 0xAB, 0xCD]
        );
    }

    #[test]
    fn burn_command() {
        let t = Template::parse("b%2i").unwrap();
        let bytes = t
            .build(&Args {
                page_id: Some(&[0x00, 0x05]),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(bytes, [b'b', 0x00, 0x05]);
    }
}
