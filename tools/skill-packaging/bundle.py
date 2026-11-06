#!/usr/bin/env python3
"""Deterministic local Skill Bundle build, verification and conflict-safe installation."""
import argparse
import gzip
import hashlib
import io
import json
import pathlib
import shutil
import sys
import tarfile
import tempfile

from preflight import digest

ROOT = pathlib.Path(__file__).resolve().parents[2]
REVISION = "1.0.7"
MAX_ARCHIVE_BYTES = 16 * 1024 * 1024


def sha(data):
    return hashlib.sha256(data).hexdigest()


def payload(root=ROOT):
    result = {"LICENSE": (root / "LICENSE").read_bytes(),
              "README.md": (root / "tools/skill-packaging/BUNDLE-README.md").read_bytes()}
    for path in sorted((root / ".agents/skills").rglob("*")):
        if path.is_symlink():
            raise ValueError("symlinks are forbidden in Skills")
        if path.is_file() and "__pycache__" not in path.parts:
            result["skills/" + path.relative_to(root / ".agents/skills").as_posix()] = path.read_bytes()
    return result


def build(output, root=ROOT):
    content = payload(root)
    framework = json.loads((root / ".agents/skills/modorch-app-development/references/framework-identity.json").read_text())
    files = {name: sha(data) for name, data in sorted(content.items())}
    manifest = {
        "format_version": 1, "bundle_revision": REVISION,
        "framework_version": "0.0.3", "framework_identity": framework["identity"],
        "compatible_versions": framework["compatible_versions"],
        "contracts_revision": "1.2.0", "files": files,
        "tree_sha256": digest(files),
        "hash_algorithm": "sha256(sorted UTF-8 path + NUL + lowercase file sha256 + LF)",
        "release_status": "Candidate; consult release-gate evidence before publication",
    }
    content["manifest.json"] = (json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()
    output = pathlib.Path(output)
    if output.exists():
        raise ValueError("output already exists; choose a fresh artifact path")
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as zipped:
            with tarfile.open(fileobj=zipped, mode="w") as archive:
                for name, data in sorted(content.items()):
                    info = tarfile.TarInfo(name)
                    info.size, info.mtime = len(data), 0
                    info.mode = 0o755 if "/scripts/" in name and pathlib.PurePosixPath(name).suffix in (".sh", ".py") else 0o644
                    archive.addfile(info, io.BytesIO(data))
    return {"status": "Passed", "bundle_sha256": sha(output.read_bytes()), "tree_sha256": manifest["tree_sha256"]}


def verify(bundle):
    content = {}
    total = 0
    with tarfile.open(bundle, "r:gz") as archive:
        for member in archive:
            path = pathlib.PurePosixPath(member.name)
            if (not member.isfile() or path.is_absolute() or ".." in path.parts
                    or "\\" in member.name or str(path) != member.name
                    or member.name in content or member.size < 0):
                raise ValueError("unsafe, duplicate or non-file archive entry")
            if member.name not in ("LICENSE", "README.md", "manifest.json") and not member.name.startswith("skills/"):
                raise ValueError("unexpected archive entry")
            total += member.size
            if total > MAX_ARCHIVE_BYTES or len(content) >= 2048:
                raise ValueError("archive exceeds size or file-count budget")
            with archive.extractfile(member) as stream:
                content[member.name] = stream.read()
    manifest = json.loads(content.pop("manifest.json"))
    actual = {name: sha(data) for name, data in sorted(content.items())}
    if manifest.get("format_version") != 1 or manifest.get("files") != actual or manifest.get("tree_sha256") != digest(actual):
        raise ValueError("Bundle manifest or file hash mismatch")
    required = {"LICENSE", "README.md"}
    for skill in ("modorch-app-development", "modorch-module-development", "modorch-framework-maintenance"):
        required.update({f"skills/{skill}/SKILL.md", f"skills/{skill}/references/framework-identity.json"})
        identity = json.loads(content.get(f"skills/{skill}/references/framework-identity.json", b"{}"))
        if identity.get("identity") != manifest.get("framework_identity"):
            raise ValueError("inconsistent framework identity")
    if not required <= set(content):
        raise ValueError("Bundle is missing required files")
    return manifest, content


def install(bundle, destination):
    manifest, content = verify(bundle)
    destination = pathlib.Path(destination).absolute()
    # Refuse symlinks in every existing destination ancestor.
    if any(p.is_symlink() for p in (destination, *destination.parents)):
        raise ValueError("destination ancestors must not be symlinks")
    skills = {}
    for name, data in content.items():
        if name.startswith("skills/"):
            _, skill, relative = name.split("/", 2)
            skills.setdefault(skill, {})[relative] = data
    pending = {}
    for skill, files in skills.items():
        current = destination / skill
        if current.exists():
            if not current.is_dir() or any(p.is_symlink() for p in current.rglob("*")):
                raise ValueError(f"unsafe existing Skill: {skill}")
            actual = {p.relative_to(current).as_posix(): p.read_bytes() for p in current.rglob("*") if p.is_file() and "__pycache__" not in p.parts}
            if actual != files:
                raise ValueError(f"Skill conflict: {skill}; install into a clean destination")
        else:
            pending[skill] = files
    destination.mkdir(parents=True, exist_ok=True)
    # Validate all conflicts before writing any Skill. Never remove unrelated content.
    with tempfile.TemporaryDirectory(prefix=".modorch-install-", dir=destination) as temporary:
        stage = pathlib.Path(temporary)
        for skill, files in pending.items():
            for name, data in files.items():
                path = stage / skill / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                if pathlib.PurePosixPath(name).parts[0] == "scripts" and path.suffix in (".sh", ".py"):
                    path.chmod(0o755)
        installed = []
        try:
            for skill in pending:
                if (destination / skill).exists():
                    raise ValueError(f"destination changed during installation: {skill}")
                (stage / skill).rename(destination / skill)
                installed.append(skill)
        except (OSError, ValueError):
            for skill in installed:
                shutil.rmtree(destination / skill)
            raise
    return {"status": "Passed", "installed": sorted(pending), "unchanged": sorted(set(skills) - set(pending)), "bundle_revision": manifest["bundle_revision"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build")
    build_parser.add_argument("--output", type=pathlib.Path, required=True)
    for command in ("verify", "install"):
        sub = commands.add_parser(command)
        sub.add_argument("bundle", type=pathlib.Path)
        if command == "install":
            sub.add_argument("--dest", type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "build":
            report = build(args.output)
        elif args.command == "verify":
            manifest, _ = verify(args.bundle)
            report = {"status": "Passed", "tree_sha256": manifest["tree_sha256"]}
        else:
            report = install(args.bundle, args.dest)
    except (ValueError, OSError, KeyError, tarfile.TarError) as error:
        print(json.dumps({"status": "Failed", "reason": str(error)}, ensure_ascii=False))
        return 2
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
