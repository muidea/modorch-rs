#!/usr/bin/env python3
"""Replay deterministic clean-room fixtures; never label them as actual Agent runs."""
import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/skill-packaging"))
import bundle

spec = importlib.util.spec_from_file_location("initializer", ROOT / "tools/skill-packaging/init-template.py")
initializer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(initializer)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output must be a new directory")
    args.output.mkdir(parents=True)
    environment = os.environ.copy()
    environment["CARGO_BUILD_JOBS"] = "2"
    environment["CARGO_TARGET_DIR"] = str(ROOT / "target")
    commands = []

    def run(case, step, command, cwd, expected=0, env=environment):
        output = args.output / f"{case}-{step}.log"
        with output.open("w") as log:
            result = subprocess.run(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, check=False, timeout=600)
        row = {"case_id": case, "step": step, "command": command, "exit_code": result.returncode,
               "status": "Passed" if result.returncode == expected else "Failed", "mode": "fixture",
               "evidence": output.name, "sha256": hashlib.sha256(output.read_bytes()).hexdigest()}
        commands.append(row)
        if result.returncode != expected:
            raise ValueError(f"fixture failed: {case}/{step}; see {output}")

    try:
        archive = args.output / f"modorch-skills-{bundle.REVISION}.tar.gz"
        build_result = bundle.build(archive)
        with tempfile.TemporaryDirectory(prefix="modorch-cleanroom-") as temporary:
            clean = pathlib.Path(temporary)
            install_result = bundle.install(archive, clean / ".agents/skills")
            again = bundle.install(archive, clean / ".agents/skills")
            if again["installed"]:
                raise ValueError("reinstall was not idempotent")
            (args.output / "bundle.json").write_text(json.dumps({"build": build_result, "install": install_result, "reinstall": again}, indent=2) + "\n")
            app_skill = clean / ".agents/skills/modorch-app-development"
            module_skill = clean / ".agents/skills/modorch-module-development"
            cases = [("AE-001", "minimal-app", None), ("AE-002", "module-template", None),
                     ("AE-003", "minimal-app", "event-messaging"), ("AE-004", "minimal-app", "request-response"),
                     ("AE-005", "minimal-app", "scheduled-tasks"), ("EX-shared", "minimal-app", "shared-service"),
                     ("EX-lifecycle", "minimal-app", "lifecycle"), ("AE-006", "minimal-app", "@dependency"), ("AE-012", "minimal-app", "@shutdown")]
            for case, template, example in cases:
                app = clean / case
                skill = module_skill if template == "module-template" else app_skill
                initializer.initialize(skill / "assets" / template, app, ROOT / "crates/modorch")
                if example:
                    source = ROOT / "tests/agent-evals/fixtures" / (example[1:] + ".rs") if example.startswith("@") else ROOT / "examples" / example / "src/main.rs"
                    shutil.copyfile(source, app / "src/main.rs")
                manifest = str(app / "Cargo.toml")
                run(case, "lock", ["cargo", "metadata", "--no-deps", "--format-version=1", "--manifest-path", manifest, "--locked", "--offline"], app)
                run(case, "preflight", [sys.executable, str(skill / "scripts/preflight.py"), "--manifest-path", manifest], app)
                for step, extra in (("check", ["--all-targets"]), ("test", []), ("run", [])):
                    run(case, step, ["cargo", step, "--manifest-path", manifest, "--locked", "--offline", *extra], app)
                # Save the actual clean-room app, lock and source for reproducibility, excluding build outputs.
                shutil.copytree(app, args.output / case / "application", ignore=shutil.ignore_patterns("target", "__pycache__"))
            # Negative version, missing-tool, security and gate behavior uses executable tooling tests.
            for case, test in (("AE-007", "test_preflight_rejects_mismatch"),
                               ("AE-008", "test_missing_toolchain_reports_notrun"),
                               ("AE-009", "test_drift_detects_stale_snapshot"),
                               ("AE-010", "test_preflight_rejects_mismatch"),
                               ("AE-011", "test_bundle_rejects_traversal_symlinks_duplicates_and_budgets"),
                               ("VAL-gate", "test_gate_rejects_notrun_or_missing")):
                run(case, "tooling", [sys.executable, "-m", "unittest", f"test_tooling.ToolingTests.{test}"], ROOT / "tests/agent-evals")
        status = "Passed"
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        status = "Failed"
        commands.append({"status": "Failed", "reason": str(error), "mode": "fixture"})
    report = {"status": status, "mode": "fixture", "actual_agent_evaluation": "NotRun",
              "limits": ["AE-007/009/010/011 tooling checks are infrastructure proxies, not semantic Agent acceptance", "Use actual contract tests for dependency and shutdown behavior"],
              "commands": commands}
    (args.output / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"status": status, "mode": "fixture", "steps": len(commands)}, ensure_ascii=False))
    return 0 if status == "Passed" else 1


if __name__ == "__main__":
    sys.exit(main())
