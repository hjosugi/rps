#!/usr/bin/env python3
"""Counts .NET type literals in PowerShell scripts: a crude stand-in for the
corpus analysis of #18, used by docs/06_dotnet_bridge.md to sketch what the
native shim (T0) has to cover first.

    research/dotnet/type-literals.py [--top N] DIRECTORY...

A type literal is `[Name]` or `[Name[...]]` not preceded by an identifier,
`$`, `)` or `]` (which would make it an index) and not containing `(` (which
would make it an attribute). Names are lowercased and a leading `system.` is
dropped, as PowerShell lets a script drop it; accelerators are not mapped to
full names. Comments and strings are not excluded. `]::` after the
literal counts as static member access.
"""
import argparse
import collections
import os
import re

LITERAL = re.compile(r'(?<![\w$\])\]])\[([A-Za-z_][\w.]*)(\[[^\[\]()]*\])?\](::)?')


def scripts(directories):
    for directory in directories:
        for base, _, files in os.walk(directory):
            for name in files:
                if name.endswith(('.ps1', '.psm1')):
                    yield os.path.join(base, name)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--top', type=int, default=30)
    parser.add_argument('--exclude', action='append', default=[])
    parser.add_argument('directories', nargs='+')
    args = parser.parse_args()
    uses = collections.Counter()
    static = collections.Counter()
    files = 0
    for path in scripts(args.directories):
        if any(part in path for part in args.exclude):
            continue
        files += 1
        with open(path, encoding='utf-8', errors='replace') as file:
            text = file.read()
        for match in LITERAL.finditer(text):
            name = match.group(1).lower()
            if name.startswith('system.'):
                name = name[len('system.'):]
            if match.group(2):
                name += '[' + match.group(2)[1:-1].strip().lower() + ']'
            uses[name] += 1
            if match.group(3):
                static[name] += 1
    print(f'files={files} literals={sum(uses.values())} distinct={len(uses)}')
    for name, count in uses.most_common(args.top):
        print(f'{count:6} {static[name]:6}  {name}')


if __name__ == '__main__':
    main()
