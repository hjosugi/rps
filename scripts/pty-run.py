#!/usr/bin/env python3
"""Run one PTY conformance case and print its final screen as JSON.

    scripts/pty-run.py --engine pwsh --pwsh PATH --rpsh PATH SPEC.json

The spec is `tests/conformance/pty/*.json`:

    {
      "program": "{engine}", "args": ["-NoLogo", "-NoExit", "-Command", "..."],
      "input": [{"after_text": "PS>", "send": "exit\\r"}],
      "timeout_ms": 5000, "cols": 80, "rows": 24
    }

`{engine}` is replaced by the pwsh or rpsh path. The runner forks a PTY,
feeds input after a delay or when text appears on screen, interprets ANSI escapes into a small
screen model and prints `{"exit_code", "screen", "timed_out"}`. The caller
(the Rust harness) compares that JSON with the recorded golden.
"""

import argparse
import codecs
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import time


class Screen:
    def __init__(self, cols, rows):
        self.cols = cols
        self.rows = rows
        self.grid = [[" "] * cols for _ in range(rows)]
        self.row = 0
        self.col = 0
        self.saved = None
        self.state = "text"
        self.buffer = ""
        self.alt = None

    def clear(self):
        self.grid = [[" "] * self.cols for _ in range(self.rows)]
        self.row = self.col = 0

    def put(self, char):
        if char == "\r":
            self.col = 0
        elif char == "\n":
            self.linefeed()
        elif char == "\b":
            self.col = max(0, self.col - 1)
        elif char == "\t":
            self.col = min(self.cols - 1, (self.col // 8 + 1) * 8)
        elif char >= " ":
            self.grid[self.row][self.col] = char
            self.col += 1
            if self.col >= self.cols:
                self.col = 0
                self.linefeed()

    def linefeed(self):
        self.row += 1
        if self.row >= self.rows:
            self.grid.pop(0)
            self.grid.append([" "] * self.cols)
            self.row = self.rows - 1

    def feed(self, text):
        for char in text:
            if self.state == "osc":
                if char == "\x07":
                    self.state = "text"
                elif char == "\x1b":
                    self.state = "osc_esc"
                else:
                    self.buffer += char
                continue
            if self.state == "osc_esc":
                if char == "\\":
                    self.state = "text"
                else:
                    self.buffer += "\x1b" + char
                    self.state = "osc"
                continue
            if self.state == "esc":
                if char == "[":
                    self.state = "csi"
                    self.buffer = ""
                elif char == "]":
                    self.state = "osc"
                    self.buffer = ""
                elif char == "7":
                    self.saved = (self.row, self.col)
                    self.state = "text"
                elif char == "8":
                    if self.saved is not None:
                        self.row, self.col = self.saved
                    self.state = "text"
                elif char == "c":
                    self.clear()
                    self.state = "text"
                else:
                    self.state = "text"
                continue
            if self.state == "csi":
                self.buffer += char
                if "\x40" <= char <= "\x7e":
                    self.csi(self.buffer[:-1], char)
                    self.state = "text"
                    self.buffer = ""
                continue
            if char == "\x1b":
                self.state = "esc"
            elif char in "\r\n\b\t" or char >= " ":
                self.put(char)

    def csi(self, body, final):
        private = body.startswith("?")
        if private:
            body = body[1:]
        # Device queries, modified-key requests and SGR subparameters do not
        # change any cursor or screen state modeled here.
        if any(char not in "0123456789;" for char in body):
            return
        values = [int(value) if value else 0 for value in body.split(";")] if body else []
        first = values[0] if values else 1
        if final in "Hf":
            row = (values[0] if len(values) > 0 and values[0] else 1) - 1
            col = (values[1] if len(values) > 1 and values[1] else 1) - 1
            self.row = max(0, min(self.rows - 1, row))
            self.col = max(0, min(self.cols - 1, col))
        elif final == "A":
            self.row = max(0, self.row - (first or 1))
        elif final == "B":
            self.row = min(self.rows - 1, self.row + (first or 1))
        elif final == "C":
            self.col = min(self.cols - 1, self.col + (first or 1))
        elif final == "D":
            self.col = max(0, self.col - (first or 1))
        elif final == "G":
            self.col = max(0, min(self.cols - 1, (first or 1) - 1))
        elif final == "d":
            self.row = max(0, min(self.rows - 1, (first or 1) - 1))
        elif final == "J":
            if first == 2:
                self.clear()
            elif first == 0:
                for column in range(self.col, self.cols):
                    self.grid[self.row][column] = " "
                for row in range(self.row + 1, self.rows):
                    self.grid[row] = [" "] * self.cols
            elif first == 1:
                for row in range(0, self.row):
                    self.grid[row] = [" "] * self.cols
                for column in range(0, self.col + 1):
                    self.grid[self.row][column] = " "
        elif final == "K":
            if first == 0:
                for column in range(self.col, self.cols):
                    self.grid[self.row][column] = " "
            elif first == 2:
                self.grid[self.row] = [" "] * self.cols
        elif final in "hl" and private and 1049 in values:
            if final == "h":
                self.alt = ([row[:] for row in self.grid], self.row, self.col)
                self.clear()
            elif self.alt is not None:
                self.grid, self.row, self.col = self.alt
                self.alt = None
        elif final == "s":
            self.saved = (self.row, self.col)
        elif final == "u":
            if self.saved is not None:
                self.row, self.col = self.saved

    def render(self):
        lines = ["".join(row).rstrip() for row in self.grid]
        while lines and not lines[-1]:
            lines.pop()
        return "\n".join(lines)


def expand(value, engine, pwsh, rpsh):
    return value.replace("{engine}", pwsh if engine == "pwsh" else rpsh)


def run(spec, engine, pwsh, rpsh):
    program = expand(spec["program"], engine, pwsh, rpsh)
    if "/" in program and not os.path.isabs(program):
        program = os.path.abspath(program)
    args = [expand(arg, engine, pwsh, rpsh) for arg in spec.get("args", [])]
    cwd = spec.get("cwd") or os.path.dirname(spec["_path"])
    cols = int(spec.get("cols", 80))
    rows = int(spec.get("rows", 24))
    timeout = int(spec.get("timeout_ms", 5000)) / 1000.0
    stop_after = spec.get("stop_after_ms")
    events = sorted(spec.get("input", []), key=lambda event: event.get("after_ms", 0))

    pid, fd = pty.fork()
    if pid == 0:
        try:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
            os.chdir(cwd)
            os.execvp(program, [program] + args)
        except OSError as error:
            try:
                os.write(2, f"pty-run: {error}\n".encode())
            except OSError:
                pass
            os._exit(127)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    screen = Screen(cols, rows)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    started = time.monotonic()
    deadline = started + (int(stop_after) / 1000.0 if stop_after else timeout)
    timed_out = False
    stopped = False
    status = None
    index = 0
    try:
        while True:
            now = time.monotonic()
            waited, candidate = os.waitpid(pid, os.WNOHANG)
            if waited == pid:
                status = candidate
                break
            if now >= deadline:
                if stop_after is None:
                    timed_out = True
                else:
                    stopped = True
                break
            wait = min(deadline - now, 0.05)
            if index < len(events):
                event = events[index]
                next_event = started + event.get("after_ms", 0) / 1000.0
                if now < next_event:
                    wait = min(wait, next_event - now)
                elif "after_text" not in event or event["after_text"] in screen.render():
                    os.write(fd, event["send"].encode("utf-8"))
                    index += 1
                    continue
            readable, _, _ = select.select([fd], [], [], max(0.0, wait))
            if not readable:
                continue
            try:
                data = os.read(fd, 65536)
            except OSError:
                data = b""
            if not data:
                break
            if b"\x1b[6n" in data:
                try:
                    os.write(fd, b"\x1b[1;1R")
                except OSError:
                    pass
            screen.feed(decoder.decode(data))
        if status is None and not timed_out:
            grace = time.monotonic() + 0.5
            while time.monotonic() < grace:
                waited, candidate = os.waitpid(pid, os.WNOHANG)
                if waited == pid:
                    status = candidate
                    break
                time.sleep(0.01)
        # Drain what is still buffered after the child exits.
        end = time.monotonic() + 0.2
        while time.monotonic() < end:
            readable, _, _ = select.select([fd], [], [], 0.02)
            if not readable:
                break
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            screen.feed(decoder.decode(data))
    finally:
        if status is None:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        try:
            waited, candidate = os.waitpid(pid, 0)
            if waited == pid:
                status = candidate
        except ChildProcessError:
            pass
        try:
            os.close(fd)
        except OSError:
            pass

    exit_code = (
        None
        if timed_out or stopped or status is None
        else os.waitstatus_to_exitcode(status)
    )
    return {"exit_code": exit_code, "screen": screen.render(), "timed_out": timed_out}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("spec")
    parser.add_argument("--engine", choices=["pwsh", "rpsh"], default="pwsh")
    parser.add_argument("--pwsh", default="")
    parser.add_argument("--rpsh", default="")
    args = parser.parse_args()
    with open(args.spec, encoding="utf-8") as handle:
        spec = json.load(handle)
    spec["_path"] = os.path.abspath(args.spec)
    result = run(spec, args.engine, args.pwsh, args.rpsh)
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()
