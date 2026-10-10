# Log-loading benchmark

This standalone executable compares the original server's full-file read/parse/
JSON response with the current progressive-loading implementation. It does not
start a server, connect to an ECU, or modify logs. No RustyTune service is needed.

Each invocation runs one scenario:

- `before`: original handler, all rows and all channels, including JSON encoding.
- `overview`: new handler with empty memory and disk caches and five default
  channels. Removes the generated cache for the supplied log before timing; use
  isolated copies of logs. Timing includes writing the persistent cache.
- `reopen`: new handler after clearing its memory cache, restoring the saved
  index and overview from disk as it would after a server restart.
- `zoom`: a 100–350 second range after initial indexing; includes every sample
  when the range has at most 5,000 valid timestamp rows.
- `cached`: repeat the default overview after it has been cached.

`load_ms` measures file reading, indexing, parsing, and downsampling as applicable.
`serialization_ms` includes JSON response construction/encoding. `total_ms`
measures both. Network transfer, HTTP scheduling, browser work, and concurrent
ECU activity are excluded. Zoom/cache setup is outside request timings but is
included in whole-process peak memory measured by `/usr/bin/time`.

## Build on the Mac for a 64-bit Raspberry Pi OS

From the repository root:

```sh
docker build --platform linux/arm64 \
  -f tools/log-benchmark/Dockerfile --target artifact \
  --output type=local,dest=/tmp/rustytune-log-benchmark-artifact .
```

A 32-bit Raspberry Pi OS requires an ARMv7 build instead; check the target's OS
and build compatibility before copying a binary. ARM64 binaries cannot run on
a 32-bit OS. On a Pi with Rust already available, a native build is also possible:

```sh
cargo build --locked --release --manifest-path tools/log-benchmark/Cargo.toml
```

Copy only the benchmark executable and the two test logs into an isolated
benchmark directory on the Pi. Run the original scenario first, then the new
scenarios, with multiple trials and the same inputs. For example:

```sh
for scenario in before overview reopen zoom cached; do
  for trial in 1 2 3 4 5; do
    /usr/bin/time -v ./rustytune-log-benchmark "$scenario" ./recording.msl
  done
done
```

If GNU `time` is unavailable, the included Python runner measures each scenario
in a fresh child process and records its peak RSS and firmware status:

```sh
python3 run.py --trials 5 ./recording-1.msl ./recording-2.msl > results.jsonl
```

The runner requires Linux and Python 3; peak RSS uses Linux's KiB units. It runs
all original-handler trials first, then the overview, reopened, zoom, and cached
trials. To compare against an earlier two-pass benchmark executable:

```sh
python3 run.py --trials 5 --comparison-binary ./rustytune-log-benchmark-two-pass \
  --scenarios two_pass,overview,reopen,zoom,cached \
  ./recording-1.msl ./recording-2.msl > optimized-results.jsonl
```

The executable emits one JSON record per invocation; `time` reports process
peak RSS separately. Record the Pi model, OS, CPU clock, temperature, throttling
status, and storage. Fresh index caches are not cold OS filesystem caches:
repeated trials will generally use cached file pages. Do not clear system caches
or change CPU settings on a shared Pi solely to improve benchmark consistency.

Pi 3 results are a useful proxy, not predicted Zero 2 W timings. Compare before/
after ratios on the same device, and report actual hardware alongside results.

## Recorded comparison

[Pi 3 B+ results](results/tempi-pi3bplus.md) include five release-build trials per
scenario on both example logs, response sizes, peak RSS, and hardware conditions.

[Initial-load optimization results](results/tempi-pi3bplus-optimized.md) compare
the earlier two-pass implementation with one-pass summaries and persistent
cache reopening on the same Pi.
