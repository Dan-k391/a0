#!/usr/bin/env python3
"""Builds A0's target packs: what linking an executable for another target needs besides lld.

    python3 targets/build.py [--out target/packs] [triple ...]

A pack is a directory, `<out>/<triple>/`, of static libraries and objects built for the target:
A0's runtime (`a0rt`), the example languages' C runtimes, compiler-rt's builtins, and a C library
whose license lets it be shipped (D79):

- Linux targets link statically against musl (MIT), built here from source.
- Windows links against the Universal C Runtime that comes with Windows, through mingw-w64's import
  libraries (public domain); the runtime is compiled against mingw-w64's headers, and a small entry
  point of A0's own (`crates/a0-runtime/c/start_windows.c`) replaces the C library's startup code.

macOS has no pack: Apple's SDK may not be redistributed.

Needs clang and llvm-ar (any recent version: clang targets everything), network access for the
musl and compiler-rt sources, and for Windows the mingw-w64 headers and libraries
(MINGW_SYSROOT, by default /usr/x86_64-w64-mingw32, from Ubuntu's mingw-w64-x86-64-dev). Runs on
Linux and macOS.
"""

import argparse
import glob
import os
import shutil
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MUSL = "1.2.5"
MUSL_URL = f"https://musl.libc.org/releases/musl-{MUSL}.tar.gz"
LLVM = "18.1.8"
COMPILER_RT_URL = (
    f"https://github.com/llvm/llvm-project/releases/download/llvmorg-{LLVM}/"
    f"compiler-rt-{LLVM}.src.tar.xz"
)

LINUX = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"]
WINDOWS = ["x86_64-pc-windows-msvc"]

# C library functions programs may call directly (extern declarations), besides what the runtimes
# call: on Windows each is imported from ucrtbase.dll when mingw-w64's libraries lack it (they leave
# much of the C library, mathematics included, to a library of mingw's own).
C_LIBRARY = """
sin cos tan asin acos atan atan2 sinh cosh tanh exp log log10 log2 pow sqrt cbrt floor ceil round
trunc fabs fmod hypot sinf cosf sqrtf powf floorf ceilf roundf fabsf
putchar getchar puts malloc calloc realloc free memcpy memmove memset memcmp memchr
strlen strcmp strncmp strcpy strncpy strcat strchr strrchr strstr strtol strtoul strtoll strtod
atoi atol atof abs labs llabs div ldiv lldiv getenv qsort bsearch rand srand toupper tolower
isalpha isdigit isalnum isspace isupper islower ispunct isxdigit time clock exit abort
""".split()

# The example languages' C runtimes, by library name (as their build.rs names them).
LANGUAGE_RUNTIMES = {
    "kaleidoscope_rt": ["examples/kaleidoscope/runtime/kaleidoscope.c"],
    "tin_rt": [
        "examples/tin/runtime/tin.c",
        "examples/tin/runtime/abi.c",
        "examples/tin/runtime/cjson/cJSON.c",
    ],
}


def log(message):
    print(f"targets: {message}", file=sys.stderr, flush=True)


def run(cmd, **kw):
    subprocess.run([str(c) for c in cmd], check=True, **kw)


def tool(name, env):
    return os.environ.get(env) or shutil.which(name) or sys.exit(f"targets: {name} is missing")


def fetch(url, work):
    name = url.rsplit("/", 1)[1]
    path = work / name
    if not path.exists():
        log(f"fetching {url}")
        with urllib.request.urlopen(url) as r, open(path, "wb") as f:
            shutil.copyfileobj(r, f)
    out = work / name.split(".tar")[0]
    if not out.exists():
        with tarfile.open(path) as t:
            t.extractall(work, filter="data")
    return out


def clang_target(triple):
    """The triple clang knows a target by."""
    return {
        "x86_64-unknown-linux-musl": "x86_64-linux-musl",
        "aarch64-unknown-linux-musl": "aarch64-linux-musl",
        "x86_64-pc-windows-msvc": "x86_64-w64-mingw32",
    }[triple]


def archive(ar, lib, objects):
    if lib.exists():
        lib.unlink()
    run([ar, "rcs", lib, *objects])


def compile_all(clang, target, flags, sources, objdir):
    objdir.mkdir(parents=True, exist_ok=True)
    objects = []
    for src in sources:
        obj = objdir / (Path(src).stem + ".o")
        run([clang, f"--target={target}", "-O2", *flags, "-c", src, "-o", obj])
        objects.append(obj)
    return objects


def build_musl(clang, ar, triple, work, sysroot):
    if (sysroot / "lib" / "libc.a").exists():
        return
    src = fetch(MUSL_URL, work)
    build = work / f"musl-build-{triple}"
    shutil.rmtree(build, ignore_errors=True)
    shutil.copytree(src, build)
    arch = triple.split("-")[0]
    log(f"building musl for {triple}")
    env = dict(os.environ, CC=f"{clang} --target={clang_target(triple)}", AR=ar,
               RANLIB=shutil.which("llvm-ranlib") or f"{ar} s")
    run(["./configure", f"--prefix={sysroot}", f"--target={arch}", "--disable-shared"],
        cwd=build, env=env)
    run(["make", f"-j{os.cpu_count() or 2}"], cwd=build, env=env)
    run(["make", "install"], cwd=build, env=env)


def build_builtins(clang, ar, triple, work, pack, sysroot_flags):
    """compiler-rt's builtins (Apache-2.0 WITH LLVM-exception): what clang's code may call that the
    target has no instruction for (128-bit long doubles on AArch64, stack probes on Windows)."""
    lib = pack / "libclang_rt.builtins.a"
    if lib.exists():
        return
    src = fetch(COMPILER_RT_URL, work) / "lib" / "builtins"
    target = clang_target(triple)
    arch = triple.split("-")[0]
    objdir = work / f"builtins-{triple}"
    objdir.mkdir(parents=True, exist_ok=True)
    log(f"building compiler-rt builtins for {triple}")
    sources = sorted(glob.glob(str(src / "*.c")))
    if "windows" in triple:
        sources += sorted(glob.glob(str(src / arch / "chkstk*.S")))
    if arch == "aarch64":
        sources += [str(src / "cpu_model" / "aarch64.c")] if (src / "cpu_model" / "aarch64.c").exists() else []
    objects = []
    for s in sources:
        obj = objdir / (Path(s).name + ".o")
        # Not every builtin builds for every target (some need an OS feature or another
        # architecture); those are left out.
        r = subprocess.run([clang, f"--target={target}", "-O2", "-fno-builtin", *sysroot_flags,
                            f"-I{src}", "-c", s, "-o", str(obj)], capture_output=True)
        if r.returncode == 0:
            objects.append(obj)
    archive(ar, lib, objects)


def ffi_call_source():
    """The runtime's generated `a0_ffi_call` (a0-runtime/build.rs), from the host's build."""
    run(["cargo", "build", "-q", "-p", "a0-runtime"], cwd=ROOT)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    found = sorted(glob.glob(str(target / "debug" / "build" / "a0-runtime-*" / "out" / "ffi_call.c")),
                   key=os.path.getmtime)
    if not found:
        sys.exit("targets: cannot find the runtime's generated ffi_call.c")
    return found[-1]


def build_pack(triple, out, work, clang, ar, mingw):
    pack = out / triple
    pack.mkdir(parents=True, exist_ok=True)
    target = clang_target(triple)
    if triple in LINUX:
        sysroot = work / f"musl-{triple}"
        build_musl(clang, ar, triple, work, sysroot)
        flags = ["-nostdinc", f"-isystem{sysroot / 'include'}",
                 f"-isystem{Path(subprocess.check_output([clang, '-print-resource-dir'], text=True).strip()) / 'include'}"]
        for f in ["libc.a", "crt1.o", "crti.o", "crtn.o"]:
            shutil.copy(sysroot / "lib" / f, pack / f)
    else:
        flags = [f"--sysroot={mingw}", f"-isystem{mingw / 'include'}", "-D__USE_MINGW_ANSI_STDIO=0",
                 "-D_UCRT"]
        for f in ["libucrt.a", "libkernel32.a", "libpsapi.a"]:
            shutil.copy(mingw / "lib" / f, pack / f)
    build_builtins(clang, ar, triple, work, pack, flags)

    log(f"building the runtime for {triple}")
    runtime = sorted(glob.glob(str(ROOT / "crates" / "a0-runtime" / "c" / "*.c")))
    runtime = [s for s in runtime if not s.endswith("start_windows.c")] + [ffi_call_source()]
    include = ["-I", ROOT / "crates" / "a0-runtime" / "include"]
    objects = compile_all(clang, target, [*flags, *include], runtime, work / f"a0rt-{triple}")
    lib_name = "a0rt.lib" if "windows" in triple else "liba0rt.a"
    archive(ar, pack / lib_name, objects)
    if "windows" in triple:
        compile_all(clang, target, [*flags, *include],
                    [ROOT / "crates" / "a0-runtime" / "c" / "start_windows.c"], pack)
    for name, sources in LANGUAGE_RUNTIMES.items():
        objects = compile_all(clang, target, [*flags, *include], [ROOT / s for s in sources],
                              work / f"{name}-{triple}")
        archive(ar, pack / (f"{name}.lib" if "windows" in triple else f"lib{name}.a"), objects)
    if "windows" in triple:
        ucrt_imports(pack, ar)
    (pack / "PACK").write_text(
        f"A0 target pack for {triple}: musl {MUSL} or mingw-w64 import libraries, compiler-rt "
        f"{LLVM} builtins, A0's runtime and its example languages' runtimes.\n")
    log(f"built {pack}")


def symbols(nm, path, kind):
    """The symbols `path` defines (`kind` "defined") or needs ("undefined")."""
    flag = "--defined-only" if kind == "defined" else "--undefined-only"
    out = subprocess.run([nm, flag, "--format=just-symbols", str(path)], capture_output=True,
                         text=True).stdout
    return {s.strip() for s in out.splitlines() if s.strip() and not s.endswith(":")}


def ucrt_imports(pack, ar):
    """An import library for the C library functions the pack's code and programs may call that
    mingw-w64's import libraries lack, all from ucrtbase.dll."""
    nm = tool("llvm-nm", "LLVM_NM")
    dlltool = tool("llvm-dlltool", "LLVM_DLLTOOL")
    provided = set()
    needed = set(C_LIBRARY)
    for f in pack.iterdir():
        if f.suffix not in (".a", ".lib", ".o"):
            continue
        provided |= symbols(nm, f, "defined")
        if f.name not in ("libucrt.a", "libkernel32.a", "libpsapi.a", "libclang_rt.builtins.a"):
            needed |= symbols(nm, f, "undefined")
    # (An `__imp_` pointer alone, which some of mingw-w64's libraries have, is not a function to
    # call.)
    missing = sorted(s for s in needed if s not in provided and not s.startswith("__imp_")
                     and not s.startswith(".") and s != "main")
    (pack / "ucrt_extra.def").write_text("LIBRARY ucrtbase.dll\nEXPORTS\n" +
                                         "".join(f"  {s}\n" for s in missing))
    run([dlltool, "-m", "i386:x86-64", "-D", "ucrtbase.dll", "-d", pack / "ucrt_extra.def",
         "-l", pack / "libucrt_extra.a"])
    log(f"imported {len(missing)} more functions from ucrtbase.dll")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default=str(ROOT / "target" / "packs"))
    parser.add_argument("triples", nargs="*", default=LINUX + WINDOWS)
    args = parser.parse_args()
    out = Path(args.out).resolve()
    work = out.parent / "packs-work"
    work.mkdir(parents=True, exist_ok=True)
    clang = tool("clang", "CLANG")
    ar = tool("llvm-ar", "LLVM_AR")
    mingw = Path(os.environ.get("MINGW_SYSROOT", "/usr/x86_64-w64-mingw32"))
    for triple in args.triples:
        if triple not in LINUX + WINDOWS:
            sys.exit(f"targets: no pack for {triple} (macOS's SDK may not be redistributed)")
        build_pack(triple, out, work, clang, ar, mingw)


if __name__ == "__main__":
    main()
