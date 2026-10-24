# Milestones

One entry per milestone: what was done, what changed along the way, and open questions.

## M1. Core IR and interpreter

**Done**

- `a0-diag`: source map, spans, diagnostics with rustc-style rendering. Shared by every layer.
- `a0-ir`: the structured IR described in `docs/ir.md`, written down before the code.
  - Builder with matched `if_`/`else_`/`end_if` and `loop_`/`end_loop` calls, mutable variables, and
    automatic unique naming. Misuse becomes a diagnostic, never a panic.
  - Text format with an exact round-trip guarantee, tested byte for byte on a file that uses every
    opcode, every float edge case (`-0.0`, `inf`, `nan(0x...)`, `1e300`) and quoted names.
  - Verifier for types, value scoping, call signatures, variables, `break`/`continue` placement and
    termination of non-void functions. Diagnostics point at the text when there is one.
- `a0-runtime`: the C runtime (lifecycle, traps, output, memory, time) as a static library, a symbol
  table for the interpreter, and `a0_ffi_call`, a generated trampoline for calling C from Rust.
- `a0-interp`: compiles functions to a slot-resolved op tree, runs them on a shared stack with real
  host memory, calls C through the trampoline, reports traps as values and bounds recursion depth.
- `a0`: `check`, `fmt` and `run` subcommands.
- Tests: unit tests per crate and `tests/programs`, a corpus of IR programs with expected stdout,
  stderr and exit status, run end to end through `a0 run`.

**Changed**

- Rust was not installed on the development machine; it was installed with `rustup` (stable,
  MSVC toolchain) after asking.
- The text format grew `nan(0x...)` to keep NaN payloads exact, and all-digit names are reserved for
  unnamed values so that names, not internal ids, are what round-trips.
- IR traps (division by zero, `unreachable`) are checked explicitly so that the interpreter and native
  code print the same message and exit with the same status (101).

**Open questions**

1. *C-bindability.* D2 in `docs/decisions.md` spells out how "no Rust-specific types in public
   signatures" was interpreted: `&self`, `&str`/`&[T]` inputs and borrowed `&str`/`&[Handle]` returns
   are allowed, lifetimes on types, generics, closures and data-carrying enums are not. Is that the
   intended line?
2. *Interpreter FFI limits.* Calls from the interpreter to C support up to 8 integer/pointer/`f64`
   arguments; `f32` arguments and results are rejected with a diagnostic. Native code has no such limit.
   Good enough until a language needs more?
3. *Small integer arguments to C.* IR integers are signless, so `i8`/`i16` arguments to C functions are
   not extended according to the C type's signedness. This only matters for C functions with `char` or
   `short` parameters (and on Apple AArch64). Frontends should pass `i32`/`i64`.

## M2. Native executables from IR

**Done**

- `a0-target`: explicit `Target` (architecture + OS) with triple, object format, calling convention,
  PIC requirement, linker flavor and file suffixes. Codegen never asks the host.
- `a0-native`: lowers IR to Cranelift. IR variables are handed to `cranelift-frontend`, which builds the
  SSA; `if`/`loop` become blocks; dead code after a terminator is not emitted. Division by zero, signed
  overflow and `unreachable` call `a0_rt_trap`, so native code prints exactly what the interpreter
  prints. With an entry point, the IR `@main` is emitted as `a0.main` behind a generated C `main` that
  initializes the runtime.
- Linking uses the platform linker (`link.exe` found through the `cc` crate's MSVC discovery, `cc`
  elsewhere) with the runtime's embedded static library. Only the host target can be linked.
- `a0 build` (executable) and `a0 obj --target T` (object for any target).
- Tests: every corpus program runs natively and must meet the same expectations as in the
  interpreter; every program is compiled for every emittable target and the objects are inspected
  (format, architecture, exported and imported symbols). A new `semantics.a0ir` exercises integer
  wrapping, shifts, narrow types, `i1` arithmetic and saturating conversions in both backends.

**Changed**

- The differential tests found one real bug: `i1` lives in an `i8` in Cranelift, so `iadd` on `i1`
  produced 2. Arithmetic results on `i1` are now masked to one bit.
- Cranelift 0.137 enables only the host ISA by default; both the `x86` and `arm64` backends are now
  enabled explicitly.

**Open questions**

1. **aarch64-windows cannot be produced.** `cranelift-object` 0.137 has no AArch64 relocations for
   COFF (it hits `unimplemented!`). The backend now reports a diagnostic instead of crashing. Fixing it
   means either contributing COFF/ARM64 relocation support to Cranelift upstream, or writing that part
   of object emission ourselves, which is close to the "don't write our own linker" line. I have not
   worked around it. Which way do you want to go, or should the target wait?
2. Linking was exercised on x86-64 Windows only, the machine available here. The Linux and macOS link
   paths are written but not run; their objects are checked structurally.
3. Executables get an 8 MiB main-thread stack where the linker can set it (Windows, macOS). On Linux
   the stack size comes from `ulimit`.
