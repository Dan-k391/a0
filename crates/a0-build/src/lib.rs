//! Build-script helpers for language crates that ship their own C runtime library.
//!
//! A language built on A0 usually needs a little C of its own (Kaleidoscope's `putchard`, Lox's
//! object model). Call [`runtime_library`] from the crate's `build.rs`: it compiles the sources against
//! `a0rt.h`, links the library into the crate (so the interpreter can call it), and keeps a copy at
//! `OUT_DIR/<name>.embedded` for the native linker:
//!
//! ```ignore
//! // build.rs
//! fn main() {
//!     a0_build::runtime_library("mylang_rt", &["runtime/mylang.c"]);
//! }
//! // src/lib.rs
//! pub fn runtime_library() -> a0_native::Library {
//!     a0_native::Library {
//!         name: "mylang_rt".into(),
//!         bytes: include_bytes!(concat!(env!("OUT_DIR"), "/mylang_rt.embedded")).to_vec(),
//!     }
//! }
//! ```
//!
//! The crate must depend on `a0-runtime` directly, which provides the header path through Cargo's
//! `DEP_A0RT_INCLUDE` variable.

use std::env;
use std::fs;
use std::path::PathBuf;

/// Compiles `sources` (paths relative to the crate) into the static library `name`.
pub fn runtime_library(name: &str, sources: &[&str]) {
    let include = env::var("DEP_A0RT_INCLUDE")
        .expect("DEP_A0RT_INCLUDE is not set: add a0-runtime as a direct dependency of this crate");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut build = cc::Build::new();
    build.include(&include).warnings(true);
    for src in sources {
        let path = manifest.join(src);
        println!("cargo:rerun-if-changed={}", path.display());
        build.file(path);
    }
    build.compile(name);
    let lib = [format!("{name}.lib"), format!("lib{name}.a")]
        .iter()
        .map(|n| out.join(n))
        .find(|p| p.exists())
        .unwrap_or_else(|| panic!("cc did not produce the {name} static library"));
    fs::copy(&lib, out.join(format!("{name}.embedded"))).unwrap();
}
