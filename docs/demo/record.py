#!/usr/bin/env python3
"""Records the README's demo: a real terminal session in Calc, typed by this script.

    python3 docs/demo/record.py path/to/a0 demo.cast    # Linux or macOS: needs a pty and nano
    python docs/demo/render.py demo.cast docs/demo/calc.gif

Every command runs for real in a shell on a pseudo-terminal, in a scratch directory holding a copy
of examples/calc/calc.a0g; only the typing is scripted. The output is an asciicast (v2), which
render.py turns into the GIF.
"""

import fcntl
import json
import os
import pty
import select
import shutil
import struct
import sys
import tempfile
import termios
import time

COLS, ROWS = 100, 25
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

HELLO = """\
fn greet(who) {
  print "hello,", who
}

3 times {
  greet("world")
}
"""

# What is typed: ("type", text) types it and presses Enter, ("keys", text) sends keys as they
# are, ("wait", seconds) pauses.
SCRIPT = [
    ("wait", 0.6),
    ("type", "# Calc is a language defined by one grammar file, no compiler code"),
    ("wait", 0.6),
    ("type", "cat hello.calc"),
    ("wait", 1.6),
    ("type", "a0 lang calc.a0g run hello.calc"),
    ("wait", 2.2),
    ("type", "# no `times` loop yet: add one line to the grammar"),
    ("wait", 0.4),
    ("type", "nano +37 calc.a0g"),
    ("wait", 1.6),
    ("keys", "\x05"),  # ^E: end of the line
    ("wait", 0.4),
    ("keys", "\x7f\r"),  # replace the `;` with a new line
    ("keys", " " * 25 + '| count:expr "times" body:block;'),
    ("wait", 1.6),
    ("keys", "\x0f"),  # ^O: write
    ("wait", 0.5),
    ("keys", "\r"),
    ("wait", 0.7),
    ("keys", "\x18"),  # ^X: exit
    ("wait", 0.6),
    ("type", "a0 lang calc.a0g run hello.calc"),
    ("wait", 1.8),
    ("type", "a0 lang calc.a0g build hello.calc -o hello && ./hello"),
    ("wait", 3.5),
]


def main():
    a0, out = os.path.abspath(sys.argv[1]), sys.argv[2]
    work = tempfile.mkdtemp(prefix="a0-demo-")
    calc = os.path.join(work, "calc")
    os.mkdir(calc)
    shutil.copy(os.path.join(ROOT, "examples/calc/calc.a0g"), calc)
    with open(os.path.join(calc, "hello.calc"), "w") as f:
        f.write(HELLO)
    os.mkdir(os.path.join(work, "bin"))
    os.symlink(a0, os.path.join(work, "bin", "a0"))

    env = {
        "HOME": work,
        "PATH": os.path.join(work, "bin") + ":/usr/bin:/bin",
        "TERM": "xterm-256color",
        "LANG": "C.UTF-8",
        "PS1": "\\[\\e[1;32m\\]~/calc\\[\\e[0m\\] $ ",
    }
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(calc)
        os.execve("/bin/bash", ["bash", "--norc", "--noprofile", "-i"], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

    events, start = [], time.monotonic()

    def pump(seconds):
        end = time.monotonic() + seconds
        while (left := end - time.monotonic()) > 0:
            if select.select([fd], [], [], left)[0]:
                data = os.read(fd, 65536)
                events.append([round(time.monotonic() - start, 4), "o", data.decode("utf-8", "replace")])

    def send(text, delay):
        for ch in text:
            os.write(fd, ch.encode())
            pump(delay)

    pump(0.5)
    events.clear()  # the shell's start-up
    start = time.monotonic()
    os.write(fd, b"\x0c")  # clear the screen
    pump(0.3)
    for kind, arg in SCRIPT:
        if kind == "type":
            send(arg, 0.035)
            pump(0.25)
            send("\r", 0.01)
        elif kind == "keys":
            send(arg, 0.03)
        else:
            pump(arg)
    os.close(fd)
    os.waitpid(pid, 0)
    shutil.rmtree(work)

    with open(out, "w") as f:
        f.write(json.dumps({"version": 2, "width": COLS, "height": ROWS}) + "\n")
        for e in events:
            f.write(json.dumps(e) + "\n")


if __name__ == "__main__":
    main()
