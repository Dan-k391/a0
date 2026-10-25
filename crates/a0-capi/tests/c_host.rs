//! Compiles `tests/host.c` with the system C compiler, links it with this crate's static library,
//! runs it, and runs the executable it built: the C API works from a real C program on every
//! platform CI covers.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `target/<profile>`; the test runs from its `deps` directory.
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().to_path_buf()
}

/// The static library cargo built along with this test: the newest `a0_capi.lib` or
/// `liba0_capi.a` in `deps` (`cargo test` does not copy it up to `target/<profile>`). Cargo leaves
/// the hash out of the name because the crate is also a cdylib, but older builds may have one.
fn static_library(msvc: bool) -> PathBuf {
    let (prefix, suffix) = if msvc {
        ("a0_capi", ".lib")
    } else {
        ("liba0_capi", ".a")
    };
    let deps = profile_dir().join("deps");
    std::fs::read_dir(&deps)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(suffix))
                .is_some_and(|hash| hash.is_empty() || hash.starts_with('-'))
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
        .unwrap_or_else(|| panic!("no {prefix}*{suffix} in {}", deps.display()))
}

fn compile_host(work: &Path) -> PathBuf {
    let target = env!("A0_CAPI_TARGET");
    let msvc = target.contains("msvc");
    let lib = static_library(msvc);
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let include = manifest.join("include");
    let source = manifest.join("tests").join("host.c");
    let host = work.join(if msvc { "host.exe" } else { "host" });
    let compiler = cc::Build::new()
        .target(target)
        .host(target)
        .opt_level(0)
        .cargo_metadata(false)
        .get_compiler();
    let mut cmd = compiler.to_command();
    cmd.current_dir(work);
    let libs = env!("A0_CAPI_NATIVE_LIBS").split_whitespace();
    if msvc {
        // The Rust standard library links the dynamic C runtime, so the host must too.
        cmd.args(["/nologo", "/MD"])
            .arg(format!("/I{}", include.display()))
            .arg(&source)
            .arg(format!("/Fe{}", host.display()))
            .arg("/link")
            .arg(&lib)
            .args(libs);
    } else {
        cmd.arg("-I")
            .arg(&include)
            .arg(&source)
            .arg(&lib)
            .args(libs)
            .arg("-o")
            .arg(&host);
    }
    let out = cmd.output().expect("cannot run the C compiler");
    assert!(
        out.status.success(),
        "compiling host.c failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    host
}

#[test]
fn a_c_program_builds_runs_and_compiles_modules() {
    let work = profile_dir().join("capi-host");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let host = compile_host(&work);

    let out = Command::new(&host).current_dir(&work).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "host failed:\n{stdout}\n{stderr}");
    let exe = if cfg!(windows) {
        "fib-native.exe"
    } else {
        "fib-native"
    };
    let expected = format!(
        "a0 {version}\n\
         text: has @fib\n\
         6765\n55\n\
         run: 0, status 7\n\
         object: fib-aarch64-linux.o\n\
         executable: {exe}\n\
         misuse: 2 problems, first: 'no_such_op' is not an instruction\n\
         calc: 1 + 2 * (10 - 3) = 15\n\
         calc: 6 * 7 = 42\n\
         calc: 1 +: error at 3: expected a number\n\
         hello, C from HIR\n\
         vars:\n7\n21\n\
         vars error at 17: [E1001] unknown name `b`\n\
         2\n42\n\
         modules: lib is a module: 1\n42\n\
         modules: `hidden` is private to module `lib`\n\
         hir misuse: 2 problems, first: 99999 is not a valid expression handle\n\
         failures: 0\n",
        version = env!("CARGO_PKG_VERSION"),
    );
    assert_eq!(stdout, expected, "stderr:\n{stderr}");

    // What the C program compiled: an object file for another target and a host executable.
    let object = std::fs::metadata(work.join("fib-aarch64-linux.o")).unwrap();
    assert!(object.len() > 0);
    let native = Command::new(work.join(exe)).output().unwrap();
    assert_eq!(native.status.code(), Some(7));
    assert_eq!(
        String::from_utf8_lossy(&native.stdout).replace("\r\n", "\n"),
        "6765\n55\n"
    );
}
