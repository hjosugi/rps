#!/usr/bin/env python3
"""Compare installed Linux command-line programs under PowerShell and rpsh."""
import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
BULK = b'large-matrix-payload\n' * 65536


@dataclass(frozen=True)
class Scenario:
    program: str
    title: str
    script: str
    expected: str  # full-match regex on stdout, or on the final PTY screen
    exit_code: int = 0
    terminal_input: tuple[str, str] | None = None  # visible text, then keys


CASES = [
    Scenario('git', 'Space in a configuration value',
             "git -c 'matrix.message=two words' config --get matrix.message", 'two words\n'),
    Scenario('ssh', 'Resolve user and host options offline',
             "ssh -T -F /dev/null -G -l matrix-user localhost | grep '^user '", 'user matrix-user\n'),
    Scenario('scp', 'Copy between local paths with spaces',
             "scp -q 'input file.txt' 'copy file.txt' && cat 'copy file.txt'", 'alpha\nbeta\n'),
    Scenario('sudo', 'Read the installed version without prompting',
             'sudo -V | head -n 1', r'(?:Sudo version |sudo-rs )[0-9][^\n]*\n'),
    Scenario('vim', 'Edit and exit on a terminal',
             """vim -Nu NONE -n -i NONE -- 'input file.txt'
$status=$LASTEXITCODE
"editor-exit=$status"
exit $status""", r'(?s).*editor-exit=0\s*', terminal_input=('alpha', ':q!\r')),
    Scenario('less', 'Display a file on a terminal and quit',
             """less -X -P matrix-pager 'page.txt'
$status=$LASTEXITCODE
"pager-exit=$status"
exit $status""", r'(?s).*matrix-page.*pager-exit=0\s*', terminal_input=('matrix-page 000', 'q')),
    Scenario('man', 'Render a local manual page through a pipe',
             "man -P cat -l matrix.1 | grep -o matrix-man-ok", 'matrix-man-ok\n'),
    Scenario('fzf', 'Filter input from a native pipeline',
             "cat 'input file.txt' | fzf --filter=beta", 'beta\n'),
    Scenario('tmux', 'Create and inspect an isolated server',
             "tmux -S ./matrix.sock -f /dev/null new-session -d -s matrix 'sleep 15' && "
             "tmux -S ./matrix.sock list-sessions -F '#{session_name}'",
             'matrix\n'),
    Scenario('top', 'Read one batch-mode process row',
             """top -b -n 1 -p $PID | awk '$1 ~ /^[0-9]+$/ { found++ } END { print found+0 }'""",
             '1\n'),
    Scenario('docker', 'Read the client context without a daemon',
             'docker --config ./docker-config context show', 'default\n'),
    Scenario('kubectl', 'Read a local kubeconfig without contacting a cluster',
             "kubectl --kubeconfig 'cluster config.yml' config current-context", 'matrix-context\n'),
    Scenario('make', 'Pass a session environment variable to a recipe',
             "$env:RPSH_MATRIX_MESSAGE='value with spaces'\nmake -s -f 'rules file.mk'",
             'value with spaces\n'),
    Scenario('cargo', 'Locate an offline manifest with a space in its path',
             "cargo locate-project --manifest-path 'crate dir/Cargo.toml' --message-format plain | sed 's|.*/||'",
             r'Cargo\.toml\n'),
    Scenario('curl', 'Read a file URL from the environment',
             'curl --fail --silent --show-error $env:RPSH_MATRIX_URL', 'alpha\nbeta\n'),
    Scenario('tar', 'Archive and list a filename with spaces',
             "tar -cf bundle.tar 'input file.txt' && tar -tf bundle.tar", r'input file\.txt\n'),
    Scenario('grep', 'Preserve a nonzero exit status',
             "grep -F absent 'input file.txt'\nexit $LASTEXITCODE", '', exit_code=1),
    Scenario('grep', 'Drain over a megabyte through a native pipe',
             "grep -F matrix-payload bulk.txt | wc -c", rf'\s*{len(BULK)}\n'),
    Scenario('sed', 'Apply a quoted edit script', "sed 's/:/ /g' columns.txt", 'a one\nb two\n'),
    Scenario('awk', 'Pass a separator and a literal field expression',
             """awk -F: '{print $2}' columns.txt""", 'one\ntwo\n'),
    Scenario('xargs', 'Carry empty and space-containing NUL-delimited arguments',
             r"""printf '%s\0' 'a b' '' '--flag' | xargs -0 -n 1 printf '<%s>\n'""",
             '<a b>\n<>\n<--flag>\n'),
    Scenario('find', 'Match a quoted wildcard below a relative root',
             "find tree -type f -name '*.txt' -print | sort", r'tree/a b\.txt\ntree/z\.txt\n'),
    Scenario('sh', 'Observe a child terminated by SIGTERM',
             "sh -c 'kill -TERM \"$$\"'\n" '"$LASTEXITCODE|$?"', r'143\|False\n'),
]


def prepare(directory, program):
    for relative, contents in {
        'input file.txt': 'alpha\nbeta\n',
        'page.txt': ''.join(f'matrix-page {i:03d}\n' for i in range(64)),
        'columns.txt': 'a:one\nb:two\n',
        'rules file.mk': 'all:\n\t@printf "%s\\n" "$$RPSH_MATRIX_MESSAGE"\n',
        'matrix.1': '.TH MATRIX 1 "2000-01-01"\n.SH NAME\nmatrix-man-ok\n',
        'cluster config.yml': ('apiVersion: v1\nkind: Config\ncurrent-context: matrix-context\n'
                               'contexts:\n- name: matrix-context\n  context: {}\n'),
        'crate dir/Cargo.toml': '[package]\nname="matrix"\nversion="0.1.0"\nedition="2021"\n',
        'crate dir/src/lib.rs': '',
        'tree/a b.txt': '',
        'tree/z.txt': '',
        'tree/ignored.bin': '',
    }.items():
        path = directory / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents, encoding='utf-8')
    if program == 'grep':
        (directory / 'bulk.txt').write_bytes(BULK)
    for subdir in ['home', 'cargo', 'docker-config', 'config', 'cache', 'data', 'tmp']:
        (directory / subdir).mkdir()


def environment(directory):
    env = dict(os.environ)
    for key in ['TMUX', 'DOCKER_HOST', 'DOCKER_CONTEXT', 'FZF_DEFAULT_COMMAND',
                'FZF_DEFAULT_OPTS', 'LESS', 'MANOPT', 'VIMINIT', 'EXINIT', 'GIT_DIR',
                'GIT_WORK_TREE', 'GIT_CONFIG_COUNT', 'MAKEFLAGS']:
        env.pop(key, None)
    env.setdefault('RUSTUP_HOME', str(Path.home() / '.rustup'))
    env.update({
        'HOME': str(directory / 'home'), 'CARGO_HOME': str(directory / 'cargo'),
        'TMPDIR': str(directory / 'tmp'), 'XDG_CONFIG_HOME': str(directory / 'config'),
        'XDG_CACHE_HOME': str(directory / 'cache'), 'XDG_DATA_HOME': str(directory / 'data'),
        'LC_ALL': 'C', 'LANG': 'C', 'TERM': 'xterm-256color', 'NO_COLOR': '1',
        'PAGER': 'cat', 'MANPAGER': 'cat', 'MANWIDTH': '72', 'LESSHISTFILE': '-',
        'CARGO_NET_OFFLINE': 'true', 'GIT_CONFIG_NOSYSTEM': '1', 'SHELL': '/bin/sh',
        'RPSH_MATRIX_URL': (directory / 'input file.txt').as_uri(),
        'POWERSHELL_TELEMETRY_OPTOUT': '1', 'POWERSHELL_UPDATECHECK': 'Off',
    })
    return env


def execute(case, engine, kind, directory):
    prepare(directory, case.program)
    script = directory / 'scenario.ps1'
    script.write_text(case.script + '\n', encoding='utf-8')
    env = environment(directory)
    args = ['-NoLogo', '-NoProfile', '-NonInteractive', '-File', script.name]
    command = [str(engine), *args]
    if case.terminal_input:
        visible, keys = case.terminal_input
        manifest = directory / 'terminal.json'
        manifest.write_text(json.dumps({
            'program': '{engine}', 'args': args, 'cwd': str(directory), 'cols': 72,
            'rows': 12, 'timeout_ms': 8000,
            'input': [{'after_text': visible, 'send': keys}],
        }), encoding='utf-8')
        command = [sys.executable, str(ROOT / 'scripts/pty-run.py'), str(manifest),
                   '--engine', kind, '--' + kind, str(engine)]
    try:
        result = subprocess.run(command, cwd=directory, env=env, input=b'', capture_output=True,
                                timeout=15, check=False)
    finally:
        if case.program == 'tmux' and (directory / 'matrix.sock').exists():
            subprocess.run(['tmux', '-S', str(directory / 'matrix.sock'), 'kill-server'],
                           env=env, capture_output=True, timeout=5, check=False)
    if case.terminal_input:
        if result.returncode:
            raise RuntimeError(f'{kind} PTY driver failed: {result.stderr.decode(errors="replace").strip()}')
        terminal = json.loads(result.stdout)
        if terminal['timed_out']:
            raise RuntimeError(f'{kind} terminal timed out, final screen {terminal["screen"]!r}')
        return terminal['exit_code'], terminal['screen'].encode(), result.stderr
    return result.returncode, result.stdout, result.stderr


def sample(data):
    return repr(data[:180])


def compare(case, reference, candidate):
    runs = []
    for kind, engine in [('pwsh', reference), ('rpsh', candidate)]:
        with tempfile.TemporaryDirectory(prefix=f'rpsh-matrix-{case.program}-') as work:
            directory = Path(work).resolve()
            code, stdout, stderr = execute(case, engine, kind, directory)
            token = os.fsencode(directory)
            runs.append((code, stdout.replace(token, b'<WORK>'), stderr.replace(token, b'<WORK>')))
    code, stdout, stderr = runs[0]
    if code != case.exit_code or re.fullmatch(case.expected.encode(), stdout) is None:
        return f'PowerShell exited {code}, stdout {sample(stdout)}, stderr {sample(stderr)}'
    for field, wanted, got in zip(['exit code', 'stdout', 'stderr'], runs[0], runs[1]):
        if wanted != got:
            fmt = sample if isinstance(wanted, bytes) else str
            return f'{field} differed, pwsh {fmt(wanted)}, rpsh {fmt(got)}'
    return ''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pwsh', type=Path)
    parser.add_argument('--rpsh', type=Path)
    parser.add_argument('--report', type=Path, help='Also save the Markdown matrix to this file')
    args = parser.parse_args()
    if not sys.platform.startswith('linux'):
        parser.error('this matrix requires Linux')
    reference = args.pwsh
    if reference is None:
        cache = ROOT / '.cache/pwsh'
        reference = cache / (cache / 'current').read_text().strip() / 'pwsh'
    candidate = args.rpsh
    if candidate is None:
        metadata = subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=ROOT)
        candidate = Path(json.loads(metadata)['target_directory']) / 'release/rpsh'
    reference, candidate = reference.resolve(), candidate.resolve()
    for executable in [reference, candidate]:
        if not executable.is_file() or not os.access(executable, os.X_OK):
            parser.error(f'engine is unavailable: {executable}')
    versions = [subprocess.check_output([engine, '--version'], text=True).strip()
                for engine in (reference, candidate)]
    rows = ['## Linux native command compatibility', '', ' / '.join(versions), '',
            '| Program | Scenario | Result |', '|---|---|---|']
    failures = []
    counts = {'PASS': 0, 'FAIL': 0, 'SKIP': 0}
    for case in CASES:
        if shutil.which(case.program) is None:
            status, note = 'SKIP', 'not installed'
        else:
            try:
                note = compare(case, reference, candidate)
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
                note = str(error)
            status = 'FAIL' if note else 'PASS'
        counts[status] += 1
        rows.append(f'| {case.program} | {case.title} | {status}' + (' (not installed)' if status == 'SKIP' else '') + ' |')
        if status == 'FAIL':
            failures.append(f'{case.program} / {case.title}: {note}')
    rows += ['', ', '.join(f'{n} {s.lower()}' for s, n in counts.items()) + '.']
    if failures:
        rows += ['', '```text', *failures, '```']
    report = '\n'.join(rows) + '\n'
    print(report, end='')
    if args.report:
        args.report.write_text(report, encoding='utf-8')
    return int(bool(failures) or not counts['PASS'])


if __name__ == '__main__':
    sys.exit(main())
