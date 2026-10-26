# The REPL

Every language built with `a0-driver` has a read-eval-print loop: `lang repl` (`a0 lang calc.a0g
repl` for a language defined by its grammar, `python lang.py repl` for one written in Python).

```
$ minnow repl
minnow 0.5.0 (A0): :help for help, :quit to leave
> let x = 20;
> fn twice(n: int) -> int {
.   return n * 2;
. }
> print(twice(x) + 2);
42
> print(y);
error[E1001]: unknown name `y`
 --> <repl>:6:7
```

- The inputs make a program. A language whose statements run at the top level (Lox, Kaleidoscope,
  Calc) keeps them in the order they were typed; one whose statements live in `main` says how
  (`.repl_main("fn main() {\n{}}\n")`), and its inputs are told apart by the
  grammar: an input that parses after the declarations so far is a declaration, anything else a
  statement of `main`.
- Each input runs the whole program again, in a process of its own (`lang run -`, which reads the
  program from standard input and runs it at native speed through the JIT, with no linking; the
  runtime keeps global state, so a process runs one program), and
  what it prints beyond what the program printed before is shown. Statements typed before run
  again, side effects included; their output is not repeated.
- An input that does not compile, or that makes the program fail (a panic, a nonzero exit status),
  is shown its errors and dropped, so the program always runs.
- An input that ends early (an open `{`, a missing `;` at the end) continues on the next line
  (`. `); an empty line submits it as it is.
- `:show` prints the program so far with line numbers (errors refer to them), `:reset` starts over,
  `:quit` or the end of the input leaves. Values are not printed by themselves: print them.
- Imports are found from the current directory.
