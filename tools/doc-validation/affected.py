#!/usr/bin/env python3
"""Report affected documentation gates from tracked and new local changes."""
import argparse
import fnmatch
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--base", default="HEAD")
args = parser.parse_args()
changed = subprocess.check_output(["git", "diff", "--name-only", args.base], cwd=ROOT, text=True).splitlines()
changed += subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard"], cwd=ROOT, text=True).splitlines()
required = set()
for rule in json.loads((ROOT / "tools/doc-validation/affected-files.yaml").read_text())["rules"]:
    if any(fnmatch.fnmatch(path, pattern) for path in changed for pattern in rule["paths"]):
        required.update(rule["requires"])
print(json.dumps({"changed_files": sorted(set(changed)), "required_checks": sorted(required), "semantic_review": "Required; this is an impact report, not proof of verification"}, ensure_ascii=False, indent=2))
