//! An interpreter for the A0 core IR.
//!
//! The interpreter first compiles each function into a tree of [`Op`]s with operands resolved to
//! frame slots, then walks that tree. It shares memory, the C runtime and C calling conventions with
//! native code (see `docs/decisions.md`, D7 and D8):
//!
//! - `load`/`store` use real host addresses; globals live in host memory.
//! - `extern func`s resolve through [`HostSymbols`] and are called with `a0_ffi_call`.
//! - Addresses of IR functions are opaque tokens understood only by `call_indirect`.
//!
//! IR traps (division by zero, `unreachable`, ...) come back as [`Trap`] values; the embedding decides
//! how to report them (the `a0` driver prints them through the runtime, exactly like native code).

use std::collections::HashMap;
use std::rc::Rc;

use a0_diag::{Diagnostic, Diagnostics};
use a0_ir::{
    FloatCC, FuncId, GlobalId, Inst, IntCC, Module, Opcode, Region, SymbolKind, Type, Value,
};
use a0_runtime::{ffi_ret, trap};

pub use a0_runtime::HostSymbols;

/// Why execution stopped abnormally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    /// One of the `a0_runtime::trap` codes, or 0 for an interpreter-specific panic.
    pub code: i32,
    pub message: String,
    /// The locations of the active frames when it happened, innermost first (ids in the program's
    /// location table; see [`Trap::trace`]).
    pub frames: Vec<u32>,
}

thread_local! {
    /// The location of the active call in each interpreted frame, outermost first, for the
    /// runtime's stack traces.
    static CALLS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The interpreter being debugged while it is in a runtime function, for the runtime's
    /// failure hook.
    static DEBUGGEE: std::cell::Cell<*const Interpreter> = const {
        std::cell::Cell::new(std::ptr::null())
    };
    /// Why the program being debugged failed.
    static FAILURE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// What a debugger is told (see [`Interpreter::new_debug`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugEvent {
    /// Execution reached a new source line: [`Interpreter::debug_frame_location`] of call 0.
    Line,
    /// The program failed ([`Interpreter::debug_failure`]); its calls are still active.
    Failure,
}

/// A debugger's view of execution. It runs on the interpreter's thread; returning resumes it.
pub type DebugHook = fn(&Interpreter, DebugEvent);

/// The runtime's failure hook for a debugged program (`a0_rt_set_fail_hook`): tells the debugger,
/// while the failing call is still active, before the runtime reports the failure and exits.
///
/// # Safety
/// `message` points to `len` bytes (the runtime passes its message).
pub unsafe extern "C" fn runtime_failed(message: *const u8, len: usize) {
    let this = DEBUGGEE.with(|d| d.get());
    if this.is_null() {
        return;
    }
    // SAFETY: the runtime passes its message; the pointer was set from the interpreter that called
    // into the runtime, which never returns to it after a failure.
    let text = unsafe { std::slice::from_raw_parts(message, len) };
    FAILURE.with(|f| *f.borrow_mut() = Some(String::from_utf8_lossy(text).into_owned()));
    let interp = unsafe { &*this };
    if let Some(hook) = interp.hook {
        hook(interp, DebugEvent::Failure);
    }
}

/// The interpreter's frames, as the runtime asks for them when a runtime function fails.
unsafe extern "C" fn frames(out: *mut u32, max: usize) -> usize {
    CALLS.with(|calls| {
        let calls = calls.borrow();
        let n = calls.len().min(max);
        for (i, &loc) in calls.iter().rev().take(n).enumerate() {
            // SAFETY: the runtime passes a buffer of `max` entries.
            unsafe { *out.add(i) = loc };
        }
        n
    })
}

impl Trap {
    /// The trap at `loc`, with the frames active now.
    fn at(mut self, loc: u32) -> Trap {
        // A stack overflow's trace would be the same call thousands of times.
        if self.code != trap::STACK_OVERFLOW {
            self.frames = std::iter::once(loc)
                .chain(CALLS.with(|c| c.borrow().iter().rev().copied().collect::<Vec<_>>()))
                .collect();
        }
        self
    }

    /// The stack trace, as the runtime prints it (empty without one).
    pub fn trace(&self) -> String {
        a0_runtime::trace_text(&self.frames)
    }

    fn new(code: i32) -> Trap {
        let message = match code {
            trap::UNREACHABLE => "unreachable code was reached",
            trap::DIVIDE_BY_ZERO => "integer division by zero",
            trap::INTEGER_OVERFLOW => "integer overflow",
            trap::STACK_OVERFLOW => "stack overflow",
            _ => "unknown trap",
        };
        Trap {
            code,
            message: message.to_string(),
            frames: Vec::new(),
        }
    }

    fn panic(message: String) -> Trap {
        Trap {
            code: 0,
            message,
            frames: Vec::new(),
        }
    }

    /// Reports the trap the same way native code does and exits the process.
    pub fn report_and_exit(&self) -> ! {
        a0_runtime::fail_frames(&self.message, &self.frames)
    }
}

impl std::fmt::Display for Trap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

type Slot = u32;

/// One compiled instruction. Operands are frame slots.
enum Op {
    Const {
        dst: Slot,
        bits: u64,
    },
    /// The address of the bytes at `offset` in the frame's stack slots.
    StackSlot {
        dst: Slot,
        offset: u32,
    },
    IntBin {
        op: Opcode,
        ty: Type,
        dst: Slot,
        a: Slot,
        b: Slot,
        /// Where a division by zero traps.
        loc: u32,
    },
    FloatBin {
        op: Opcode,
        ty: Type,
        dst: Slot,
        a: Slot,
        b: Slot,
    },
    Unary {
        op: Opcode,
        ty: Type,
        dst: Slot,
        a: Slot,
    },
    Icmp {
        cc: IntCC,
        ty: Type,
        dst: Slot,
        a: Slot,
        b: Slot,
    },
    Fcmp {
        cc: FloatCC,
        ty: Type,
        dst: Slot,
        a: Slot,
        b: Slot,
    },
    Cast {
        op: Opcode,
        from: Type,
        to: Type,
        dst: Slot,
        a: Slot,
    },
    Select {
        dst: Slot,
        c: Slot,
        a: Slot,
        b: Slot,
    },
    Load {
        ty: Type,
        dst: Slot,
        p: Slot,
    },
    Store {
        ty: Type,
        v: Slot,
        p: Slot,
    },
    Ptradd {
        dst: Slot,
        p: Slot,
        off: Slot,
    },
    Call {
        func: u32,
        args: Box<[Slot]>,
        dst: Option<Slot>,
        loc: u32,
    },
    CallIndirect {
        sig: Rc<Shape>,
        callee: Slot,
        args: Box<[Slot]>,
        dst: Option<Slot>,
        loc: u32,
    },
    Copy {
        dst: Slot,
        src: Slot,
    },
    If {
        c: Slot,
        then: Box<[Op]>,
        els: Box<[Op]>,
    },
    Loop {
        body: Box<[Op]>,
    },
    /// Leaves (or restarts) the loop this many loops out from the innermost.
    Break(u32),
    Continue(u32),
    Return(Option<Slot>),
    Unreachable(u32),
    /// Execution reaches a new source line (only when debugging): its location id.
    Line(u32),
}

/// A call signature as the interpreter needs it.
struct Shape {
    params: Vec<Type>,
    ret: Option<Type>,
}

impl Shape {
    /// The `a0_ffi_call` description of this shape, or why it cannot be called from the interpreter.
    fn ffi(&self, structs: bool) -> Result<(u32, i32), String> {
        if structs {
            return Err("C struct parameters".to_string());
        }
        if self.params.len() > a0_runtime::FFI_MAX_ARGS {
            return Err(format!("more than {} arguments", a0_runtime::FFI_MAX_ARGS));
        }
        let mut mask = 0u32;
        for (i, &t) in self.params.iter().enumerate() {
            match t {
                Type::F64 => mask |= 1 << i,
                Type::F32 => return Err("f32 parameters".to_string()),
                _ => {}
            }
        }
        let ret = match self.ret {
            None => ffi_ret::VOID,
            Some(Type::F64) => ffi_ret::F64,
            Some(Type::F32) => return Err("an f32 result".to_string()),
            Some(_) => ffi_ret::INT,
        };
        Ok((mask, ret))
    }
}

struct Code {
    ops: Box<[Op]>,
    params: Vec<Slot>,
    frame_size: usize,
    /// The bytes of the function's stack slots (`stack_slot`), allocated for each call.
    slot_bytes: usize,
    /// The slot of each variable, for debuggers.
    vars: Vec<(a0_ir::Var, Slot)>,
}

enum FuncKind {
    Defined(Rc<Code>),
    /// A resolved C function.
    Native {
        address: usize,
        mask: u32,
        ret_kind: i32,
        /// It is one of the runtime's functions that take a location and a frame pointer.
        reports: bool,
    },
    /// A C function (at `function`) called through a thunk (at `address`, `HostSymbols::thunk`).
    Thunk {
        address: usize,
        function: usize,
    },
    /// An extern that is never called or referenced; calling it is a bug in the verifier's sense.
    Unresolved,
}

struct FuncInfo {
    name: String,
    shape: Rc<Shape>,
    kind: FuncKind,
}

enum Flow {
    Next,
    Break(u32),
    Continue(u32),
    Return(u64),
}

/// Default limit on nested calls before reporting a stack overflow trap. In WebAssembly each call
/// takes some 400 bytes of the engine's own stack, which is about 1 MB in browsers.
pub const DEFAULT_MAX_DEPTH: u32 = if cfg!(target_family = "wasm") {
    1_000
} else {
    10_000
};

/// A prepared program, ready to call into.
pub struct Interpreter {
    module: Module,
    funcs: Vec<FuncInfo>,
    /// Host memory backing `data` and `global` items. Never moved after preparation.
    _global_memory: Vec<Box<[u64]>>,
    /// Function tokens are addresses inside this allocation, so they never collide with real code.
    tokens: Box<[u8]>,
    /// The stack slots of the active calls that have any, innermost last.
    slot_memory: Vec<Box<[u128]>>,
    stack: Vec<u64>,
    depth: u32,
    max_depth: u32,
    /// The location table, in the runtime's format.
    debug: Vec<u8>,
    locations: a0_ir::Locations,
    /// The debugger, when one is attached (`Interpreter::new_debug`).
    hook: Option<DebugHook>,
    /// Per active call, innermost last, when debugging.
    frames: Vec<DebugFrame>,
    /// Whether the debugger was told of the failure being unwound.
    failure_told: bool,
}

/// A call as the debugger sees it.
struct DebugFrame {
    func: u32,
    base: usize,
    /// The location id of the line it is at (0 before its first).
    loc: u32,
}

impl Interpreter {
    /// Verifies and prepares a module. Every extern that the module calls or takes the address of
    /// must be present in `symbols`.
    pub fn new(module: Module, symbols: &HostSymbols) -> Result<Interpreter, Diagnostics> {
        Interpreter::new_with(module, symbols, None)
    }

    /// Like [`Interpreter::new`], for a debugger: `hook` is called whenever execution reaches a
    /// new source line, and when the program fails, before its calls are unwound; it can inspect
    /// the calls with the `debug_*` methods, and stops the program by not returning.
    pub fn new_debug(
        module: Module,
        symbols: &HostSymbols,
        hook: DebugHook,
    ) -> Result<Interpreter, Diagnostics> {
        Interpreter::new_with(module, symbols, Some(hook))
    }

    fn new_with(
        module: Module,
        symbols: &HostSymbols,
        hook: Option<DebugHook>,
    ) -> Result<Interpreter, Diagnostics> {
        let diags = a0_ir::verify(&module);
        if diags.has_errors() {
            return Err(diags);
        }
        let mut diags = Diagnostics::new();
        let referenced = a0_ir::referenced_externs(&module);

        let tokens = vec![0u8; module.func_count().max(1) as usize].into_boxed_slice();
        let token_base = tokens.as_ptr() as u64;

        // Globals live in 8-byte aligned host memory.
        let mut global_memory = Vec::new();
        let mut global_addrs = Vec::new();
        for g in 0..module.global_count() {
            let g = GlobalId(g);
            let mem = match module.global_type(g) {
                None => {
                    let bytes = module.global_bytes(g);
                    let mut words = vec![0u64; bytes.len().div_ceil(8).max(1)].into_boxed_slice();
                    // SAFETY: the buffer holds at least `bytes.len()` bytes.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            words.as_mut_ptr() as *mut u8,
                            bytes.len(),
                        )
                    };
                    words
                }
                Some(ty) => {
                    let mut words = vec![0u64; 1].into_boxed_slice();
                    unsafe { store(ty, words.as_mut_ptr() as u64, module.global_init_bits(g)) };
                    words
                }
            };
            global_addrs.push(mem.as_ptr() as u64);
            global_memory.push(mem);
        }

        let mut funcs = Vec::new();
        for f in 0..module.func_count() {
            let f = FuncId(f);
            let shape = Rc::new(Shape {
                params: module.func_params(f).to_vec(),
                ret: module.func_ret(f),
            });
            let name = module.func_name(f).to_string();
            let kind = if module.func_is_extern(f) {
                if !referenced[f.index()] {
                    FuncKind::Unresolved
                } else {
                    match symbols.lookup(&name) {
                        None => {
                            diags.push(
                                Diagnostic::error(
                                    "interp.unresolved",
                                    &format!("unresolved external function @{name}"),
                                )
                                .with_span_opt(module.func_span(f))
                                .with_note(
                                    "register its address in HostSymbols before interpreting",
                                ),
                            );
                            FuncKind::Unresolved
                        }
                        Some(function) if symbols.thunk(&name).is_some() => FuncKind::Thunk {
                            address: symbols.thunk(&name).unwrap(),
                            function,
                        },
                        Some(address) => match shape.ffi(module.func_has_struct_params(f)) {
                            Ok((mask, ret_kind)) => FuncKind::Native {
                                address,
                                mask,
                                ret_kind,
                                reports: a0_runtime::reports_location(&name),
                            },
                            Err(why) => {
                                diags.push(
                                    Diagnostic::error(
                                        "interp.ffi",
                                        &format!(
                                            "the interpreter cannot call @{name}: it has {why}"
                                        ),
                                    )
                                    .with_span_opt(module.func_span(f))
                                    .with_note(
                                        "a0_native::add_thunks compiles what it needs to call it",
                                    ),
                                );
                                FuncKind::Unresolved
                            }
                        },
                    }
                }
            } else {
                FuncKind::Unresolved // filled in below, once every function has a slot
            };
            funcs.push(FuncInfo { name, shape, kind });
        }
        if diags.has_errors() {
            return Err(diags);
        }

        let addr_of = |sym: a0_ir::Symbol| -> u64 {
            match sym.kind {
                SymbolKind::Global => global_addrs[sym.index as usize],
                SymbolKind::Func => match &funcs[sym.index as usize].kind {
                    FuncKind::Native { address, .. } => *address as u64,
                    FuncKind::Thunk { function, .. } => *function as u64,
                    _ => token_base + sym.index as u64,
                },
            }
        };
        let locations = a0_ir::Locations::build(&module);
        let mut codes = Vec::new();
        for f in 0..module.func_count() {
            let f = FuncId(f);
            codes.push(
                module
                    .func_body(f)
                    .map(|_| compile_function(&module, f, &addr_of, &locations, hook.is_some())),
            );
        }
        let entries: Vec<a0_runtime::debug::Entry> = locations
            .entries
            .iter()
            .map(|l| (l.func.as_str(), l.file.as_str(), l.line, l.col, l.parent))
            .collect();
        let debug = a0_runtime::debug::encode(&entries);
        for (info, code) in funcs.iter_mut().zip(codes) {
            if let Some(code) = code {
                info.kind = FuncKind::Defined(Rc::new(code));
            }
        }

        Ok(Interpreter {
            module,
            funcs,
            _global_memory: global_memory,
            tokens,
            stack: Vec::with_capacity(1 << 16),
            depth: 0,
            max_depth: DEFAULT_MAX_DEPTH,
            slot_memory: Vec::new(),
            debug,
            locations,
            hook,
            frames: Vec::new(),
            failure_told: false,
        })
    }

    // ---------------------------------------------------------------- debugging

    /// The number of active calls of defined functions (when debugging).
    pub fn debug_frame_count(&self) -> usize {
        self.frames.len()
    }

    /// The function of active call `i`, 0 being the innermost.
    pub fn debug_frame_function(&self, i: usize) -> FuncId {
        FuncId(self.frames[self.frames.len() - 1 - i].func)
    }

    /// The location id of the line active call `i` is at (0 if it has not reached one yet).
    pub fn debug_frame_location(&self, i: usize) -> u32 {
        self.frames[self.frames.len() - 1 - i].loc
    }

    /// A location by id: the function as traces name it, the file, line and column.
    pub fn debug_location(&self, id: u32) -> Option<&a0_ir::Location> {
        self.locations.get(id)
    }

    /// The variables of active call `i`'s function and their current values (raw bits).
    pub fn debug_frame_vars(&self, i: usize) -> Vec<(a0_ir::Var, u64)> {
        let frame = &self.frames[self.frames.len() - 1 - i];
        match &self.funcs[frame.func as usize].kind {
            FuncKind::Defined(code) => code
                .vars
                .iter()
                .map(|&(v, slot)| (v, self.stack[frame.base + slot as usize]))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Why the program failed, during a [`DebugEvent::Failure`].
    pub fn debug_failure(&self) -> Option<String> {
        FAILURE.with(|f| f.borrow().clone())
    }

    /// Tells the debugger that the program failed with `trap`, once, while its calls are active.
    fn tell_failure(&mut self, trap: &Trap) {
        if let Some(hook) = self.hook
            && !self.failure_told
        {
            self.failure_told = true;
            FAILURE.with(|f| *f.borrow_mut() = Some(trap.message.clone()));
            hook(self, DebugEvent::Failure);
        }
    }

    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Limits how deeply calls may nest before a stack overflow trap.
    pub fn set_max_depth(&mut self, depth: u32) {
        self.max_depth = depth;
    }

    /// Initializes the runtime with `args`, calls `@main` and returns its exit status.
    pub fn run_main(&mut self, args: &[String]) -> Result<i32, Trap> {
        let diags = a0_ir::check_entry(&self.module);
        if diags.has_errors() {
            return Err(Trap::panic(diags.get(0).message.clone()));
        }
        let main = self.module.lookup_func("main").unwrap();
        a0_runtime::init(args);
        // The runtime prints traces from the table until the process ends, after this
        // interpreter is gone.
        let table: &'static [u8] = Box::leak(self.debug.clone().into_boxed_slice());
        // SAFETY: the table lives until the process ends.
        unsafe { a0_runtime::set_debug(table, frames) };
        let result = self.call(main, &[])?;
        Ok(result.unwrap_or(0) as i32)
    }

    /// Calls a function with raw argument bits (integers in canonical form, floats as bits).
    /// Returns the raw result bits, or `None` for void functions.
    pub fn call(&mut self, f: FuncId, args: &[u64]) -> Result<Option<u64>, Trap> {
        let shape = self.funcs[f.index()].shape.clone();
        if args.len() != shape.params.len() {
            return Err(Trap::panic(format!(
                "@{} takes {} arguments, {} given",
                self.funcs[f.index()].name,
                shape.params.len(),
                args.len()
            )));
        }
        let base = self.stack.len();
        self.stack.extend_from_slice(args);
        let slots: Vec<Slot> = (0..args.len() as u32).collect();
        let r = self.call_func(f.0, base, &slots, 0);
        self.stack.truncate(base);
        let v = r?;
        Ok(shape.ret.map(|_| v))
    }

    /// Calls function `f` from a call at location `loc`.
    fn call_func(
        &mut self,
        f: u32,
        caller_base: usize,
        args: &[Slot],
        loc: u32,
    ) -> Result<u64, Trap> {
        match &self.funcs[f as usize].kind {
            FuncKind::Defined(code) => {
                let code = code.clone();
                if self.depth >= self.max_depth {
                    return Err(Trap::new(trap::STACK_OVERFLOW));
                }
                self.depth += 1;
                let base = self.stack.len();
                self.stack.resize(base + code.frame_size, 0);
                // Real memory that does not move while the call runs, aligned to 16.
                if code.slot_bytes > 0 {
                    self.slot_memory
                        .push(vec![0u128; code.slot_bytes.div_ceil(16)].into_boxed_slice());
                }
                for (&param, &arg) in code.params.iter().zip(args) {
                    self.stack[base + param as usize] = self.stack[caller_base + arg as usize];
                }
                CALLS.with(|c| c.borrow_mut().push(loc));
                if self.hook.is_some() {
                    self.frames.push(DebugFrame {
                        func: f,
                        base,
                        loc: 0,
                    });
                }
                let flow = self.exec(&code.ops, base);
                if self.hook.is_some() {
                    if let Err(trap) = &flow {
                        let trap = trap.clone();
                        self.tell_failure(&trap);
                    }
                    self.frames.pop();
                }
                CALLS.with(|c| c.borrow_mut().pop());
                self.stack.truncate(base);
                if code.slot_bytes > 0 {
                    self.slot_memory.pop();
                }
                self.depth -= 1;
                match flow? {
                    Flow::Return(v) => Ok(v),
                    _ => Ok(0),
                }
            }
            &FuncKind::Native {
                address,
                mask,
                ret_kind,
                reports,
            } => {
                let ret = self.funcs[f as usize].shape.ret;
                let extra = if reports { Some(loc) } else { None };
                self.native_call(address, mask, ret_kind, ret, caller_base, args, extra)
            }
            &FuncKind::Thunk { address, .. } => {
                let ret = self.funcs[f as usize].shape.ret;
                let words: Vec<u64> = args
                    .iter()
                    .map(|&a| self.stack[caller_base + a as usize])
                    .collect();
                let mut result = 0u64;
                if self.hook.is_some() {
                    let this: *const Interpreter = self;
                    DEBUGGEE.with(|d| d.set(this));
                }
                // SAFETY: the thunk was compiled for this function's IR signature, and takes the
                // arguments' words and where its result goes.
                let raw = unsafe {
                    a0_runtime::ffi_call(
                        address,
                        &[words.as_ptr() as u64, &raw mut result as u64],
                        0,
                        ffi_ret::VOID,
                    )
                };
                if self.hook.is_some() {
                    DEBUGGEE.with(|d| d.set(std::ptr::null()));
                }
                match raw {
                    Some(_) => Ok(ret.map(|t| normalize(t, result)).unwrap_or(0)),
                    None => Err(Trap::panic("unsupported C call shape".to_string())),
                }
            }
            FuncKind::Unresolved => Err(Trap::panic(format!(
                "call to unresolved external function @{}",
                self.funcs[f as usize].name
            ))),
        }
    }

    /// Calls a C function; `report` is the location to pass a runtime function that takes one
    /// (with a null frame pointer: the runtime asks the interpreter for its frames).
    #[allow(clippy::too_many_arguments)]
    fn native_call(
        &mut self,
        address: usize,
        mask: u32,
        ret_kind: i32,
        ret: Option<Type>,
        base: usize,
        args: &[Slot],
        report: Option<u32>,
    ) -> Result<u64, Trap> {
        let mut buf = [0u64; a0_runtime::FFI_MAX_ARGS];
        for (i, &a) in args.iter().enumerate() {
            buf[i] = self.stack[base + a as usize];
        }
        let mut n = args.len();
        if let Some(loc) = report {
            buf[n] = loc as u64;
            buf[n + 1] = 0;
            n += 2;
        }
        // A runtime function that fails never returns: the debugger hears of it through the
        // runtime's failure hook (`debug::runtime_failed`), which finds this interpreter here.
        if self.hook.is_some() {
            let this: *const Interpreter = self;
            DEBUGGEE.with(|d| d.set(this));
        }
        // SAFETY: the address came from a symbol table entry whose signature the IR declares.
        let raw = unsafe { a0_runtime::ffi_call(address, &buf[..n], mask, ret_kind) };
        if self.hook.is_some() {
            DEBUGGEE.with(|d| d.set(std::ptr::null()));
        }
        match raw {
            Some(v) => Ok(ret.map(|t| normalize(t, v)).unwrap_or(0)),
            None => Err(Trap::panic("unsupported C call shape".to_string())),
        }
    }

    fn call_indirect(
        &mut self,
        sig: &Shape,
        callee: u64,
        base: usize,
        args: &[Slot],
        loc: u32,
    ) -> Result<u64, Trap> {
        let token_base = self.tokens.as_ptr() as u64;
        if callee >= token_base && callee < token_base + self.tokens.len() as u64 {
            let f = (callee - token_base) as u32;
            let actual = &self.funcs[f as usize].shape;
            if actual.params != sig.params || actual.ret != sig.ret {
                return Err(Trap::panic(format!(
                    "call_indirect signature does not match @{}",
                    self.funcs[f as usize].name
                )));
            }
            return self.call_func(f, base, args, loc);
        }
        if callee == 0 {
            return Err(Trap::panic(
                "call_indirect through a null pointer".to_string(),
            ));
        }
        match sig.ffi(false) {
            Ok((mask, ret_kind)) => {
                self.native_call(callee as usize, mask, ret_kind, sig.ret, base, args, None)
            }
            Err(why) => Err(Trap::panic(format!(
                "the interpreter cannot call a C function pointer with {why}"
            ))),
        }
    }

    #[inline]
    fn get(&self, base: usize, s: Slot) -> u64 {
        self.stack[base + s as usize]
    }

    #[inline]
    fn put(&mut self, base: usize, s: Slot, v: u64) {
        self.stack[base + s as usize] = v;
    }

    fn exec(&mut self, ops: &[Op], base: usize) -> Result<Flow, Trap> {
        for op in ops {
            match op {
                &Op::Const { dst, bits } => self.put(base, dst, bits),
                &Op::StackSlot { dst, offset } => {
                    let memory = self.slot_memory.last().expect("a frame with stack slots");
                    let address = memory.as_ptr() as u64 + offset as u64;
                    self.put(base, dst, address)
                }
                &Op::Copy { dst, src } => {
                    let v = self.get(base, src);
                    self.put(base, dst, v)
                }
                &Op::IntBin {
                    op,
                    ty,
                    dst,
                    a,
                    b,
                    loc,
                } => {
                    let v = int_binary(op, ty, self.get(base, a), self.get(base, b))
                        .map_err(|t| t.at(loc))?;
                    self.put(base, dst, v)
                }
                &Op::FloatBin { op, ty, dst, a, b } => {
                    let v = float_binary(op, ty, self.get(base, a), self.get(base, b));
                    self.put(base, dst, v)
                }
                &Op::Unary { op, ty, dst, a } => {
                    let x = self.get(base, a);
                    let v = match op {
                        Opcode::Ineg => canon(ty, (x as i64).wrapping_neg() as u64),
                        Opcode::Not => canon(ty, !x),
                        _ => match ty {
                            Type::F32 => (-f32::from_bits(x as u32)).to_bits() as u64,
                            _ => (-f64::from_bits(x)).to_bits(),
                        },
                    };
                    self.put(base, dst, v)
                }
                &Op::Icmp { cc, ty, dst, a, b } => {
                    let v = icmp(cc, ty, self.get(base, a), self.get(base, b));
                    self.put(base, dst, v as u64)
                }
                &Op::Fcmp { cc, ty, dst, a, b } => {
                    let v = fcmp(cc, ty, self.get(base, a), self.get(base, b));
                    self.put(base, dst, v as u64)
                }
                &Op::Cast {
                    op,
                    from,
                    to,
                    dst,
                    a,
                } => {
                    let v = cast(op, from, to, self.get(base, a));
                    self.put(base, dst, v)
                }
                &Op::Select { dst, c, a, b } => {
                    let v = if self.get(base, c) != 0 {
                        self.get(base, a)
                    } else {
                        self.get(base, b)
                    };
                    self.put(base, dst, v)
                }
                &Op::Load { ty, dst, p } => {
                    // SAFETY: memory safety of IR programs is the program's responsibility, as in C.
                    let v = unsafe { load(ty, self.get(base, p)) };
                    self.put(base, dst, v)
                }
                &Op::Store { ty, v, p } => unsafe {
                    store(ty, self.get(base, p), self.get(base, v))
                },
                &Op::Ptradd { dst, p, off } => {
                    let v = self.get(base, p).wrapping_add(self.get(base, off));
                    self.put(base, dst, v)
                }
                Op::Call {
                    func,
                    args,
                    dst,
                    loc,
                } => {
                    let v = self.call_func(*func, base, args, *loc)?;
                    if let Some(d) = *dst {
                        self.put(base, d, v)
                    }
                }
                Op::CallIndirect {
                    sig,
                    callee,
                    args,
                    dst,
                    loc,
                } => {
                    let target = self.get(base, *callee);
                    let v = self.call_indirect(sig, target, base, args, *loc)?;
                    if let Some(d) = *dst {
                        self.put(base, d, v)
                    }
                }
                Op::If { c, then, els } => {
                    let flow = if self.get(base, *c) != 0 {
                        self.exec(then, base)?
                    } else {
                        self.exec(els, base)?
                    };
                    if !matches!(flow, Flow::Next) {
                        return Ok(flow);
                    }
                }
                Op::Loop { body } => loop {
                    match self.exec(body, base)? {
                        Flow::Next | Flow::Continue(0) => {}
                        Flow::Break(0) => break,
                        Flow::Break(n) => return Ok(Flow::Break(n - 1)),
                        Flow::Continue(n) => return Ok(Flow::Continue(n - 1)),
                        ret @ Flow::Return(_) => return Ok(ret),
                    }
                },
                &Op::Break(depth) => return Ok(Flow::Break(depth)),
                &Op::Continue(depth) => return Ok(Flow::Continue(depth)),
                &Op::Return(v) => {
                    return Ok(Flow::Return(v.map(|s| self.get(base, s)).unwrap_or(0)));
                }
                &Op::Unreachable(loc) => return Err(Trap::new(trap::UNREACHABLE).at(loc)),
                &Op::Line(loc) => {
                    if let Some(hook) = self.hook {
                        if let Some(frame) = self.frames.last_mut() {
                            frame.loc = loc;
                        }
                        hook(self, DebugEvent::Line);
                    }
                }
            }
        }
        Ok(Flow::Next)
    }
}

// ------------------------------------------------------------------------------------------ compile

struct FuncCompiler<'a> {
    m: &'a Module,
    slots: HashMap<Value, Slot>,
    /// The offset of each `stack_slot` in the frame's slot memory.
    stack_slots: HashMap<Inst, u32>,
    vars: HashMap<a0_ir::Var, Slot>,
    addr_of: &'a dyn Fn(a0_ir::Symbol) -> u64,
    locations: &'a a0_ir::Locations,
    /// Whether to mark new source lines for a debugger.
    lines: bool,
}

fn compile_function(
    m: &Module,
    f: FuncId,
    addr_of: &dyn Fn(a0_ir::Symbol) -> u64,
    locations: &a0_ir::Locations,
    lines: bool,
) -> Code {
    let mut c = FuncCompiler {
        m,
        slots: HashMap::new(),
        stack_slots: HashMap::new(),
        vars: HashMap::new(),
        addr_of,
        locations,
        lines,
    };
    let mut next = 0u32;
    let mut params = Vec::new();
    for &p in m.func_param_values(f) {
        c.slots.insert(p, next);
        params.push(next);
        next += 1;
    }
    let body = m.func_body(f).unwrap();
    let mut slot_bytes = 0u32;
    for inst in a0_ir::insts_in_text_order(m, body) {
        if let Some(v) = m.inst_result(inst) {
            c.slots.insert(v, next);
            next += 1;
        }
        if m.inst_opcode(inst) == Opcode::StackSlot {
            let (size, align) = m.inst_stack_slot(inst);
            let offset = slot_bytes.div_ceil(align.max(1)) * align.max(1);
            c.stack_slots.insert(inst, offset);
            slot_bytes = offset + size;
        }
    }
    for &var in m.func_vars(f) {
        c.vars.insert(var, next);
        next += 1;
    }
    let ops = c.region(body);
    let vars = m.func_vars(f).iter().map(|v| (*v, c.vars[v])).collect();
    Code {
        ops,
        params,
        frame_size: next as usize,
        slot_bytes: slot_bytes as usize,
        vars,
    }
}

impl FuncCompiler<'_> {
    fn s(&self, v: Value) -> Slot {
        self.slots[&v]
    }

    fn region(&self, r: Region) -> Box<[Op]> {
        let insts = self.m.region_insts(r);
        if !self.lines {
            return insts.iter().map(|&i| self.inst(i)).collect();
        }
        // A debugger hears of each new line: where a region starts (so every loop iteration),
        // and where an instruction is on another line than the one before it.
        let mut ops = Vec::with_capacity(insts.len() + 1);
        let mut last = None;
        for &i in insts {
            let id = self.locations.of(i);
            if let Some(l) = self.locations.get(id)
                && l.line != 0
            {
                let key = (l.file.as_str(), l.line, l.parent);
                if last != Some(key) {
                    ops.push(Op::Line(id));
                    last = Some(key);
                }
            }
            ops.push(self.inst(i));
        }
        ops.into_boxed_slice()
    }

    fn inst(&self, i: Inst) -> Op {
        let m = self.m;
        let op = m.inst_opcode(i);
        let args = m.inst_args(i);
        let a = |n: usize| self.s(args[n]);
        let dst = m.inst_result(i).map(|v| self.s(v));
        let ty = |n: usize| m.value_type(args[n]);
        match op {
            Opcode::Iconst => Op::Const {
                dst: dst.unwrap(),
                bits: m.inst_iconst(i) as u64,
            },
            Opcode::Fconst => Op::Const {
                dst: dst.unwrap(),
                bits: m.inst_fconst_bits(i),
            },
            Opcode::Null => Op::Const {
                dst: dst.unwrap(),
                bits: 0,
            },
            Opcode::StackSlot => Op::StackSlot {
                dst: dst.unwrap(),
                offset: self.stack_slots[&i],
            },
            Opcode::Addr => Op::Const {
                dst: dst.unwrap(),
                bits: (self.addr_of)(m.inst_symbol(i)),
            },
            op if op.is_int_binary() => Op::IntBin {
                op,
                ty: ty(0),
                dst: dst.unwrap(),
                a: a(0),
                b: a(1),
                loc: self.locations.of(i),
            },
            op if op.is_float_binary() => Op::FloatBin {
                op,
                ty: ty(0),
                dst: dst.unwrap(),
                a: a(0),
                b: a(1),
            },
            op if op.is_unary() => Op::Unary {
                op,
                ty: ty(0),
                dst: dst.unwrap(),
                a: a(0),
            },
            Opcode::Icmp => Op::Icmp {
                cc: m.inst_int_cc(i),
                ty: ty(0),
                dst: dst.unwrap(),
                a: a(0),
                b: a(1),
            },
            Opcode::Fcmp => Op::Fcmp {
                cc: m.inst_float_cc(i),
                ty: ty(0),
                dst: dst.unwrap(),
                a: a(0),
                b: a(1),
            },
            op if op.is_cast() => Op::Cast {
                op,
                from: ty(0),
                to: m.value_type(m.inst_result(i).unwrap()),
                dst: dst.unwrap(),
                a: a(0),
            },
            Opcode::Select => Op::Select {
                dst: dst.unwrap(),
                c: a(0),
                a: a(1),
                b: a(2),
            },
            Opcode::Load => Op::Load {
                ty: m.value_type(m.inst_result(i).unwrap()),
                dst: dst.unwrap(),
                p: a(0),
            },
            Opcode::Store => Op::Store {
                ty: ty(0),
                v: a(0),
                p: a(1),
            },
            Opcode::Ptradd => Op::Ptradd {
                dst: dst.unwrap(),
                p: a(0),
                off: a(1),
            },
            Opcode::Call => Op::Call {
                func: m.inst_callee(i).0,
                args: args.iter().map(|&v| self.s(v)).collect(),
                dst,
                loc: self.locations.of(i),
            },
            Opcode::CallIndirect => {
                let sig = m.inst_sig(i);
                Op::CallIndirect {
                    sig: Rc::new(Shape {
                        params: m.sig_params(sig).to_vec(),
                        ret: m.sig_ret(sig),
                    }),
                    callee: a(0),
                    args: args[1..].iter().map(|&v| self.s(v)).collect(),
                    dst,
                    loc: self.locations.of(i),
                }
            }
            Opcode::Get => Op::Copy {
                dst: dst.unwrap(),
                src: self.vars[&m.inst_var(i)],
            },
            Opcode::Set => Op::Copy {
                dst: self.vars[&m.inst_var(i)],
                src: a(0),
            },
            Opcode::If => Op::If {
                c: a(0),
                then: self.region(m.inst_region(i, 0).unwrap()),
                els: m
                    .inst_region(i, 1)
                    .map(|r| self.region(r))
                    .unwrap_or_default(),
            },
            Opcode::Loop => Op::Loop {
                body: self.region(m.inst_region(i, 0).unwrap()),
            },
            Opcode::Break => Op::Break(m.inst_loop_depth(i)),
            Opcode::Continue => Op::Continue(m.inst_loop_depth(i)),
            Opcode::Return => Op::Return(args.first().map(|&v| self.s(v))),
            Opcode::Unreachable => Op::Unreachable(self.locations.of(i)),
            _ => unreachable!("verified IR has no other opcodes"),
        }
    }
}

// ------------------------------------------------------------------------------------------ semantics

/// Bit mask of an integer type's width.
fn mask(ty: Type) -> u64 {
    match ty.bits() {
        Some(64) | None => u64::MAX,
        Some(b) => (1u64 << b) - 1,
    }
}

/// Canonical form of an integer value: sign-extended from its width (`i1` is 0 or 1).
fn canon(ty: Type, v: u64) -> u64 {
    a0_ir::canonical_int(ty, v as i64) as u64
}

/// Normalizes a raw 64-bit value returned from C to the canonical form of `ty`.
fn normalize(ty: Type, v: u64) -> u64 {
    match ty {
        Type::F64 | Type::Ptr => v,
        Type::F32 => v & 0xffff_ffff,
        _ => canon(ty, v),
    }
}

fn signed_min(ty: Type) -> i64 {
    match ty.bits() {
        Some(b) if b < 64 => -(1i64 << (b - 1)),
        _ => i64::MIN,
    }
}

fn int_binary(op: Opcode, ty: Type, a: u64, b: u64) -> Result<u64, Trap> {
    let (sa, sb) = (a as i64, b as i64);
    let (ua, ub) = (a & mask(ty), b & mask(ty));
    let bits = ty.bits().unwrap_or(64);
    let shift = (ub % bits as u64) as u32;
    let r = match op {
        Opcode::Iadd => sa.wrapping_add(sb) as u64,
        Opcode::Isub => sa.wrapping_sub(sb) as u64,
        Opcode::Imul => sa.wrapping_mul(sb) as u64,
        Opcode::Sdiv | Opcode::Srem => {
            if sb == 0 {
                return Err(Trap::new(trap::DIVIDE_BY_ZERO));
            }
            if op == Opcode::Sdiv {
                if sa == signed_min(ty) && sb == -1 {
                    return Err(Trap::new(trap::INTEGER_OVERFLOW));
                }
                sa.wrapping_div(sb) as u64
            } else {
                sa.wrapping_rem(sb) as u64
            }
        }
        Opcode::Udiv | Opcode::Urem => {
            if ub == 0 {
                return Err(Trap::new(trap::DIVIDE_BY_ZERO));
            }
            if op == Opcode::Udiv { ua / ub } else { ua % ub }
        }
        Opcode::And => a & b,
        Opcode::Or => a | b,
        Opcode::Xor => a ^ b,
        Opcode::Shl => ua.wrapping_shl(shift),
        Opcode::Lshr => ua.wrapping_shr(shift),
        Opcode::Ashr => sa.wrapping_shr(shift) as u64,
        _ => unreachable!(),
    };
    Ok(canon(ty, r))
}

fn float_binary(op: Opcode, ty: Type, a: u64, b: u64) -> u64 {
    if ty == Type::F32 {
        let (x, y) = (f32::from_bits(a as u32), f32::from_bits(b as u32));
        let r = match op {
            Opcode::Fadd => x + y,
            Opcode::Fsub => x - y,
            Opcode::Fmul => x * y,
            _ => x / y,
        };
        r.to_bits() as u64
    } else {
        let (x, y) = (f64::from_bits(a), f64::from_bits(b));
        let r = match op {
            Opcode::Fadd => x + y,
            Opcode::Fsub => x - y,
            Opcode::Fmul => x * y,
            _ => x / y,
        };
        r.to_bits()
    }
}

fn icmp(cc: IntCC, ty: Type, a: u64, b: u64) -> bool {
    let (sa, sb) = (a as i64, b as i64);
    let (ua, ub) = (a & mask(ty), b & mask(ty));
    match cc {
        IntCC::Eq => ua == ub,
        IntCC::Ne => ua != ub,
        IntCC::Slt => sa < sb,
        IntCC::Sle => sa <= sb,
        IntCC::Sgt => sa > sb,
        IntCC::Sge => sa >= sb,
        IntCC::Ult => ua < ub,
        IntCC::Ule => ua <= ub,
        IntCC::Ugt => ua > ub,
        IntCC::Uge => ua >= ub,
    }
}

fn fcmp(cc: FloatCC, ty: Type, a: u64, b: u64) -> bool {
    let (x, y) = if ty == Type::F32 {
        (
            f32::from_bits(a as u32) as f64,
            f32::from_bits(b as u32) as f64,
        )
    } else {
        (f64::from_bits(a), f64::from_bits(b))
    };
    match cc {
        FloatCC::Eq => x == y,
        FloatCC::Ne => x != y,
        FloatCC::Lt => x < y,
        FloatCC::Le => x <= y,
        FloatCC::Gt => x > y,
        FloatCC::Ge => x >= y,
    }
}

fn float_of(ty: Type, bits: u64) -> f64 {
    if ty == Type::F32 {
        f32::from_bits(bits as u32) as f64
    } else {
        f64::from_bits(bits)
    }
}

fn float_bits(ty: Type, v: f64) -> u64 {
    if ty == Type::F32 {
        (v as f32).to_bits() as u64
    } else {
        v.to_bits()
    }
}

/// Saturating float-to-int conversion; NaN becomes 0.
fn float_to_int(ty: Type, v: f64, signed: bool) -> u64 {
    let r = match (ty, signed) {
        (Type::I1, _) => (v as i64 != 0) as u64,
        (Type::I8, true) => v as i8 as i64 as u64,
        (Type::I16, true) => v as i16 as i64 as u64,
        (Type::I32, true) => v as i32 as i64 as u64,
        (_, true) => v as i64 as u64,
        (Type::I8, false) => v as u8 as u64,
        (Type::I16, false) => v as u16 as u64,
        (Type::I32, false) => v as u32 as u64,
        (_, false) => v as u64,
    };
    canon(ty, r)
}

fn cast(op: Opcode, from: Type, to: Type, a: u64) -> u64 {
    match op {
        Opcode::Zext => canon(to, a & mask(from)),
        Opcode::Sext => {
            if from == Type::I1 {
                canon(to, (a & 1).wrapping_neg())
            } else {
                canon(to, a)
            }
        }
        Opcode::Trunc => canon(to, a),
        Opcode::Sitofp => float_bits(to, a as i64 as f64),
        Opcode::Uitofp => float_bits(to, (a & mask(from)) as f64),
        Opcode::Fptosi => float_to_int(to, float_of(from, a), true),
        Opcode::Fptoui => float_to_int(to, float_of(from, a), false),
        Opcode::Fext => (f32::from_bits(a as u32) as f64).to_bits(),
        Opcode::Ftrunc => (f64::from_bits(a) as f32).to_bits() as u64,
        Opcode::Bitcast => match to {
            Type::F32 => a & 0xffff_ffff,
            Type::I32 => canon(Type::I32, a),
            _ => a,
        },
        Opcode::Ptrtoint => canon(to, a),
        Opcode::Inttoptr => a & mask(from),
        _ => unreachable!(),
    }
}

/// Reads a value of type `ty` at `addr` and returns it in canonical form.
///
/// # Safety
/// `addr` must be valid for reading `ty`.
unsafe fn load(ty: Type, addr: u64) -> u64 {
    let p = addr as usize as *const u8;
    unsafe {
        match ty {
            Type::I1 => (p.read() & 1) as u64,
            Type::I8 => (p as *const i8).read_unaligned() as i64 as u64,
            Type::I16 => (p as *const i16).read_unaligned() as i64 as u64,
            Type::I32 => (p as *const i32).read_unaligned() as i64 as u64,
            Type::F32 => (p as *const u32).read_unaligned() as u64,
            Type::I64 | Type::F64 | Type::Ptr => (p as *const u64).read_unaligned(),
        }
    }
}

/// Writes a canonical value of type `ty` to `addr`.
///
/// # Safety
/// `addr` must be valid for writing `ty`.
unsafe fn store(ty: Type, addr: u64, v: u64) {
    let p = addr as usize as *mut u8;
    unsafe {
        match ty {
            Type::I1 | Type::I8 => p.write(v as u8),
            Type::I16 => (p as *mut u16).write_unaligned(v as u16),
            Type::I32 | Type::F32 => (p as *mut u32).write_unaligned(v as u32),
            Type::I64 | Type::F64 | Type::Ptr => (p as *mut u64).write_unaligned(v),
        }
    }
}

/// Runs `@main` of a module on a thread with a large stack and returns its exit status. This is
/// what command-line drivers use; traps are returned rather than reported.
pub fn run_main(
    module: Module,
    symbols: HostSymbols,
    args: Vec<String>,
) -> Result<Result<i32, Trap>, Diagnostics> {
    // WebAssembly has no threads: the interpreter runs on the main thread, whose stack the program
    // sets when it is linked (the playground's is large enough for `DEFAULT_MAX_DEPTH`).
    if cfg!(target_family = "wasm") {
        let mut interp = Interpreter::new(module, &symbols)?;
        return Ok(interp.run_main(&args));
    }
    let handle = std::thread::Builder::new()
        .name("a0-interp".to_string())
        .stack_size(1 << 30)
        .spawn(move || {
            let mut interp = Interpreter::new(module, &symbols)?;
            Ok(interp.run_main(&args))
        })
        .expect("failed to spawn the interpreter thread");
    handle.join().expect("the interpreter thread panicked")
}
