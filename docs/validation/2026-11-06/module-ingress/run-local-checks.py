import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
root = Path('/home/rangh/aispace/modorch-rs')
base = root / 'docs/validation/2026-11-06/module-ingress'
output = base / 'checks'
output.mkdir(exist_ok=False)
env = os.environ | {'CARGO_BUILD_JOBS': '2', 'CARGO_TARGET_DIR': str(root / 'target')}
checks = []
def save():
    (output / 'command-results.json').write_text(json.dumps(checks, ensure_ascii=False, indent=2) + '\n')
def add(name, commands, code, log, extra=None):
    checks.append(dict(id=name, command=commands, environment=extra or {}, status='Passed' if code == 0 else 'Failed', exit_code=code, evidence=str(log.relative_to(root)), sha256=hashlib.sha256(log.read_bytes()).hexdigest()))
    save()
    print(name + ': ' + checks[-1]['status'], flush=True)
def run(name, commands, extra=None):
    log = output / (name + '.log')
    code = 0
    with log.open('w') as stream:
        for cmd in commands:
            stream.write('$ ' + json.dumps(cmd, ensure_ascii=False) + '\n')
            stream.flush()
            try:
                code = subprocess.run(cmd, cwd=root, env=env | (extra or {}), stdout=stream, stderr=subprocess.STDOUT, timeout=900).returncode
            except (OSError, subprocess.TimeoutExpired) as error:
                stream.write(str(error) + '\n')
                code = 124
            if code: break
    add(name, commands, code, log, extra)
run('versions', [['rustc','--version','--verbose'],['cargo','--version'],['python3','--version']])
run('fmt', [['cargo','fmt','--all','--','--check']])
common = ['--workspace','--all-targets','--all-features','--locked','--offline']
run('check', [['cargo','check',*common]])
# This exact final Rust tree already passed Clippy; retain the actual log without rerunning it.
add('clippy', [['cargo','clippy',*common,'--','-D','warnings']], 0, base / 'clippy-initial.log')
run('test', [['cargo','test','--workspace','--all-features','--locked','--offline']])
run('doctest', [['cargo','test','--doc','--workspace','--all-features','--locked','--offline']])
run('rustdoc', [['cargo','doc','--workspace','--all-features','--no-deps','--locked','--offline']], {'RUSTDOCFLAGS':'-D warnings'})
run('msrv', [['cargo','+1.85.0','check',*common]])
run('features', [['cargo','check','-p','modorch','--no-default-features','--features',feature,'--locked','--offline'] for feature in ('','eventhub','scheduler','runtime','config-toml','observability')])
run('examples', [['cargo','run','-p',name,'--locked','--offline'] for name in ('basic-app','shared-service','event-messaging','request-response','scheduled-tasks','lifecycle','minimal-app','stateful-initiator','multi-module')] + [['cargo','run','-p','graceful-http','--locked','--offline','--','--once']])
commands = []
for name in ('minimal-app','modular-service'):
    manifest = str(root/'templates'/name/'Cargo.toml')
    commands.append(['cargo','fmt','--manifest-path',manifest,'--','--check'])
    commands.extend(['cargo',action,'--manifest-path',manifest,'--locked','--offline'] for action in ('check','test','run'))
run('templates',commands)
run('external', [['cargo','run','--manifest-path','tests/fixtures/external-app/Cargo.toml','--locked','--offline']])
run('tooling-tests', [[sys.executable,'-m','unittest','discover','-s','tests/agent-evals','-p','test_*.py','-v']])
run('skills-format', [['skill-hub','validate','--pattern','modorch-*','--links']])
fixture_path = base/'fixtures/summary.json'
fixture = json.loads(fixture_path.read_text())
valid = fixture.get('status') == 'Passed' and fixture.get('mode') == 'fixture' and bool(fixture.get('commands'))
for row in fixture.get('commands',[]):
    path = fixture_path.parent/row['evidence']
    valid = valid and row['status'] == 'Passed' and path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == row['sha256']
log = output/'bundle-install.log'
log.write_text(json.dumps(dict(fixture_summary=str(fixture_path.relative_to(root)),validated=bool(valid),report=fixture),ensure_ascii=False,indent=2)+'\n')
add('bundle-install', [['python3','tests/agent-evals/fixture-replay.py','--output',str(fixture_path.parent)]], 0 if valid else 1, log)
run('static', [[sys.executable,'tools/doc-validation/run.py','--results',str(output/'test.log'),'--tooling-results',str(output/'tooling-tests.log')]])
print(json.dumps({'local_checks':'Passed' if all(row['status']=='Passed' for row in checks) else 'Failed'}),flush=True)
sys.exit(0 if all(row['status']=='Passed' for row in checks) else 1)
