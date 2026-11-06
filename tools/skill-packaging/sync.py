#!/usr/bin/env python3
"""Generate portable, hashed Skill references from controlled repository sources."""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys

from preflight import identity, target_triple

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKILLS = {
    "modorch-app-development": {"application.md": "docs/guides/application.md", "api-compatibility.md": "docs/reference/compatibility.md", "validation.md": "docs/testing-guide.md"},
    "modorch-module-development": {"component-model.md": "docs/concepts/component-model.md", "messaging.md": "docs/guides/messaging.md", "lifecycle.md": "docs/concepts/lifecycle.md", "validation.md": "docs/testing-guide.md"},
    "modorch-framework-maintenance": {"contract-change.md": "CONTRIBUTING.md", "compatibility.md": "docs/reference/compatibility.md", "release-gates.md": "contracts/validation-contract.md", "validation.md": "docs/testing-guide.md"},
}


def generated(root=ROOT):
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version=1", "--all-features", "--locked", "--offline", "--filter-platform", target_triple()], cwd=root, text=True))
    framework = {"framework_version": "0.0.3", "compatible_versions": ["0.0.3"],
                 "skill_revision": "1.0.7", "contracts_revision": "1.2.0", "identity": identity(metadata["packages"]),
                 "features": ["default", "runtime", "eventhub", "scheduler", "config-toml", "observability"],
                 "required_features": ["runtime"], "hash_algorithm": "sha256(sorted UTF-8 path + NUL + lowercase file sha256 + LF)"}
    result = {}
    for name, refs in SKILLS.items():
        base = pathlib.Path(".agents/skills") / name
        sources = {f"references/{key}": value for key, value in refs.items()}
        sources.update({f"references/contracts/{path.name}": path.relative_to(root).as_posix() for path in sorted((root / "contracts").glob("*.md"))})
        reverse = {source: target for target, source in sources.items()}
        records = []
        for target, source in sorted(sources.items()):
            path = root / source
            data = path.read_bytes()
            def replace_link(match):
                label, link = match.groups()
                if re.match(r"[a-z]+://", link) or link.startswith("#"):
                    return match[0]
                local = (path.parent / link.split("#")[0]).resolve()
                if local.is_relative_to(root.resolve()):
                    source_key = local.relative_to(root.resolve()).as_posix()
                    if source_key in reverse:
                        import os
                        destination = pathlib.Path(reverse[source_key])
                        relative = pathlib.Path(os.path.relpath(destination, pathlib.Path(target).parent)).as_posix()
                        fragment = "#" + link.split("#", 1)[1] if "#" in link else ""
                        return f"[{label}]({relative}{fragment})"
                # This is a controlled-source locator, not an installed required file.
                return f"{label}（源码定位：`{link}`；从匹配框架源码查询）"
            text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", replace_link, data.decode())
            rendered = f"> Generated snapshot; source: {source}; source sha256: {hashlib.sha256(data).hexdigest()}; revision 1.0.7. Do not edit.\n\n{text}"
            result[base / target] = rendered.encode()
            records.append({"target": target, "source": source, "source_sha256": hashlib.sha256(data).hexdigest(), "snapshot_sha256": hashlib.sha256(rendered.encode()).hexdigest()})
        for filename in ("preflight.py", "init-template.py", "validate-report.py"):
            result[base / "scripts" / filename] = (root / "tools/skill-packaging" / filename).read_bytes()
        result[base / "references/result.schema.json"] = (root / "tools/skill-packaging/result.schema.json").read_bytes()
        result[base / "references/framework-identity.json"] = (json.dumps(framework, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()
        result[base / "references/sources.json"] = (json.dumps({"revision": "1.0.7", "sources": records}, ensure_ascii=False, indent=2) + "\n").encode()
        template = "module-template" if name == "modorch-module-development" else "minimal-app"
        source_template = "modular-service" if template == "module-template" else "minimal-app"
        for path in sorted((root / "templates" / source_template).rglob("*")):
            relative = path.relative_to(root / "templates" / source_template)
            if not path.is_file() or "target" in relative.parts:
                continue
            target = base / "assets" / template / relative
            data = path.read_bytes()
            if path.name == "Cargo.toml":
                target = target.with_name("Cargo.toml.in")
                data = data.replace(b'"../../crates/modorch"', b'"__MODORCH_PATH__"')
            result[target] = data
    return result


def verify_generated(expected, root=ROOT):
    stale = [str(path) for path, data in expected.items() if not (root / path).is_file() or (root / path).read_bytes() != data]
    # Extra resources in generated directories are drift, including stale retired references.
    actual = {p.relative_to(root) for name in SKILLS for sub in ("references", "assets") for p in (root / ".agents/skills" / name / sub).rglob("*") if p.is_file() and "__pycache__" not in p.parts}
    stale += [str(p) for p in sorted(actual - set(expected))]
    if stale:
        raise ValueError("stale Skill snapshots/resources: " + ", ".join(stale))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        expected = generated()
        if args.check:
            verify_generated(expected)
        else:
            for path, data in expected.items():
                target = ROOT / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
        print(json.dumps({"status": "Passed", "generated_files": len(expected), "mode": "check" if args.check else "refresh"}))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
