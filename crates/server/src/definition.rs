//! The UI-facing slice of the parsed INI: front-page gauges with their
//! bounds/zones resolved to numbers, and indicator lamp definitions.
//!
//! Gauge bounds are INI expressions over PcVariables and tune constants
//! (`hi = {rpmhigh}`); until a live tune is downloaded (Phase 5) they are
//! evaluated against `[DefaultValues]`, which covers the stock front page.

use std::collections::HashMap;

use serde::Serialize;
use ts_ini::{GaugeDef, IniDef, NumOrExpr, SymbolSource, Value};

/// `[DefaultValues]` as a symbol source (PcVariables like `rpmhigh`, tune
/// constants like `stoich`). Non-numeric defaults are skipped.
pub struct Defaults {
    values: HashMap<String, f64>,
}

impl Defaults {
    pub fn from_ini(def: &IniDef) -> Self {
        let values = def
            .default_values
            .iter()
            .filter_map(|(name, text)| Some((name.clone(), text.trim().parse::<f64>().ok()?)))
            .collect();
        Defaults { values }
    }
}

impl SymbolSource for Defaults {
    fn value(&self, name: &str) -> Option<Value> {
        self.values.get(name).copied().map(Value::Num)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GaugeUi {
    pub name: String,
    pub channel: String,
    pub title: String,
    pub units: String,
    pub lo: f64,
    pub hi: f64,
    pub lo_danger: f64,
    pub lo_warn: f64,
    pub hi_warn: f64,
    pub hi_danger: f64,
    pub value_digits: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorUi {
    pub off_label: String,
    pub on_label: String,
    pub off_bg: String,
    pub off_fg: String,
    pub on_bg: String,
    pub on_fg: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionUi {
    pub signature: String,
    pub gauges: Vec<GaugeUi>,
    pub indicators: Vec<IndicatorUi>,
}

fn eval_or(field: &NumOrExpr, syms: &dyn SymbolSource, fallback: f64) -> f64 {
    field.eval(syms).unwrap_or(fallback)
}

fn gauge_ui(gauge: &GaugeDef, syms: &dyn SymbolSource) -> GaugeUi {
    let lo = eval_or(&gauge.lo, syms, 0.0);
    let hi = eval_or(&gauge.hi, syms, 100.0);
    GaugeUi {
        name: gauge.name.clone(),
        channel: gauge.channel.clone(),
        title: gauge.title.clone(),
        units: gauge.units.clone(),
        lo,
        hi,
        // A zone whose bound fails to resolve collapses to zero width.
        lo_danger: eval_or(&gauge.lo_danger, syms, lo),
        lo_warn: eval_or(&gauge.lo_warn, syms, lo),
        hi_warn: eval_or(&gauge.hi_warn, syms, hi),
        hi_danger: eval_or(&gauge.hi_danger, syms, hi),
        value_digits: gauge.value_digits,
    }
}

pub fn definition_ui(def: &IniDef, defaults: &Defaults) -> DefinitionUi {
    let gauges = def
        .front_page
        .gauges
        .iter()
        .filter_map(|name| {
            let gauge = def.gauges.get(name);
            if gauge.is_none() {
                tracing::warn!("front page references unknown gauge `{name}`");
            }
            gauge
        })
        .map(|g| gauge_ui(g, defaults))
        .collect();

    let indicators = def
        .front_page
        .indicators
        .iter()
        .map(|ind| {
            let color = |i: usize, fallback: &str| {
                ind.colors
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| fallback.to_string())
            };
            IndicatorUi {
                off_label: ind.off_label.clone(),
                on_label: ind.on_label.clone(),
                off_bg: color(0, "white"),
                off_fg: color(1, "black"),
                on_bg: color(2, "green"),
                on_fg: color(3, "black"),
            }
        })
        .collect();

    DefinitionUi {
        signature: def.signature.clone(),
        gauges,
        indicators,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> IniDef {
        let src = include_str!("../../../fixtures/speeduino202405_dev.ini");
        ts_ini::parse(src).unwrap()
    }

    #[test]
    fn front_page_gauges_resolve_against_defaults() {
        let def = fixture();
        let ui = definition_ui(&def, &Defaults::from_ini(&def));

        assert_eq!(ui.signature, "speeduino 202405-dev");
        assert_eq!(ui.gauges.len(), 8, "fixture front page has 8 gauges");

        let tach = &ui.gauges[0];
        assert_eq!(tach.name, "tachometer");
        assert_eq!(tach.channel, "rpm");
        // hi = {rpmhigh}, hiWarn = {rpmwarn}, hiDanger = {rpmdang} — all from
        // [DefaultValues].
        assert_eq!(tach.lo, 0.0);
        assert_eq!(tach.hi, 8000.0);
        assert_eq!(tach.hi_warn, 3000.0);
        assert_eq!(tach.hi_danger, 5000.0);

        assert!(!ui.indicators.is_empty());
        let running = &ui.indicators[0];
        assert_eq!(running.off_label, "Not Running");
        assert_eq!(running.on_bg, "green");
    }
}
