# Tutorial: Kaleidoscope on A0

This tutorial builds a complete programming language on A0, one piece at a time. The language is
Kaleidoscope, the small language of [LLVM's tutorial](https://llvm.org/docs/tutorial/MyFirstLanguageFrontend/).
Its only type is the double, a 64-bit floating-point number. It has functions, `if`, `for` loops,
mutable variables, and operators that programs define themselves.

You write two things: a grammar, which says what programs look like, and a compiler, which says
what they mean. A0 provides the rest:

- an interpreter, and a JIT that compiles programs to machine code in memory and runs them;
- native executables;
- an optimizer;
- error messages that point into the source;
- editor support: highlighting, an outline, go to definition.

The whole language is about 500 lines of Rust and 70 lines of grammar. The chapters follow LLVM's
order: the grammar, the compiler, running programs, control flow, user-defined operators and
mutable variables.

Before you start, follow [Getting started](book/getting-started.md). It installs Rust, a C
compiler and the `a0` command, which this tutorial uses.

The finished language is in `examples/kaleidoscope` in the A0 repository. Every listing on this
page is cut from that code. A test checks three things: that each listing is the code, that the
listings together show all of it, and that every command prints the output shown here.

This is the program you will be able to run at the end. `#` starts a comment, and the `expect:`
comments say what the program prints:

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

A language on A0 is a Rust crate, that is, a Rust package. The crate has two parts:

- a library, which is the compiler;
- a binary, which is the language's command, here `kaleidoscope`.

Building it needs Rust and a C compiler: MSVC on Windows, `cc` on Linux and macOS.
Kaleidoscope's `Cargo.toml` depends on these A0 crates:

```toml file=examples/kaleidoscope/Cargo.toml anchor=deps
[dependencies]
a0-driver.workspace = true
a0-diag.workspace = true
a0-hir.workspace = true
a0-syntax.workspace = true
# Only for its C header, which a0-build compiles the runtime against (docs/runtime.md).
a0-runtime.workspace = true
```

Kaleidoscope is inside the A0 repository, so each dependency says `workspace = true`. In your own
project, depend on A0 from git, pinned to a release:
`a0-hir = { git = "https://github.com/Dan-k391/a0", tag = "v1.0.0" }`. Do the same for the other
crates. [Compatibility](compatibility.md) says what stays the same from one release to the next.

What each crate does:

- **`a0-syntax`** reads a grammar file and makes a parser from it (chapter 1).
- **`a0-diag`** holds source files, positions in them, and error messages. A0 calls error messages
  diagnostics.
- **`a0-hir`** is A0's high-level IR, or HIR. An IR (intermediate representation) is a form of the
  program between its source text and machine code. The compiler describes the program in HIR, and
  A0 does the rest, from checking types to collecting garbage (chapter 2).
- **`a0-driver`** gives the language its command, with `run`, `build` and more (chapter 3).
- **`a0-runtime`** is A0's runtime: C code that programs call while they run, for output, memory
  and errors. Kaleidoscope depends on it for one reason only. Its own two C functions are compiled
  against the runtime's header, `a0rt.h` (chapter 3).

## 1. The grammar

LLVM's chapters 1 and 2 write a lexer and a parser by hand. A lexer splits the program's text into
tokens: numbers, names and symbols. A parser groups the tokens into a syntax tree, following the
language's rules. On A0, both come from one grammar file, `kaleidoscope.a0g`. The compiler loads it
when it starts. [Grammars](grammar.md) describes the whole notation.

The grammar begins with its tokens:

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

- `skip` drops what it matches: whitespace, and comments from `#` to the end of the line.
- `token` names a kind of token and gives its pattern.
- `unknown` is the error for a character that no token matches.

A literal written in a rule, such as `"def"` or `";"`, is a token too, with no declaration. At each
position, the lexer takes the longest match. When a literal and a pattern match the same text, the
literal wins. So `define` is an `IDENT`, and `def` is the keyword.

Next come the rules. A rule says what one kind of node in the syntax tree contains. A program is a
list of items. An item is one of these:

- a function definition, `def`;
- an `extern` declaration of a function written in C;
- a top-level expression, which the program runs.

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

Rules use the usual notation:

- items written one after another form a sequence;
- `|` is an ordered choice: the first alternative that matches wins;
- `*` repeats, and `?` makes something optional;
- `!x` looks ahead: it matches when `x` does not come next, and consumes nothing.

A few more marks help the compiler and the error messages:

- `name:x` labels a child, so that the compiler can find it by name, such as `proto` or `body`.
- `x ~ "message"` is the error to report when `x` is missing.
- `@inline` means the rule makes no node of its own; its children go to the parent. `item` is only
  a choice, so the tree has a `definition` node directly, with no `item` node around it.
- `@recover(after ";" before "def" "extern")` keeps one broken item from stopping the parse. The
  parser reports the error, then skips to just after a `;` or just before a `def` or `extern`, and
  goes on. So one run reports every syntax error, not only the first.

`@def` and `@scope` tell editors which names a rule declares. Chapter 7 explains them.

Expressions are where Kaleidoscope is unusual. Programs can define new binary operators, each with
its own precedence (chapter 5). So the precedence of an operator is not known when the grammar is
written. The grammar parses a flat chain, `operand (op operand)*`, and the compiler groups it later.

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

The `;` lines on their own are where later chapters add alternatives. A language whose operators
are fixed does not need a chain. It writes a precedence table in the grammar instead, with `@prec`,
as Lox's and Minnow's grammars do.

You can try a grammar before writing any Rust. `a0 parse` prints the syntax tree of a program, or
its errors. Here is a program with a function, a call and a C function, `add.k`:

```kaleidoscope file=examples/kaleidoscope/tests/tutorial/add.k
# A function, a call and a C function.
extern printd(x);
def add(a b) a + b;
printd(add(1, 2) * 3);
```

`a0 parse` takes the grammar and a program. The second program, `broken.k`, is the same `add`
definition with a `)` missing:

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

Each node in the tree is in parentheses, with its kind first. Labeled children show as
`label=...`. For the broken program, the item becomes an `error` node, and the error says where the
`)` was expected.

## 2. From trees to HIR

LLVM's chapter 3 turns the syntax tree into LLVM IR. On A0, the compiler turns it into HIR instead.
HIR has functions, typed expressions, local variables and calls. It also has strings, structs,
closures and more, which Kaleidoscope does not need. [The high-level IR](hir.md) lists all of it.

The compiler builds HIR with a builder: an object with a method for each kind of expression. The
compiler refers to variables and functions by their names. The builder works out what each name
means, infers and checks the types, and reports errors. So the compiler only translates. It walks
the tree and calls the builder for each node.

The compiler is `src/lib.rs`. It starts with its imports. The items from `a0_driver::backend` are
for the C runtime library of chapter 3.

```rust file=examples/kaleidoscope/src/lib.rs anchor=uses
use std::collections::{HashMap, HashSet};

use a0_diag::{Diagnostics, SourceId};
use a0_driver::backend::{A0Symbol, HostSymbols, Library};
use a0_hir::{BinOp, BodyId, ExprId, FuncId, HirBuilder, HirModule, TypeId, UnOp};
use a0_syntax::{Grammar, NodeId, Operand, SyntaxTree, resolve_precedence};
```

The grammar file is compiled into the crate as a string. `grammar` loads it into a `Grammar` when
the compiler runs. The grammar ships with the compiler, so a grammar that fails to load is a bug in
the language, and it panics:

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

`compile` is the compiler's entry point. It does three things:

1. It parses the text. If there are syntax errors, it stops and returns them.
2. It walks the tree, which builds the HIR.
3. It calls `finish`, which checks the whole module. `finish` returns the module, or every error
   found.

`source` tells the errors which file they belong to.

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

The compiler keeps its state in a `Compiler`:

- the tree, and the builder, `b`;
- the functions declared so far, with how many parameters each takes. Kaleidoscope checks the
  number of parameters and redefinitions itself, because those are Kaleidoscope's rules, not A0's;
- the table of binary operators and their precedence, which chapter 5 lets programs extend;
- `main`, the function a program starts in, and the top-level expressions it runs, in order.

The tree's nodes are handles (`NodeId`): small numbers that the tree looks up. A few helpers read
them:

- `field` finds a child by its label, and `opt` finds one that may be missing;
- `text` is the source text that a node covers;
- `at(node)` sets the builder's span, the place in the source that the code built next comes from.
  Errors are reported there, and debuggers show it;
- `error` reports one of Kaleidoscope's own errors at a node.

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

`program` loops over the items and handles each kind. A top-level expression is built inside
`main`'s body. `begin_body` makes a body the one being built and puts its parameters in scope, and
`end_body` closes it. After the loop, `main`'s body becomes a block: the top-level expressions,
then `0`, which `main` returns.

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

A prototype is a function's name and parameters, as in `add(a b)`. `prototype` declares the
function the first time its prototype appears, and checks later ones against it. Every
Kaleidoscope value is a double, which is `TypeId::FLOAT` in the HIR. Two builder methods declare
functions:

- `declare_extern` declares a C function, which the runtime library provides (chapter 3);
- `declare_function` declares a function whose body is built later. It names the parameters, so the
  body can refer to them.

The `match` with a single arm is where chapter 5 adds operators.

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

`definition` opens the function's body and builds the expression that is the function's value. It
sets that expression as the body's root.

Expressions are a `match` on the node's kind. Later chapters add arms above the last one, which
reports anything unsupported. Each arm calls the builder:

- a number is `b.float(...)`;
- a variable is `b.name(...)`. The builder finds the parameter or local variable that the name
  means, or reports an unknown name;
- a call is `b.call_named(...)`. The builder checks the call against the function it names.

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

An `expr` node is the flat chain from the grammar, such as `1 + 2 * 3`, not yet grouped.
`resolve_precedence` groups it. It takes each operator's precedence from the table, and returns
steps: the order in which to combine the operands. `combine` builds each step:

- `+`, `-` and `*` are HIR operators;
- `<` gives 1.0 or 0.0, as in LLVM's tutorial;
- any other operator is a call to a function named after it, such as `binary|`. Programs define
  these functions in chapter 5.

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

That is a whole compiler for functions and arithmetic. Next, it needs a command that runs it.

## 3. The command and the runtime

LLVM's chapter 4 adds a JIT. A JIT (just-in-time compiler) compiles a program to machine code in
memory, and runs it at once. On A0, the JIT and everything else a command needs come from
`a0-driver`. This is the whole command, `src/main.rs`:

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

`Language::new` takes the language's name and its compile function. `grammar` gives the driver the
grammar, which the language server and the REPL use. `runtime` gives it the runtime library, which
comes next. `main` reads the command line and runs the command. The command then has these
subcommands:

| Command | What it does |
|---|---|
| `run` | compiles the program into memory with the JIT and runs it; with `A0_INTERP=1`, the interpreter runs it instead |
| `build` | compiles the program to a native executable |
| `check` | reports the program's errors without running it |
| `hir` | prints the HIR the compiler built |
| `ir` | prints the core IR, the lower-level IR that A0 turns HIR into before making machine code |
| `lsp` | a language server, for editors (chapter 7) |
| `dap` | a debugger, for editors |
| `repl` | a read-eval-print loop, which runs what you type |
| `explain` | explains an error code |

A0 makes machine code with Cranelift, a code generator written in Rust. `build` then links the
executable with the system's linker. The interpreter, the JIT and native executables give the same
output, the same errors and the same exit status.

Errors are shown with the source line they point at, and with A0's error codes. The driver's
`codes` option can hide the codes, or show the language's own codes instead.

The REPL needs no setup in Kaleidoscope. It adds each expression you type to the program's top
level, where Kaleidoscope's expressions run anyway. A language whose statements must go inside a
`main` function says so with `repl_main`.

Kaleidoscope programs call `printd` and `putchard`, which they declare with `extern`. These are C
functions in a small runtime library, `runtime/kaleidoscope.c`. They print through `a0_rt_write`,
the A0 runtime's output function, so that a debugger can capture the output. The table at the end,
`kaleidoscope_symbols`, lists the functions by name for the interpreter and the JIT:

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

Cargo runs a crate's build script, `build.rs`, before it compiles the crate. Kaleidoscope's build
script compiles the C file with `a0-build`, A0's helper for build scripts:

```rust file=examples/kaleidoscope/build.rs
fn main() {
    a0_build::runtime_library("kaleidoscope_rt", &["runtime/kaleidoscope.c"]);
}
```

The compiler gives the library to the driver in two forms, the two functions that `main.rs` passes
to `runtime`:

- `host_symbols` is the table of functions, which the interpreter and the JIT call inside the
  `kaleidoscope` process;
- `runtime_library` is the compiled library, which `build` links into executables.

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

Many languages need none of this. The HIR has strings, collections and printing built in, and a
language that uses only those has no C code.

Now Kaleidoscope runs. Take `add.k` from chapter 1:

- `hir` prints what the compiler built, in the HIR's text form. A0 can read that text back, too.
  `main` comes first because the compiler declares it first.
- `run` runs the program with the JIT.
- `build` makes an executable, `add` (`add.exe` on Windows), which runs on its own.

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

Errors come from the builder. Each one points at the place the compiler gave with `at`, and has an
error code. For a misspelled name, the builder also suggests the closest name in scope. In this
program, `y` is not defined:

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

`kaleidoscope explain E1001` says more about the error code.

## 4. Control flow

LLVM's chapter 5 adds `if`/`then`/`else` and `for` loops. In LLVM IR, they need basic blocks and
phi nodes. HIR has `if` and loops as expressions, so the compiler builds them directly. This
chapter adds them in three places: the grammar, the `expr` match, and two new functions.

The grammar gets two rules:

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=control
if_expr = "if" cond:expr "then" ~ "expected 'then'" then:expr "else" ~ "expected 'else'" else:expr;

for_expr @def(variable: var) @scope
    = "for" var:IDENT ~ "expected an identifier after 'for'"
      "=" ~ "expected '=' after the loop variable" start:expr
      "," ~ "expected ',' after the start value" end:expr
      ("," step:expr)?
      "in" ~ "expected 'in' after the loop header" body:expr;
```

`primary` gets two alternatives, on the line before its `;`:

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=primary_control
| if_expr | for_expr
```

`expr` in the compiler gets two arms:

```rust file=examples/kaleidoscope/src/lib.rs anchor=expr_control
"if_expr" => self.if_expr(node),
"for_expr" => self.for_expr(node),
```

Then come the functions those arms call:

- `truthy` decides whether a condition is true. It is true unless it is 0.0 or NaN. `truthy` tests
  the value twice, so it first stores it in a local variable. `new_local` makes a local that has no
  name in scope, so the program cannot refer to it.
- `if_expr` builds a HIR `if` with an `else`, so the `if` has a value.
- `for_expr` builds a HIR `loop` that ends with a `break`. Each pass runs in the order of LLVM's
  tutorial: the body, then the step and the end condition, then the increment.

The loop variable is declared in a scope of its own, between `push_scope` and `pop_scope`. So it
hides an outer variable of the same name only inside the loop.

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

This program, `count.k`, uses both:

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

The loop runs for `i = 3` too. The end condition is tested after the body, as in LLVM's tutorial.

## 5. User-defined operators

LLVM's chapter 6 lets programs define their own operators. An operator is a function with a
precedence:

- `def binary^ 50 (base exp)` defines a binary operator `^` with precedence 50. A higher
  precedence binds tighter.
- `def unary~(n)` defines a prefix operator `~`.

This program defines six operators and uses them:

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

The grammar changes in two places. First, a prototype can now start with `binary` or `unary`. A
binary operator may also give its precedence:

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=prototype_operators
| "binary" op:opchar ~ "expected an operator character after 'binary'"
  precedence:NUMBER?
  "(" ~ "expected '(' in prototype" params:params ")" ~ "expected ')' in prototype"
| "unary" op:opchar ~ "expected an operator character after 'unary'"
  "(" ~ "expected '(' in prototype" params:params ")" ~ "expected ')' in prototype"
```

Second, an operand can be a unary operator applied to an operand. `operand` gets an alternative,
and a new rule, `unary`, describes it:

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=operand_unary
| unary
```

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=unary
unary = op:OPCHAR operand:operand;
```

In the compiler, an operator's function is named after the operator: `binary|`, `unary!` and so
on. A prototype that starts with `binary` or `unary` gets that name from `operator`. This arm goes
into the `match` in `prototype`:

```rust file=examples/kaleidoscope/src/lib.rs anchor=prototype_operator
kind @ ("binary" | "unary") => self.operator(proto, kind, params.len())?,
```

and `expr` gets an arm for unary operators:

```rust file=examples/kaleidoscope/src/lib.rs anchor=expr_unary
"unary" => self.unary(node),
```

`operator` checks the definition:

- the number of parameters: two for a binary operator, one for a unary one;
- that it does not redefine a built-in operator;
- the precedence: a whole number from 1 to 100, or 30 when none is given.

It puts a binary operator's precedence in the table at once, so the operator's own body may use
it. `unary` calls a unary operator's function. Binary operators need nothing more, because
`combine` already calls `binary` followed by the operator for any operator that is not built in.

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

These operators cost nothing at run time. A0's optimizer works on the HIR before machine code is
made, and it is on by default. It inlines small functions such as `binary&`: it puts the function's
body in place of each call. To compare, `A0_OPT=0` turns the optimizer off.

## 6. Mutable variables

LLVM's chapter 7 adds assignment, `x = value`, and local variables, `var a = 1 in ...`. In LLVM,
they need stack slots (`alloca`) and a pass that turns those back into SSA registers. HIR has
mutable local variables, so Kaleidoscope needs neither.

The grammar gets a rule for `var`, and `primary` an alternative for it:

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=variables
var_expr @scope
    = "var" binding:var_binding ("," binding:var_binding)*
      "in" ~ "expected 'in' after 'var'" body:expr;
var_binding @def(variable) = name:IDENT ~ "expected an identifier after 'var'" ("=" init:expr)?;
```

```text file=examples/kaleidoscope/kaleidoscope.a0g anchor=primary_var
| var_expr
```

`=` has been in the operator table since chapter 2. Now `combine` turns it into an assignment,
before it builds the operands:

```rust file=examples/kaleidoscope/src/lib.rs anchor=chain_assign
if self.text(op) == "=" {
    return self.assign(steps, operands, ops, step);
}
```

`expr` gets an arm for `var`:

```rust file=examples/kaleidoscope/src/lib.rs anchor=expr_var
"var_expr" => self.var_expr(node),
```

The functions:

- `var_expr` declares each variable with `let_named`, after it builds the variable's initializer.
  So an initializer still sees an outer variable of the same name.
- `assign` checks that the left side is a variable. Then `assign_named` assigns whatever local the
  name means, and parameters can be assigned too. The assignment's value is the variable's new
  value.
- `unparen` looks through parentheses, so `(x) = 1` assigns `x`.

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

That is the whole language. The program from the top of this page, `fib.k`, now runs with the JIT,
or as an executable:

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

A language server is a program that an editor talks to, through the Language Server Protocol, for
features such as highlighting and go to definition. Kaleidoscope has one, `kaleidoscope lsp`, with
no code for it. It comes from two annotations in the grammar:

- `@def` marks a rule that declares a name. `definition @def(function)` declares a function, and
  `params @def(parameter: param)` declares each `param` as a parameter.
- `@scope` marks a rule that opens a scope: the names declared inside it are visible only there.
  `definition @def(function) @scope` makes a function's parameters visible only in its body.

From these, the server gives editors highlighting, an outline of the program's functions, go to
definition and hovers. It also shows the compiler's errors as you type. [Editor support](editors.md)
sets it up in VS Code, Neovim or Helix.

## 8. Where to go from here

- **More of the HIR.** Kaleidoscope uses only doubles and functions. The HIR also has `int`, `bool`,
  strings, arrays, maps, structs, enums with `match`, generics, closures, nullable references with
  `if let`, and `any` for dynamic typing. A0's garbage collector frees all of them. Minnow
  (`examples/minnow`, about 860 lines) is a typed language that uses them. Lox (`examples/lox`) is
  a dynamically typed one.
- **Tests.** `examples/kaleidoscope/tests` runs every program four ways: interpreted, with the JIT,
  as an executable, and with the garbage collector running at every allocation. Each runs with the
  optimizer on and off. The tests also check the compiler's errors, and this tutorial.
- **Other hosts.** A program written in C can build HIR with the same builder, through
  [the C API](c-api.md).
