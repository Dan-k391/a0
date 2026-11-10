# Diagnostics

A diagnostic is a message about a problem in a program: an error, or a warning. A language built
on A0 gets error messages as good as the Rust compiler's without writing them. This page shows
what they look like, how a language makes them good, and lists every error code.

## A first error

This Minnow program (Minnow is one of A0's example languages, in `examples/minnow`) misspells a
variable and passes a string where a number belongs:

```
fn area(width: int, height: int) -> int {
  return width * height;
}

fn main() {
  let size = 3;
  print(area(siz, "4"));
}
```

`minnow check` compiles a program without running it:

```text
$ minnow check area.mn
error[E1001]: unknown name `siz`
 --> area.mn:7:14
  |
7 |   print(area(siz, "4"));
  |              ^^^ help: a variable with a similar name exists: `size`

error[E2001]: mismatched types
 --> area.mn:7:19
  |
1 | fn area(width: int, height: int) -> int {
  |                     ------ parameter declared here
...
7 |   print(area(siz, "4"));
  |                   ^^^ expected `int`, found `string`
```

Each error has these parts:

- **Severity and code**: `error[E1001]`. The code names the kind of problem, and never changes
  meaning. `minnow explain E1001` says what it means and how to fix it ([Codes](#codes)).
- **Message**: `unknown name `siz``, what is wrong, in a few words.
- **Location**: `area.mn:7:14`, the file, line and column.
- **Primary label**: the `^^^` under the code that is wrong, with what is wrong there.
- **Fix**: `help: a variable with a similar name exists: `size``. An editor offers it as a quick
  fix.
- **Related labels**: the `------` under `height`, which says where the expected type comes from.
  `...` stands for the lines between.

`explain` prints the code's explanation:

```text
$ minnow explain E2001
E2001: mismatched types

A value has a different type than the place it is used requires. The message
says which type was expected and which was found, and points at where the
expectation comes from: a declared type, a parameter, a field or a function's
result.
```

Minnow's compiler wrote none of these messages. A0's [high-level IR](hir.md) (HIR) is the typed
form of the program that a language's compiler builds, through a builder. The builder knows every
name, type and declaration in the program, so it finds these problems itself. The compiler only
told it where each part of the program is ([Reporting from a language](#reporting-from-a-language)).
The language's driver (the part of A0 that runs its commands) then renders the errors, colors
them on terminals, explains their codes, and serves them to editors.

## What a diagnostic holds

A `Diagnostic` (in the crate `a0_diag`) has:

- a severity: error or warning;
- a code, such as `E2001`;
- a message;
- a primary span (a range of the source) with a label saying what is wrong there;
- related labels: other spans, each with its own message;
- notes and helps: lines of text after the snippet;
- fixes: edits that resolve the problem.

## Reporting from a language

A language's compiler gets good diagnostics by giving the HIR builder what it cannot know: where
things are in the source, and what the language calls them.

### Say where things are

- **Spans.** `set_span` before building a node attributes it, and every problem found in it, to
  that span. Attribute a declaration (a function, struct, field, variant or global) to its
  *name*: messages such as "first declared here" and "`f` declared here" point there.
- **Parameters.** `set_param_span(body, i, span)` after `set_signature` says where the `i`th
  parameter is declared. A wrong argument then points at the parameter it should fit, as with
  `height` above.
- **Names.** A misspelled name, field, variant or type gets a "did you mean" suggestion from what
  is in scope. The suggestion becomes a fix, which editors offer as a quick fix, when the builder
  knows where the name is. That is the current span when the span is exactly the name. Otherwise,
  call `set_name_span` before a node built by name, such as a call's callee or a variant after its
  enum.

### Use the language's words

- `set_type_syntax` writes types the language's way, such as `[int]` for `array<int>`.
- `set_term` names the concepts the language calls something else: functions, closures, structs,
  enums, variants, fields, variables, null, the unwrap operator, narrowing (`if let`), `match`,
  `break`, `continue` and passing a failure on (`?`).
- Messages never use the HIR's own spellings otherwise. The unit type shows as `()`, and a value
  nothing has typed yet as `_`.

### Report the language's own errors

Some problems only the language's compiler can see. Minnow, for example, does not allow two
variables of the same name in one scope, which the HIR allows:

```
fn main() {
  let count = 1;
  let count = 2;
  print(count);
}
```

```text
$ minnow check twice.mn
error[E1005]: `count` is already declared in this scope
 --> twice.mn:3:7
  |
2 |   let count = 1;
  |       ----- first declared here
3 |   let count = 2;
  |       ^^^^^ declared again here
```

Minnow's compiler reports it like this. `self.at(name)` sets the builder's current span to the
second `count`:

```rust
self.at(name);
let msg = format!("`{}` is already declared in this scope", self.text(name));
self.b.report_code(code::DUPLICATE, &msg);
self.b.report_primary_label("declared again here");
let first = self.b.lookup_local(self.text(name));
if let Some(span) = first.and_then(|l| self.b.module().local_span(l)) {
    self.b.report_label(span, "first declared here");
}
```

- `report_code(code, message)` reports an error at the current span. Use A0's code for the same
  kind of problem (from `a0_diag::codes`), or one of the language's own. `report(message)` reports
  one without a code.
- `report_primary_label`, `report_label`, `report_note`, `report_help` and `report_fix` add to the
  error reported last.
- `unknown_variant` reports an unknown variant with a suggestion, for languages that resolve
  patterns themselves.

The C API binds all of these (`a0_hir_set_name_span`, `a0_hir_set_term`, `a0_hir_report_code`,
...), and `a0_diagnostics_code` reads a diagnostic's code ([the C API](c-api.md)).

### Choose how codes are shown

A language chooses how codes are shown with `Language::codes`:

| Choice | Shows |
|---|---|
| `Codes::A0` | A0's codes, such as `E2001` |
| `Codes::Hidden` | no codes |
| `Codes::Own(map)` | the language's codes: `map` is a function from A0's code to the language's, and `None` hides one |

With `Codes::Own`, `explain` takes the language's codes. The language server shows the same codes
as the command line.

## How diagnostics render

Diagnostics render the way the Rust compiler's do. In this program, `area2.mn`, a field name is
misspelled:

```
struct Rect { width: int, height: int }

fn main() {
  let r = new Rect { width: 3, height: 4 };
  print(r.widht);
}
```

The fix does not fit in a label, so it is shown as the corrected line, after a note:

```text
$ minnow check area2.mn
error[E1003]: no field `widht` on type `Rect`
 --> area2.mn:5:11
  |
5 |   print(r.widht);
  |           ^^^^^ unknown field
  |
  = note: the fields of `Rect` are `width`, `height`
help: a field with a similar name exists
  |
5 |   print(r.width);
  |           ~~~~~
```

A span over several lines is drawn with a track in the left margin, as in this output of
Minnow's `tests/diagnostics/multiline.mn`:

```text
error[E1007]: missing field `debug` in `Config`
 --> multiline.mn:5:11
  |
2 |   struct Config { name: string, size: int, debug: bool }
  |          ------ `Config` declared here
...
5 |     let c = new Config {
  |  ___________^
6 | |     name: "demo",
7 | |     size: 3,
8 | |   };
  | |___^ missing `debug`
```

The rules:

- **Labels.** Every label in one file shares one snippet. The primary span is underlined `^^^` and
  the others `---`. Labels that do not fit after their markers hang below on `|` connectors.
  Labels in other files get their own snippet, under `::: file:line:col`.
- **Spans over several lines.** The track starts with `/` when the span starts where its line's
  code starts. A long span shows its first and last two lines.
- **Gaps.** A single line between two shown lines is shown. Longer gaps are elided with `...`.
- **Width.** Lines wider than 140 columns are cut around what they point at. A tab counts as four
  columns.
- **Notes and helps** follow as `= note:` and `= help:`.
- **Fixes.** A fix that replaces the primary span with a short text becomes its label:
  `^^^^^ help: a variable with a similar name exists: `count``. Other fixes are shown as the
  corrected lines, with `~` under replaced text and `+` under inserted text.
- **Colors.** `RenderOptions::stderr()` colors the output with ANSI escapes when stderr is a
  terminal, unless `NO_COLOR` is set. `CLICOLOR_FORCE=1` forces colors. On Windows, the console is
  asked to process the escapes. `codes: false` renders without codes.

## Codes

Every kind of error has its own code, which never changes meaning. `lang explain <code>` (or
`a0 explain <code>`) says what it means. A code's letter and first digit say what kind of problem
it is:

- `E0xxx`: syntax ([grammars](grammar.md#syntax-errors-and-recovery))
- `E1xxx`: names
- `E2xxx`: types
- `E3xxx`: control flow
- `E4xxx`: the entry point
- `E5xxx`: packages ([packages](packages.md))
- `Wxxxx`: warnings ([Warnings](#warnings))

The catalog is `a0_diag::codes`, and a test checks that this table matches it.

| Code | Meaning |
|---|---|
| E0001 | syntax error |
| E0002 | invalid token |
| E0003 | nested too deeply |
| E0004 | written for a newer A0 |
| E1001 | unknown name |
| E1002 | unknown type |
| E1003 | unknown field |
| E1004 | unknown variant |
| E1005 | declared twice |
| E1006 | field given twice |
| E1007 | missing fields |
| E1008 | not a value |
| E1009 | cannot assign |
| E1010 | unknown module |
| E1011 | private item |
| E1012 | import cycle |
| E1013 | unknown method |
| E1015 | inline struct holds a reference |
| E1016 | inline struct contains itself |
| E2001 | mismatched types |
| E2002 | wrong number of arguments |
| E2003 | not a function |
| E2004 | operator not defined for this type |
| E2005 | cannot infer a type |
| E2006 | not nullable |
| E2007 | may be null |
| E2008 | invalid conversion |
| E2009 | invalid type test |
| E2010 | null where no null is allowed |
| E2011 | nullable value type |
| E2012 | cannot index |
| E2013 | not an enumeration |
| E2014 | match does not cover every variant |
| E2015 | wrong result |
| E2016 | wrong number of type arguments |
| E2017 | invalid map key |
| E2018 | not a structure |
| E2019 | invalid use of a type parameter |
| E2020 | cannot pass the failure on |
| E2021 | interface not implemented |
| E2022 | not callable through an interface value |
| E2023 | literal out of range |
| E2024 | not possible with an inline value |
| E2025 | cannot take the address |
| E2026 | only without the collector |
| E2027 | needs `unsafe` |
| E3001 | outside a loop |
| E4001 | no entry point |
| E4002 | invalid entry point |
| E5001 | invalid manifest |
| E5002 | invalid manifest entry |
| E5003 | dependency not found |
| E5004 | dependency cannot be fetched |
| E5005 | lock file out of date |
| E5006 | no package here |
| W0001 | unused variable |
| W0002 | unreachable code |

Problems that only a language's own code can cause keep descriptive codes instead, since they are
bugs in the language rather than errors in a program:

- `hir.builder`: the builder was misused;
- `hir.check` and `hir.tree`: a malformed module;
- `ir.*` and `native.*`: the core IR and native code generation;
- `grammar.*`: a grammar that is not valid.

## Warnings

```text
$ minnow run warnings.mn
warning[W0001]: unused variable: `doubled`
 --> warnings.mn:3:7
  |
3 |   let doubled = n * 2;
  |       ^^^^^^^ help: if this is intentional, start its name with an underscore: `_doubled`

warning[W0002]: unreachable code
 --> warnings.mn:5:3
  |
4 |   return n;
  |   --------- any code following this expression is unreachable
5 |   print("never");
  |   ^^^^^^^^^^^^^^ unreachable

42
```

This is Minnow's `tests/diagnostics/warnings.mn`. The warnings go to stderr, before the program
runs and prints `42`.

A language turns warnings on with `enable_warning`. None is on by default, since what is worth a
warning depends on the language. Minnow turns both on; Lox, whose output the book *Crafting
Interpreters* prescribes, none.

| Warning | Code | What it finds |
|---|---|---|
| `Warning::UnusedVariable` | W0001 | a variable whose value is never read; parameters and names that start with `_` are exempt, and the fix prefixes the name with `_` |
| `Warning::UnreachableCode` | W0002 | code after something that never finishes (`return`, `break`, `continue`, a call that never returns), once per block, pointing at what made it unreachable |

- Warnings are found when a module without errors is finished, as the Rust compiler does: an
  error often makes code look unused.
- The module keeps them (`HirModule::warnings`). The driver prints them before it runs or builds
  the program, and the language server shows them.
- `a0_hir_enable_warning` and `a0_hir_warnings` do the same from C.

## Runtime errors

A program that fails while it runs prints what failed, with the values involved, and the stack of
source-level functions that led there. It then exits with status 101. This is the same in the
interpreter and in native code, optimized or not. Minnow's `tests/diagnostics/runtime_index.mn`
prints `3`, then reads past the end of an array:

```text
$ minnow run runtime_index.mn
3
panic: index out of bounds: the length is 3 but the index is 3
    at at (runtime_index.mn:4:10)
    at last (runtime_index.mn:8:10)
    at main (runtime_index.mn:14:9)
```

- Functions the optimizer inlined still have their frames: `at` and `last` above were inlined into
  `main`. Closures are named after the function they are in (`main::{closure}`).
- Messages carry the values: the length and the index, the missing key (`key not found:
  "missing"`), the range of a substring, the type an `any` held (`expected `int`, found
  `string``). Unwrapping a null says so in the language's words (`unwrapped a `null` value with
  `!``, from its terms).
- Integer division by zero, the program's own `panic`, a failed `pop` and unreachable code are
  reported the same way.
- Deep recursion through one call shows the call once, with `... the frame above repeats N more
  times`. At most 64 frames are shown.
- A stack overflow prints no trace. It would be the same call thousands of times, and how deep
  the stack got is not the same in the interpreter and native code.
- Programs that do not fail pay nothing measurable for this. Only the runtime functions that can
  fail are given the location and frame pointer, and the trace is found from tables when it is
  needed.

Lox prints its own runtime errors in jlox's format, since the book's test suite prescribes them.
A language can do the same with its own checks.

## Testing diagnostics with snapshots

A snapshot test keeps a program next to a `.out` file that holds exactly what `lang run` prints:
diagnostics rendered in full, the program's output, and its exit status. Each example language has
a directory of them, `tests/diagnostics/`, checked by a test like Minnow's:

```rust
#[test]
fn diagnostics_are_what_the_snapshots_show() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/diagnostics");
    let binary = Path::new(env!("CARGO_BIN_EXE_minnow"));
    let failures = a0_driver::snapshots::check(binary, &dir, "mn", true);
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}
```

- Programs that compile are also built natively, and the executable must print what the
  interpreter prints.
- `A0_BLESS=1` rewrites the `.out` files from what the programs print. Read the diff before you
  commit it.

## In editors

The language server ([editors](editors.md)) publishes each diagnostic:

- with its code;
- with its message, primary label, notes and helps as the text;
- with its labels as related information (`relatedInformation`);
- with its fixes offered as quick fixes (`textDocument/codeAction`).
