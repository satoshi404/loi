#!/usr/bin/env python3
"""Build and time paired Loi/C standard-library benchmarks.

Each executable is run once as a correctness check and then nine timed runs.
The reported number is the minimum of those nine runs. All runs are pinned to
one CPU when taskset is available so the heterogeneous ARM machine does not
silently move a benchmark between Cortex-A55 and Cortex-A75 cores.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "benchmark"
NAMES = ["hash", "crc32", "sort", "int_map", "str_map", "random"]
RUNS = 9
PIN_CPU = "6"


def command_output(args: list[str]) -> str:
    return subprocess.check_output(args, text=True).strip()


def run_once(executable: Path) -> tuple[float, str]:
    command = [str(executable)]
    if shutil.which("taskset"):
        command = ["taskset", "-c", PIN_CPU, str(executable)]
    start = time.perf_counter_ns()
    result = subprocess.run(command, capture_output=True, text=True, check=True)
    elapsed = (time.perf_counter_ns() - start) / 1_000_000_000.0
    return elapsed, result.stdout


def build_all(build_dir: Path) -> dict[str, dict[str, Path]]:
    loi = ROOT / "target" / "debug" / "loi"
    if not loi.is_file():
        raise SystemExit(f"missing compiler; run cargo build first: {loi}")
    cc = os.environ.get("CC", "gcc")
    binaries: dict[str, dict[str, Path]] = {}

    for name in NAMES:
        loi_bin = build_dir / f"loi-{name}"
        subprocess.run(
            [
                str(loi),
                "build",
                str(BENCH / f"{name}.loi"),
                "-o",
                str(loi_bin),
                "--release",
                "--rebuild",
                "--arena-size",
                "16777216",
            ],
            check=True,
            env={**os.environ, "CC": cc},
            stdout=subprocess.DEVNULL,
        )
        c_bin = build_dir / f"c-{name}"
        subprocess.run(
            [
                cc,
                "-std=c11",
                "-O2",
                str(BENCH / f"{name}_ref.c"),
                "-o",
                str(c_bin),
                "-lm",
            ],
            check=True,
        )
        binaries[name] = {"loi": loi_bin, "c": c_bin}
    return binaries


def metadata(loi: Path) -> dict[str, object]:
    lscpu = command_output(["lscpu"])
    models = sorted(
        {line.split(":", 1)[1].strip() for line in lscpu.splitlines()
         if line.startswith("Model name:")}
    )
    return {
        "architecture": command_output(["uname", "-m"]),
        "cpu_models": models,
        "cpus": os.cpu_count(),
        "pinned_to_cpu": PIN_CPU if shutil.which("taskset") else None,
        "loi": command_output([str(loi), "--version"]).splitlines()[0],
        "cc": command_output([os.environ.get("CC", "gcc"), "--version"]).splitlines()[0],
        "build_flags": {
            "loi": "--release --arena-size 16777216",
            "c": "-std=c11 -O2",
        },
        "runs_per_executable": RUNS,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--build-dir",
        type=Path,
        default=Path("/tmp/opencode/loi-bench"),
        help="directory for generated binaries",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="write measured results as JSON to this path",
    )
    args = parser.parse_args()
    args.build_dir.mkdir(parents=True, exist_ok=True)

    binaries = build_all(args.build_dir)
    loi = ROOT / "target" / "debug" / "loi"
    info = metadata(loi)
    results: dict[str, object] = {
        "metadata": info,
        "benchmarks": {},
    }

    print(f"pinned CPU: {info['pinned_to_cpu']}")
    print(f"{'benchmark':<10} {'loi min (s)':>12} {'c min (s)':>12} {'ratio':>8}")
    for name in NAMES:
        loi_times: list[float] = []
        c_times: list[float] = []
        _, loi_output = run_once(binaries[name]["loi"])
        _, c_output = run_once(binaries[name]["c"])
        if loi_output != c_output:
            raise SystemExit(
                f"{name}: output mismatch\nLoi: {loi_output!r}\nC:   {c_output!r}"
            )
        for _ in range(RUNS):
            seconds, output = run_once(binaries[name]["loi"])
            if output != loi_output:
                raise SystemExit(f"{name}: nondeterministic Loi output: {output!r}")
            loi_times.append(seconds)
            seconds, output = run_once(binaries[name]["c"])
            if output != c_output:
                raise SystemExit(f"{name}: nondeterministic C output: {output!r}")
            c_times.append(seconds)
        loi_min = min(loi_times)
        c_min = min(c_times)
        ratio = loi_min / c_min
        print(f"{name:<10} {loi_min:>12.6f} {c_min:>12.6f} {ratio:>8.3f}")
        results["benchmarks"][name] = {
            "output": loi_output.strip(),
            "loi_seconds": loi_times,
            "c_seconds": c_times,
            "loi_min": loi_min,
            "c_min": c_min,
            "loi_over_c": ratio,
        }

    if args.output:
        args.output.write_text(json.dumps(results, indent=2) + "\n")
        print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
