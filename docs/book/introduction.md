# Introduction

A0 is a framework for building programming languages. A language on A0 provides two things: a
grammar, which says what its programs look like, and a way to say what each part of a program
means. A0 does everything after that. It checks the program, compiles it, runs it, builds
executables, and gives the language an editor server, a debugger and a REPL.

This chapter follows one small program through A0, one step at a time, and says what each part of
A0 does to it.

## The steps

```
program ──grammar──▶ syntax tree ──@hir forms or your compiler──▶ HIR ──▶ core IR ──┬─ interpreter
                                                                                  ├─ JIT (run)
                                                                                  ├─ native executables, for any target
                                                                                  └─ WebAssembly
```

The example is a one-line program in Calc, a calculator language that comes with A0. It is in a
file `small.calc`:

```
print 2 + 3 * 4
```

## 1. The grammar makes a syntax tree

A **grammar** is a file of tokens and rules that describes a language's syntax. Calc's grammar is
[`examples/calc/calc.a0g`](../../examples/calc/calc.a0g). Two of its rules cover this program:

```
print_stmt @hir(print) = "print" value:expr ~ "expected a value" ("," value:expr)*;

expr @inline = @prec primary ~ "expected an expression" {
    ...
    left "+" "-" => binary;
    left "*" "/" => binary;
    ...
};
```

The first rule says a print statement is the word `print` and an expression, called `value`. The
second lists the operators from loosest to tightest, so `*` binds tighter than `+`. The text after
`~` is the error A0 reports when that part is missing.

A0 reads the grammar when the language starts and makes a parser from it. The parser turns the
program into a **syntax tree**: the program's parts, nested, each with its rule's name.
`a0 parse` prints it:

```
$ a0 parse examples/calc/calc.a0g small.calc
(program (print_stmt 'print' value=(binary left=(primary '2') op='+' right=(binary left=(primary '3') op='*' right=(primary '4')))))
```

The tree has `3 * 4` inside the `+`, as the operator table says.

A grammar can also tell editors which rules declare names and where scopes begin, and tell A0 how
one file imports another. [Grammars](../grammar.md) explains the notation.

## 2. The tree becomes HIR

The **high-level IR (HIR)** is A0's typed form of a program. "IR" means intermediate
representation: a form of the program between the source text and machine code. The HIR is close
to what a programmer writes. It has numbers, strings, arrays, maps, structs, enums, closures,
generics, interfaces, exceptions, and values whose type is only known when the program runs.

Something has to say how each part of the tree becomes HIR. Calc does it in the grammar itself:
`@hir(print)` on the rule above says a print statement means HIR's `print`. `a0 lang` runs a
language like this, with no other code. Here is the HIR it builds, shown with `a0 lang ... hir`
(the end of the output, after the standard library's mathematics):

```
$ a0 lang examples/calc/calc.a0g hir small.calc
...
fn main() {
  {
    print(2.0 + 3.0 * 4.0);
    print("\n");
  };
}
```

Calc's numbers are floating point, so `2` became `2.0`. A0 worked out the types of every value and
checked them. Had the program said `print 2 + "a"`, this is where A0 would have reported the
mistake, in Calc's terms.

Most languages need more than the `@hir` forms, so they have a **compiler**: Rust code that walks
the syntax tree and calls A0's HIR builder for each part. This is the part of the compiler that
`a0 new` makes, for a `binary` node like the ones above:

```rust
"binary" | "logical" => {
    let (l, r) = (sub(self, "left"), sub(self, "right"));
    self.at(node);
    match self.text(self.field(node, "op")) {
        "&&" => self.b.and(l, r),
        "||" => self.b.or(l, r),
        op => self.b.binary(BinOp::from_symbol(op).unwrap(), l, r),
    }
}
```

It builds the two operands, records where the node is in the source (`self.at`), and builds the
operation. The builder `self.b` does the rest. Given a name, it finds what the name refers to. It
infers and checks types, and reports errors in the language's own words. So a compiler is often one
walk over the tree, with no type checker of its own. [The high-level IR](../hir.md) lists what the
HIR has.

## 3. The HIR becomes core IR

The **core IR** is A0's low-level form of a program. It has only numbers, pointers, memory, and
calls, close to what the machine runs. A0 optimizes the HIR, then lowers it to core IR. `a0 lang
... ir` prints it. This is the program's `main`:

```
$ a0 lang examples/calc/calc.a0g ir small.calc
...
func @hir.main() {
  %0: i64 = iconst 0
  %1: ptr = call @a0_gc_frame_push(%0)
  %2: f64 = fconst 2.0
  %3: f64 = fconst 3.0
  %4: f64 = fconst 4.0
  %5: f64 = fmul %3, %4
  %6: f64 = fadd %2, %5
  %7: ptr = call @a0_string_from_float(%6)
  call @a0_string_print(%7)
  %8: ptr = addr @hir.str.0
  %9: ptr = load %8
  call @a0_string_print(%9)
  call @a0_gc_frame_pop(%0)
  return
}
```

Each line makes one value. `fmul` and `fadd` do the arithmetic. Printing became calls to functions
whose names start with `a0_`. A0 also adds a function `@main` that prepares the program's
constants and calls this one. [The core IR](../ir.md) describes every instruction.

## 4. The runtime

The functions called above, `a0_string_from_float`, `a0_string_print` and the others, belong to
A0's **runtime**: a library written in C that every program runs with. It holds what compiled code
does not do inline: strings, arrays and maps, printing, threads, the garbage collector, and the
error message and stack trace when a program fails. `a0_gc_frame_push` and `a0_gc_frame_pop` tell
the collector which values a function holds. This one holds none, hence the `0`.

A program can instead manage its memory itself, with no collector ([Memory](memory.md)). A
language can also ship C functions of its own ([the runtime](../runtime.md)).

## 5. A backend runs it

A **backend** turns the core IR into something that runs. A0 has four, and they give the same
output, the same errors and the same exit code for every program:

| Backend | Used by | What it does |
|---|---|---|
| The JIT | `run` | Compiles the program to machine code in memory with Cranelift, then runs it |
| Native code | `build` | Compiles the program with Cranelift and links it into an executable, for this machine or [another platform](../cross.md) |
| The interpreter | the debugger, `A0_INTERP=1` | Runs the core IR directly. It is the reference the other backends are tested against |
| WebAssembly | [the playground](../playground.md) | Compiles the program to run in a browser |

All three commands print the same thing:

```
$ a0 lang examples/calc/calc.a0g run small.calc
14
$ A0_INTERP=1 a0 lang examples/calc/calc.a0g run small.calc
14
$ a0 lang examples/calc/calc.a0g build small.calc -o small && ./small
14
```

## The driver puts it together

A language's command, such as `calc` or `mylang`, comes from A0's **driver**. The language gives
the driver its grammar and its compile function:

```rust
Language::new("mylang", compile)
```

The driver turns that into a command with `run`, `build`, `check`, `hir`, `ir`, `lsp` (the language
server, for editors), `dap` (the debugger), `repl` and `explain` (what an error code means). It
also finds a program's other files through the grammar's imports, and reads packages and their
dependencies. A standard library comes with it: files, input and output, strings, mathematics and
threads.

## The languages in this repository

A0 comes with languages built on it, each an example of one kind of language:

| Language | What it shows | Guide |
|---|---|---|
| B0 | everything at once: the flagship, built on the public API only | [`b0/README.md`](../../b0/README.md), [Building a full language](b0.md) |
| Minnow | a typed language with modules, generics and interfaces | [`examples/minnow/README.md`](../../examples/minnow/README.md) |
| Lox | a dynamic language, from *Crafting Interpreters* | [`examples/lox/README.md`](../../examples/lox/README.md), [Dynamic languages](dynamic.md) |
| Kaleidoscope | LLVM's tutorial language | [the tutorial](../tutorial.md) |
| Tin | a systems language without a garbage collector | [`examples/tin/README.md`](../../examples/tin/README.md), [Memory](memory.md) |
| Calc, Lispy | languages defined by their grammar alone | [`examples/calc/README.md`](../../examples/calc/README.md) |
| Pebble | a language written in Python | [A0 from Python](../python.md) |

[The language guides](languages.md) say what each one is for and what to read in it.

## How to read this book

1. [Getting started](getting-started.md) makes a language with `a0 new` and runs it.
2. [The tutorial](../tutorial.md) builds Kaleidoscope step by step. Every listing is cut from the
   real code.
3. Part II is the reference for what a language is made of: grammars, the HIR, diagnostics, the
   standard library and packages.
4. Part III is about the tools a language gets without code of its own: editors, the debugger and
   the REPL.
5. Part IV goes further: a full language, memory and systems programming, threads, dynamic
   languages, performance, other platforms, and using A0 from C and Python.
6. Part V is the reference for the rest: the compatibility promise, the API, and the decisions
   behind A0.
