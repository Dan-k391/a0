# Design decisions

Short records of decisions that shape the code. Newest last. Each entry says what was decided and why.

## D1. Workspace of small crates, one per layer

`a0-diag` (sources, spans, diagnostics) is shared by every layer. Then `a0-ir` (IR, builder, text,
verifier), `a0-runtime` (C runtime + its Rust build glue), `a0-interp`, `a0-target`, `a0-native`
(Cranelift backend + system linker), `a0-syntax` (grammar layer), `a0-sema` (semantic toolkit) and the
`a0` command-line driver. Example languages live in `examples/`. Layers only depend downward, so each one
can be used on its own; the frontend does not depend on the IR and vice versa.

## D2. What "C-bindable" means for public APIs

Public signatures use opaque owner types (`Module`, `Builder`, `Grammar`, `SyntaxTree`, ...) and `u32`
handles (`FuncId`, `Value`, `NodeId`, ...). Allowed: `&self`/`&mut self` receivers (they become a handle
argument), `&str` and `&[T]` of plain data as inputs, and `&str`/`&[Handle]` returns that borrow from
the owner (a C API returns pointer + length, valid until the owner changes). Not allowed in public
signatures: types with lifetime parameters, generic parameters or trait objects, closures, and Rust
enums carrying data. Plain `#[repr(u8)]` enums are fine. `Option<T>` and `Result<T, Diagnostics>`
are used where a C API would use a sentinel or a status code plus an out-parameter.

## D3. Structured IR with mutable variables

The IR has no basic blocks: `if`/`loop` own nested regions, values are lexically scoped, and mutable
state goes through function-local variables. This is what frontends naturally produce, it makes the
interpreter a straightforward tree walk, and Cranelift's `cranelift-frontend` already turns variables
into SSA, so we do not write SSA construction (or any optimizer) ourselves. See `docs/ir.md`.

## D4. Backends: interpreter first, Cranelift for native code, system linker

Cranelift is the existing code generator (pure Rust, x86-64 + AArch64, ELF/Mach-O/COFF through
`cranelift-object`). LLVM is out of scope. Executables are linked by the platform's linker (`link.exe`
for MSVC targets, `cc` elsewhere); we never write a linker.

## D5. Every codegen decision goes through `a0-target`

`Target` is explicit (architecture, OS, object format, linker flavor, ...). The backend looks up the
Cranelift ISA from the target's triple, never from the host. `Target::host()` exists for tools that
genuinely mean "this machine" (the interpreter's FFI, the default of the CLI). Linking requires the
target to equal the host, because cross-compilation is out of scope; object emission works for all six
supported targets, and tests exercise that.

## D6. One C runtime, used by both backends

The runtime (`a0-runtime`) is compiled once at build time into a static library. Rust crates link it
(the interpreter calls it directly), and the native backend embeds the library bytes and passes them to
the linker. Interpreted and native programs therefore share printing, traps, memory management and
exit behaviour, which is what makes differential testing meaningful.

## D7. The interpreter calls C through a generated trampoline

The interpreter resolves `extern func`s through a symbol registry (name -> address) filled from tables
that C libraries export. Calls go through `a0_ffi_call`, a C function generated at build time that
switches over every signature shape of up to 8 integer-class or `f64` arguments. The C compiler does the
ABI work, so it is correct on SysV, Win64 and AArch64 without assembly or libffi. Signatures outside that
shape (for example `f32` arguments) are reported as errors by the interpreter, not by native code.

## D8. Interpreter memory is real memory

`load`/`store`/`ptradd` in the interpreter use real host addresses, globals are allocated in host
memory, and pointers can be passed to and from C freely. Function addresses of IR functions are opaque
tokens that only `call_indirect` inside the interpreter understands.

## D9. Grammars are data, loaded at run time

A grammar is a text file in a small DSL (`docs/grammar.md`), parsed by `Grammar::from_str` when the
compiler starts. There is no parser generator step and no Rust code per language, which keeps the
frontend layer C-bindable (a grammar is a string, a tree is handles) and makes a grammar easy to edit.
Lexing uses our own small longest-match regex engine, because lexers need POSIX-style maximal munch
across all token patterns, which backtracking regex libraries do not provide directly.

## D10. Ordered choice, furthest-failure errors, custom messages, panic-mode recovery

Rules are PEG-style ordered choice with backtracking, which is forgiving to write. Errors are reported
at the furthest token reached; `x ~ "message"` attaches the exact message a hand-written parser would
give, and `@recover` gives jlox-style panic-mode recovery. The furthest failure is kept across
recovered constructs, so a branch abandoned by backtracking (a broken `else`) is still reported where it
broke. Together these reproduce jlox's messages and recovery exactly, which the Lox suite requires.

## D11. Operator precedence: static `@prec`, dynamic `resolve_precedence`

Both example languages need precedence parsing. Fixed tables are declared in the grammar (`@prec`, a
Pratt parser with prefix, infix and postfix forms). Tables that programs extend (Kaleidoscope's
`def binary| 5`) are parsed as flat chains and resolved later with `resolve_precedence`.

## D12. The semantic toolkit starts with scopes only

`a0-sema` provides lexical scopes (declare, define, resolve, capture detection), which both example
languages need. A type checker is not built yet: Kaleidoscope has a single type and Lox is dynamically
typed, so there is no second language to generalize from (see the fundamentals).

## D13. Lox's object model and garbage collector live in Lox's runtime

Only Lox needs garbage collection, strings and hash tables so far, so they are in
`examples/lox/runtime/lox.c` rather than in `a0rt`, following "generalize only when two or more
example languages need something". The collector is precise: compiled code keeps live values in
shadow-stack frames that the runtime scans, which works the same in the interpreter and in native code
and needs no stack maps from Cranelift. When a second language needs a collector, the language-neutral
parts (allocation with type descriptors, gray stack, shadow stack, weak tables) can move into `a0rt`.

## D14. A high-level IR above the core IR, as a typed expression tree

The HIR (`docs/hir.md`) is a typed tree of expressions, not a list of instructions: it is what a
frontend already has after parsing and resolving names, and it makes closures, `match` and blocks with
values natural. It lowers to the core IR plus runtime calls, so both backends, the verifier and the
text format of the core IR keep working unchanged underneath. The core IR stays the low-level layer.
The "two languages" rule does not apply to the HIR and the default runtime: they are the product.

## D15. `any`: NaN-boxed dynamic values

Dynamically typed languages need a value that can hold anything. `any` is a 64-bit NaN-boxed word:
floats unboxed, `()`/booleans as NaN patterns, references as tagged pointers, `int`s boxed. Lox numbers
therefore never allocate, and type tests are a few bit operations in the generated code. Enums alone
would have forced a heap object per number.

## D16. One precise collector in `a0rt`, generalized from Lox's (supersedes D13)

The default runtime owns the garbage collector. It is Lox's design made general: objects carry a type
descriptor with a per-word layout (raw, reference, `any`); roots are the shadow stack (every word in the
`any` encoding), registered globals and C-side temporaries; collection is mark-sweep with an explicit
gray stack; `A0_GC_STRESS=1` collects on every allocation. It works the same in the interpreter and in
native code and needs nothing from Cranelift. Strings are not interned; maps hash them by content.
References are nullable because languages need cyclic and optional structures; a null dereference
panics.

## D17. The HIR text format is parsed with A0's own grammar layer

`a0-hir` depends on `a0-syntax` and `a0-sema` for its text format (a grammar file plus a tree walk,
instead of a third hand-written parser). That is a deliberate exception to "frontend and IR are
independent": the HIR is a layer above both, and using the grammar layer for it is also its biggest
real-world test.
