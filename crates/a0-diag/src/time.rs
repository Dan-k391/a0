//! How long each stage of compiling takes, printed to stderr when `A0_TIME` is set (`A0_TIME=1
//! minnow build`): one line per stage, in the order they end.

use std::sync::OnceLock;
use std::time::Instant;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("A0_TIME").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// Runs `f`, printing how long it took as `stage` when timing is on.
pub fn stage<T>(name: &str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let out = f();
    eprintln!(
        "time: {:>9.1} ms  {name}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    out
}
