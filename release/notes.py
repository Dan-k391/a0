#!/usr/bin/env python3
"""Prints the changelog's section for a version, for the release's notes.

    python3 release/notes.py 1.0.0
"""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    version = sys.argv[1]
    lines = (ROOT / "CHANGELOG.md").read_text().splitlines()
    out, inside = [], False
    for line in lines:
        if line.startswith("## ["):
            if inside:
                break
            inside = line.startswith(f"## [{version}]")
            continue
        if inside:
            out.append(line)
    if not out:
        raise SystemExit(f"no section for {version} in CHANGELOG.md")
    print("\n".join(out).strip())
    print("\nThe archives hold A0's tools, its target packs, the C API and the Python package for "
          "one platform each (docs/releases.md), each with a `.sha256` file beside it.")


if __name__ == "__main__":
    main()
