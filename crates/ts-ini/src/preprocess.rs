//! Preprocessor pass: comment stripping, `#set`/`#unset`, `#define` with
//! `$name` expansion, and `#if`/`#elif`/`#else`/`#endif` conditionals driven
//! by a symbol set (CELSIUS, LAMBDA, mcu_*, COMMS_COMPAT, ... — in
//! TunerStudio these come from project settings / [SettingGroups]).

use std::collections::{HashMap, HashSet};

use crate::Error;

/// One surviving source line after preprocessing.
#[derive(Debug, Clone)]
pub struct Line {
    /// 1-based line number in the original file, for error messages.
    pub num: u32,
    pub text: String,
}

#[derive(Clone, Copy)]
struct IfFrame {
    /// This branch is being emitted.
    active: bool,
    /// Some earlier branch of this if-chain was taken (suppresses elif/else).
    taken: bool,
    /// The enclosing context was active.
    parent_active: bool,
}

pub fn preprocess(src: &str, symbols: &mut HashSet<String>) -> Result<Vec<Line>, Error> {
    let mut defines: HashMap<String, String> = HashMap::new();
    let mut stack: Vec<IfFrame> = Vec::new();
    let mut out = Vec::new();

    for (idx, raw) in src.lines().enumerate() {
        let num = (idx + 1) as u32;
        let line = strip_comment(raw);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let active = stack.iter().all(|f| f.active);

        if let Some(directive) = trimmed.strip_prefix('#') {
            let (word, rest) = split_word(directive);
            match word {
                "if" => {
                    let parent_active = active;
                    let cond = parent_active && eval_condition(rest, symbols, num)?;
                    stack.push(IfFrame {
                        active: cond,
                        taken: cond,
                        parent_active,
                    });
                }
                "elif" => {
                    let frame = stack
                        .last_mut()
                        .ok_or_else(|| Error::at(num, "#elif without #if"))?;
                    if frame.taken {
                        frame.active = false;
                    } else {
                        frame.active = frame.parent_active && eval_condition(rest, symbols, num)?;
                        frame.taken = frame.active;
                    }
                }
                "else" => {
                    let frame = stack
                        .last_mut()
                        .ok_or_else(|| Error::at(num, "#else without #if"))?;
                    frame.active = frame.parent_active && !frame.taken;
                    frame.taken = true;
                }
                "endif" => {
                    stack
                        .pop()
                        .ok_or_else(|| Error::at(num, "#endif without #if"))?;
                }
                "set" if active => {
                    symbols.insert(rest.trim().to_string());
                }
                "unset" if active => {
                    symbols.remove(rest.trim());
                }
                "define" if active => {
                    let (name, body) = rest
                        .split_once('=')
                        .ok_or_else(|| Error::at(num, "#define without '='"))?;
                    // Bodies may reference earlier defines; expand now so
                    // later use is a single substitution.
                    let body = expand(body.trim(), &defines, num)?;
                    defines.insert(name.trim().to_string(), body);
                }
                "set" | "unset" | "define" => {} // inside a false branch
                other => {
                    return Err(Error::at(num, format!("unknown directive #{other}")));
                }
            }
            continue;
        }

        if active {
            out.push(Line {
                num,
                text: expand(trimmed, &defines, num)?,
            });
        }
    }

    if !stack.is_empty() {
        return Err(Error::at(0, "unterminated #if"));
    }
    Ok(out)
}

/// `#if X` / `#if !X`: a bare symbol name, true when present in the set.
fn eval_condition(cond: &str, symbols: &HashSet<String>, num: u32) -> Result<bool, Error> {
    let cond = cond.trim();
    if let Some(name) = cond.strip_prefix('!') {
        return Ok(!symbols.contains(name.trim()));
    }
    if cond.is_empty() {
        return Err(Error::at(num, "#if with empty condition"));
    }
    Ok(symbols.contains(cond))
}

/// Truncate at the first `;` that is outside double quotes.
fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            ';' if !in_quote => return &line[..i],
            _ => {}
        }
    }
    line
}

fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    match s.find(|c: char| c.is_whitespace()) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

/// Replace each `$name` (not preceded by `\` — `"\$tsCanId"` is a runtime
/// variable reference inside command strings, not a define use) with its
/// define body.
fn expand(text: &str, defines: &HashMap<String, String>, num: u32) -> Result<String, Error> {
    if !text.contains('$') {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut prev = '\0';
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '$' && prev != '\\' {
            let start = i + 1;
            let mut end = start;
            while let Some(&(j, n)) = chars.peek() {
                if n.is_ascii_alphanumeric() || n == '_' {
                    end = j + n.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let name = &text[start..end];
            if name.is_empty() {
                out.push('$');
            } else {
                let body = defines
                    .get(name)
                    .ok_or_else(|| Error::at(num, format!("undefined ${name}")))?;
                out.push_str(body);
            }
            prev = '\0';
        } else {
            out.push(c);
            prev = c;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, syms: &[&str]) -> Vec<String> {
        let mut symbols: HashSet<String> = syms.iter().map(|s| s.to_string()).collect();
        preprocess(src, &mut symbols)
            .unwrap()
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    #[test]
    fn strips_comments_and_blank_lines() {
        assert_eq!(
            run("a = 1 ; comment\n\n; whole line\nb = \"x;y\", 2", &[]),
            vec!["a = 1", "b = \"x;y\", 2"]
        );
    }

    #[test]
    fn if_else_endif() {
        let src = "#if CELSIUS\na = C\n#else\na = F\n#endif";
        assert_eq!(run(src, &["CELSIUS"]), vec!["a = C"]);
        assert_eq!(run(src, &[]), vec!["a = F"]);
    }

    #[test]
    fn elif_chain_and_nesting() {
        let src = "#if A\nx\n#elif B\ny\n#if C\nz\n#endif\n#else\nw\n#endif";
        assert_eq!(run(src, &["B", "C"]), vec!["y", "z"]);
        assert_eq!(run(src, &["B"]), vec!["y"]);
        assert_eq!(run(src, &["A", "B", "C"]), vec!["x"]);
        assert_eq!(run(src, &[]), vec!["w"]);
    }

    #[test]
    fn set_unset_only_when_active() {
        let src = "#if NO\n#set X\n#endif\n#if X\nbad\n#endif\n#set Y\n#if Y\ngood\n#endif";
        assert_eq!(run(src, &[]), vec!["good"]);
    }

    #[test]
    fn nested_defines_expand() {
        let src = "#define a8 = \"I\", \"I\"\n#define a16 = $a8, $a8\nlist = $a16";
        assert_eq!(run(src, &[]), vec!["list = \"I\", \"I\", \"I\", \"I\""]);
    }

    #[test]
    fn escaped_dollar_untouched() {
        let src = "#define tsCanId = BOOM\ncmd = \"r\\$tsCanId\\x30\"";
        assert_eq!(run(src, &[]), vec!["cmd = \"r\\$tsCanId\\x30\""]);
    }
}
