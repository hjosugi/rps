#!/usr/bin/env python3
"""Analyse the fetched script corpus and write docs/08_corpus.md.

    scripts/fetch-corpus.sh
    scripts/analyze-corpus.py [--rpsh PATH]

The parser counts come from the reference pwsh (`analyze-corpus.ps1`); the
refusal counts come from running rpsh on every script. Because rpsh refuses
at the first unsupported construct, a refusal count is a lower bound on how
often that feature appears.
"""
import argparse
import collections
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / '.cache/corpus'


def version():
    try:
        return (ROOT / '.cache/pwsh/current').read_text().strip()
    except OSError:
        return 'unknown'


def pwsh():
    path = ROOT / '.cache/pwsh' / version() / 'pwsh'
    if not path.exists():
        raise SystemExit('no pwsh; run scripts/fetch-pwsh.sh')
    return path


def rpsh_path(explicit):
    if explicit:
        return Path(explicit)
    target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
    return target / 'release/rpsh'


def reference_env():
    return {
        'HOME': str(ROOT / '.cache/home'),
        'PATH': '/usr/local/bin:/usr/bin:/bin',
        'LANG': 'en_US.UTF-8',
        'LC_ALL': 'en_US.UTF-8',
        'TZ': 'UTC',
        'NO_COLOR': '1',
        'POWERSHELL_TELEMETRY_OPTOUT': '1',
        'POWERSHELL_UPDATECHECK': 'Off',
    }


def analyse_ast(source):
    out = ROOT / 'bench/results/corpus-ast.json'
    out.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [str(pwsh()), '-NoProfile', '-NonInteractive', '-File',
         str(ROOT / 'scripts/analyze-corpus.ps1'),
         '-Root', str(source), '-Out', str(out)],
        check=True, cwd=ROOT,
    )
    return json.loads(out.read_text(encoding='utf-8'))


def analyse_refusals(source, rpsh):
    features = collections.Counter()
    ran = timed_out = failed = 0
    files = sorted(source.rglob('*.ps1'))
    for index, path in enumerate(files, 1):
        if index % 50 == 0:
            print(f'  rpsh {index}/{len(files)}', file=sys.stderr)
        try:
            result = subprocess.run(
                [str(rpsh), '-NoProfile', '-NonInteractive', '-File', str(path)],
                cwd=path.parent, env=reference_env(), stdin=subprocess.DEVNULL,
                capture_output=True, timeout=2,
            )
        except subprocess.TimeoutExpired:
            timed_out += 1
            continue
        if result.returncode == 99:
            text = result.stderr.decode(errors='replace')
            marker = 'rpsh: not implemented: '
            if text.startswith(marker):
                line = text[len(marker):].splitlines()[0]
                feature = line.split(' (at offset ')[0]
                features[feature] += 1
            else:
                failed += 1
        elif result.returncode == 0:
            ran += 1
        else:
            failed += 1
    return features, ran, timed_out, failed


def table(counter, heading, limit=15):
    rows = [f'| {heading} | count |', '|---|---:|']
    for name, count in counter.most_common(limit):
        rows.append(f'| `{name}` | {count} |')
    return '\n'.join(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rpsh', help='rpsh binary (default: target/release/rpsh)')
    args = parser.parse_args()

    sources = sorted(CORPUS.glob('PowerShell-*/test/powershell'))
    if not sources:
        raise SystemExit('no corpus; run scripts/fetch-corpus.sh')
    source = sources[-1]
    print(f'corpus: {source}', file=sys.stderr)
    ast = analyse_ast(source)
    rpsh = rpsh_path(args.rpsh)
    if not rpsh.exists():
        raise SystemExit(f'no rpsh at {rpsh}; run cargo build --release -p rpsh-cli')
    features, ran, timed_out, failed = analyse_refusals(source, rpsh)

    nodes = collections.Counter(ast['nodes'])
    commands = collections.Counter(ast['commands'])
    types = collections.Counter(ast['types'])
    members = collections.Counter(ast['members'])
    operators = collections.Counter(ast['operators'])

    lines = [
        '# Script corpus',
        '',
        f'The first corpus is the upstream PowerShell test suite at the latest '
        f'stable release ({version()}), fetched by `scripts/fetch-corpus.sh` '
        f'and not committed. `scripts/analyze-corpus.py` parses every file with '
        f'the reference pwsh and runs rpsh on it.',
        '',
        f'* files: **{ast["files"]}**',
        f'* files with parse errors: **{ast["parse_errors"]}**',
        f'* rpsh ran the script: **{ran}**',
        f'* rpsh refused it by name: **{sum(features.values())}**',
        f'* rpsh timed out (2 s): **{timed_out}**',
        f'* rpsh exited with another failure: **{failed}**',
        '',
        'A refusal is the first unsupported construct in the file, so each '
        'count is a lower bound. Files that contain several missing features '
        'contribute only to the first one.',
        '',
        '## Most frequent refusals',
        '',
        table(features, 'feature', 20),
        '',
        '## Most frequent commands',
        '',
        table(commands, 'command', 20),
        '',
        '## Most frequent types (type literals)',
        '',
        table(types, 'type', 20),
        '',
        '## Most frequent members',
        '',
        table(members, 'member', 20),
        '',
        '## Most frequent operators',
        '',
        table(operators, 'operator', 20),
        '',
        '## Most frequent AST nodes',
        '',
        table(nodes, 'node', 20),
        '',
        '## Milestone alignment',
        '',
        'The refusal list is dominated by command invocation because the '
        'corpus is Pester (Should, It, Describe, Mock). The parser gaps '
        '(statement keywords, variables, type literals) come next. In the '
        'milestone order this is still M1 for the parser, then M3/M5 for '
        'command invocation and the cmdlet surface; no milestone is '
        'reordered by this table.',
        '',
        'The corpus is upstream tests, not PowerShell Gallery modules yet. '
        'Adding a pinned set of public modules is a follow-up; the fetch and '
        'analysis scripts do not change when it lands.',
    ]
    out = ROOT / 'docs/08_corpus.md'
    out.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    print(f'wrote {out}', file=sys.stderr)


if __name__ == '__main__':
    main()
