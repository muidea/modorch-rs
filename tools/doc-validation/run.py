#!/usr/bin/env python3
"""Offline documentation, contract, capability and Skill consistency gates."""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKILLS = ("modorch-app-development", "modorch-module-development", "modorch-framework-maintenance")
STATES = {"Implemented", "Verified", "Experimental", "Planned", "Unsupported"}


def load(path):
    return json.loads(pathlib.Path(path).read_text())


def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def prose(text):
    # Exclude fenced examples and inline code from Markdown link checks.
    text = re.sub(r"(?ms)^```.*?^```[^\n]*", "", text)
    return re.sub(r"`[^`\n]*`", "", text)


def anchors(text):
    result, seen = set(), {}
    for heading in re.findall(r"(?m)^#{1,6}\s+(.+?)(?:\s+#+)?$", prose(text)):
        name = re.sub(r"[^\w\-\s]", "", heading.lower()).replace(" ", "-")
        count = seen.get(name, 0)
        seen[name] = count + 1
        result.add(name + (f"-{count}" if count else ""))
    return result


def markdown_files(root):
    candidates = [*root.glob("*.md"), *root.glob("docs/**/*.md"), *root.glob("contracts/**/*.md"),
                  *root.glob(".agents/skills/**/*.md"), *root.glob("tools/**/*.md"),
                  *root.glob("tests/agent-evals/**/*.md"), *root.glob("templates/**/*.md"),
                  *root.glob("examples/**/*.md"), *root.glob("integrations/**/*.md")]
    return sorted(set(p for p in candidates if "target" not in p.parts))


def links(root=ROOT):
    checked = 0
    for path in markdown_files(root):
        text = prose(path.read_text())
        # Inline and reference-style links; remote targets are syntax-only, never fetched.
        targets = re.findall(r"!?\[[^\]\n]*\]\((<[^>]+>|[^)\s]+)(?:\s+\"[^\"]*\")?\)", text)
        targets += re.findall(r"(?m)^\[[^\]]+\]:\s*(\S+)", text)
        for value in targets:
            value = value.strip("<>")
            url = urllib.parse.urlsplit(value)
            if url.scheme or url.netloc:
                continue
            target = (path.parent / urllib.parse.unquote(url.path)).resolve() if url.path else path.resolve()
            if not target.is_relative_to(root.resolve()) or not target.exists():
                raise ValueError(f"broken or escaping link: {path.relative_to(root)} -> {value}")
            if url.fragment and target.suffix == ".md" and urllib.parse.unquote(url.fragment) not in anchors(target.read_text()):
                raise ValueError(f"missing Markdown anchor: {path.relative_to(root)} -> {value}")
            checked += 1
    return {"local_links_checked": checked, "remote_links": "NotRun (offline gate)"}


def contracts(root=ROOT, results=None, tooling_results=None):
    mapping = load(root / "tests/contracts/mapping.json")
    ids, semantics = set(), set()
    log = pathlib.Path(results).read_text() if results else None
    for row in mapping["tests"]:
        if row["test_id"] in ids:
            raise ValueError("duplicate Contract Test ID")
        ids.add(row["test_id"])
        semantics.update(row["semantic_ids"])
        body = (root / row["contract"]).read_text()
        if row["rule_id"] not in body or row["test_id"] not in body:
            raise ValueError(f"contract rule/Test ID is missing: {row['rule_id']}")
        source = (root / row["source"]).read_text()
        # Ensure the named function really has a test attribute, not just a helper.
        pattern = r"#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*(?:async\s+)?fn\s+" + re.escape(row["name"]) + r"\s*\("
        if not re.search(pattern, source):
            raise ValueError(f"mapped test does not exist: {row['test_id']}")
        if log is not None and not re.search(r"(?m)^test (?:[\w:]+::)?" + re.escape(row["name"]) + r" \.\.\. ok$", log):
            raise ValueError(f"no actual passing result: {row['test_id']}")
    if semantics != {f"SEM-{i:03}" for i in range(1, 13)}:
        raise ValueError("SEM-001..012 mapping is incomplete")
    tooling_log = pathlib.Path(tooling_results).read_text() if tooling_results else None
    for row in mapping.get("tooling_tests", []):
        body = (root / row["contract"]).read_text()
        source = (root / row["source"]).read_text()
        if row["test_id"] in ids or row["test_id"] not in body or row["rule_id"] not in body:
            raise ValueError("invalid tooling Contract Test ID: " + row["test_id"])
        ids.add(row["test_id"])
        if not re.search(r"(?m)^    def " + re.escape(row["name"]) + r"\(self\):", source):
            raise ValueError("missing mapped tooling test: " + row["name"])
        if tooling_log is not None and not re.search(r"(?m)^" + re.escape(row["name"]) + r" \([^\n]+\) \.\.\. ok$", tooling_log):
            raise ValueError("no actual passing tooling result: " + row["test_id"])
    python_source = (root / "tests/agent-evals/test_tooling.py").read_text()
    for filename in ("compatibility-contract.md", "validation-contract.md"):
        for name in re.findall(r"\[(test_\w+)\]", (root / "contracts" / filename).read_text()):
            if f"def {name}(" not in python_source:
                raise ValueError(f"missing tooling contract test: {name}")
    return {"contract_tests": len(ids), "test_execution": "Passed" if log is not None else "NotRun (mapping only)", "tooling_execution": "Passed" if tooling_log is not None else "NotRun (mapping only)"}


def frontmatter(path):
    text = path.read_text()
    if not text.startswith("---\n") or "\n---\n" not in text[4:]:
        raise ValueError(f"Skill frontmatter missing: {path}")
    header, body = text[4:].split("\n---\n", 1)
    # Repository subset: quoted scalar YAML plus a string-valued metadata map.
    result, metadata = {}, {}
    in_metadata = False
    for line in header.splitlines():
        if line == "metadata:":
            in_metadata = True
            continue
        match = re.fullmatch(r"(  )?([a-zA-Z0-9_-]+): (.+)", line)
        if not match or bool(match[1]) != in_metadata:
            raise ValueError(f"unsupported or invalid Skill YAML: {path}")
        key, value = match[2], match[3]
        value = json.loads(value) if value.startswith('"') else value
        if not isinstance(value, str):
            raise ValueError("Skill metadata values must be strings")
        target = metadata if in_metadata else result
        if key in target:
            raise ValueError("duplicate Skill metadata key")
        target[key] = value
    result["metadata"] = metadata
    if result.get("name") != path.parent.name or not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", result.get("name", "")):
        raise ValueError("Skill name must match its directory")
    if len(result["name"]) > 64 or not 1 <= len(result.get("description", "")) <= 1024:
        raise ValueError("Skill metadata length is invalid")
    if result["metadata"].get("version") != "1.0.5":
        raise ValueError("Skill revision missing or inconsistent")
    return result, body


def skills(root=ROOT):
    for name in SKILLS:
        base = root / ".agents/skills" / name
        _, body = frontmatter(base / "SKILL.md")
        for heading in ("适用", "输入", "预检", "决策", "流程", "停止", "输出", "参考", "Formatter"):
            if not re.search(r"(?m)^## .*" + heading, body):
                raise ValueError(f"Skill missing required workflow section: {name}/{heading}")
        if len(body.splitlines()) > 300 or "TODO" in body:
            raise ValueError("Skill body unfinished or too long")
        # Skill-required local links must never escape the Skill root after installation.
        for value in re.findall(r"\[[^\]]*\]\(([^)]+)\)", prose(body)):
            if urllib.parse.urlsplit(value).scheme:
                continue
            target = (base / value.split("#")[0]).resolve()
            if not target.is_relative_to(base.resolve()) or not target.exists():
                raise ValueError(f"nonportable Skill reference: {name}/{value}")
        for script in ("preflight.py", "init-template.py"):
            if (base / "scripts" / script).read_bytes() != (root / "tools/skill-packaging" / script).read_bytes():
                raise ValueError("stale portable Skill script")
    return {"skills_checked": len(SKILLS), "format": "Agent Skills open format; repository YAML subset"}


def matrix(root=ROOT):
    data = load(root / "docs/reference/capabilities.json")
    ids = {row["rule_id"] for row in load(root / "tests/contracts/mapping.json")["tests"]}
    for row in data["capabilities"]:
        if row["status"] not in STATES or not row.get("version") or not row.get("platform"):
            raise ValueError("capability status, version or platform missing")
        if row["status"] in ("Verified", "Experimental"):
            if not row["rules"] or not set(row["rules"]) <= ids:
                raise ValueError("verified capability lacks mapped rules")
            evidence = root / row["evidence"]
            if not evidence.is_file():
                raise ValueError("verified capability evidence missing")
            log = evidence.read_text()
            mapped = load(root / "tests/contracts/mapping.json")["tests"]
            for test in (test for test in mapped if test["rule_id"] in row["rules"]):
                if not re.search(r"(?m)^test (?:[\w:]+::)?" + re.escape(test["name"]) + r" \.\.\. ok$", log):
                    raise ValueError("verified capability has no passing mapped result: " + test["name"])
    rendered = (root / "docs/reference/capability-matrix.md").read_text()
    for row in data["capabilities"]:
        if row["id"] not in rendered or row["status"] not in rendered:
            raise ValueError("capability Markdown/JSON drift")
    return {"capabilities_checked": len(data["capabilities"])}


def drift(root=ROOT):
    subprocess.run([sys.executable, str(root / "tools/skill-packaging/sync.py"), "--check"], cwd=root, check=True, capture_output=True, text=True)
    return {"snapshots": "current"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", nargs="?", choices=["links", "contracts", "skills", "matrix", "drift", "all"], default="all")
    parser.add_argument("--results", type=pathlib.Path)
    parser.add_argument("--tooling-results", type=pathlib.Path)
    args = parser.parse_args()
    checks = {"links": links, "contracts": lambda: contracts(results=args.results, tooling_results=args.tooling_results), "skills": skills, "matrix": matrix, "drift": drift}
    reports = []
    for name, check in checks.items():
        if args.kind not in ("all", name):
            continue
        try:
            reports.append({"check": name, "status": "Passed", "details": check()})
        except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
            reports.append({"check": name, "status": "Failed", "reason": str(error)})
    print(json.dumps(reports, ensure_ascii=False, indent=2))
    return 1 if any(row["status"] == "Failed" for row in reports) else 0


if __name__ == "__main__":
    sys.exit(main())
