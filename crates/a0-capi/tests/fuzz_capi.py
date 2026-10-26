"""Fuzzes A0's C API (D84): random sequences of calls, read from include/a0.h, must never crash.

The calls keep the header's contract (objects are NULL or live, arrays as long as their counts) and
break everything else: handles of other kinds, of other objects, out of range or A0_NONE; unknown
names and codes; strings that are empty, long or not UTF-8; calls in any order. Errors are fine;
a crash (a panic aborts across the C boundary) is a failure.

Run by tests/fuzz.rs: `python3 fuzz_capi.py <header> <first seed> <count> <log>`, with A0_LIBRARY
naming the shared library. Each sequence's calls are written to <log> as they are made, so that the
last sequence in it is the one that crashed.
"""

import ctypes
import os
import random
import re
import sys
import tempfile
from ctypes import (POINTER, byref, c_char_p, c_double, c_int, c_int32, c_int64, c_size_t,
                    c_uint8, c_uint32, c_void_p)

OBJECTS = {"A0Diagnostics", "A0Builder", "A0Module", "A0Grammar", "A0Tree", "A0Hir",
           "A0HirBuilder", "A0Language"}
# Functions that free or take over their object argument.
CONSUMES = {"a0_builder_finish": 0, "a0_hir_finish": 0, "a0_hir_builder_from": 0}
# Functions that run programs, link or take over the process: covered by other tests.
SKIP = {"a0_run", "a0_jit_run", "a0_build_executable", "a0_language_main",
        "a0_language_set_compiler"}

WORDS = [
    "", "int", "float", "bool", "string", "any", "unit", "object", "array", "map", "never", "i8",
    "u8", "i32", "u32", "f32", "ptr", "x", "y", "main", "f", "T", "self", "len", "push", "pop", "has",
    "remove", "keys", "filled", "field", "set_field", "offset", "data", "addr", "free", "substr",
    "byte_at", "str", "print", "eprint", "exit", "panic", "clock", "add", "sub", "mul", "sdiv",
    "srem", "and", "or", "xor", "shl", "eq", "ne", "slt", "sle", "ult", "oeq", "olt", "neg", "not",
    "+", "-", "*", "/", "%", "==", "!=", "<", "<=", ">", "&&", "||", "!", "sext", "zext", "trunc",
    "sitofp", "fptosi", "bitcast", "C", "a0", "gc", "manual", "variable", "function", "struct",
    "enum", "field", "method", "unused_variable", "unreachable_code", "E0308", "x86_64-linux",
    "aarch64-linux", "wasm32", "nonsense", "a.b", "std", "std/math", "fn main() {\n{}}\n",
    "module \"m\";\nfn main() -> int { 0 }\n",
]


def parse_header(path):
    """The functions of the header: (name, return type, [(type, name)])."""
    text = open(path, encoding="utf-8").read()
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    text = re.sub(r"#[^\n]*", " ", text)
    funcs = []
    for m in re.finditer(r"([A-Za-z_][\w\s\*]*?)\b(a0_\w+)\s*\(([^()]*)\)\s*;", text):
        ret, name, params = " ".join(m.group(1).split()), m.group(2), m.group(3)
        if ret.startswith("typedef"):
            continue
        args = []
        for p in params.split(","):
            p = " ".join(p.split())
            if p in ("", "void"):
                continue
            pm = re.match(r"(.*?)(\w+)$", p)
            args.append((pm.group(1).replace(" *", "*").strip(), pm.group(2)))
        funcs.append((name, ret.replace(" *", "*").strip(), args))
    return funcs


def handle_types(path):
    text = open(path, encoding="utf-8").read()
    return set(re.findall(r"typedef uint32_t (\w+);", text))


class Str(ctypes.Structure):
    _fields_ = [("ptr", c_char_p), ("len", c_size_t)]


class Fuzzer:
    def __init__(self, lib, funcs, handles, log):
        self.lib = lib
        self.funcs = [f for f in funcs if f[0] not in SKIP]
        self.handles = handles
        self.log = log
        self.keep = []  # buffers the library may still point at

    def base(self, t):
        return t.replace("const ", "").replace("*", "").strip()

    def text(self):
        r = self.r.random()
        if r < 0.75:
            s = self.r.choice(WORDS).encode()
        elif r < 0.85:
            s = bytes(self.r.randrange(256) for _ in range(self.r.randrange(12)))
        elif r < 0.95:
            s = (self.r.choice(WORDS) + "é中\U0001F600").encode()
        else:
            s = self.r.choice(WORDS).encode() * self.r.randrange(1, 200)
        buf = ctypes.create_string_buffer(s, len(s) + 1)
        self.keep.append(buf)
        return buf, len(s)

    def u32(self, kind=None):
        pool = self.pool.get(kind) if kind else None
        r = self.r.random()
        if pool and r < 0.7:
            return self.r.choice(pool)
        if r < 0.85:
            return self.r.randrange(8)
        if r < 0.92:
            return 0xFFFFFFFF
        return self.r.randrange(1 << 32)

    def scalar(self, t):
        if t in self.handles:
            return c_uint32(self.u32(t))
        if t in ("uint32_t", "A0Type"):
            return c_uint32(self.r.choice([0, 1, 2, 5, 8, 9, 64, 0xFFFFFFFF])
                            if t == "A0Type" else self.u32())
        if t == "int64_t":
            return c_int64(self.r.choice([0, 1, -1, 42, 2**63 - 1, -2**63]))
        if t == "double":
            return c_double(self.r.choice([0.0, -0.0, 1.5, float("nan"), float("inf"), 1e308]))
        if t in ("int", "int32_t"):
            return c_int(self.r.choice([0, 1, -1, 2**31 - 1]))
        if t == "uint8_t":
            return c_uint8(self.r.randrange(256))
        if t == "size_t":
            return c_size_t(self.r.choice([0, 1, 2, 3, 100, 2**32]))
        raise ValueError(f"no value for {t}")

    def counted(self, params, i):
        """Whether params[i] starts arrays followed by their count."""
        while i < len(params) and params[i][0].startswith("const") and params[i][0].endswith("*"):
            i += 1
        return i < len(params) and params[i][0] == "size_t"

    def objects(self, kind):
        return self.live.get(kind, [])

    def args_for(self, name, params):
        """Arguments for a call, or None when an object it needs is missing. Also returns the
        out-pointers to read afterwards: (kind, holder)."""
        args, outs, describe = [], [], []
        i = 0
        while i < len(params):
            t, pname = params[i]
            nxt = params[i + 1] if i + 1 < len(params) else None
            b = self.base(t)
            stars = t.count("*")
            if b in OBJECTS and stars == 1:
                pool = self.objects(b)
                if not pool:
                    return None
                obj = self.r.choice(pool) if self.r.random() < 0.97 else None
                args.append(c_void_p(obj))
                describe.append(f"{b}@{self.ids.get(obj, 'NULL')}")
            elif b in OBJECTS and stars == 2:
                holder = c_void_p()
                outs.append((b, holder))
                args.append(byref(holder) if self.r.random() < 0.9 else None)
                describe.append(f"&{b}")
            elif b == "char" and stars == 1 and nxt and nxt[0] == "size_t":
                if name == "a0_write_object" and pname == "path":
                    s = os.path.join(self.tmp, "out.o").encode()
                    buf = ctypes.create_string_buffer(s)
                    self.keep.append(buf)
                    args += [buf, c_size_t(len(s))]
                    describe.append("<temp path>")
                else:
                    buf, n = self.text()
                    args += [buf, c_size_t(n)]
                    describe.append(repr(buf.raw[:n][:40]))
                i += 2
                continue
            elif b == "char" and stars == 2 and nxt and nxt[0] == "size_t":
                n = self.r.randrange(3)
                arr = (c_char_p * max(n, 1))(*[self.text()[0].value for _ in range(n)])
                self.keep.append(arr)
                args += [arr, c_size_t(n)]
                describe.append(f"[{n} strings]")
                i += 2
                continue
            elif stars == 1 and t.startswith("const") and b != "char" and self.counted(params, i):
                # Arrays before their count, which they share: `names, types, count`.
                group = []
                while params[i][0] != "size_t":
                    group.append(self.base(params[i][0]))
                    i += 1
                n = self.r.choice([0, 0, 1, 1, 2, 3, 5])
                for b in group:
                    if b == "A0Str":
                        items = []
                        for _ in range(n):
                            buf, k = self.text()
                            items.append(Str(ctypes.cast(buf, c_char_p), k))
                        arr = (Str * max(n, 1))(*items)
                    else:
                        arr = (c_uint32 * max(n, 1))(*[self.scalar(b).value for _ in range(n)])
                    self.keep.append(arr)
                    args.append(arr if n or self.r.random() < 0.5 else None)
                    describe.append(f"[{n} {b}]")
                args.append(c_size_t(n))
                i += 1
                continue
            elif stars == 1 and b in ("size_t", "uint32_t", "int32_t", "int64_t", "int"):
                holder = {"size_t": c_size_t, "uint32_t": c_uint32, "int32_t": c_int32,
                          "int64_t": c_int64, "int": c_int}[b]()
                outs.append((None, holder))
                args.append(byref(holder) if self.r.random() < 0.95 else None)
                describe.append(f"&{b}")
            elif b == "void" and stars == 1:
                args.append(None)
                describe.append("NULL")
            else:
                v = self.scalar(b)
                args.append(v)
                describe.append(str(v.value))
            i += 1
        return args, outs, describe

    def call(self, name, ret, params):
        prepared = self.args_for(name, params)
        if prepared is None:
            return
        args, outs, describe = prepared
        self.log.write(f"{name}({', '.join(describe)})\n")
        self.log.flush()
        f = getattr(self.lib, name)
        rb = self.base(ret)
        if rb in OBJECTS or ret.endswith("*"):
            f.restype = c_void_p
        elif rb in ("int", "int32_t"):
            f.restype = c_int
        elif rb == "void":
            f.restype = None
        elif rb in ("size_t",):
            f.restype = c_size_t
        else:
            f.restype = c_uint32
        result = f(*args)
        # What the call took over is gone, whatever it returned.
        if name in CONSUMES or name.endswith("_free"):
            gone = args[CONSUMES.get(name, 0)].value
            if gone is not None:
                for pool in self.live.values():
                    if gone in pool:
                        pool.remove(gone)
        for kind, holder in outs:
            if kind is not None and holder.value:
                self.adopt(kind, holder.value)
        if rb in OBJECTS and result:
            self.adopt(rb, result)
        elif rb == "char" and result:
            # A borrowed string, with its length in the size_t out-parameter.
            sizes = [h for k, h in outs if k is None and isinstance(h, c_size_t)]
            if sizes:
                ctypes.string_at(result, sizes[-1].value)
        elif rb in self.handles and result != 0xFFFFFFFF:
            self.pool.setdefault(rb, []).append(result)

    def adopt(self, kind, ptr):
        self.ids[ptr] = len(self.ids)
        self.live.setdefault(kind, []).append(ptr)

    def sequence(self, seed, samples):
        self.r = random.Random(seed)
        self.live, self.pool, self.ids, self.keep = {}, {}, {}, []
        self.log.seek(0)
        self.log.truncate()
        self.log.write(f"seed {seed}\n")
        # Something to start from: builders, and modules and grammars parsed from real text.
        starts = [("a0_hir_builder_new", "A0HirBuilder*", [("const char*", "n"), ("size_t", "l")]),
                  ("a0_builder_new", "A0Builder*", [("const char*", "n"), ("size_t", "l")])]
        for name, ret, params in starts:
            self.call(name, ret, params)
        for kind, text in samples:
            buf = ctypes.create_string_buffer(text, len(text) + 1)
            self.keep.append(buf)
            out = c_void_p()
            parse = {"hir": "a0_hir_parse", "ir": "a0_module_parse", "grammar": "a0_grammar_new"}
            f = getattr(self.lib, parse[kind])
            f.restype = c_void_p
            p = f(buf, c_size_t(len(text)), byref(out))
            if out.value:
                self.adopt("A0Diagnostics", out.value)
            if p:
                self.adopt({"hir": "A0Hir", "ir": "A0Module", "grammar": "A0Grammar"}[kind], p)
        hir_calls = [f for f in self.funcs if f[2] and self.base(f[2][0][0]) == "A0HirBuilder"]
        for _ in range(self.r.randrange(20, 400)):
            # Mostly calls on a HIR builder, the largest part of the API.
            pick = hir_calls if self.r.random() < 0.6 else self.funcs
            self.call(*self.r.choice(pick))
        # Free what is left, as a host would.
        frees = {k: n for n, _, p in self.funcs
                 if n.endswith("_free") and len(p) == 1 for k in [self.base(p[0][0])]}
        for kind, pool in list(self.live.items()):
            for ptr in list(pool):
                if kind in frees:
                    self.call(frees[kind], "void", [(kind + "*", "x")])
                    if ptr in pool:  # the free picked another object of the kind
                        pass


def main():
    header, first, count, log_path = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
    lib = ctypes.CDLL(os.environ["A0_LIBRARY"], mode=getattr(ctypes, "RTLD_GLOBAL", 0))
    root = os.path.join(os.path.dirname(os.path.abspath(header)), "..", "..", "..")
    samples = []
    for kind, path in [("hir", "tests/hir/closures.hir"), ("hir", "tests/hir/generics.hir"),
                       ("ir", "tests/programs/basics.a0ir"),
                       ("grammar", "examples/calc/calc.a0g")]:
        full = os.path.join(root, path)
        if os.path.exists(full):
            samples.append((kind, open(full, "rb").read()))
    funcs = parse_header(header)
    assert len(funcs) > 150, f"only {len(funcs)} functions found in the header"
    with tempfile.TemporaryDirectory() as tmp, open(log_path, "w", encoding="utf-8") as log:
        fz = Fuzzer(lib, funcs, handle_types(header), log)
        fz.tmp = tmp
        for seed in range(first, first + count):
            fz.sequence(seed, samples)
        log.write("done\n")


if __name__ == "__main__":
    main()
