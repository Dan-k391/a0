//! In WebAssembly the interpreter runs on the main thread (there are no threads), so the engine is
//! linked with a stack (the part in linear memory) large enough for its deepest recursion.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("wasm") {
        println!("cargo:rustc-link-arg-bins=-zstack-size=16777216");
    }
}
