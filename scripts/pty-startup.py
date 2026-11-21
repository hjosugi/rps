#!/usr/bin/env python3
"""Measure time from spawn to the first output on a pseudo-terminal.

    scripts/pty-startup.py PROGRAM [ARG...]

Prints nanoseconds from `forkpty` to the first byte the program writes, then
sends "exit\\n" and tears the session down. Interactive shells write their
prompt first, so this is the prompt latency; its own Python startup is before
the clock starts. Used by `cargo harness bench --suite shells`.
"""

import os
import pty
import select
import signal
import sys
import time

TIMEOUT_SECONDS = 10.0
EXIT_WAIT_SECONDS = 2.0


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: pty-startup.py PROGRAM [ARG...]", file=sys.stderr)
        return 2
    argv = sys.argv[1:]
    start = time.monotonic_ns()
    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.execvp(argv[0], argv)
        except OSError as error:
            try:
                os.write(2, f"pty-startup: {error}\n".encode())
            except OSError:
                pass
            os._exit(127)

    try:
        first = None
        deadline = start + int(TIMEOUT_SECONDS * 1e9)
        while first is None:
            remaining = (deadline - time.monotonic_ns()) / 1e9
            if remaining <= 0:
                print("pty-startup: timed out", file=sys.stderr)
                return 2
            readable, _, _ = select.select([fd], [], [], remaining)
            if not readable:
                print("pty-startup: timed out", file=sys.stderr)
                return 2
            try:
                data = os.read(fd, 4096)
            except OSError:
                data = b""
            if data:
                first = time.monotonic_ns()
            else:
                print("pty-startup: no output before exit", file=sys.stderr)
                return 2

        try:
            os.write(fd, b"exit\n")
        except OSError:
            pass
        deadline = time.monotonic() + EXIT_WAIT_SECONDS
        while time.monotonic() < deadline:
            waited, _ = os.waitpid(pid, os.WNOHANG)
            if waited == pid:
                break
            time.sleep(0.01)
        print(first - start)
        return 0
    finally:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass
        try:
            os.close(fd)
        except OSError:
            pass


if __name__ == "__main__":
    sys.exit(main())
