"""The command line of a language written in Python: A0's driver, the same as a Rust language's
(`a0_driver`), through the C API (`a0_language_main`, D67):

    lang run <file> [args...]      run the program, at native speed (`-` reads it from standard
                                   input; `A0_INTERP=1` interprets it)
    lang build <file> [-o output]  compile it to a native executable for the host
    lang hir <file>                print the HIR
    lang ir <file>                 print the core IR
    lang lsp                       the language server (docs/editors.md)
    lang dap                       the debugger (docs/debugging.md)
    lang repl                      a read-eval-print loop (docs/repl.md)
    lang explain <code>            explain an error code

A language calls `sys.exit(main("lang", compile))`, where `compile(text)` returns the program as an
`a0.Hir` or raises `a0.Error` with spans into `text`. Errors are shown against the source the way
the `a0` tool shows them; a program's exit status is passed through. `grammar` (an `a0.Grammar`)
gives the language server its outline, definitions and completion, and lets the REPL tell
declarations from statements and unfinished lines; `repl_main` is where a language whose
statements live in `main` puts them (`"fn main() {\n{}}\n"`), as in the Rust driver.
"""

import ctypes
import sys
import traceback
from ctypes import CFUNCTYPE, POINTER, c_char_p, c_int, c_size_t, c_void_p

import a0

_compile_fn = CFUNCTYPE(c_void_p, c_void_p, c_void_p, c_size_t, POINTER(c_void_p))
_language_new = a0._fn("a0_language_new", c_void_p, c_char_p, c_size_t)
_language_set_compiler = a0._fn("a0_language_set_compiler", None, c_void_p, _compile_fn, c_void_p)
_language_set_grammar = a0._fn("a0_language_set_grammar", None, c_void_p, c_char_p, c_size_t)
_language_set_repl_main = a0._fn("a0_language_set_repl_main", None, c_void_p, c_char_p, c_size_t)
_language_main = a0._fn("a0_language_main", c_int, c_void_p, POINTER(c_char_p), c_size_t)
_language_free = a0._fn("a0_language_free", None, c_void_p)


def main(name, compile, argv=None, grammar=None, repl_main=None):
    """Runs the command line `argv` (by default the process's, without the program name) and
    returns the exit status."""
    args = sys.argv[1:] if argv is None else list(argv)

    def compiler(_context, text, length, diags):
        source = ctypes.string_at(text, length).decode("utf-8", "replace")
        try:
            hir = compile(source)
        except a0.Error as e:
            # The driver takes the diagnostics over.
            diags[0], e._handle = e._handle, None
            return None
        except Exception:
            traceback.print_exc()
            return None
        return hir._take()

    # Kept in a local for as long as the driver runs: ctypes must not free the callback.
    callback = _compile_fn(compiler)
    data = name.encode()
    language = _language_new(data, len(data))
    try:
        _language_set_compiler(language, callback, None)
        if grammar is not None:
            data = grammar.text.encode()
            _language_set_grammar(language, data, len(data))
        if repl_main is not None:
            data = repl_main.encode()
            _language_set_repl_main(language, data, len(data))
        sys.stdout.flush()
        sys.stderr.flush()
        encoded = [a.encode() for a in args]
        argv = (c_char_p * max(len(encoded), 1))(*encoded)
        return _language_main(language, argv, len(encoded))
    finally:
        _language_free(language)
