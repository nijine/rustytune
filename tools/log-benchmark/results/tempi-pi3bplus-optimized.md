# Pi 3 B+ initial-load optimizations

Measured on 2026-10-10 on `tempi.local`, Raspberry Pi 3 Model B Plus Rev 1.3, 64-bit Debian 12 Bookworm. Release binaries built with Rust 1.88 in Linux ARM64 Bookworm Docker containers. Five trials per log per scenario (50 trials).

This compares the previous progressive-loading implementation (timestamp-index pass followed by an overview pass) against one-pass indexing/overview generation plus persistent caches. It supplements the [original full-load comparison](tempi-pi3bplus.md). Both implementations were measured again on the same Pi in this run; all previous-version trials ran first.

The UI now requests a fixed 1,500-point overview budget. Its width changes resize charts without requesting data again. The standalone benchmark does not measure that avoided browser request, so the timings below only reflect server changes.

**Hardware condition:** intermittent undervoltage/throttling continued. Firmware boundary readings alternated between 600 MHz and 1.4 GHz; 83 of 100 before/after observations reported active throttling (`0x50005`). These are observed timings under those conditions, not guaranteed worst-case bounds or Zero 2 W predictions.

## Request timings

| Log | Scenario | Median | Slowest | Fastest | Response bytes | Plot points |
|---|---|---:|---:|---:|---:|---:|
| `rustytune_20260807_071716.msl` | Previous two-pass initial load | 2880.65 ms | 3020.15 ms | 2122.42 ms | 35,018 | 918 |
| `rustytune_20260807_071716.msl` | New one-pass initial load | 796.06 ms | 816.78 ms | 543.28 ms | 35,081 | 920 |
| `rustytune_20260807_071716.msl` | Reopen from disk cache | 96.83 ms | 97.37 ms | 96.74 ms | 35,081 | 920 |
| `rustytune_20260807_071716.msl` | Full-detail 5,000-row zoom | 93.26 ms | 94.49 ms | 41.37 ms | 173,899 | 5,000 |
| `rustytune_20260807_071716.msl` | In-memory overview | 2.35 ms | 2.46 ms | 2.34 ms | 35,081 | 920 |
| `rustytune_20260807_082651.msl` | Previous two-pass initial load | 1369.40 ms | 1503.99 ms | 1185.79 ms | 33,841 | 890 |
| `rustytune_20260807_082651.msl` | New one-pass initial load | 404.13 ms | 404.42 ms | 227.06 ms | 34,398 | 906 |
| `rustytune_20260807_082651.msl` | Reopen from disk cache | 50.73 ms | 50.75 ms | 22.57 ms | 34,398 | 906 |
| `rustytune_20260807_082651.msl` | Full-detail 5,000-row zoom | 91.57 ms | 91.90 ms | 91.22 ms | 175,354 | 5,000 |
| `rustytune_20260807_082651.msl` | In-memory overview | 2.42 ms | 2.47 ms | 1.10 ms | 34,398 | 906 |

The larger log has 83,779 rows; the smaller has 41,938. Both contain 99 columns. Responses use time plus RPM, MAP, AFR, TPS, and CLT. Byte-sized overview buckets now replace row-sized buckets, which allows extrema/gap-preserving summaries to be generated during the initial scan; summary point counts therefore differ slightly.

## Measurement details

- Initial-load cases start with empty application caches. The new case removes its generated disk cache before each trial, and includes writing the new cache in its reported request time.
- Reopen trials prepare the persistent cache outside the timer, then discard the memory index. The measured request restores the index and overview from disk. No source-log scan is needed.
- Zooms cover 100–350 seconds and return 5,000 samples. Index/cache setup is outside the zoom request timer.
- Every request timing includes JSON response serialization. Network transfer, HTTP scheduling, browser rendering, and concurrent ECU activity are excluded.
- OS filesystem pages are generally warm from copying logs and previous trials. No system caches were dropped and CPU/power settings were not changed. A true reboot with cold SD-card pages can have different timings.
- Cache files were 1,656,898 bytes for the larger log and 819,616 bytes for the smaller log. They contain versioned metadata, row offsets/timestamps, and overview responses. Source files are unchanged.
- Median Linux child-process peak RSS was 10.625 MiB in all cases; as in the earlier report, this includes the Python launcher’s possible pre-exec high-water floor and is not total service memory.

Raw records, including hardware readings, are in [tempi-pi3bplus-optimized.jsonl](tempi-pi3bplus-optimized.jsonl). Benchmark copies and executables were removed from the Pi after results were collected; no RustyTune service was installed.

## Reproducibility

Commands and scenario semantics are documented in [the benchmark README](../README.md). Executable SHA-256 values:

- Previous two-pass binary: `0cdac84ad10f6e58e4f984d7ff50c92d4d461b5eb6442a6f92d54bdfa8482733`.
- Optimized binary: `6bc20a450b46db6991fb6510e537f80cf1068f5253445c4d2e05c529c899c7db`.

The previous binary is the retained artifact from the 2026-10-09 benchmark. The optimized binary compiles the server’s `log_view.rs` directly; a later cleanup-loop lint fix does not change indexing or plotting behavior.
