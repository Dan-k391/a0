#!/usr/bin/env python3
"""Prints bench/history.csv as a Markdown table: each measure by release, for releases measured on
one machine (`docs/benchmarks.md`, "From release to release").

    python3 bench/history.py [--machine TEXT]
"""

import argparse
import csv
from pathlib import Path

HISTORY = Path(__file__).resolve().parent / "history.csv"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--machine", help="only rows measured on a machine whose name contains this")
    args = parser.parse_args()
    rows = list(csv.DictReader(HISTORY.open()))
    machines = sorted({r["machine"] for r in rows})
    machine = next((m for m in machines if args.machine is None or args.machine in m), None)
    rows = [r for r in rows if r["machine"] == machine]
    versions = list(dict.fromkeys(r["version"] for r in rows))
    measures = list(dict.fromkeys(r["measure"] for r in rows))
    value = {(r["version"], r["measure"]): r["ms"] for r in rows}
    print(f"On {machine}, in milliseconds:\n")
    print("| | " + " | ".join(versions) + " |")
    print("|---|" + "---|" * len(versions))
    for m in measures:
        print(f"| {m} | " + " | ".join(value.get((v, m), "") for v in versions) + " |")


if __name__ == "__main__":
    main()
