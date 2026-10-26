//! A language whose compiler is a C function (or a Python one, through ctypes), given every
//! command of the Rust driver (`a0_driver::Language`): `run`, `build`, `hir`, `ir`, `lsp`, `dap`,
//! `repl` and `explain` (D67).
//!
//! The driver takes its compiler as a plain function, so the C one is kept in a global for the
//! function that calls it: a process runs one language's driver.

use std::ffi::{c_char, c_int, c_void};
use std::sync::Mutex;

use a0_diag::SourceId;
use a0_driver::{CompileError, Message, Program};

use crate::{A0Diagnostics, A0Hir, free, give, items, text};

/// Compiles the text of one source file into HIR (spans in source 0, which is where the driver
/// puts the file), or returns NULL with diagnostics in `*diags` (NULL too if the compiler failed
/// without saying why).
pub type A0CompileFn = unsafe extern "C" fn(
    context: *mut c_void,
    text: *const c_char,
    len: usize,
    diags: *mut *mut A0Diagnostics,
) -> *mut A0Hir;

pub struct A0Language {
    name: String,
    compile: Option<(A0CompileFn, usize)>,
    grammar: Option<String>,
    repl_main: Option<String>,
}

/// The compiler of the language whose driver runs in this process, and its context.
static COMPILER: Mutex<Option<(A0CompileFn, usize)>> = Mutex::new(None);

fn compile(source: &str, id: SourceId) -> Result<Program, CompileError> {
    let Some((f, context)) = *COMPILER.lock().unwrap() else {
        return Err(failed(
            "the language has no compiler (a0_language_set_compiler)",
        ));
    };
    if id != SourceId(0) {
        return Err(failed(
            "a language compiled by a C function has one source file",
        ));
    }
    let mut diags: *mut A0Diagnostics = std::ptr::null_mut();
    let hir = unsafe {
        f(
            context as *mut c_void,
            source.as_ptr().cast(),
            source.len(),
            &mut diags,
        )
    };
    if !hir.is_null() {
        let hir = unsafe { Box::from_raw(hir) };
        unsafe { free(diags) };
        return Ok(Program::Hir(hir.0));
    }
    if diags.is_null() {
        return Err(failed("the compiler failed without saying why"));
    }
    let d = unsafe { Box::from_raw(diags) };
    if !d.diags.has_errors() {
        return Err(failed("the compiler returned no module and no errors"));
    }
    Err(CompileError::Diagnostics(d.diags))
}

fn failed(message: &str) -> CompileError {
    CompileError::Messages(vec![Message::new(message, None)])
}

/// A language named `name` (its command, in messages and the usage text).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_language_new(name: *const c_char, len: usize) -> *mut A0Language {
    unsafe {
        give(A0Language {
            name: text(name, len).into_owned(),
            compile: None,
            grammar: None,
            repl_main: None,
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_language_set_compiler(
    l: *mut A0Language,
    compile: Option<A0CompileFn>,
    context: *mut c_void,
) {
    if let Some(l) = unsafe { l.as_mut() } {
        l.compile = compile.map(|f| (f, context as usize));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_language_set_grammar(
    l: *mut A0Language,
    grammar: *const c_char,
    len: usize,
) {
    if let Some(l) = unsafe { l.as_mut() } {
        l.grammar = Some(unsafe { text(grammar, len) }.into_owned());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_language_set_repl_main(
    l: *mut A0Language,
    program: *const c_char,
    len: usize,
) {
    if let Some(l) = unsafe { l.as_mut() } {
        l.repl_main = Some(unsafe { text(program, len) }.into_owned());
    }
}

/// Leaks `s`: the driver keeps the language's strings for the rest of the process.
fn forever(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_language_main(
    l: *const A0Language,
    args: *const *const c_char,
    count: usize,
) -> c_int {
    let Some(l) = (unsafe { l.as_ref() }) else {
        return 2;
    };
    *COMPILER.lock().unwrap() = l.compile;
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
    let mut language = a0_driver::Language::new(forever(&l.name), compile);
    if let Some(g) = l.grammar.as_deref() {
        language = language.grammar(forever(g));
    }
    if let Some(r) = l.repl_main.as_deref() {
        language = language.repl_main(forever(r));
    }
    let status = language.run_status(&args);
    a0_runtime::flush();
    status as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn a0_language_free(l: *mut A0Language) {
    unsafe { free(l) }
}
