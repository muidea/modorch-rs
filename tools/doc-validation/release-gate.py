#!/usr/bin/env python3
"""Refuse Release Ready without all local and actual Agent evidence."""
import argparse
import hashlib
import json
import pathlib
import sys
import importlib.util

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "skill-packaging"))
from preflight import identity, FRAMEWORK_PACKAGES

ROOT = pathlib.Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location("modorch_report", ROOT / "tools/skill-packaging/validate-report.py")
_report = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_report)
REQUIRED = {"fmt", "check", "clippy", "test", "doctest", "rustdoc", "msrv", "features", "static", "tooling-tests", "examples", "templates", "external", "bundle-install", "skills-format"}
P0_CASES = {f"AE-{i:03}" for i in (1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12)}


def current_framework_sha256(root=ROOT):
    packages = [{"name": name, "manifest_path": str(root / "crates" / name / "Cargo.toml")} for name in sorted(FRAMEWORK_PACKAGES)]
    return identity(packages)["sha256"]


def current_delivery_sha256(root=ROOT):
    """Bind evidence to documentation, Skills and validation sources, excluding evidence itself."""
    files = set(root.glob("*.md")) | {root / "Cargo.toml", root / "Cargo.lock"}
    for folder in ("docs", "contracts", ".agents/skills", "templates", "examples", "tools", "tests/agent-evals", "tests/contracts", ".github/workflows", "integrations"):
        files.update(path for path in (root / folder).rglob("*")
                     if path.is_file() and "target" not in path.parts and "__pycache__" not in path.parts
                     and path.suffix in {".md", ".rs", ".toml", ".lock", ".py", ".sh", ".json", ".yaml", ".yml", ".in"}
                     and not path.is_relative_to(root / "docs/validation"))
    data = "".join(f"{path.relative_to(root).as_posix()}\0{hashlib.sha256(path.read_bytes()).hexdigest()}\n" for path in sorted(files))
    return hashlib.sha256(data.encode()).hexdigest()


def artifact(root, record):
    path = root / record.get("evidence", "")
    return (path.is_file() and path.resolve().is_relative_to(root.resolve())
            and hashlib.sha256(path.read_bytes()).hexdigest() == record.get("sha256"))


def validate(report, root=ROOT):
    blockers = []
    if report.get("framework_sha256") != current_framework_sha256(root):
        blockers.append("framework source fingerprint is missing or stale")
    delivery_sha = current_delivery_sha256(root)
    if report.get("delivery_sha256") != delivery_sha:
        blockers.append("documentation/Skill/validation fingerprint is missing or stale")
    checks = report.get("checks", [])
    by_id = {}
    for check in checks:
        if check.get("id") in by_id:
            blockers.append("duplicate check identity")
        by_id[check.get("id")] = check
    for name in sorted(REQUIRED):
        check = by_id.get(name, {})
        if check.get("status") != "Passed" or check.get("exit_code") != 0 or not check.get("command"):
            blockers.append(f"{name}: missing or not Passed")
            continue
        evidence = root / check.get("evidence", "")
        if not artifact(root, check):
            blockers.append(f"{name}: missing or changed execution log")
    # Never accept a fixture replay as an independent Agent evaluation.
    evaluations = report.get("agent_evals", [])
    verified_hosts = set()
    case_inputs = {row["case_id"]: row for row in json.loads((root / "tests/agent-evals/cases.json").read_text())["cases"]}
    for case in sorted(P0_CASES):
        passed = False
        goal_sha = hashlib.sha256((json.dumps(case_inputs[case], ensure_ascii=False, indent=2) + "\n").encode()).hexdigest()
        for row in evaluations:
            if row.get("case_id") != case or not artifact(root, row):
                continue
            record_path = root / row["evidence"]
            record = json.loads(record_path.read_text())
            if (record.get("case_id") != case or record.get("status") != "Passed"
                    or record.get("mode") != "agent" or not record.get("agent_version") or not record.get("host")
                    or record.get("goal_sha256") != goal_sha or record.get("review", {}).get("status") != "Passed"
                    or record.get("delivery_sha256") != delivery_sha
                    or not record.get("review", {}).get("notes")):
                continue
            # Actual inputs, invocation, diff, model output and validation logs must all survive review.
            files = record.get("artifacts", {})
            required_artifacts = {"goal.json", "command.json", "changes.diff", "stdout.log", "stderr.log", "result.json", "validation.log"}
            if not required_artifacts <= set(files):
                continue
            if any(not (record_path.parent / name).is_file() or hashlib.sha256((record_path.parent / name).read_bytes()).hexdigest() != files[name] for name in required_artifacts):
                continue
            goal_path = record_path.parent / "goal.json"
            if hashlib.sha256(goal_path.read_bytes()).hexdigest() != goal_sha:
                continue
            invocation = json.loads((record_path.parent / "command.json").read_text())
            if not isinstance(invocation, list) or not invocation or not all(isinstance(part, str) for part in invocation):
                continue
            output = json.loads((record_path.parent / "result.json").read_text())
            try:
                _report.validate(output, json.loads((root / "tools/skill-packaging/result.schema.json").read_text()))
            except ValueError:
                continue
            allowed = {"Passed", "Blocked"} if case in {"AE-007", "AE-008", "AE-010"} else {"Passed"}
            skill_metadata = (root / ".agents/skills" / case_inputs[case]["skill"] / "references/framework-identity.json")
            skill_revision = json.loads(skill_metadata.read_text())["skill_revision"]
            if (output["result"]["overall_status"] not in allowed
                    or output["result"]["framework_version"] != "0.0.2"
                    or output["result"]["skill"] != case_inputs[case]["skill"]
                    or output["result"]["skill_revision"] != skill_revision):
                continue
            passed = True
            verified_hosts.add(record["host"])
        if not passed:
            blockers.append(f"{case}: actual Agent execution/review evidence NotRun, missing or failed")
    for host in report.get("claimed_hosts", []):
        if host not in verified_hosts:
            blockers.append(f"{host}: unsupported host claim without execution evidence")
    return {"status": "Candidate" if blockers else "Documentation & Skills Release Ready", "blockers": blockers}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=pathlib.Path)
    args = parser.parse_args()
    try:
        result = validate(json.loads(args.report.read_text()))
    except (OSError, ValueError, KeyError) as error:
        result = {"status": "Candidate", "blockers": [str(error)]}
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 1 if result["blockers"] else 0


if __name__ == "__main__":
    sys.exit(main())
