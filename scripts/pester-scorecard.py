#!/usr/bin/env python3
"""Render the upstream Pester scorecard skeleton.

    scripts/pester-scorecard.py [--results FILE] [--out FILE]

The areas and file counts are the classification in tests/pester/areas.tsv.
Until Pester runs on rpsh, every result is zero and the sheet is explicitly
marked as a skeleton. A future runner writes a JSON result keyed by area:

    {"Modules": {"tests": 1201, "passed": 10, "failed": 2, "skipped": 80, "blocked": 1109}}

`cargo harness` does not run Pester; this script only formats results so the
CI artifact and the plan in docs/07_pester_plan.md use one shape.
"""
import argparse
import csv
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
AREAS = os.path.join(ROOT, 'tests', 'pester', 'areas.tsv')


def load_areas():
    with open(AREAS, newline='', encoding='utf-8') as handle:
        return list(csv.DictReader(handle, delimiter='\t'))


def load_results(path):
    if path is None:
        return {}
    with open(path, encoding='utf-8') as handle:
        return json.load(handle)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', help='JSON scorecard results by area')
    parser.add_argument('--out', help='write the markdown here instead of stdout')
    args = parser.parse_args()

    areas = load_areas()
    results = load_results(args.results)
    skeleton = not results

    lines = [
        '# Pester scorecard',
        '',
        'The upstream suite is not run yet. This sheet is the format the runner '
        'will fill; see [docs/07_pester_plan.md](../docs/07_pester_plan.md).',
        '',
        '| area | files | tests | pass | fail | skip | blocked | pass rate |',
        '|---|---:|---:|---:|---:|---:|---:|---:|',
    ]
    total_files = 0
    total = {'tests': 0, 'passed': 0, 'failed': 0, 'skipped': 0, 'blocked': 0}
    for area in areas:
        name = area['area']
        files = int(area['files'])
        total_files += files
        counts = results.get(name, {})
        row = {key: int(counts.get(key, 0)) for key in total}
        for key in total:
            total[key] += row[key]
        ran = row['passed'] + row['failed']
        rate = 'n/a' if ran == 0 else f"{row['passed'] * 100.0 / ran:.1f}%"
        lines.append(
            f"| {name} | {files} | {row['tests']} | {row['passed']} | "
            f"{row['failed']} | {row['skipped']} | {row['blocked']} | {rate} |"
        )
    ran = total['passed'] + total['failed']
    rate = 'n/a' if ran == 0 else f"{total['passed'] * 100.0 / ran:.1f}%"
    lines.append(
        f"| **total** | **{total_files}** | **{total['tests']}** | "
        f"**{total['passed']}** | **{total['failed']}** | **{total['skipped']}** | "
        f"**{total['blocked']}** | **{rate}** |"
    )
    if skeleton:
        lines.extend(['', 'Skeleton only: no Pester results were supplied.'])
    text = '\n'.join(lines) + '\n'
    if args.out:
        with open(args.out, 'w', encoding='utf-8') as handle:
            handle.write(text)
        print(f'wrote {args.out}')
    else:
        sys.stdout.write(text)


if __name__ == '__main__':
    main()
