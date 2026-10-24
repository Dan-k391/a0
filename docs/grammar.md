# A0 grammars

Status: implemented in `crates/a0-syntax`.

A language's syntax is one grammar file. A0 loads it at run time (`Grammar::from_str`), so there is no
code generation step, and the same grammar object lexes, parses and reports errors. Parsing produces
a uniform syntax tree: every rule makes a node of its kind, every token is a leaf, and children can be
labeled with fields. Frontends walk that tree directly.

## File layout

```
grammar lox;                                    // optional name

skip /[ \t\r\n]+/;                              // text the lexer drops
skip /\/\/[^\n]*/;
token NUMBER /[0-9]+(\.[0-9]+)?/;               // named token
token IDENTIFIER /[A-Za-z_][A-Za-z0-9_]*/;
error "Unterminated string." /"[^"]*/;          // matched text is reported with this message
unknown "Unexpected character.";                // message for characters nothing matches

program = (!EOF declaration)* EOF;              // the first rule is the start rule
...
```

Comments are `// ...` and `/* ... */`.

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
