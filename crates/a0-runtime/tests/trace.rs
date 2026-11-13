//! Native frames find their functions in a code table whose functions are not in address order,
//! as the JIT's are not.

use std::ffi::c_void;

#[repr(C)]
struct CodeFunction {
    start: *const u8,
    size: u32,
    first: u32,
    count: u32,
    unused: u32,
}

#[repr(C)]
struct CodeRange {
    start: u32,
    end: u32,
    location: u32,
}

#[repr(C)]
struct CodeTable {
    count: u64,
    functions: *const CodeFunction,
    ranges: *const CodeRange,
}

unsafe extern "C" {
    fn a0_rt_set_debug(locations: *const c_void, code: *const CodeTable);
    fn a0_rt_add_code(code: *const CodeTable);
    fn a0_rt_frames(loc: u64, frame: *mut c_void, out: *mut u32, max: usize) -> usize;
}

/// The locations of a chain of frames whose return addresses are `ras`, innermost first.
fn frames(ras: &[usize]) -> Vec<u32> {
    // Each frame is the caller's frame pointer and the return address; the stack grows down, so
    // callers are at higher addresses. The last frame's return address is in no function.
    let mut stack = vec![0usize; 2 * ras.len() + 2];
    let base = stack.as_mut_ptr();
    for (i, &ra) in ras.iter().enumerate() {
        stack[2 * i] = unsafe { base.add(2 * i + 2) } as usize;
        stack[2 * i + 1] = ra;
    }
    let mut out = [0u32; 16];
    let n = unsafe { a0_rt_frames(99, base as *mut c_void, out.as_mut_ptr(), out.len()) };
    out[..n].to_vec()
}

#[test]
fn return_addresses_find_their_functions_out_of_order() {
    a0_runtime::init(&[]);
    let code = [0u8; 400];
    let at = |offset: usize| code.as_ptr().wrapping_add(offset);
    let function = |start: usize, size: u32, first: u32| CodeFunction {
        start: at(start),
        size,
        first,
        count: 1,
        unused: 0,
    };
    // Three functions of 100 bytes, listed out of address order, and an empty one among them.
    let functions = [
        function(200, 100, 0),
        function(0, 100, 1),
        function(50, 0, 0),
        function(100, 100, 2),
    ];
    let range = |location: u32| CodeRange {
        start: 0,
        end: 100,
        location,
    };
    let ranges = [range(1), range(2), range(3)];
    let table = CodeTable {
        count: functions.len() as u64,
        functions: functions.as_ptr(),
        ranges: ranges.as_ptr(),
    };
    // A second table, as a program compiled in parallel has, after the first.
    let other_functions = [function(300, 100, 0)];
    let other_ranges = [range(4)];
    let other = CodeTable {
        count: 1,
        functions: other_functions.as_ptr(),
        ranges: other_ranges.as_ptr(),
    };
    unsafe {
        a0_rt_set_debug(std::ptr::null(), &table);
        a0_rt_add_code(&other);
    }
    let addr = |offset: usize| at(offset) as usize;
    assert_eq!(
        frames(&[addr(250), addr(10), addr(150), addr(350)]),
        [99, 1, 2, 3, 4]
    );
    // A return address at a function's end is the end of a call in it; at its start, it is not
    // (100 and 300 are in the functions that end there); the empty function holds nothing.
    assert_eq!(
        frames(&[addr(100), addr(300), addr(60), addr(1)]),
        [99, 2, 1, 2, 2]
    );
    // An address in no function ends the walk.
    assert_eq!(frames(&[addr(0), addr(10)]), [99]);
    assert_eq!(frames(&[addr(150), addr(401)]), [99, 3]);
    // Registering again replaces the tables.
    unsafe { a0_rt_set_debug(std::ptr::null(), &other) };
    assert_eq!(frames(&[addr(350), addr(150)]), [99, 4]);
}
