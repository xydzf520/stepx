#!/usr/bin/env python3
"""Independent release-only homepage performance gate (no display/phone required).

CI: python3 desktop/scripts/check-home-performance.py --output artifacts/home-performance.json
Same-host comparison: add --baseline artifacts/previous-home-performance.json
Quick validator regression checks: --self-test

Compilation ends before measurement. One app is reused per process. Linux startup
work must settle; idle workers stay alive, and /proc CPU ticks/state are checked
before and after sampling (kernel accounting resolution, not proof of zero work).
Three fresh, single-test processes collect
80 warm samples for each of 30 cases. The median across process medians and P95s
must BOTH remain below the original 15 ms budget. A supplied compatible baseline
also gates regressions >25% AND >250 us, avoiding timer noise on very short cases.
This measures View construction/destruction, not painting, FPS or phone latency.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import tempfile
import time

TEST = "ui::pages::home::tests::home_release_frame_budget_regression"
PREFIX = "STEPX_HOME_PERF_JSON "
PRESETS = {"DefaultAll", "WorkFocus", "LifeAndMessages", "MinimalPulse", "CompleteAll"}
VIEWPORTS = {(1000, 650), (1440, 900), (1920, 1080)}
FIXTURES = {"empty-v1", "populated-v1"}
BUDGET_US = 15_000.0
REGRESSION_RATIO = 1.25
REGRESSION_FLOOR_US = 250.0
ISOLATION_PROTOCOL = "one-companion-idle-workers-v1"
FIXTURE_ENVIRONMENT = "temporary-state-offline-helpers-v1"


def key(row):
    return (row["fixture"], row["width"], row["height"], row["preset"])


def validate_probe(probe):
    if probe.get("schema") != 2 or probe.get("debug_assertions") is not False:
        raise ValueError("Expected schema 2 and a release probe; a debug/zero-test run cannot pass")
    if probe.get("background_quiesced") is not True:
        raise ValueError("Benchmark startup/background work has not been verified quiescent")
    isolation = probe.get("isolation", {})
    if (isolation.get("protocol") != ISOLATION_PROTOCOL
            or isolation.get("settle_ms", 0) < 2600
            or not isinstance(isolation.get("workers_before"), dict)
            or isolation["workers_before"] != isolation.get("workers_after")):
        raise ValueError("Missing or invalid idle-worker isolation evidence")
    for worker in isolation["workers_before"].values():
        if (worker.get("state") != "S" or worker.get("wait") != "futex_do_wait"
                or not isinstance(worker.get("cpu_ticks"), int) or worker["cpu_ticks"] < 0):
            raise ValueError("A background worker was active during the measurement window")
    if probe.get("metric") != "home_view_tree_build_and_drop_us":
        raise ValueError("Incompatible metric")
    cases = probe.get("cases", [])
    expected = {(f, w, h, p) for f in FIXTURES for w, h in VIEWPORTS for p in PRESETS}
    if len(cases) != len(expected) or {key(r) for r in cases} != expected:
        raise ValueError("Missing or duplicate fixture/viewport/preset cases")
    for row in cases:
        if row.get("samples") != 80 or row.get("warmup") != 20:
            raise ValueError("Sample/warmup protocol changed")
        for metric in ("first_us", "median_us", "p95_us", "mean_us", "min_us", "max_us"):
            value = row.get(metric)
            if not isinstance(value, (float, int)) or not math.isfinite(value) or value <= 0:
                raise ValueError(f"Invalid {metric}: {value}")
        if not row["min_us"] <= row["median_us"] <= row["p95_us"] <= row["max_us"]:
            raise ValueError("Invalid percentile ordering")
    return {key(row): row for row in cases}


def aggregate(probes):
    validated = [validate_probe(p) for p in probes]
    if len(validated) < 3:
        raise ValueError("At least three isolated process runs are required")
    rows = []
    for identity in sorted(validated[0]):
        source = [run[identity] for run in validated]
        row = {k: source[0][k] for k in ("fixture", "width", "height", "preset")}
        for metric in ("first_us", "median_us", "p95_us", "mean_us"):
            row[metric] = statistics.median(s[metric] for s in source)
        row["worst_run_p95_us"] = max(s["p95_us"] for s in source)
        rows.append(row)
    return rows


def gate(rows, baseline=None):
    failures = []
    comparisons = []
    previous = {key(r): r for r in baseline["cases"]} if baseline else {}
    if baseline and set(previous) != {key(r) for r in rows}:
        raise ValueError("Baseline case matrix differs; do not compare different fixtures")
    for row in rows:
        for metric in ("median_us", "p95_us"):
            value = row[metric]
            if value >= BUDGET_US:
                failures.append(f"{key(row)} {metric}={value:.1f} exceeded {BUDGET_US:.0f} us")
            if previous:
                old = previous[key(row)][metric]
                if not isinstance(old, (float, int)) or not math.isfinite(old) or old <= 0:
                    raise ValueError("Invalid baseline timing")
                comparison = {"case": list(key(row)), "metric": metric,
                              "previous_us": old, "current_us": value,
                              "change_percent": (value / old - 1) * 100}
                comparisons.append(comparison)
                if value > old * REGRESSION_RATIO and value - old > REGRESSION_FLOOR_US:
                    failures.append(f"{key(row)} {metric} regressed {old:.1f} → {value:.1f} us")
    return failures, comparisons


def command_text(args, cwd):
    return subprocess.check_output(args, cwd=cwd, text=True, stderr=subprocess.DEVNULL).strip()


def host_signature(repo):
    cpu = platform.processor()
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.exists():
        cpu = next((line.partition(":")[2].strip() for line in cpuinfo.read_text().splitlines()
                    if line.startswith("model name")), cpu)
    return {"system": platform.system(), "machine": platform.machine(), "cpu": cpu,
            "logical_cpus": os.cpu_count(), "node": platform.node(),
            "rustc": command_text(["rustc", "--version"], repo)}


def build_probe(repo, log):
    command = ["cargo", "test", "--locked", "--release", "--manifest-path",
               str(repo / "desktop/Cargo.toml"), "--bin", "stepx-companion", "--no-run",
               "--message-format=json"]
    print("Compiling release test binary (not timed)…", flush=True)
    result = subprocess.run(command, cwd=repo, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE)
    log.write_text(result.stdout + "\n" + result.stderr)
    if result.returncode:
        raise RuntimeError(f"Release compilation failed; see {log}\n{result.stderr[-5000:]}")
    binaries = []
    for line in result.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (message.get("reason") == "compiler-artifact" and message.get("executable")
                and message.get("target", {}).get("name") == "stepx-companion"
                and message.get("profile", {}).get("test")):
            binaries.append(Path(message["executable"]))
    if len(set(binaries)) != 1:
        raise RuntimeError(f"Expected one release test executable; found {binaries}")
    return binaries[0]


def run_probe(binary, repo, log):
    # SearchState opens its SQLite worker during construction. Keep it off the
    # user's live database. App-store icon prefetch uses Python; that unrelated
    # network helper must not turn a View-only fixture into a download benchmark.
    # No normal worker or production thread pool is terminated for measurement.
    with tempfile.TemporaryDirectory(prefix="stepx-home-perf-") as scratch:
        env = os.environ.copy()
        env["STEPX_STATE_DIR"] = str(Path(scratch) / "state")
        env["STEPX_PYTHON"] = str(Path(scratch) / "disabled-external-helper")
        for name in ("STEPX_STORE_QUERY", "STEPX_STORE_SUB_TAG", "STEPX_STORE_DETAIL",
                     "STEPX_STORE_CATEGORY", "STEPX_PAGE", "STEPX_TOOL"):
            env.pop(name, None)
        result = subprocess.run([str(binary), TEST, "--exact", "--test-threads=1", "--nocapture"],
                                cwd=repo, env=env, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT)
    log.write_text(result.stdout)
    if result.returncode:
        raise RuntimeError(f"Probe failed; see {log}\n{result.stdout[-3000:]}")
    lines = [line.split(PREFIX, 1)[1] for line in result.stdout.splitlines() if PREFIX in line]
    if len(lines) != 1:
        raise RuntimeError(f"Expected exactly one benchmark record (zero tests is a failure); see {log}")
    probe = json.loads(lines[0])
    validate_probe(probe)
    return probe


def self_test():
    import copy
    cases = [{"fixture": f, "width": w, "height": h, "preset": p,
              "warmup": 20, "samples": 80, "first_us": 3000., "median_us": 1000.,
              "p95_us": 1100., "mean_us": 1050., "min_us": 950., "max_us": 1150.}
             for f in FIXTURES for w, h in VIEWPORTS for p in PRESETS]
    probe = {"schema": 2, "debug_assertions": False, "background_quiesced": True,
             "isolation": {"protocol": ISOLATION_PROTOCOL, "settle_ms": 2600,
                           "workers_before": {}, "workers_after": {}},
             "metric": "home_view_tree_build_and_drop_us", "cases": cases}
    rows = aggregate([probe] * 3)
    assert not gate(rows)[0]
    slow = copy.deepcopy(rows)
    slow[0]["p95_us"] = BUDGET_US
    assert gate(slow)[0], "P95 budget must fail even if median passes"
    regressed = copy.deepcopy(rows)
    regressed[0]["median_us"] = 1300.
    assert gate(regressed, {"cases": rows})[0], "Same-host baseline regression must fail"
    noisy = copy.deepcopy(rows)
    noisy[0]["median_us"] = 1100.
    assert not gate(noisy, {"cases": rows})[0]
    active = copy.deepcopy(probe)
    active["isolation"]["workers_after"] = {"1": {"cpu_ticks": 1, "state": "R", "wait": "0"}}
    for invalid in ({**probe, "debug_assertions": True}, {**probe, "background_quiesced": False},
                    {**probe, "isolation": {}}, active,
                    {**probe, "cases": []},
                    {**probe, "cases": cases + [cases[0]]}):
        try:
            validate_probe(invalid)
        except ValueError:
            pass
        else:
            raise AssertionError("Invalid/empty/debug measurements must not pass")
    print("Performance gate checks passed: budget, baseline, noise, completeness, release profile")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("artifacts/home-performance.json"))
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--binary", type=Path, help="Reuse an already compiled release test executable")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if args.runs < 3:
        parser.error("--runs must be at least 3")
    if platform.system() != "Linux":
        parser.error("This isolated timing gate runs on Linux; use normal interaction tests on other platforms")
    repo = Path(__file__).resolve().parents[2]
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    host = host_signature(repo)
    baseline = json.loads(args.baseline.read_text()) if args.baseline else None
    if baseline and (baseline.get("schema") != 2 or baseline.get("host") != host
                     or baseline.get("profile") != "release"
                     or baseline.get("isolation_protocol") != ISOLATION_PROTOCOL
                     or baseline.get("fixture_environment") != FIXTURE_ENVIRONMENT
                     or baseline.get("metric") != "home_view_tree_build_and_drop_us"):
        raise ValueError("Baseline must use the same host, compiler, release profile and metric")
    binary = args.binary.resolve() if args.binary else build_probe(repo, output.with_suffix(".build.log"))
    probes = []
    for index in range(args.runs):
        print(f"Measuring isolated process {index + 1}/{args.runs}…", flush=True)
        probes.append(run_probe(binary, repo, output.with_suffix(f".run-{index + 1}.log")))
    rows = aggregate(probes)
    failures, comparisons = gate(rows, baseline)
    report = {"schema": 2, "profile": "release", "metric": "home_view_tree_build_and_drop_us",
              "isolation_protocol": ISOLATION_PROTOCOL,
              "fixture_environment": FIXTURE_ENVIRONMENT,
              "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "host": host,
              "commit": command_text(["git", "rev-parse", "HEAD"], repo),
              "dirty": bool(command_text(["git", "status", "--porcelain"], repo)),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "budget_us": BUDGET_US, "regression_ratio": REGRESSION_RATIO,
              "regression_floor_us": REGRESSION_FLOOR_US, "cases": rows, "runs": probes,
              "baseline": str(args.baseline) if args.baseline else None,
              "comparisons": comparisons, "passed": not failures, "failures": failures}
    output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    for row in rows:
        print(f"{row['fixture']:12s} {row['width']}x{row['height']} {row['preset']:16s} "
              f"median={row['median_us']:8.1f}us p95={row['p95_us']:8.1f}us")
    print(f"Report: {output}")
    if failures:
        raise RuntimeError("Performance gate failed:\n" + "\n".join(failures))
    print("PASS: all 30 cases within the 15 ms median/P95 budget" +
          (" and same-host baseline gate" if baseline else "; no historical baseline comparison claimed"))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError, subprocess.SubprocessError, OSError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)
