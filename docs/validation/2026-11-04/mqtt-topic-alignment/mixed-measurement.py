#!/usr/bin/env python3
"""Exercise the built framework under mixed async/CPU/blocking I/O and route churn."""
import argparse
import hashlib
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[4]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    binary = ROOT / "target/release/mixed-load"
    rows = []
    for policy in ("concurrent", "dynamic"):
        command = [str(binary), "--policy", policy, "--seconds", "2", "--cycles", "3", "--subscriptions", "1024", "--cpu-iterations", "10000", "--async-ms", "1", "--blocking-ms", "1", "--idle-ms", "200", "--recover", "true", "--max-idle-rss-growth-kib", "8192"]
        stdout, stderr = args.output / (policy + ".json"), args.output / (policy + ".log")
        with stdout.open("w") as out, stderr.open("w") as err:
            result = subprocess.run(command, cwd=ROOT, stdout=out, stderr=err, check=False, timeout=120)
        data = json.loads(stdout.read_text())
        passed = result.returncode == 0 and data["passed"] and data["shutdown"] == "GracefullyStopped"
        row = {"command": command, "exit_code": result.returncode, "status": "Passed" if passed else "Failed", "files": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in (stdout, stderr)}, "successful_per_active_second": data["successful_per_active_second"], "idle_rss_growth_kib": data["idle_rss_growth_kib"], "recoveries": data["recoveries"], "errors": data["errors"]}
        rows.append(row)
        print(json.dumps(row), flush=True)
    report = {"status": "Passed" if all(r["status"] == "Passed" for r in rows) else "Failed", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "limits": ["Local short workload, concurrent host load; not a production latency SLO or long-duration memory guarantee", "RSS budget 8192 KiB bounds three-cycle idle growth, not precise heap allocation"], "runs": rows}
    (args.output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["status"] == "Passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
