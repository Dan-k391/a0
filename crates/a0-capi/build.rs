//! Tells the C host test which target it builds for and which system libraries a C program must
//! link along with this crate's static library: what `rustc --print native-static-libs` reports for
//! the standard library on this target, plus what the native backend needs on Windows.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target = env::var("TARGET").unwrap();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let empty = out.join("empty.rs");
    fs::write(&empty, "").unwrap();
    let output = Command::new(rustc)
        .args([
            "--crate-type",
            "staticlib",
            "--print",
            "native-static-libs",
            "--target",
        ])
        .arg(&target)
        .arg("--out-dir")
        .arg(&out)
        .arg(&empty)
        .output()
        .expect("cannot run rustc");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let libs = stderr
        .lines()
        .find_map(|l| {
            l.split_once("native-static-libs:")
                .map(|(_, libs)| libs.trim())
        })
        .unwrap_or("");
    // The native backend finds the MSVC linker through COM and the registry.
    let extra = if target.contains("msvc") {
        " ole32.lib oleaut32.lib advapi32.lib"
    } else {
        ""
    };
    println!("cargo:rustc-env=A0_CAPI_TARGET={target}");
    println!("cargo:rustc-env=A0_CAPI_NATIVE_LIBS={libs}{extra}");
    println!("cargo:rerun-if-changed=build.rs");
}
