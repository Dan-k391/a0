# Cross-compilation

`build --target` makes an executable for another platform, from any host, with nothing else to
install: A0 links it with lld against a *target pack*, a directory of prebuilt libraries (D79).

```
a0 build --target aarch64-linux prog.hir -o prog          # static Linux executable, any distribution
minnow build --target x86_64-windows game.mn -o game.exe   # every language driver too
```

| Target | Pack | C library | Runs on |
|---|---|---|---|
| `x86_64-linux` | `x86_64-unknown-linux-musl` | musl, linked statically | any x86-64 Linux |
| `aarch64-linux` | `aarch64-unknown-linux-musl` | musl, linked statically | any aarch64 Linux |
| `x86_64-windows` | `x86_64-pc-windows-msvc` | the Universal C Runtime (`ucrtbase.dll`) | Windows 10 and later |
| `aarch64-windows` | none yet | | build on Windows ARM64 |
| `x86_64-macos`, `aarch64-macos` | none: Apple's SDK may not be redistributed | | build on a Mac |

Building for the host does not use a pack: the system's linker and C library link it, as before.
`A0_LINK=pack` uses the host's pack anyway, which is how CI checks a pack.

## Where A0 looks

Packs, the first directory with `<dir>/<pack>/PACK` winning:

1. `A0_TARGETS`;
2. `targets/` next to the `a0` executable, then `../targets` (a release's layout);
3. `target/packs` in the source tree A0 was built from (a developer's build).

lld: `A0_LLD`; `lld` or `rust-lld` next to `a0`; `rust-lld` in the Rust toolchain's sysroot
(`rustc --print sysroot`); `ld.lld` or `lld-link` on `PATH`.

## Building packs

```
cargo build --workspace
python3 targets/build.py                 # all packs into target/packs
python3 targets/build.py x86_64-unknown-linux-musl --out /somewhere
```

`targets/build.py` runs on Linux or macOS and needs clang, `llvm-ar`, `llvm-nm`, `llvm-dlltool`,
network access (it downloads musl and compiler-rt's sources, pinned versions) and, for Windows,
mingw-w64's headers and import libraries (`MINGW_SYSROOT`, by default Ubuntu's
`/usr/x86_64-w64-mingw32`). On Ubuntu: `apt install clang lld llvm mingw-w64`.

A pack holds:

- `liba0rt.a` (`a0rt.lib` on Windows): A0's runtime, compiled for the target;
- the example languages' C runtimes (`libkaleidoscope_rt.a`, `libtin_rt.a`), named as each
  language's `Library` names them, so their drivers cross-compile too;
- `libclang_rt.builtins.a`: compiler-rt's builtins (128-bit arithmetic and the like);
- Linux: musl's `libc.a` and startup objects (`crt1.o`, `crti.o`, `crtn.o`);
- Windows: `start_windows.o` (the entry point, `crates/a0-runtime/c/start_windows.c`), mingw-w64's
  `libucrt.a`, `libkernel32.a`, `libpsapi.a`, and `libucrt_extra.a`, generated, importing from
  `ucrtbase.dll` what mingw-w64 leaves to its own static library.

A language with a C runtime of its own adds it to `LANGUAGE_RUNTIMES` in `targets/build.py`.

## Testing

`cargo test -p a0 --test programs pack` runs the IR and HIR corpus through every installed pack:
the host's pack runs every program (`A0_LINK=pack`), and every other pack links every program, and
runs it when `A0_CROSS_RUNNERS` names a runner for its target
(`A0_CROSS_RUNNERS=aarch64-linux=qemu-aarch64-static`). Tin's tests (`cargo test -p tin --test tin pack`) run its programs, which call cJSON and the C
library, through the host's pack the same way. Without packs these tests check nothing. CI's
`packs` job builds the packs on Linux and runs the tests there and, with the same packs, on Windows.
