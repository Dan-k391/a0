#!/usr/bin/env python3
"""Generates a large Minnow package: the program A0's large-program measurements use.

    python3 bench/large/gen.py <dir> [--lines 100000]

The package has a main file and modules of about 500 lines each. Every module imports up to three
earlier ones and calls into them; its functions loop, branch, build strings and arrays, and use
structs of their own, so that every stage of the compiler has work to do. The program runs in a
fraction of a second and prints a checksum. The same arguments always give the same files.
"""

import argparse
from pathlib import Path

FUNCS_PER_MODULE = 36


def module(i, imports):
    out = []
    w = out.append
    w(f"// Module {i} of the generated package.")
    for j in imports:
        w(f"import m{j};")
    w("")
    w(f"pub struct Item{i} {{ id: int, weight: float, label: string, tags: [int] }}")
    w("")
    w(f"pub fn make{i}(id: int) -> Item{i} {{")
    w(f"  let tags: [int] = [];")
    w(f"  let k = 0;")
    w(f"  while k < 3 {{")
    w(f"    push(tags, id * {i % 7 + 2} + k);")
    w(f"    k = k + 1;")
    w(f"  }}")
    w(f"  return new Item{i} {{ id: id, weight: (id as float) * 0.5, label: \"item\" + str(id), tags: tags }};")
    w(f"}}")
    w("")
    for k in range(FUNCS_PER_MODULE):
        name = f"f{i}_{k}"
        w(f"pub fn {name}(x: int) -> int {{")
        w(f"  let total = x;")
        w(f"  let i = 0;")
        w(f"  while i < {k % 5 + 2} {{")
        w(f"    if (total + i) % {k % 3 + 2} == 0 {{")
        w(f"      total = total + i * {k + 1};")
        w(f"    }} else {{")
        w(f"      total = total - {i % 4 + 1};")
        w(f"    }}")
        w(f"    i = i + 1;")
        w(f"  }}")
        if k > 0:
            w(f"  total = total + f{i}_{k - 1}(x % 3);")
        elif imports:
            j = imports[0]
            w(f"  total = total + m{j}.f{j}_{FUNCS_PER_MODULE - 1}(x % 5);")
        w(f"  return total;")
        w(f"}}")
        w("")
    w(f"pub fn describe{i}(item: Item{i}) -> string {{")
    w(f"  let s = item.label + \":\";")
    w(f"  for t in item.tags {{")
    w(f"    s = s + \" \" + str(t);")
    w(f"  }}")
    w(f"  return s;")
    w(f"}}")
    w("")
    w(f"pub fn check{i}() -> int {{")
    w(f"  let item = make{i}({i});")
    w(f"  let sum = len(describe{i}(item)) + f{i}_{FUNCS_PER_MODULE - 1}({i % 11});")
    # One import's check, so that the calls reach every module once; the others are called into
    # (calling every import's check would make the calls grow exponentially with the modules).
    for n, j in enumerate(imports):
        if n == 0:
            w(f"  sum = sum + m{j}.check{j}() % 7;")
        else:
            w(f"  sum = sum + m{j}.f{j}_0({i}) % 7;")
    w(f"  return sum % 1000;")
    w(f"}}")
    return "\n".join(out) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("dir")
    parser.add_argument("--lines", type=int, default=100_000)
    args = parser.parse_args()
    root = Path(args.dir)
    (root / "src").mkdir(parents=True, exist_ok=True)
    (root / "minnow.toml").write_text('[package]\nname = "large"\nversion = "0.1.0"\n')
    total = 0
    i = 0
    count = 0
    while total < args.lines:
        imports = sorted({j for j in (i - 1, i // 2, i // 3) if 0 <= j < i})
        text = module(i, imports)
        (root / "src" / f"m{i}.mn").write_text(text)
        total += text.count("\n")
        i += 1
    count = i
    # The main file calls the last modules, which reach all the others.
    last = list(range(max(0, count - 4), count))
    main = [f"import m{j};" for j in last]
    main.append("")
    main.append("fn main() {")
    main.append("  let sum = 0;")
    for j in last:
        main.append(f"  sum = sum + m{j}.check{j}();")
    main.append("  print(sum);")
    main.append("}")
    (root / "src" / "main.mn").write_text("\n".join(main) + "\n")
    total += len(main)
    print(f"{count} modules, {total} lines")


if __name__ == "__main__":
    main()
