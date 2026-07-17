//! Parser for TunerStudio ECU-definition INI files (the Speeduino subset).
//!
//! The format is INI-shaped but not INI: values are comma-separated token
//! lists, there is a `#define`/`#if` preprocessor, and many fields may be
//! `{ expression }` blocks evaluated against other constants. Parsing runs
//! a preprocessor pass (driven by a caller-supplied symbol set — CELSIUS,
//! LAMBDA, mcu_*, ... — matching TunerStudio project settings), then typed
//! per-section parsers over the surviving lines.
//!
//! Sections outside the tuning MVP ([Menu], [UserDefined], [VeAnalyze],
//! [LoggerDefinition], ...) are skipped, not parsed.

use std::collections::HashSet;

pub mod expr;
pub mod lex;
pub mod model;
mod preprocess;
mod sections;

pub use expr::{Expr, SymbolSource, Value};
pub use lex::Shape;
pub use model::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("line {0}: {1}")]
    Parse(u32, String),
    #[error("{0}")]
    Eval(String),
}

impl Error {
    pub(crate) fn at(num: u32, msg: impl std::fmt::Display) -> Self {
        Error::Parse(num, msg.to_string())
    }

    pub(crate) fn eval(msg: impl std::fmt::Display) -> Self {
        Error::Eval(msg.to_string())
    }
}

/// Parse with an empty symbol set (all `#if` symbols false — TunerStudio's
/// "DEFAULT" project settings, Fahrenheit units, AFR display).
pub fn parse(src: &str) -> Result<IniDef, Error> {
    parse_with_symbols(src, &HashSet::new())
}

/// Parse with the given preprocessor symbols set (e.g. `CELSIUS`, `LAMBDA`,
/// `mcu_stm32`, `COMMS_COMPAT` — see [SettingGroups] options).
pub fn parse_with_symbols(src: &str, symbols: &HashSet<String>) -> Result<IniDef, Error> {
    let mut symbols = symbols.clone();
    let lines = preprocess::preprocess(src, &mut symbols)?;

    // Group lines per section, preserving order of first appearance.
    let mut section = String::new();
    let mut groups: Vec<(String, Vec<preprocess::Line>)> = Vec::new();
    for line in lines {
        let text = line.text.trim();
        if let Some(name) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            section = name.trim().to_string();
            groups.push((section.clone(), Vec::new()));
            continue;
        }
        if let Some((name, group)) = groups.last_mut()
            && *name == section
        {
            group.push(line);
        }
        // Lines before any section header are file-level directives handled
        // by the preprocessor; anything else is ignored.
    }

    let mut def = IniDef::default();
    let mut ctx = sections::Ctx { def: &mut def };
    for (name, lines) in &groups {
        match name.as_str() {
            "MegaTune" => sections::megatune(lines, &mut ctx)?,
            "SettingGroups" => sections::setting_groups(lines, &mut ctx)?,
            "PcVariables" => sections::pc_variables(lines, &mut ctx)?,
            "Constants" => sections::constants(lines, &mut ctx)?,
            "ConstantsExtensions" => sections::constants_extensions(lines, &mut ctx)?,
            "OutputChannels" => sections::output_channels(lines, &mut ctx)?,
            "TableEditor" => sections::table_editor(lines, &mut ctx)?,
            "CurveEditor" => sections::curve_editor(lines, &mut ctx)?,
            "GaugeConfigurations" => sections::gauge_configurations(lines, &mut ctx)?,
            "FrontPage" => sections::front_page(lines, &mut ctx)?,
            "Datalog" => sections::datalog(lines, &mut ctx)?,
            // Known sections we deliberately skip in the MVP.
            _ => {}
        }
    }
    Ok(def)
}
