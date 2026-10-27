#!/usr/bin/env python3
"""Makes A0's release archive for the host (docs/releases.md).

    python3 release/package.py [--packs DIR] [--out DIR] [--skip-build]

The archive, `a0-<version>-<platform>.tar.gz` (`.zip` on Windows), holds everything needed to use
A0 without building it:

    bin/        a0, b0, minnow, lox, kaleidoscope, tin, and rust-lld for linking other targets
    targets/    the target packs (python3 targets/build.py), found by the tools beside bin/
    lib/        the C API: a0_capi as a static and a shared library
    include/    a0.h (the C API) and a0rt.h (for languages' C runtimes)
    python/     the Python package `a0`, with the shared library inside it
    editors/    the VS Code extension
    docs/       the book and the reference, README, CHANGELOG, LICENSE

with a SHA256SUMS file beside it. It also writes, under `--out`, the packages to publish: the
crates (`cargo package`), a Python wheel when the `build` module is installed, and the VS Code
extension when `vsce` is. Nothing is published.
"""

import argparse
import hashlib
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOLS = ["a0", "b0", "minnow", "lox", "kaleidoscope", "tin"]
# The package each tool is built from, where it is not the tool's name (D88).
PACKAGE = {"a0": "a0-cli"}
# The crates of the workspace that are published, dependencies first (docs/releases.md).
CRATES = [
    "a0-diag", "a0-target", "a0-runtime", "a0-ir", "a0-syntax", "a0-sema", "a0-hir",
    "a0-interp", "a0-native", "a0-wasm", "a0-build", "a0-driver", "a0-capi", "a0-cli", "b0",
]


def run(cmd, **kw):
    print("+", " ".join(str(c) for c in cmd), file=sys.stderr)
    return subprocess.run(cmd, check=True, **kw)


def version():
    for line in (ROOT / "Cargo.toml").read_text().splitlines():
        if line.startswith("version = "):
            return line.split('"')[1]
    raise SystemExit("no version in Cargo.toml")


def host_platform():
    """A0's name for the host (`x86_64-linux`), as `--target` takes them."""
    machine = platform.machine().lower()
    arch = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(machine)
    os_name = {"linux": "linux", "darwin": "macos", "win32": "windows"}.get(sys.platform)
    if arch is None or os_name is None:
        raise SystemExit(f"no release for {machine} {sys.platform}")
    return f"{arch}-{os_name}"


def rust_lld():
    sysroot = run(["rustc", "--print", "sysroot"], capture_output=True, text=True).stdout.strip()
    host = run(["rustc", "--print", "host-tuple"], capture_output=True, text=True).stdout.strip()
    exe = ".exe" if sys.platform == "win32" else ""
    lld = Path(sysroot) / "lib" / "rustlib" / host / "bin" / f"rust-lld{exe}"
    return lld if lld.exists() else None


def library_names():
    if sys.platform == "win32":
        return "a0_capi.lib", "a0_capi.dll"
    if sys.platform == "darwin":
        return "liba0_capi.a", "liba0_capi.dylib"
    return "liba0_capi.a", "liba0_capi.so"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--packs", default=str(ROOT / "target" / "packs"),
                        help="the target packs to include (python3 targets/build.py)")
    parser.add_argument("--out", default=str(ROOT / "target" / "dist"))
    parser.add_argument("--skip-build", action="store_true", help="use what target/release has")
    parser.add_argument("--allow-dirty", action="store_true",
                        help="package the crates from a checkout with uncommitted changes")
    args = parser.parse_args()
    ver, plat = version(), host_platform()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    name = f"a0-{ver}-{plat}"
    stage = out / name
    if stage.exists():
        shutil.rmtree(stage)

    if not args.skip_build:
        packages = sum((["-p", PACKAGE.get(t, t)] for t in TOOLS + ["a0-capi"]), [])
        run(["cargo", "build", "--release", "--locked", *packages], cwd=ROOT)
    release = ROOT / "target" / "release"
    exe = ".exe" if sys.platform == "win32" else ""

    (stage / "bin").mkdir(parents=True)
    for tool in TOOLS:
        shutil.copy2(release / f"{tool}{exe}", stage / "bin" / f"{tool}{exe}")
    lld = rust_lld()
    if lld:
        shutil.copy2(lld, stage / "bin" / lld.name)
    else:
        print("warning: no rust-lld in the Rust toolchain; cross-compiling will need one", file=sys.stderr)

    packs = Path(args.packs)
    if packs.is_dir() and any(packs.iterdir()):
        shutil.copytree(packs, stage / "targets")
    else:
        print(f"warning: no target packs in {packs}; the archive cannot cross-compile", file=sys.stderr)

    static, shared = library_names()
    (stage / "lib").mkdir()
    for lib in (static, shared):
        candidates = [release / lib, release / "deps" / lib]
        found = next((c for c in candidates if c.exists()), None)
        if found is None:
            raise SystemExit(f"{lib} was not built")
        shutil.copy2(found, stage / "lib" / lib)
    if sys.platform == "win32" and (release / "a0_capi.dll.lib").exists():
        shutil.copy2(release / "a0_capi.dll.lib", stage / "lib" / "a0_capi.dll.lib")
    (stage / "include").mkdir()
    shutil.copy2(ROOT / "crates" / "a0-capi" / "include" / "a0.h", stage / "include")
    shutil.copy2(ROOT / "crates" / "a0-runtime" / "include" / "a0rt.h", stage / "include")

    python = stage / "python"
    shutil.copytree(ROOT / "bindings" / "python", python,
                    ignore=shutil.ignore_patterns("__pycache__", "tests"))
    (python / "a0" / "lib").mkdir()
    shutil.copy2(stage / "lib" / shared, python / "a0" / "lib" / shared)

    shutil.copytree(ROOT / "editors" / "vscode", stage / "editors" / "vscode",
                    ignore=shutil.ignore_patterns("node_modules"))
    shutil.copytree(ROOT / "docs", stage / "docs")
    for f in ["README.md", "CHANGELOG.md", "LICENSE"]:
        shutil.copy2(ROOT / f, stage / f)

    if sys.platform == "win32":
        archive = out / f"{name}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
            for p in sorted(stage.rglob("*")):
                z.write(p, p.relative_to(out))
    else:
        archive = out / f"{name}.tar.gz"
        with tarfile.open(archive, "w:gz") as t:
            t.add(stage, arcname=name)
    (out / f"{archive.name}.sha256").write_text(f"{sha256(archive)}  {archive.name}\n")
    print(archive)

    # The packages to publish, made but not published.
    crates = sum((["-p", c] for c in CRATES), [])
    dirty = ["--allow-dirty"] if args.allow_dirty else []
    run(["cargo", "package", "--locked", "--no-verify", *dirty, *crates], cwd=ROOT)
    for crate in (ROOT / "target" / "package").glob(f"*-{ver}.crate"):
        shutil.copy2(crate, out / crate.name)
    try:
        import build  # noqa: F401
        run([sys.executable, "-m", "build", "--wheel", "--outdir", str(out), str(python)])
    except ImportError:
        print("note: no Python wheel (pip install build to make one)", file=sys.stderr)
    if shutil.which("vsce"):
        run(["vsce", "package", "--out", str(out)], cwd=ROOT / "editors" / "vscode")
    else:
        print("note: no VS Code package (npm install -g @vscode/vsce to make one)", file=sys.stderr)


if __name__ == "__main__":
    main()
