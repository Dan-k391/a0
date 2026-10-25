//! Rust glue for the A0 C runtime.
//!
//! The runtime itself is C (`include/a0rt.h`, `c/*.c`). This crate compiles it into a static
//! library, links it into Rust users (so the interpreter can call it), and embeds a copy of the
//! library for the native backend to pass to the system linker.

use std::ffi::{CStr, c_char, c_void};

/// One entry of a C symbol table (`A0Symbol` in `a0rt.h`).
#[repr(C)]
pub struct A0Symbol {
    pub name: *const c_char,
    pub address: *mut c_void,
}

unsafe extern "C" {
    static a0_rt_symbols: A0Symbol;
    fn a0_rt_find_symbol(name: *const c_char) -> *mut c_void;

    pub fn a0_rt_init(argc: i32, argv: *mut *mut c_char);
    pub fn a0_rt_exit(code: i32) -> !;
    pub fn a0_rt_flush();
    pub fn a0_rt_trap(code: i32, loc: u64, frame: *mut c_void) -> !;
    fn a0_rt_set_debug(locations: *const u8, code: *const c_void);
    fn a0_rt_set_frames(frames: Option<unsafe extern "C" fn(*mut u32, usize) -> usize>);
    fn a0_rt_fail_frames(message: *const c_char, ids: *const u32, count: usize) -> !;
    fn a0_rt_trace_text(ids: *const u32, count: usize, buf: *mut c_char, cap: usize) -> usize;
    pub fn a0_ffi_call(
        f: *const c_void,
        nargs: i32,
        args: *const u64,
        float_mask: u32,
        ret_kind: i32,
        ret: *mut u64,
    ) -> i32;
}

/// Trap codes understood by `a0_rt_trap`.
pub mod trap {
    pub const UNREACHABLE: i32 = 1;
    pub const DIVIDE_BY_ZERO: i32 = 2;
    pub const INTEGER_OVERFLOW: i32 = 3;
    pub const STACK_OVERFLOW: i32 = 4;
}

/// Exit status of a process that trapped or panicked.
pub const PANIC_EXIT_CODE: i32 = 101;

/// Runtime functions that can fail. Generated code passes them two arguments more than the IR
/// declares: the location of the call (a `u64` id in the location table, see [`debug`]) and the
/// caller's frame pointer (null from the interpreter), so that a failure can say where it happened
/// and how the program got there.
pub const REPORTS_LOCATION: &[&str] = &[
    "a0_rt_trap",
    "a0_panic_bytes",
    "a0_any_to_int",
    "a0_any_cast_failed",
    "a0_string_substr",
    "a0_string_byte_at",
    "a0_string_panic",
    "a0_array_get",
    "a0_array_set",
    "a0_array_pop",
    "a0_map_get",
    "a0_string_from_c",
    "a0_rt_throw",
];

/// Whether calls to the runtime function `name` pass a location and a frame pointer.
pub fn reports_location(name: &str) -> bool {
    REPORTS_LOCATION.contains(&name)
}

/// The location table the runtime prints traces from (`A0LocationTable` in `a0rt.h`).
pub mod debug {
    /// One location: (function as traces name it, file, line, column, parent id), where the parent
    /// is the id (index plus one) of the location of the call it was inlined at, or 0.
    pub type Entry<'a> = (&'a str, &'a str, u32, u32, u32);

    /// The table's bytes: a header, the entries, then their strings, NUL-terminated.
    pub fn encode(entries: &[Entry]) -> Vec<u8> {
        let mut strings: Vec<u8> = Vec::new();
        let mut offsets = std::collections::HashMap::new();
        let mut string = |s: &str| -> u32 {
            *offsets.entry(s.to_string()).or_insert_with(|| {
                let at = strings.len() as u32;
                strings.extend_from_slice(s.as_bytes());
                strings.push(0);
                at
            })
        };
        let mut words: Vec<u32> = Vec::with_capacity(2 + entries.len() * 5);
        words.push(entries.len() as u32);
        words.push(0);
        for &(func, file, line, col, parent) in entries {
            words.extend([string(func), string(file), line, col, parent]);
        }
        words[1] = (words.len() * 4) as u32;
        let mut out: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        out.extend(strings);
        out
    }
}

/// Maximum number of arguments `a0_ffi_call` supports.
pub const FFI_MAX_ARGS: usize = 8;

/// Result kinds for [`ffi_call`].
pub mod ffi_ret {
    pub const VOID: i32 = 0;
    pub const INT: i32 = 1;
    pub const F64: i32 = 2;
}

/// Calls the C function at `address`.
///
/// `args` holds one 64-bit slot per argument; bit `i` of `float_mask` marks argument `i` as a double
/// (its bit pattern). Returns the raw 64-bit result, or `None` if the call shape is unsupported.
///
/// # Safety
/// `address` must point to a C function whose signature matches the described shape.
pub unsafe fn ffi_call(
    address: usize,
    args: &[u64],
    float_mask: u32,
    ret_kind: i32,
) -> Option<u64> {
    let mut ret = 0u64;
    let status = unsafe {
        a0_ffi_call(
            address as *const c_void,
            args.len() as i32,
            args.as_ptr(),
            float_mask,
            ret_kind,
            &mut ret,
        )
    };
    (status == 0).then_some(ret)
}

/// Reads a NULL-terminated C symbol table into `(name, address)` pairs.
///
/// # Safety
/// `table` must point to a valid `A0Symbol` array terminated by a NULL name.
pub unsafe fn read_symbol_table(table: *const A0Symbol) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut p = table;
    loop {
        let entry = unsafe { &*p };
        if entry.name.is_null() {
            return out;
        }
        let name = unsafe { CStr::from_ptr(entry.name) }
            .to_string_lossy()
            .into_owned();
        out.push((name, entry.address as usize));
        p = unsafe { p.add(1) };
    }
}

/// The runtime's own symbol table: its functions plus common C library functions.
pub fn runtime_symbols() -> Vec<(String, usize)> {
    unsafe { read_symbol_table(&raw const a0_rt_symbols) }
}

/// The address of a C function `name` exported by the executable or a library loaded into this
/// process (the C library, for one).
pub fn find_symbol(name: &str) -> Option<usize> {
    let name = std::ffi::CString::new(name).ok()?;
    let p = unsafe { a0_rt_find_symbol(name.as_ptr()) };
    (!p.is_null()).then_some(p as usize)
}

/// Initializes the runtime for an in-process run (the interpreter calls this before `main`).
pub fn init(args: &[String]) {
    // The runtime keeps the pointers, so leak them for the life of the process.
    let mut ptrs: Vec<*mut c_char> = args
        .iter()
        .map(|a| {
            let mut bytes = a.clone().into_bytes();
            bytes.push(0);
            Box::leak(bytes.into_boxed_slice()).as_mut_ptr() as *mut c_char
        })
        .collect();
    ptrs.push(std::ptr::null_mut());
    let argv = Box::leak(ptrs.into_boxed_slice()).as_mut_ptr();
    unsafe { a0_rt_init(args.len() as i32, argv) };
}

/// Flushes C stdio buffers.
pub fn flush() {
    unsafe { a0_rt_flush() };
}

/// Exits the process through the runtime (flushing C stdio first).
pub fn exit(code: i32) -> ! {
    unsafe { a0_rt_exit(code) }
}

/// Reports a trap with no location and exits with [`PANIC_EXIT_CODE`].
pub fn trap(code: i32) -> ! {
    unsafe { a0_rt_trap(code, 0, std::ptr::null_mut()) }
}

/// Gives the runtime the program's location table ([`debug::encode`]), which must live as long as
/// the program runs, and how to find the interpreter's frames.
///
/// # Safety
/// `locations` must stay valid until the program ends.
pub unsafe fn set_debug(locations: &[u8], frames: unsafe extern "C" fn(*mut u32, usize) -> usize) {
    unsafe {
        a0_rt_set_debug(locations.as_ptr(), std::ptr::null());
        a0_rt_set_frames(Some(frames));
    }
}

/// Fails with `message` and the frames `ids` (innermost first), and exits.
pub fn fail_frames(message: &str, ids: &[u32]) -> ! {
    let message = std::ffi::CString::new(message.replace('\0', " ")).unwrap();
    unsafe { a0_rt_fail_frames(message.as_ptr(), ids.as_ptr(), ids.len()) }
}

/// The trace the runtime prints for the frames `ids`, with the table given to [`set_debug`].
pub fn trace_text(ids: &[u32]) -> String {
    let len = unsafe { a0_rt_trace_text(ids.as_ptr(), ids.len(), std::ptr::null_mut(), 0) };
    let mut buf = vec![0u8; len + 1];
    unsafe {
        a0_rt_trace_text(
            ids.as_ptr(),
            ids.len(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        )
    };
    buf.truncate(len);
    String::from_utf8_lossy(&buf).into_owned()
}

/// The runtime compiled as a static library for the host, to be passed to the system linker.
pub fn static_library() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/a0rt.embedded"))
}

/// Directory containing `a0rt.h`, for compiling C code against the runtime.
pub fn include_dir() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/include")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_table_has_runtime_and_libc() {
        let syms = runtime_symbols();
        for name in ["a0_print_f64", "a0_alloc", "sin", "putchar", "memcpy"] {
            assert!(
                syms.iter().any(|(n, a)| n == name && *a != 0),
                "{name} missing"
            );
        }
    }

    fn lookup(name: &str) -> usize {
        runtime_symbols()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, a)| a)
            .unwrap()
    }

    #[test]
    fn ffi_calls_doubles_and_integers() {
        let pow = lookup("pow");
        let r = unsafe { ffi_call(pow, &[2f64.to_bits(), 10f64.to_bits()], 0b11, ffi_ret::F64) };
        assert_eq!(f64::from_bits(r.unwrap()), 1024.0);

        let strlen = lookup("strlen");
        let s = c"hello";
        let r = unsafe { ffi_call(strlen, &[s.as_ptr() as u64], 0, ffi_ret::INT) };
        assert_eq!(r, Some(5));

        // memset(buf, 'x', 3): mixed pointer and int arguments.
        let memset = lookup("memset");
        let mut buf = [0u8; 4];
        let r = unsafe {
            ffi_call(
                memset,
                &[buf.as_mut_ptr() as u64, b'x' as u64, 3],
                0,
                ffi_ret::INT,
            )
        };
        assert!(r.is_some());
        assert_eq!(&buf, b"xxx\0");
    }

    #[test]
    fn ffi_rejects_unsupported_shapes() {
        let pow = lookup("pow");
        let args = [0u64; 9];
        assert_eq!(unsafe { ffi_call(pow, &args, 0, ffi_ret::F64) }, None);
        // A float flag beyond the argument count is malformed.
        assert_eq!(unsafe { ffi_call(pow, &[0], 0b10, ffi_ret::F64) }, None);
    }

    unsafe extern "C" {
        fn a0_format_f64(value: f64, buf: *mut c_char) -> i32;
    }

    fn fmt(v: f64) -> String {
        let mut buf = [0 as c_char; 32];
        let n = unsafe { a0_format_f64(v, buf.as_mut_ptr()) };
        let bytes: Vec<u8> = buf[..n as usize].iter().map(|&c| c as u8).collect();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn formats_doubles_shortest() {
        assert_eq!(fmt(0.0), "0");
        assert_eq!(fmt(-0.0), "-0");
        assert_eq!(fmt(100.0), "100");
        assert_eq!(fmt(123.456), "123.456");
        assert_eq!(fmt(-0.001), "-0.001");
        assert_eq!(fmt(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(fmt(1e16), "10000000000000000");
        assert_eq!(fmt(1e300), "1e+300");
        assert_eq!(fmt(f64::NAN), "nan");
        assert_eq!(fmt(f64::NEG_INFINITY), "-inf");
    }
}
