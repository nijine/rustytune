//! AFR table generation and preset recognition; no display-only offsets.
use serde::{Deserialize, Serialize};
use ts_ini::{IniDef, SymbolSource, Value};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub name: String,
    pub volts_low: f64,
    pub afr_low: f64,
    pub volts_high: f64,
    pub afr_high: f64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Linear {
    pub volts_low: f64,
    pub afr_low: f64,
    pub volts_high: f64,
    pub afr_high: f64,
    pub preset: Option<String>,
}
impl Linear {
    pub fn table(&self, def: &IniDef) -> Result<Vec<u8>, String> {
        let cal = metadata(def)?;
        if ![self.volts_low, self.volts_high, self.afr_low, self.afr_high]
            .iter()
            .all(|v| v.is_finite())
            || self.volts_low < 0.0
            || self.volts_high > 5.0
            || self.volts_low >= self.volts_high
            || self.afr_low <= 0.0
            || self.afr_high <= self.afr_low
        {
            return Err(
                "Use increasing voltage points within 0–5 V and positive, increasing AFR points"
                    .into(),
            );
        }
        if let Some(name) = &self.preset {
            return presets(def)
                .into_iter()
                .find(|(p, _)| &p.name == name)
                .map(|(_, table)| table)
                .ok_or_else(|| "Unknown calibration preset".into());
        }
        let slope = (self.afr_high - self.afr_low) / (self.volts_high - self.volts_low);
        (0..cal.adc_count)
            .map(|adc| {
                let afr = self.afr_low
                    + (adc as f64 * 5.0 / (cal.adc_count - 1) as f64 - self.volts_low) * slope;
                encode(afr, cal.scale)
            })
            .collect()
    }
}
fn encode(afr: f64, scale: f64) -> Result<u8, String> {
    let value = (afr * scale).round();
    if !value.is_finite() || !(0.0..=255.0).contains(&value) {
        return Err("Curve exceeds the ECU AFR range at 0 or 5 V".into());
    }
    Ok(value as u8)
}
pub fn metadata(def: &IniDef) -> Result<&ts_ini::AfrCalibration, String> {
    let c = def
        .afr_calibration
        .as_ref()
        .ok_or("INI has no AFR calibration definition")?;
    if c.identifier != 2
        || c.adc_count != 1024
        || c.bytes_per_adc != 1
        || c.scale != 10.0
        || c.blocking_factor == 0
        || c.blocking_factor > 256
        || !c.blocking_factor.is_multiple_of(32)
        || c.write_command != r"t\$tsCanId%2i%2o%2c%v"
        || def.header.table_crc_command.as_deref() != Some(r"k\$tsCanId%2i%2o%2c")
    {
        return Err("Unsupported AFR calibration definition".into());
    }
    Ok(c)
}
struct Adc(f64);
impl SymbolSource for Adc {
    fn value(&self, name: &str) -> Option<Value> {
        (name == "adcValue").then_some(Value::Num(self.0))
    }
}
pub fn presets(def: &IniDef) -> Vec<(Preset, Vec<u8>)> {
    let Ok(cal) = metadata(def) else {
        return vec![];
    };
    let mut out = Vec::new();
    for (name, expr) in &cal.solutions {
        let table: Result<Vec<u8>, String> = (0..cal.adc_count)
            .map(|adc| match expr.eval(&Adc(adc as f64)) {
                Ok(Value::Num(v)) => encode(v, cal.scale),
                _ => Err("Unavailable curve".into()),
            })
            .collect();
        if let Ok(table) = table {
            let number = |adc| match expr.eval(&Adc(adc)) {
                Ok(Value::Num(v)) => v,
                _ => 0.0,
            };
            out.push((
                Preset {
                    name: name.clone(),
                    volts_low: 0.0,
                    afr_low: number(0.0),
                    volts_high: 5.0,
                    afr_high: number(1023.0),
                },
                table,
            ));
        }
    }
    for (name, lo, al, hi, ah) in [
        ("AEM X-Series 30-0300", 0.5, 8.5, 4.5, 18.0),
        (
            "AEM Classic 30-4110 (0–5 V / 10–20 AFR)",
            0.0,
            10.0,
            5.0,
            20.0,
        ),
    ] {
        let linear = Linear {
            volts_low: lo,
            afr_low: al,
            volts_high: hi,
            afr_high: ah,
            preset: None,
        };
        if let Ok(table) = linear.table(def) {
            out.insert(
                0,
                (
                    Preset {
                        name: name.into(),
                        volts_low: lo,
                        afr_low: al,
                        volts_high: hi,
                        afr_high: ah,
                    },
                    table,
                ),
            );
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aem_curve_and_invalid_endpoints() {
        let def = ts_ini::parse(include_str!("../../../fixtures/speeduino202501_7.ini")).unwrap();
        let mut curve = Linear {
            volts_low: 0.5,
            afr_low: 8.5,
            volts_high: 4.5,
            afr_high: 18.0,
            preset: None,
        };
        let table = curve.table(&def).unwrap();
        assert_eq!(table.len(), 1024);
        assert_eq!((table[0], table[512], table[1023]), (73, 133, 192));
        curve.volts_high = 0.5;
        assert!(curve.table(&def).is_err());
        assert!(
            presets(&def)
                .iter()
                .any(|(p, _)| p.name == "AEM X-Series 30-0300")
        );
    }
}
