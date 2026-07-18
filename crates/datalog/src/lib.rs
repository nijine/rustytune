//! MegaLogViewer `.msl` datalog output (and, later, reading for the log
//! viewer). Tab-separated, CRLF line endings, three header lines (title /
//! column labels / units), columns and formats taken from the INI
//! `[Datalog]` section. Rows are flushed per write and fsync'd periodically
//! so an ignition-off power cut loses at most about a second of data.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ts_ini::{IniDef, OutputChannel, SymbolSource, Value};

/// FSYNC_INTERVAL_MS in the C logger.
const FSYNC_INTERVAL: Duration = Duration::from_secs(1);

/// How one cell is rendered, from the INI's C-style format string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellFormat {
    /// `"%d"` — rounded integer.
    Int,
    /// `"%.3f"` — fixed decimals.
    Fixed(u8),
}

impl CellFormat {
    /// Parse `"%d"` / `"%.1f"`; anything unrecognized logs as 3-decimal float.
    pub fn parse(s: &str) -> CellFormat {
        let s = s.trim();
        if s.ends_with('d') {
            return CellFormat::Int;
        }
        if let Some(frac) = s.strip_prefix("%.")
            && let Some(digits) = frac.strip_suffix('f')
            && let Ok(n) = digits.parse::<u8>()
        {
            return CellFormat::Fixed(n);
        }
        CellFormat::Fixed(3)
    }

    pub fn render(&self, v: f64) -> String {
        match self {
            CellFormat::Int => format!("{}", v.round() as i64),
            CellFormat::Fixed(p) => format!("{:.*}", *p as usize, v),
        }
    }
}

/// One `.msl` column: a [Datalog] entry resolved against the definition.
#[derive(Debug, Clone)]
pub struct Column {
    /// Output channel supplying the value.
    pub channel: String,
    /// Column header label.
    pub label: String,
    /// Units line text (may be empty).
    pub units: String,
    pub format: CellFormat,
}

/// Build the column set for a new log from the INI [Datalog] section.
///
/// Entries whose channel isn't an output channel are dropped. Entries with a
/// condition (`{ flexEnabled }`) are included when it evaluates truthy against
/// `syms` — or when evaluation fails, since an extra column is cheaper than a
/// silently missing one.
pub fn columns(def: &IniDef, syms: &dyn SymbolSource) -> Vec<Column> {
    def.datalog
        .iter()
        .filter(|entry| def.output_channels.contains_key(&entry.channel))
        .filter(|entry| match &entry.condition {
            None => true,
            Some(cond) => match cond.eval(syms) {
                Ok(Value::Num(n)) => n != 0.0,
                Ok(Value::Str(_)) | Err(_) => true,
            },
        })
        .map(|entry| Column {
            channel: entry.channel.clone(),
            label: entry.label.clone(),
            units: channel_units(def, &entry.channel, syms),
            format: CellFormat::parse(&entry.format),
        })
        .collect()
}

fn channel_units(def: &IniDef, channel: &str, syms: &dyn SymbolSource) -> String {
    let units = match def.output_channels.get(channel) {
        Some(OutputChannel::Scalar { units, .. }) => units.as_ref(),
        Some(OutputChannel::Derived { units, .. }) => {
            return units.clone().unwrap_or_default();
        }
        _ => None,
    };
    units
        .and_then(|u| u.eval(syms).ok())
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

pub struct MslWriter {
    file: File,
    path: PathBuf,
    columns: Vec<Column>,
    rows: u64,
    last_sync: Instant,
}

impl MslWriter {
    /// Create the file and write the three-line MSL header.
    pub fn create(path: &Path, title: &str, columns: Vec<Column>) -> io::Result<Self> {
        let mut file = File::create(path)?;
        let labels: Vec<&str> = columns.iter().map(|c| c.label.as_str()).collect();
        let units: Vec<&str> = columns.iter().map(|c| c.units.as_str()).collect();
        write!(
            file,
            "{title}\r\n{}\r\n{}\r\n",
            labels.join("\t"),
            units.join("\t")
        )?;
        Ok(MslWriter {
            file,
            path: path.to_path_buf(),
            columns,
            rows: 0,
            last_sync: Instant::now(),
        })
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// Write one row; `values` aligns with `columns()`, `None` leaves a cell
    /// empty (channel failed to decode this frame).
    pub fn write_row(&mut self, values: &[Option<f64>]) -> io::Result<()> {
        debug_assert_eq!(values.len(), self.columns.len());
        let mut line = String::new();
        for (i, (v, col)) in values.iter().zip(&self.columns).enumerate() {
            if i > 0 {
                line.push('\t');
            }
            if let Some(v) = v {
                line.push_str(&col.format.render(*v));
            }
        }
        line.push_str("\r\n");
        self.file.write_all(line.as_bytes())?;
        self.rows += 1;

        if self.last_sync.elapsed() >= FSYNC_INTERVAL {
            self.last_sync = Instant::now();
            self.file.sync_data()?;
        }
        Ok(())
    }

    /// Flush and fsync; call when logging stops.
    pub fn finish(mut self) -> io::Result<PathBuf> {
        self.file.flush()?;
        self.file.sync_all()?;
        Ok(self.path)
    }
}

/// A parsed .msl file, column-major. `None` cells were empty or
/// non-numeric.
pub struct MslData {
    pub title: String,
    pub labels: Vec<String>,
    pub units: Vec<String>,
    pub rows: usize,
    pub columns: Vec<Vec<Option<f64>>>,
}

/// Parse MSL text (three header lines, then tab-separated rows). Also
/// accepts files written by TunerStudio/MegaLogViewer — short rows pad
/// with `None`, "MARK" annotation lines are skipped.
pub fn read_msl(text: &str) -> Result<MslData, String> {
    let mut lines = text.split(['\n']).map(|l| l.trim_end_matches('\r'));
    let title = lines.next().ok_or("empty file")?.to_string();
    let labels: Vec<String> = lines
        .next()
        .ok_or("missing label line")?
        .split('\t')
        .map(str::to_string)
        .collect();
    if labels.len() < 2 {
        return Err("not an MSL file (no tab-separated labels)".into());
    }
    let units: Vec<String> = lines
        .next()
        .ok_or("missing units line")?
        .split('\t')
        .map(str::to_string)
        .collect();

    let mut columns: Vec<Vec<Option<f64>>> = vec![Vec::new(); labels.len()];
    let mut rows = 0usize;
    for line in lines {
        if line.is_empty() || line.starts_with("MARK") {
            continue;
        }
        let mut cells = line.split('\t');
        for col in &mut columns {
            col.push(cells.next().and_then(|c| c.trim().parse::<f64>().ok()));
        }
        rows += 1;
    }
    Ok(MslData {
        title,
        labels,
        units,
        rows,
        columns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parse_and_render() {
        assert_eq!(CellFormat::parse("%d"), CellFormat::Int);
        assert_eq!(CellFormat::parse("%.3f"), CellFormat::Fixed(3));
        assert_eq!(CellFormat::parse("%.1f"), CellFormat::Fixed(1));
        assert_eq!(CellFormat::parse("junk"), CellFormat::Fixed(3));

        assert_eq!(CellFormat::Int.render(14.6), "15");
        assert_eq!(CellFormat::Int.render(-2.4), "-2");
        assert_eq!(CellFormat::Fixed(1).render(22.04), "22.0");
        assert_eq!(CellFormat::Fixed(3).render(14.7), "14.700");
    }

    #[test]
    fn msl_layout() {
        let dir = std::env::temp_dir().join(format!("rustytune-msl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.msl");

        let cols = vec![
            Column {
                channel: "time".into(),
                label: "Time".into(),
                units: "sec".into(),
                format: CellFormat::Fixed(3),
            },
            Column {
                channel: "rpm".into(),
                label: "RPM".into(),
                units: "RPM".into(),
                format: CellFormat::Int,
            },
            Column {
                channel: "afr".into(),
                label: "AFR".into(),
                units: String::new(),
                format: CellFormat::Fixed(3),
            },
        ];
        let mut w = MslWriter::create(&path, "\"rustytune test log\"", cols).unwrap();
        w.write_row(&[Some(0.016), Some(3450.0), Some(14.7)])
            .unwrap();
        w.write_row(&[Some(0.066), Some(3455.0), None]).unwrap();
        assert_eq!(w.rows(), 2);
        let path = w.finish().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "\"rustytune test log\"\r\n\
             Time\tRPM\tAFR\r\n\
             sec\tRPM\t\r\n\
             0.016\t3450\t14.700\r\n\
             0.066\t3455\t\r\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();

        // The reader round-trips what the writer produced.
        let data = read_msl(&text).unwrap();
        assert_eq!(data.title, "\"rustytune test log\"");
        assert_eq!(data.labels, ["Time", "RPM", "AFR"]);
        assert_eq!(data.units, ["sec", "RPM", ""]);
        assert_eq!(data.rows, 2);
        assert_eq!(data.columns[0], [Some(0.016), Some(0.066)]);
        assert_eq!(data.columns[1], [Some(3450.0), Some(3455.0)]);
        assert_eq!(data.columns[2], [Some(14.7), None]);
    }

    #[test]
    fn read_msl_tolerates_marks_and_short_rows() {
        let text = "\"t\"\r\nTime\tRPM\r\nsec\tRPM\r\n\
                    0.1\t900\r\nMARK 001 something\r\n0.2\r\n";
        let data = read_msl(text).unwrap();
        assert_eq!(data.rows, 2);
        assert_eq!(data.columns[1], [Some(900.0), None]);

        assert!(read_msl("just some text\nno tabs\n").is_err());
    }
}
