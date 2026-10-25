# json

A small tool for JSON files, written in Minnow: v0.2's test of A0 on a real program. It reads
JSON (RFC 8259, with line and column in its errors), writes it back pretty or compact, picks values
by path, and counts what a file holds.

```
minnow run json/main.mn pretty data.json        # indented
minnow run json/main.mn compact data.json       # on one line
minnow run json/main.mn get .items[0].name data.json
minnow run json/main.mn stats data.json         # values of each kind, nesting depth
echo '{"a": [1, 2]}' | minnow run json/main.mn get .a[1]
minnow build json/main.mn -o json               # a native executable
```

| File | |
|---|---|
| `main.mn` | the command line: arguments, files or standard input, errors (exit 1) |
| `value.mn` | `Value`, an enum, and `Object`, which keeps its keys in order (a map and a key list) |
| `reader.mn` | a recursive descent reader, a struct with methods, returning `std.Result` and using `?` |
| `writer.mn` | compact and pretty writing, with escapes |
| `query.mn` | paths: `.name` and `[index]` |

It uses modules, `pub`, the standard library (`std.fs`, `std.io`, `std.env`, `std.str`,
`std.math`), results with `?`, methods, enums with `match`, maps and closures-free recursion.
`tests/json.rs` runs it on `tests/json` interpreted, natively, unoptimized and under GC stress.

## What writing it changed in A0

- The optimizer never inlined a function ending in `return x;`, which is every Minnow function:
  it took a body of type `never` for one that never returns. Small methods like `peek` are now
  inlined.
- `named_type` did not know the HIR's own `array` and `map`, so Minnow could not name a map type.
- Lengths, `byte_at` and array elements were runtime calls; they are inline now.
- Printing numbers was slow (up to 17 tries of `snprintf` and `strtod` per number): 6x faster.
- The collector spent most of its time sweeping a list of every object; it sweeps memory in order
  now. `A0_GC_STATS=1` shows what it does.

On a 5 MB file (40 000 records), the native tool reads it in 0.30 s and writes it back in 0.29 s
(before these fixes, 0.60 s and 0.64 s). Most of what remains is the collector marking the growing
tree: a non-generational collector marks everything that is live at every collection.
