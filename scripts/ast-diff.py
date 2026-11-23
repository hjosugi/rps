#!/usr/bin/env python3
"""Compare rpsh's normalized AST with the reference parser's AST.

    scripts/ast-diff.py [--cases FILE]

Both sides print the S-expression in scripts/ast-dump.ps1 and
crates/rpsh-syntax/src/bin/ast-dump.rs. The cases cover the expression forms
implemented so far; see docs/ for the remaining AST nodes.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def pwsh():
    version = (ROOT / '.cache/pwsh/current').read_text().strip()
    return ROOT / '.cache/pwsh' / version / 'pwsh'


def ours():
    subprocess.run(['cargo', 'build', '-q', '-p', 'rpsh-syntax', '--bin', 'ast-dump'],
                   cwd=ROOT, check=True)
    metadata = subprocess.run(['cargo', 'metadata', '--format-version', '1', '--no-deps'],
                              cwd=ROOT, capture_output=True, check=True)
    target = Path(json.loads(metadata.stdout)['target_directory'])
    return target / 'debug/ast-dump'


def dump(command, source):
    return subprocess.run(command, input=source.encode(), capture_output=True,
                          check=True, timeout=10).stdout.decode().strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cases', default=str(ROOT / 'tests/tokenizer/ast-cases.json'))
    args = parser.parse_args()
    cases = json.loads(Path(args.cases).read_text(encoding='utf-8'))
    reference = [str(pwsh()), '-NoProfile', '-NonInteractive', '-File',
                 str(ROOT / 'scripts/ast-dump.ps1'), '-Source']
    binary = [str(ours())]
    failures = 0
    for case in cases:
        expected = dump(reference + [case['source']], case['source'])
        actual = dump(binary, case['source'])
        if expected != actual:
            failures += 1
            print(f"FAIL {case['name']}")
            print(f"  reference: {expected}")
            print(f"  rpsh:      {actual}")
    print(f'{len(cases) - failures}/{len(cases)} ASTs match')
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
