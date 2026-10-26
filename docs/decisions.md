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
into SSA, so we do not write SSA construction ourselves. See `docs/ir.md`. (This also said we
write no optimizer; D27 later added a small one on the HIR, which Cranelift cannot replace.)

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
and cleans up (constants, copies, dead code). It is about 820 lines, runs in linear passes (inlining bounded as in D37), is on by
default for every HIR language with no frontend work, and `A0_OPT=0` turns it off. It never removes
or duplicates an expression with an effect. Languages that emit the core IR directly (Kaleidoscope)
do not get it; an inliner on the core IR would be the next step if one needs it.

## D28. Fast paths for `any` live in lowering and the runtime, not in frontends

Dynamic languages spend their time testing, unpacking and comparing `any` values, so these paths are
the framework's to make fast. Lowering tests an object's type inline (two loads and a compare)
instead of calling the runtime, and decides `any` equality inline unless both words are references.
Generated code thereby depends on two offsets in the runtime's layouts (an object's type 24 bytes
before it, 8 since D64, a type's family 24 bytes into its descriptor); `a0rt.h` documents them and `gc.c` checks
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

The HIR builder is bound the same way (M17). Its Rust API trusts handles, which can only come from
the builder there; the C layer checks each handle against the module's counts before calling in
and reports a bad one, so the builder itself stays unchanged. `Item` parameters, the one
data-carrying enum in its API, are a kind and a handle in C (as D2 proposed), operators and
builtins are passed by their text-format names, and lists of names are arrays of `A0Str`. A test
compares the header with the library's exports, so neither can gain a function the other lacks.

## D34. Nullable references are narrowed with `if let`, not by flow typing

D23 left a frontend two ways to use a `T?`: `x!`, which panics on null, or a test followed by `x!`.
Flow typing (a test changes the type of `x` afterwards) would remove the `!` but ties the type of a
name to the control flow around it: every language would get one particular set of narrowing rules,
and the builder would need them for locals, fields, captured variables and loops. `if let x = e`
instead binds a new local of type `T` for one branch. It is one node with a local typing rule, any
language can spell it as it likes (Minnow has `if let` and builds `while let` from it with `loop`
and `break`), and it costs one null test, the same as `!`. Lox's prelude uses it for its call fast
paths, and Minnow's programs lost their `!`s where a test came first.

## D35. Kaleidoscope is built on the HIR, and numeric code pushes no frame

Kaleidoscope emitted the core IR because it has one type, which made it the one example language
without the builder's name resolution and without the optimizer, and the tutorial taught a path
other languages do not take. It now builds HIR by name like Minnow: 745 lines became 556 (579 once
the tutorial gave each chapter's additions regions of their own), its unknown names and wrong
argument counts are reported by the builder, and its user-defined operators are inlined. What it kept is what is Kaleidoscope's own: operator precedence decided while compiling
(`resolve_precedence`), its prototype rules, and conditions that treat NaN as false. To keep numeric
code free of the collector's costs, lowering skips the shadow-stack frame of a body that has no
reference or `any` value at all (no such local, capture or expression), so such functions make no
runtime calls; `fib` runs as fast as from the core IR, and a loop calling an operator 40% faster.

## D36. The tutorial is cut from the code and checked by a test

A tutorial whose code is typed into Markdown drifts from the code it describes. Every listing in
`docs/tutorial.md` instead names a file and `ANCHOR` regions of it (`rust
file=examples/kaleidoscope/src/lib.rs anchor=control`), and `examples/kaleidoscope/tests/tutorial.rs`
checks that each listing is exactly that code, that the listings together show every line of the
compiler, grammar, runtime and command, and that every command in a `console` block prints what
the block shows; `A0_BLESS=1` rewrites them after a change. Later chapters add code rather than
change earlier listings: `match` arms and grammar alternatives that a chapter introduces have
regions of their own, so the program as it stands after each chapter builds and runs (checked by
hand for chapters 3 to 5 when the tutorial was written). This is mdBook's anchor convention without
mdBook, since the tutorial is plain Markdown that GitHub renders.

## D37. Inlining goes bottom-up and is bounded by sizes after inlining

The first inliner measured each callee once, before any inlining, and then inlined into the copies
it made, up to three levels deep. A chain of helpers that each call the previous one twice grew
exponentially: every copy carried the copies inlined into it. The optimizer now visits functions
callees first and copies a callee as it is after its own inlining, so a copy is never inlined into
again, and it measures the callee at that point: a function is inlined only if its body is at most
120 nodes after inlining (the old limit was 80 nodes of a function's own code, which with its small
callees inlined came to about 100). Each body may grow to 4000 nodes or to twice its size, whichever
is more, the rule GCC uses for large functions; a limit relative to the size alone stopped inlining
in long straight-line code such as a loop of 30 calls. The Lox suite compiles as fast as before
(3.2 s with the optimizer, 2.3 s without), the benchmarks run as fast, and a chain of 40 helpers
(`tests/hir/inline_chain.hir`), which did not finish before, compiles in a fifth of a second.

## D38. Random programs are HIR text, checked against each other

The optimizer and lowering are tested by hand-written programs whose output is known. To find what
nobody thought to write, `crates/a0/tests/random.rs` generates programs from a seed and requires the
six modes (interpreted, native, GC stress; optimized and not) to print the same and exit the same
way, with the unoptimized interpreter as the reference, so no expected output is needed. The
generator writes HIR text rather than calling the builder: a failing program is then a file that
`a0 run` reproduces and that can be shrunk by hand. Programs are well-typed by construction and
terminate by construction (loop counters nothing else assigns, recursion only on a fuel parameter
that decreases, a budget of calls per function), so a program that does not compile or a run that
differs is a bug. The generator is part of the test (about 700 lines, no dependency), with its own
small PRNG. Its first run found a bug: the text format gave an `if let` binding no type until the
whole `if let` was typed, so a `match` in its body could not find its enum.

## D39. Diagnostics render as rustc's do, colored on terminals, with no dependency

A diagnostic now has a label under its primary span, related labels, notes, helps and suggested
fixes, and `a0-diag` renders it in rustc's layout: each file it points into is one snippet holding
every label there (primary `^^^`, others `---`, labels that do not fit on the marker row hanging
below on `|` connectors), spans over several lines drawn with a track in the margin (`/` when they
start where the line's code starts), one missing line shown and longer gaps elided with `...`,
other files under `:::`, long lines cut around what they point at. A fix that replaces the primary
span with a short text becomes its label (`help: ...: `count``); others are shown as the corrected
line. We wrote the renderer (about 600 lines) instead of using `annotate-snippets` or `codespan`:
both would be new dependencies, and the C API and the language server need the same data model,
which is ours either way. Colors are ANSI escapes when stderr is a terminal, off with `NO_COLOR`
and forced with `CLICOLOR_FORCE` (the no-color.org conventions); on Windows the driver asks the
console for escape processing with `SetConsoleMode`, declared directly instead of through a crate.

## D40. Error codes are numbered, explained, and the language's to show

Codes were layer names (`hir.type` for every type error), which say where an error was found but
not what it is. Every problem a program can have now has a code of its own in `a0_diag::codes`
(`E0xxx` syntax, `E1xxx` names, `E2xxx` types, `E3xxx` control flow, `E4xxx` the entry point,
`Wxxxx` warnings) with a title and an explanation, which `lang explain <code>` prints, as
`rustc --explain` does. Numbers rather than names because they are stable while wording changes,
and short enough to search for. The explanations speak of concepts, not syntax, since each
language writes them its own way. A language shows A0's codes, none, or its own through a mapping
(`Codes::Own`), which `explain` then accepts. Problems only a language's own code can cause (a
misused builder, a malformed tree) keep descriptive codes, since they are bugs to fix in the
language, not errors to explain to its users.

## D41. Messages say what is wrong at the node and point at where it came from

A message is now short and a primary label says what is wrong at the span, as rustc does
("mismatched types" / "expected `int`, found `string`"), and related labels show where an
expectation came from: the declared variable, the parameter (`set_param_span`), the field, the
function's result type, the earlier declaration of a name declared twice, the declaration of a
called function, the variants a `match` misses. The builder knows all of these already, so
languages get them without work beyond attributing declarations to their names. Wording that was
the HIR's own is gone or the language's: concepts come from `Term`s (`set_term(Term::Narrow,
"`if (x != nil)`")`), the unit type shows as `()`, a null nothing typed yet as `null` rather than
`_?`, and suggestions that only make sense in HIR syntax are phrased with terms. The text format,
which is HIR's own language, keeps its words.

## D42. Suggestions come from the builder's scopes, and fixes need the name's span

The builder resolves names, so it knows what was in scope when a name was not: "did you mean"
candidates are the visible locals (innermost first), then globals, functions and builtins (for
calls), types, a struct's fields or an enum's variants, ranked by edit distance with rustc's
threshold (a third of the length) and case-insensitive equality first. A suggestion becomes a fix,
which the language server offers as a quick fix, only where the builder knows the name's own span:
the current span when it is exactly as long as the name, or one the frontend gives with
`set_name_span`; otherwise it is a help line. A heuristic on spans, rather than requiring every
by-name call to take a name span, keeps the by-name API unchanged and is right for every frontend
that attributes a name node to its name, which is what the example languages do.

## D43. Warnings are opt-in per language and wait for a program without errors

What deserves a warning depends on the language: Lox's output is prescribed by the book's suite,
and a scripting language may want unused variables. So the builder finds warnings only when a
language turns them on (`enable_warning`), starting with the two most useful and least
controversial: unused variables (not parameters, not names starting with `_`, which callbacks and
intentional discards use) and unreachable code (once per block, pointing at what diverged). They
are found at `finish` and only for a program without errors, as rustc does, since an error makes
code look unused (a misspelled name leaves the real one unread). A finished module carries its
warnings, so the driver can print them before running or building, the language server can show
them, and a C host can read them, without changing what `finish` returns.

## D44. Rendered diagnostics are snapshot-tested per language

Substring checks of messages say nothing about what a user sees: labels, snippets, alignment,
fixes. Each example language has `tests/diagnostics/` with programs and a `.out` file holding
exactly what `lang run` prints (stdout, stderr, exit status), checked by
`a0_driver::snapshots::check`, which also builds each program that compiles and requires the
executable to print what the interpreter printed. Programs run with their directory as the working
directory, so paths in diagnostics are as written. `A0_BLESS=1` rewrites the files, as the tutorial
test does; the diff is the review. The helper lives in the driver, beside `lsp::session`, because
every language's tests use it.

## D45. Runtime errors find their stack from tables, not from work at every call

A runtime error must say where it happened, with the values involved, and show the stack of
source-level functions, inlined ones included, the same in both backends, while programs that do
not fail must not get slower. So nothing is recorded at calls. The IR knows each instruction's span
and the call it was inlined at (HIR inlining records it), and both backends build the same table of
locations from it (`a0_ir::Locations`). Only the runtime functions that can fail
(`REPORTS_LOCATION`) take the failing operation's location and the caller's frame pointer as two
hidden arguments, and traps pass them from their cold blocks. Native code keeps frame pointers and
emits, from Cranelift's source locations, a table from each call's return address to its location,
so the runtime walks the frame pointer chain and stops at the first return address outside
generated code: C frames, whose frame pointers MSVC does not keep, are never read. The interpreter,
which has no native frames, keeps the location of each active call in a thread-local stack and hands
it to the runtime through a callback; the C runtime formats both. We rejected a shadow stack of call
sites (a store at every call), unwinding through DWARF or Windows unwind tables (Cranelift does not
emit them into objects, and it would mean `gimli` and per-OS code), and `backtrace`-style symbol
lookup (it knows machine functions, not inlined source functions or columns). The Lox benchmarks
run as fast as before natively (geometric mean ×1.00 over ten benchmarks, interleaved, best of
five) and interpreted. A first version was 3.6% slower: the failing functions formatted their
messages in place, and the stack buffer this gave them (with MSVC's stack checks) cost every call;
formatting now happens in separate cold functions. A stack overflow prints no trace, as Rust does:
it would repeat one call thousands of times, and its depth differs between the backends.

## D46. Modules are files compiled into one HIR module, found through the grammar

Programs of several files could have been compiled file by file and linked, or as one module
with a namespace per file. A0 does the second: the driver loads the main file and, transitively,
what it imports, and compiles every file into one HIR builder, each after `begin_module`. Types
and functions then cross files with no interface files or linking, and the optimizer and
monomorphization see the whole program. Imports are found from a grammar annotation (`@import`,
whose `path` field names the module) rather than from the compiler, so the driver can load files
before compiling any, and the language server and DSLs with no host code get modules too. A
module name is resolved from the main file's directory, so a file is the same module whoever
imports it; a string path is relative to the importing file. Cycles are rejected: they would need
declarations collected across files before any is compiled, and the languages that motivate
modules do not need them. Module 0 is the prelude every module sees; the main file's names stay
as they are so its `main` is the entry point, and other modules' items are `module.item`, which is
also how stack traces name them. Visibility is the builder's: `set_public` before declarations,
checked on `alias.item` and on imports by name.

## D47. C functions take strings and arrays as pointers into A0's objects

Calling C was limited to numbers, so every C function that takes a string needed a wrapper in a
language's runtime. Now an `extern fn` takes strings as `const char *` and arrays of `int`, `float`
or `bool` as pointers to their elements, pointing into the objects themselves: A0's strings are
NUL-terminated and arrays are contiguous 64-bit words, and C code cannot start a collection, so
nothing is copied or pinned (a string with a NUL byte looks shorter to C; the alternative, a copy per
call, costs on every call). A string result is copied, since A0 cannot know who owns C's memory.
Bools are passed as 64-bit 0 or 1, because Cranelift does not extend small integers at calls. The
interpreter, which used to find only functions in registered tables, also looks the name up in the
process (`dlsym`, the loaded modules' exports on Windows), so a declaration alone calls a C library
function in both backends; native code links against the same libraries. Functions that need A0's
own values (to return a new array, say) are declared `extern "a0"` and get the objects: the
runtime's way to grow without a new builtin each time, and the standard library's (D48).

## D48. The standard library is HIR over runtime functions, added on import

A library every language can offer could have been builtins (typed and lowered one by one in the
builder and the lowering), a C library each language wraps, or code in the HIR. It is HIR text,
one module per area (`std.fs`, `std.str`, ...), over `extern "a0"` functions in the runtime's
`std.c`: the builder parses a module into the program the first time the program imports it, so
nothing is compiled unless used, functions are written in HIR where that is enough, and adding one
needs no change to the builder, the checker or the lowering. Fallible operations return
`std.Result<T, E>` with a message, which result-style languages use as it is and exception-style
languages turn into their exceptions; the C side reports failure through a last-error message
(one thread, and C functions return one value). Names `std` and `std.*` are reserved for the
library, so a program's own `std/` directory is not a module. The library's code has no spans for
now: its text is not in the driver's source map, so diagnostics and traces cannot point into it.

## D49. Exceptions are a pending flag and checks after calls, not unwinding

Exception-style languages need `throw` to leave any number of frames, identically in the
interpreter and in native code. Unwinding native frames (`setjmp`/`longjmp`, or Cranelift's
exception tables with an unwinder of our own) would differ between the backends, need care with
values in registers and with the shadow stack of GC roots, and on Windows interact with SEH. Instead
an exception is two globals, a flag and the value (an `any`, registered as a root), and every exit
is an ordinary one: `throw` sets them and leaves to the innermost `try` of its function (a `try`
body runs in an IR loop, which `break N`, added for this, leaves from inside other loops) or returns
with the exception pending, clearing GC slots as `return` and `break` already do. Calls that may
raise are followed by a test of the flag. Which calls may raise is computed over the functions
reachable from `main`, so programs that never throw get no checks, and a throw costs a walk of the
stack to remember where it happened, for an exception nothing catches. Runtime errors stay fatal
panics rather than exceptions: making them catchable would put checks after every operation that
can fail. Results need no runtime support: `propagate` (`?`) is a `match` and a `return`.

## D50. Lowering skips functions `main` cannot reach

With the standard library and language preludes added to programs, most functions a module holds
are never called. Lowering now follows calls and the closures a program makes from `main` and lowers
only those functions, which saves compile time in both backends (Cranelift compiles each function
it is given) and keeps `lang ir` output to the program's own code. The HIR, its text and its
diagnostics still cover every function, so errors in unused code are still reported.

## D51. Lox is the language with exceptions, as an extension

v0.2 asks for an example language that uses exceptions. Minnow returns results; a new language
would show nothing Lox cannot. So Lox gets `throw` and `try`/`catch` (with jlox-style syntax
errors), plus natives over the standard library whose failures are exceptions, kept apart from the
book: runtime errors stay fatal, so the official suite's behavior is untouched and Lox's operators
need no checks; natives other than `clock` exist only in programs that name them, so other programs
pay neither for the library nor for exceptions; and the extensions' tests (`tests/a0`) run with the
suite in all six modes but are counted apart from its 241.

## D52. Interface values are plain references; methods are found in tables on the type

Languages need methods, interfaces with dynamic dispatch, and generics that require operations of a
type. A value of an interface type could have been a fat pointer (the object and a table of
methods), as in Go and Rust, but every HIR value is one word, which the collector, `any`, arrays,
fields and nullability all rely on. So an interface value is the object reference itself, and each
type descriptor carries an interface table: for each interface of the program, the type's methods
for it. A call loads the type from the object's header, the table, and the method (three loads and
an indirect call); converting an object to an interface costs nothing; `x is I` reads the table.
Implementations are declared (`implement`), not inferred from the methods a type happens to have:
the declaration says which tables to build and gives errors a place to point at. Bounded type
parameters use the same interfaces, but monomorphization resolves their method calls to direct
calls, so generic code pays nothing at run time and value types (`int`) can satisfy bounds without
being objects. Methods that use `Self` work through bounds but not on interface values, which do not
know the object's type. Left for later: default methods, generic interfaces, and interfaces
implemented by value types at run time (which would need boxing).

## D53. The collector sweeps small objects chunk by chunk

The JSON tool spent more time in the collector than in its own code, and most of that sweeping: the
sweep followed a list linking every object, a cache miss per object. Small objects already lived
in chunks of equal cells, so the sweep now walks each chunk's cells in memory order, a free cell
being one whose type is null; only big objects (over 256 bytes, malloc'd) stay on a list. Buffers
(array elements, map tables) moved to chunks of their own, since their cells hold anything. The
header keeps its layout, so generated code still finds an object's type at the same offset.
Alternatives were a mark bitmap per chunk (the same walk, with more bookkeeping) or a moving,
generational collector, which would also cut marking, now the larger part, but needs the shadow
stack to be precise about every reference's location and is a project of its own.

## D54. `a0 new` writes a project from templates in the `a0` command

v0.3 asks that one command create a working language project. The project could have been a cargo
template (cargo-generate, an extra tool), a copy of an example language (too much to delete), or
files the `a0` command writes. It is the last: templates under `crates/a0/templates/starter`,
embedded in the binary, for a small typed language (functions, `let`, `if`, `while`, `print`) with
its driver, tests of its programs in both backends and of its language server, an example, VS Code
settings and a README. The project depends on A0 by git tag of the `a0` that made it, or on a local
checkout (`--a0`), which is how a test builds a generated project and runs its tests on every CI
run, so the template cannot rot.

## D55. Languages without host code are grammars whose rules name HIR forms

v0.3 asks for a small DSL defined with no host code. The meaning could have come from a second file
(a table from node kinds to HIR, which must be kept in step with the grammar), from HIR templates
with holes in the grammar (powerful, but a second language to learn inside the first), or from the
rules themselves. It is the last: `@hir(form)` on a rule, or a rule named after a form, picks one of
a fixed set of forms (`function`, `let`, `if`, `binary`, `call`, ...) whose parts are the rule's
fields, and `a0_hir::dsl` builds HIR from the tree with the ordinary builder, so names, inference,
diagnostics and the language server are A0's. A node without a form that has one child means that
child, which leaves wrappers and precedence nodes unannotated. `a0 lang <grammar>` runs such a
language with the usual driver commands. The set is small on purpose (numbers are floats, no types
of a language's own, no modules): past it, a language is a compiler, and `a0 new` starts one.

## D56. Python reaches A0 through the C API with ctypes, from a table checked against the header

v0.3 asks that languages can be built from Python on top of the C API. The choices were PyO3 (a
second binding of the Rust API, a build step for every user and a new dependency), cffi (a compiler
or a dependency at install time) or ctypes over the C API's shared library (`a0-capi` is now also a
cdylib). It is ctypes: nothing to install but the library, and Python exercises the same API C
does. Most of the binding is a table of the builders' C signatures in a one-letter notation, from
which the methods are made; a test parses `a0.h` and compares, so the table cannot drift from the
header. Two functions were added to the C API for it, and are useful to C as well:
`a0_diagnostics_render` (errors shown against their source) and `a0_hir_text`. A Python language
gets the driver's `run`, `build`, `hir` and `ir` from `a0.driver`; not its language server, which
needs the driver bound to C with a compile callback (left for later).

## D57. The playground is A0 built for wasm32-wasip1, with 64-bit pointer slots and call wrappers

v0.3 asks for a browser playground that runs A0 itself with nothing installed. The engine is the
interpreter and the C runtime, so the runtime had to compile to WebAssembly. Emscripten brings its
own JavaScript runtime; wasm64 would keep pointers 64-bit but is not in every browser; wasm32 with
wasi-sdk keeps the runtime plain C and the page in charge of the little system it needs. It is
wasm32-wasip1. Two things assumed 64-bit pointers: fields that generated code reads as words, now
`A0_WORD_PTR` slots (the same layout on every target, so lowering is unchanged), and calls through
a generic function pointer, which WebAssembly checks, so the symbol table holds generated wrappers
there. The page implements the WASI calls itself (about 110 lines) rather than using a library or
Node's built-in WASI, which crashed on these programs. The interpreter recurses on the engine's
stack, so in WebAssembly calls nest at most 1000 deep. The playground is published to GitHub
Pages from `main`; CI builds and tests it with Node on every push.

## D58. The debugger runs the program in the interpreter, behind the Debug Adapter Protocol

v0.4 asks for a debugger for every language with no extra code. Native code under gdb or lldb would
need a debug adapter per platform debugger and variable locations from Cranelift; a debugger of our
own over the interpreter needs neither. It is the latter, served as the Debug Adapter Protocol by
the driver (`lang dap`), so any DAP client works. The interpreter, built for debugging, marks each
new source line and calls a plain-function hook there and when the program fails before unwinding;
the hook decides on breakpoints and steps and, while stopped, answers requests from the frames.
Lowering records each local's source name, type and declaration (IR debug types, which DWARF could
use later), so values show as the language writes them. The program shares the adapter's process,
so the runtime hands its output to the debugger and keeps standard input from it; programs are
lowered without the HIR optimizer, so that every call keeps its frame. The cost: a debugged program
runs at the interpreter's speed, and globals and expressions beyond names are not shown yet.

## D59. Native debug information is DWARF lines and functions, written with gimli

The goal also asks for debug information for the platform's debugger where feasible. Cranelift
reports a source location for every instruction it emits (A0 already gave it each call's, for
traces), which is enough for DWARF line tables and subprograms: `-g` writes them with `gimli` (already
a dependency of Cranelift, now a direct one of `a0-native` with its `write` feature) into ELF and
Mach-O objects, relocated against the functions (on Mach-O against the code section, with each
function's address in the object written in place: dsymutil and lldb read the written value, as
LLVM's own objects have it), and runs `dsymutil` on macOS. Not done: variables,
whose locations would come from Cranelift's value labels and need location lists; and Windows, whose
debuggers read PDB, a format of its own (CodeView) that no library we use writes. `lang dap` covers
both.

## D60. Completion, references and rename are syntactic, across files through imports

v0.4 asks for completion, rename and find-references that stay responsive on multi-file projects.
They could come from the HIR (exact, but there only for programs that compile, and with spans on
expressions, not on type names) or from the grammar's indexes (`@def`, `@scope`) and the program's
imports (`@import`). They come from the indexes: a top-level item is found in the other files of
the program where they import it, as `alias.name` or as a name imported unqualified, so rename
works on programs with type errors and reaches type annotations too. The HIR adds the members of a
value's type after `.` for completion. What syntax cannot tell apart is refused rather than guessed
(a member name several types declare). For responsiveness the server reads messages on a thread of
its own: an edit only reindexes the document, and the program is analyzed when edits pause (200 ms)
or a request needs it, which turns a burst of edits into one analysis (a program of 10,000 lines
analyzes in about a quarter of a second).

## D61. The REPL reruns the program the inputs make, in a process of its own

v0.4 asks for a REPL for every language. An incremental one would keep the program's state between
inputs, which needs the interpreter to add functions to a running program and the runtime to keep
its heap across modules; the runtime's global state means one process runs one program. So the
REPL keeps the inputs as a program (top-level statements in order, or declarations and `main` for
a language that says how, `Language::repl_main`), runs all of it again for each input with `lang
run -` in a process of its own, and shows the output beyond what was shown before. The price:
earlier statements run again (output is not repeated, other side effects are), and an input costs
a run of the whole session. Inputs that fail are dropped, so the session always runs; the grammar
tells declarations from statements and unfinished lines, so a language needs at most one line.

## D62. The benchmark is a script over pinned references, outside CI

The benchmark should be small, honest and reproducible. `bench/run.py` (Python 3, which the
bindings' tests already need) builds A0's languages and the references, checks that every
implementation prints the same, and reports medians of wall-clock times. The references are
fetched at pinned versions rather than copied into the repository: clox at a fixed commit of
Crafting Interpreters, and the Kaleidoscope tutorial's chapter 7 at the tag of the installed LLVM,
since it must build against that LLVM. Lox uses the book's own benchmark programs, and Go is the
garbage-collected reference for Minnow.

LLVM's tutorial is a JIT that compiles a function when a call first needs it. So its compile time
is the same program with its calls doing no work, and its run time is the rest of the full run.
The script runs on Linux and macOS, not in CI: timings on shared runners are noise, and the goal
is a picture rather than a gate. Rejected: a timing tool such as hyperfine (one more thing to
install, for medians that the script takes itself), and copying the references' sources.

## D63. `run` compiles into memory with Cranelift's JIT; the interpreter stays the reference

v0.5 asks that `lang run` and the REPL run at native speed with no linking. The choices were a
faster interpreter (still 10 to 50 times slower than native code, D62's numbers), linking with a
faster linker (still a process and a file), or loading Cranelift's code into the running process.
It is the last, through `cranelift-jit` (Cranelift's own crate, the same version as the rest; a new
dependency of `a0-native`). The code generator was made generic over `cranelift_module::Module`, so
the JIT compiles exactly what an executable contains: the same functions and data, and the same
tables from return addresses to locations, which the runtime then reads from memory, so traces
come out the same. C functions resolve through `HostSymbols`, as in the interpreter, which moved
to `a0-runtime` for both. The program runs on a thread with an executable's stack (8 MB).
`a0_native::run_main` is the entry point, `a0_jit_run` its C binding.

`lang run`, `a0 run`, the REPL (which reruns `lang run -`) and Python's driver use it;
`A0_INTERP=1` interprets instead, and hosts without native code (WebAssembly) always interpret.
The interpreter stays the reference: the debugger runs on it (D58), the playground has nothing
else, and the differential tests run every program in it. Those tests now have eight modes: the
interpreter, the JIT, the JIT under GC stress and native executables, each optimized and not. GC
stress moved from the interpreter to the JIT, since that is where a missing root in generated
code would show and it runs faster there.

## D64. The collector's cheap gains: a 16-byte header, prefetched marking, a heap three times live

v0.5 asks that allocation-heavy programs spend clearly less time in the collector, starting with
the cheapest change that works. Without generations a collection costs about what survives it,
so the collector's total is about the allocation divided by the headroom left after each
collection; the cheap levers are fewer bytes per object, cheaper marking per object, and more
headroom. All three were taken; measured on the benchmark's `binarytrees` (Minnow) and
`binary_trees` (Lox), native, on Linux:

- The object header shrank from 32 to 16 bytes: the size (now 32 bits) and the mark bit, then the
  type, which generated code now finds 8 bytes before the object instead of 24. Only big objects
  (over 256 bytes) need the link and the 64-bit size the sweep uses; they get a second header
  before the first. Minnow's peak heap went from 19.6 to 16.0 MB.
- Marking pushes objects unmarked with their header prefetched, and checks the mark bit when they
  leave a queue of eight behind the gray stack, so the cache misses overlap; an object reached
  twice is pushed twice. About a fifth less marking time.
- The next collection comes when the heap has grown to three times what survived the last, not
  two: half as many collections for 1.4 times the peak memory.

Together: Minnow's collector time fell from 123 to 56 ms (the program from 0.30 to 0.20 s), Lox's
from 833 to 563 ms. What is left is marking the long-lived data again at every collection, which
only a generational collector avoids. That needs a write barrier on every store of a reference
into an object (two helpers in lowering and the runtime's array and map stores, it turned out)
and is left for when large programs show the need (v0.9). Lox's own share shrinks further with
objects that keep their fields in slots (D66).

## D65. Arrays are allocated at a size with `filled`, and booleans take a byte

v0.5 asks that arrays can be allocated at a size and that elements narrower than a word are
stored compactly; the benchmark's sieve paid for both (16 MB of 64-bit booleans, filled by
`push`). Allocation at a size could be `new array<T>(n)`, which needs a default value of every
type (references have none, D23), or a builtin that takes the value: `filled(n, v)`, an array of
`n` copies of `v`, one runtime call. Any language reaches it by name.

The only HIR type narrower than a word is `bool`, so `array<bool>` keeps a byte per element (the
runtime's element kind `A0_ELEM_BYTE`; maps with `bool` keys give their keys as such arrays), and
lowering loads and stores those bytes inline with a stride of one. The array's kind says how wide
its elements are, so the runtime, the debugger and C (which sees a `bool *`) read them alike, and
narrower integer types can join later without another layout. The sieve went from 84 to 50 ms
and from 16 to 2 MB.

## D66. Fast names for dynamic languages: dynamic objects with shapes, interned names and caches

v0.5 asks that dynamic languages get fast name lookup from the framework, and that Lox use it.
Lox kept an instance's fields in a `map<string, any>` and a class's methods in another, so a
property access was runtime calls that hashed and compared strings, a method call first allocated a
bound method, and an instance was four allocations (the struct, the map and its two tables). The
framework could have offered interned strings alone (equality by pointer, but still a hash table
per object), or per-site caches keyed by map (useless when every instance has its own map), or
what JavaScript engines do: hidden classes. It is the last, as a HIR type `object` with builtins
`has_field`, `field`, `set_field` and `field_names`:

- Names are interned by the runtime (`a0_intern`), once each, kept for the program's life.
- An object is a shape and a row of `any` slots (four inline). Shapes form a tree from the empty
  shape, each adding a name, so objects built alike share one, and a field's slot is a property of
  the shape. Shapes are never freed.
- Lowering gives each place whose field name is a literal a cache (shape, slot, and the shape
  after a set): a hit is a comparison and a load. The caches are words in one block allocated at
  startup, since the core IR has no mutable data. Other places use the runtime, which keeps a
  table by shape and name. The optimizer now propagates string literals through `let`s, so a
  prelude function inlined with a literal name gets a cache at each call site.

Lox's instances and classes keep their fields and methods in objects, and `o.m(args)` looks `m` up
before evaluating the arguments (jlox's order) and calls it on `o` with no bound method: a plain
function's code ignores its receiver, since only methods use `this`. Native Lox against the
previous version and clox (Linux, best of five): `binary_trees` 1546 → 521 ms (clox 1784; the
collector's share 833 → 134 ms), `zoo` 274 → 112 ms (clox 182), `method_call` 142 → 81 ms (clox
119), `invocation` 284 → 175 ms (clox 176). The debugger shows an object's fields by name, and
lowering now describes every struct and enum, so `any` values holding them show their fields too.

## D67. The driver is bound in C with a compile callback, so Python languages get all of it

v0.5 asks for the language server and the debugger for languages written in Python. Python's
driver was a copy of the Rust driver's simple commands; the server and the debugger are thousands
of lines that a copy would not keep up with. So the C API binds the driver itself: an opaque
`A0Language` with a name, a grammar, a REPL template and a compile callback (`text` in, an
`A0Hir` or diagnostics out), and `a0_language_main(l, args)`, which runs `a0_driver::Language` with
a compile function that calls the callback. The driver takes its compiler as a plain function, so
the callback lives in a global: one language per process, which is how drivers run anyway.
Python's `a0.driver.main` is now a few lines over it, and Python languages have every command of
Rust ones (`run` in the JIT, `build`, `lsp`, `dap`, `repl`, `explain`); the REPL reruns `python
lang.py run -` by the process's own arguments. The layer rule changes: the C API may depend on
`a0-driver`, which is the library languages build on, not a tool. `Language::run_status` returns the
exit status as a number for it. Not covered: programs of several files (`Modules` takes Rust
functions).

## D68. The standard library's modules are built-in sources

v0.5 asks that the standard library have source spans; it had none because its text is compiled
into the tools and is in no driver's `SourceMap`. Drivers could have added the library's texts to
every map they make (many places, and every new tool would have to remember), or the HIR could
have rewritten the library's spans after the fact. Instead `a0-diag` keeps built-in sources: texts
registered once per process at ids from `BUILTIN_SOURCES` up, which every `SourceMap` resolves as
if they were its own (`contains`, `name`, `text`, lines). The HIR registers each module of the
library when a program first imports it, as `<std>/fs.hir`, and parses it with spans there. A
process-wide table is justified here because the texts are constants of the binary, the same for
every map. Diagnostics then point into the library (the parameter declared there), stack traces
give its locations, and the language server and the debugger open a copy written to a directory
of this version of A0 under the system's temporary directory, since editors open files. Warnings
leave the library out.

## D69. Sized numbers: literals take the type expected of them

v0.6 asks for what a systems language needs, and C's types come first: explicit layouts and C
functions need 8-, 16- and 32-bit integers, unsigned ones and single floats, and bit operations.
The HIR gains `i8` .. `u64` and `f32` (`int` and `float` stay `i64` and `f64`), the operators `&`,
`|`, `^`, `~`, `shl` and `shr`, and conversions between all numbers. The question is the type of a
literal. Rust gives literals inference variables that default to `i32`; C gives them `int` and
converts. A0 already infers types, but literal variables would reach every rule of the builder.
Instead a literal is an `int` or a `float` unless a sized number type is expected of it (a
declaration, an argument, a result, the other operand, array elements), in which case it takes
that type if it fits (E2023): one function, `adapt_literal`, called where expectations meet, which
also looks through operators and branches made of literals. `let x = 200;` is an `int`, as in C.

Arrays of sized numbers keep each element in its own width (one, two or four bytes, generalizing
D65), so C sees `uint8_t *` and `int32_t *`. C functions take and return sized integers; those
narrower than 32 bits pass as C promotes them, extended to 32 bits by their sign. `f32` is not yet
a C function's parameter type: the interpreter's call trampoline (D7) has no single floats, which
calls through compiled thunks will lift (step five of v0.6). Values of sized types convert to
`any` as `int` or `float`. The debugger shows unsigned and single-float values (`DebugKind::UInt`,
`Float32`).
