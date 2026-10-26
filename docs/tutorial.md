# Tutorial: Kaleidoscope on A0

This tutorial builds Kaleidoscope, the language of [LLVM's tutorial](https://llvm.org/docs/tutorial/MyFirstLanguageFrontend/),
on A0, in the order of LLVM's chapters: the grammar, code generation, running programs, control
flow, user-defined operators and mutable variables. The language ends up with an interpreter,
native executables, an optimizer, error messages with source locations and an editor with
highlighting, an outline and go-to-definition, in about 490 lines of Rust and 70 of grammar.

The finished language is `examples/kaleidoscope`. Every listing below is cut from it, and a test
(`examples/kaleidoscope/tests/tutorial.rs`) checks that the listings are the code, that together
they are all of it, and that every command shown prints what is shown. Kaleidoscope looks like
this:

```kaleidoscope file=examples/kaleidoscope/tests/programs/fib.k
# LLVM's chapter 7: recursive and iterative fib, ':' as a sequencing operator.
# expect: 6765.000000
# expect: 55.000000
# expect: 832040.000000

extern printd(x);

# Define ':' for sequencing: as a low-precedence operator that ignores operands
# and just returns the RHS.
def binary : 1 (x y) y;

# Recursive fib, we could do this before.
def fib(x)
  if (x < 3) then
    1
  else
    fib(x-1)+fib(x-2);

# Iterative fib.
def fibi(x)
  var a = 1, b = 1, c in
  (for i = 3, i < x in
     c = a + b :
     a = b :
     b = c) :
  b;

printd(fib(20));
printd(fibi(10));
printd(fibi(30));
```

## 0. Setting up

A language on A0 is a Rust crate with a library (the compiler) and a binary (the command). You need
Rust and a C compiler (MSVC on Windows, `cc` on Linux and macOS). Kaleidoscope uses these A0
crates:

```toml file=examples/kaleidoscope/Cargo.toml anchor=deps
[dependencies]
a0-driver.workspace = true
a0-diag.workspace = true
a0-hir.workspace = true
a0-syntax.workspace = true
# Only for its C header, which a0-build compiles the runtime against (docs/runtime.md).
a0-runtime.workspace = true
```

Inside the A0 repository a crate says `workspace = true`. Elsewhere, depend on A0 from git, pinned
to a release: `a0-hir = { git = "https://github.com/Dan-k391/a0", tag = "v1.0.0" }`, and the same
for the others (`docs/compatibility.md` says what stays compatible).

The pieces are:

- `a0-syntax` turns a grammar file into a parser (chapter 1);
- `a0-diag` holds source files, positions and error messages;
- `a0-hir` is A0's high-level IR: the compiler builds it and A0 does the rest, from checking types to
  garbage collection (chapter 2);
- `a0-driver` gives the language its command (chapter 3);
- `a0-runtime` is the runtime programs run with; Kaleidoscope names it only so that its own two C
  functions compile against the runtime's header (`a0rt.h`, chapter 3).

## 1. The grammar

LLVM's chapters 1 and 2 write a lexer and a parser by hand. On A0 both come from one grammar file,
loaded when the compiler starts (`docs/grammar.md`). It begins with the tokens:

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=tokens
grammar kaleidoscope;

skip /[ \t\r\n]+/;
skip /#[^\n]*/;
token NUMBER /[0-9]+\.?[0-9]*|\.[0-9]+/;
token IDENT /[A-Za-z][A-Za-z0-9]*/;
// Every other single character is an operator character, as in the tutorial's lexer.
token OPCHAR /[^ \t\r\n0-9A-Za-z#(),;.]/;
unknown "unexpected character";
```

`skip` drops whitespace and comments, `token` names the kinds of tokens, and a literal written in a
rule (`"def"`, `";"`) is a token by itself. The lexer takes the longest match, preferring literals,
so `define` is an `IDENT` and `def` is the keyword.

A program is a list of items: function definitions, `extern` declarations of C functions, and
top-level expressions.

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=items,items_end
program = (!EOF item)* EOF;

item @inline @recover(after ";" before "def" "extern")
    = definition | external | toplevel | ";";

definition @def(function) @scope = "def" proto:prototype body:expr;
external @def(function) = "extern" proto:prototype;
toplevel = expr:expr;

prototype = name:IDENT ~ "expected a function name in prototype"
            "(" ~ "expected '(' in prototype" params:params ")" ~ "expected ')' in prototype"
          ;

params @def(parameter: param) = param:IDENT*;
```

Rules use the usual notation: sequences, `|` for ordered choice, `*` and `?`, and `!x` to look
ahead. `name:x` labels a child so that the compiler can find it. `x ~ "message"` is the error to
report if `x` is missing, and two attributes make the parser robust: `@inline` means `item` makes
no node of its own, and `@recover(after ";" before "def" "extern")` means a broken item does not
stop the parse: the error is reported, the parser skips to the next item, and one run reports every
syntax error. (`@def` and `@scope` tell editors about names; chapter 7 comes back to them.)

Expressions are where Kaleidoscope is unusual: programs define new binary operators with their own
precedence (chapter 5), so precedence is not known when the grammar is written. The grammar parses
a flat chain `operand (op operand)*`, and the compiler groups it.

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=expressions,primary,atoms
// Binary operators are user-definable, so expressions are parsed as a flat chain
// `operand (op operand)*` and the compiler resolves precedence with the operator table in effect.
expr = operand:operand (op:opchar operand:operand)*;
opchar @inline = OPCHAR | "=";

operand @inline = primary
                ;

primary @inline = number | call | variable | paren
                ;

number = NUMBER;
call = callee:IDENT "(" args:args? ")" ~ "expected ')' or ',' in argument list";
variable = IDENT;
args = arg:expr ("," arg:expr)*;
paren = "(" inner:expr ")" ~ "expected ')'";
```

The odd `;` lines are where later chapters add alternatives. A language with fixed operators writes
a precedence table instead (`@prec`, as in Lox's and Minnow's grammars).

Try a grammar before writing any Rust: `a0 parse` (the command-line tool of the A0 repository)
prints the tree, or the errors. With this program,

```kaleidoscope file=examples/kaleidoscope/tests/tutorial/add.k
# A function, a call and a C function.
extern printd(x);
def add(a b) a + b;
printd(add(1, 2) * 3);
```

```console
$ a0 parse kaleidoscope.a0g add.k
(program (external 'extern' proto=(prototype name='printd' '(' params=(params param='x') ')')) ';' (definition 'def' proto=(prototype name='add' '(' params=(params param='a' param='b') ')') body=(expr operand=(variable 'a') op='+' operand=(variable 'b'))) ';' (toplevel expr=(expr operand=(call callee='printd' '(' args=(args arg=(expr operand=(call callee='add' '(' args=(args arg=(expr operand=(number '1')) ',' arg=(expr operand=(number '2'))) ')') op='*' operand=(number '3'))) ')'))) ';')
$ a0 parse kaleidoscope.a0g broken.k
(program (error 'def' 'add' '(' 'a' 'b' 'a' '+' 'b' ';'))
error[E0001]: expected ')' in prototype
 --> broken.k:1:15
  |
1 | def add(a b a + b;
  |               ^
```

## 2. From trees to HIR

LLVM's chapter 3 generates LLVM IR from the syntax tree. On A0, the compiler builds the high-level IR
(`docs/hir.md`): functions, typed expressions, locals and calls, plus strings, structs, closures and
more that Kaleidoscope does not need. It builds by name, so A0 resolves names, checks types and
reports errors, and the compiler only translates. Its imports:

```rust file=examples/kaleidoscope/src/lib.rs anchor=uses
use std::collections::{HashMap, HashSet};

use a0_diag::{Diagnostics, SourceId};
use a0_driver::backend::{A0Symbol, HostSymbols, Library};
use a0_hir::{BinOp, BodyId, ExprId, FuncId, HirBuilder, HirModule, TypeId, UnOp};
use a0_syntax::{Grammar, NodeId, Operand, SyntaxTree, resolve_precedence};
```

The grammar is a string compiled into the crate and loaded at run time:

```rust file=examples/kaleidoscope/src/lib.rs anchor=grammar
/// The Kaleidoscope grammar.
pub const GRAMMAR: &str = include_str!("../kaleidoscope.a0g");

/// Loads the grammar. It is part of this crate, so failing to load it is a bug.
pub fn grammar() -> Grammar {
    match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("kaleidoscope.a0g is invalid:\n{}", d.summary()),
    }
}
```

and compiling is parsing, then walking the tree, then `finish`, which returns the checked module or
every error found:

```rust file=examples/kaleidoscope/src/lib.rs anchor=compile
/// Compiles a Kaleidoscope program to a HIR module whose `main` runs its top-level expressions.
/// `source` is the id of `text` in the caller's `SourceMap`, for the spans of errors.
pub fn compile(text: &str, source: SourceId) -> Result<HirModule, Diagnostics> {
    let (tree, diags) = grammar().parse(text, source);
    if diags.has_errors() {
        return Err(diags);
    }
    let mut c = Compiler::new(&tree);
    c.program();
    c.b.finish()
}
```

The compiler holds the tree, the `HirBuilder`, a table of the functions declared so far (Kaleidoscope
checks arities and redefinitions itself, since those are its rules, not A0's), the operator table of
chapter 5, and `main`, which runs the top-level expressions in order. A few helpers read the tree,
whose nodes are handles: `field` finds a child by label, `text` is a node's source text.
`at(node)` makes the builder report errors at that node:

```rust file=examples/kaleidoscope/src/lib.rs anchor=compiler
/// A function or operator declared with `def` or `extern`.
struct Function {
    arity: usize,
    /// `None` for an `extern`, a C function.
    func: Option<FuncId>,
    defined: bool,
}

struct Compiler<'a> {
    tree: &'a SyntaxTree,
    b: HirBuilder,
    /// Functions and operators (`binary|`, `unary!`) declared so far.
    functions: HashMap<String, Function>,
    /// Binary operators and their precedence, built-in ones included.
    binary_ops: HashMap<String, i32>,
    /// The body of `main`, and the top-level expressions it runs.
    main: BodyId,
    toplevel: Vec<ExprId>,
}

impl<'a> Compiler<'a> {
    fn new(tree: &'a SyntaxTree) -> Compiler<'a> {
        let mut b = HirBuilder::new("kaleidoscope");
        let main = b.declare_function("main", &[], &[], TypeId::INT);
        let binary_ops = [("=", 2), ("<", 10), ("+", 20), ("-", 20), ("*", 40)];
        Compiler {
            tree,
            main: b.function_body(main),
            b,
            functions: HashMap::new(),
            binary_ops: binary_ops.map(|(op, p)| (op.to_string(), p)).into(),
            toplevel: Vec::new(),
        }
    }

    // Reading the tree. Fields the grammar always produces are unwrapped.

    fn field(&self, node: NodeId, name: &str) -> NodeId {
        self.tree.field_named(node, name).unwrap()
    }

    fn opt(&self, node: NodeId, name: &str) -> Option<NodeId> {
        self.tree.field_named(node, name)
    }

    fn text(&self, node: NodeId) -> &'a str {
        self.tree.text(node)
    }

    /// Errors and the nodes built next are reported at `node`.
    fn at(&mut self, node: NodeId) {
        self.b.set_span(Some(self.tree.span(node)));
    }

    /// Reports one of Kaleidoscope's own errors; the result stands in for the expression.
    fn error(&mut self, node: NodeId, message: &str) -> ExprId {
        self.at(node);
        self.b.report(message);
        self.b.error_expr()
    }
}
```

A program is a loop over its items. A top-level expression is built inside `main`'s body
(`begin_body` puts a body's names in scope):

```rust file=examples/kaleidoscope/src/lib.rs anchor=program
impl Compiler<'_> {
    fn program(&mut self) {
        for item in self.tree.nodes(self.tree.root()) {
            match self.tree.kind_name(item) {
                "definition" => self.definition(item),
                "external" => {
                    let _ = self.prototype(self.field(item, "proto"), true);
                }
                _ => self.toplevel(item),
            }
        }
        // `main`'s own code comes from no line of the program.
        self.b.set_span(None);
        let zero = self.b.int(0);
        let root = self.b.block(&self.toplevel, Some(zero));
        self.b.set_root(self.main, root);
    }

    /// A top-level expression runs in `main`, which ignores its value.
    fn toplevel(&mut self, item: NodeId) {
        self.b.begin_body(self.main);
        let e = self.expr(self.field(item, "expr"));
        self.b.end_body();
        self.toplevel.push(e);
    }
}
```

A prototype declares a function the first time it is seen. Every Kaleidoscope value is a double,
`TypeId::FLOAT` in the HIR. `declare_extern` declares a C function and `declare_function` a function
whose body is built later, its parameters given by name:

```rust file=examples/kaleidoscope/src/lib.rs anchor=functions,functions_rest
impl Compiler<'_> {
    /// Declares the function or operator a prototype describes, or checks it against an earlier
    /// declaration. Returns its name, or `None` after reporting an error.
    fn prototype(&mut self, proto: NodeId, external: bool) -> Option<String> {
        let params = match self.opt(proto, "params") {
            Some(p) => self.tree.fields_named(p, "param"),
            None => Vec::new(),
        };
        let name = match self.text(self.tree.child(proto, 0)) {
            _ => self.text(self.field(proto, "name")).to_string(),
        };
        if name == "main" {
            self.error(proto, "'main' is reserved for the program entry point");
            return None;
        }
        let mut seen = HashSet::new();
        for &p in &params {
            if !seen.insert(self.text(p)) {
                let msg = format!("duplicate parameter '{}'", self.text(p));
                self.error(p, &msg);
                return None;
            }
        }
        if let Some(f) = self.functions.get(&name) {
            if f.arity != params.len() {
                let msg = format!(
                    "function '{name}' was declared with {} parameter(s), not {}",
                    f.arity,
                    params.len()
                );
                self.error(proto, &msg);
                return None;
            }
            return Some(name);
        }
        self.at(proto);
        let types = vec![TypeId::FLOAT; params.len()];
        let func = if external {
            self.b.declare_extern(&name, &types, TypeId::FLOAT);
            None
        } else {
            let names: Vec<&str> = params.iter().map(|&p| self.text(p)).collect();
            Some(
                self.b
                    .declare_function(&name, &names, &types, TypeId::FLOAT),
            )
        };
        let arity = params.len();
        let defined = false;
        self.functions.insert(
            name.clone(),
            Function {
                arity,
                func,
                defined,
            },
        );
        Some(name)
    }

    fn definition(&mut self, def: NodeId) {
        let proto = self.field(def, "proto");
        let Some(name) = self.prototype(proto, false) else {
            return;
        };
        let f = &self.functions[&name];
        let (defined, func) = (f.defined, f.func);
        let Some(func) = func.filter(|_| !defined) else {
            self.error(proto, &format!("function '{name}' cannot be redefined"));
            return;
        };
        self.functions.get_mut(&name).unwrap().defined = true;
        let body = self.b.function_body(func);
        self.b.begin_body(body);
        let root = self.expr(self.field(def, "body"));
        self.b.end_body();
        self.b.set_root(body, root);
    }
}
```

`definition` opens the function's body, builds the expression that is its value, and sets it as the
body's root. Expressions are a `match` on the node kind; later chapters add arms above the last one.
A variable is `b.name(...)`: the builder finds the parameter or local it means, or reports an
unknown name. A call is `b.call_named(...)`, checked against the function it names:

```rust file=examples/kaleidoscope/src/lib.rs anchor=expr,expr_end
impl Compiler<'_> {
    fn expr(&mut self, node: NodeId) -> ExprId {
        match self.tree.kind_name(node) {
            "expr" => self.chain(node),
            "number" => {
                let value: f64 = self.text(node).parse().unwrap_or(f64::NAN);
                self.at(node);
                self.b.float(value)
            }
            "variable" => {
                self.at(node);
                self.b.name(self.text(node))
            }
            "paren" => self.expr(self.field(node, "inner")),
            "call" => self.call(node),
            _ => self.error(node, "unsupported expression"),
        }
    }

    fn call(&mut self, node: NodeId) -> ExprId {
        let callee = self.text(self.field(node, "callee"));
        let args = match self.opt(node, "args") {
            Some(a) => self.tree.fields_named(a, "arg"),
            None => Vec::new(),
        };
        let args: Vec<ExprId> = args.into_iter().map(|a| self.expr(a)).collect();
        self.at(node);
        self.b.call_named(callee, &args)
    }
}
```

An operator chain is grouped with `resolve_precedence`, which takes each operator's precedence from
the table and returns the order in which to combine operands. The built-in `+`, `-` and `*` are HIR
operators; `<` gives 1.0 or 0.0 as in the tutorial; any other operator is a call to a function
named after it, which programs define in chapter 5.

```rust file=examples/kaleidoscope/src/lib.rs anchor=chain,chain_end
impl Compiler<'_> {
    /// `operand (op operand)*`, grouped with the operator table as it stands at this point.
    fn chain(&mut self, node: NodeId) -> ExprId {
        let operands = self.tree.fields_named(node, "operand");
        let ops = self.tree.fields_named(node, "op");
        let mut precedences = Vec::new();
        for &op in &ops {
            match self.binary_ops.get(self.text(op)) {
                Some(&p) => precedences.push(p),
                None => {
                    let msg = format!("unknown binary operator '{}'", self.text(op));
                    return self.error(op, &msg);
                }
            }
        }
        if ops.is_empty() {
            return self.expr(operands[0]);
        }
        // The tutorial's parser groups equal precedence to the left, '=' included.
        let steps = resolve_precedence(&precedences, &vec![false; ops.len()]);
        let last = Operand::step(steps.len() as u32 - 1);
        self.combine(&steps, &operands, &ops, last)
    }

    /// Builds one operand of a chain: an operand node, or a step that combines two.
    fn combine(
        &mut self,
        steps: &[a0_syntax::Combine],
        operands: &[NodeId],
        ops: &[NodeId],
        operand: Operand,
    ) -> ExprId {
        if !operand.is_step {
            return self.expr(operands[operand.index as usize]);
        }
        let step = steps[operand.index as usize];
        let op = ops[step.op as usize];
        let l = self.combine(steps, operands, ops, step.left);
        let r = self.combine(steps, operands, ops, step.right);
        self.at(op);
        match self.text(op) {
            "+" => self.b.binary(BinOp::Add, l, r),
            "-" => self.b.binary(BinOp::Sub, l, r),
            "*" => self.b.binary(BinOp::Mul, l, r),
            "<" => {
                // The tutorial's `fcmp ult` (true if either is NaN), as 0.0 or 1.0.
                let ge = self.b.binary(BinOp::Ge, l, r);
                let (no, yes) = (self.b.float(0.0), self.b.float(1.0));
                self.b.if_(ge, no, Some(yes))
            }
            // Any other operator is a function: `def binary| (a b)` (chapter 5 of the tutorial).
            op => self.b.call_named(&format!("binary{op}"), &[l, r]),
        }
    }
}
```

That is a whole compiler for functions and arithmetic.

## 3. The command and the runtime

LLVM's chapter 4 adds a JIT. On A0, `a0-driver` gives a language its command: `run` compiles a
program into memory with Cranelift's JIT and runs it at once (`A0_INTERP=1` interprets it), `build` compiles it to a native executable through Cranelift and the system linker, `hir`
and `ir` print what the compiler built, `lsp` is a language server, `dap` a debugger, `repl` a
read-eval-print loop (Kaleidoscope's top-level expressions run where they are typed, so
it sets no `repl_main`) and `explain` explains an error code. Errors are rendered with the source,
with A0's codes (`codes` can hide them or give the language's own); the interpreter, the JIT and
native executables give the same output, errors and exit codes.

```rust file=examples/kaleidoscope/src/main.rs
//! `kaleidoscope`: run or compile Kaleidoscope programs (`kaleidoscope run|build|ir <file.k>`).

use std::process::ExitCode;

use a0_driver::{CompileError, Language, Program};

fn main() -> ExitCode {
    Language::new("kaleidoscope", |text, source| {
        match kaleidoscope::compile(text, source) {
            Ok(m) => Ok(Program::Hir(m)),
            Err(d) => Err(CompileError::Diagnostics(d)),
        }
    })
    .grammar(kaleidoscope::GRAMMAR)
    .runtime(kaleidoscope::host_symbols, || {
        vec![kaleidoscope::runtime_library()]
    })
    .main()
}
```

Kaleidoscope programs call `printd` and `putchard` through `extern`. They are C functions in a small
runtime library:

```c file=examples/kaleidoscope/runtime/kaleidoscope.c
/* Kaleidoscope's runtime library: the helpers the LLVM tutorial's programs call through `extern`.
 * Unlike the tutorial, output goes to stdout rather than stderr. It goes through a0_rt_write, as
 * the A0 runtime's own does, so that a debugger running the program can take it. */
#include "a0rt.h"

#include <stdio.h>

/* putchard - putchar that takes a double and returns 0. */
double putchard(double x) {
    uint8_t c = (uint8_t)(char)x;
    a0_rt_write(1, &c, 1);
    return 0;
}

/* printd - printf that takes a double, prints it as "%f\n", and returns 0. */
double printd(double x) {
    char text[512];
    int n = snprintf(text, sizeof text, "%f\n", x);
    if (n > (int)sizeof text - 1) n = (int)sizeof text - 1;
    if (n > 0) a0_rt_write(1, (const uint8_t *)text, (size_t)n);
    return 0;
}

const A0Symbol kaleidoscope_symbols[] = {
    {"putchard", (void *)&putchard},
    {"printd", (void *)&printd},
    {NULL, NULL},
};
```

The build script compiles it (`a0-build` does the work):

```rust file=examples/kaleidoscope/build.rs
fn main() {
    a0_build::runtime_library("kaleidoscope_rt", &["runtime/kaleidoscope.c"]);
}
```

and the compiler hands it to the driver: `host_symbols` lets the interpreter call the functions, and
`runtime_library` links them into executables.

```rust file=examples/kaleidoscope/src/lib.rs anchor=runtime
/// The Kaleidoscope runtime library (`putchard`, `printd`), for linking native executables.
pub fn runtime_library() -> Library {
    Library::new(
        "kaleidoscope_rt",
        include_bytes!(concat!(env!("OUT_DIR"), "/kaleidoscope_rt.embedded")),
    )
}

unsafe extern "C" {
    static kaleidoscope_symbols: A0Symbol;
}

/// Host symbols for the interpreter: the A0 runtime, the C library and Kaleidoscope's runtime.
pub fn host_symbols() -> HostSymbols {
    let mut symbols = HostSymbols::new();
    // SAFETY: `kaleidoscope_symbols` is a NULL-terminated table defined in runtime/kaleidoscope.c.
    unsafe { symbols.add_table(&raw const kaleidoscope_symbols) };
    symbols
}
```

(A language whose programs need only what the HIR has built in, such as strings, collections and
printing, has no C code and skips all of this.)

Now Kaleidoscope runs. `hir` shows what the compiler built; the HIR has a text form that A0 can
read back. `main` comes first because the compiler declares it first.

```console
$ kaleidoscope hir add.k
module "kaleidoscope" version 1;

fn main() -> int {
  printd(add(1.0, 2.0) * 3.0);
  0
}

extern fn printd(float) -> float;

fn add(a: float, b: float) -> float {
  a + b
}
$ kaleidoscope run add.k
9.000000
$ kaleidoscope build add.k -o add
$ ./add
9.000000
```

Errors come from the builder, at the place in the source the compiler gave with `at`, with a code
(`kaleidoscope explain E1001` says what it means) and, for a misspelled name, the closest one in
scope:

```kaleidoscope file=examples/kaleidoscope/tests/tutorial/undefined.k
def f(x) x + y;
```

```console
$ kaleidoscope run undefined.k
error[E1001]: unknown name `y`
 --> undefined.k:1:14
  |
1 | def f(x) x + y;
  |              ^ help: a variable with a similar name exists: `x`
```

## 4. Control flow

LLVM's chapter 5 adds `if`/`then`/`else` and `for` loops, and with them basic blocks and phi nodes.
HIR has `if` and loops as expressions, so the compiler builds them directly. The grammar gets two
rules,

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=control
if_expr = "if" cond:expr "then" ~ "expected 'then'" then:expr "else" ~ "expected 'else'" else:expr;

for_expr @def(variable: var) @scope
    = "for" var:IDENT ~ "expected an identifier after 'for'"
      "=" ~ "expected '=' after the loop variable" start:expr
      "," ~ "expected ',' after the start value" end:expr
      ("," step:expr)?
      "in" ~ "expected 'in' after the loop header" body:expr;
```

and `primary` two alternatives, before its `;`:

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=primary_control
| if_expr | for_expr
```

The compiler gets two arms in `expr`:

```rust file=examples/kaleidoscope/src/lib.rs anchor=expr_control
"if_expr" => self.if_expr(node),
"for_expr" => self.for_expr(node),
```

and the code they call. A condition is true unless it is 0.0 or NaN; to test the value twice it goes
into a local made with `new_local`, which is not in scope by any name the program could use. `if`
is a HIR `if` with an `else`, so it has a value. `for` is a `loop` that ends with a `break`, laid out
in the tutorial's order: the body, then the step and the end condition, then the increment. The
loop variable is declared in a scope of its own, so it shadows an outer variable of the same name
only inside the loop.

```rust file=examples/kaleidoscope/src/lib.rs anchor=control
impl Compiler<'_> {
    /// Whether `value` is true: neither 0.0 nor NaN, as the tutorial's `fcmp one`.
    fn truthy(&mut self, value: ExprId) -> ExprId {
        let local = self
            .b
            .new_local(self.b.current_body(), "cond", TypeId::FLOAT);
        let set = self.b.let_(local, value);
        let (x, zero) = (self.b.local(local), self.b.float(0.0));
        let below = self.b.binary(BinOp::Lt, x, zero);
        let (x, zero) = (self.b.local(local), self.b.float(0.0));
        let above = self.b.binary(BinOp::Gt, x, zero);
        let either = self.b.or(below, above);
        self.b.block(&[set], Some(either))
    }

    fn if_expr(&mut self, node: NodeId) -> ExprId {
        let cond = self.expr(self.field(node, "cond"));
        let cond = self.truthy(cond);
        let then = self.expr(self.field(node, "then"));
        let els = self.expr(self.field(node, "else"));
        self.b.if_(cond, then, Some(els))
    }

    /// `for i = start, end, step in body`: the body runs, then the step and the end condition are
    /// computed, then `i` steps, and the loop stops if the condition was false. Its value is 0.
    fn for_expr(&mut self, node: NodeId) -> ExprId {
        let start = self.expr(self.field(node, "start"));
        self.b.push_scope();
        let var = self.text(self.field(node, "var"));
        let init = self.b.let_named(var, TypeId::FLOAT, start);
        let i = self.b.lookup_local(var).unwrap();
        let body = self.expr(self.field(node, "body"));
        let step = match self.opt(node, "step") {
            Some(s) => self.expr(s),
            None => self.b.float(1.0),
        };
        let end = self.expr(self.field(node, "end"));
        self.b.pop_scope();
        // The step and the condition live in locals of their own, out of the program's reach.
        let body_id = self.b.current_body();
        let step_local = self.b.new_local(body_id, "step", TypeId::FLOAT);
        let going_local = self.b.new_local(body_id, "going", TypeId::BOOL);
        let set_step = self.b.let_(step_local, step);
        let going = self.truthy(end);
        let set_going = self.b.let_(going_local, going);
        let (current, by) = (self.b.local(i), self.b.local(step_local));
        let next = self.b.binary(BinOp::Add, current, by);
        let advance = self.b.set_local(i, next);
        let going = self.b.local(going_local);
        let stop = self.b.unary(UnOp::Not, going);
        let exit = self.b.break_();
        let exit = self.b.if_(stop, exit, None);
        let iteration = self
            .b
            .block(&[body, set_step, set_going, advance, exit], None);
        let repeat = self.b.loop_(iteration);
        let zero = self.b.float(0.0);
        self.b.block(&[init, repeat], Some(zero))
    }
}
```

```kaleidoscope file=examples/kaleidoscope/tests/tutorial/count.k
# Chapter 4: if/then/else and a for loop.
extern printd(x);
def max(a b) if a < b then b else a;
for i = 1, i < 3 in printd(max(i, 2));
```

```console
$ kaleidoscope run count.k
2.000000
2.000000
3.000000
```

(The end condition is tested after the body, so the loop runs for `i = 3` too, as in LLVM's
tutorial.)

## 5. User-defined operators

LLVM's chapter 6 lets programs define operators, as functions with a precedence:

```kaleidoscope file=examples/kaleidoscope/tests/programs/operators.k
# User-defined operators with precedence, including recursion through an operator.
# expect: 1.000000
# expect: 0.000000
# expect: 13.000000
# expect: 8.000000
# expect: 120.000000

extern printd(x);

def unary!(v) if v then 0 else 1;
def binary> 10 (l r) r < l;
def binary| 5 (l r) if l then 1 else if r then 1 else 0;
def binary& 6 (l r) if !l then 0 else !!r;

# Exponentiation, binding tighter than '*'.
def binary^ 50 (base exp)
  if exp < 1 then 1 else base * (base ^ (exp - 1));

# Factorial as a prefix operator that calls itself.
def unary~(n) if n < 2 then 1 else n * ~(n - 1);

printd(3 > 2 & 1 > 0 | 0);
printd(!1);
printd(1 + 3 * 2 ^ 2);
printd(2 ^ 3);
printd(~5);
```

A prototype can now start with `binary` or `unary`,

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=prototype_operators
| "binary" op:opchar ~ "expected an operator character after 'binary'"
  precedence:NUMBER?
  "(" ~ "expected '(' in prototype" params:params ")" ~ "expected ')' in prototype"
| "unary" op:opchar ~ "expected an operator character after 'unary'"
  "(" ~ "expected '(' in prototype" params:params ")" ~ "expected ')' in prototype"
```

an operand can be a unary operator applied to an operand,

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=operand_unary
| unary
```

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=unary
unary = op:OPCHAR operand:operand;
```

and the compiler names operators `binary|`, `unary!` and so on. A prototype that starts with
`binary` or `unary` gets its name from the operator:

```rust file=examples/kaleidoscope/src/lib.rs anchor=prototype_operator
kind @ ("binary" | "unary") => self.operator(proto, kind, params.len())?,
```

```rust file=examples/kaleidoscope/src/lib.rs anchor=expr_unary
"unary" => self.unary(node),
```

`operator` checks the definition and puts a binary operator's precedence in the table at once, so
the operator's own body may use it; `unary` calls the function of a unary operator. A binary
operator needs nothing more: `combine` already calls `binary` + the operator for anything that is
not built in.

```rust file=examples/kaleidoscope/src/lib.rs anchor=operators
/// The precedence of a binary operator that does not give one.
pub const DEFAULT_PRECEDENCE: i32 = 30;

impl Compiler<'_> {
    /// The name of an operator a prototype defines (`binary|`, `unary!`), after checking its
    /// operands and precedence. A binary operator's precedence takes effect at once, so its own
    /// body may use it.
    fn operator(&mut self, proto: NodeId, kind: &str, params: usize) -> Option<String> {
        let op = self.text(self.field(proto, "op"));
        let operands = if kind == "binary" { 2 } else { 1 };
        if params != operands {
            let msg = format!("a {kind} operator takes {operands} operand(s), not {params}");
            self.error(proto, &msg);
            return None;
        }
        if kind == "binary" {
            if ["=", "<", "+", "-", "*"].contains(&op) {
                let msg = format!("cannot redefine the built-in operator '{op}'");
                self.error(proto, &msg);
                return None;
            }
            let precedence = match self.opt(proto, "precedence") {
                Some(p) => match self.text(p).parse::<f64>() {
                    Ok(v) if (1.0..=100.0).contains(&v) && v.fract() == 0.0 => v as i32,
                    _ => {
                        self.error(
                            p,
                            "invalid precedence: must be a whole number from 1 to 100",
                        );
                        return None;
                    }
                },
                None => DEFAULT_PRECEDENCE,
            };
            self.binary_ops.insert(op.to_string(), precedence);
        }
        Some(format!("{kind}{op}"))
    }

    fn unary(&mut self, node: NodeId) -> ExprId {
        let op = self.field(node, "op");
        let name = format!("unary{}", self.text(op));
        if !self.functions.contains_key(&name) {
            let msg = format!("unknown unary operator '{}'", self.text(op));
            return self.error(op, &msg);
        }
        let operand = self.expr(self.field(node, "operand"));
        self.at(node);
        self.b.call_named(&name, &[operand])
    }
}
```

```console
$ kaleidoscope run operators.k
1.000000
0.000000
13.000000
8.000000
120.000000
```

These operators cost nothing at run time: A0's optimizer, on by default, inlines small functions
such as `binary&` into their callers. `A0_OPT=0` turns it off, to compare.

## 6. Mutable variables

LLVM's chapter 7 adds assignment and `var`/`in`, and with them stack slots, `alloca` and a pass that
turns them back into SSA registers. HIR has mutable locals, so Kaleidoscope needs neither:

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=variables
var_expr @scope
    = "var" binding:var_binding ("," binding:var_binding)*
      "in" ~ "expected 'in' after 'var'" body:expr;
var_binding @def(variable) = name:IDENT ~ "expected an identifier after 'var'" ("=" init:expr)?;
```

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=primary_var
| var_expr
```

`=` has been in the operator table since chapter 2. It becomes an assignment in `combine`, before
the operands are built:

```rust file=examples/kaleidoscope/src/lib.rs anchor=chain_assign
if self.text(op) == "=" {
    return self.assign(steps, operands, ops, step);
}
```

```rust file=examples/kaleidoscope/src/lib.rs anchor=expr_var
"var_expr" => self.var_expr(node),
```

`var` declares each variable with `let_named` after building its initializer, so an initializer
sees the outer variable of the same name; `assign_named` assigns whatever local the name means
(parameters are mutable too).

```rust file=examples/kaleidoscope/src/lib.rs anchor=variables
impl Compiler<'_> {
    /// `var a = 1, b in body`: each initializer runs before its own variable is in scope.
    fn var_expr(&mut self, node: NodeId) -> ExprId {
        self.b.push_scope();
        let mut lets = Vec::new();
        for binding in self.tree.fields_named(node, "binding") {
            let init = match self.opt(binding, "init") {
                Some(i) => self.expr(i),
                None => self.b.float(0.0),
            };
            let name = self.text(self.field(binding, "name"));
            self.at(binding);
            lets.push(self.b.let_named(name, TypeId::FLOAT, init));
        }
        let body = self.expr(self.field(node, "body"));
        self.b.pop_scope();
        self.b.block(&lets, Some(body))
    }

    /// `x = value` sets the variable `x` and evaluates to the value.
    fn assign(
        &mut self,
        steps: &[a0_syntax::Combine],
        operands: &[NodeId],
        ops: &[NodeId],
        step: a0_syntax::Combine,
    ) -> ExprId {
        let op = ops[step.op as usize];
        let target = match step.left.is_step {
            false => Some(self.unparen(operands[step.left.index as usize])),
            true => None,
        };
        let Some(target) = target.filter(|&t| self.tree.kind_name(t) == "variable") else {
            return self.error(op, "destination of '=' must be a variable");
        };
        let value = self.combine(steps, operands, ops, step.right);
        let name = self.text(target);
        self.at(target);
        let set = self.b.assign_named(name, value);
        let get = self.b.name(name);
        self.b.block(&[set], Some(get))
    }

    /// `(x)` or a one-operand chain is just its operand.
    fn unparen(&self, mut node: NodeId) -> NodeId {
        loop {
            match self.tree.kind_name(node) {
                "paren" => node = self.field(node, "inner"),
                "expr" if self.tree.fields_named(node, "op").is_empty() => {
                    node = self.field(node, "operand");
                }
                _ => return node,
            }
        }
    }
}
```

That is the whole language. The example at the top of the tutorial runs at once, or as an
executable:

```console
$ kaleidoscope run fib.k
6765.000000
55.000000
832040.000000
$ kaleidoscope build fib.k -o fib
$ ./fib
6765.000000
55.000000
832040.000000
```

## 7. Editor support

The `@def` and `@scope` annotations in the grammar say which rules declare names and which open
scopes: `definition @def(function) @scope`, `params @def(parameter: param)`, and so on. That is
all `kaleidoscope lsp` needs to give editors highlighting, an outline of the program's functions,
go-to-definition and hovers, next to the compiler's errors as you type. `docs/editors.md` sets it
up in VS Code (`editors/vscode`), Neovim or Helix.

## 8. Where to go from here

- **More of the HIR.** Kaleidoscope uses doubles and functions. The HIR also has `int`, `bool`,
  strings, arrays, maps, structs, enums with `match`, generics, closures, nullable references with
  `if let`, and `any` for dynamic typing, all garbage collected. `examples/minnow` is a typed
  language that uses them (about 760 lines); `examples/lox` is a dynamically typed one.
- **Tests.** `examples/kaleidoscope/tests` runs every program interpreted, in the JIT, native and under GC stress,
  with the optimizer on and off, checks the compiler's errors, and checks this tutorial.
- **Other hosts.** A program written in C can build HIR with the same builder (`docs/c-api.md`).
