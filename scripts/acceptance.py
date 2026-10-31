#!/usr/bin/env python3
"""Sequential release measurements, bounded smoke and burst/idle recovery soak.

Build before running. Output directories must be new: historical evidence is never
overwritten. Results include source/binary hashes, quotas, raw JSON and stderr.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def optional_text(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return None


def validate_result(result):
    if "passed" in result:
        if not result["passed"] or result["errors"]:
            raise ValueError(f"mixed acceptance failed: {result.get('errors')}")
        if result["after"]["pending_operations"] != 0:
            raise ValueError("operations remain after mixed shutdown")
    elif result.get("error") is not None or result["completed"] != result["replies"] or result["pending_after_shutdown"] != 0:
        raise ValueError("echo counts/accounting failed")
    if result["shutdown"] != "GracefullyStopped":
        raise ValueError("shutdown was not graceful")


def sample(out, name, command, timeout=180):
    command = [str(x) for x in command]
    cpu_before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.monotonic()
    peak_rss = 0
    peak_threads = 0
    print(name, flush=True)
    # File-backed output avoids pipe-buffer deadlock for detailed sample JSON.
    with (out / f"{name}.stdout").open("w+") as stdout, (out / f"{name}.stderr").open("w") as stderr:
        process = subprocess.Popen(command, stdout=stdout, stderr=stderr, cwd=ROOT)
        try:
            while process.poll() is None:
                status = optional_text(f"/proc/{process.pid}/status") or ""
                for line in status.splitlines():
                    if line.startswith("VmRSS:"):
                        peak_rss = max(peak_rss, int(line.split()[1]))
                    if line.startswith("Threads:"):
                        peak_threads = max(peak_threads, int(line.split()[1]))
                if time.monotonic() - start > timeout:
                    raise TimeoutError(f"{name} exceeded {timeout}s")
                time.sleep(.025)
            if process.returncode:
                raise RuntimeError(f"{name} failed ({process.returncode}); see {name}.stdout/.stderr")
        finally:
            if process.poll() is None:
                process.kill()
            process.wait()
        stdout.seek(0)
        result = json.load(stdout)
    cpu_after = resource.getrusage(resource.RUSAGE_CHILDREN)
    validate_result(result)
    result["measurement"] = {
        "command": command, "wall_seconds": time.monotonic() - start,
        "rss_peak_kib": peak_rss, "thread_peak": peak_threads,
        "user_seconds": cpu_after.ru_utime - cpu_before.ru_utime,
        "system_seconds": cpu_after.ru_stime - cpu_before.ru_stime,
        "rss_sample_ms": 25,
    }
    (out / f"{name}.json").write_text(json.dumps(result, indent=2) + "\n")
    (out / f"{name}.stdout").unlink()
    return result


def aggregate(results):
    groups = {}
    for name, result in results.items():
        if name.startswith("dhat-"):
            continue
        group = name.rsplit("-trial", 1)[0]
        groups.setdefault(group, []).append(result)
    summary = {}
    for group, samples in groups.items():
        row = {"samples": len(samples),
               "rss_peak_kib": max(s["measurement"]["rss_peak_kib"] for s in samples),
               "user_seconds_median": statistics.median(s["measurement"]["user_seconds"] for s in samples),
               "system_seconds_median": statistics.median(s["measurement"]["system_seconds"] for s in samples)}
        if "kinds" in samples[0]:
            row["successful_per_active_second"] = statistics.median(s["successful_per_active_second"] for s in samples)
            row["kinds"] = {kind: {
                "successful_median": statistics.median(s["kinds"][kind]["successful"] for s in samples),
                "p95_us_median": statistics.median(s["kinds"][kind]["successful_latency"]["p95_us"] for s in samples),
                "p99_us_median": statistics.median(s["kinds"][kind]["successful_latency"]["p99_us"] for s in samples),
                "rejected_total": sum(s["kinds"][kind]["admission_rejected"] + s["kinds"][kind]["resource_rejected"] for s in samples),
            } for kind in samples[0]["kinds"]}
        else:
            row["requests_per_second"] = statistics.median(s["requests_per_second"] for s in samples)
        summary[group] = row
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=["smoke", "performance", "stability", "all"], default="smoke")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--mixed-binary", type=Path, default=ROOT / "target/release/mixed-load")
    parser.add_argument("--echo-binary", type=Path, default=ROOT / "target/release/runtime-load")
    parser.add_argument("--baseline-binary", type=Path)
    parser.add_argument("--baseline-sha256", help="Optional expected immutable baseline binary hash")
    parser.add_argument("--profile", type=Path, default=ROOT / "examples/mixed-load/profiles/local.toml")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--seconds", type=int, default=2)
    parser.add_argument("--soak-seconds", type=int, default=300)
    parser.add_argument("--cycles", type=int, default=12)
    parser.add_argument("--dhat", action="store_true")
    args = parser.parse_args()
    if args.repeats < 1 or args.repeats > 20 or args.seconds < 1 or args.cycles < 2 or args.soak_seconds < args.cycles:
        parser.error("require 1..20 repeats, positive seconds, at least 2 cycles and soak-seconds >= cycles")
    if args.baseline_sha256 and not args.baseline_binary:
        parser.error("baseline-sha256 requires baseline-binary")
    if args.baseline_binary and args.baseline_sha256 and digest(args.baseline_binary) != args.baseline_sha256:
        parser.error("baseline binary hash does not match")
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    binaries = {"mixed": args.mixed_binary.resolve(), "echo": args.echo_binary.resolve()}
    if args.baseline_binary:
        binaries["baseline_echo"] = args.baseline_binary.resolve()
    files = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / ".github/workflows/ci.yml"]
    for directory in ["crates", "examples", "scripts"]:
        files += [p for p in (ROOT / directory).rglob("*") if p.is_file() and p.suffix in {".rs", ".toml", ".py"} and "__pycache__" not in p.parts]
    manifest = {str(p.relative_to(ROOT)): digest(p) for p in sorted(files)}
    environment = {
        "command": sys.argv, "uname": list(os.uname()), "rust": subprocess.check_output(["rustc", "-Vv"], text=True),
        "binaries": {name: {"path": str(path), "sha256": digest(path)} for name, path in binaries.items()},
        "source_files_sha256": manifest, "profile": str(args.profile.resolve()), "profile_sha256": digest(args.profile),
        "cpu_max": optional_text("/sys/fs/cgroup/cpu.max"), "memory_max": optional_text("/sys/fs/cgroup/memory.max"),
        "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "caveat": "Sequential synthetic measurements on this host; RSS includes runtime, harness and allocator. Histogram quantiles are upper-bucket estimates. No production certification.",
    }
    (out / "environment.json").write_text(json.dumps(environment, indent=2) + "\n")
    mixed = [binaries["mixed"], "--profile", args.profile.resolve()]
    results = {}
    allocations = {}

    def run(name, command, timeout=180):
        results[name] = sample(out, name, command, timeout)

    try:
        if args.suite in {"smoke", "all"}:
            run("mixed-smoke", mixed + ["--messages", 16, "--cycles", 1])
            run("dynamic-recovery-smoke", mixed + ["--messages", 16, "--cycles", 2, "--policy", "dynamic", "--recover", "true"])
            run("overload-smoke", [binaries["mixed"], "--profile", ROOT / "examples/mixed-load/profiles/overload.toml", "--messages", 32])
        if args.suite in {"performance", "all"}:
            for mode in ["serial", "partitioned", "concurrent", "dynamic"]:
                for producers in [1, 64]:
                    for trial in range(args.repeats):
                        variants = [("current", binaries["echo"])]
                        if args.baseline_binary:
                            variants.insert(0, ("baseline", binaries["baseline_echo"]))
                        if trial % 2:
                            variants.reverse()
                        for label, binary in variants:
                            run(f"echo-{mode}-p{producers}-{label}-trial{trial+1}", [binary, "--mode", mode, "--seconds", args.seconds, "--producers", producers, "--capacity", 8])
            for trial in range(args.repeats):
                modes = ["inline", "isolated"] if trial % 2 == 0 else ["isolated", "inline"]
                for execution in modes:
                    run(f"mixed-{execution}-trial{trial+1}", mixed + ["--execution", execution, "--messages", 0, "--seconds", args.seconds, "--cycles", 1, "--recover", "false"])
            for subscriptions in [1, 128, 1024]:
                for trial in range(args.repeats):
                    run(f"subscriptions-{subscriptions}-trial{trial+1}", mixed + ["--subscriptions", subscriptions, "--messages", 0, "--seconds", args.seconds, "--cycles", 1, "--recover", "false"])
        if args.suite in {"stability", "all"}:
            seconds = (args.soak_seconds + args.cycles - 1) // args.cycles
            run("dynamic-recovery-soak", mixed + ["--policy", "dynamic", "--recover", "true", "--payload-bytes", 65536,
                "--messages", 0, "--seconds", seconds, "--cycles", args.cycles], timeout=args.cycles * (seconds + 20) + 120)
        if args.dhat:
            for mode in ["serial", "concurrent"]:
                variants = [("current", binaries["echo"])]
                if args.baseline_binary:
                    variants.insert(0, ("baseline", binaries["baseline_echo"]))
                for label, binary in variants:
                    name = f"dhat-{mode}-{label}"
                    dhat = out / f"{name}-heap.json"
                    run(name, ["valgrind", "--tool=dhat", f"--dhat-out-file={dhat}", binary, "--mode", mode, "--messages", 1000, "--producers", 8, "--capacity", 8], timeout=300)
                    points = json.loads(dhat.read_text())["pps"]
                    values = {key: sum(p[key] for p in points) for key in ["tb", "tbk", "eb", "ebk"]}
                    values["allocations_per_reply"] = values["tbk"] / results[name]["replies"]
                    allocations[name] = values
            name = "dhat-mixed-current"
            dhat = out / f"{name}-heap.json"
            run(name, ["valgrind", "--tool=dhat", f"--dhat-out-file={dhat}"] + mixed + ["--messages", 32, "--cycles", 2, "--timeout-ms", 30000], timeout=300)
            points = json.loads(dhat.read_text())["pps"]
            allocations[name] = {key: sum(p[key] for p in points) for key in ["tb", "tbk", "eb", "ebk"]}
    except Exception as error:
        (out / "failure.json").write_text(json.dumps({"error": str(error)}, indent=2) + "\n")
        raise
    summary = {"passed": True, "samples": len(results), "groups": aggregate(results), "allocations": allocations,
               "baseline_supplied": args.baseline_binary is not None,
               "production_slo_certified": False}
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({"passed": True, "samples": len(results), "output": str(out)}))


if __name__ == "__main__":
    main()
