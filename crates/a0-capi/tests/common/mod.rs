//! Finding the libraries cargo built along with a test (`cargo test` does not copy them up to
//! `target/<profile>`).

use std::path::PathBuf;

/// The newest file named `prefix<hash>suffix` (`hash` empty or `-…`) that cargo built along with
/// this test. Cargo up to 1.99 puts both in `target/<profile>/deps`; from 1.100 each unit has its
/// own `target/<profile>/build/a0-capi/<hash>/out`, so the library is in a sibling of the test's.
/// Cargo leaves the hash out of a library's name when the crate is also a cdylib, but older builds
/// may have one.
pub fn built_library(prefix: &str, suffix: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let here = exe.parent().unwrap();
    let siblings = std::fs::read_dir(here.parent().unwrap().parent().unwrap())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path().join("out"));
    std::iter::once(here.to_path_buf())
        .chain(siblings)
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(suffix))
                .is_some_and(|hash| hash.is_empty() || hash.starts_with('-'))
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
        .unwrap_or_else(|| panic!("no {prefix}*{suffix} near {}", here.display()))
}

/// The C API's shared library, which the Python tests load.
#[allow(dead_code)] // c_host links the static library instead
pub fn shared_library() -> PathBuf {
    if cfg!(windows) {
        built_library("a0_capi", ".dll")
    } else if cfg!(target_os = "macos") {
        built_library("liba0_capi", ".dylib")
    } else {
        built_library("liba0_capi", ".so")
    }
}
