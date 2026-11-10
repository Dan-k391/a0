# Grammars

A grammar file describes the syntax of a language: the words and symbols it is made of (its
tokens) and how they fit together (its rules). A0 reads the grammar when the language starts and
uses it to split a program into tokens, parse them into a syntax tree, and report syntax errors.
Nothing is generated from the grammar ahead of time.

## A first grammar

This grammar describes a tiny language of `let` statements. Save it as `lets.a0g`:

```
grammar lets;

skip /[ \t\r\n]+/;
skip /#[^\n]*/;
token NUMBER /[0-9]+/;
token NAME /[A-Za-z_][A-Za-z0-9_]*/;

program = (!EOF stmt)* EOF;
stmt = "let" name:NAME "=" value:expr ";" ~ "expected ';'";
expr @inline = @prec primary ~ "expected an expression" {
    left "+" "-" => binary;
    left "*" "/" => binary;
};
primary = NUMBER | NAME | "(" expr ")";
```

Save a program in it as `ok.lets`:

```
let x = 1 + 2 * 3;
```

`a0 parse` parses a file with a grammar and prints the syntax tree:

```
$ a0 parse lets.a0g ok.lets
(program (stmt 'let' name='x' '=' value=(binary left=(primary '1') op='+' right=(binary left=(primary '2') op='*' right=(primary '3'))) ';'))
```

Each `(kind ...)` is a node of the tree, made by the rule of that name. Each `'text'` is a token.
`name=` and `value=` are fields: labels the rule gave those children, so that a compiler can find
them by name. `2 * 3` is grouped first because `*` is on a later line of the `@prec` table, which
makes it bind more tightly than `+`.

A language's compiler walks this tree to build the program (see [the tutorial](tutorial.md)). The
rest of this page explains each part of a grammar, in the order you need them.

## The file

A grammar file starts with an optional name and format version, then lists tokens and rules:

```
grammar lets version 1;
```

- The version is the version of the grammar format, not of your language. It is optional and is
  1 when absent. A0 1.x reads version 1, and refuses any other version with error `E0004`
  ([compatibility](compatibility.md)).
- Comments are `// ...` and `/* ... */`.
- Token declarations and rules can come in any order, and a rule can use rules defined after it.
- The first rule is the start rule: it is what a whole file must match. It cannot be `@inline`
  (see [Rules](#rules)).

## Tokens

```
skip /[ \t\r\n]+/;                      // text the lexer drops: whitespace
skip /#[^\n]*/;                         // and comments
token NUMBER /[0-9]+/;                  // a named token
token NAME /[A-Za-z_][A-Za-z0-9_]*/;
error "unterminated string" /"[^"]*/;   // text that is always an error, with this message
unknown "unexpected character";         // the message for characters nothing matches
```

The lexer is the part of the parser that splits text into tokens. It knows four kinds of
declarations:

| Declaration | What it does |
|---|---|
| `skip /pattern/;` | drops the text it matches, such as whitespace and comments |
| `token NAME /pattern/;` | makes a token of kind `NAME`, which rules can use by that name |
| `error "message" /pattern/;` | reports the text it matches with `message`, then drops it |
| `unknown "message";` | the message for a character no pattern matches; the default is `unexpected character` |

You do not declare keywords and operators. Every quoted literal used in a rule, such as `"let"`
or `">="`, is a token automatically.

**Which token wins.** At each position, the lexer takes the longest match among all literals and
patterns. On a tie, a literal wins, and then the pattern declared first. So `printer` is a `NAME`
because it is longer than the keyword `print`, `print` alone is the keyword, and `>=` is one token
rather than `>` followed by `=`.

**Errors in the text.** Text matched by an `error` pattern, and characters nothing matches, are
reported with code `E0002` and skipped. Lexing then continues, so one file can report several of
them.

**Patterns.** A pattern between slashes supports:

- characters, and `.` for any character except a newline;
- classes such as `[a-z_]` and negated classes such as `[^"]`;
- escapes: `\n \t \r \\ \/ \.` and so on, `\d \w \s`, `\xHH` and `\u{...}`;
- groups `( )` and alternatives `|`;
- repetition: `* + ?`, `{n}`, `{n,}` and `{n,m}` (at most 1000).

A pattern that can match empty text is rejected, since the lexer would never move past it.

**Literals.** A literal is text in double quotes. It may use the escapes `\" \\ \n \t \r`, and it
cannot be empty.

## Rules

```
stmt = "let" name:NAME "=" value:expr ";";
```

A rule is a name, `=`, what it matches, and `;`. Each time a rule matches, it makes a node of its
name in the tree, whose children are the tokens and nodes it matched. A rule can use:

| Syntax | Meaning |
|---|---|
| `"lit"` | a literal token |
| `NAME` | a named token, or another rule |
| `a b c` | a sequence: `a`, then `b`, then `c` |
| `a \| b` | ordered choice: `a` if it matches, otherwise `b` |
| `x*`, `x+`, `x?` | `x` any number of times, at least once, at most once |
| `( ... )` | grouping |
| `!x`, `&x` | lookahead: succeeds if `x` does not match here (`!`) or does (`&`), and consumes nothing |
| `EOF` | the end of the input |
| `field:x` | labels the children `x` makes with `field` ([Fields](#fields)) |
| `x ~ "message"` | if `x` cannot start here, the syntax error is `message` ([Syntax errors](#syntax-errors-and-recovery)) |

**Choices are ordered.** The parser tries the alternatives of `a | b` in order and takes the first
that matches. If an alternative fails partway, the parser backtracks and tries the next one. So
put the longer of two alternatives with the same start first.

**Lookahead stops loops.** `program = (!EOF stmt)* EOF;` reads statements until the end of the
input. `!EOF` makes the loop stop there without trying another `stmt`. This matters once `stmt`
[recovers from errors](#recovering-from-errors): a recovering rule never fails, so at the end of
the input it would report a missing statement.

**Checks on rules.** A0 rejects a grammar that it could not parse with:

- A rule that can reach itself without consuming a token is left-recursive, such as
  `expr = expr "+" N | N;`. Write operators with [`@prec`](#operators-prec), and lists with
  repetition.
- A `*` or `+` applied to something that can match nothing would loop forever.
- A name that is neither a rule nor a token is an error.

For example, for a grammar `lr.a0g` with that rule:

```
$ a0 parse lr.a0g input
error[grammar.invalid]: rule 'expr' is left-recursive; use @prec for operators or rewrite with repetition
 --> lr.a0g:2:1
  |
2 | expr = expr "+" N | N;
  | ^^^^
```

### Attributes

Attributes go between a rule's name and `=`. `@inline` makes the rule make no node of its own:
its children go straight to the parent node. Use it for rules that only choose between others:

```
stmt @inline = let_stmt | print_stmt | if_stmt;
```

The other attributes are `@prec` (after the `=`, [below](#operators-prec)), `@recover`
([Recovering from errors](#recovering-from-errors)), `@def` and `@scope`
([Names](#names-def-and-scope)), `@import` ([Imports](#imports-import)) and `@hir`
([Languages without host code](#languages-without-host-code-hir)).

## Fields

```
stmt = "let" name:NAME "=" value:expr ";";
```

`name:NAME` labels the `NAME` token with the field `name`. In the tree it prints as `name='x'`,
and a compiler finds it with `field(node, name)` instead of counting children.

- A field labels every child its item makes. `args:(expr ("," expr)*)` labels each `expr` and
  each `,` with `args`.
- Several children can have the same field. In `print_stmt = "print" value:expr ("," value:expr)*;`
  every printed expression is a `value`, and `fields(node, value)` returns them all.

## Operators: `@prec`

```
expr @inline = @prec primary ~ "expected an expression" {
    left "+" "-" => binary;
    left "*" "/" => binary;
};
```

A `@prec` rule parses operator expressions. It names the operand rule (`primary`), then lists
operators in a table, one line per precedence level. Lines go from the loosest binding to the
tightest, so `*` binds more tightly than `+` above. A line is one of:

| Line | Example | Makes a node with the fields |
|---|---|---|
| `left ops => kind;` | `left "-" "+" => binary;` | `left`, `op`, `right`; `1 - 2 - 3` is `(1 - 2) - 3` |
| `right ops => kind;` | `right "=" => assign;` | `left`, `op`, `right`; `a = b = c` is `a = (b = c)` |
| `prefix ops => kind;` | `prefix "!" "-" => unary;` | `op`, `operand` |
| `postfix sequence => kind;` | `postfix "." name:IDENTIFIER => get;` | `operand`, then the children of the sequence |

A larger table, in the style of Lox:

```
expression @inline = @prec primary ~ "Expect expression." {
    right "=" => assign;                 // loosest first
    left "or" => logical;
    left "==" "!=" => binary;
    left "-" "+" => binary;
    left "/" "*" => binary;
    prefix "!" "-" => unary;
    postfix "(" args:arguments? ")" ~ "Expect ')' after arguments." => call;
    postfix "." name:IDENTIFIER => get;  // tightest last
};
arguments = expression ("," expression)*;
primary = NUMBER | IDENTIFIER | "(" expression ")";
```

With the start rule `program = expression EOF;`, whitespace skipped, and tokens `NUMBER` and
`IDENTIFIER` as in [the first grammar](#a-first-grammar), `a = -f(1, 2).x` parses to:

```
(program (assign left=(primary 'a') op='=' right=(unary op='-' operand=(get operand=(call operand=(primary 'f') '(' args=(arguments (primary '1') ',' (primary '2')) ')') '.' name='x'))))
```

- A postfix line starts with a literal token. The rest of its sequence is ordinary rule syntax,
  with fields and `~` messages. The operator's tokens are children of the node too, without a
  field.
- A prefix operator applies to everything that binds at least as tightly as its own line. The
  postfix lines are below `prefix` here, so `-f(1, 2).x` is `-((f(1, 2)).x)`.
- The operand must make exactly one node: a token, or a rule that is not `@inline`.
- A token can be both a prefix and a binary operator, as `-` is here. Otherwise an operator is
  listed once, and a postfix line cannot start with a binary operator.

Some languages only know their operators after parsing, such as Kaleidoscope, whose programs
define their own. Parse those as a flat chain of operands and operators, then group them with
`a0_syntax::resolve_precedence`.

## Syntax errors and recovery

When a program does not match the grammar, A0 reports a syntax error. With the
[first grammar](#a-first-grammar), this program, `bad.lets`, is missing a value:

```
let x = 1 +;
```

```
$ a0 parse lets.a0g bad.lets
(error 'let' 'x' '=' '1' '+' ';')
error[E0001]: expected an expression
 --> bad.lets:1:12
  |
1 | let x = 1 +;
  |            ^
```

The message comes from `~ "expected an expression"` on the operand of `@prec`. Since nothing
recovered from the error, the whole file became one `error` node.

### Where the error is, and which message

Parsing tries alternatives and backtracks, so many things can fail. A0 reports the error at the
furthest token any alternative reached. Among the things expected at that token:

1. The first `~` message wins.
2. A `~` message outranks the messages from inside the item it is attached to.
3. A message from an earlier item in a sequence outranks one from a later item.
4. Without any `~` message, the error lists the tokens that would have been accepted.

For example, `bad4.lets` holds `let = 1;`, and the grammar has no `~` message where the name
should be:

```
$ a0 parse lets.a0g bad4.lets
(error 'let' '=' '1' ';')
error[E0001]: expected NAME, found '='
 --> bad4.lets:1:5
  |
1 | let = 1;
  |     ^
```

With several possible tokens, the message lists them all, as in `expected ',' or ']', found 'b'`.

Syntax errors have these [codes](diagnostics.md#codes):

| Code | Reported by |
|---|---|
| E0001 | the parser, with a `~` message or the list of expected tokens |
| E0002 | the lexer, for `error` patterns and unknown characters |
| E0003 | the parser, when a program nests more than 10,000 levels deep |

An error's span is the token where it was found, and is empty at the end of the input. A frontend
that wants its own error format, as Lox does to match jlox, can format diagnostics from the code,
message and span.

### Recovering from errors

```
stmt @recover(after ";") = "let" name:NAME "=" value:expr ";" ~ "expected ';'";
```

Without recovery, the first syntax error ends parsing. With `@recover`, a rule never fails:

1. If the rule cannot parse, it reports the error.
2. It skips at least one token, then keeps skipping until just after an `after` token or just
   before a `before` token (or the end of the input).
3. An `error` node holding the skipped tokens takes the rule's place, and parsing continues.

So one file can report several errors. Save the first grammar with this `stmt` rule as
`lets2.a0g`, and this program as `two.lets`:

```
let x = 1 +;
let y = 2;
let = 3;
let z = y;
```

```
$ a0 parse lets2.a0g two.lets
(program (error 'let' 'x' '=' '1' '+' ';') (stmt 'let' name='y' '=' value=(primary '2') ';') (error 'let' '=' '3' ';') (stmt 'let' name='z' '=' value=(primary 'y') ';'))
error[E0001]: expected an expression
 --> two.lets:1:12
  |
1 | let x = 1 +;
  |            ^

error[E0001]: expected NAME, found '='
 --> two.lets:3:5
  |
3 | let = 3;
  |     ^
```

- Put `@recover` on the unit of recovery, such as a statement or a declaration.
- List both kinds of tokens where they help: `@recover(after ";" before "class" "fun")` stops after
  a `;` or before a keyword that starts a new declaration.
- Guard the loop that uses the rule with lookahead, such as `(!"}" !EOF declaration)*`. A
  recovering rule always succeeds, so without the guard the loop would not stop at a `}`.

The Lox grammar reproduces jlox's messages and recovery exactly; its tests are in
`crates/a0-syntax/tests`.

## Trees

A compiler written in Rust gets the tree from `Grammar::parse` (in the crate `a0_syntax`), as a
`SyntaxTree`. Nodes are `NodeId` handles into it:

| Method | Returns |
|---|---|
| `root()` | the root node |
| `kind(n)`, `kind_name(n)` | the node's kind, and its name |
| `span(n)`, `text(n)` | where the node is, and its text |
| `children(n)` | its children |
| `field(n, f)`, `fields(n, f)` | the first child with field `f`, and all of them |
| `field_named(n, "name")` | the same, by the field's name |
| `to_sexpr(n)` | the text `a0 parse` prints, for debugging |

Kinds and fields are numbered by the grammar: `grammar.kind("call")`, `grammar.literal("+")` and
`grammar.field("cond")` find them.

## Names: `@def` and `@scope`

```
fun_decl @def(function) @scope = "fun" name:IDENTIFIER "(" params:parameters? ")" body:block;
parameters @def(parameter: param) = param:IDENTIFIER ("," param:IDENTIFIER)*;
var_decl @def(variable) = "var" name:IDENTIFIER ("=" init:expression)? ";";
block @scope = "{" (!"}" !EOF declaration)* "}";
```

These attributes say where a language declares names. They are all an editor needs from a
language: the language server every A0 language has (`lang lsp`, see [editors](editors.md))
builds its outline, highlighting, go to definition and hovers from them. Rust code reads the same
information from `SyntaxIndex`.

| Attribute | Meaning |
|---|---|
| `@def(kind)` | each node of the rule declares one symbol, named by the first token in its `name` field |
| `@def(kind: field)` | every token in `field` declares a symbol of its own |
| `@scope` | each node of the rule opens a scope: a block, a function, a loop that declares a variable |

- For `@def(kind)`, the name is looked for among the node's children, then inside child nodes that
  declare nothing themselves. The symbol is visible in the innermost `@scope` around the node, so a
  function's name is visible next to the function, not only inside it.
- For `@def(kind: field)`, the symbols are visible in the innermost `@scope` around them, this node
  included. So `parameters` above declares each parameter inside the function.
- The kinds are `function`, `method`, `class`, `struct`, `enum`, `variant`, `field`, `variable`,
  `constant`, `parameter`, `type_parameter`, `type`, `interface` and `module`.

**What a name refers to.** A name refers to the closest declaration before it, in the innermost
scope that declares the name.

- Functions, types and members are visible in their whole scope, even before their declaration.
  So is everything declared at file level.
- A variable is not visible in its own declaration: `var a = a + 1` reads an outer `a`.
- A name after `.` or `::` refers to every method, field and variant of that name, since which one
  is meant depends on types the grammar does not know.

**Highlighting.** Tokens are classified from the grammar too:

- literals that look like words are keywords, and other literals are operators;
- named tokens are numbers, strings or names, according to what they match;
- skipped text other than whitespace is a comment.

## Imports: `@import`

```
import_decl @import = "import" path:module_path ("as" alias:NAME)? ";";
module_path = NAME ("." NAME)*;
```

`@import` marks the rule that imports a module. The driver (the part of A0 that runs a language's
commands) then loads a program's files, and the language server analyzes them, with no code from
the language. Modules are described in [the high-level IR](hir.md) ("Modules").

- The `path` field names the module. A0 rejects `@import` on a rule without one.
- A string literal is a file path, relative to the importing file: `"lib/json.mn"`.
- Anything else is a module name, whose segments are separated by `.`, `::` or `/`: `text.json`.
  It is the file `text/json.mn`, found from the directory of the program's main file. In a
  [package](packages.md), it is found from the package's source directory, and a first segment
  that names a dependency is found in that dependency. Names that start with `std.` are modules of
  the [standard library](std.md).
- An optional `alias` field is the name the importing file uses for the module. By default it is
  the last segment.
- `name` fields import single items by their names.

`a0_syntax::imports(&grammar, &tree)` lists a file's imports.

## Languages without host code: `@hir`

A language can be defined by its grammar alone, with no compiler of its own. Each rule says what
its nodes mean, and `a0 lang` runs programs by those meanings. Calc
(`examples/calc/calc.a0g`) is one. This program:

```
fn hypot(a, b) {
  return sqrt(a * a + b * b)
}
print hypot(3, 4)
```

runs with:

```
$ a0 lang examples/calc/calc.a0g run hyp.calc
5
```

`a0 lang <grammar.a0g> <command> <file>` has the commands of every language's driver: `run`,
`build`, `check`, `hir`, `ir`, `lsp`, `dap` and `explain`. So such a language gets the interpreter,
native executables, the language server, the debugger and A0's diagnostics.

**Forms.** A rule's nodes mean what their form says. The forms build A0's
[high-level IR](hir.md) (HIR), the typed form of a program that A0 runs and compiles.

- `@hir(form)` gives a rule's nodes a form: `let_stmt @hir(let) = ...`.
- A rule named after a form (`function`, `block`, `binary`, ...) has that form without the
  attribute.
- A node without a form that has a single child means that child. That is why `@inline` rules,
  choices such as `primary = call | paren | ...` and other wrappers need nothing.

Forms read their parts from fields:

| Form | Fields | Meaning |
|---|---|---|
| `program` | its children | `function`s, and the other statements in order as `main` |
| `function` | `name`, `param` nodes or fields, `type`, `body` | a function; untyped parameters and results are inferred |
| `param` | `name`, `type` (or the node is the name) | a parameter |
| `block` | its children | statements in a scope of their own |
| `let` | `name`, `value`, `type` | declare a variable |
| `assign` | `name`, `value` | assign to a variable |
| `print` | `value`s | print them on a line, separated by spaces |
| `if` | `cond`, `then`, `else` | |
| `while` | `cond`, `body` | |
| `repeat` | `count`, `body` | the body `count` times |
| `return` | `value` | |
| `eval` | its child | an expression used as a statement |
| `binary`, `unary` | `left`, `op`, `right`; `op`, `operand` | the operators of the HIR, `and`/`&&`, `or`/`\|\|`, `not`/`!` |
| `call` | `callee` (or `operand`), `arg`s, or an `args` node of `arg`s | a function, a builtin (`str`, `len`, ...) or `std.math` (`sqrt`, `floor`, `pi`, ...) |
| `name` | the text | a variable |
| `paren` | `inner` | |
| `number`, `float`, `int`, `string`, `bool` | the text | literals (`number`s are floats) |

Tokens named `NUMBER`, `INT`, `FLOAT`, `STRING`, `NAME`, `IDENT` or `IDENTIFIER`, and the literals
`true` and `false`, have the form their name says. A node the grammar gives no meaning, an unknown
form, and a form without a field it needs are reported as errors.

```
function @def(function) @scope = "fn" name:NAME "(" (param ("," param)*)? ")" body:block;
param @def(parameter) = name:NAME;
let_stmt @def(variable) @hir(let) = "let" name:NAME "=" value:expr;
print_stmt @hir(print) = "print" value:expr ("," value:expr)*;
variable @hir(name) = NAME;
```

The forms are deliberately few. A language that needs types of its own, modules, or a meaning the
table does not have is written as a compiler (`a0 new`, see [Getting started](book/getting-started.md)),
which builds the same HIR.
