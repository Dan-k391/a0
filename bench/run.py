#!/usr/bin/env python3
"""A0's benchmark (docs/benchmarks.md): builds A0's languages and the references, checks that they
print the same, and times compiling and running each program, reporting medians.

    python3 bench/run.py [--runs N] [--out target/bench/results.md]

Needs, besides Rust: clang, llvm-config (LLVM's development files, for LLVM's own Kaleidoscope)
and Go. On Ubuntu: `sudo apt install clang llvm-dev zlib1g-dev libzstd-dev golang-go`. The
reference sources are fetched at pinned versions: clox from Crafting Interpreters, and the
Kaleidoscope tutorial's chapter 7 from the LLVM release that is installed. Linux and macOS.
"""

import argparse
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "bench"
WORK = ROOT / "target" / "bench"
CLOX_COMMIT = "4a840f70f69c6ddd17cfef4f6964f8e1bcd8c3d4"
CLOX_FILES = [
    "chunk.c", "chunk.h", "common.h", "compiler.c", "compiler.h", "debug.c", "debug.h", "main.c",
    "memory.c", "memory.h", "object.c", "object.h", "scanner.c", "scanner.h", "table.c", "table.h",
    "value.c", "value.h", "vm.c", "vm.h",
]
# The book's benchmarks but string_equality, which clox rejects (more than 256 constants in a
# chunk), and zoo_batch, which runs for a fixed time instead of a fixed amount of work.
LOX_PROGRAMS = ["fib", "binary_trees", "method_call", "invocation", "equality", "zoo"]
NATIVE = ["fib", "sieve", "nbody", "binarytrees"]
# The numbers `--record` keeps in bench/history.csv, by name, in milliseconds.
RECORD = {}
# Programs in Minnow, C and Go only (Tin has no closures).
MINNOW_ONLY = ["closures"]
# Each Kaleidoscope program's last line with the same calls doing no work, for LLVM's tutorial:
# its JIT compiles a function when a call first needs it, so the calls must stay.
TRIVIAL = {
    "fib": "printd(fib(1));",
    "mandel": "printd(mandelsum(0, 0, 0, 0.003, 0.0025));",
}


# LLVM's tutorial with its four function passes replaced by LLVM's standard -O2 pipeline, run on
# one module that holds the whole program: the tutorial gives each definition a module of its own,
# which would hide every callee from the inliner. The benchmark's programs end with their only
# top-level expression, so the program is optimized and compiled at once when it comes.
TOY_O2 = [
    # Definitions stay in the module of the whole program.
    ("""      ExitOnErr(TheJIT->addModule(
          ThreadSafeModule(std::move(TheModule), std::move(TheContext))));
      InitializeModuleAndManagers();
""", ""),
    ("""    TheFPM->run(*TheFunction, *TheFAM);
""", ""),
    ("""      auto TSM = ThreadSafeModule(std::move(TheModule), std::move(TheContext));""",
     """      {
        PassBuilder PB;
        LoopAnalysisManager LAM;
        FunctionAnalysisManager FAM;
        CGSCCAnalysisManager CGAM;
        ModuleAnalysisManager MAM;
        PB.registerModuleAnalyses(MAM);
        PB.registerCGSCCAnalyses(CGAM);
        PB.registerFunctionAnalyses(FAM);
        PB.registerLoopAnalyses(LAM);
        PB.crossRegisterProxies(LAM, FAM, CGAM, MAM);
        PB.buildPerModuleDefaultPipeline(OptimizationLevel::O2).run(*TheModule, MAM);
      }
      auto TSM = ThreadSafeModule(std::move(TheModule), std::move(TheContext));"""),
]


def toy_o2(source):
    """The tutorial's toy.cpp with LLVM's -O2 pipeline (TOY_O2)."""
    for old, new in TOY_O2:
        if old not in source:
            raise RuntimeError(f"toy.cpp has changed; cannot find:\n{old}")
        source = source.replace(old, new, 1)
    return source


def sh(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw)


def first_line(cmd):
    try:
        out = subprocess.run(cmd, capture_output=True, text=True)
        return (out.stdout or out.stderr).strip().splitlines()[0]
    except OSError:
        return None


def timed(cmd, runs, env=None, stdin=None, expect=None):
    """The median wall time of `runs` runs of `cmd`, and its output (checked against `expect`)."""
    times = []
    out = None
    for _ in range(runs):
        start = time.perf_counter()
        r = subprocess.run(cmd, capture_output=True, text=True, env={**os.environ, **(env or {})},
                           input=stdin)
        times.append(time.perf_counter() - start)
        if r.returncode != 0:
            raise RuntimeError(f"{cmd} failed:\n{r.stdout}{r.stderr}")
        out = r.stdout
    if expect is not None and out != expect:
        raise RuntimeError(f"{cmd} printed\n{out}\ninstead of\n{expect}")
    return statistics.median(times), out


def fetch(url, path):
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(url) as r:
            path.write_bytes(r.read())


def code_lines(paths, comment):
    """Lines that are neither blank nor only a comment."""
    n = 0
    block = False
    for p in paths:
        for line in Path(p).read_text().splitlines():
            s = line.strip()
            if block:
                block = "*/" not in s
                continue
            if not s or any(s.startswith(c) for c in comment):
                continue
            if s.startswith("/*"):
                block = "*/" not in s
                continue
            n += 1
    return n


def ms(seconds):
    return f"{seconds * 1000:.0f} ms" if seconds < 10 else f"{seconds:.1f} s"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--out", default=str(WORK / "results.md"))
    parser.add_argument("--record", metavar="VERSION",
                        help="append the headline numbers to bench/history.csv under this version")
    args = parser.parse_args()
    runs = args.runs
    WORK.mkdir(parents=True, exist_ok=True)
    for tool in ["clang", "clang++", "llvm-config", "go", "cargo"]:
        if shutil.which(tool) is None:
            sys.exit(f"bench: {tool} is missing (see the top of this file)")

    print("building A0's languages (release)", file=sys.stderr)
    sh(["cargo", "build", "--release", "--locked", "-p", "a0-cli", "-p", "minnow", "-p", "lox", "-p",
        "kaleidoscope", "-p", "tin"], cwd=ROOT)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "release"
    a0, minnow, lox, kaleidoscope, tin = (str(target / n) for n in ("a0", "minnow", "lox", "kaleidoscope", "tin"))

    # References.
    clox_dir = WORK / "clox"
    for f in CLOX_FILES:
        fetch(f"https://raw.githubusercontent.com/munificent/craftinginterpreters/{CLOX_COMMIT}/c/{f}",
              clox_dir / f)
    clox = str(WORK / "clox-bin")
    clox_flags = ["-O2", "-std=c99", "-w"]
    sh(["clang", *clox_flags, *[str(clox_dir / f) for f in CLOX_FILES if f.endswith(".c")], "-o", clox])
    llvm_version = sh(["llvm-config", "--version"]).stdout.strip()
    tutorial = f"https://raw.githubusercontent.com/llvm/llvm-project/llvmorg-{llvm_version}/llvm/examples/Kaleidoscope"
    toy_dir = WORK / f"kaleidoscope-llvm-{llvm_version}"
    fetch(f"{tutorial}/Chapter7/toy.cpp", toy_dir / "toy.cpp")
    fetch(f"{tutorial}/include/KaleidoscopeJIT.h", toy_dir / "include" / "KaleidoscopeJIT.h")
    toy = str(WORK / "kaleidoscope-llvm")
    llvm_flags = sh(["llvm-config", "--cxxflags", "--ldflags", "--system-libs", "--libs", "core",
                     "orcjit", "native"]).stdout.split()
    toy_build = ["clang++", "-O2", "-rdynamic", f"-I{toy_dir / 'include'}", str(toy_dir / "toy.cpp"),
                 *llvm_flags, "-o", toy]
    sh(toy_build)
    (toy_dir / "toy_o2.cpp").write_text(toy_o2((toy_dir / "toy.cpp").read_text()))
    toy2 = str(WORK / "kaleidoscope-llvm-O2")
    sh(["clang++", "-O2", "-rdynamic", f"-I{toy_dir / 'include'}", str(toy_dir / "toy_o2.cpp"),
        *llvm_flags, "-o", toy2])

    report = []
    w = report.append
    w("## Tools\n")
    w(f"- Machine: {platform.machine()}, {platform.platform()}, {os.cpu_count()} threads")
    try:
        cpu = [l for l in Path("/proc/cpuinfo").read_text().splitlines() if l.startswith("model name")]
        if cpu:
            w(f"- CPU: {cpu[0].split(':', 1)[1].strip()}")
    except OSError:
        pass
    git = sh(["git", "describe", "--always", "--dirty"], cwd=ROOT).stdout.strip()
    w(f"- A0: {git}, {first_line(['rustc', '--version'])}, Cranelift 0.137, release build")
    w(f"- {first_line(['clang', '--version'])}")
    w(f"- `cc`, which A0 links with: {first_line(['cc', '--version'])}")
    w(f"- LLVM {llvm_version} (the Kaleidoscope tutorial, chapter 7, built with `clang++ -O2`; and the "
      f"same with LLVM's -O2 pipeline on the whole program)")
    w(f"- {first_line(['go', 'version'])}")
    w(f"- clox at craftinginterpreters {CLOX_COMMIT[:12]}, built with `clang {' '.join(clox_flags)}`")
    w(f"- Times are medians of {runs} runs (wall clock); compile times include linking.\n")

    # Kaleidoscope: A0 against LLVM's tutorial.
    w("## Kaleidoscope: A0 against LLVM's tutorial\n")
    a0_k = [ROOT / "examples/kaleidoscope/kaleidoscope.a0g", *sorted((ROOT / "examples/kaleidoscope/src").glob("*.rs")),
            *sorted((ROOT / "examples/kaleidoscope/runtime").glob("*.c")), ROOT / "examples/kaleidoscope/build.rs"]
    loc_a0 = code_lines(a0_k, ["//", "#"])
    loc_llvm = code_lines([toy_dir / "toy.cpp", toy_dir / "include" / "KaleidoscopeJIT.h"], ["//"])
    w(f"Lines of code (without blank lines and comments): A0's Kaleidoscope {loc_a0} (grammar, "
      f"compiler, driver, runtime); LLVM's chapter 7 {loc_llvm} (`toy.cpp` and `KaleidoscopeJIT.h`).\n")
    w("| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`run`) | A0 interpreted | LLVM JIT: startup and compile | LLVM JIT: run | LLVM -O2: startup and compile | LLVM -O2: run |")
    w("|---|---|---|---|---|---|---|---|---|---|---|")
    for prog in ["fib", "mandel"]:
        src = BENCH / "kaleidoscope" / f"{prog}.k"
        exe = str(WORK / f"k-{prog}")
        build, _ = timed([kaleidoscope, "build", str(src), "-o", exe], runs)
        build0, _ = timed([kaleidoscope, "build", str(src), "-o", exe + "-noopt"], runs, env={"A0_OPT": "0"})
        run, out = timed([exe], runs)
        RECORD[f"kaleidoscope {prog}, native"] = run
        run0, _ = timed([exe + "-noopt"], runs, expect=out)
        jit, _ = timed([kaleidoscope, "run", str(src)], runs, expect=out)
        interp, _ = timed([kaleidoscope, "run", str(src)], max(1, runs // 2), expect=out,
                          env={"A0_INTERP": "1"})
        # LLVM's tutorial reads the program from standard input, compiles each top-level
        # expression with its JIT as it comes and runs it; printd writes to standard error. The
        # same program with the same calls doing no work gives startup and compilation, the rest
        # is running.
        text = src.read_text()
        trivial = re.sub(r"printd\((.*)\);\s*$", lambda _: TRIVIAL[prog], text.strip()) + "\n"
        assert trivial != text.strip() + "\n", prog
        full = timed_toy(toy, text, runs, expect=out.strip())
        empty = timed_toy(toy, trivial, runs)
        full2 = timed_toy(toy2, text, runs, expect=out.strip())
        empty2 = timed_toy(toy2, trivial, runs)
        w(f"| {prog} | {ms(build)} | {ms(build0)} | {ms(run)} | {ms(run0)} | {ms(jit)} | {ms(interp)} | "
          f"{ms(empty)} | {ms(max(full - empty, 0))} | {ms(empty2)} | {ms(max(full2 - empty2, 0))} |")
    w("")

    # Minnow against C and Go.
    w("## Minnow against C and Go\n")
    w("| Program | Minnow build | no HIR optimizer | clang -O0 | clang -O2 | go build | Minnow run | no HIR optimizer | Minnow JIT (`run`) | clang -O0 | clang -O2 | Go |")
    w("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for prog in NATIVE + MINNOW_ONLY:
        mn = BENCH / "minnow" / f"{prog}.mn"
        exe = str(WORK / f"mn-{prog}")
        b_mn, _ = timed([minnow, "build", str(mn), "-o", exe], runs)
        b_mn0, _ = timed([minnow, "build", str(mn), "-o", exe + "-noopt"], runs, env={"A0_OPT": "0"})
        c = BENCH / "c" / f"{prog}.c"
        b_c0, _ = timed(["clang", "-O0", str(c), "-lm", "-o", str(WORK / f"c-{prog}-O0")], runs)
        b_c2, _ = timed(["clang", "-O2", str(c), "-lm", "-o", str(WORK / f"c-{prog}-O2")], runs)
        go = BENCH / "go" / f"{prog}.go"
        b_go = go_build(go, WORK / f"go-{prog}", runs)
        r_c2, out = timed([str(WORK / f"c-{prog}-O2")], runs)
        r_c0, _ = timed([str(WORK / f"c-{prog}-O0")], runs, expect=out)
        r_go, _ = timed([str(WORK / f"go-{prog}")], runs, expect=out)
        r_mn, _ = timed([exe], runs, expect=out)
        RECORD[f"minnow {prog}, native"] = r_mn
        r_mn0, _ = timed([exe + "-noopt"], runs, expect=out)
        r_jit, _ = timed([minnow, "run", str(mn)], runs, expect=out)
        w(f"| {prog} | {ms(b_mn)} | {ms(b_mn0)} | {ms(b_c0)} | {ms(b_c2)} | {ms(b_go)} | {ms(r_mn)} | "
          f"{ms(r_mn0)} | {ms(r_jit)} | {ms(r_c0)} | {ms(r_c2)} | {ms(r_go)} |")
    w("")
    w(f"A0's collector in `binarytrees` (`A0_GC_STATS=1`): {gc_stats(str(WORK / 'mn-binarytrees'))}.\n")

    # Tin, the systems language (no collector, raw pointers, C's allocator), against the same C.
    w("## Tin against C\n")
    w("| Program | Tin build | Tin run | no HIR optimizer | Tin JIT (`run`) | clang -O0 | clang -O2 |")
    w("|---|---|---|---|---|---|---|")
    for prog in NATIVE:
        src = BENCH / "tin" / f"{prog}.tin"
        exe = str(WORK / f"tin-{prog}")
        b_tin, _ = timed([tin, "build", str(src), "-o", exe], runs)
        timed([tin, "build", str(src), "-o", exe + "-noopt"], 1, env={"A0_OPT": "0"})
        r_c2, out = timed([str(WORK / f"c-{prog}-O2")], runs)
        r_c0, _ = timed([str(WORK / f"c-{prog}-O0")], runs, expect=out)
        r_tin, _ = timed([exe], runs, expect=out)
        RECORD[f"tin {prog}, native"] = r_tin
        r_tin0, _ = timed([exe + "-noopt"], runs, expect=out)
        r_jit, _ = timed([tin, "run", str(src)], runs, expect=out)
        w(f"| {prog} | {ms(b_tin)} | {ms(r_tin)} | {ms(r_tin0)} | {ms(r_jit)} | {ms(r_c0)} | {ms(r_c2)} |")
    w("")

    # Threads: the same work split among more threads.
    w("## Threads\n")
    w(f"`parallel` counts the primes below 5,000,000 on several threads ({os.cpu_count()} processors).\n")
    w("| Threads | Minnow native | speedup | Minnow JIT (`run`) | speedup |")
    w("|---|---|---|---|---|")
    par = BENCH / "minnow" / "parallel.mn"
    par_exe = str(WORK / "mn-parallel")
    sh([minnow, "build", str(par), "-o", par_exe])
    base = None
    for n in [1, 2, 4, 8]:
        r_native, out = timed([par_exe, str(n), "5000000"], runs)
        RECORD[f"parallel, {n} threads"] = r_native
        r_jit, _ = timed([minnow, "run", str(par), str(n), "5000000"], runs)
        if base is None:
            base = (r_native, r_jit)
        w(f"| {n} | {ms(r_native)} | {base[0] / r_native:.1f}x | {ms(r_jit)} | {base[1] / r_jit:.1f}x |")
    w("")

    # Where a build's time goes. Each of A0's steps includes the ones before it (the object file
    # from the IR text); A0 links with `cc`, or `$CC`, like the C programs' links beside it.
    cc = os.path.realpath(shutil.which(os.environ.get("CC", "cc")))
    w(f"Where Minnow's build time goes (`cc` is {Path(cc).name}):\n")
    w("| Program | to HIR | to IR | object file | executable | executable, `CC=clang` | cc linking a C object | clang linking it |")
    w("|---|---|---|---|---|---|---|---|")
    for prog in ["fib", "nbody"]:
        mn = str(BENCH / "minnow" / f"{prog}.mn")
        t_hir, _ = timed([minnow, "hir", mn], runs)
        t_ir, ir = timed([minnow, "ir", mn], runs)
        ir_file = WORK / f"mn-{prog}.a0ir"
        ir_file.write_text(ir)
        t_obj, _ = timed([a0, "obj", str(ir_file), "-o", str(WORK / f"mn-{prog}.o")], runs)
        t_exe, _ = timed([minnow, "build", mn, "-o", str(WORK / f"mn-{prog}")], runs)
        t_exe_clang, _ = timed([minnow, "build", mn, "-o", str(WORK / f"mn-{prog}")], runs,
                               env={"CC": "clang"})
        obj = str(WORK / f"c-{prog}.o")
        sh(["clang", "-O0", "-c", str(BENCH / "c" / f"{prog}.c"), "-o", obj])
        t_cc, _ = timed(["cc", obj, "-lm", "-o", str(WORK / f"c-{prog}-linked")], runs)
        t_clang, _ = timed(["clang", obj, "-lm", "-o", str(WORK / f"c-{prog}-linked")], runs)
        w(f"| {prog} | {ms(t_hir)} | {ms(t_ir)} | {ms(t_obj)} | {ms(t_exe)} | {ms(t_exe_clang)} | "
          f"{ms(t_cc)} | {ms(t_clang)} |")
    w("")

    # Lox against clox.
    w("## Lox: A0 against clox\n")
    w("| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`lox file`) | A0 interpreted | clox |")
    w("|---|---|---|---|---|---|---|---|")
    suite = ROOT / "examples/lox/tests/craftinginterpreters/test/benchmark"
    for prog in LOX_PROGRAMS:
        src = suite / f"{prog}.lox"
        exe = str(WORK / f"lox-{prog}")
        build, _ = timed([lox, "build", str(src), "-o", exe], runs)
        build0, _ = timed([lox, "build", str(src), "-o", exe + "-noopt"], runs, env={"A0_OPT": "0"})
        r_clox, out = timed([clox, str(src)], runs)
        # The programs print the time they took; the rest must agree.
        r_native, mine = timed([exe], runs)
        RECORD[f"lox {prog}, native"] = r_native
        RECORD[f"clox {prog}"] = r_clox
        r_native0, mine0 = timed([exe + "-noopt"], runs)
        r_jit, mine_j = timed([lox, str(src)], runs)
        r_interp, mine_i = timed([lox, str(src)], max(1, runs // 2), env={"A0_INTERP": "1"})
        for o in (mine, mine0, mine_j, mine_i):
            if without_times(o) != without_times(out):
                raise RuntimeError(f"A0's Lox printed\n{o}\ninstead of\n{out}")
        w(f"| {prog} | {ms(build)} | {ms(build0)} | {ms(r_native)} | {ms(r_native0)} | {ms(r_jit)} | "
          f"{ms(r_interp)} | {ms(r_clox)} |")
    w("")
    w(f"A0's collector in `binary_trees` (`A0_GC_STATS=1`): {gc_stats(str(WORK / 'lox-binary_trees'))}.\n")

    large_programs(w, minnow, runs)

    Path(args.out).write_text("\n".join(report) + "\n")
    print("\n".join(report))
    if args.record:
        record(args.record)


def record(version):
    """Appends the headline numbers to bench/history.csv: one row per number, with the version,
    the date and the machine, so that releases measured on one machine can be compared."""
    import csv
    import datetime
    machine = f"{platform.machine()}, {os.cpu_count()} threads"
    try:
        cpu = [l for l in Path("/proc/cpuinfo").read_text().splitlines() if l.startswith("model name")]
        if cpu:
            machine = f"{cpu[0].split(':', 1)[1].strip()}, {os.cpu_count()} threads"
    except OSError:
        pass
    path = BENCH / "history.csv"
    new = not path.exists()
    with path.open("a", newline="") as f:
        out = csv.writer(f, lineterminator="\n")
        if new:
            out.writerow(["version", "date", "machine", "measure", "ms"])
        day = datetime.date.today().isoformat()
        for name, seconds in RECORD.items():
            out.writerow([version, day, machine, name, f"{seconds * 1000:.0f}"])


def large_programs(w, minnow, runs):
    """A generated 100,000-line Minnow package (bench/large): its build, by stage, and the
    language server on it."""
    large = WORK / "large"
    shutil.rmtree(large, ignore_errors=True)
    made = sh([sys.executable, str(BENCH / "large" / "gen.py"), str(large)]).stdout.strip()
    main = str(large / "src" / "main.mn")
    exe = str(WORK / "large-exe")
    w("## Large programs\n")
    w(f"`bench/large/gen.py` makes a Minnow package of {made}: each module imports up to three "
      "earlier ones and calls into them.\n")
    t_build, _ = timed([minnow, "build", main, "-o", exe], runs)
    RECORD["large program, build"] = t_build
    t_run, out = timed([minnow, "run", main], runs)
    t_exe, out2 = timed([exe], runs)
    if out != out2:
        raise RuntimeError(f"the large program printed {out2} natively and {out} in the JIT")
    w("| Build | `minnow run` (compiling included) | the executable |")
    w("|---|---|---|")
    w(f"| {ms(t_build)} | {ms(t_run)} | {ms(t_exe)} |\n")
    stages = subprocess.run([minnow, "build", main, "-o", exe], capture_output=True, text=True,
                            env={**os.environ, "A0_TIME": "1"}).stderr
    rows = [re.match(r"time:\s+([\d.]+) ms\s+(.*)", l) for l in stages.splitlines()]
    rows = [(m.group(2), float(m.group(1))) for m in rows if m]
    w("Where the build's time goes (`A0_TIME=1`):\n")
    w("| " + " | ".join(name for name, _ in rows) + " |")
    w("|" + "---|" * len(rows))
    w("| " + " | ".join(f"{t:.0f} ms" for _, t in rows) + " |\n")
    sys.path.insert(0, str(BENCH / "large"))
    import lsp
    w("`minnow lsp` on it: the time from opening a file to its diagnostics, from an edit to the "
      "answer to the next request (which waits for the new analysis), and to answer requests:\n")
    w("| File open | its program | open to diagnostics | edit to an answer | hover | definition | completion |")
    w("|---|---|---|---|---|---|---|")
    for file, program in [(None, "half the modules"), ("main.mn", "every module")]:
        name, medians = lsp.measure(minnow, large, runs, file)
        if file == "main.mn":
            RECORD["large program, language server edit to an answer"] = medians[lsp.KEYS[1]] / 1000
        w(f"| {name} | {program} | " + " | ".join(f"{medians[k]:.0f} ms" for k in lsp.KEYS) + " |")
    w("")


def gc_stats(exe):
    """What A0's collector reports about a run of `exe`."""
    r = subprocess.run([exe], capture_output=True, text=True, env={**os.environ, "A0_GC_STATS": "1"})
    lines = [l for l in r.stderr.splitlines() if l.startswith("gc: ")]
    return lines[-1][len("gc: "):] if lines else "no report"


def without_times(out):
    """A Lox benchmark's output without the times it prints (numbers with a fraction), and with
    other numbers as clox prints them (`%g`: zoo's sum is 1e+07 there, 10000002 on A0)."""
    lines = []
    for l in out.splitlines():
        if re.fullmatch(r"-?\d+\.\d+", l):
            continue
        try:
            lines.append(f"{float(l):g}")
        except ValueError:
            lines.append(l)
    return lines


def timed_toy(toy, text, runs, expect=None):
    """The median time of LLVM's tutorial on a program (from standard input). It prints what
    `printd` prints on standard error, among its prompts."""
    times = []
    for _ in range(runs):
        start = time.perf_counter()
        r = subprocess.run([toy], input=text, capture_output=True, text=True)
        times.append(time.perf_counter() - start)
        if r.returncode != 0:
            raise RuntimeError(f"LLVM's Kaleidoscope failed:\n{r.stderr}")
        if expect is not None and expect not in r.stderr:
            raise RuntimeError(f"LLVM's Kaleidoscope did not print {expect}:\n{r.stderr}")
    return statistics.median(times)


def go_build(src, out, runs):
    """The median time of `go build`. Go caches what it compiled by content, so each build gets a
    copy of the source that differs in a comment (the standard library stays cached, as it would
    for a programmer)."""
    times = []
    for i in range(runs):
        copy = WORK / "go-src" / f"{src.stem}.go"
        copy.parent.mkdir(parents=True, exist_ok=True)
        copy.write_text(src.read_text() + f"\n// build {time.time()} {i}\n")
        start = time.perf_counter()
        subprocess.run(["go", "build", "-o", str(out), str(copy)], check=True, capture_output=True)
        times.append(time.perf_counter() - start)
    return statistics.median(times)


if __name__ == "__main__":
    main()
