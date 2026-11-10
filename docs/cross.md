# Building for other platforms

`build --target` makes an executable for another platform than the one you are on. For example, on
Linux or macOS, this makes a Windows executable of a Minnow program:

```
minnow build --target x86_64-windows game.mn -o game.exe
```

Every language built on A0 has the same option, and so does the `a0` command for IR and HIR files:

```
a0 build --target aarch64-linux prog.hir -o prog
```

## What you need

Two things, both included in the [release archive](https://github.com/Dan-k391/a0/releases/latest):

- **A target pack** for the platform. A target pack is a directory of libraries prebuilt for one
  platform: A0's runtime, a C library, and the C runtimes of A0's example languages. The archive
  has them in `targets/`.
- **lld**, LLVM's linker, which links the executable against the pack. It comes with Rust as
  `rust-lld`, and the archive has it in `bin/`.

With the release archive unpacked, `build --target` works from any platform, with nothing else to
install. If you built A0 from source, lld is usually found in your Rust toolchain, but you have to
build the packs yourself ([Building packs](#building-packs)).

Without the pack, the build stops and says so:

```
$ minnow build --target x86_64-windows -o hello.exe
error[native.link]: cannot link for x86_64-pc-windows-msvc: no target pack x86_64-pc-windows-msvc is installed (targets/build.py builds the packs into target/packs; A0_TARGETS names another directory)
```

## Targets

`--target` takes a short name from this table, or a full target triple such as
`x86_64-unknown-linux-gnu`.

| Target | Pack | C library | The executable runs on |
|---|---|---|---|
| `x86_64-linux` | `x86_64-unknown-linux-musl` | musl, linked statically | any x86-64 Linux |
| `aarch64-linux` | `aarch64-unknown-linux-musl` | musl, linked statically | any aarch64 Linux |
| `x86_64-windows` | `x86_64-pc-windows-msvc` | the Universal C Runtime (`ucrtbase.dll`) | Windows 10 and later |
| `aarch64-windows` | none yet | | build it on Windows on ARM64 |
| `x86_64-macos`, `aarch64-macos` | none: Apple's SDK may not be redistributed | | build it on a Mac |

Linux executables are linked statically, so they run on any distribution.

The targets without a pack say why when you try them:

```
$ minnow build --target aarch64-macos
error[native.link]: cannot link for aarch64-apple-darwin: executables for macOS need Apple's SDK, which may not be redistributed: build on a Mac
$ minnow build --target aarch64-windows
error[native.link]: cannot link for aarch64-pc-windows-msvc: there is no target pack for it yet
```

Building for the platform you are on does not use a pack, even with `--target`. The system's
linker and C library link the executable, as they do without `--target`. Set `A0_LINK=pack` to use
your own platform's pack instead; CI does this to test a pack.

`a0 obj --target <target>` writes an object file for any of the targets above, and needs no pack.

## Where A0 looks for packs and lld

A pack is a directory named after the pack (the second column above) with a file `PACK` in it. A0
uses the first one it finds in:

1. the directory `A0_TARGETS` names;
2. `targets/` next to the running executable, then `../targets` from it (the release archive's
   layout, where the executables are in `bin/`);
3. `target/packs` in the source tree A0 was built from, where `targets/build.py` puts them.

A0 uses the first lld it finds:

1. the program `A0_LLD` names;
2. `lld` or `rust-lld` next to the running executable;
3. `rust-lld` in the Rust toolchain (under `rustc --print sysroot`);
4. `ld.lld` (for Linux) or `lld-link` (for Windows) on `PATH`, or `lld` there.

## Languages with a C runtime of their own

A language can link a C library of its own into its programs (its C runtime). To build for another
platform, that library must be in the pack too, built for that platform. The packs have the
example languages' runtimes: `kaleidoscope_rt` and `tin_rt`. Their file names follow the
platform: `libkaleidoscope_rt.a` on Linux, `kaleidoscope_rt.lib` on Windows. The name is the one the
language's `Library` gives.

If the library is missing from the pack, the build fails with an error that names the file:
`the target pack has no <file>`. To add your language's runtime, add its C files to `LANGUAGE_RUNTIMES` in `targets/build.py` and
build the packs again.

## Building packs

The release archive has the packs. Build them yourself when you work from a clone of A0:

```
cargo build --workspace
python3 targets/build.py                                     # every pack, into target/packs
python3 targets/build.py x86_64-unknown-linux-musl --out /somewhere   # one pack, elsewhere
```

`targets/build.py` runs on Linux or macOS. It needs:

- clang, `llvm-ar`, `llvm-nm` and `llvm-dlltool`;
- network access: it downloads the sources of musl (1.2.5) and compiler-rt (from LLVM 18.1.8);
- for the Windows pack, mingw-w64's headers and import libraries, in the directory
  `MINGW_SYSROOT` names (by default `/usr/x86_64-w64-mingw32`, as Ubuntu installs them).

On Ubuntu, `apt install clang lld llvm mingw-w64` installs all of these.

A pack holds:

- A0's runtime, compiled for the target: `liba0rt.a` (`a0rt.lib` for Windows);
- the example languages' C runtimes, as above;
- `libclang_rt.builtins.a`: compiler-rt's builtins, the helper functions compiled code may call
  where the target has no instruction (128-bit arithmetic and the like);
- for Linux: musl's `libc.a` and its startup objects `crt1.o`, `crti.o` and `crtn.o`;
- for Windows: `start_windows.o`, A0's own entry point (from
  `crates/a0-runtime/c/start_windows.c`), which replaces the C library's startup code; mingw-w64's
  `libucrt.a`, `libkernel32.a` and `libpsapi.a`; and `libucrt_extra.a`, generated, which imports
  from `ucrtbase.dll` the C library functions that mingw-w64's import libraries leave out;
- the file `PACK`, which says what the pack is built from.

## Testing packs

With packs built, two tests use them:

- `cargo test -p a0-cli --test programs pack` runs the IR and HIR test programs through every
  installed pack. With your own platform's pack, every program is linked against it
  (`A0_LINK=pack`) and run. With other platforms' packs, every program is linked, and also run if
  `A0_CROSS_RUNNERS` names a program that runs that platform's executables, for example
  `A0_CROSS_RUNNERS=aarch64-linux=qemu-aarch64-static`.
- `cargo test -p tin --test tin pack` runs Tin's programs, which call cJSON and the C library,
  through your own platform's pack in the same way.

Without packs, these tests check nothing. CI's `packs` job builds the packs on Linux and runs both
tests there, aarch64 Linux executables under QEMU. The `packs-windows` job runs them again on
Windows with the same packs.
