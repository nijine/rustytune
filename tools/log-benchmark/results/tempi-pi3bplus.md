# Pi 3 B+ before/after log benchmark

Measured on 2026-10-09 on `tempi.local`: Raspberry Pi 3 Model B Plus Rev 1.3, 64-bit Debian 12 Bookworm, release binary built with Rust 1.88 in the ARM64 Bookworm Docker image. Kernel `6.6.62+rpt-rpi-v8`, CPU governor `ondemand`. Five trials per file per scenario (40 total).

Baseline: original full-file `/data` handler at commit `2bde84f6a546e71598f2365fbb2d9402a8109e12`; the standalone executable reproduces its file read, unchanged `datalog::read_msl` parser, JSON object construction, and response serialization. The new cases compile the current `crates/server/src/log_view.rs` directly. All baseline trials ran before the new cases. No appliance service was installed or started.

**Hardware condition:** the Pi reported intermittent active undervoltage/throttling (`0x50005`), and CPU readings alternated between 600 MHz and 1.4 GHz. Temperature was approximately 38–41 °C. Ratios below describe this run; they are not a controlled maximum-performance comparison or a prediction for a Zero 2 W. See [Raspberry Pi flag definitions](https://github.com/RaspberryPiFoundation/documentation-pages-source-test/blob/master/raspbian/applications/vcgencmd.md#get_throttled).

## Measured request timings

Times include reading/indexing/parsing/downsampling and JSON response generation. Medians are over five trials; the range gives the fastest and slowest trial. Network transfer, HTTP scheduling, browser rendering, and concurrent ECU activity are excluded. File pages were generally warm from upload/prior trials; “initial overview” means an empty application index cache, not a cold filesystem cache.

| Log | Scenario | Median time | Trial range | Response bytes | Plot points | Median process peak RSS |
|---|---|---:|---:|---:|---:|---:|
| `rustytune_20260807_071716.msl` | Original full response | 7086.80 ms | 5938.49–7913.62 ms | 38,469,893 | 83,779 | 435.9 MiB |
| `rustytune_20260807_071716.msl` | Initial overview | 3002.04 ms | 1291.33–3012.35 ms | 35,018 | 918 | 10.5 MiB |
| `rustytune_20260807_071716.msl` | 5,000-row zoom | 82.91 ms | 82.39–190.68 ms | 173,899 | 5,000 | 10.5 MiB |
| `rustytune_20260807_071716.msl` | Cached overview | 2.42 ms | 1.04–2.56 ms | 35,018 | 918 | 10.5 MiB |
| `rustytune_20260807_082651.msl` | Original full response | 3749.67 ms | 3333.39–4496.17 ms | 19,239,622 | 41,938 | 221.3 MiB |
| `rustytune_20260807_082651.msl` | Initial overview | 1134.37 ms | 645.06–1508.33 ms | 33,841 | 890 | 10.5 MiB |
| `rustytune_20260807_082651.msl` | 5,000-row zoom | 191.01 ms | 82.58–193.43 ms | 175,354 | 5,000 | 10.5 MiB |
| `rustytune_20260807_082651.msl` | Cached overview | 2.30 ms | 1.01–2.40 ms | 33,841 | 890 | 10.5 MiB |

## Initial-load differences

- `rustytune_20260807_071716.msl`: 2.36× faster by median request time, 1099× smaller response (99.909% reduction).
- `rustytune_20260807_082651.msl`: 3.31× faster by median request time, 569× smaller response (99.824% reduction).

The original responses contain all 99 columns; the new responses contain time plus RPM, MAP, AFR, TPS, and CLT. Overviews retain per-bucket extrema and gap markers. Zoom requests cover 100–350 seconds and return exactly 5,000 rows for both logs.

Zoom and cached trials build an overview before starting their request timers. That setup is still included in process peak RSS. The original implementation zooms client-side after its full response has loaded, so its initial-load timing is the baseline comparison; the new zoom timing is an additional lazy request, not a direct old/new zoom speedup.

Peak RSS is measured per fresh Linux child process by the Python runner. The child inherits Python launch overhead before exec, so the approximately 10.5 MiB new-case measurements may include a launch floor; they are not measurements of a running service’s total memory. The original larger-log case peaks at approximately 436 MiB even without the service/browser overhead.

Raw trial records, including before/after firmware flags and clock readings, are in [tempi-pi3bplus.jsonl](tempi-pi3bplus.jsonl). Logs themselves are not stored in this repository.
