#!/usr/bin/env python3
"""Run local acceptance checks and retain original logs without claiming Agent acceptance."""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("release_gate", ROOT / "tools/doc-validation/release-gate.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--fixtures", type=pathlib.Path, required=True)
    parser.add_argument("--agent-plan", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output = args.output.resolve()
    if not args.output.is_relative_to(ROOT) or args.output.exists():
        parser.error("output must be a new directory inside the repository")
    args.output.mkdir(parents=True)
    environment = os.environ.copy()
    environment["CARGO_BUILD_JOBS"] = "2"
    environment["CARGO_TARGET_DIR"] = str(ROOT / "target")
    checks = []
    report = {
        "recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "framework_sha256": gate.current_framework_sha256(),
        "delivery_sha256": gate.current_delivery_sha256(),
        "checks": checks,
        "agent_evals": json.loads(args.agent_plan.read_text())["agent_evals"],
        "claimed_hosts": [],
    }
    report_path = args.output / "verification.json"

    def save():
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")

    def record(name, commands, extra_environment=None):
        output = args.output / (name + ".log")
        code = 0
        status = "Passed"
        env = environment | (extra_environment or {})
        with output.open("w") as log:
            for command in commands:
                log.write("$ " + json.dumps(command, ensure_ascii=False) + "\n")
                log.flush()
                try:
                    result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=900, check=False)
                    code = result.returncode
                except FileNotFoundError as error:
                    log.write(str(error) + "\n")
                    code, status = 127, "NotRun"
                except (OSError, subprocess.TimeoutExpired) as error:
                    log.write(str(error) + "\n")
                    code = 124
                if code:
                    break
        checks.append({"id": name, "command": commands, "environment": extra_environment or {},
                       "status": status if code == 0 or status == "NotRun" else "Failed", "exit_code": code,
                       "evidence": str(output.relative_to(ROOT)),
                       "sha256": hashlib.sha256(output.read_bytes()).hexdigest()})
        save()
        print(f"{name}: {checks[-1]['status']}", flush=True)

    save()
    record("versions", [["rustc", "--version", "--verbose"], ["cargo", "--version"], ["python3", "--version"]])
    record("fmt", [["cargo", "fmt", "--all", "--", "--check"]])
    common = ["--workspace", "--all-targets", "--all-features", "--locked", "--offline"]
    record("check", [["cargo", "check", *common]])
    record("clippy", [["cargo", "clippy", *common, "--", "-D", "warnings"]])
    record("test", [["cargo", "test", "--workspace", "--all-features", "--locked", "--offline"]])
    record("doctest", [["cargo", "test", "--doc", "--workspace", "--all-features", "--locked", "--offline"]])
    record("rustdoc", [["cargo", "doc", "--workspace", "--all-features", "--no-deps", "--locked", "--offline"]], {"RUSTDOCFLAGS": "-D warnings"})
    record("msrv", [["cargo", "+1.85.0", "check", *common]])
    record("features", [["cargo", "check", "-p", "modorch", "--no-default-features", "--features", feature, "--locked", "--offline"]
                        for feature in ("", "eventhub", "scheduler", "runtime", "config-toml", "observability")])
    record("examples", [["cargo", "run", "-p", name, "--locked", "--offline"]
                        for name in ("basic-app", "shared-service", "event-messaging", "request-response", "scheduled-tasks", "lifecycle")])
    commands = []
    for name in ("minimal-app", "modular-service"):
        manifest = str(ROOT / "templates" / name / "Cargo.toml")
        commands.append(["cargo", "fmt", "--manifest-path", manifest, "--", "--check"])
        commands.extend(["cargo", action, "--manifest-path", manifest, "--locked", "--offline"] for action in ("check", "test", "run"))
    record("templates", commands)
    record("external", [["cargo", "run", "--manifest-path", "tests/fixtures/external-app/Cargo.toml", "--locked", "--offline"]])
    record("tooling-tests", [[sys.executable, "-m", "unittest", "discover", "-s", "tests/agent-evals", "-p", "test_*.py", "-v"]])
    record("skills-format", [["skill-hub", "validate", "--pattern", "modorch-*", "--links"]])
    # Preserve references to every real fixture command and check their original bytes.
    fixture = json.loads(args.fixtures.read_text())
    valid = fixture.get("status") == "Passed" and fixture.get("mode") == "fixture" and bool(fixture.get("commands"))
    for row in fixture.get("commands", []):
        source = args.fixtures.parent / row.get("evidence", "")
        valid = valid and row.get("status") == "Passed" and source.is_file()
        valid = valid and hashlib.sha256(source.read_bytes()).hexdigest() == row.get("sha256")
    output = args.output / "bundle-install.log"
    output.write_text(json.dumps({"fixture_summary": str(args.fixtures), "validated": bool(valid), "report": fixture}, ensure_ascii=False, indent=2) + "\n")
    checks.append({"id": "bundle-install", "command": ["python3", "tests/agent-evals/fixture-replay.py", "--output", str(args.fixtures.parent)],
                   "status": "Passed" if valid else "Failed", "exit_code": 0 if valid else 1,
                   "evidence": str(output.relative_to(ROOT)), "sha256": hashlib.sha256(output.read_bytes()).hexdigest()})
    save()
    record("static", [[sys.executable, "tools/doc-validation/run.py", "--results", str(args.output / "test.log"), "--tooling-results", str(args.output / "tooling-tests.log")]])
    readiness = gate.validate(report)
    (args.output / "release-readiness.json").write_text(json.dumps(readiness, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"local_checks": "Passed" if all(row["status"] == "Passed" for row in checks) else "Failed", "release": readiness["status"]}))
    return 0 if all(row["status"] == "Passed" for row in checks) else 1


if __name__ == "__main__":
    sys.exit(main())
