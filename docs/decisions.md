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

Revised in M16, after binding the core IR, interpreter, native backend, grammar and HIR text APIs in
C (`a0-capi`, D33): the line holds, and binding was mechanical, about 15 lines of C API per Rust
function at most. Four clarifications came out of it. Text a Rust API produces as an owned `String`
(a module printed, diagnostics summarized) is kept in the C wrapper object and returned borrowed, so
APIs may return `String` where they compute text. Enums with many variants that have a text form
(opcodes, comparison conditions) are bound by their names, which cannot drift from the Rust side.
A method that takes `self` (`Builder::finish`) consumes the C object. And an object C must keep using
after an operation that takes ownership needs `Clone` (`Module` gained it for `a0_run`). Not yet
bound, and slightly outside the line where they are: the HIR builder, whose `Item` enum carries
data (a binding would split it into a kind and a handle), and `SyntaxIndex`, whose `IndexedToken`
holds an `Option` (a binding would use `A0_NONE`).

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

## D12. The semantic toolkit starts with scopes only (superseded by D21)

*Superseded: `a0-sema` gained static types (D21) once Minnow, a typed language, needed them.*

`a0-sema` provides lexical scopes (declare, define, resolve, capture detection), which both example
languages need. A type checker is not built yet: Kaleidoscope has a single type and Lox is dynamically
typed, so there is no second language to generalize from (see the fundamentals).

## D13. Lox's object model and garbage collector live in Lox's runtime (superseded by D16)

*Superseded: the collector, strings and maps moved into `a0rt` (D16), and Lox's C runtime is gone.*

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

Later (phase 3): integers that fit in 48 bits are stored inline under their own tag
(`0x7ffe` in the top 16 bits) and only larger ones are boxed, so the common case does not allocate.
Each integer has exactly one encoding, which keeps equality a word comparison for inline integers.
References keep the low 48 bits of the pointer; user-space addresses fit on every supported target,
and the collector panics if an address it hands out does not, instead of corrupting references.

## D16. One precise collector in `a0rt`, generalized from Lox's (supersedes D13)

The default runtime owns the garbage collector. It is Lox's design made general: objects carry a type
descriptor with a per-word layout (raw, reference, `any`); roots are the shadow stack (every word in the
`any` encoding), registered globals and C-side temporaries; collection is mark-sweep with an explicit
gray stack; `A0_GC_STRESS=1` collects on every allocation. It works the same in the interpreter and in
native code and needs nothing from Cranelift. Strings are not interned; maps hash them by content.
Equality compares cached hashes before contents, so comparing the same long strings repeatedly is
O(1) after the first time.
References are nullable because languages need cyclic and optional structures; a null dereference
panics.

## D17. The HIR text format is parsed with A0's own grammar layer

`a0-hir` depends on `a0-syntax` and `a0-sema` for its text format (a grammar file plus a tree walk,
instead of a third hand-written parser). That is a deliberate exception to "frontend and IR are
independent": the HIR is a layer above both, and using the grammar layer for it is also its biggest
real-world test.

## D18. Small objects come from size-class free lists

`a0rt` first allocated every object and every array buffer with `calloc` and released it with `free`.
That cost about 40 ns per object, and Lox on the HIR spent most of its time there (every call
allocates its argument array). Objects up to 256 bytes now come from free lists, one per 16-byte size
class, carved from 64 KB chunks; the sweep returns dead objects to their lists. Bigger objects still
use `malloc`. Chunks are never returned to the system, which is acceptable for the programs A0 runs
today; a compacting or generational collector would be the next step if it is not. Under
`A0_GC_STRESS=1` dead objects are filled with `0xdd`, so a missing root shows up as garbage at once
instead of as stale data that happens to still look right. Arrays created with a capacity of at most
16 keep their elements inline in the object, so an array literal is one allocation.

## D19. Lox globals are HIR globals with a "defined" flag

The first port kept Lox globals in a map looked up by name at run time, which made every global
access two hash lookups (about 13 ns, ten times the old IR's indexed globals). Each Lox global is now
a HIR global of type `any` plus a `bool` HIR global that says whether it has been defined; reading
or assigning an undefined one is jlox's runtime error. Names are prefixed (`g_x`, `d_x`) so they never
clash with each other or with the prelude.

## D20. One command-line driver for all languages, in the tools layer

`a0-driver` gives every language the same `run`, `build`, `ir` and `hir` commands, diagnostics
rendering and exit-code mapping; a language supplies a `Language` value (its compile function,
exit codes, and C runtime hooks if it has one). Compile errors are either `Diagnostics`, rendered
against the source, or messages in the language's own format, because Lox must print jlox's format
exactly. Like the `a0` CLI, the driver is a Rust convenience in the tools layer, not part of the
C-bindable library API (D2): it takes plain `fn` pointers and returns `ExitCode`. A C host would call
the library crates directly.

## D21. Static types in `a0-sema`: interned types, nominal structs, unification

`a0_sema::Types` holds a language's types as handles. Named types (`int`), arrays and functions are
interned structurally; every struct declaration is a distinct type. Type variables stand for types
not known yet and are bound by unification (with an occurs check), which is all the inference a
small language needs (`let xs = []; push(xs, 1);`), without committing a language to a full
Hindley-Milner system: there is no generalization, so no polymorphism. `expect` gives every
language the same mismatch message. Frontends use a fresh variable as the type of an erroneous
expression, so one mistake does not cascade. The API is handle based with a fieldless `TypeTag` and
accessors, per D2. Types stay separate from HIR types: the toolkit is part of the frontend layer,
which must not depend on an IR, and a language's types need not map one to one onto the HIR's.

## D22. Minnow is statement-oriented and writes struct literals with `new`

Minnow, the typed example language, has statements (`let`, `if`, `while`, `for`, `return`) and
expressions, like most languages people write compilers for, rather than the HIR's expression
blocks. Struct literals are `new P { ... }`: with `P { ... }`, `if p { ... }` would be ambiguous.

## D23. References are non-null unless their type says `T?`

Phase 2's HIR made every reference nullable and checked for null at every use. Now a reference type
`T` excludes null and `T?` admits it; `null<T>` has type `T?`, a `T` fits where a `T?` is expected,
and `e!` is the only way back (it panics with `null reference`). Field access, indexing and calls
require a non-null value. Soundness needs every way a reference comes into being to produce a valid
one: struct literals give every field, arrays and maps start empty, `let` always has an initializer,
`any` holds `()` rather than a null reference, and globals (which start zeroed) must be nullable if
they are references. Generated code therefore checks for null only at `!`. Flow typing (`if x !=
null` making `x` non-null) was left out: it would be the first HIR feature that depends on control
flow, and a frontend can bind `x!` to a local where it needs it.

## D24. The HIR builder infers types; frontends need no checker of their own

Each builder call types its node with unification over inference variables (`fresh_type`, `[]`,
generic calls, calls of unknown closures). A rule that needs structure the type does not have yet is
deferred and retried at `finish`; what stays unknown is reported as "cannot infer". Errors carry the
span of the sub-expression they are about and read as messages for the language's users. The
checker re-types every node of the finished module as a safety net. Unification is invariant; the
only implicit conversions are `never` to anything and `T` to `T?` at the top of a type. This is what
lets a typed language skip writing its own checker (M12); `a0_sema::Types` (D21) remains for
languages whose type systems do not map onto the HIR's.

## D25. Generics by monomorphization in a separate pass

Generic structs, enums and functions are type checked once, with their type parameters opaque (no
bounds), and instantiated by `mono.rs` into plain copies, one per combination of type arguments,
before lowering. Lowering, the native backend and the optimizer therefore never see a type parameter,
and every instance gets its own object layouts (an `int` field is raw, a `string` field is a
reference). Instance names carry their arguments (`id<int>`), which no frontend name can contain.
The alternative, a uniform representation with boxing (as Java does), would have kept one copy per
item but cost an allocation per `int` stored in a generic container.

## D26. Name resolution and error recovery belong to the HIR builder

Every frontend of a typed language repeats the same work: scopes, resolving a name to a local,
global, function or builtin, struct literals by field name, "unknown name" errors. The builder now
offers this (`frontend.rs`) on top of its handle-based API, so a frontend is one walk over its tree;
Minnow went from 1513 lines (with its own checker) to 692 while gaining enums, generics and nullable
types. Two details make that possible. An error gives its node the error type (`TypeId::ERROR`),
which fits and unifies with everything and spreads to whatever is inferred from it, so a mistake is
reported once; earlier, an error typed `never` could not fix an inference variable, and later uses
then produced follow-on errors. And diagnostics show types in the language's own syntax
(`set_type_syntax`), while the text format keeps the HIR's. Item names may be anything; the text
format quotes those that are keywords (`` `map` ``) instead of making frontends rename them.

## D27. An optimizer of our own, on the HIR, on by default

Cranelift optimizes each function well but does not inline, and it cannot know what an `any` holds.
Languages on the HIR express their semantics as small HIR functions (Lox's operators are prelude
functions), so calls and `any` boxing dominate their run time. The optimizer works on the
monomorphized HIR, where types are concrete: it inlines small functions (turning mid-function
`return`s into `if` values), decides `is`/`as` on `any` statically where the boxed type is known,
and cleans up (constants, copies, dead code). It is about 820 lines, runs in linear passes, is on by
default for every HIR language with no frontend work, and `A0_OPT=0` turns it off. It never removes
or duplicates an expression with an effect. Languages that emit the core IR directly (Kaleidoscope)
do not get it; an inliner on the core IR would be the next step if one needs it.

## D28. Fast paths for `any` live in lowering and the runtime, not in frontends

Dynamic languages spend their time testing, unpacking and comparing `any` values, so these paths are
the framework's to make fast. Lowering tests an object's type inline (two loads and a compare)
instead of calling the runtime, and decides `any` equality inline unless both words are references.
Generated code thereby depends on two offsets in the runtime's layouts (an object's type 24 bytes
before it, a type's family 24 bytes into its descriptor); `a0rt.h` documents them and `gc.c` checks
them at compile time. The optimizer marks a cast as unchecked when an enclosing `is` test of the
same never-assigned local guards it, so `if v is T { v as T }` tests once. In the runtime, array and
map buffers up to 256 bytes come from the same free lists as small objects (D18), a map remembers the
entry its last lookup found (so `has(m, k)` followed by `m[k]` hashes once), and string equality
compares cached hashes before bytes. The alternative, letting each language write such paths in its
prelude, would contradict the goal that programs run fast without tuning by the language author.

## D29. Lox functions take up to three parameters directly

Lox checks arity at run time, so the first port compiled every function to a closure taking an
argument array, and every call allocated that array. Functions with at most three parameters now
compile to closures that take them directly, stored in the `LoxFunction` field for their arity
(`code0` .. `code3`; `code` holds the array form of longer ones). Call sites with up to three
arguments call the prelude's `call0` .. `call3`, which call the code of matching arity directly and
send everything else (classes, natives, arity errors) through the general `call`. They are written
without early returns so the optimizer inlines them at every call site. This is what any frontend
for a language with dynamic arity does on a typed IR; it costs the prelude about 100 lines and
halves the time of call-heavy programs (`invocation`, `fib`).

## D30. Frames are cleared by the code that used them, not when pushed

The collector scans every word of every frame on the shadow stack, so a frame must not hold stale
words from earlier frames: those may point at freed objects. `a0_gc_frame_push` used to clear the
new frame with `memset`. HIR functions have large frames (inlining gives each inlined call its own
locals), and clearing 20 to 30 words on every call cost more than the rest of a small function's
call overhead: with MSVC's `memset`, `fib` spent about 40% of its time there. Now generated code
clears each slot when its scope ends or is left (`return`, `break`, `continue`), so the words above
the stack's top are always zero and pushing clears nothing. A call that returns early touches only
the slots it used, and blocks reuse the slots of blocks before them. Under `A0_GC_STRESS`,
`a0_gc_frame_pop` checks that the frame was cleared, so a lowering bug fails the stress tests. At
the same time, reference locals became IR variables written through to their slots instead of
being reloaded from them, which the non-moving collector (D16, D18) allows. Hand-written users of
the frame API clear their frames before popping (`a0rt.h`).

## D31. Editors learn a language's names from grammar annotations

Editor support needs to know what a file declares and what each name refers to. Three sources were
considered. The compiler's own name resolution is exact, but only frontends that build HIR by name
(D26) go through the framework's resolver; Lox has its own resolver and Kaleidoscope emits core IR,
so both would need code just for editors. Heuristics over the syntax tree (a field called `name`
declares something) fail on common grammars: `new Point { x: 1 }` and `struct Point { x: int }` have
the same shape. So the grammar says it, declaratively, like it already says `@inline` and
`@recover`: `@def(kind)` marks a rule whose nodes declare a name, `@def(kind: field)` one whose tokens
in `field` do, and `@scope` a rule that opens a scope. `SyntaxIndex` (in `a0-syntax`) resolves names
lexically from these and classifies tokens from the grammar's own lexer. It is approximate where types
matter (a member name refers to every member of that name), and the language server adds types from
the HIR when the compiler records spans. Lox, Minnow and Kaleidoscope needed 6 to 13 annotations
each and no code.

## D32. The language server lives in the driver and writes its own JSON

`lang lsp` is a command of `a0-driver`, so every language that uses the driver has a server without
writing one: diagnostics from its compiler, everything else from D31. The protocol needs JSON;
`a0-driver` has a small JSON module of its own (about 300 lines with tests) instead of depending on
`serde_json`, since it needs only values, a parser and a printer, and the driver stays free of
dependencies outside the workspace. The server is single-threaded, keeps whole documents and
re-analyzes a document on every change; the example languages compile in milliseconds, so there is
no need yet for incremental analysis or cancellation. Editors are reached through the protocol only:
`editors/vscode` is a generic client that starts `<command> lsp` for configured file types.

## D33. The C API is a static library over the library crates, with names for opcodes

`a0-capi` binds the library crates, not the driver (D20): a C host builds and verifies core IR,
prints and parses it, runs it in the interpreter, compiles it to objects or executables, parses
with grammars, and lowers HIR text. It is a static library, so a host links one file plus the system
libraries Rust needs, and nothing has to be installed at run time; a shared library would add a
loader path to every platform's setup. Instructions and conditions are passed by their text-format
names, types by a small enum. Mistakes C can make with a builder (an unknown name, a bad type
code) are recorded and reported by `a0_builder_finish` with the builder's and the verifier's
findings, so the C side, like the Rust side, never crashes on misuse. A C program in the test suite
(`tests/host.c`) exercises all of it and is compiled and run by the system C compiler in CI on every
platform.
