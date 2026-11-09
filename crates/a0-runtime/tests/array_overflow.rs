//! Array capacities must not wrap the byte count into a smaller allocation.

use std::process::Command;

unsafe extern "C" {
    fn a0_array_new(kind: i64, capacity: i64) -> *mut std::ffi::c_void;
}

#[test]
fn oversized_array_allocations() {
    const CASES: [(i64, i64); 4] = [
        (0, 1 << 61),
        (4, 1 << 62),
        (5, 1 << 62),
        (16 + (1 << 60), 16),
    ];
    if let Ok(case) = std::env::var("A0_TEST_ARRAY_OVERFLOW") {
        let (kind, capacity) = CASES[case.parse::<usize>().unwrap()];
        a0_runtime::init(&[]);
        unsafe { a0_array_new(kind, capacity) };
        return;
    }
    // Runtime failures end the process; keep each allocation in a child of its own.
    for case in 0..CASES.len() {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "oversized_array_allocations", "--nocapture"])
            .env("A0_TEST_ARRAY_OVERFLOW", case.to_string())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(101), "case {case}: {stderr}");
        assert!(
            stderr.contains("panic: array size is too large"),
            "case {case}: {stderr}"
        );
    }
}
