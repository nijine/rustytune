//! Line lexer for the value side of `key = v1, v2, ...` lines.
//!
//! Values are comma-separated, but commas inside double quotes and inside
//! `{ expression }` blocks don't split. Quoted strings keep their inner text
//! raw (escapes like `\$tsCanId` and `\x01` are meaningful to command-string
//! consumers, not to us). `[0:3]` / `[10]` / `[16x16]` shape tokens contain
//! no commas and land as `Bare`, parsed on demand by `parse_shape`.

use crate::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Unquoted word: identifier, number, shape, color name, ...
    Bare(String),
    /// `"..."` with the outer quotes removed, inner text untouched.
    Str(String),
    /// `{ ... }` with the outer braces removed and trimmed, unparsed.
    Expr(String),
}

impl Token {
    pub fn as_str(&self) -> &str {
        match self {
            Token::Bare(s) | Token::Str(s) | Token::Expr(s) => s,
        }
    }

    pub fn number(&self, num: u32) -> Result<f64, Error> {
        match self {
            Token::Bare(s) => s
                .parse::<f64>()
                .map_err(|_| Error::at(num, format!("expected a number, got `{s}`"))),
            other => Err(Error::at(
                num,
                format!("expected a number, got `{}`", other.as_str()),
            )),
        }
    }
}

/// Split at the first `=` (outside quotes) into `(key, value_text)`.
pub fn split_kv(line: &str) -> Option<(&str, &str)> {
    let mut in_quote = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            '=' if !in_quote => return Some((line[..i].trim(), line[i + 1..].trim())),
            _ => {}
        }
    }
    None
}

/// Tokenize the value side of an assignment.
pub fn tokenize(values: &str, num: u32) -> Result<Vec<Token>, Error> {
    let mut tokens = Vec::new();
    let mut piece = String::new();
    let mut in_quote = false;
    let mut escaped = false;
    let mut brace_depth = 0usize;

    for c in values.chars() {
        if in_quote {
            piece.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_quote = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_quote = true;
                piece.push(c);
            }
            '{' => {
                // TunerStudio accepts a condition immediately after a value
                // without a separating comma (`field = "Label", name { x }`).
                // Treat the expression as a new token when a top-level bare
                // value has already been accumulated.
                if brace_depth == 0 && !piece.trim().is_empty() {
                    push_piece(&mut tokens, &piece, num)?;
                    piece.clear();
                }
                brace_depth += 1;
                piece.push(c);
            }
            '}' => {
                brace_depth = brace_depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::at(num, "unbalanced `}`"))?;
                piece.push(c);
            }
            ',' if brace_depth == 0 => {
                push_piece(&mut tokens, &piece, num)?;
                piece.clear();
            }
            _ => piece.push(c),
        }
    }
    if in_quote {
        return Err(Error::at(num, "unterminated string"));
    }
    if brace_depth != 0 {
        return Err(Error::at(num, "unterminated `{` expression"));
    }
    push_piece(&mut tokens, &piece, num)?;

    // `key =` with nothing after: no tokens rather than one empty token.
    if tokens.len() == 1 && matches!(&tokens[0], Token::Bare(s) if s.is_empty()) {
        tokens.clear();
    }
    Ok(tokens)
}

fn push_piece(tokens: &mut Vec<Token>, piece: &str, num: u32) -> Result<(), Error> {
    let piece = piece.trim();
    let token = if let Some(rest) = piece.strip_prefix('"') {
        let inner = rest
            .strip_suffix('"')
            .ok_or_else(|| Error::at(num, "text after closing quote"))?;
        Token::Str(inner.to_string())
    } else if let Some(rest) = piece.strip_prefix('{') {
        let inner = rest
            .strip_suffix('}')
            .ok_or_else(|| Error::at(num, "text after closing `}`"))?;
        Token::Expr(inner.trim().to_string())
    } else {
        Token::Bare(piece.to_string())
    };
    tokens.push(token);
    Ok(())
}

/// A `[...]` shape token from a constant definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `[lo:hi]` — a bit range within the underlying word.
    Bits { lo: u8, hi: u8 },
    /// `[n]` — a 1-D array.
    Array1D(u16),
    /// `[x×y]` — a 2-D table, x columns by y rows.
    Array2D { x: u16, y: u16 },
}

impl Shape {
    /// Number of elements ([Bits] occupies its word; callers size by type).
    pub fn element_count(&self) -> u32 {
        match *self {
            Shape::Bits { .. } => 1,
            Shape::Array1D(n) => n as u32,
            Shape::Array2D { x, y } => x as u32 * y as u32,
        }
    }
}

pub fn parse_shape(token: &str, num: u32) -> Result<Shape, Error> {
    let inner = token
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| Error::at(num, format!("expected `[shape]`, got `{token}`")))?
        .trim();
    let bad = || Error::at(num, format!("malformed shape `[{inner}]`"));
    if let Some((lo, hi)) = inner.split_once(':') {
        let lo = lo.trim().parse().map_err(|_| bad())?;
        let hi = hi.trim().parse().map_err(|_| bad())?;
        return Ok(Shape::Bits { lo, hi });
    }
    if let Some((x, y)) = inner.split_once('x') {
        let x = x.trim().parse().map_err(|_| bad())?;
        let y = y.trim().parse().map_err(|_| bad())?;
        return Ok(Shape::Array2D { x, y });
    }
    Ok(Shape::Array1D(inner.parse().map_err(|_| bad())?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_top_level_commas_only() {
        let toks = tokenize(
            r#"scalar, U08, 38, { bitStringValue( idleUnits, iacAlgorithm ) }, { a ? 1 : 2 }, 0.000"#,
            1,
        )
        .unwrap();
        assert_eq!(toks.len(), 6);
        assert_eq!(toks[0], Token::Bare("scalar".into()));
        assert_eq!(
            toks[3],
            Token::Expr("bitStringValue( idleUnits, iacAlgorithm )".into())
        );
    }

    #[test]
    fn splits_trailing_expression_without_comma() {
        let toks = tokenize(r#""Trigger edge", TrigEdge { TrigPattern != 4 }"#, 1).unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Str("Trigger edge".into()),
                Token::Bare("TrigEdge".into()),
                Token::Expr("TrigPattern != 4".into()),
            ]
        );
    }

    #[test]
    fn expression_after_comma_remains_one_token() {
        let toks = tokenize(r#"name, { enabled && mode == 1 }"#, 1).unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Bare("name".into()),
                Token::Expr("enabled && mode == 1".into()),
            ]
        );
    }

    #[test]
    fn quoted_strings_keep_commas_and_escapes() {
        let toks = tokenize(r#""a, b", "r\$tsCanId\x30%2o%2c""#, 1).unwrap();
        assert_eq!(toks[0], Token::Str("a, b".into()));
        assert_eq!(toks[1], Token::Str(r"r\$tsCanId\x30%2o%2c".into()));
    }

    #[test]
    fn kv_split_ignores_equals_in_strings() {
        let (k, v) = split_kv(r#"indicator = { a == 1 }, "x""#).unwrap();
        assert_eq!(k, "indicator");
        assert_eq!(v, r#"{ a == 1 }, "x""#);
    }

    #[test]
    fn shapes() {
        assert_eq!(
            parse_shape("[0:3]", 1).unwrap(),
            Shape::Bits { lo: 0, hi: 3 }
        );
        assert_eq!(parse_shape("[ 4]", 1).unwrap(), Shape::Array1D(4));
        assert_eq!(
            parse_shape("[16x16]", 1).unwrap(),
            Shape::Array2D { x: 16, y: 16 }
        );
    }
}
