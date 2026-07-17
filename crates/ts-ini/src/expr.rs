//! `{ expression }` parsing and evaluation.
//!
//! Expressions appear anywhere a scalar or string is expected: output-channel
//! math (`(coolantRaw - 40) * 1.8 + 32`), gauge limits (`rpmhigh`), constant
//! scales (`0.1 / stoich`), indicator conditions, datalog visibility, and
//! unit selection (`bitStringValue(algorithmUnits, algorithm)`). Identifiers
//! are resolved at evaluation time against a caller-supplied [`SymbolSource`]
//! (output channels, constants, PcVariables — whatever is in scope).

use std::fmt;

use crate::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(String),
    Ident(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    /// `name[index]` — array constant element.
    Index(String, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

/// Result of evaluating an expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
}

impl Value {
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Str(_) => None,
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Num(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "{s}"),
        }
    }
}

/// Where identifiers and builtin calls get their values from.
pub trait SymbolSource {
    /// Value of a plain identifier (`rpm`, `stoich`, `timeNow`, ...).
    fn value(&self, name: &str) -> Option<Value>;

    /// `bitStringValue(listName, index)` — the index'th label of a bits
    /// constant / PcVariable list.
    fn bit_string(&self, _list: &str, _index: usize) -> Option<String> {
        None
    }

    /// `arrayValue(name, index)` — element of an array constant.
    fn array_value(&self, _name: &str, _index: usize) -> Option<f64> {
        None
    }
}

/// A `SymbolSource` with nothing in scope; evaluation still works for
/// constant-only expressions.
pub struct EmptySymbols;

impl SymbolSource for EmptySymbols {
    fn value(&self, _name: &str) -> Option<Value> {
        None
    }
}

// ---------------------------------------------------------------- parsing

pub fn parse(text: &str, num: u32) -> Result<Expr, Error> {
    let tokens = scan(text, num)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        num,
        text,
    };
    let expr = p.expression(0)?;
    if p.pos != p.tokens.len() {
        return Err(p.err("trailing input"));
    }
    Ok(expr)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Op(&'static str),
}

fn scan(text: &str, num: u32) -> Result<Vec<Tok>, Error> {
    let mut toks = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            c if c.is_ascii_digit() || c == '.' => {
                let mut end = i;
                while let Some(&(j, n)) = chars.peek() {
                    if n.is_ascii_digit() || n == '.' {
                        end = j + 1;
                        chars.next();
                    } else {
                        break;
                    }
                }
                let s = &text[i..end];
                toks.push(Tok::Num(s.parse().map_err(|_| {
                    Error::at(num, format!("bad number `{s}` in {{ {text} }}"))
                })?));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut end = i;
                while let Some(&(j, n)) = chars.peek() {
                    // Dots occur in namespaced names (`array.boardFuelOutputs`).
                    if n.is_ascii_alphanumeric() || n == '_' || n == '.' {
                        end = j + 1;
                        chars.next();
                    } else {
                        break;
                    }
                }
                toks.push(Tok::Ident(text[i..end].to_string()));
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        Some((_, '"')) => break,
                        Some((_, ch)) => s.push(ch),
                        None => {
                            return Err(Error::at(
                                num,
                                format!("unterminated string in {{ {text} }}"),
                            ));
                        }
                    }
                }
                toks.push(Tok::Str(s));
            }
            _ => {
                chars.next();
                let two = chars.peek().map(|&(_, n)| n);
                let op = match (c, two) {
                    ('&', Some('&')) => Some("&&"),
                    ('|', Some('|')) => Some("||"),
                    ('=', Some('=')) => Some("=="),
                    ('!', Some('=')) => Some("!="),
                    ('<', Some('=')) => Some("<="),
                    ('>', Some('=')) => Some(">="),
                    ('<', Some('<')) => Some("<<"),
                    ('>', Some('>')) => Some(">>"),
                    _ => None,
                };
                if let Some(op) = op {
                    chars.next();
                    toks.push(Tok::Op(op));
                } else {
                    let op = match c {
                        '+' => "+",
                        '-' => "-",
                        '*' => "*",
                        '/' => "/",
                        '%' => "%",
                        '(' => "(",
                        ')' => ")",
                        ',' => ",",
                        '?' => "?",
                        ':' => ":",
                        '<' => "<",
                        '>' => ">",
                        '!' => "!",
                        '&' => "&",
                        '|' => "|",
                        '^' => "^",
                        '[' => "[",
                        ']' => "]",
                        _ => {
                            return Err(Error::at(
                                num,
                                format!("unexpected `{c}` in {{ {text} }}"),
                            ));
                        }
                    };
                    toks.push(Tok::Op(op));
                }
            }
        }
    }
    Ok(toks)
}

struct Parser<'a> {
    tokens: Vec<Tok>,
    pos: usize,
    num: u32,
    text: &'a str,
}

/// Left binding power per binary operator (C-like precedence).
fn lbp(op: &str) -> Option<(u8, BinOp)> {
    Some(match op {
        "||" => (1, BinOp::Or),
        "&&" => (2, BinOp::And),
        "|" => (3, BinOp::BitOr),
        "^" => (4, BinOp::BitXor),
        "&" => (5, BinOp::BitAnd),
        "==" => (6, BinOp::Eq),
        "!=" => (6, BinOp::Ne),
        "<" => (7, BinOp::Lt),
        ">" => (7, BinOp::Gt),
        "<=" => (7, BinOp::Le),
        ">=" => (7, BinOp::Ge),
        "<<" => (8, BinOp::Shl),
        ">>" => (8, BinOp::Shr),
        "+" => (9, BinOp::Add),
        "-" => (9, BinOp::Sub),
        "*" => (10, BinOp::Mul),
        "/" => (10, BinOp::Div),
        "%" => (10, BinOp::Rem),
        _ => return None,
    })
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> Error {
        Error::at(self.num, format!("{msg} in {{ {} }}", self.text))
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect_op(&mut self, op: &str) -> Result<(), Error> {
        match self.next() {
            Some(Tok::Op(o)) if o == op => Ok(()),
            _ => Err(self.err(&format!("expected `{op}`"))),
        }
    }

    fn expression(&mut self, min_bp: u8) -> Result<Expr, Error> {
        let mut lhs = self.primary()?;
        while let Some(Tok::Op(op)) = self.peek() {
            let op = *op;
            // Ternary: lowest precedence, right-associative.
            if op == "?" {
                if min_bp > 0 {
                    break;
                }
                self.next();
                let then = self.expression(0)?;
                self.expect_op(":")?;
                let alt = self.expression(0)?;
                lhs = Expr::Ternary(Box::new(lhs), Box::new(then), Box::new(alt));
                continue;
            }
            let Some((bp, binop)) = lbp(op) else { break };
            if bp < min_bp {
                break;
            }
            self.next();
            let rhs = self.expression(bp + 1)?;
            lhs = Expr::Binary(binop, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn primary(&mut self) -> Result<Expr, Error> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Str(s)) => Ok(Expr::Str(s)),
            Some(Tok::Ident(name)) => {
                if matches!(self.peek(), Some(Tok::Op("["))) {
                    self.next();
                    let idx = self.expression(0)?;
                    self.expect_op("]")?;
                    return Ok(Expr::Index(name, Box::new(idx)));
                }
                if matches!(self.peek(), Some(Tok::Op("("))) {
                    self.next();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Tok::Op(")"))) {
                        loop {
                            args.push(self.expression(0)?);
                            match self.next() {
                                Some(Tok::Op(",")) => continue,
                                Some(Tok::Op(")")) => break,
                                _ => return Err(self.err("expected `,` or `)`")),
                            }
                        }
                    } else {
                        self.next();
                    }
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            Some(Tok::Op("(")) => {
                let e = self.expression(0)?;
                self.expect_op(")")?;
                Ok(e)
            }
            Some(Tok::Op("-")) => Ok(Expr::Unary(UnOp::Neg, Box::new(self.primary()?))),
            Some(Tok::Op("!")) => Ok(Expr::Unary(UnOp::Not, Box::new(self.primary()?))),
            _ => Err(self.err("expected a value")),
        }
    }
}

// ------------------------------------------------------------- evaluation

impl Expr {
    pub fn eval(&self, syms: &dyn SymbolSource) -> Result<Value, Error> {
        match self {
            Expr::Num(n) => Ok(Value::Num(*n)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Ident(name) => syms
                .value(name)
                .ok_or_else(|| Error::eval(format!("unknown identifier `{name}`"))),
            Expr::Unary(op, e) => {
                let v = e.eval(syms)?;
                match op {
                    UnOp::Neg => Ok(Value::Num(-num(&v)?)),
                    UnOp::Not => Ok(Value::Num(if v.truthy() { 0.0 } else { 1.0 })),
                }
            }
            Expr::Binary(op, a, b) => {
                // Short-circuit logic ops before evaluating both sides.
                match op {
                    BinOp::And => {
                        return Ok(Value::Num(
                            (a.eval(syms)?.truthy() && b.eval(syms)?.truthy()) as u8 as f64,
                        ));
                    }
                    BinOp::Or => {
                        return Ok(Value::Num(
                            (a.eval(syms)?.truthy() || b.eval(syms)?.truthy()) as u8 as f64,
                        ));
                    }
                    _ => {}
                }
                let a = a.eval(syms)?;
                let b = b.eval(syms)?;
                if let (Value::Str(x), Value::Str(y)) = (&a, &b) {
                    return match op {
                        BinOp::Eq => Ok(Value::Num((x == y) as u8 as f64)),
                        BinOp::Ne => Ok(Value::Num((x != y) as u8 as f64)),
                        _ => Err(Error::eval("string operands in numeric expression")),
                    };
                }
                let (x, y) = (num(&a)?, num(&b)?);
                let r = match op {
                    BinOp::Add => x + y,
                    BinOp::Sub => x - y,
                    BinOp::Mul => x * y,
                    BinOp::Div => x / y,
                    BinOp::Rem => x % y,
                    BinOp::Lt => (x < y) as u8 as f64,
                    BinOp::Gt => (x > y) as u8 as f64,
                    BinOp::Le => (x <= y) as u8 as f64,
                    BinOp::Ge => (x >= y) as u8 as f64,
                    BinOp::Eq => (x == y) as u8 as f64,
                    BinOp::Ne => (x != y) as u8 as f64,
                    BinOp::BitAnd => ((x as i64) & (y as i64)) as f64,
                    BinOp::BitOr => ((x as i64) | (y as i64)) as f64,
                    BinOp::BitXor => ((x as i64) ^ (y as i64)) as f64,
                    BinOp::Shl => ((x as i64) << (y as i64)) as f64,
                    BinOp::Shr => ((x as i64) >> (y as i64)) as f64,
                    BinOp::And | BinOp::Or => unreachable!(),
                };
                Ok(Value::Num(r))
            }
            Expr::Ternary(c, t, f) => {
                if c.eval(syms)?.truthy() {
                    t.eval(syms)
                } else {
                    f.eval(syms)
                }
            }
            Expr::Index(name, idx) => {
                let idx = num(&idx.eval(syms)?)? as usize;
                syms.array_value(name, idx)
                    .map(Value::Num)
                    .ok_or_else(|| Error::eval(format!("unknown array `{name}[{idx}]`")))
            }
            Expr::Call(name, args) => match name.as_str() {
                "bitStringValue" => {
                    let list = match args.first() {
                        Some(Expr::Ident(l)) => l,
                        _ => return Err(Error::eval("bitStringValue: first arg must be a name")),
                    };
                    let idx = args
                        .get(1)
                        .ok_or_else(|| Error::eval("bitStringValue: missing index"))?
                        .eval(syms)?;
                    let idx = num(&idx)? as usize;
                    syms.bit_string(list, idx)
                        .map(Value::Str)
                        .ok_or_else(|| Error::eval(format!("bitStringValue({list}, {idx})")))
                }
                "arrayValue" => {
                    let arr = match args.first() {
                        Some(Expr::Ident(a)) => a,
                        _ => return Err(Error::eval("arrayValue: first arg must be a name")),
                    };
                    let idx = args
                        .get(1)
                        .ok_or_else(|| Error::eval("arrayValue: missing index"))?
                        .eval(syms)?;
                    let idx = num(&idx)? as usize;
                    syms.array_value(arr, idx)
                        .map(Value::Num)
                        .ok_or_else(|| Error::eval(format!("arrayValue({arr}, {idx})")))
                }
                // Zero-argument calls fall back to identifier lookup
                // (`timeNow` appears both bare and as a call).
                _ if args.is_empty() => syms
                    .value(name)
                    .ok_or_else(|| Error::eval(format!("unknown function `{name}`"))),
                _ => Err(Error::eval(format!("unknown function `{name}`"))),
            },
        }
    }
}

fn num(v: &Value) -> Result<f64, Error> {
    v.as_num()
        .ok_or_else(|| Error::eval("expected a number, got a string"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Map(HashMap<&'static str, f64>);

    impl SymbolSource for Map {
        fn value(&self, name: &str) -> Option<Value> {
            self.0.get(name).copied().map(Value::Num)
        }

        fn bit_string(&self, list: &str, index: usize) -> Option<String> {
            (list == "idleUnits").then(|| format!("unit{index}"))
        }
    }

    fn eval(text: &str, vars: &[(&'static str, f64)]) -> Value {
        let syms = Map(vars.iter().copied().collect());
        parse(text, 1).unwrap().eval(&syms).unwrap()
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(
            eval("(coolantRaw - 40) * 1.8 + 32", &[("coolantRaw", 90.0)]),
            Value::Num(122.0)
        );
        assert_eq!(eval("2 + 3 * 4", &[]), Value::Num(14.0));
        assert_eq!(
            eval("0.1 / stoich", &[("stoich", 14.7)]),
            Value::Num(0.1 / 14.7)
        );
        assert_eq!(eval("-5 + 1", &[]), Value::Num(-4.0));
    }

    #[test]
    fn comparisons_logic_ternary() {
        assert_eq!(
            eval(
                "(iacAlgorithm == 2 || iacAlgorithm == 3 || iacMaxSteps <= 255) ? 1.000 : 2.000",
                &[("iacAlgorithm", 3.0), ("iacMaxSteps", 999.0)]
            ),
            Value::Num(1.0)
        );
        assert_eq!(
            eval(
                "(tps > tpsflood) && (rpm < crankRPM)",
                &[
                    ("tps", 90.0),
                    ("tpsflood", 80.0),
                    ("rpm", 200.0),
                    ("crankRPM", 400.0)
                ]
            ),
            Value::Num(1.0)
        );
        assert_eq!(eval("!0", &[]), Value::Num(1.0));
    }

    #[test]
    fn bit_string_value_call() {
        assert_eq!(
            eval(
                "bitStringValue( idleUnits , iacAlgorithm  )",
                &[("iacAlgorithm", 2.0)]
            ),
            Value::Str("unit2".into())
        );
    }

    #[test]
    fn leading_dot_numbers() {
        assert_eq!(eval(".5 * 4", &[]), Value::Num(2.0));
    }
}
