"""Negative-path tests for install integrity, compatibility, reports and release gates."""
import copy
import hashlib
import importlib.util
import io
import json
import os
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/skill-packaging"))
import bundle
import preflight


def module(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


validation = module("validation", "tools/doc-validation/run.py")
sync = module("sync", "tools/skill-packaging/sync.py")
gate = module("gate", "tools/doc-validation/release-gate.py")
report_validator = module("report_validator", "tools/skill-packaging/validate-report.py")
initializer = module("initializer", "tools/skill-packaging/init-template.py")


class ToolingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.expected = json.loads((ROOT / ".agents/skills/modorch-app-development/references/framework-identity.json").read_text())
        cls.metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--manifest-path", str(ROOT / "tests/fixtures/external-app/Cargo.toml"), "--format-version=1", "--locked", "--offline", "--filter-platform", preflight.target_triple()], text=True))

    def test_bundle_is_deterministic_and_install_is_idempotent(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            first, second = root / "a.tar.gz", root / "b.tar.gz"
            bundle.build(first)
            bundle.build(second)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            bundle.install(first, root / "target")
            result = bundle.install(second, root / "target")
            self.assertEqual(result["installed"], [])
            self.assertEqual(len(result["unchanged"]), 3)

    def test_bundle_rejects_tamper_and_missing_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            source = root / "source.tar.gz"
            bundle.build(source)
            with tarfile.open(source) as archive:
                entries = [(member.name, archive.extractfile(member).read()) for member in archive]
            for skip in (False, True):
                broken = root / f"broken-{skip}.tar.gz"
                with tarfile.open(broken, "w:gz") as archive:
                    for name, data in entries:
                        if name == "LICENSE":
                            if skip:
                                continue
                            data += b"corruption"
                        info = tarfile.TarInfo(name)
                        info.size = len(data)
                        archive.addfile(info, io.BytesIO(data))
                with self.assertRaises(ValueError):
                    bundle.verify(broken)

    def test_bundle_rejects_traversal_symlinks_duplicates_and_budgets(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            for index, kind in enumerate(("../escaped", "/absolute", "symlink", "duplicate", "budget")):
                archive_path = root / f"bad-{index}.tar.gz"
                with tarfile.open(archive_path, "w:gz") as archive:
                    info = tarfile.TarInfo(kind if index < 2 else "README.md")
                    if kind == "symlink":
                        info.type, info.linkname = tarfile.SYMTYPE, "../../escaped"
                    if kind == "budget":
                        info.size = bundle.MAX_ARCHIVE_BYTES + 1
                        # A forged header is enough; verifier rejects before reading payload.
                        archive.fileobj.write(info.tobuf())
                    else:
                        archive.addfile(info)
                        if kind == "duplicate":
                            archive.addfile(info)
                with self.assertRaises((ValueError, tarfile.TarError)):
                    bundle.verify(archive_path)
            self.assertFalse((root.parent / "escaped").exists())

    def test_install_checks_all_conflicts_before_writing(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            archive = root / "bundle.tar.gz"
            bundle.build(archive)
            target = root / "skills"
            conflict = target / "modorch-module-development/SKILL.md"
            conflict.parent.mkdir(parents=True)
            conflict.write_text("user-authored content")
            with self.assertRaises(ValueError):
                bundle.install(archive, target)
            self.assertEqual(conflict.read_text(), "user-authored content")
            self.assertFalse((target / "modorch-app-development").exists())
            linked = root / "link"
            linked.symlink_to(target, target_is_directory=True)
            with self.assertRaises(ValueError):
                bundle.install(archive, linked)

    def test_preflight_matches_external_resolved_graph(self):
        result = preflight.evaluate(self.metadata, self.expected)
        self.assertEqual(result["status"], "Passed")
        self.assertIn("runtime", result["features"])

    def test_preflight_rejects_mismatch(self):
        metadata = copy.deepcopy(self.metadata)
        facade = next(p for p in metadata["packages"] if p["name"] == "modorch")
        facade["version"] = "0.2.0"
        with self.assertRaisesRegex(ValueError, "version"):
            preflight.evaluate(metadata, self.expected)
        expected = copy.deepcopy(self.expected)
        expected["identity"]["files"]["modorch/src/lib.rs"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "fingerprint"):
            preflight.evaluate(self.metadata, expected)
        metadata = copy.deepcopy(self.metadata)
        facade_id = next(p["id"] for p in metadata["packages"] if p["name"] == "modorch")
        next(n for n in metadata["resolve"]["nodes"] if n["id"] == facade_id)["features"] = []
        with self.assertRaisesRegex(ValueError, "runtime"):
            preflight.evaluate(metadata, self.expected)

    def test_missing_toolchain_reports_notrun(self):
        result = subprocess.run([sys.executable, str(ROOT / "tools/skill-packaging/preflight.py"), "--manifest-path", str(ROOT / "Cargo.toml")], env={**os.environ, "PATH": ""}, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        output = json.loads(result.stdout)
        self.assertEqual(output["status"], "Blocked")
        self.assertEqual(output["validation_status"], "NotRun")

    def test_drift_detects_stale_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            file = root / ".agents/skills/modorch-app-development/references/application.md"
            file.parent.mkdir(parents=True)
            file.write_bytes(b"stale")
            with self.assertRaisesRegex(ValueError, "stale"):
                sync.verify_generated({file.relative_to(root): b"current"}, root)

    def test_contract_mapping_rejects_missing_test(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            (root / "tests/contracts").mkdir(parents=True)
            mapping = json.loads((ROOT / "tests/contracts/mapping.json").read_text())
            row = mapping["tests"][0]
            mapping["tests"] = [row]
            (root / "tests/contracts/mapping.json").write_text(json.dumps(mapping))
            contract = root / row["contract"]
            contract.parent.mkdir(parents=True)
            contract.write_text(row["rule_id"] + " " + row["test_id"])
            source = root / row["source"]
            source.parent.mkdir(parents=True)
            source.write_text("fn helper() {}")
            with self.assertRaisesRegex(ValueError, "does not exist"):
                validation.contracts(root)

    def test_tooling_contract_mapping_requires_passing_log(self):
        with tempfile.TemporaryDirectory() as temporary:
            log = pathlib.Path(temporary) / "tooling.log"
            log.write_text("test_drift_detects_stale_snapshot (test_tooling.ToolingTests.test_drift_detects_stale_snapshot) ... FAIL\n")
            with self.assertRaisesRegex(ValueError, "no actual passing tooling result"):
                validation.contracts(tooling_results=log)
            log.write_text("test_drift_detects_stale_snapshot (test_tooling.ToolingTests.test_drift_detects_stale_snapshot) ... ok\n")
            self.assertEqual(validation.contracts(tooling_results=log)["tooling_execution"], "Passed")

    def test_gate_rejects_notrun_or_missing(self):
        for report in ({}, {"checks": [{"id": "fmt", "status": "NotRun"}]}, {"agent_evals": [{"case_id": "AE-001", "status": "Passed", "mode": "fixture"}]}):
            result = gate.validate(report)
            self.assertEqual(result["status"], "Candidate")
            self.assertTrue(result["blockers"])

    def test_report_rejects_false_pass_and_missing_fields(self):
        schema = json.loads((ROOT / "tools/skill-packaging/result.schema.json").read_text())
        with self.assertRaises(ValueError):
            report_validator.validate({"result": {"overall_status": "Passed"}}, schema)
        result = {"task": "test", "framework_version": "0.0.2", "skill": "modorch-app-development", "skill_revision": "1.0.0", "changed_files": [], "validation": [{"command": "cargo test", "status": "NotRun", "evidence": "toolchain missing"}], "contract_checks": [{"rule_id": "COMP-002", "status": "NotRun", "evidence": "not executed"}], "limitations": [], "unresolved": [], "overall_status": "Passed"}
        with self.assertRaisesRegex(ValueError, "unexecuted"):
            report_validator.validate({"result": result}, schema)
        result["overall_status"] = "Blocked"
        report_validator.validate({"result": result}, schema)

    def test_template_never_overwrites_existing_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            destination = pathlib.Path(temporary) / "application"
            destination.mkdir()
            with self.assertRaisesRegex(ValueError, "must not exist"):
                initializer.initialize(ROOT / "templates/minimal-app", destination, ROOT / "crates/modorch")

    def test_skill_trigger_boundaries_are_explicit(self):
        for name in validation.SKILLS:
            metadata, body = validation.frontmatter(ROOT / ".agents/skills" / name / "SKILL.md")
            self.assertIn("不用于", metadata["description"])
            self.assertIn("适用与不适用", body)
            self.assertIn("版本/指纹不匹配", body)
            self.assertIn("不受信数据", body)



class ReleaseEvidenceTests(unittest.TestCase):
    def test_complete_evidence_can_pass_and_tamper_blocks_it(self):
        # Synthetic artifacts exercise the gate policy; they are never published as real evals.
        target = ROOT / "target"
        target.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as temporary:
            base = pathlib.Path(temporary)
            log = base / "checks.log"
            log.write_text("synthetic gate-policy test only\n")
            report = {"framework_sha256": gate.current_framework_sha256(), "delivery_sha256": gate.current_delivery_sha256(), "checks": [], "agent_evals": []}
            for name in gate.REQUIRED:
                report["checks"].append({"id": name, "status": "Passed", "exit_code": 0, "command": ["synthetic-policy-test"], "evidence": str(log.relative_to(ROOT)), "sha256": hashlib.sha256(log.read_bytes()).hexdigest()})
            cases = {row["case_id"]: row for row in json.loads((ROOT / "tests/agent-evals/cases.json").read_text())["cases"]}
            for case_id in gate.P0_CASES:
                directory = base / case_id
                directory.mkdir()
                goal = json.dumps(cases[case_id], ensure_ascii=False, indent=2) + "\n"
                (directory / "goal.json").write_text(goal)
                (directory / "command.json").write_text('["synthetic-policy-test"]')
                for name in ("changes.diff", "stdout.log", "stderr.log", "validation.log"):
                    (directory / name).write_text("synthetic gate-policy test only\n")
                result = {"task": "synthetic gate policy test", "framework_version": "0.0.2", "skill": cases[case_id]["skill"], "skill_revision": "1.0.5", "changed_files": [], "validation": [{"command": "synthetic", "status": "Passed", "evidence": "policy-test only"}], "contract_checks": [{"rule_id": "COMP-002", "status": "Passed", "evidence": "policy-test only"}], "limitations": ["synthetic"], "unresolved": [], "overall_status": "Passed"}
                (directory / "result.json").write_text(json.dumps({"result": result}))
                record = {"case_id": case_id, "status": "Passed", "mode": "agent", "host": "synthetic-policy-test", "agent_version": "test", "review": {"status": "Passed", "notes": "synthetic gate policy test only"}, "goal_sha256": hashlib.sha256(goal.encode()).hexdigest(), "artifacts": {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in directory.iterdir()}}
                record["delivery_sha256"] = gate.current_delivery_sha256()
                record_path = directory / "record.json"
                record_path.write_text(json.dumps(record))
                report["agent_evals"].append({"case_id": case_id, "evidence": str(record_path.relative_to(ROOT)), "sha256": hashlib.sha256(record_path.read_bytes()).hexdigest()})
            self.assertFalse(gate.validate(report)["blockers"])
            report["claimed_hosts"] = ["synthetic-policy-test"]
            self.assertFalse(gate.validate(report)["blockers"])
            report["claimed_hosts"] = ["unverified-host"]
            report["agent_evals"].append({"host": "unverified-host", "mode": "agent", "status": "Passed"})
            self.assertIn("unverified-host: unsupported host claim without execution evidence", gate.validate(report)["blockers"])
            report["claimed_hosts"] = []
            log.write_text("changed after validation")
            self.assertTrue(gate.validate(report)["blockers"])
            report["framework_sha256"] = "stale"
            self.assertIn("framework source fingerprint is missing or stale", gate.validate(report)["blockers"])


if __name__ == "__main__":
    unittest.main()
