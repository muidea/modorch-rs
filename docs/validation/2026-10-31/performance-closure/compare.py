#!/usr/bin/env python3
"""Run alternating release A/B samples; builds and other checks must finish first.
Usage: compare.py BASELINE_BINARY_DIR OPTIMIZED_PROBE [--skip-dhat]
The baseline source must be the pre-closure version described in environment.json.
"""
import hashlib
import json
import os
from pathlib import Path
import resource
import statistics
import subprocess
import sys
import time

out = Path(__file__).resolve().parent
repo = out.parents[3]
baseline = Path(sys.argv[1]).resolve()
probe = Path(sys.argv[2]).resolve()
load = repo / 'target/release/runtime-load'

def sample(name, command):
    cpu_start = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.monotonic()
    process = subprocess.Popen([str(x) for x in command], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    rss = 0
    while process.poll() is None:
        try:
            for line in Path(f'/proc/{process.pid}/status').read_text().splitlines():
                if line.startswith('VmRSS:'):
                    rss = max(rss, int(line.split()[1]))
        except FileNotFoundError:
            pass
        if time.monotonic() - start > (15 if 'saturation' in command else 120):
            process.kill()
            raise RuntimeError(f'timeout: {name}')
        time.sleep(.025)
    stdout, stderr = process.communicate()
    if process.returncode:
        raise RuntimeError(f'{name}: {stderr}\n{stdout}')
    result = json.loads(stdout)
    cpu_end = resource.getrusage(resource.RUSAGE_CHILDREN)
    result['external_sample'] = dict(rss_peak_kib=rss, user_seconds=cpu_end.ru_utime-cpu_start.ru_utime,
        system_seconds=cpu_end.ru_stime-cpu_start.ru_stime, wall_seconds=time.monotonic()-start)
    (out / f'{name}.json').write_text(json.dumps(result, indent=2)+'\n')
    return result

results = {}
for subscriptions in [1, 128, 1024]:
    for trial in range(3):
        variants = [('baseline', baseline/'modorch-perf-review'), ('optimized', probe)]
        if trial % 2: variants.reverse()
        for label, binary in variants:
            name = f'route-{subscriptions}-{label}-{trial+1}'
            results[name] = sample(name, [binary, 'route', subscriptions, 2000])
for mode in ['serial', 'partitioned', 'concurrent', 'dynamic']:
    for producers in [1, 64]:
        for trial in range(3):
            variants = [('baseline', baseline/'runtime-load'), ('optimized', load)]
            if trial % 2: variants.reverse()
            for label, binary in variants:
                name = f'{mode}-p{producers}-{label}-{trial+1}'
                results[name] = sample(name, [binary, '--mode', mode, '--seconds', 2, '--producers', producers, '--capacity', 8])
for label, binary in [('baseline', baseline/'modorch-perf-review'), ('optimized', probe)]:
    results[f'observation-{label}'] = sample(f'observation-{label}', [binary, 'observation'])
for case in ['saturation', 'mixed']:
    for trial in range(3):
        modes = ['inline','isolated']
        if trial % 2: modes.reverse()
        for mode in modes:
            name = f'framework-{case}-{mode}-{trial+1}'
            results[name] = sample(name, [probe, case, mode])
if '--skip-dhat' not in sys.argv:
    for mode in ['serial', 'concurrent']:
        for label, binary in [('baseline', baseline/'runtime-load'), ('optimized',load)]:
            name = f'dhat-{mode}-{label}'
            result = subprocess.run(['valgrind','--tool=dhat',f'--dhat-out-file={out/name}.json',str(binary),'--mode',mode,'--messages','1000','--producers','8','--capacity','8'], text=True,capture_output=True,check=True,timeout=120)
            (out/f'{name}.txt').write_text(result.stderr)
            (out/f'{name}-load.json').write_text(result.stdout)

summary = {}
for subscriptions in [1,128,1024]:
    for label in ['baseline','optimized']:
        summary[f'route-{subscriptions}-{label}'] = statistics.median(results[f'route-{subscriptions}-{label}-{i}']['requests_per_second'] for i in [1,2,3])
summary['samples'] = results
(out/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
environment = dict(date=time.strftime('%Y-%m-%d'), uname=os.uname(), rust=subprocess.check_output(['rustc','-Vv'],text=True),
    command=sys.argv, source_sha256=hashlib.sha256(b''.join(p.read_bytes() for p in sorted((repo/'crates').rglob('*.rs')))).hexdigest(),
    binaries={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [baseline/'runtime-load',baseline/'modorch-perf-review',load,probe]},
    caveat='Shared WSL2 host, short synthetic samples; CPU/RSS include harness and runtime, DHAT times are not throughput.')
(out/'environment.json').write_text(json.dumps(environment, indent=2)+'\n')
print(json.dumps({k:v for k,v in summary.items() if k!='samples'}, indent=2))
