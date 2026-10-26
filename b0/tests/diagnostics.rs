//! Every program in `tests/diagnostics` prints exactly what its `.out` file says, and the same
//! natively (`A0_BLESS=1` rewrites the files).

use std::path::Path;

#[test]
fn diagnostics_are_what_the_snapshots_show() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/diagnostics");
    let binary = Path::new(env!("CARGO_BIN_EXE_b0"));
    let failures = a0_driver::snapshots::check(binary, &dir, "b0", true);
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}
