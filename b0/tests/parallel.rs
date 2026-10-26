//! Threads run at the same time: `tests/programs/parallel.b0` counts primes with one thread and
//! with as many as the machine has (up to four), interpreted, in the JIT and natively, and must be
//! clearly faster with more. (On a machine with one processor there is nothing to measure.)

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn program() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/programs/parallel.b0")
}

/// The output of `cmd` and the best of three runs' wall times.
fn timed(mut make: impl FnMut() -> Command) -> (String, Duration) {
    let mut best = Duration::MAX;
    let mut out = String::new();
    for _ in 0..3 {
        let start = Instant::now();
        let output = make().output().unwrap();
        let took = start.elapsed();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        out = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
        best = best.min(took);
    }
    (out, best)
}

/// The count a run printed, without the number of threads.
fn count(out: &str) -> &str {
    out.split(" (").next().unwrap_or(out)
}

fn check(mode: &str, threads: usize, work: u64, run: &mut dyn FnMut(usize, u64) -> Command) {
    // What every run costs whatever its work (starting, and in `run` compiling), which no number
    // of threads shortens: a run that counts almost nothing measures it.
    let (_, fixed) = timed(|| run(1, 1000));
    let (one, t1) = timed(|| run(1, work));
    let (many, tn) = timed(|| run(threads, work));
    assert_eq!(count(&one), count(&many), "{mode}");
    // Twice the threads should take well under the time; leave room for a busy machine.
    let counting = |t: Duration| t.saturating_sub(fixed).as_secs_f64();
    let ratio = counting(tn) / counting(t1);
    let limit = if threads == 2 { 0.85 } else { 0.75 };
    eprintln!("{mode}: 1 thread {t1:?}, {threads} threads {tn:?}, fixed {fixed:?} ({ratio:.2})");
    assert!(
        ratio < limit,
        "{mode}: {threads} threads took {tn:?}, one took {t1:?}, of which {fixed:?} is fixed ({ratio:.2} of it)"
    );
}

#[test]
fn more_threads_count_faster_in_every_mode() {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    if cores < 2 {
        eprintln!("one processor: nothing to measure");
        return;
    }
    let threads = cores.min(4);
    let b0 = env!("CARGO_BIN_EXE_b0");
    let file = program();

    let mut interpreted = |n: usize, work: u64| {
        let mut cmd = Command::new(b0);
        cmd.arg("run")
            .arg(&file)
            .arg(n.to_string())
            .arg(work.to_string());
        cmd.env("A0_INTERP", "1");
        cmd
    };
    check("interpreted", threads, 400000, &mut interpreted);

    let mut jit = |n: usize, work: u64| {
        let mut cmd = Command::new(b0);
        cmd.arg("run")
            .arg(&file)
            .arg(n.to_string())
            .arg(work.to_string());
        cmd
    };
    check("JIT", threads, 4000000, &mut jit);

    let dir = std::env::temp_dir().join(format!("b0-parallel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(format!("parallel{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(b0)
        .arg("build")
        .arg(&file)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let mut native = |n: usize, work: u64| {
        let mut cmd = Command::new(&exe);
        cmd.arg(n.to_string()).arg(work.to_string());
        cmd
    };
    check("native", threads, 3000000, &mut native);
    std::fs::remove_dir_all(&dir).ok();
}
