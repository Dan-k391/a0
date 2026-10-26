# A0 core IR

Status: implemented in `crates/a0-ir`. Keep this document in sync with the code.

## Goals

- Easy to produce from a frontend: no hand-built SSA, no basic blocks, no phi nodes.
- Easy to consume: an interpreter and a native backend (Cranelift) both walk it directly.
- A textual format that round-trips exactly, so IR can be written by hand, diffed and golden-tested.
- A verifier that reports diagnostics; the IR libraries never panic on malformed input.
- C-bindable API: entities are opaque `u32` handles, the module is the only owner.

## Model

A **module** is an ordered list of items. Every item has a symbol name that is unique in the module.

| Item | Text | Meaning |
|---|---|---|
| data | `data @msg = "hi\0a"` | Read-only bytes. |
| global | `global @n: i64 = 0` | One mutable scalar, initialized, exported under its name. |
| extern function | `extern func @sin(f64) -> f64` | Defined outside the module (C ABI). |
| function | `func @f(%x: f64) -> f64 { ... }` | Defined here; always exported under its name. |

A **function body** is a *region*: an ordered list of instructions. Control flow is structured:
`if` and `loop` instructions own nested regions. There are no labels or gotos.

**Values** (`%name`) are immutable and defined exactly once, by a parameter or an instruction.
A value is visible after its definition in the same region and in regions nested inside it,
never outside the region that defines it (lexical scoping, like a block-structured language).
This replaces dominance: what you can see in the text is what you can use.

**Variables** (`$name`) are mutable, typed, function-local storage declared at the top of the body
(`var $i: i64`). `get`/`set` read and write them. Variables start out as zero. Frontends never build SSA:
anything that changes over time, or that flows out of an `if`, goes through a variable. The native
backend hands variables to Cranelift's SSA construction; the interpreter keeps them in a frame.

## Types

`i1 i8 i16 i32 i64 f32 f64 ptr`. `ptr` is an opaque address whose width comes from the target.
Functions return zero or one value. There are no aggregate types yet: aggregates live in memory and are
accessed with `load`/`store`/`ptradd` (see "Not yet" below).

## Instructions

Every value-producing instruction is written `%v: T = op operands`. The result type is always printed;
for most opcodes it is implied by the operands and the verifier checks that it matches.

| Group | Opcodes | Notes |
|---|---|---|
| constants | `iconst N`, `fconst F`, `null`, `addr @sym` | `addr` takes a function, data or global. |
| integer | `iadd isub imul sdiv udiv srem urem and or xor shl lshr ashr` | Both operands and the result share one integer type. |
| integer unary | `ineg not` | |
| float | `fadd fsub fmul fdiv` , `fneg` | |
| compare | `icmp CC a, b` with CC in `eq ne slt sle sgt sge ult ule ugt uge`; `fcmp CC a, b` with CC in `eq ne lt le gt ge` | Result `i1`. `icmp` also accepts two `ptr`s. Float compares are ordered, except `ne` (true when unordered). |
| conversion | `zext sext trunc sitofp uitofp fptosi fptoui fext ftrunc bitcast ptrtoint inttoptr` | The target type is the result type. |
| select | `select c, a, b` | `c: i1`. |
| memory | `load p`, `store v, p`, `ptradd p, off` | `off: i64`, byte offset. Unaligned access is allowed. |
| stack | `stack_slot SIZE, ALIGN` | The address of `SIZE` bytes in the function's frame, aligned to `ALIGN` (a power of two up to 16). Each instruction has its own slot, the same address on every execution within one call and another in each call; it is gone when the function returns, and its contents start undefined. |
| calls | `call @f(args)`, `call_indirect p(args) : (T..) -> R` | `call_indirect` spells out the signature. |
| variables | `get $v`, `set $v, x` | |
| control | `if c { } else { }`, `loop { }`, `break [n]`, `continue [n]`, `return [v]`, `unreachable` | `break`/`continue` refer to the innermost `loop`, or with `n` to the loop `n` loops out from it (`break 1` leaves two loops). |

Defined behaviour (identical in the interpreter and native code):

- Integer arithmetic wraps. Shift amounts are taken modulo the bit width.
- `sdiv udiv srem urem` by zero, and `sdiv` of the minimum value by -1, **trap**.
- `fptosi`/`fptoui` saturate; NaN converts to 0.
- `unreachable` traps.
- A trap prints `a0: trap: <reason>` to stderr and exits with status 101.

Instructions after a terminator (`return`, `break`, `continue`, `unreachable`) in the same region are
allowed and never execute. A function with a return type must not be able to fall off the end of its
body: the verifier proves that the body *terminates* (it ends in a terminator, an `if` whose two branches
both terminate, or a `loop` with no `break`). A void function that falls off the end returns.

## C structs

C functions that take or return structs by value are declared with the struct's layout on a `ptr`
parameter (D73):

```
extern func @div(ptr sret(8, 4: i32 0, i32 4), i32, i32)
extern func @blend(ptr byval(24, 8: f64 0, i8 8, f32 12, f64 16), f32) -> f32
```

- `byval(SIZE, ALIGN: TYPE OFFSET, ...)`: the argument points at the struct's bytes, which are passed
  by value. The callee gets a copy, so the memory is not changed.
- `sret(...)`, on parameter 0 of a function that returns nothing in the IR: the argument points at
  where the struct the C function returns goes.
- The layout is the struct's size and alignment and its scalar fields with their offsets, nested
  structs flattened: what a C ABI looks at. The verifier checks that the struct parameters are
  `ptr`s of externs, that the size is a multiple of the alignment (a power of two), and that every
  field fits, aligned.

Each backend passes the struct as the target's C ABI says, from one implementation in the native
backend (`a0-native/src/cabi.rs`): System V x86-64 in up to two registers, integer or SSE, or copied
onto the stack; Windows x64 in a register when it is 1, 2, 4 or 8 bytes, otherwise by pointer to a
copy; AArch64 in floating-point registers for one to four floats of one type, in up to two
registers when it is 16 bytes or less, otherwise by pointer. Results come back in registers the same
way, or are written through a pointer the caller passes. Where a struct goes also depends on the
registers the arguments before it left: one that does not fit goes in memory as a whole (System V
then gives the registers to the arguments after it; AArch64 does not, and A0 uses them up with
unused arguments, since Cranelift has no struct arguments there).

The interpreter's own foreign calls take up to eight integer or `f64` arguments and return one.
It calls other C functions (C structs, `f32`, more arguments) through **thunks**:
`a0_native::add_thunks(&module, &mut symbols)` compiles, in memory, one function
`void thunk(const uint64_t *args, uint64_t *ret)` per such C function, which the interpreter finds
with `HostSymbols::thunk`. The drivers and the C API add them before interpreting; without them the
interpreter reports which function it cannot call.

## Entry point

A module that is run or turned into an executable defines `func @main() -> i32`. The interpreter and the
native backend both initialize the C runtime before calling it and pass its result as the exit status.
Natively, the backend emits the C `main` symbol as a small wrapper, and the IR `main` is emitted as
`a0.main`.

## Text format

```
module "fib" version 1

extern func @printd(f64) -> f64

func @fib(%n: f64) -> f64 {
  var $r: f64
  %two: f64 = fconst 2.0
  %small: i1 = fcmp lt %n, %two
  if %small {
    set $r, %n
  } else {
    %one: f64 = fconst 1.0
    %a: f64 = fsub %n, %one
    %fa: f64 = call @fib(%a)
    %b: f64 = fsub %n, %two
    %fb: f64 = call @fib(%b)
    %s: f64 = fadd %fa, %fb
    set $r, %s
  }
  %res: f64 = get $r
  return %res
}
```

`version 1` names the version of the text format (`docs/compatibility.md`); it is optional, 1 when
absent, and the printer always writes it. A0 1.x reads version 1 and refuses a later one with
`E0004`.

- Comments run from `;` to the end of the line.
- Names: `@sym`, `%value`, `$var`. A bare name matches `[A-Za-z_][A-Za-z0-9_.]*`; anything else is
  quoted (`@"binary|"`), with `\"`, `\\` and `\XX` hex escapes. All-digit names (`%0`, `$3`) mean
  *unnamed*; the printer numbers unnamed values and variables in order of appearance.
- Strings in `data` use the same escapes. Integers are decimal, read in the result type's width.
  Floats use the shortest decimal that round-trips, `inf`, `-inf`, `nan`, or `nan(0x...)` with the
  exact bit pattern for any other NaN.

**Round-trip guarantee.** For every module `M`, `parse(print(M))` has the same items, signatures,
names and instructions as `M`, and `print(parse(print(M))) == print(M)` byte for byte. Comments and
formatting in hand-written text are not preserved; `print` is the canonical form.

Value and variable names are unique within a function. The builder makes them unique (`x`, `x.1`, ...)
so that names, not internal ids, are what round-trips.

## Builder API

The `Builder` owns the module while it is being built (no lifetimes), and tracks an insertion point:

```rust
let mut b = Builder::new("demo");
let sin = b.declare_extern("sin", &[Type::F64], Some(Type::F64));
let f = b.declare_function("f", &[Type::F64], Some(Type::F64));
b.begin_function(f);
let x = b.param(0);
let acc = b.declare_var("acc", Type::F64);
let c = b.fcmp(FloatCC::Lt, x, zero);
b.if_(c);  /* then */  b.else_();  /* else */  b.end_if();
b.loop_(); /* body */  b.break_(); b.end_loop();   // break_out(n), continue_out(n): outer loops
b.ret(Some(v));
b.end_function();
let module = b.finish()?;  // structural errors + verifier diagnostics
```

`patch_iconst` changes the value of an `iconst` built earlier, for numbers known only after more code
is built (a frame size emitted in a prologue). `set_struct_param(f, index, pass, size, align,
field_types, field_offsets)` gives a C function's struct parameter its layout ("C structs" above;
`func_struct_param` reads it back).

Misuse (an `else_` without an `if_`, building outside a function, ...) is recorded as a diagnostic and
reported by `finish`, rather than panicking, so the API stays safe to expose over C.

## Locations and stack traces

Runtime errors say where they happened and how the program got there, in both backends, with the
same text (`docs/diagnostics.md`). The IR carries what that needs; none of it is in the text format.

- Every instruction has a span (set with `set_span`; the text parser sets it to the instruction's
  text) and an inline site (`set_inline`): `add_inline_site(callee, call, parent)` records that a
  source-level function was inlined at `call`, inside code itself inlined at `parent`, and
  instructions copied from it name it.
- Every function has a trace name, its IR name by default: `set_trace_name` gives the source name,
  `hide_from_traces` leaves out entry points and wrappers.
- `Module::set_sources` gives the module the `SourceMap` its spans point into; the drivers do it
  before running or building.
- `Locations::build(&module)` makes the table of locations both backends give the runtime: for each
  instruction, its function, file, line and column, and for inlined code the location of the call
  it was inlined at.

Runtime functions that can fail (`a0_runtime::REPORTS_LOCATION`: array and map access, substrings,
casts out of `any`, panics, traps) take two arguments more than the IR declares, which the backends
pass: the location of the call and the caller's frame pointer (null in the interpreter). Native code
keeps frame pointers and emits a table from the return address of each call to its location, so the
runtime walks the frame pointer chain through generated frames only; nothing is done at calls that
do not fail. A large program is compiled into several objects at once (`compile_objects`, D82),
each with the table of its own code (`a0_rt_add_code`); the location table is one. The interpreter
keeps the location of each active call and gives it to the runtime when asked.

## Debug information

Debuggers (`docs/debugging.md`) show variables by what the frontend says they hold; none of it is in
the text format either.

- `debug_type(kind, name, elems)` makes a source-level type: its name as the language writes it
  (`[int]`, `Point`) and a `DebugKind` saying how its 64-bit words read: `int`, `float`, `bool`,
  `string` (a pointer to an `A0String`), `array` and `map` (with element types), `struct` (an object
  of fields, added with `debug_type_add_field`), `enum` (an object whose first word is the variant,
  added with `debug_type_add_variant`), `nullable`, `any`, `boxed` (a captured variable's box),
  `object` (whatever type the object's descriptor names) and `opaque`. Types are made before their
  fields, so that they can refer to themselves.
- `set_var_debug(var, name, type, decl)` says which source variable a `var` holds and where it is
  declared; variables without it are the compiler's own (shown by their IR names and types only in
  modules with no debug types at all, like a frontend's core IR).
- `Module::var_debug`, `debug_type_kind` and the like read it back. HIR lowering describes every
  local; the native backend uses the locations for DWARF line tables (`-g`).

## Verifier

`verify(&Module) -> Diagnostics`. It checks symbol references, operand and result types, call
signatures, value scoping, `break`/`continue` placement, return types, termination of non-void
functions, and that names are unique. When the module came from text, diagnostics point at the source
span of the offending instruction.

## Not yet

Aggregate types, switch and calling-convention attributes are left out until an example language
needs them (see `docs/decisions.md`); aggregates live in memory, stack slots included, and cross
into C as "C structs" above say.
