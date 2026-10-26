fn main() {
    a0_build::runtime_library(
        "tin_rt",
        &["runtime/tin.c", "runtime/abi.c", "runtime/cjson/cJSON.c"],
    );
}
