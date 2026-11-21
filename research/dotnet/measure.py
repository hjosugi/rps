#!/usr/bin/env python3
"""Runs research/dotnet's hostfxr probe several times per mode and prints the
median of every step (docs/06_dotnet_bridge.md).

    research/dotnet/build.sh
    research/dotnet/measure.py [--runs N] [--out FILE]

The probe runs in the reference environment of docs/01_compat_contract.md.
Writes bench/results/research-dotnet_t1.json.
"""
import argparse
import json
import os
import platform
import statistics
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
try:
    with open(os.path.join(ROOT, '.cache', 'pwsh', 'current')) as marker:
        _version = marker.read().strip()
except OSError:
    _version = ''
PWSH_DIR = os.path.join(ROOT, '.cache', 'pwsh', _version)
PROBE_DIR = os.path.join(ROOT, '.cache', 'dotnet-probe')
if not os.path.isfile(os.path.join(PWSH_DIR, 'pwsh')):
    raise SystemExit('no pwsh in .cache/pwsh; run scripts/fetch-pwsh.sh')


def machine():
    cpu = ''
    try:
        with open('/proc/cpuinfo') as info:
            for line in info:
                if line.startswith('model name'):
                    cpu = line.split(':', 1)[1].strip()
                    break
    except OSError:
        pass
    commit = subprocess.run(['git', 'rev-parse', '--short', 'HEAD'], cwd=ROOT,
                            capture_output=True, text=True).stdout.strip()
    return {
        'cpu': cpu,
        'kernel': platform.release(),
        'commit': commit,
        'python': platform.python_version(),
    }


def run(mode):
    env = {
        'HOME': os.path.join(ROOT, '.cache', 'home'),
        'PATH': '/usr/local/bin:/usr/bin:/bin',
        'LANG': 'en_US.UTF-8', 'LC_ALL': 'en_US.UTF-8', 'TZ': 'UTC',
    }
    result = subprocess.run(
        [os.path.join(PROBE_DIR, 'hostfxr-probe'), mode, PWSH_DIR, os.path.join(PROBE_DIR, 'probe.dll')],
        capture_output=True, text=True, env=env, stdin=subprocess.DEVNULL)
    steps = []
    for line in result.stdout.splitlines():
        if not line.startswith('step '):
            continue
        name, _, fields = line[5:].partition(' rc=')
        values = dict(field.split('=') for field in ('rc=' + fields).split())
        steps.append({'step': name.strip(), 'rc': values['rc'], 'at_ms': float(values['at_ms']),
                      'delta_ms': float(values['delta_ms']), 'rss_kib': int(values['rss_kib'])})
    return result.returncode, steps, result.stderr.strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--runs', type=int, default=11)
    parser.add_argument('--out', default=os.path.join(ROOT, 'bench', 'results', 'research-dotnet_t1.json'))
    args = parser.parse_args()
    document = {'schema': 'rpsh-research-dotnet-t1-v1', 'runs': args.runs,
                'machine': machine(), 'modes': {}}
    for mode in ('cmdline', 'default', 'runtimeconfig', 'named'):
        runs = [run(mode) for _ in range(args.runs)]
        exit_code, first_steps, stderr = runs[0]
        names = [step['step'] for step in first_steps]
        summary = []
        print(f'== {mode}: exit {exit_code}' + (f' ({stderr})' if stderr else ''))
        for index, name in enumerate(names):
            samples = [steps[index] for _, steps, _ in runs if len(steps) > index]
            row = {
                'step': name,
                'rc': samples[0]['rc'],
                'delta_ms': statistics.median(s['delta_ms'] for s in samples),
                'at_ms': statistics.median(s['at_ms'] for s in samples),
                'rss_kib': statistics.median(s['rss_kib'] for s in samples),
            }
            summary.append(row)
            print(f"  {name:<28} rc={row['rc']:<20} +{row['delta_ms']:8.2f} ms  "
                  f"at {row['at_ms']:8.2f} ms  RSS {row['rss_kib'] / 1024:6.1f} MiB")
        document['modes'][mode] = {'exit': exit_code, 'stderr': stderr, 'steps': summary,
                                   'samples': [steps for _, steps, _ in runs]}
    with open(os.path.join('/proc/loadavg')) as file:
        document['load_average_after'] = file.read().split()[:3]
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, 'w') as file:
        json.dump(document, file, indent=1)
        file.write('\n')
    print(f'wrote {os.path.relpath(args.out, ROOT)}')


if __name__ == '__main__':
    main()
