# A0 grammars

Status: implemented in `crates/a0-syntax`.

A language's syntax is one grammar file. A0 loads it at run time (`Grammar::from_str`), so there is no
code generation step, and the same grammar object lexes, parses and reports errors. Parsing produces
a uniform syntax tree: every rule makes a node of its kind, every token is a leaf, and children can be
labeled with fields. Frontends walk that tree directly.

## File layout

```
grammar lox version 1;                          // optional name, and the format's version

skip /[ \t\r\n]+/;                              // text the lexer drops
skip /\/\/[^\n]*/;
token NUMBER /[0-9]+(\.[0-9]+)?/;               // named token
token IDENTIFIER /[A-Za-z_][A-Za-z0-9_]*/;
error "Unterminated string." /"[^"]*/;          // matched text is reported with this message
unknown "Unexpected character.";                // message for characters nothing matches

program = (!EOF declaration)* EOF;              // the first rule is the start rule
...
```

Comments are `// ...` and `/* ... */`. The version of the grammar format (`docs/compatibility.md`) is
optional and 1 when absent; A0 1.x reads version 1 and refuses a later one with `E0004`.

## Lexing

Literals used anywhere in rules (`"while"`, `">="`) are tokens automatically. At each position the
lexer takes the **longest** match among all literals and patterns; on a tie, literals win, then
patterns in file order. So `printer` is an `IDENTIFIER`, `print` is the keyword, and `>=` beats `>`.
Error patterns and unknown characters are reported and skipped, and lexing continues.

Patterns (`/.../`) support characters, `.`, classes (`[a-z_]`, `[^"]`), escapes (`\n \t \r \\ \/`,
`\d \w \s`, `\xHH`, `\u{...}`), groups, `|`, and `* + ? {n} {n,} {n,m}`. A pattern that can match
empty text is rejected.

## Rules

| Syntax | Meaning |
|---|---|
| `"lit"`, `NAME` | a literal token, a named token, or another rule |
| `a b c` | sequence |
| `a \| b` | ordered choice: the first alternative that succeeds |
| `x*`, `x+`, `x?` | repetition, as usual |
| `!x`, `&x` | negative / positive lookahead (consumes nothing) |
| `EOF` | end of input |
| `field:x` | label the children produced by `x` with `field` |
| `x ~ "message"` | if `x` cannot start here, report `message` |

Attributes go between the rule name and `=`:

- `@inline`: the rule makes no node; its children go to the parent. Useful for choices like
  `statement @inline = if_stmt | while_stmt | ...`.
- `@recover(after ";" before "class" "fun")`: the rule never fails. If it cannot parse, the error is
  reported, tokens are skipped (at least one, then until just after an `after` token or just before a
  `before` token), and an `error` node takes its place. Put it on the unit of recovery (a statement or
  declaration) and guard the loop that calls it with lookahead: `(!"}" !EOF declaration)*`.

## Operators: `@prec`

```
expression @inline = @prec primary ~ "Expect expression." {
    right "=" => assign;                 // lowest precedence first
    left "or" => logical;
    left "==" "!=" => binary;
    left "-" "+" => binary;
    left "/" "*" => binary;
    prefix "!" "-" => unary;
    postfix "(" args:arguments? ")" ~ "Expect ')' after arguments." => call;
    postfix "." name:IDENTIFIER => get;  // highest precedence last
};
```

Lines go from loosest to tightest binding. Binary nodes have fields `left`, `op`, `right`; prefix
nodes `op`, `operand`; postfix nodes `operand` followed by the children of the postfix sequence. The
operand rule must produce a single node (a token or a non-inline rule). Operator tables that are only
known after parsing (Kaleidoscope's user-defined operators) are parsed as a flat chain and resolved
with `resolve_precedence`.

## Errors

Parsing is ordered choice with backtracking. When parsing fails, the error is reported at the furthest
token any alternative reached. Among what was expected there, the first `~` message wins; a `~` message
outranks messages from inside the item it is attached to, and messages from earlier siblings outrank
later ones. Without any message, the error lists the expected tokens: `expected ',' or ']', found 'b'`.

Diagnostics have code `syntax.lexical` (lexer), `syntax.error` (a `~` message) or `syntax.expected`
(generic), and their span is the offending token (empty at the end of input), so a frontend can format
them in its own style. The Lox grammar reproduces jlox's messages and recovery exactly; see
`crates/a0-syntax/tests`.

## Trees

`SyntaxTree` is an arena with `NodeId` handles: `kind`, `span`, `text`, `children`, `field(node, f)`,
`fields(node, f)` (all children with that label), `field_named` for convenience, and `to_sexpr` for
debugging. Kinds and fields are interned by the grammar (`grammar.kind("call")`,
`grammar.literal("+")`, `grammar.field("cond")`).

## Names: `@def` and `@scope`

Two more attributes say where a language declares names. Editors need nothing else from a language:
the language server every A0 language gets (`lang lsp`, see `docs/editors.md`) builds its outline,
highlighting, go-to-definition and hovers from them, and `SyntaxIndex` exposes the same information
to Rust code.

- `@def(kind)`: each node of the rule declares one symbol, named by the first token in its `name`
  field (looked for in its children, then in child nodes that declare nothing themselves). The name
  is visible in the innermost `@scope` around the node, so a function's name is visible next to it.
- `@def(kind: field)`: every token in `field` declares a symbol of its own, visible in the innermost
  `@scope` around it, this node included: `parameters @def(parameter: param) = param:ID ("," param:ID)*`.
- `@scope`: nodes of the rule open a scope (blocks, functions, loops that declare a variable).

Kinds are `function`, `method`, `class`, `struct`, `enum`, `variant`, `field`, `variable`,
`constant`, `parameter`, `type_parameter`, `type`, `interface` and `module`.

A name refers to the closest declaration before it in the innermost scope that declares the name.
Functions, types and members are visible in their whole scope, as is everything declared at file
level; a variable is not visible in its own declaration (`var a = a + 1` reads an outer `a`). A name
after `.` or `::` refers to every method, field and variant of that name, since which one is meant
depends on types the grammar does not know. Tokens are classified for highlighting from the grammar
too: literals that look like words are keywords, other literals operators, and named tokens are
numbers, strings or names according to what they match; skipped text other than whitespace is a
comment.

```
fun_decl @def(function) @scope = "fun" name:IDENTIFIER "(" params:parameters? ")" body:block;
parameters @def(parameter: param) = param:IDENTIFIER ("," param:IDENTIFIER)*;
var_decl @def(variable) = "var" name:IDENTIFIER ("=" init:expression)? ";";
block @scope = "{" (!"}" !EOF declaration)* "}";
```

## Imports: `@import`

`@import` marks the rule that imports a module, so that the driver can load a program's files, and
the language server analyze them, with no code from the language (`docs/hir.md`, "Modules").

- The `path` field names the module: a string literal is a file path relative to the importing file
  (`"lib/json.mn"`), anything else a module name whose segments are separated by `.`, `::` or `/`
  (`text.json`), found from the program's main file's directory (`text/json.mn` there).
- An optional `alias` field is the name the importing file uses for the module (by default the last
  segment); `name` fields import single items by their names.

```
import_decl @import = "import" path:module_path ("as" alias:NAME)? ";";
module_path = NAME ("." NAME)*;
```

`a0_syntax::imports(&grammar, &tree)` lists a file's imports.

## Languages without host code: `@hir`

A language can be defined by its grammar alone: each rule says what its nodes mean, and
`a0 lang <grammar.a0g> <command> <file>` compiles programs by those meanings (`a0_hir::dsl`). The
commands are those of every language's driver (`run`, `build`, `hir`, `ir`, `lsp`, `explain`), so
such a language gets the interpreter, native executables, the language server and A0's diagnostics
with no code of its own. `examples/calc` is one.

`@hir(form)` gives a rule's nodes a form; a rule named after a form (`function`, `block`, `binary`,
...) has it without the attribute. A node without a form that has a single child means that child,
which is how `@inline` rules, `primary = call | paren | ...` and wrappers need nothing. Forms read
their parts from fields:

| Form | Fields | Meaning |
|---|---|---|
| `program` | its children | `function`s, and the other statements in order as `main` |
| `function` | `name`, `param` nodes or fields, `type`, `body` | a function; untyped parameters and results are inferred |
| `param` | `name`, `type` (or the node is the name) | a parameter |
| `block` | its children | statements in a scope of their own |
| `let`, `assign` | `name`, `value`, `type` | declare a variable; assign to one |
| `print` | `value`s | print them on a line, separated by spaces |
| `if`, `while` | `cond`, `then`, `else`; `cond`, `body` | |
| `repeat` | `count`, `body` | the body `count` times |
| `return`, `eval` | `value`; its child | |
| `binary`, `unary` | `left`, `op`, `right`; `op`, `operand` | the operators of the HIR, `and`/`&&`, `or`/`||`, `not`/`!` |
| `call` | `callee` (or `operand`), `arg`s, or an `args` node of `arg`s | a function, a builtin (`str`, `len`, ...) or `std.math` (`sqrt`, `floor`, `pi`, ...) |
| `name`, `paren` | the text; `inner` | |
| `number`, `float`, `int`, `string`, `bool` | the text | literals (`number`s are floats) |

Tokens named `NUMBER`, `INT`, `FLOAT`, `STRING`, `NAME`, `IDENT` or `IDENTIFIER` and the literals
`true` and `false` have the form their name says. A node the grammar gives no meaning, an unknown
form, or a form without a field it needs is reported as an error.

```
function @def(function) @scope = "fn" name:NAME "(" (param ("," param)*)? ")" body:block;
param @def(parameter) = name:NAME;
let_stmt @def(variable) @hir(let) = "let" name:NAME "=" value:expr;
print_stmt @hir(print) = "print" value:expr ("," value:expr)*;
variable @hir(name) = NAME;
```

The forms are deliberately few: a language that needs types of its own, modules, or a meaning the
table does not have is written as a compiler (`a0 new`), which builds the same HIR.
