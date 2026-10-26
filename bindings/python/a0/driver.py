"""The command line of a language written in Python, like the Rust driver's (`a0_driver`):

    lang run <file> [args...]      run the program (`-` reads it from standard input)
    lang build <file> [-o output]  compile it to a native executable for the host
    lang hir <file>                print the HIR
    lang ir <file>                 print the core IR
    lang repl                      a read-eval-print loop

A language calls `sys.exit(main("lang", compile))`, where `compile(text)` returns the program as an
`a0.Hir` or raises `a0.Error` with spans into `text`. Errors are shown against the source the way
the `a0` tool shows them; a program's exit status is passed through. For the REPL, `grammar` (an
`a0.Grammar`) tells declarations from statements and unfinished lines, and `repl_main` is where a
language whose statements live in `main` puts them (`"fn main() {\\n{}}\\n"`), as in the Rust
driver (`docs/repl.md`).
"""

import os
import subprocess
import sys

import a0

USAGE = """usage: {name} <command> <file>

commands:
  run <file> [args...]   run the program
  build <file> [-o out]  compile it to a native executable
  hir <file>             print the high-level IR
  ir <file>              print the core IR
  repl                   a read-eval-print loop
"""


def main(name, compile, argv=None, grammar=None, repl_main=None):
    """Runs the command line `argv` (by default the process's, without the program name) and
    returns the exit status."""
    args = sys.argv[1:] if argv is None else list(argv)
    if args == ["repl"]:
        return repl(name, compile, grammar, repl_main)
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
        if path == "-":
            text = sys.stdin.read()
            path = "<repl>" if os.environ.get("A0_REPL") else "<stdin>"
        else:
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
    if warnings is not None and not os.environ.get("A0_REPL"):
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
            # Compiled into memory, as the Rust driver runs programs; `A0_INTERP=1` interprets.
            if os.environ.get("A0_INTERP", "0") not in ("", "0"):
                return module.run([path] + rest)
            return module.run_jit([path] + rest)
        if output is None:
            output = os.path.splitext(os.path.basename(path))[0]
        if sys.platform == "win32" and not os.path.splitext(output)[1]:
            output += ".exe"
        module.build_executable(output)
        return 0
    except a0.Error as e:
        sys.stderr.write(e.render(path, text))
        return 1


def _ends_early(grammar, text):
    """Whether the only syntax errors of `text` are at its end: an input not finished yet."""
    try:
        grammar.parse(text)
        return False
    except a0.Error as e:
        end = len(text.rstrip().encode())
        return all(d.span is not None and d.span[0] >= end for d in e.diagnostics)


def _parses(grammar, text):
    try:
        grammar.parse(text)
        return True
    except a0.Error:
        return False


def repl(name, compile, grammar, repl_main):
    """The read-eval-print loop: the inputs make a program, run again in a process of its own for
    each input; what it prints beyond what it printed before is shown, and an input that does not
    compile or makes the program fail is dropped (`docs/repl.md`)."""
    interactive = sys.stdin.isatty()
    inputs = []  # (text, is a declaration)
    seen_out, seen_err = b"", b""
    pending = ""

    def program(extra=None):
        items = inputs + ([extra] if extra else [])
        if repl_main is None:
            return "".join(t for t, _ in items)
        declarations = "".join(t for t, d in items if d)
        return declarations + repl_main.replace("{}", "".join(t for t, d in items if not d))

    if interactive:
        print(f"{name} (A0): :quit to leave")
    while True:
        if interactive:
            sys.stdout.write("> " if not pending else ". ")
            sys.stdout.flush()
        line = sys.stdin.readline()
        if not line:
            break
        line = line.rstrip("\n")
        if not pending:
            if line.strip() in (":quit", ":q", ":exit"):
                break
            if line.strip() == ":show":
                for i, text in enumerate(program().splitlines()):
                    print(f"{i + 1:4} | {text}")
                continue
            if not line.strip():
                continue
        pending += line + "\n"
        declarations = "".join(t for t, d in inputs if d) + pending
        as_statement = program((pending, False))
        if grammar is not None and line.strip():
            if _ends_early(grammar, as_statement) or (
                repl_main is not None and _ends_early(grammar, declarations)
            ):
                continue
        declaration = repl_main is not None and grammar is not None and _parses(grammar, declarations)
        text = program((pending, declaration))
        entered, pending = pending, ""
        try:
            compile(text)
        except a0.Error as e:
            sys.stderr.write(e.render("<repl>", text))
            continue
        run = subprocess.run(
            [sys.executable, os.path.abspath(sys.argv[0]), "run", "-"],
            input=text.encode(),
            capture_output=True,
            env={**os.environ, "A0_REPL": "1"},
        )
        for before, now, stream in ((seen_out, run.stdout, sys.stdout), (seen_err, run.stderr, sys.stderr)):
            new = now[len(before):] if now.startswith(before) else now
            stream.write(new.decode("utf-8", "replace"))
            stream.flush()
        if run.returncode == 0:
            inputs.append((entered, declaration))
            seen_out, seen_err = run.stdout, run.stderr
    return 0
