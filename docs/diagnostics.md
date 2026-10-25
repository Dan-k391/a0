# Diagnostics

Status: implemented in `crates/a0-diag` (data model, rendering, codes, suggestions), the HIR builder
(`crates/a0-hir`) and the driver (`crates/a0-driver`). Keep this document in sync with the code.

A language built on A0 gets rustc-quality errors without writing any: the HIR builder knows every
name, type and declaration of the program, so it reports problems with the related locations, in
the language's own words, with suggested fixes; the driver renders them, colors them on terminals,
explains their codes, and serves them to editors.

## What a diagnostic holds

A `Diagnostic` has a severity, a code, a message, a primary span with a label saying what is wrong
there, related labels (other spans with their own messages), notes and helps, and fixes (edits that
resolve it). It renders the way rustc's do:

```text
error[E2001]: mismatched types
 --> main.mn:5:5
  |
1 | fn f(a: int) {}
  |      - parameter declared here
...
5 |   f("x");
  |     ^^^ expected `int`, found `string`
```

- Every label in one file shares one snippet: the primary span is underlined `^^^` and the others
  `---`, and labels that do not fit after their markers hang below on `|` connectors. Labels in
  other files get their own snippet under `::: file:line:col`.
- A span over several lines is drawn with a track in the left margin, starting with `/` when the
  span starts where its line's code starts; a long one shows its first and last two lines.
- A single line between two shown ones is shown; longer gaps are elided with `...`. Lines wider
  than 140 columns are cut around what they point at. Tabs count as four columns.
- Notes and helps follow as `= note:` and `= help:`. A fix that replaces the primary span with a
  short text becomes its label (`^^^^^ help: a variable with a similar name exists: `count``);
  other fixes are shown as the corrected lines, with `~` under replaced and `+` under inserted text.
- `RenderOptions::stderr()` colors the output with ANSI escapes when stderr is a terminal, unless
  `NO_COLOR` is set; `CLICOLOR_FORCE=1` forces colors. On Windows the console is asked to process
  escapes. `codes: false` renders without codes.

## Codes

Every kind of error has its own code, which never changes meaning, and `lang explain <code>` (or
`a0 explain <code>`) says what it means. The catalog is `a0_diag::codes`; a test checks that this
list matches it.

| Code | Meaning |
|---|---|
| E0001 | syntax error |
| E0002 | invalid token |
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
| E3001 | outside a loop |
| E4001 | no entry point |
| E4002 | invalid entry point |
| W0001 | unused variable |
| W0002 | unreachable code |

Problems only a language's own code can cause keep descriptive codes, since they are bugs in the
language rather than errors in a program: `hir.builder` (the builder was misused), `hir.check` and
`hir.tree` (a malformed module), `ir.*`, `native.*`, `grammar.*`.

A language chooses how codes are shown with `Language { codes, .. }`: `Codes::A0`, `Codes::Hidden`,
or `Codes::Own(map)`, a function from A0's code to the language's (`None` hides one); `explain` then
takes the language's codes. The language server shows the same codes.

## Reporting well from a frontend

The builder needs to know where things are. A frontend that does these gets everything above:

- **Spans.** `set_span` before building a node attributes it, and every problem found in it, to that
  span. Attribute declarations (functions, structs, fields, variants, globals) to their *names*:
  "first declared here" and "`f` declared here" point there.
- **Parameters.** `set_param_span(body, i, span)` after `set_signature` lets a wrong argument point
  at the parameter it should fit.
- **Names.** Misspelled names, fields, variants and types get "did you mean" suggestions from what
  is in scope. The suggestion becomes a fix, which editors offer as a quick fix, when the builder
  knows where the name is: the current span when it is exactly the name, or `set_name_span` before
  a node built by name (a call's callee, a variant after its enum).
- **Words.** `set_type_syntax` writes types the language's way (`[int]` for `array<int>`), and
  `set_term` names concepts the language calls something else: functions, closures, structs,
  enums, variants, fields, variables, null, the unwrap operator, narrowing (`if let`), `match`,
  `break` and `continue`. Messages never use the HIR's own spellings otherwise: the unit type shows
  as `()`, and a value nothing typed yet as `_`.
- **Its own errors.** `report_code(code, message)` reports at the current span, with A0's code for
  the same kind of problem or the language's own (`report` gives none). `report_primary_label`,
  `report_label`, `report_note`, `report_help` and `report_fix` add to the error reported last.
  `unknown_variant` reports an unknown variant with a suggestion, for languages that resolve
  patterns themselves.

The C API binds all of these (`a0_hir_set_name_span`, `a0_hir_set_term`, `a0_hir_report_code`, ...)
and `a0_diagnostics_code` reads a diagnostic's code.

## Runtime errors

A program that fails prints what failed, with the values involved, and the stack of source-level
functions that led there, then exits with status 101, the same in the interpreter and in native
code, optimized or not:

```text
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
  `!``, from its `Term`s). Integer division by zero, the program's own `panic`, a failed `pop`
  and unreachable code are reported the same way.
- Deep recursion through one call shows the call once, with `... the frame above repeats N more
  times`; at most 64 frames are shown. A stack overflow prints no trace: it would be the same call
  thousands of times, and how deep the stack got is not the same in both backends.
- Programs that do not fail pay nothing measurable (D45): failures carry their location and frame
  pointer only into the runtime functions that can fail, and the trace is found from tables when it
  is needed.

Lox prints its own runtime errors in jlox's format, since the book's suite prescribes them; a
language can do the same with its own checks.

## Warnings

A language turns warnings on with `enable_warning`; none is on by default, since what is worth a
warning depends on the language (Minnow turns both on; Lox, whose output the book prescribes, none).

| Warning | Code | What it finds |
|---|---|---|
| `Warning::UnusedVariable` | W0001 | a variable whose value is never read; parameters and names that start with `_` are exempt, and the fix prefixes the name with `_` |
| `Warning::UnreachableCode` | W0002 | code after something that never finishes (`return`, `break`, `continue`, a call that never returns), once per block, pointing at what made it unreachable |

Warnings are found when a module without errors is finished, as rustc does: an error often makes
code look unused. The module keeps them (`HirModule::warnings`); the driver prints them before it
runs or builds the program, and the language server shows them. `a0_hir_enable_warning` and
`a0_hir_warnings` do the same from C.

## Snapshots

Each example language has `tests/diagnostics/`: programs with errors, warnings and runtime errors,
and next to each a `.out` file with exactly what `lang run` prints, diagnostics rendered in full
(`a0_driver::snapshots::check`). Programs that compile are also built natively, and the executable
must print what the interpreter prints. `A0_BLESS=1` rewrites the `.out` files; read the diff.

## In editors

The language server publishes each diagnostic with its code, its message, primary label, notes and
helps as the text, and its labels as related information (`relatedInformation`), and offers its
fixes as quick fixes (`textDocument/codeAction`).
