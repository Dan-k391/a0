"""Tests of the Python bindings and of Pebble, the example language written with them. Run by
`crates/a0-capi/tests/python.rs` (as part of `cargo test`), or directly:

    A0_LIBRARY=target/debug/a0_capi.dll python -m unittest discover -s bindings/python/tests
"""

import os
import re
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "bindings", "python"))

import a0  # noqa: E402

PEBBLE = os.path.join(ROOT, "bindings", "python", "examples", "pebble")
EXE = ".exe" if sys.platform == "win32" else ""


def header_functions(prefix, receiver):
    """The builder functions in a0.h, in the notation of `a0.HIR_FUNCTIONS`."""
    with open(os.path.join(ROOT, "crates", "a0-capi", "include", "a0.h"), encoding="utf-8") as f:
        header = re.sub(r"/\*.*?\*/", "", f.read(), flags=re.S)
    handles = {"uint32_t", "A0HirType", "A0HirExpr", "A0HirLocal", "A0HirBody", "A0HirFunc",
               "A0HirStruct", "A0HirEnum", "A0HirGlobal", "A0HirExtern", "A0HirArm", "A0Value",
               "A0Func", "A0Var", "A0Global", "A0Type"}
    out = {}
    for decl in re.findall(r"^([A-Za-z][^;{}#]*?\ba0_\w+\s*\([^;]*?\));", header, re.M | re.S):
        decl = " ".join(decl.split())
        result, name, params = re.match(r"(.*?)\b(a0_\w+)\s*\((.*)\)$", decl).groups()
        params = [p.strip() for p in params.split(",")]
        if not name.startswith(prefix) or not params[0].startswith(receiver):
            continue
        if result.strip().endswith("*") or name.endswith("_free"):
            continue
        types = [re.sub(r"\s*\b\w+$", "", p).strip() for p in params[1:]]
        spec, i = "", 0
        while i < len(types):
            t, rest = types[i], types[i + 1:]
            if t == "const char *" and rest[:1] == ["size_t"]:
                spec, i = spec + "s", i + 2
            elif t == "const A0Str *" and len(rest) > 1 and rest[1] == "size_t":
                spec, i = spec + "P", i + 3
            elif t == "const A0Str *":
                spec, i = spec + "N", i + 2
            elif t == "const uint8_t *":
                spec, i = spec + "B", i + 2
            elif t == "const uint32_t *" or (t.startswith("const A0") and t.endswith("*")):
                spec, i = spec + "U", i + 2
            else:
                spec += {"uint32_t *": "o", "A0Diagnostics **": "D", "int64_t": "i",
                         "double": "f", "int": "b"}.get(t, "u" if t in handles else "?" + t)
                i += 1
        result = result.strip()
        spec += ":" + ("v" if result == "void" else "u" if result in handles else "i")
        out[name[len(prefix):]] = spec
    return out


def expected_stdout(path):
    with open(path, encoding="utf-8") as f:
        return "".join(line[len("// expect-stdout: "):] for line in f
                       if line.startswith("// expect-stdout: "))


def pebble(*args, env=None):
    return subprocess.run([sys.executable, os.path.join(PEBBLE, "pebble.py"), *args],
                          capture_output=True, text=True, env={**os.environ, **(env or {})})


class Bindings(unittest.TestCase):
    def test_the_tables_match_the_header(self):
        self.assertEqual(a0.HIR_FUNCTIONS, header_functions("a0_hir_", "A0HirBuilder *"))
        self.assertEqual(a0.IR_FUNCTIONS, header_functions("a0_builder_", "A0Builder *"))

    def test_the_versions_match(self):
        # The package, the library and the header are released together.
        self.assertEqual(a0.__version__, a0.version())
        header = open(os.path.join(ROOT, "crates", "a0-capi", "include", "a0.h")).read()
        parts = [re.search(rf"#define A0_VERSION_{p} (\d+)", header).group(1)
                 for p in ("MAJOR", "MINOR", "PATCH")]
        self.assertEqual(".".join(parts), a0.__version__)

    def test_grammars_and_trees(self):
        with self.assertRaises(a0.Error) as e:
            a0.Grammar("program = missing;")
        self.assertIn("'missing' is not a rule or token", str(e.exception))
        g = a0.Grammar('skip / +/; token N /[0-9]+/; program = item*; item = "add" a:N b:N;')
        tree = g.parse("add 1 22 add 3 4")
        items = tree.root.nodes
        self.assertEqual([i.kind for i in items], ["item", "item"])
        self.assertEqual(items[0].field("b").text, "22")
        self.assertEqual(items[0].field("b").span, (6, 8))
        self.assertEqual([c.kind for c in items[1].children], ['"add"', "N", "N"])
        self.assertIsNone(items[0].field("c"))
        with self.assertRaises(a0.Error) as e:
            g.parse("add 1")
        self.assertEqual(e.exception.diagnostics[0].span, (5, 5))

    def test_building_running_and_errors(self):
        b = a0.HirBuilder("demo")
        main = b.declare_function("main", [])
        b.set_signature(main, ([], []), b.named_type("int", []))
        body = b.function_body(main)
        b.begin_body(body)
        let = b.let_named("x", b.fresh_type(), b.int(40))
        b.set_root(body, b.block([let], b.binary("+", b.name("x"), b.int(2))))
        b.end_body()
        hir = b.finish()
        self.assertIn("fn main() -> int", str(hir))
        self.assertEqual(hir.lower().run(["demo"]), 42)
        self.assertEqual(hir.lower().run_jit(["demo"]), 42)
        self.assertEqual(a0.Hir.parse(str(hir)).lower().run([]), 42)
        # Problems are reported with codes and spans, and render against the source.
        source = "print(y)"
        b = a0.HirBuilder("broken")
        main = b.declare_function("main", [])
        b.set_signature(main, ([], []), b.named_type("unit", []))
        body = b.function_body(main)
        b.begin_body(body)
        b.set_span(6, 7)
        b.set_root(body, b.name("y"))
        b.end_body()
        with self.assertRaises(a0.Error) as e:
            b.finish()
        d = e.exception.diagnostics[0]
        self.assertEqual((d.code, d.span), ("E1001", (6, 7)))
        self.assertIn("--> broken.x:1:7", e.exception.render("broken.x", source))
        with self.assertRaises(ValueError):
            b.finish()

    def test_core_ir(self):
        b = a0.Builder("ir")
        main = b.declare_function("main", [], a0.I32)
        b.begin_function(main)
        b.return_(b.binary("iadd", b.iconst(a0.I32, 2), b.iconst(a0.I32, 3)))
        b.end_function()
        module = b.finish()
        self.assertEqual(module.run([]), 5)
        self.assertEqual(a0.Module.parse(str(module)).run([]), 5)

    def test_c_structs(self):
        # lldiv returns a struct of two int64_t: 100 / 7 is 14, remainder 2.
        b = a0.Builder("structs")
        lldiv = b.declare_extern("lldiv", [a0.PTR, a0.I64, a0.I64], a0.VOID)
        b.set_struct_param(lldiv, 0, "sret", 16, 8, [a0.I64, a0.I64], [0, 8])
        main = b.declare_function("main", [], a0.I32)
        b.begin_function(main)
        r = b.stack_slot(16, 8)
        b.call(lldiv, [r, b.iconst(a0.I64, 100), b.iconst(a0.I64, 7)])
        quot = b.load(a0.I64, r)
        rem = b.load(a0.I64, b.ptradd(r, b.iconst(a0.I64, 8)))
        both = b.binary("iadd", b.binary("imul", quot, b.iconst(a0.I64, 10)), rem)
        b.return_(b.cast("trunc", both, a0.I32))
        b.end_function()
        module = b.finish()
        self.assertIn("ptr sret(16, 8: i64 0, i64 8)", str(module))
        self.assertEqual(module.run([]), 142)
        self.assertEqual(module.run_jit([]), 142)


class Pebble(unittest.TestCase):
    def programs(self):
        folder = os.path.join(PEBBLE, "programs")
        return [os.path.join(folder, p) for p in sorted(os.listdir(folder)) if p.endswith(".pb")]

    def test_programs_run_interpreted_and_in_the_jit(self):
        for program in self.programs():
            for env in ({"A0_INTERP": "1"}, {}, {"A0_GC_STRESS": "1"}, {"A0_OPT": "0"},
                        {"A0_OPT": "0", "A0_INTERP": "1"}):
                with self.subTest(program=os.path.basename(program), env=env):
                    out = pebble("run", program, env=env)
                    self.assertEqual(out.returncode, 0, out.stderr)
                    self.assertEqual(out.stdout, expected_stdout(program))

    def test_programs_run_natively(self):
        with tempfile.TemporaryDirectory() as tmp:
            for program in self.programs():
                with self.subTest(program=os.path.basename(program)):
                    exe = os.path.join(tmp, os.path.basename(program)[:-3])
                    build = pebble("build", program, "-o", exe)
                    self.assertEqual(build.returncode, 0, build.stderr)
                    for stress in ("0", "1"):
                        out = subprocess.run([exe + EXE], capture_output=True, text=True,
                                             env={**os.environ, "A0_GC_STRESS": stress})
                        self.assertEqual(out.returncode, 0, out.stderr)
                        self.assertEqual(out.stdout, expected_stdout(program))

    def test_errors_and_panics(self):
        with tempfile.TemporaryDirectory() as tmp:
            broken = os.path.join(tmp, "broken.pb")
            with open(broken, "w", encoding="utf-8") as f:
                f.write("fn main() {\n  print(fib(3));\n  let s = 1 + \"a\";\n}\n")
            out = pebble("run", broken)
            self.assertEqual(out.returncode, 1)
            self.assertIn("error[E1001]: unknown function `fib`", out.stderr)
            self.assertIn("broken.pb:2:9", out.stderr)
            self.assertIn("error[E2001]: mismatched types", out.stderr)
            # A panic stops the program the same way interpreted, in the JIT and native.
            panics = os.path.join(tmp, "panics.pb")
            with open(panics, "w", encoding="utf-8") as f:
                f.write("fn main() {\n  let xs = [1];\n  print(\"before\");\n  print(xs[3]);\n}\n")
            interpreted = pebble("run", panics, env={"A0_INTERP": "1"})
            jit = pebble("run", panics)
            exe = os.path.join(tmp, "panics")
            self.assertEqual(pebble("build", panics, "-o", exe).returncode, 0)
            native = subprocess.run([exe + EXE], capture_output=True, text=True)
            for out in (interpreted, jit, native):
                self.assertEqual(out.returncode, a0.PANIC_EXIT_CODE)
                self.assertEqual(out.stdout, "before\n")
                self.assertIn("panic: index out of bounds", out.stderr)

    def test_repl(self):
        session = ("let x = 20;\nfn twice(n: int) -> int {\n  return n * 2;\n}\n"
                   "print(twice(x) + 2);\nprint(nope);\nprint(\"done\");\n")
        out = subprocess.run([sys.executable, os.path.join(PEBBLE, "pebble.py"), "repl"],
                             input=session, capture_output=True, text=True)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(out.stdout, "42\ndone\n")
        self.assertIn("unknown name `nope`", out.stderr)


if __name__ == "__main__":
    unittest.main()
