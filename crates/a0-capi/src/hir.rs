//! The HIR builder for C (`docs/hir.md`, the "HIR builder" part of `include/a0.h`).
//!
//! The builder trusts the handles it is given, since in Rust they can only come from it; here every
//! handle is checked against the module first, and a bad one is reported by `a0_hir_finish` like
//! any other misuse. Operators, builtins and type tags are passed by name, an `Item` as a kind and a
//! handle, and lists of names as arrays of `A0Str`.

use std::ffi::{c_char, c_int};
use std::ptr;

use a0_diag::{Diagnostic, Diagnostics, Span};
use a0_hir::{
    ArmId, BinOp, BodyId, Builtin, EnumId, ExprId, ExternId, FuncId, GlobalId, HirBuilder,
    InterfaceId, Item, LocalId, StructId, Term, TypeId, TypeTag, UnOp, Warning,
};

use crate::{A0Diagnostics, A0Hir, NONE, SOURCE, error, free, give, items, report, text};

pub struct A0HirBuilder {
    b: HirBuilder,
    /// Mistakes the C layer catches before the builder sees them (bad handles, unknown names).
    errors: Diagnostics,
}

/// A string as pointer and length, for arrays of names.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct A0Str {
    ptr: *const c_char,
    len: usize,
}

const ITEM_STRUCT: u32 = 1;
const ITEM_ENUM: u32 = 2;
const ITEM_FUNC: u32 = 3;
const ITEM_GLOBAL: u32 = 4;
const ITEM_EXTERN: u32 = 5;
const ITEM_INTERFACE: u32 = 6;

/// An item as C sees it: a kind (`A0_HIR_STRUCT`, ...) and a handle.
fn split_item(item: Item) -> (u32, u32) {
    match item {
        Item::Struct(s) => (ITEM_STRUCT, s.0),
        Item::Enum(e) => (ITEM_ENUM, e.0),
        Item::Func(f) => (ITEM_FUNC, f.0),
        Item::Global(g) => (ITEM_GLOBAL, g.0),
        Item::Extern(x) => (ITEM_EXTERN, x.0),
        Item::Interface(i) => (ITEM_INTERFACE, i.0),
    }
}

unsafe fn names(p: *const A0Str, n: usize) -> Vec<String> {
    unsafe { items(p, n) }
        .iter()
        .map(|s| unsafe { text(s.ptr, s.len) }.into_owned())
        .collect()
}

fn refs(names: &[String]) -> Vec<&str> {
    names.iter().map(String::as_str).collect()
}

impl A0HirBuilder {
    fn fail(&mut self, message: String) {
        self.errors.push(Diagnostic::error("capi", &message));
    }

    fn check(&mut self, what: &str, id: u32, count: u32) -> Option<u32> {
        if id < count {
            Some(id)
        } else {
            self.fail(format!("{id} is not a valid {what} handle"));
            None
        }
    }

    fn e(&mut self, e: u32) -> Option<ExprId> {
        let n = self.b.module().expr_count();
        self.check("expression", e, n).map(ExprId)
    }

    /// An optional expression: `A0_NONE` is none, anything else must be valid.
    fn opt_e(&mut self, e: u32) -> Option<Option<ExprId>> {
        if e == NONE {
            Some(None)
        } else {
            self.e(e).map(Some)
        }
    }

    fn es(&mut self, es: &[u32]) -> Option<Vec<ExprId>> {
        es.iter().map(|&e| self.e(e)).collect()
    }

    fn t(&mut self, t: u32) -> Option<TypeId> {
        let n = self.b.module().type_count();
        self.check("type", t, n).map(TypeId)
    }

    fn ts(&mut self, ts: &[u32]) -> Option<Vec<TypeId>> {
        ts.iter().map(|&t| self.t(t)).collect()
    }

    fn l(&mut self, l: u32) -> Option<LocalId> {
        let n = self.b.module().local_count();
        self.check("local", l, n).map(LocalId)
    }

    fn s(&mut self, s: u32) -> Option<StructId> {
        let n = self.b.module().struct_count();
        self.check("struct", s, n).map(StructId)
    }

    fn en(&mut self, e: u32) -> Option<EnumId> {
        let n = self.b.module().enum_count();
        self.check("enum", e, n).map(EnumId)
    }

    fn i(&mut self, i: u32) -> Option<InterfaceId> {
        let n = self.b.module().interface_count();
        self.check("interface", i, n).map(InterfaceId)
    }

    fn f(&mut self, f: u32) -> Option<FuncId> {
        let n = self.b.module().func_count();
        self.check("function", f, n).map(FuncId)
    }

    fn g(&mut self, g: u32) -> Option<GlobalId> {
        let n = self.b.module().global_count();
        self.check("global", g, n).map(GlobalId)
    }

    fn x(&mut self, x: u32) -> Option<ExternId> {
        let n = self.b.module().extern_count();
        self.check("extern", x, n).map(ExternId)
    }

    fn body(&mut self, body: u32) -> Option<BodyId> {
        let n = self.b.module().body_count();
        self.check("body", body, n).map(BodyId)
    }

    fn arm(&mut self, a: u32) -> Option<ArmId> {
        let n = self.b.module().arm_count();
        self.check("match arm", a, n).map(ArmId)
    }

    /// Declaring a local by name needs an open body.
    fn needs_body(&mut self) -> Option<()> {
        if self.b.in_body() {
            Some(())
        } else {
            self.fail("no body is being built; call a0_hir_begin_body first".to_string());
            None
        }
    }

    fn item(&mut self, kind: u32, id: u32) -> Option<Item> {
        match kind {
            ITEM_STRUCT => self.s(id).map(Item::Struct),
            ITEM_ENUM => self.en(id).map(Item::Enum),
            ITEM_FUNC => self.f(id).map(Item::Func),
            ITEM_GLOBAL => self.g(id).map(Item::Global),
            ITEM_EXTERN => self.x(id).map(Item::Extern),
            ITEM_INTERFACE => self.i(id).map(Item::Interface),
            _ => {
                self.fail(format!("{kind} is not an item kind"));
                None
            }
        }
    }
}

/// Runs `f` on the builder; `A0_NONE` if there is no builder or `f` found a bad argument.
unsafe fn with(b: *mut A0HirBuilder, f: impl FnOnce(&mut A0HirBuilder) -> Option<u32>) -> u32 {
    match unsafe { b.as_mut() } {
        Some(h) => f(h).unwrap_or(NONE),
        None => NONE,
    }
}

unsafe fn with_unit(b: *mut A0HirBuilder, f: impl FnOnce(&mut A0HirBuilder) -> Option<()>) {
    if let Some(h) = unsafe { b.as_mut() } {
        f(h);
    }
}

// ---------------------------------------------------------------------------------- the builder

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_builder_new(name: *const c_char, len: usize) -> *mut A0HirBuilder {
    let name = unsafe { text(name, len) };
    unsafe {
        give(A0HirBuilder {
            b: HirBuilder::new(&name),
            errors: Diagnostics::new(),
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_builder_from(h: *mut A0Hir) -> *mut A0HirBuilder {
    if h.is_null() {
        return ptr::null_mut();
    }
    let A0Hir(m, _) = *unsafe { Box::from_raw(h) };
    unsafe {
        give(A0HirBuilder {
            b: HirBuilder::from_module(m),
            errors: Diagnostics::new(),
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_builder_free(b: *mut A0HirBuilder) {
    unsafe { free(b) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_span(b: *mut A0HirBuilder, start: u32, end: u32) {
    unsafe {
        with_unit(b, |h| {
            h.b.set_span(Some(Span::new(SOURCE, start, end)));
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_clear_span(b: *mut A0HirBuilder) {
    unsafe {
        with_unit(b, |h| {
            h.b.set_span(None);
            Some(())
        })
    }
}

/// Returns 0 if the text was added, -1 with `*diags` set if it has errors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_add_text(
    b: *mut A0HirBuilder,
    text: *const c_char,
    len: usize,
    diags: *mut *mut A0Diagnostics,
) -> i32 {
    let text = unsafe { self::text(text, len) };
    let Some(h) = (unsafe { b.as_mut() }) else {
        unsafe { report(diags, Some(error("a0_hir_add_text: no builder"))) };
        return -1;
    };
    match h.b.add_text(&text, SOURCE) {
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
pub unsafe extern "C" fn a0_hir_finish(
    b: *mut A0HirBuilder,
    diags: *mut *mut A0Diagnostics,
) -> *mut A0Hir {
    if b.is_null() {
        unsafe { report(diags, Some(error("a0_hir_finish: no builder"))) };
        return ptr::null_mut();
    }
    let A0HirBuilder { b, mut errors } = *unsafe { Box::from_raw(b) };
    let result = b.finish();
    match result {
        Ok(m) if !errors.has_errors() => {
            unsafe { report(diags, None) };
            unsafe { give(A0Hir(m, String::new())) }
        }
        result => {
            // The C layer's mistakes first: the builder's errors may follow from them.
            if let Err(more) = result {
                errors.extend(more);
            }
            unsafe { report(diags, Some(errors)) };
            ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------------- types

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_fresh_type(b: *mut A0HirBuilder) -> u32 {
    unsafe { with(b, |h| Some(h.b.fresh_type().0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_named_type(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    args: *const u32,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let args = h.ts(items(args, count))?;
            Some(h.b.named_type(&name, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_ptr_type(b: *mut A0HirBuilder, elem: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let elem = h.t(elem)?;
            Some(h.b.ptr_type(elem).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_size_of(b: *mut A0HirBuilder, t: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let t = h.t(t)?;
            Some(h.b.size_of(t).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_array_type(b: *mut A0HirBuilder, elem: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let elem = h.t(elem)?;
            Some(h.b.array_type(elem).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_map_type(b: *mut A0HirBuilder, key: u32, value: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let (key, value) = (h.t(key)?, h.t(value)?);
            Some(h.b.map_type(key, value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_fn_type(
    b: *mut A0HirBuilder,
    params: *const u32,
    count: usize,
    ret: u32,
) -> u32 {
    unsafe {
        with(b, |h| {
            let params = h.ts(items(params, count))?;
            let ret = h.t(ret)?;
            Some(h.b.fn_type(&params, ret).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_nullable_type(b: *mut A0HirBuilder, t: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let t = h.t(t)?;
            Some(h.b.nullable_type(t).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_struct_type(
    b: *mut A0HirBuilder,
    s: u32,
    args: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let s = h.s(s)?;
            let args = h.ts(items(args, count))?;
            Some(h.b.struct_type_args(s, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_enum_type(
    b: *mut A0HirBuilder,
    e: u32,
    args: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let e = h.en(e)?;
            let args = h.ts(items(args, count))?;
            Some(h.b.enum_type_args(e, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_type_param(
    b: *mut A0HirBuilder,
    kind: u32,
    item: u32,
    index: u32,
) -> u32 {
    unsafe {
        with(b, |h| {
            let item = h.item(kind, item)?;
            if index as usize >= h.b.module().type_params(item).len() {
                h.fail(format!("the item has no type parameter {index}"));
                return None;
            }
            Some(h.b.type_param(item, index).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_type_of(b: *mut A0HirBuilder, e: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let e = h.e(e)?;
            Some(h.b.type_of(e).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_type_syntax(
    b: *mut A0HirBuilder,
    tag: *const c_char,
    tag_len: usize,
    template: *const c_char,
    template_len: usize,
) {
    let tag = unsafe { text(tag, tag_len) };
    let template = unsafe { text(template, template_len) };
    unsafe {
        with_unit(b, |h| {
            let tag = match tag.as_ref() {
                "unit" => TypeTag::Unit,
                "bool" => TypeTag::Bool,
                "int" => TypeTag::Int,
                "float" => TypeTag::Float,
                "string" => TypeTag::String,
                "any" => TypeTag::Any,
                "never" => TypeTag::Never,
                "array" => TypeTag::Array,
                "map" => TypeTag::Map,
                "struct" => TypeTag::Struct,
                "enum" => TypeTag::Enum,
                "fn" => TypeTag::Fn,
                "nullable" => TypeTag::Nullable,
                "interface" => TypeTag::Interface,
                other => {
                    h.fail(format!("'{other}' is not a type tag"));
                    return None;
                }
            };
            h.b.set_type_syntax(tag, &template);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_term(
    b: *mut A0HirBuilder,
    term: *const c_char,
    term_len: usize,
    text_: *const c_char,
    text_len: usize,
) {
    let term = unsafe { text(term, term_len) };
    let words = unsafe { text(text_, text_len) };
    unsafe {
        with_unit(b, |h| {
            let term = match term.as_ref() {
                "function" => Term::Function,
                "closure" => Term::Closure,
                "struct" => Term::Struct,
                "enum" => Term::Enum,
                "variant" => Term::Variant,
                "field" => Term::Field,
                "variable" => Term::Variable,
                "null" => Term::Null,
                "unwrap" => Term::Unwrap,
                "narrow" => Term::Narrow,
                "propagate" => Term::Propagate,
                "match" => Term::Match,
                "break" => Term::Break,
                "continue" => Term::Continue,
                other => {
                    h.fail(format!("'{other}' is not a term"));
                    return None;
                }
            };
            h.b.set_term(term, &words);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_enable_warning(
    b: *mut A0HirBuilder,
    warning: *const c_char,
    len: usize,
) {
    let warning = unsafe { text(warning, len) };
    unsafe {
        with_unit(b, |h| {
            let w = match warning.as_ref() {
                "unused_variable" => Warning::UnusedVariable,
                "unreachable_code" => Warning::UnreachableCode,
                other => {
                    h.fail(format!("'{other}' is not a warning"));
                    return None;
                }
            };
            h.b.enable_warning(w);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_name_span(b: *mut A0HirBuilder, start: u32, end: u32) {
    unsafe {
        with_unit(b, |h| {
            h.b.set_name_span(Some(Span::new(SOURCE, start, end)));
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_param_span(
    b: *mut A0HirBuilder,
    body: u32,
    index: u32,
    start: u32,
    end: u32,
) {
    unsafe {
        with_unit(b, |h| {
            let body = h.body(body)?;
            h.b.set_param_span(body, index, Span::new(SOURCE, start, end));
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_report_code(
    b: *mut A0HirBuilder,
    code: *const c_char,
    code_len: usize,
    message: *const c_char,
    len: usize,
) {
    let code = unsafe { text(code, code_len) };
    let message = unsafe { text(message, len) };
    unsafe {
        with_unit(b, |h| {
            h.b.report_code(&code, &message);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_report_label(
    b: *mut A0HirBuilder,
    start: u32,
    end: u32,
    message: *const c_char,
    len: usize,
) {
    let message = unsafe { text(message, len) };
    unsafe {
        with_unit(b, |h| {
            h.b.report_label(Span::new(SOURCE, start, end), &message);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_report_note(b: *mut A0HirBuilder, note: *const c_char, len: usize) {
    let note = unsafe { text(note, len) };
    unsafe {
        with_unit(b, |h| {
            h.b.report_note(&note);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_report_fix(
    b: *mut A0HirBuilder,
    message: *const c_char,
    len: usize,
    start: u32,
    end: u32,
    replacement: *const c_char,
    replacement_len: usize,
) {
    let message = unsafe { text(message, len) };
    let replacement = unsafe { text(replacement, replacement_len) };
    unsafe {
        with_unit(b, |h| {
            h.b.report_fix(&message, Span::new(SOURCE, start, end), &replacement);
            Some(())
        })
    }
}

// ---------------------------------------------------------------------------------- declarations

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_memory(b: *mut A0HirBuilder, mode: *const c_char, len: usize) {
    let mode = unsafe { text(mode, len) };
    unsafe {
        with_unit(b, |h| {
            match a0_hir::Memory::from_name(&mode) {
                Some(m) => h.b.set_memory(m),
                None => h.fail(format!("'{mode}' is not a memory mode (gc, manual)")),
            }
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_struct_inline(b: *mut A0HirBuilder, s: u32) {
    unsafe {
        with_unit(b, |h| {
            h.b.set_struct_inline(a0_hir::StructId(s));
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_struct(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    type_params: *const A0Str,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    let params = unsafe { names(type_params, count) };
    unsafe {
        with(b, |h| {
            Some(h.b.declare_generic_struct(&name, &refs(&params)).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_add_field(
    b: *mut A0HirBuilder,
    s: u32,
    name: *const c_char,
    len: usize,
    ty: u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let (s, ty) = (h.s(s)?, h.t(ty)?);
            Some(h.b.add_field(s, &name, ty))
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_enum(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    type_params: *const A0Str,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    let params = unsafe { names(type_params, count) };
    unsafe {
        with(b, |h| {
            Some(h.b.declare_generic_enum(&name, &refs(&params)).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_add_variant(
    b: *mut A0HirBuilder,
    e: u32,
    name: *const c_char,
    len: usize,
    fields: *const u32,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let e = h.en(e)?;
            let fields = h.ts(items(fields, count))?;
            Some(h.b.add_variant(e, &name, &fields))
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_global(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    ty: u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let ty = h.t(ty)?;
            Some(h.b.declare_global(&name, ty).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_extern(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    params: *const u32,
    count: usize,
    ret: u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let params = h.ts(items(params, count))?;
            let ret = h.t(ret)?;
            Some(h.b.declare_extern(&name, &params, ret).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_extern_abi(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    params: *const u32,
    count: usize,
    ret: u32,
    abi: *const c_char,
    abi_len: usize,
) -> u32 {
    let (name, abi) = unsafe { (text(name, len), text(abi, abi_len)) };
    unsafe {
        with(b, |h| {
            let params = h.ts(items(params, count))?;
            let ret = h.t(ret)?;
            let Some(abi) = a0_hir::Abi::from_name(&abi) else {
                h.fail(format!("{abi} is not an ABI (\"C\" or \"a0\")"));
                return None;
            };
            Some(h.b.declare_extern_abi(&name, &params, ret, abi).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_function(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    type_params: *const A0Str,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    let params = unsafe { names(type_params, count) };
    unsafe {
        with(b, |h| {
            Some(h.b.declare_generic_function(&name, &refs(&params)).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_signature(
    b: *mut A0HirBuilder,
    f: u32,
    param_names: *const A0Str,
    param_types: *const u32,
    count: usize,
    ret: u32,
) {
    let param_names = unsafe { names(param_names, count) };
    unsafe {
        with_unit(b, |h| {
            let f = h.f(f)?;
            let types = h.ts(items(param_types, count))?;
            let ret = h.t(ret)?;
            h.b.set_signature(f, &refs(&param_names), &types, ret);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_function_body(b: *mut A0HirBuilder, f: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let f = h.f(f)?;
            Some(h.b.function_body(f).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_new_closure_body(
    b: *mut A0HirBuilder,
    parent: u32,
    param_names: *const A0Str,
    param_types: *const u32,
    count: usize,
    ret: u32,
) -> u32 {
    let param_names = unsafe { names(param_names, count) };
    unsafe {
        with(b, |h| {
            let parent = h.body(parent)?;
            let types = h.ts(items(param_types, count))?;
            let ret = h.t(ret)?;
            Some(
                h.b.new_closure_body(parent, &refs(&param_names), &types, ret)
                    .0,
            )
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_closure_name(
    b: *mut A0HirBuilder,
    body: u32,
    name: *const c_char,
    len: usize,
) {
    let name = unsafe { text(name, len) };
    unsafe {
        with_unit(b, |h| {
            let body = h.body(body)?;
            h.b.set_closure_name(body, &name);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_param(b: *mut A0HirBuilder, body: u32, i: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let body = h.body(body)?;
            let n = h.b.module().body_params(body).len() as u32;
            let i = h.check("parameter index", i, n)?;
            Some(h.b.param(body, i).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_new_local(
    b: *mut A0HirBuilder,
    body: u32,
    name: *const c_char,
    len: usize,
    ty: u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let (body, ty) = (h.body(body)?, h.t(ty)?);
            Some(h.b.new_local(body, &name, ty).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_root(b: *mut A0HirBuilder, body: u32, root: u32) {
    unsafe {
        with_unit(b, |h| {
            let (body, root) = (h.body(body)?, h.e(root)?);
            h.b.set_root(body, root);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_lookup_type(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    kind: *mut u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let (k, id) = split_item(h.b.lookup_type_name(&name)?);
            if !kind.is_null() {
                *kind = k;
            }
            Some(id)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_lookup_value(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    kind: *mut u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let (k, id) = split_item(h.b.lookup_value_name(&name)?);
            if !kind.is_null() {
                *kind = k;
            }
            Some(id)
        })
    }
}

// ---------------------------------------------------------------------------------- expressions

/// Expressions without operands.
macro_rules! leaf {
    ($name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(b: *mut A0HirBuilder) -> u32 {
            unsafe { with(b, |h| Some(h.b.$method().0)) }
        }
    };
}

leaf!(a0_hir_unit, unit);
leaf!(a0_hir_break, break_);
leaf!(a0_hir_continue, continue_);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_bool(b: *mut A0HirBuilder, v: c_int) -> u32 {
    unsafe { with(b, |h| Some(h.b.bool_lit(v != 0).0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_int(b: *mut A0HirBuilder, v: i64) -> u32 {
    unsafe { with(b, |h| Some(h.b.int(v).0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_float(b: *mut A0HirBuilder, v: f64) -> u32 {
    unsafe { with(b, |h| Some(h.b.float(v).0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_string(b: *mut A0HirBuilder, s: *const c_char, len: usize) -> u32 {
    let s = unsafe { text(s, len) };
    unsafe { with(b, |h| Some(h.b.string(&s).0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_null(b: *mut A0HirBuilder, ty: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let ty = h.t(ty)?;
            Some(h.b.null(ty).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_local(b: *mut A0HirBuilder, l: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let l = h.l(l)?;
            Some(h.b.local(l).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_global(b: *mut A0HirBuilder, g: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let g = h.g(g)?;
            Some(h.b.global(g).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_func(
    b: *mut A0HirBuilder,
    f: u32,
    type_args: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let f = h.f(f)?;
            let targs = h.ts(items(type_args, count))?;
            Some(h.b.func_generic(f, &targs).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_let(b: *mut A0HirBuilder, l: u32, init: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let (l, init) = (h.l(l)?, h.e(init)?);
            Some(h.b.let_(l, init).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_local(b: *mut A0HirBuilder, l: u32, value: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let (l, value) = (h.l(l)?, h.e(value)?);
            Some(h.b.set_local(l, value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_global(b: *mut A0HirBuilder, g: u32, value: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let (g, value) = (h.g(g)?, h.e(value)?);
            Some(h.b.set_global(g, value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_field(
    b: *mut A0HirBuilder,
    object: u32,
    field: u32,
    value: u32,
) -> u32 {
    unsafe {
        with(b, |h| {
            let (object, value) = (h.e(object)?, h.e(value)?);
            Some(h.b.set_field(object, field, value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_field_named(
    b: *mut A0HirBuilder,
    object: u32,
    name: *const c_char,
    len: usize,
    value: u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let (object, value) = (h.e(object)?, h.e(value)?);
            Some(h.b.set_field_named(object, &name, value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_index(
    b: *mut A0HirBuilder,
    target: u32,
    index: u32,
    value: u32,
) -> u32 {
    unsafe {
        with(b, |h| {
            let (target, index, value) = (h.e(target)?, h.e(index)?, h.e(value)?);
            Some(h.b.set_index(target, index, value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_unary(
    b: *mut A0HirBuilder,
    op: *const c_char,
    len: usize,
    x: u32,
) -> u32 {
    let op = unsafe { text(op, len) };
    unsafe {
        with(b, |h| {
            let x = h.e(x)?;
            let op = match op.as_ref() {
                "-" => UnOp::Neg,
                "!" => UnOp::Not,
                other => {
                    h.fail(format!("'{other}' is not a unary operator"));
                    return None;
                }
            };
            Some(h.b.unary(op, x).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_binary(
    b: *mut A0HirBuilder,
    op: *const c_char,
    len: usize,
    x: u32,
    y: u32,
) -> u32 {
    let op = unsafe { text(op, len) };
    unsafe {
        with(b, |h| {
            let (x, y) = (h.e(x)?, h.e(y)?);
            let built = match op.as_ref() {
                "&&" => h.b.and(x, y),
                "||" => h.b.or(x, y),
                other => match BinOp::from_symbol(other) {
                    Some(op) => h.b.binary(op, x, y),
                    None => {
                        h.fail(format!("'{other}' is not a binary operator"));
                        return None;
                    }
                },
            };
            Some(built.0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_call(
    b: *mut A0HirBuilder,
    f: u32,
    type_args: *const u32,
    type_count: usize,
    args: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let f = h.f(f)?;
            let targs = h.ts(items(type_args, type_count))?;
            let args = h.es(items(args, count))?;
            Some(h.b.call_generic(f, &targs, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_call_extern(
    b: *mut A0HirBuilder,
    x: u32,
    args: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let x = h.x(x)?;
            let args = h.es(items(args, count))?;
            Some(h.b.call_extern(x, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_call_value(
    b: *mut A0HirBuilder,
    callee: u32,
    args: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let callee = h.e(callee)?;
            let args = h.es(items(args, count))?;
            Some(h.b.call_value(callee, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_lambda(b: *mut A0HirBuilder, body: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let body = h.body(body)?;
            Some(h.b.lambda(body).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_new_struct(
    b: *mut A0HirBuilder,
    s: u32,
    type_args: *const u32,
    type_count: usize,
    fields: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let s = h.s(s)?;
            let targs = h.ts(items(type_args, type_count))?;
            let fields = h.es(items(fields, count))?;
            Some(h.b.new_struct_generic(s, &targs, &fields).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_field(b: *mut A0HirBuilder, object: u32, field: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let object = h.e(object)?;
            Some(h.b.field(object, field).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_field_named(
    b: *mut A0HirBuilder,
    object: u32,
    name: *const c_char,
    len: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let object = h.e(object)?;
            Some(h.b.field_named(object, &name).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_variant(
    b: *mut A0HirBuilder,
    e: u32,
    type_args: *const u32,
    type_count: usize,
    variant: u32,
    args: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let e = h.en(e)?;
            let targs = h.ts(items(type_args, type_count))?;
            let args = h.es(items(args, count))?;
            Some(h.b.variant_generic(e, &targs, variant, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_arm(
    b: *mut A0HirBuilder,
    variant: u32,
    binds: *const u32,
    count: usize,
    body: u32,
) -> u32 {
    unsafe {
        with(b, |h| {
            let binds = items(binds, count)
                .iter()
                .map(|&l| h.l(l))
                .collect::<Option<Vec<_>>>()?;
            let body = h.e(body)?;
            let variant = (variant != NONE).then_some(variant);
            Some(h.b.arm(variant, &binds, body).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_match(
    b: *mut A0HirBuilder,
    scrutinee: u32,
    arms: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let scrutinee = h.e(scrutinee)?;
            let arms = items(arms, count)
                .iter()
                .map(|&a| h.arm(a))
                .collect::<Option<Vec<_>>>()?;
            Some(h.b.match_(scrutinee, &arms).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_array(
    b: *mut A0HirBuilder,
    elems: *const u32,
    count: usize,
) -> u32 {
    unsafe {
        with(b, |h| {
            let elems = h.es(items(elems, count))?;
            Some(h.b.array(&elems).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_new_array(b: *mut A0HirBuilder, array_type: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let t = h.t(array_type)?;
            Some(h.b.new_array(t).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_new_map(b: *mut A0HirBuilder, map_type: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let t = h.t(map_type)?;
            Some(h.b.new_map(t).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_new_object(b: *mut A0HirBuilder) -> u32 {
    unsafe { with(b, |h| Some(h.b.new_object().0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_index(b: *mut A0HirBuilder, target: u32, index: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let (target, index) = (h.e(target)?, h.e(index)?);
            Some(h.b.index(target, index).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_builtin(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    args: *const u32,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let args = h.es(items(args, count))?;
            let Some(op) = Builtin::from_name(&name) else {
                h.fail(format!("'{name}' is not a builtin"));
                return None;
            };
            Some(h.b.builtin(op, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_cast(b: *mut A0HirBuilder, e: u32, ty: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let (e, ty) = (h.e(e)?, h.t(ty)?);
            Some(h.b.cast(e, ty).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_unwrap(b: *mut A0HirBuilder, e: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let e = h.e(e)?;
            Some(h.b.unwrap(e).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_is(b: *mut A0HirBuilder, e: u32, ty: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let (e, ty) = (h.e(e)?, h.t(ty)?);
            Some(h.b.is(e, ty).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_if(b: *mut A0HirBuilder, cond: u32, then: u32, els: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let (cond, then, els) = (h.e(cond)?, h.e(then)?, h.opt_e(els)?);
            Some(h.b.if_(cond, then, els).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_if_let(
    b: *mut A0HirBuilder,
    local: u32,
    value: u32,
    then: u32,
    els: u32,
) -> u32 {
    unsafe {
        with(b, |h| {
            let local = h.l(local)?;
            let (value, then, els) = (h.e(value)?, h.e(then)?, h.opt_e(els)?);
            Some(h.b.if_let(local, value, then, els).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_while(b: *mut A0HirBuilder, cond: u32, body: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let (cond, body) = (h.e(cond)?, h.e(body)?);
            Some(h.b.while_(cond, body).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_loop(b: *mut A0HirBuilder, body: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let body = h.e(body)?;
            Some(h.b.loop_(body).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_return(b: *mut A0HirBuilder, value: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let value = h.opt_e(value)?;
            Some(h.b.return_(value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_block(
    b: *mut A0HirBuilder,
    items_: *const u32,
    count: usize,
    tail: u32,
) -> u32 {
    unsafe {
        with(b, |h| {
            let list = h.es(items(items_, count))?;
            let tail = h.opt_e(tail)?;
            Some(h.b.block(&list, tail).0)
        })
    }
}

// ---------------------------------------------------------------------------------- by name

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_begin_body(b: *mut A0HirBuilder, body: u32) {
    unsafe {
        with_unit(b, |h| {
            let body = h.body(body)?;
            h.b.begin_body(body);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_end_body(b: *mut A0HirBuilder) {
    unsafe {
        with_unit(b, |h| {
            h.b.end_body();
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_push_scope(b: *mut A0HirBuilder) {
    unsafe {
        with_unit(b, |h| {
            h.b.push_scope();
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_pop_scope(b: *mut A0HirBuilder) {
    unsafe {
        with_unit(b, |h| {
            h.b.pop_scope();
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_is_declared_in_scope(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
) -> c_int {
    let name = unsafe { text(name, len) };
    unsafe { b.as_ref() }.is_some_and(|h| h.b.is_declared_in_scope(&name)) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_local(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    ty: u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            h.needs_body()?;
            let ty = h.t(ty)?;
            Some(h.b.declare_local(&name, ty).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_throw(b: *mut A0HirBuilder, value: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let value = h.e(value)?;
            Some(h.b.throw(value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_try_catch(
    b: *mut A0HirBuilder,
    body: u32,
    local: u32,
    handler: u32,
) -> u32 {
    unsafe {
        with(b, |h| {
            let local = h.l(local)?;
            let (body, handler) = (h.e(body)?, h.e(handler)?);
            Some(h.b.try_catch(body, local, handler).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_catch(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            h.needs_body()?;
            Some(h.b.declare_catch(&name).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_propagate(b: *mut A0HirBuilder, e: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let e = h.e(e)?;
            Some(h.b.propagate(e).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_if_let(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    value: u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            h.needs_body()?;
            let value = h.e(value)?;
            Some(h.b.declare_if_let(&name, value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_let_named(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    ty: u32,
    value: u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            h.needs_body()?;
            let (ty, value) = (h.t(ty)?, h.e(value)?);
            Some(h.b.let_named(&name, ty, value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_lookup_local(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe { with(b, |h| h.b.lookup_local(&name).map(|l| l.0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_report(b: *mut A0HirBuilder, message: *const c_char, len: usize) {
    let message = unsafe { text(message, len) };
    unsafe {
        with_unit(b, |h| {
            h.b.report(&message);
            Some(())
        })
    }
}

leaf!(a0_hir_error_expr, error_expr);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_name(b: *mut A0HirBuilder, name: *const c_char, len: usize) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe { with(b, |h| Some(h.b.name(&name).0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_call_named(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    args: *const u32,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let args = h.es(items(args, count))?;
            Some(h.b.call_named(&name, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_assign_named(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    value: u32,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let value = h.e(value)?;
            Some(h.b.assign_named(&name, value).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_bind_type_params(b: *mut A0HirBuilder, kind: u32, item: u32) {
    unsafe {
        with_unit(b, |h| {
            let item = h.item(kind, item)?;
            h.b.bind_type_params(item);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_new_struct_named(
    b: *mut A0HirBuilder,
    s: u32,
    field_names: *const A0Str,
    values: *const u32,
    count: usize,
) -> u32 {
    let field_names = unsafe { names(field_names, count) };
    unsafe {
        with(b, |h| {
            let s = h.s(s)?;
            let values = h.es(items(values, count))?;
            Some(h.b.new_struct_named(s, &refs(&field_names), &values).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_variant_named(
    b: *mut A0HirBuilder,
    e: u32,
    name: *const c_char,
    len: usize,
    args: *const u32,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let e = h.en(e)?;
            let args = h.es(items(args, count))?;
            Some(h.b.variant_named(e, &name, &args).0)
        })
    }
}

// ------------------------------------------------------------------------------------- modules

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_begin_module(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    root: c_int,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe { with(b, |h| Some(h.b.begin_module(&name, root != 0).0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_set_public(b: *mut A0HirBuilder, public: c_int) {
    unsafe {
        with_unit(b, |h| {
            h.b.set_public(public != 0);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_import_module(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
    alias: *const c_char,
    alias_len: usize,
) {
    let (name, alias) = unsafe { (text(name, len), text(alias, alias_len)) };
    unsafe {
        with_unit(b, |h| {
            h.b.import_module(&name, &alias);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_import_item(
    b: *mut A0HirBuilder,
    module: *const c_char,
    module_len: usize,
    name: *const c_char,
    len: usize,
) {
    let (module, name) = unsafe { (text(module, module_len), text(name, len)) };
    unsafe {
        with_unit(b, |h| {
            h.b.import_item(&module, &name);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_is_module(
    b: *mut A0HirBuilder,
    alias: *const c_char,
    len: usize,
) -> c_int {
    let alias = unsafe { text(alias, len) };
    unsafe { b.as_ref() }.is_some_and(|h| h.b.is_module(&alias)) as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_member(
    b: *mut A0HirBuilder,
    alias: *const c_char,
    alias_len: usize,
    name: *const c_char,
    len: usize,
) -> u32 {
    let (alias, name) = unsafe { (text(alias, alias_len), text(name, len)) };
    unsafe { with(b, |h| Some(h.b.member(&alias, &name).0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_call_member(
    b: *mut A0HirBuilder,
    alias: *const c_char,
    alias_len: usize,
    name: *const c_char,
    len: usize,
    args: *const u32,
    count: usize,
) -> u32 {
    let (alias, name) = unsafe { (text(alias, alias_len), text(name, len)) };
    unsafe {
        with(b, |h| {
            let args = h.es(items(args, count))?;
            Some(h.b.call_member(&alias, &name, &args).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_member_type(
    b: *mut A0HirBuilder,
    alias: *const c_char,
    alias_len: usize,
    name: *const c_char,
    len: usize,
    args: *const u32,
    count: usize,
) -> u32 {
    let (alias, name) = unsafe { (text(alias, alias_len), text(name, len)) };
    unsafe {
        with(b, |h| {
            let args = h.ts(items(args, count))?;
            Some(h.b.member_type(&alias, &name, &args).0)
        })
    }
}

// ----------------------------------------------------------------------- methods and interfaces

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_interface(
    b: *mut A0HirBuilder,
    name: *const c_char,
    len: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe { with(b, |h| Some(h.b.declare_interface(&name).0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_interface_self(b: *mut A0HirBuilder, i: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let i = h.i(i)?;
            Some(h.b.interface_self(i).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_interface_type(b: *mut A0HirBuilder, i: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let i = h.i(i)?;
            Some(h.b.interface_type(i).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_add_interface_method(
    b: *mut A0HirBuilder,
    i: u32,
    name: *const c_char,
    len: usize,
    params: *const u32,
    count: usize,
    ret: u32,
) {
    let name = unsafe { text(name, len) };
    unsafe {
        with_unit(b, |h| {
            let i = h.i(i)?;
            let params = h.ts(items(params, count))?;
            let ret = h.t(ret)?;
            h.b.add_interface_method(i, &name, &params, ret);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_declare_method(
    b: *mut A0HirBuilder,
    owner: u32,
    name: *const c_char,
    len: usize,
    type_params: *const A0Str,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    let type_params = unsafe { names(type_params, count) };
    unsafe {
        with(b, |h| {
            let owner = h.t(owner)?;
            Some(h.b.declare_method(owner, &name, &refs(&type_params)).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_method_self_type(b: *mut A0HirBuilder, f: u32) -> u32 {
    unsafe {
        with(b, |h| {
            let f = h.f(f)?;
            Some(h.b.method_self_type(f).0)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_implement(b: *mut A0HirBuilder, owner: u32, i: u32) {
    unsafe {
        with_unit(b, |h| {
            let (owner, i) = (h.t(owner)?, h.i(i)?);
            h.b.implement(owner, i);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_add_bound(
    b: *mut A0HirBuilder,
    item_kind: u32,
    item: u32,
    index: u32,
    i: u32,
) {
    unsafe {
        with_unit(b, |h| {
            let (item, i) = (h.item(item_kind, item)?, h.i(i)?);
            h.b.add_bound(item, index, i);
            Some(())
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_hir_method_call(
    b: *mut A0HirBuilder,
    receiver: u32,
    name: *const c_char,
    len: usize,
    args: *const u32,
    count: usize,
) -> u32 {
    let name = unsafe { text(name, len) };
    unsafe {
        with(b, |h| {
            let receiver = h.e(receiver)?;
            let args = h.es(items(args, count))?;
            Some(h.b.method_call(receiver, &name, &args).0)
        })
    }
}
