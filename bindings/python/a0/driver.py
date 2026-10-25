"""The command line of a language written in Python, like the Rust driver's (`a0_driver`):

    lang run <file> [args...]      interpret the program
    lang build <file> [-o output]  compile it to a native executable for the host
    lang hir <file>                print the HIR
    lang ir <file>                 print the core IR

A language calls `sys.exit(main("lang", compile))`, where `compile(text)` returns the program as an
`a0.Hir` or raises `a0.Error` with spans into `text`. Errors are shown against the source the way
the `a0` tool shows them; a program's exit status is passed through.
"""

import os
import sys

import a0

USAGE = """usage: {name} <command> <file>

commands:
  run <file> [args...]   interpret the program
  build <file> [-o out]  compile it to a native executable
  hir <file>             print the high-level IR
  ir <file>              print the core IR
"""


def main(name, compile, argv=None):
    """Runs the command line `argv` (by default the process's, without the program name) and
    returns the exit status."""
    args = sys.argv[1:] if argv is None else list(argv)
    if len(args) < 2 or args[0] not in ("run", "build", "hir", "ir"):
        sys.stderr.write(USAGE.format(name=name))
        return 2
    command, path, rest = args[0], args[1], args[2:]
    output = None
    if command == "build" and len(rest) == 2 and rest[0] == "-o":
        output = rest[1]
    elif command != "run" and rest:
        sys.stderr.write(USAGE.format(name=name))
        return 2
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        sys.stderr.write(f"{name}: cannot read {path}: {e.strerror}\n")
        return 2
    try:
        hir = compile(text)
    except a0.Error as e:
        sys.stderr.write(e.render(path, text))
        return 1
    warnings = hir.warnings
    if warnings is not None:
        sys.stderr.write(warnings.render(path, text) + "\n")
    if command == "hir":
        sys.stdout.write(str(hir))
        return 0
    try:
        module = hir.lower()
        if command == "ir":
            sys.stdout.write(str(module))
            return 0
        if command == "run":
            return module.run([path] + rest)
        if output is None:
            output = os.path.splitext(os.path.basename(path))[0]
        if sys.platform == "win32" and not os.path.splitext(output)[1]:
            output += ".exe"
        module.build_executable(output)
        return 0
    except a0.Error as e:
        sys.stderr.write(e.render(path, text))
        return 1
