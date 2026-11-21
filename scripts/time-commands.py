#!/usr/bin/env python3
"""Times whole command lines: wall time, CPU time, peak RSS, page faults.

    scripts/time-commands.py bench/startup/stages.json [--runs N] [--warmup N]
                             [--filter TEXT] [--out FILE]

`cargo harness bench` compares pwsh and rpsh on scripts. This compares any
set of command lines, interleaved, in the reference environment of
docs/01_compat_contract.md, which is what startup questions need: stages of
pwsh's start, environment variables that change it, other shells.

A spec is JSON:

    {
      "description": "...",
      "runs": 30, "warmup": 3,
      "commands": [
        {"name": "true", "argv": ["/usr/bin/true"]},
        {"name": "pwsh, fresh HOME", "argv": ["{pwsh}", "-NoProfile", "-Command", "exit"],
         "env": {"DOTNET_TieredPGO": "0"}, "fresh_home": true, "metrics": true}
      ]
    }

`{root}`, `{pwsh}`, `{rpsh}` and `{probes}` in argv are replaced by the
checkout, the pinned pwsh, the release rpsh and .cache/probes. A command with
"metrics" prints `name=value` lines on stdout; their medians are reported.
"fresh_home" gives every run a new, empty HOME.

Each command runs under scripts/spawn-measure.c, compiled into .cache/tools
on first use, which forks it and reports its rusage: forked from Python, a
child's peak RSS would include Python's.

Rounds are interleaved and each round starts at a different command, so
that drift in the machine's state spreads over all of them. Every sample is
kept in the JSON output (schema rpsh-commands-v1).
"""
import argparse
import datetime
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PWSH_VERSION = '7.7.0'
REFERENCE_PATH = '/usr/local/bin:/usr/bin:/bin'
SPAWN_MEASURE = None


def placeholders():
    target = os.environ.get('CARGO_TARGET_DIR') or os.path.join(ROOT, 'target')
    return {
        '{root}': ROOT,
        '{pwsh}': os.path.join(ROOT, '.cache', 'pwsh', PWSH_VERSION, 'pwsh'),
        '{rpsh}': os.path.join(target, 'release', 'rpsh'),
        '{probes}': os.path.join(ROOT, '.cache', 'probes'),
    }


def spawn_measure():
    """Compiles scripts/spawn-measure.c when the binary is missing or stale."""
    source = os.path.join(ROOT, 'scripts', 'spawn-measure.c')
    binary = os.path.join(ROOT, '.cache', 'tools', 'spawn-measure')
    if not os.path.exists(binary) or os.path.getmtime(binary) < os.path.getmtime(source):
        os.makedirs(os.path.dirname(binary), exist_ok=True)
        compiler = os.environ.get('CC', 'cc')
        subprocess.run([compiler, '-O2', '-o', binary, source], check=True)
    return binary


def expand(value, table):
    for key, replacement in table.items():
        value = value.replace(key, replacement)
    return value


def reference_env(home):
    """The environment `cargo harness` gives both engines."""
    return {
        'HOME': home,
        'PATH': REFERENCE_PATH,
        'LANG': 'en_US.UTF-8',
        'LC_ALL': 'en_US.UTF-8',
        'TZ': 'UTC',
        'NO_COLOR': '1',
        'POWERSHELL_TELEMETRY_OPTOUT': '1',
        'POWERSHELL_UPDATECHECK': 'Off',
    }


def run_once(command, home_root):
    home = os.path.join(ROOT, '.cache', 'home')
    fresh = None
    if command.get('fresh_home'):
        fresh = tempfile.mkdtemp(prefix='home-', dir=home_root)
        home = fresh
    env = reference_env(home)
    env.update(command.get('env', {}))
    read_end, write_end = os.pipe()
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        child = subprocess.Popen([SPAWN_MEASURE, str(write_end)] + command['argv'],
                                 stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
                                 env=env, cwd=command.get('cwd', ROOT), pass_fds=(write_end,))
        os.close(write_end)
        with os.fdopen(read_end) as report_file:
            report = report_file.read().split()
        if child.wait() != 0 or len(report) != 7:
            stderr.seek(0)
            raise SystemExit(f"{command['name']}: spawn-measure failed\n"
                             f"{stderr.read().decode(errors='replace')}")
        stdout.seek(0)
        stderr.seek(0)
        out, err = stdout.read(), stderr.read()
    wall_ns, user_us, sys_us, max_rss, minor, major, status = (int(field) for field in report)
    exit_code = os.waitstatus_to_exitcode(status)
    if fresh:
        shutil.rmtree(fresh, ignore_errors=True)
    expected = command.get('exit', 0)
    if exit_code != expected:
        raise SystemExit(f"{command['name']}: exit {exit_code}, expected {expected}\n"
                         f"{err.decode(errors='replace')}")
    sample = {
        'wall_ms': wall_ns / 1e6,
        'user_ms': user_us / 1e3,
        'sys_ms': sys_us / 1e3,
        'max_rss_kib': max_rss,
        'minor_faults': minor,
        'major_faults': major,
    }
    if command.get('metrics'):
        metrics = {}
        for line in out.decode(errors='replace').splitlines():
            for part in line.split():
                name, sep, value = part.partition('=')
                if sep:
                    try:
                        metrics[name] = float(value)
                    except ValueError:
                        pass
        sample['metrics'] = metrics
    return sample


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
    governor = ''
    try:
        with open('/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor') as file:
            governor = file.read().strip()
    except OSError:
        pass
    commit = subprocess.run(['git', 'rev-parse', '--short', 'HEAD'], cwd=ROOT,
                            capture_output=True, text=True).stdout.strip()
    return {
        'cpu': cpu,
        'logical_cpus': os.cpu_count(),
        'governor': governor,
        'kernel': platform.release(),
        'load_average': list(os.getloadavg()),
        'commit': commit,
        'python': platform.python_version(),
    }


def summarize(samples):
    summary = {}
    for key in ('wall_ms', 'user_ms', 'sys_ms', 'max_rss_kib', 'minor_faults', 'major_faults'):
        values = [sample[key] for sample in samples]
        summary[key] = statistics.median(values)
    walls = sorted(sample['wall_ms'] for sample in samples)
    summary['wall_min_ms'] = walls[0]
    summary['wall_p90_ms'] = walls[min(len(walls) - 1, int(len(walls) * 0.9))]
    metrics = {}
    for sample in samples:
        for name, value in sample.get('metrics', {}).items():
            metrics.setdefault(name, []).append(value)
    if metrics:
        summary['metrics'] = {name: statistics.median(values) for name, values in metrics.items()}
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('spec')
    parser.add_argument('--runs', type=int)
    parser.add_argument('--warmup', type=int)
    parser.add_argument('--filter')
    parser.add_argument('--out')
    args = parser.parse_args()

    with open(args.spec) as file:
        spec = json.load(file)
    table = placeholders()
    commands = []
    for command in spec['commands']:
        if args.filter and args.filter not in command['name']:
            continue
        command = dict(command)
        command['argv'] = [expand(part, table) for part in command['argv']]
        if 'cwd' in command:
            command['cwd'] = expand(command['cwd'], table)
        if not os.path.exists(command['argv'][0]):
            raise SystemExit(f"{command['name']}: {command['argv'][0]} does not exist")
        commands.append(command)
    if not commands:
        raise SystemExit('no command matches')
    runs = args.runs or spec.get('runs', 20)
    warmup = args.warmup if args.warmup is not None else spec.get('warmup', 2)

    global SPAWN_MEASURE
    SPAWN_MEASURE = spawn_measure()
    os.makedirs(os.path.join(ROOT, '.cache', 'home'), exist_ok=True)
    home_root = tempfile.mkdtemp(prefix='rpsh-time-')
    before = machine()
    samples = {command['name']: [] for command in commands}
    try:
        for command in commands:
            for _ in range(warmup):
                run_once(command, home_root)
        for round_index in range(runs):
            shift = round_index % len(commands)
            for command in commands[shift:] + commands[:shift]:
                samples[command['name']].append(run_once(command, home_root))
            print(f'\rround {round_index + 1}/{runs}', end='', file=sys.stderr, flush=True)
        print(file=sys.stderr)
    finally:
        shutil.rmtree(home_root, ignore_errors=True)

    results = []
    width = max(len(command['name']) for command in commands)
    print(f"{'command':<{width}}  {'wall ms':>9} {'min':>8} {'p90':>8} {'cpu ms':>8} "
          f"{'RSS MiB':>8} {'minflt':>7}")
    for command in commands:
        summary = summarize(samples[command['name']])
        results.append({
            'name': command['name'],
            'argv': [part.replace(ROOT, '{root}') for part in command['argv']],
            'env': command.get('env', {}),
            'fresh_home': bool(command.get('fresh_home')),
            'summary': summary,
            'samples': samples[command['name']],
        })
        print(f"{command['name']:<{width}}  {summary['wall_ms']:9.2f} {summary['wall_min_ms']:8.2f} "
              f"{summary['wall_p90_ms']:8.2f} {summary['user_ms'] + summary['sys_ms']:8.1f} "
              f"{summary['max_rss_kib'] / 1024:8.1f} {summary['minor_faults']:7.0f}")
        for name, value in summary.get('metrics', {}).items():
            print(f"{'':<{width}}    {name} = {value:g}")

    document = {
        'schema': 'rpsh-commands-v1',
        'spec': os.path.relpath(os.path.abspath(args.spec), ROOT),
        'description': spec.get('description', ''),
        'date': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
        'runs': runs,
        'warmup': warmup,
        'machine': before,
        'load_average_after': list(os.getloadavg()),
        'results': results,
    }
    out = args.out or os.path.join(ROOT, 'bench', 'results', 'commands-latest.json')
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, 'w') as file:
        json.dump(document, file, indent=1)
        file.write('\n')
    print(f'wrote {os.path.relpath(out, ROOT)}')


if __name__ == '__main__':
    main()
