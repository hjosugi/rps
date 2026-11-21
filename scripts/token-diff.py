#!/usr/bin/env python3
"""Compare rpsh's full tokenizer with the reference parser's token stream.

    scripts/token-diff.py [--cases FILE] [--rpsh-bin PATH]

The comparison is kind and byte extent, the same fields #22 names. It needs
the reference pwsh (scripts/fetch-pwsh.sh); the corpus is tests/tokenizer/cases.json.
"""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def pwsh():
    version = (ROOT / '.cache/pwsh/current').read_text().strip()
    return ROOT / '.cache/pwsh' / version / 'pwsh'


def rpsh_dump():
    subprocess.run(['cargo', 'build', '-q', '-p', 'rpsh-syntax', '--bin', 'token-dump'],
                   cwd=ROOT, check=True)
    metadata = subprocess.run(['cargo', 'metadata', '--format-version', '1', '--no-deps'],
                              cwd=ROOT, capture_output=True, check=True)
    target = Path(json.loads(metadata.stdout)['target_directory'])
    return target / 'debug/token-dump' 


def lines(command, source):
    result = subprocess.run(command, input=source.encode(), capture_output=True, check=True, timeout=5)
    return result.stdout.decode().strip().splitlines()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cases', default=str(ROOT / 'tests/tokenizer/cases.json'))
    args = parser.parse_args()
    cases = json.loads(Path(args.cases).read_text(encoding='utf-8'))
    reference = [str(pwsh()), '-NoProfile', '-NonInteractive', '-File',
                 str(ROOT / 'scripts/token-dump.ps1'), '-Source']
    ours = [str(rpsh_dump())]
    failures = 0
    for case in cases:
        print(f"checking {case['name']}", file=sys.stderr)
        expected = lines(reference + [case['source']], case['source'])
        actual = lines(ours, case['source'])
        if expected != actual:
            failures += 1
            print(f"FAIL {case['name']}")
            for index in range(max(len(expected), len(actual))):
                left = expected[index] if index < len(expected) else '<missing>'
                right = actual[index] if index < len(actual) else '<missing>'
                if left != right:
                    print(f"  {index}: reference {left!r} / rpsh {right!r}")
    print(f'{len(cases) - failures}/{len(cases)} token streams match')
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
