#!/usr/bin/env python3
"""Checks a release archive (docs/releases.md): unpacked anywhere, its tools run a program and
cross-compile one with nothing else installed than what the archive holds.

    python3 release/check.py target/dist
"""

import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

PROGRAM = """fn main() {
  let xs = map(range(1, 5), fn(x: int) -> int { return x * x; });
  print(fold(xs, 0, fn(a: int, b: int) -> int { return a + b; }));
}
"""


def main():
    dist = Path(sys.argv[1])
    archives = sorted(dist.glob("a0-*.tar.gz")) + sorted(dist.glob("a0-*.zip"))
    if not archives:
        raise SystemExit(f"no archive in {dist}")
    archive = archives[0]
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        if archive.suffix == ".zip":
            zipfile.ZipFile(archive).extractall(tmp)
        else:
            tarfile.open(archive).extractall(tmp, filter="data")
        top = next(tmp.iterdir())
        exe = ".exe" if os.name == "nt" else ""
        b0 = top / "bin" / f"b0{exe}"
        if os.name != "nt":
            for f in (top / "bin").iterdir():
                f.chmod(0o755)
        prog = tmp / "squares.b0"
        prog.write_text(PROGRAM)
        # Without the checkout: the archive's own tools, packs and lld.
        env = {k: v for k, v in os.environ.items() if k not in ("A0_TARGETS", "A0_LLD")}
        out = subprocess.run([str(b0), "run", str(prog)], capture_output=True, text=True, env=env)
        if out.stdout.strip() != "30":
            raise SystemExit(f"b0 run printed {out.stdout!r} {out.stderr}")
        targets = [p.name for p in (top / "targets").iterdir()] if (top / "targets").is_dir() else []
        # A target other than the host's, so that the archive's rust-lld links it, not the host's
        # C compiler.
        x86_64_linux = sys.platform == "linux" and platform.machine().lower() in ("x86_64", "amd64")
        target = "aarch64-linux" if x86_64_linux else "x86_64-linux"
        pack = "aarch64-unknown-linux-musl" if x86_64_linux else "x86_64-unknown-linux-musl"
        if pack in targets:
            exe_out = tmp / "squares-linux"
            built = subprocess.run([str(b0), "build", str(prog), "--target", target, "-o",
                                    str(exe_out)], capture_output=True, text=True, env=env)
            if built.returncode != 0 or not exe_out.exists():
                raise SystemExit(f"cross-compiling failed: {built.stderr}")
        else:
            print("note: no packs in the archive; cross-compiling not checked")
        print(f"{archive.name}: ok")


if __name__ == "__main__":
    main()
