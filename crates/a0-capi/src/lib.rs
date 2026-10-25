//! A0's C API: build, run and compile modules from C. The header is `include/a0.h`; the conventions
//! are in `docs/c-api.md` and decision D2.
//!
//! Every function is a thin wrapper over the library crates: objects are boxed and handed to C as
//! opaque pointers, handles pass through as `u32`, strings come in as pointer and length and go out
//! borrowed from the object that owns them, and failures become [`Diagnostics`]. Misusing a builder
//! is recorded, never a crash; invalid pointers are undefined behavior, as in any C API.
//!
//! # Safety
//!
//! Every function has the same contract, stated once in `include/a0.h`: object pointers are NULL
//! or come from A0 and have not been freed, and a pointer with a length points at that many
//! readable elements (or is NULL with length 0).

#![allow(clippy::missing_safety_doc)]

mod hir;

pub use hir::{A0HirBuilder, A0Str};

use std::borrow::Cow;
use std::ffi::{c_char, c_int};
use std::path::Path;
use std::ptr;
use std::slice;

use a0_diag::{Diagnostic, Diagnostics, SourceId};
use a0_ir::{Builder, FloatCC, FuncId, GlobalId, IntCC, Module, Opcode, Type, Value, Var};

pub struct A0Diagnostics {
    diags: Diagnostics,
    text: String,
    /// The last `a0_diagnostics_render`.
    rendered: String,
}

pub struct A0Builder {
    b: Builder,
    /// Mistakes the C layer catches before the builder sees them (unknown names, bad type codes).
    errors: Diagnostics,
}

pub struct A0Module {
    module: Module,
    text: String,
}

pub struct A0Grammar(a0_syntax::Grammar);

pub struct A0Tree(a0_syntax::SyntaxTree);

/// A HIR module and its text, kept for `a0_hir_text`.
pub struct A0Hir(a0_hir::HirModule, String);

const NONE: u32 = u32::MAX;
const SOURCE: SourceId = SourceId(0);

/// A string argument. Invalid UTF-8 is replaced rather than rejected.
unsafe fn text<'a>(p: *const c_char, len: usize) -> Cow<'a, str> {
    if p.is_null() || len == 0 {
        return Cow::Borrowed("");
    }
    String::from_utf8_lossy(unsafe { slice::from_raw_parts(p.cast::<u8>(), len) })
}

/// An array argument.
unsafe fn items<'a, T>(p: *const T, n: usize) -> &'a [T] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(p, n) }
    }
}

/// Returns a borrowed string to C.
unsafe fn give_str(s: &str, len: *mut usize) -> *const c_char {
    if !len.is_null() {
        unsafe { *len = s.len() };
    }
    s.as_ptr().cast()
}

unsafe fn give<T>(value: T) -> *mut T {
    Box::into_raw(Box::new(value))
}

unsafe fn free<T>(p: *mut T) {
    if !p.is_null() {
        drop(unsafe { Box::from_raw(p) });
    }
}

/// Sets `*out` to the diagnostics, or to NULL when there are none.
unsafe fn report(out: *mut *mut A0Diagnostics, diags: Option<Diagnostics>) {
    if out.is_null() {
        return;
    }
    let value = match diags {
        Some(diags) => unsafe {
            give(A0Diagnostics {
                text: diags.summary(),
                diags,
                rendered: String::new(),
            })
        },
        None => ptr::null_mut(),
    };
    unsafe { *out = value };
}

fn error(message: &str) -> Diagnostics {
    Diagnostics::from(Diagnostic::error("capi", message))
}

/// A type code from `A0Type`, or `None` for `A0_VOID`. Unknown codes are `Err`.
fn type_code(code: u32) -> Result<Option<Type>, String> {
    match code {
        0 => Ok(None),
        c => Type::ALL
            .get(c as usize - 1)
            .copied()
            .map(Some)
            .ok_or_else(|| format!("{c} is not an A0Type")),
    }
}

impl A0Builder {
    fn fail(&mut self, message: String) {
        self.errors.push(Diagnostic::error("capi", &message));
    }

    fn ty(&mut self, code: u32) -> Option<Type> {
        match type_code(code) {
            Ok(t) => t,
            Err(e) => {
                self.fail(e);
                None
            }
        }
    }

    /// A value type: a type code other than `A0_VOID`.
    fn value_ty(&mut self, code: u32) -> Type {
        self.ty(code).unwrap_or_else(|| {
            if code == 0 {
                self.fail("A0_VOID is not a value type".to_string());
            }
            Type::I64
        })
    }

    fn types(&mut self, codes: &[u32]) -> Vec<Type> {
        codes.iter().map(|&c| self.value_ty(c)).collect()
    }

    fn opcode(&mut self, name: &str) -> Option<Opcode> {
        let op = Opcode::from_name(name);
        if op.is_none() {
            self.fail(format!("'{name}' is not an instruction"));
        }
        op
    }
}

macro_rules! builder {
    ($b:expr, $none:expr) => {
        match unsafe { $b.as_mut() } {
            Some(b) => b,
            None => return $none,
        }
    };
    ($b:expr) => {
        match unsafe { $b.as_mut() } {
            Some(b) => b,
            None => return,
        }
    };
}

#[unsafe(no_mangle)]
pub extern "C" fn a0_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

// ------------------------------------------------------------------------------------ diagnostics

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_diagnostics_count(d: *const A0Diagnostics) -> usize {
    unsafe { d.as_ref() }.map_or(0, |d| d.diags.len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_diagnostics_message(
    d: *const A0Diagnostics,
    i: usize,
    len: *mut usize,
) -> *const c_char {
    match unsafe { d.as_ref() } {
        Some(d) if i < d.diags.len() => unsafe { give_str(&d.diags.get(i).message, len) },
        _ => unsafe { give_str("", len) },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_diagnostics_code(
    d: *const A0Diagnostics,
    i: usize,
    len: *mut usize,
) -> *const c_char {
    match unsafe { d.as_ref() } {
        Some(d) if i < d.diags.len() => unsafe { give_str(&d.diags.get(i).code, len) },
        _ => unsafe { give_str("", len) },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_diagnostics_span(
    d: *const A0Diagnostics,
    i: usize,
    start: *mut u32,
    end: *mut u32,
) -> c_int {
    let span = match unsafe { d.as_ref() } {
        Some(d) if i < d.diags.len() => d.diags.get(i).span,
        _ => None,
    };
    let Some(span) = span else { return 0 };
    unsafe {
        if !start.is_null() {
            *start = span.start;
        }
        if !end.is_null() {
            *end = span.end;
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_diagnostics_text(
    d: *const A0Diagnostics,
    len: *mut usize,
) -> *const c_char {
    let text = unsafe { d.as_ref() }.map_or("", |d| d.text.as_str());
    unsafe { give_str(text, len) }
}

/// Renders the diagnostics against the text their spans refer to, the way the `a0` tool does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_diagnostics_render(
    d: *mut A0Diagnostics,
    name: *const c_char,
    name_len: usize,
    source: *const c_char,
    source_len: usize,
    len: *mut usize,
) -> *const c_char {
    let Some(d) = (unsafe { d.as_mut() }) else {
        return unsafe { give_str("", len) };
    };
    let mut sources = a0_diag::SourceMap::new();
    let id = sources.add(&unsafe { text(name, name_len) }, &unsafe {
        text(source, source_len)
    });
    debug_assert_eq!(id, SOURCE);
    d.rendered = d.diags.render(&sources);
    unsafe { give_str(&d.rendered, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_diagnostics_free(d: *mut A0Diagnostics) {
    unsafe { free(d) }
}

// ------------------------------------------------------------------------------------ builder

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_new(name: *const c_char, len: usize) -> *mut A0Builder {
    let name = unsafe { text(name, len) };
    unsafe {
        give(A0Builder {
            b: Builder::new(&name),
            errors: Diagnostics::new(),
        })
    }
}

unsafe fn declare(
    b: *mut A0Builder,
    name: *const c_char,
    len: usize,
    params: *const u32,
    count: usize,
    ret: u32,
    external: bool,
) -> u32 {
    let b = builder!(b, NONE);
    let name = unsafe { text(name, len) };
    let params = b.types(unsafe { items(params, count) });
    let ret = b.ty(ret);
    let f = if external {
        b.b.declare_extern(&name, &params, ret)
    } else {
        b.b.declare_function(&name, &params, ret)
    };
    f.0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_declare_extern(
    b: *mut A0Builder,
    name: *const c_char,
    len: usize,
    params: *const u32,
    count: usize,
    ret: u32,
) -> u32 {
    unsafe { declare(b, name, len, params, count, ret, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_declare_function(
    b: *mut A0Builder,
    name: *const c_char,
    len: usize,
    params: *const u32,
    count: usize,
    ret: u32,
) -> u32 {
    unsafe { declare(b, name, len, params, count, ret, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_add_data(
    b: *mut A0Builder,
    name: *const c_char,
    len: usize,
    bytes: *const u8,
    count: usize,
) -> u32 {
    let b = builder!(b, NONE);
    let name = unsafe { text(name, len) };
    b.b.add_data(&name, unsafe { items(bytes, count) }).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_add_global_int(
    b: *mut A0Builder,
    name: *const c_char,
    len: usize,
    ty: u32,
    init: i64,
) -> u32 {
    let b = builder!(b, NONE);
    let name = unsafe { text(name, len) };
    let ty = b.value_ty(ty);
    b.b.add_global_int(&name, ty, init).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_add_global_float(
    b: *mut A0Builder,
    name: *const c_char,
    len: usize,
    ty: u32,
    init: f64,
) -> u32 {
    let b = builder!(b, NONE);
    let name = unsafe { text(name, len) };
    let ty = b.value_ty(ty);
    b.b.add_global_float(&name, ty, init).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_begin_function(b: *mut A0Builder, f: u32) {
    builder!(b).b.begin_function(FuncId(f))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_end_function(b: *mut A0Builder) {
    builder!(b).b.end_function()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_param(b: *mut A0Builder, i: u32) -> u32 {
    builder!(b, NONE).b.param(i).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_declare_var(
    b: *mut A0Builder,
    name: *const c_char,
    len: usize,
    ty: u32,
) -> u32 {
    let b = builder!(b, NONE);
    let name = unsafe { text(name, len) };
    let ty = b.value_ty(ty);
    b.b.declare_var(&name, ty).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_get(b: *mut A0Builder, var: u32) -> u32 {
    builder!(b, NONE).b.get(Var(var)).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_set(b: *mut A0Builder, var: u32, value: u32) {
    builder!(b).b.set(Var(var), Value(value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_iconst(b: *mut A0Builder, ty: u32, value: i64) -> u32 {
    let b = builder!(b, NONE);
    let ty = b.value_ty(ty);
    b.b.iconst(ty, value).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_fconst(b: *mut A0Builder, ty: u32, value: f64) -> u32 {
    let b = builder!(b, NONE);
    let ty = b.value_ty(ty);
    b.b.fconst(ty, value).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_null(b: *mut A0Builder) -> u32 {
    builder!(b, NONE).b.null().0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_func_addr(b: *mut A0Builder, f: u32) -> u32 {
    builder!(b, NONE).b.func_addr(FuncId(f)).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_global_addr(b: *mut A0Builder, g: u32) -> u32 {
    builder!(b, NONE).b.global_addr(GlobalId(g)).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_binary(
    b: *mut A0Builder,
    op: *const c_char,
    len: usize,
    x: u32,
    y: u32,
) -> u32 {
    let b = builder!(b, NONE);
    match b.opcode(&unsafe { text(op, len) }) {
        Some(op) => b.b.binary(op, Value(x), Value(y)).0,
        None => NONE,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_unary(
    b: *mut A0Builder,
    op: *const c_char,
    len: usize,
    x: u32,
) -> u32 {
    let b = builder!(b, NONE);
    match b.opcode(&unsafe { text(op, len) }) {
        Some(op) => b.b.unary(op, Value(x)).0,
        None => NONE,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_cast(
    b: *mut A0Builder,
    op: *const c_char,
    len: usize,
    x: u32,
    to: u32,
) -> u32 {
    let b = builder!(b, NONE);
    let to = b.value_ty(to);
    match b.opcode(&unsafe { text(op, len) }) {
        Some(op) => b.b.cast(op, Value(x), to).0,
        None => NONE,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_icmp(
    b: *mut A0Builder,
    cond: *const c_char,
    len: usize,
    x: u32,
    y: u32,
) -> u32 {
    let b = builder!(b, NONE);
    let name = unsafe { text(cond, len) };
    match IntCC::from_name(&name) {
        Some(cc) => b.b.icmp(cc, Value(x), Value(y)).0,
        None => {
            b.fail(format!("'{name}' is not an icmp condition"));
            NONE
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_fcmp(
    b: *mut A0Builder,
    cond: *const c_char,
    len: usize,
    x: u32,
    y: u32,
) -> u32 {
    let b = builder!(b, NONE);
    let name = unsafe { text(cond, len) };
    match FloatCC::from_name(&name) {
        Some(cc) => b.b.fcmp(cc, Value(x), Value(y)).0,
        None => {
            b.fail(format!("'{name}' is not an fcmp condition"));
            NONE
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_select(b: *mut A0Builder, c: u32, x: u32, y: u32) -> u32 {
    builder!(b, NONE).b.select(Value(c), Value(x), Value(y)).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_load(b: *mut A0Builder, ty: u32, ptr: u32) -> u32 {
    let b = builder!(b, NONE);
    let ty = b.value_ty(ty);
    b.b.load(ty, Value(ptr)).0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_store(b: *mut A0Builder, value: u32, ptr: u32) {
    builder!(b).b.store(Value(value), Value(ptr))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_ptradd(b: *mut A0Builder, ptr: u32, offset: u32) -> u32 {
    builder!(b, NONE).b.ptradd(Value(ptr), Value(offset)).0
}

fn values(handles: &[u32]) -> Vec<Value> {
    handles.iter().map(|&v| Value(v)).collect()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_call(
    b: *mut A0Builder,
    f: u32,
    args: *const u32,
    count: usize,
) -> u32 {
    let b = builder!(b, NONE);
    let args = values(unsafe { items(args, count) });
    b.b.call(FuncId(f), &args).map_or(NONE, |v| v.0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_call_indirect(
    b: *mut A0Builder,
    callee: u32,
    params: *const u32,
    param_count: usize,
    ret: u32,
    args: *const u32,
    arg_count: usize,
) -> u32 {
    let b = builder!(b, NONE);
    let params = b.types(unsafe { items(params, param_count) });
    let ret = b.ty(ret);
    let args = values(unsafe { items(args, arg_count) });
    b.b.call_indirect(Value(callee), &params, ret, &args)
        .map_or(NONE, |v| v.0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_if(b: *mut A0Builder, cond: u32) {
    builder!(b).b.if_(Value(cond))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_else(b: *mut A0Builder) {
    builder!(b).b.else_()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_end_if(b: *mut A0Builder) {
    builder!(b).b.end_if()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_loop(b: *mut A0Builder) {
    builder!(b).b.loop_()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_end_loop(b: *mut A0Builder) {
    builder!(b).b.end_loop()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_break(b: *mut A0Builder) {
    builder!(b).b.break_()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_continue(b: *mut A0Builder) {
    builder!(b).b.continue_()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_break_out(b: *mut A0Builder, depth: u32) {
    builder!(b).b.break_out(depth)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_continue_out(b: *mut A0Builder, depth: u32) {
    builder!(b).b.continue_out(depth)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_return(b: *mut A0Builder, value: u32) {
    let value = (value != NONE).then_some(Value(value));
    builder!(b).b.ret(value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_unreachable(b: *mut A0Builder) {
    builder!(b).b.unreachable()
}

fn module(module: Module) -> A0Module {
    A0Module {
        module,
        text: String::new(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_builder_finish(
    b: *mut A0Builder,
    diags: *mut *mut A0Diagnostics,
) -> *mut A0Module {
    if b.is_null() {
        unsafe { report(diags, Some(error("a0_builder_finish: no builder"))) };
        return ptr::null_mut();
    }
    let A0Builder { b, mut errors } = *unsafe { Box::from_raw(b) };
    let result = if errors.has_errors() {
        // Report the C layer's mistakes together with the builder's and the verifier's.
        let (m, more) = b.finish_unverified();
        errors.extend(more);
        errors.extend(a0_ir::verify(&m));
        Err(errors)
    } else {
        b.finish()
    };
    match result {
        Ok(m) => {
            unsafe { report(diags, None) };
            unsafe { give(module(m)) }
        }
        Err(d) => {
            unsafe { report(diags, Some(d)) };
            ptr::null_mut()
        }
    }
}

// ------------------------------------------------------------------------------------ modules

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_module_parse(
    src: *const c_char,
    len: usize,
    diags: *mut *mut A0Diagnostics,
) -> *mut A0Module {
    let src = unsafe { text(src, len) };
    match a0_ir::parse_and_verify(&src, SOURCE) {
        Ok(m) => {
            unsafe { report(diags, None) };
            unsafe { give(module(m)) }
        }
        Err(d) => {
            unsafe { report(diags, Some(d)) };
            ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_module_text(m: *mut A0Module, len: *mut usize) -> *const c_char {
    let Some(m) = (unsafe { m.as_mut() }) else {
        return unsafe { give_str("", len) };
    };
    m.text = m.module.to_string();
    unsafe { give_str(&m.text, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_module_free(m: *mut A0Module) {
    unsafe { free(m) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_run(
    m: *const A0Module,
    args: *const *const c_char,
    count: usize,
    status: *mut i32,
    diags: *mut *mut A0Diagnostics,
) -> c_int {
    let Some(m) = (unsafe { m.as_ref() }) else {
        unsafe { report(diags, Some(error("a0_run: no module"))) };
        return -1;
    };
    let args: Vec<String> = unsafe { items(args, count) }
        .iter()
        .map(|&a| {
            if a.is_null() {
                String::new()
            } else {
                unsafe { std::ffi::CStr::from_ptr(a) }
                    .to_string_lossy()
                    .into_owned()
            }
        })
        .collect();
    let symbols = a0_interp::HostSymbols::new();
    let outcome = a0_interp::run_main(m.module.clone(), symbols, args);
    a0_runtime::flush();
    let set = |code: i32| {
        if !status.is_null() {
            unsafe { *status = code };
        }
    };
    match outcome {
        Ok(Ok(code)) => {
            set(code);
            unsafe { report(diags, None) };
            0
        }
        Ok(Err(trap)) => {
            set(a0_runtime::PANIC_EXIT_CODE);
            // What a program prints when it fails: the message, then its stack trace.
            let message = format!("panic: {}\n{}", trap.message, trap.trace());
            unsafe { report(diags, Some(error(message.trim_end()))) };
            1
        }
        Err(d) => {
            unsafe { report(diags, Some(d)) };
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_write_object(
    m: *const A0Module,
    target: *const c_char,
    target_len: usize,
    path: *const c_char,
    path_len: usize,
    diags: *mut *mut A0Diagnostics,
) -> c_int {
    let Some(m) = (unsafe { m.as_ref() }) else {
        unsafe { report(diags, Some(error("a0_write_object: no module"))) };
        return -1;
    };
    let target = unsafe { text(target, target_len) };
    let target = if target.is_empty() {
        a0_native::Target::host().ok_or_else(|| "this host is not a supported target".to_string())
    } else {
        a0_native::Target::parse(&target)
    };
    let path = unsafe { text(path, path_len) };
    let result = target.map_err(|e| error(&e)).and_then(|target| {
        let opts = a0_native::CodegenOptions {
            optimize: true,
            entry_point: false,
            debug_info: false,
        };
        let bytes = a0_native::compile_object(&m.module, target, &opts)?;
        std::fs::write(Path::new(path.as_ref()), bytes)
            .map_err(|e| error(&format!("cannot write {path}: {e}")))
    });
    match result {
        Ok(()) => {
            unsafe { report(diags, None) };
            0
        }
        Err(d) => {
            unsafe { report(diags, Some(d)) };
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_build_executable(
    m: *const A0Module,
    path: *const c_char,
    path_len: usize,
    diags: *mut *mut A0Diagnostics,
) -> c_int {
    let Some(m) = (unsafe { m.as_ref() }) else {
        unsafe { report(diags, Some(error("a0_build_executable: no module"))) };
        return -1;
    };
    let path = unsafe { text(path, path_len) };
    let result = match a0_native::Target::host() {
        Some(target) => a0_native::build_executable(
            &m.module,
            target,
            Path::new(path.as_ref()),
            &a0_native::BuildOptions::default(),
        ),
        None => Err(error("native code is not supported on this host")),
    };
    match result {
        Ok(()) => {
            unsafe { report(diags, None) };
            0
        }
        Err(d) => {
            unsafe { report(diags, Some(d)) };
            -1
        }
    }
}

// ------------------------------------------------------------------------------------ syntax

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_grammar_new(
    src: *const c_char,
    len: usize,
    diags: *mut *mut A0Diagnostics,
) -> *mut A0Grammar {
    let src = unsafe { text(src, len) };
    match a0_syntax::Grammar::from_str(&src, SOURCE) {
        Ok(g) => {
            unsafe { report(diags, None) };
            unsafe { give(A0Grammar(g)) }
        }
        Err(d) => {
            unsafe { report(diags, Some(d)) };
            ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_grammar_free(g: *mut A0Grammar) {
    unsafe { free(g) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_grammar_parse(
    g: *const A0Grammar,
    src: *const c_char,
    len: usize,
    diags: *mut *mut A0Diagnostics,
) -> *mut A0Tree {
    let Some(g) = (unsafe { g.as_ref() }) else {
        unsafe { report(diags, Some(error("a0_grammar_parse: no grammar"))) };
        return ptr::null_mut();
    };
    let src = unsafe { text(src, len) };
    let (tree, d) = g.0.parse(&src, SOURCE);
    unsafe { report(diags, (!d.is_empty()).then_some(d)) };
    unsafe { give(A0Tree(tree)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_free(t: *mut A0Tree) {
    unsafe { free(t) }
}

/// The tree and a node of it, if the node exists.
unsafe fn node<'a>(
    t: *const A0Tree,
    n: u32,
) -> Option<(&'a a0_syntax::SyntaxTree, a0_syntax::NodeId)> {
    let t = &unsafe { t.as_ref() }?.0;
    (n < t.node_count()).then_some((t, a0_syntax::NodeId(n)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_root(t: *const A0Tree) -> u32 {
    unsafe { t.as_ref() }.map_or(NONE, |t| t.0.root().0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_kind(t: *const A0Tree, n: u32, len: *mut usize) -> *const c_char {
    let kind = unsafe { node(t, n) }.map_or("", |(t, n)| t.kind_name(n));
    unsafe { give_str(kind, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_text(t: *const A0Tree, n: u32, len: *mut usize) -> *const c_char {
    let s = unsafe { node(t, n) }.map_or("", |(t, n)| t.text(n));
    unsafe { give_str(s, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_span(
    t: *const A0Tree,
    n: u32,
    start: *mut u32,
    end: *mut u32,
) -> c_int {
    let Some((t, n)) = (unsafe { node(t, n) }) else {
        return 0;
    };
    let span = t.span(n);
    unsafe {
        if !start.is_null() {
            *start = span.start;
        }
        if !end.is_null() {
            *end = span.end;
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_is_token(t: *const A0Tree, n: u32) -> c_int {
    unsafe { node(t, n) }.is_some_and(|(t, n)| t.is_token(n)) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_child_count(t: *const A0Tree, n: u32) -> u32 {
    unsafe { node(t, n) }.map_or(0, |(t, n)| t.child_count(n))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_child(t: *const A0Tree, n: u32, i: u32) -> u32 {
    match unsafe { node(t, n) } {
        Some((t, n)) if i < t.child_count(n) => t.child(n, i).0,
        _ => NONE,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_child_field(
    t: *const A0Tree,
    n: u32,
    i: u32,
    len: *mut usize,
) -> *const c_char {
    match unsafe { node(t, n) } {
        Some((t, n)) if i < t.child_count(n) => match t.child_field(n, i) {
            Some(f) => unsafe { give_str(t.field_name(f), len) },
            None => ptr::null(),
        },
        _ => ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_tree_field(
    t: *const A0Tree,
    n: u32,
    field: *const c_char,
    len: usize,
) -> u32 {
    let field = unsafe { text(field, len) };
    unsafe { node(t, n) }
        .and_then(|(t, n)| t.field_named(n, &field))
        .map_or(NONE, |c| c.0)
}

// ------------------------------------------------------------------------------------ HIR

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_parse(
    src: *const c_char,
    len: usize,
    diags: *mut *mut A0Diagnostics,
) -> *mut A0Hir {
    let src = unsafe { text(src, len) };
    match a0_hir::parse(&src, SOURCE) {
        Ok(h) => {
            unsafe { report(diags, None) };
            unsafe { give(A0Hir(h, String::new())) }
        }
        Err(d) => {
            unsafe { report(diags, Some(d)) };
            ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_lower(
    h: *const A0Hir,
    diags: *mut *mut A0Diagnostics,
) -> *mut A0Module {
    let Some(h) = (unsafe { h.as_ref() }) else {
        unsafe { report(diags, Some(error("a0_hir_lower: no module"))) };
        return ptr::null_mut();
    };
    match a0_hir::lower(&h.0) {
        Ok(m) => {
            unsafe { report(diags, None) };
            unsafe { give(module(m)) }
        }
        Err(d) => {
            unsafe { report(diags, Some(d)) };
            ptr::null_mut()
        }
    }
}

/// The module in the HIR text format (`docs/hir.md`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_text(h: *mut A0Hir, len: *mut usize) -> *const c_char {
    let Some(h) = (unsafe { h.as_mut() }) else {
        return unsafe { give_str("", len) };
    };
    h.1 = h.0.to_string();
    unsafe { give_str(&h.1, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_free(h: *mut A0Hir) {
    unsafe { free(h) }
}

/// Sets `*warnings` to the module's warnings, or to NULL if it has none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_warnings(h: *const A0Hir, warnings: *mut *mut A0Diagnostics) {
    let found = unsafe { h.as_ref() }.map(|h| h.0.warnings().clone());
    unsafe { report(warnings, found.filter(|w| !w.is_empty())) }
}
