//! Bounded plot responses with one-pass overviews and persistent row indexes.
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, Metadata, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    rc::Rc,
    time::SystemTime,
};

const FULL_DETAIL_ROWS: usize = 5_000;
const CACHE_VERSION: u32 = 1;

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Query {
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub channels: Option<String>,
    pub points: Option<usize>,
}

#[derive(Serialize, Deserialize)]
pub struct Index {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    title: String,
    labels: Vec<String>,
    units: Vec<String>,
    rows: usize,
    entries: Vec<(u64, f64)>,
    #[serde(serialize_with = "serialize_overviews")]
    views: Vec<(Query, View)>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct View {
    pub name: String,
    pub title: String,
    pub labels: Vec<String>,
    pub units: Vec<String>,
    pub rows: usize,
    pub visible_rows: usize,
    pub sampled: bool,
    pub range: Option<[f64; 2]>,
    pub column_labels: Vec<String>,
    pub columns: Vec<Vec<Option<f64>>>,
}

#[derive(Serialize, Deserialize)]
struct DiskCache {
    version: u32,
    index: Index,
}

#[derive(Serialize)]
struct DiskCacheRef<'a> {
    version: u32,
    index: &'a Index,
}

struct Sample {
    row: usize,
    values: Vec<Option<f64>>,
}

/// Only endpoints, extrema, and one missing-value marker per channel survive.
/// Byte-sized buckets can be determined before scanning, without a row-count pass.
struct Bucket {
    first: Rc<Sample>,
    last: Rc<Sample>,
    low: Vec<Option<Rc<Sample>>>,
    high: Vec<Option<Rc<Sample>>>,
    gaps: Vec<Option<Rc<Sample>>>,
}

impl Bucket {
    fn new(sample: Rc<Sample>) -> Self {
        let n = sample.values.len();
        let mut bucket = Self {
            first: sample.clone(),
            last: sample.clone(),
            low: vec![None; n],
            high: vec![None; n],
            gaps: vec![None; n],
        };
        bucket.add(sample);
        bucket
    }

    fn add(&mut self, sample: Rc<Sample>) {
        self.last = sample.clone();
        for c in 1..sample.values.len() {
            match sample.values[c] {
                Some(v) => {
                    if self.low[c]
                        .as_ref()
                        .is_none_or(|s| v < s.values[c].unwrap())
                    {
                        self.low[c] = Some(sample.clone());
                    }
                    if self.high[c]
                        .as_ref()
                        .is_none_or(|s| v > s.values[c].unwrap())
                    {
                        self.high[c] = Some(sample.clone());
                    }
                }
                None if self.gaps[c].is_none() => self.gaps[c] = Some(sample.clone()),
                None => {}
            }
        }
    }

    fn finish(self, columns: &mut [Vec<Option<f64>>]) {
        let mut keep = vec![self.first, self.last];
        keep.extend(self.low.into_iter().flatten());
        keep.extend(self.high.into_iter().flatten());
        keep.extend(self.gaps.into_iter().flatten());
        keep.sort_unstable_by_key(|s| s.row);
        keep.dedup_by_key(|s| s.row);
        for sample in keep {
            for (col, v) in columns.iter_mut().zip(&sample.values) {
                col.push(*v);
            }
        }
    }
}

fn selected_columns(labels: &[String], query: &Query) -> Vec<usize> {
    let requested: Vec<&str> = query
        .channels
        .as_deref()
        .unwrap_or("RPM,MAP,AFR,TPS,CLT")
        .split(',')
        .collect();
    let mut selected = vec![0];
    selected.extend(
        labels
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, l)| requested.contains(&l.as_str()))
            .map(|(i, _)| i),
    );
    selected
}

fn parse_columns(row: &str, selected: &[usize], time: f64) -> Vec<Option<f64>> {
    let mut cells = row.split('\t');
    cells.next();
    let mut previous = 0;
    let mut values = Vec::with_capacity(selected.len());
    values.push(Some(time));
    for ci in selected.iter().skip(1) {
        let value = cells
            .nth(ci - previous - 1)
            .and_then(|c| c.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite());
        values.push(value);
        previous = *ci;
    }
    values
}

fn serialize_overviews<S: serde::Serializer>(
    views: &[(Query, View)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    views
        .iter()
        .filter(|(q, _)| q.start.is_none() && q.end.is_none())
        .collect::<Vec<_>>()
        .serialize(serializer)
}

fn cache_path(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or(Path::new("."))
        .join(".rustytune-cache")
        .join(format!(
            "{}.json",
            path.file_name().unwrap().to_string_lossy()
        ))
}

impl Index {
    fn matches(&self, path: &Path, meta: &Metadata) -> bool {
        self.path == path
            && self.size == meta.len()
            && self.modified.is_some()
            && self.modified == meta.modified().ok()
    }

    fn restore(path: &Path, meta: &Metadata) -> Option<Self> {
        let saved: DiskCache =
            serde_json::from_slice(&std::fs::read(cache_path(path)).ok()?).ok()?;
        let index = saved.index;
        if saved.version != CACHE_VERSION
            || !index.matches(path, meta)
            || index.labels.len() < 2
            || index.entries.len() > index.rows
            || index
                .entries
                .iter()
                .any(|(offset, t)| *offset >= index.size || !t.is_finite())
            || index.entries.windows(2).any(|w| w[0].0 >= w[1].0)
            || index.views.len() > 4
            || index.views.iter().any(|(q, v)| {
                q.start.is_some()
                    || q.end.is_some()
                    || v.labels != index.labels
                    || v.units != index.units
                    || v.rows != index.rows
                    || v.visible_rows != index.entries.len()
                    || v.columns.len() != v.column_labels.len()
                    || v.columns.is_empty()
                    || v.columns.iter().any(|c| c.len() != v.columns[0].len())
                    || v.columns.iter().flatten().flatten().any(|v| !v.is_finite())
            })
        {
            return None;
        }
        Some(index)
    }

    fn persist(&self) {
        // A read-only log folder or a failed cache write must never prevent viewing.
        // Do not save an index if the recording grew while it was being scanned.
        let result = (|| -> std::io::Result<()> {
            if !self.matches(&self.path, &std::fs::metadata(&self.path)?) {
                return Ok(());
            }
            let destination = cache_path(&self.path);
            let directory = destination.parent().unwrap();
            std::fs::create_dir_all(directory)?;
            let suffix = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let temporary = directory.join(format!(".{}-{suffix}.tmp", std::process::id()));
            let write_result = (|| -> std::io::Result<()> {
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                let mut writer = BufWriter::with_capacity(128 * 1024, file);
                serde_json::to_writer(
                    &mut writer,
                    &DiskCacheRef {
                        version: CACHE_VERSION,
                        index: self,
                    },
                )?;
                writer.flush()?;
                std::fs::rename(&temporary, &destination)
            })();
            if write_result.is_err() {
                let _ = std::fs::remove_file(&temporary);
            }
            write_result?;
            // Retention/deletion of source logs should not leave orphaned indexes.
            if let Ok(files) = std::fs::read_dir(directory) {
                for file in files.flatten() {
                    let name = file.file_name();
                    let Some(log) = name.to_str().and_then(|n| n.strip_suffix(".msl.json")) else {
                        continue;
                    };
                    if !self
                        .path
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(format!("{log}.msl"))
                        .exists()
                    {
                        let _ = std::fs::remove_file(file.path());
                    }
                }
            }
            Ok(())
        })();
        let _ = result;
    }

    fn build(path: PathBuf, name: String, query: &Query) -> Result<Self, String> {
        let file = File::open(&path).map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        // Capture a bounded snapshot of actively growing recordings.
        let mut reader = BufReader::with_capacity(128 * 1024, file.take(meta.len()));
        let mut line = String::new();
        let mut headers = Vec::new();
        let mut position = 0;
        for _ in 0..3 {
            line.clear();
            let bytes = reader.read_line(&mut line).map_err(|e| e.to_string())?;
            if bytes == 0 {
                return Err("missing MSL header".into());
            }
            position += bytes as u64;
            headers.push(line.trim_end_matches(['\r', '\n']).to_owned());
        }
        let labels: Vec<String> = headers[1].split('\t').map(str::to_owned).collect();
        if labels.len() < 2 {
            return Err("not an MSL file".into());
        }
        let selected = selected_columns(&labels, query);
        let data_start = position;
        let bucket_count =
            (query.points.unwrap_or(1500).clamp(100, 5000) / (2 + 3 * (selected.len() - 1))).max(1);
        let bucket_bytes = meta
            .len()
            .saturating_sub(data_start)
            .div_ceil(bucket_count as u64)
            .max(1);
        let mut bucket: Option<Bucket> = None;
        let mut bucket_id = 0;
        let mut overview = vec![Vec::new(); selected.len()];
        let mut full = vec![Vec::new(); selected.len()];
        let mut entries = Vec::new();
        let mut rows = 0;
        loop {
            let offset = position;
            line.clear();
            let bytes = reader.read_line(&mut line).map_err(|e| e.to_string())?;
            if bytes == 0 {
                break;
            }
            position += bytes as u64;
            let row = line.trim_end_matches(['\r', '\n']);
            if row.is_empty() || row.starts_with("MARK") {
                continue;
            }
            rows += 1;
            let Some(t) = row
                .split('\t')
                .next()
                .and_then(|c| c.trim().parse::<f64>().ok())
                .filter(|t| t.is_finite())
            else {
                continue;
            };
            let values = parse_columns(row, &selected, t);
            entries.push((offset, t));
            if entries.len() <= FULL_DETAIL_ROWS {
                for (column, v) in full.iter_mut().zip(&values) {
                    column.push(*v);
                }
            } else if entries.len() == FULL_DETAIL_ROWS + 1 {
                full.clear();
            }
            let id = (offset - data_start) / bucket_bytes;
            if id != bucket_id {
                if let Some(previous) = bucket.take() {
                    previous.finish(&mut overview);
                }
                bucket_id = id;
            }
            let sample = Rc::new(Sample {
                row: entries.len() - 1,
                values,
            });
            match &mut bucket {
                Some(b) => b.add(sample),
                None => bucket = Some(Bucket::new(sample)),
            }
        }
        if let Some(b) = bucket {
            b.finish(&mut overview);
        }
        let mut index = Self {
            path,
            size: meta.len(),
            modified: meta.modified().ok(),
            title: headers[0].clone(),
            labels,
            units: headers[2].split('\t').map(str::to_owned).collect(),
            rows,
            entries,
            views: Vec::new(),
        };
        let columns = if index.entries.len() <= FULL_DETAIL_ROWS {
            full
        } else {
            overview
        };
        let mut overview_query = query.clone();
        overview_query.start = None;
        overview_query.end = None;
        let view = index.view(name, &selected, index.entries.len(), columns);
        index.views.push((overview_query, view));
        index.persist();
        Ok(index)
    }

    fn view(
        &self,
        name: String,
        selected: &[usize],
        visible_rows: usize,
        columns: Vec<Vec<Option<f64>>>,
    ) -> View {
        let range = self
            .entries
            .iter()
            .map(|(_, t)| *t)
            .fold(None, |acc: Option<[f64; 2]>, t| {
                Some(acc.map_or([t, t], |[a, b]| [a.min(t), b.max(t)]))
            });
        View {
            name,
            title: self.title.clone(),
            labels: self.labels.clone(),
            units: self.units.clone(),
            rows: self.rows,
            visible_rows,
            sampled: visible_rows > FULL_DETAIL_ROWS,
            range,
            column_labels: selected.iter().map(|i| self.labels[*i].clone()).collect(),
            columns,
        }
    }
}

pub fn load(
    cache: &mut Option<Index>,
    path: PathBuf,
    name: String,
    mut query: Query,
) -> Result<View, String> {
    query.points = Some(query.points.unwrap_or(1500).clamp(100, 5000));
    query
        .channels
        .get_or_insert_with(|| "RPM,MAP,AFR,TPS,CLT".to_owned());
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if !cache.as_ref().is_some_and(|i| i.matches(&path, &meta)) {
        *cache = Some(match Index::restore(&path, &meta) {
            Some(index) => index,
            None => Index::build(path.clone(), name.clone(), &query)?,
        });
    }
    let index = cache.as_mut().unwrap();
    if let Some((_, view)) = index.views.iter().find(|(q, _)| q == &query) {
        let mut view = view.clone();
        view.name = name;
        return Ok(view);
    }
    let selected = selected_columns(&index.labels, &query);
    let entries: Vec<_> = index
        .entries
        .iter()
        .enumerate()
        .filter(|(_, (_, t))| {
            query.start.is_none_or(|s| *t >= s) && query.end.is_none_or(|e| *t <= e)
        })
        .collect();
    let sampled = entries.len() > FULL_DETAIL_ROWS;
    let budget = query.points.unwrap_or(1500).clamp(100, 5000);
    let bucket_size = if sampled {
        entries
            .len()
            .div_ceil((budget / (2 + 3 * (selected.len() - 1))).max(1))
    } else {
        1
    };
    let mut reader = BufReader::with_capacity(
        128 * 1024,
        File::open(&index.path).map_err(|e| e.to_string())?,
    );
    let mut line = String::new();
    let mut columns = vec![Vec::new(); selected.len()];
    let mut position = 0;
    for group in entries.chunks(bucket_size) {
        let mut bucket = None;
        for (row, (offset, time)) in group {
            if position != *offset {
                reader
                    .seek(SeekFrom::Start(*offset))
                    .map_err(|e| e.to_string())?;
            }
            line.clear();
            position = *offset + reader.read_line(&mut line).map_err(|e| e.to_string())? as u64;
            let values = parse_columns(line.trim_end_matches(['\r', '\n']), &selected, *time);
            let sample = Rc::new(Sample { row: *row, values });
            match &mut bucket {
                Some(b) => Bucket::add(b, sample),
                None => bucket = Some(Bucket::new(sample)),
            }
        }
        if let Some(b) = bucket {
            b.finish(&mut columns);
        }
    }
    let view = index.view(name, &selected, entries.len(), columns);
    // Prefer overview views in the persistent cache; zooms only need a memory cache.
    if index.views.len() == 4 {
        let evict = index
            .views
            .iter()
            .position(|(q, _)| q.start.is_some() || q.end.is_some())
            .unwrap_or(0);
        index.views.remove(evict);
    }
    index.views.push((query.clone(), view.clone()));
    if query.start.is_none() && query.end.is_none() {
        index.persist();
    }
    Ok(view)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn overview_preserves_extrema_and_zoom_returns_exact_rows() {
        let directory = tempfile::tempdir().unwrap();
        let mut file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        writeln!(file, "test\nTime\tRPM\tAFR\n s\trpm\t").unwrap();
        for i in 0..6000 {
            writeln!(
                file,
                "{}\t{}\t14",
                i,
                if i == 1234 {
                    9000
                } else if i == 3456 {
                    -100
                } else {
                    1000
                }
            )
            .unwrap();
        }
        let mut cache = None;
        let overview = load(
            &mut cache,
            file.path().into(),
            "test.msl".into(),
            Query {
                channels: Some("RPM".into()),
                points: Some(100),
                ..Query::default()
            },
        )
        .unwrap();
        assert_eq!(overview.rows, 6000);
        assert!(overview.sampled);
        assert!(overview.columns[0].len() <= 100);
        assert!(overview.columns[1].contains(&Some(9000.0)));
        assert!(overview.columns[1].contains(&Some(-100.0)));
        assert_eq!(overview.column_labels, ["Time", "RPM"]);
        let zoom = load(
            &mut cache,
            file.path().into(),
            "test.msl".into(),
            Query {
                start: Some(1000.0),
                end: Some(5999.0),
                channels: Some("AFR".into()),
                ..Query::default()
            },
        )
        .unwrap();
        assert!(!zoom.sampled);
        assert_eq!(zoom.visible_rows, 5000);
        assert_eq!(zoom.columns[0].len(), 5000);
        assert_eq!(zoom.columns[0][0], Some(1000.0));
        assert_eq!(zoom.column_labels, ["Time", "AFR"]);
        writeln!(file, "6000\t2000\t15").unwrap();
        let changed = load(
            &mut cache,
            file.path().into(),
            "test.msl".into(),
            Query::default(),
        )
        .unwrap();
        assert_eq!(changed.rows, 6001);
        assert_eq!(changed.range, Some([0.0, 6000.0]));
        let empty = load(
            &mut cache,
            file.path().into(),
            "test.msl".into(),
            Query {
                start: Some(7000.0),
                ..Query::default()
            },
        )
        .unwrap();
        assert_eq!(empty.visible_rows, 0);
    }

    #[test]
    fn malformed_times_marks_and_gaps() {
        let directory = tempfile::tempdir().unwrap();
        let mut file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        write!(
            file,
            "test\r\nTime\tRPM\r\ns\trpm\r\nMARK event\r\nbad\t3\r\nNaN\t4\r\n0\t1\r\n1\t\r\n\r\n"
        )
        .unwrap();
        let view = load(
            &mut None,
            file.path().into(),
            "test.msl".into(),
            Query::default(),
        )
        .unwrap();
        assert_eq!(view.rows, 4);
        assert_eq!(view.visible_rows, 2);
        assert_eq!(view.columns[1], [Some(1.0), None]);
    }

    #[test]
    fn persistent_cache_survives_restart_and_channel_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recording.msl");
        std::fs::write(
            &path,
            "test\nTime\tRPM\tAFR\ns\trpm\t\n0\t1000\t14\n1\t2000\t15\n",
        )
        .unwrap();
        let mut cache = None;
        let initial = load(
            &mut cache,
            path.clone(),
            "recording.msl".into(),
            Query::default(),
        )
        .unwrap();
        assert!(cache_path(&path).is_file());
        assert!(Index::restore(&path, &std::fs::metadata(&path).unwrap()).is_some());
        let restarted = load(
            &mut None,
            path.clone(),
            "recording.msl".into(),
            Query::default(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&initial).unwrap(),
            serde_json::to_value(&restarted).unwrap()
        );
        let other = Query {
            channels: Some("AFR".into()),
            ..Query::default()
        };
        let expected = load(
            &mut cache,
            path.clone(),
            "recording.msl".into(),
            other.clone(),
        )
        .unwrap();
        let restarted = load(&mut None, path.clone(), "recording.msl".into(), other).unwrap();
        assert_eq!(restarted.columns, expected.columns);
        assert_eq!(restarted.column_labels, ["Time", "AFR"]);
        // Same length, different contents: modification time must invalidate it.
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(
            &path,
            "test\nTime\tRPM\tAFR\ns\trpm\t\n0\t3000\t14\n1\t4000\t15\n",
        )
        .unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(modified + std::time::Duration::from_secs(1)),
            )
            .unwrap();
        let changed = load(
            &mut cache,
            path.clone(),
            "recording.msl".into(),
            Query::default(),
        )
        .unwrap();
        assert_eq!(changed.columns[1], [Some(3000.0), Some(4000.0)]);
        let changed = load(&mut None, path, "recording.msl".into(), Query::default()).unwrap();
        assert_eq!(changed.columns[1], [Some(3000.0), Some(4000.0)]);
    }

    #[test]
    fn corrupt_cache_and_unwritable_cache_directory_fall_back_to_scan() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recording.msl");
        std::fs::write(&path, "test\nTime\tRPM\ns\trpm\n0\t1000\n").unwrap();
        let query = Query::default();
        load(
            &mut None,
            path.clone(),
            "recording.msl".into(),
            query.clone(),
        )
        .unwrap();
        std::fs::write(cache_path(&path), b"broken JSON").unwrap();
        let recovered = load(
            &mut None,
            path.clone(),
            "recording.msl".into(),
            query.clone(),
        )
        .unwrap();
        assert_eq!(recovered.columns[1], [Some(1000.0)]);
        // Reject structurally invalid, but syntactically valid cache data too.
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(cache_path(&path)).unwrap()).unwrap();
        saved["index"]["views"][0][1]["columns"] = serde_json::json!([]);
        std::fs::write(cache_path(&path), serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(
            load(
                &mut None,
                path.clone(),
                "recording.msl".into(),
                query.clone()
            )
            .unwrap()
            .columns[1],
            [Some(1000.0)]
        );
        std::fs::remove_file(cache_path(&path)).unwrap();
        std::fs::remove_dir(cache_path(&path).parent().unwrap()).unwrap();
        std::fs::write(
            directory.path().join(".rustytune-cache"),
            b"not a directory",
        )
        .unwrap();
        let uncached = load(&mut None, path, "recording.msl".into(), query).unwrap();
        assert_eq!(uncached.columns[1], [Some(1000.0)]);
    }

    #[test]
    fn one_pass_buckets_preserve_irregular_rows_extrema_and_gaps() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recording.msl");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "test\nTime\tRPM\tAFR\tIgnored\ns\trpm\t\t").unwrap();
        for i in 0..10_000 {
            let rpm = if i == 1234 {
                "9000"
            } else if i == 3456 {
                "-100"
            } else if i == 999 {
                ""
            } else {
                "1000"
            };
            let afr = if i == 5432 {
                "19"
            } else if i == 6789 {
                "10"
            } else {
                "14"
            };
            writeln!(
                file,
                "{}\t{}\t{}\t{}",
                i,
                rpm,
                afr,
                "x".repeat(if i < 5000 { 1 } else { 300 })
            )
            .unwrap();
        }
        let query = Query {
            channels: Some("RPM,AFR".into()),
            points: Some(100),
            ..Query::default()
        };
        let overview = load(
            &mut None,
            path.clone(),
            "recording.msl".into(),
            query.clone(),
        )
        .unwrap();
        assert!(overview.sampled);
        assert!(overview.columns[0].len() <= 100);
        assert_eq!(overview.columns[0].first(), Some(&Some(0.0)));
        assert_eq!(overview.columns[0].last(), Some(&Some(9999.0)));
        assert!(overview.columns[1].contains(&Some(9000.0)));
        assert!(overview.columns[1].contains(&Some(-100.0)));
        assert!(overview.columns[1].contains(&None));
        assert!(overview.columns[2].contains(&Some(19.0)));
        assert!(overview.columns[2].contains(&Some(10.0)));
        assert!(overview.columns[0].windows(2).all(|w| w[0] < w[1]));
        let restarted = load(&mut None, path, "recording.msl".into(), query).unwrap();
        assert_eq!(restarted.columns, overview.columns);
    }

    // Optional local benchmark; does not copy personal logs into the repository.
    #[test]
    #[ignore]
    fn benchmark_local_log() {
        let path = PathBuf::from(std::env::var("RUSTYTUNE_BENCH_LOG").unwrap());
        let mut cache = None;
        let started = std::time::Instant::now();
        let view = load(
            &mut cache,
            path.clone(),
            "benchmark.msl".into(),
            Query::default(),
        )
        .unwrap();
        eprintln!(
            "initial: {:?}, rows: {}, points: {}, JSON bytes: {}",
            started.elapsed(),
            view.rows,
            view.columns[0].len(),
            serde_json::to_vec(&view).unwrap().len()
        );
        let started = std::time::Instant::now();
        let view = load(
            &mut cache,
            path,
            "benchmark.msl".into(),
            Query {
                start: Some(100.0),
                end: Some(350.0),
                ..Query::default()
            },
        )
        .unwrap();
        eprintln!(
            "zoom: {:?}, visible rows: {}, sampled: {}",
            started.elapsed(),
            view.visible_rows,
            view.sampled
        );
    }
}
