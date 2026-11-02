# Modorch Skills 1.0.3 Candidate

Framework: modorch 0.0.1 development tree, exact source fingerprint in manifest.json.
License: MIT. Requires Rust >= 1.85, Cargo, Python >= 3.11 and a resolved lockfile/cache.

This bundle contains complete Agent Skills in skills/. Adding a Cargo dependency does
not install them. Copy the three skill directories into an explicitly chosen project
.agents/skills location, or use the version-matched source repository's bundle.py install
command to verify integrity and reject conflicts before writing. Do not overwrite an
unreviewed existing Skill. Automatic discovery is host/version-specific.

Run the installed scripts/preflight.py --manifest-path <app>/Cargo.toml before using
APIs. It checks actual Cargo metadata offline against the framework version, feature
set and source hashes. Unknown or modified sources block API generation. Missing tools
or cache means validation NotRun, never Passed. Read SKILL.md and only the task-relevant
references. Template rendering uses an explicit Facade source path and a new destination.

manifest.json uses per-file SHA-256; tree hash is SHA-256 of sorted UTF-8 path + NUL +
lowercase file hash + LF records. All payload files except manifest are covered.
Check archive and manifest integrity against the source repository's trusted build output.
Hashes detect corruption; they do not authenticate an untrusted publisher.

Release readiness is independent of successful local packaging: missing actual Agent
P0 evaluations or supported-host evidence keeps this bundle Candidate. No crates.io,
docs.rs, remote release, automatic retry, persistent messaging or arbitrary side-effect
rollback is promised. Host authorization remains required by the host's execution policy.
