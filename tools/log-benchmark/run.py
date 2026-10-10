#!/usr/bin/env python3
"""Repeated Linux trials with per-process peak RSS, without GNU time."""
import argparse
import json
import resource
import subprocess
import sys
from pathlib import Path


def environment():
    values = {}
    for command in ("measure_temp", "get_throttled", "measure_clock arm"):
        try:
            result = subprocess.run(["vcgencmd", *command.split()], capture_output=True, text=True, check=True)
            values[command] = result.stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            pass
    return values


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--one":
        # A fresh wrapper has only one child, making RUSAGE_CHILDREN peak RSS
        # specific to this trial rather than the highest of previous trials.
        result = subprocess.run(sys.argv[2:], capture_output=True, text=True, check=True)
        record = json.loads(result.stdout)
        record["peak_rss_kib"] = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        print(json.dumps(record))
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+", type=Path)
    parser.add_argument("--binary", default="./rustytune-log-benchmark")
    parser.add_argument("--trials", type=int, default=5)
    parser.add_argument("--scenarios", default="before,overview,reopen,zoom,cached")
    parser.add_argument("--comparison-binary", help="Previous two-pass benchmark executable")
    args = parser.parse_args()
    scenarios = args.scenarios.split(",")
    if any(s not in ("before", "two_pass", "overview", "reopen", "zoom", "cached") for s in scenarios):
        parser.error("unknown scenario")
    if "two_pass" in scenarios and not args.comparison_binary:
        parser.error("two_pass requires --comparison-binary")
    for scenario in scenarios:
        for log in args.logs:
            for trial in range(1, args.trials + 1):
                before = environment()
                binary = args.comparison_binary if scenario == "two_pass" else args.binary
                mode = "overview" if scenario == "two_pass" else scenario
                result = subprocess.run([sys.executable, __file__, "--one", binary, mode, str(log)], capture_output=True, text=True, check=True)
                record = json.loads(result.stdout)
                record.update(scenario=scenario, trial=trial, environment_before=before, environment_after=environment())
                print(json.dumps(record), flush=True)


if __name__ == "__main__":
    main()
