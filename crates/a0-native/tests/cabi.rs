//! C structs by value across the C boundary (`docs/ir.md`, "C structs"): IR calls C functions
//! that take and return structs of many shapes, in code compiled into memory and in the
//! interpreter (through thunks). The C functions are Rust `extern "C"` functions, which follow the
//! platform's C ABI; what they receive and return is recorded and compared.

use std::fmt::Write;
use std::mem::{align_of, offset_of, size_of};
use std::sync::Mutex;

use a0_native::{JitOptions, add_thunks, jit_supported, run_main};
use a0_runtime::HostSymbols;

static RECORDED: Mutex<Vec<String>> = Mutex::new(Vec::new());

extern "C" fn record_i64(v: i64) {
    RECORDED.lock().unwrap().push(v.to_string());
}

extern "C" fn record_f64(v: f64) {
    RECORDED.lock().unwrap().push(format!("{v}"));
}

/// A C struct's layout as the IR spells it, with its C functions' addresses.
struct Shape {
    name: &'static str,
    size: usize,
    align: usize,
    fields: Vec<(&'static str, usize)>,
    sum: usize,
    make: usize,
}

trait IrType {
    const NAME: &'static str;
}

impl IrType for i8 {
    const NAME: &'static str = "i8";
}
impl IrType for i32 {
    const NAME: &'static str = "i32";
}
impl IrType for i64 {
    const NAME: &'static str = "i64";
}
impl IrType for f32 {
    const NAME: &'static str = "f32";
}
impl IrType for f64 {
    const NAME: &'static str = "f64";
}

/// A struct, a C function that sums its fields with weights 1, 100, 10000... (so every field and
/// its order count), and one that builds it from its fields.
macro_rules! shape {
    ($s:ident, $($f:ident : $t:ty),*) => {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct $s { $($f: $t),* }

        impl $s {
            extern "C" fn sum(s: $s) -> f64 {
                let mut weight = 1.0;
                let mut total = 0.0;
                $(
                    total += s.$f as f64 * weight;
                    weight *= 100.0;
                )*
                let _ = weight;
                total
            }

            extern "C" fn make($($f: $t),*) -> $s {
                $s { $($f),* }
            }

            fn shape() -> Shape {
                Shape {
                    name: stringify!($s),
                    size: size_of::<$s>(),
                    align: align_of::<$s>(),
                    fields: vec![$((<$t as IrType>::NAME, offset_of!($s, $f))),*],
                    sum: $s::sum as *const () as usize,
                    make: $s::make as *const () as usize,
                }
            }
        }
    };
}

shape!(PairI32, a: i32, b: i32);
shape!(PairF32, a: f32, b: f32);
shape!(PairF64, a: f64, b: f64);
shape!(TripleF64, a: f64, b: f64, c: f64);
shape!(Mixed, a: i8, b: i32, c: f64);
shape!(FloatInt, a: f32, b: i32);
shape!(Bytes3, a: i8, b: i8, c: i8);
shape!(QuadI64, a: i64, b: i64, c: i64, d: i64);
shape!(TripleF32, a: f32, b: f32, c: f32);
shape!(OneF32, a: f32);
shape!(OneI8, a: i8);

/// Structs among other arguments, past the argument registers.
extern "C" fn many(
    a: i32,
    p: PairF64,
    b: i32,
    q: PairI32,
    c: f64,
    t: TripleF64,
    d: i64,
    s: Bytes3,
    u: TripleF32,
) -> f64 {
    a as f64
        + PairF64::sum(p) * 2.0
        + b as f64 * 3.0
        + PairI32::sum(q) * 4.0
        + c * 5.0
        + TripleF64::sum(t) * 6.0
        + d as f64 * 7.0
        + Bytes3::sum(s) * 8.0
        + TripleF32::sum(u) * 9.0
}

/// More arguments than the interpreter's own foreign calls take.
#[allow(clippy::too_many_arguments)]
extern "C" fn ten(
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    e: i64,
    f: i64,
    g: i64,
    h: i64,
    i: i64,
    j: i64,
) -> i64 {
    [a, b, c, d, e, f, g, h, i, j]
        .iter()
        .enumerate()
        .map(|(k, v)| v * (k as i64 + 1))
        .sum()
}

extern "C" fn scale_f32(x: f32, y: f32) -> f32 {
    x * y
}

fn shapes() -> Vec<Shape> {
    vec![
        PairI32::shape(),
        PairF32::shape(),
        PairF64::shape(),
        TripleF64::shape(),
        Mixed::shape(),
        FloatInt::shape(),
        Bytes3::shape(),
        QuadI64::shape(),
        TripleF32::shape(),
        OneF32::shape(),
        OneI8::shape(),
    ]
}

fn layout(s: &Shape) -> String {
    let fields: Vec<String> = s.fields.iter().map(|(t, at)| format!("{t} {at}")).collect();
    format!("{}, {}: {}", s.size, s.align, fields.join(", "))
}

/// The value field `k` of a struct gets.
fn value(ty: &str, k: usize) -> String {
    match ty {
        "f32" | "f64" => format!("{}.5", k + 2),
        _ => format!("{}", k + 2),
    }
}

/// Code that leaves a struct of shape `s` with fields `value(k)` in `%{var}`.
fn build(out: &mut String, s: &Shape, var: &str) {
    let _ = writeln!(out, "  %{var}: ptr = stack_slot {}, {}", s.size, s.align);
    for (k, (ty, at)) in s.fields.iter().enumerate() {
        let _ = writeln!(out, "  %{var}_v{k}: {ty} = {} {}", konst(ty), value(ty, k));
        let _ = writeln!(out, "  %{var}_o{k}: i64 = iconst {at}");
        let _ = writeln!(out, "  %{var}_p{k}: ptr = ptradd %{var}, %{var}_o{k}");
        let _ = writeln!(out, "  store %{var}_v{k}, %{var}_p{k}");
    }
}

fn konst(ty: &str) -> &'static str {
    if ty.starts_with('f') {
        "fconst"
    } else {
        "iconst"
    }
}

/// A call of `@record_f64` or `@record_i64` with `%v` of type `ty`, through `%{tmp}`.
fn record(out: &mut String, ty: &str, v: &str, tmp: &str) {
    match ty {
        "f64" => {
            let _ = writeln!(out, "  call @record_f64(%{v})");
        }
        "f32" => {
            let _ = writeln!(out, "  %{tmp}: f64 = fext %{v}");
            let _ = writeln!(out, "  call @record_f64(%{tmp})");
        }
        "i64" => {
            let _ = writeln!(out, "  call @record_i64(%{v})");
        }
        _ => {
            let _ = writeln!(out, "  %{tmp}: i64 = sext %{v}");
            let _ = writeln!(out, "  call @record_i64(%{tmp})");
        }
    }
}

/// The program: every shape passed (its sum recorded) and returned (its fields recorded), then
/// `many`, `ten` and `scale_f32`.
fn program(shapes: &[Shape]) -> String {
    let mut out = String::from("module \"cabi\"\n\n");
    out.push_str("extern func @record_i64(i64)\nextern func @record_f64(f64)\n");
    for s in shapes {
        let l = layout(s);
        let params: Vec<&str> = s.fields.iter().map(|(t, _)| *t).collect();
        let _ = writeln!(out, "extern func @{}_sum(ptr byval({l})) -> f64", s.name);
        let _ = writeln!(
            out,
            "extern func @{}_make(ptr sret({l}), {})",
            s.name,
            params.join(", ")
        );
    }
    let by = |name: &str| layout(shapes.iter().find(|s| s.name == name).unwrap());
    let _ = writeln!(
        out,
        "extern func @many(i32, ptr byval({}), i32, ptr byval({}), f64, ptr byval({}), i64, ptr byval({}), ptr byval({})) -> f64",
        by("PairF64"),
        by("PairI32"),
        by("TripleF64"),
        by("Bytes3"),
        by("TripleF32"),
    );
    let _ = writeln!(
        out,
        "extern func @ten(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64"
    );
    out.push_str("extern func @scale_f32(f32, f32) -> f32\n\nfunc @main() -> i32 {\n");
    for (n, s) in shapes.iter().enumerate() {
        let a = format!("a{n}");
        build(&mut out, s, &a);
        let _ = writeln!(out, "  %sum{n}: f64 = call @{}_sum(%{a})", s.name);
        let _ = writeln!(out, "  call @record_f64(%sum{n})");
        let r = format!("r{n}");
        let _ = writeln!(out, "  %{r}: ptr = stack_slot {}, {}", s.size, s.align);
        let args: Vec<String> = (0..s.fields.len()).map(|k| format!("%{a}_v{k}")).collect();
        let _ = writeln!(out, "  call @{}_make(%{r}, {})", s.name, args.join(", "));
        for (k, (ty, at)) in s.fields.iter().enumerate() {
            let _ = writeln!(out, "  %{r}_o{k}: i64 = iconst {at}");
            let _ = writeln!(out, "  %{r}_p{k}: ptr = ptradd %{r}, %{r}_o{k}");
            let _ = writeln!(out, "  %{r}_v{k}: {ty} = load %{r}_p{k}");
            record(&mut out, ty, &format!("{r}_v{k}"), &format!("{r}_w{k}"));
        }
    }
    let index = |name: &str| shapes.iter().position(|s| s.name == name).unwrap();
    let _ = writeln!(
        out,
        "  %m0: i32 = iconst 1\n  %m1: i32 = iconst 3\n  %m2: f64 = fconst 5.5\n  %m3: i64 = iconst 7"
    );
    let _ = writeln!(
        out,
        "  %many: f64 = call @many(%m0, %a{}, %m1, %a{}, %m2, %a{}, %m3, %a{}, %a{})",
        index("PairF64"),
        index("PairI32"),
        index("TripleF64"),
        index("Bytes3"),
        index("TripleF32"),
    );
    out.push_str("  call @record_f64(%many)\n");
    let mut tens = Vec::new();
    for k in 0..10 {
        let _ = writeln!(out, "  %t{k}: i64 = iconst {}", 10 - k);
        tens.push(format!("%t{k}"));
    }
    let _ = writeln!(out, "  %ten: i64 = call @ten({})", tens.join(", "));
    out.push_str("  call @record_i64(%ten)\n");
    out.push_str("  %x: f32 = fconst 1.5\n  %y: f32 = fconst -2.25\n");
    out.push_str("  %scaled: f32 = call @scale_f32(%x, %y)\n");
    record(&mut out, "f32", "scaled", "scaled64");
    out.push_str("  %zero: i32 = iconst 0\n  return %zero\n}\n");
    out
}

fn symbols(shapes: &[Shape]) -> HostSymbols {
    let mut syms = HostSymbols::new();
    syms.add("record_i64", record_i64 as *const () as usize);
    syms.add("record_f64", record_f64 as *const () as usize);
    for s in shapes {
        syms.add(&format!("{}_sum", s.name), s.sum);
        syms.add(&format!("{}_make", s.name), s.make);
    }
    syms.add("many", many as *const () as usize);
    syms.add("ten", ten as *const () as usize);
    syms.add("scale_f32", scale_f32 as *const () as usize);
    syms
}

/// What the program should record, computed by calling the same functions from Rust.
fn expected(shapes: &[Shape]) -> Vec<String> {
    let mut out = Vec::new();
    for s in shapes {
        let mut weight = 1.0;
        let mut total = 0.0;
        for (k, (ty, _)) in s.fields.iter().enumerate() {
            let v: f64 = value(ty, k).parse().unwrap();
            total += v * weight;
            weight *= 100.0;
        }
        out.push(format!("{total}"));
        for (k, (ty, _)) in s.fields.iter().enumerate() {
            out.push(value(ty, k).trim_end_matches(".0").to_string());
        }
    }
    let many = many(
        1,
        PairF64::make(2.5, 3.5),
        3,
        PairI32::make(2, 3),
        5.5,
        TripleF64::make(2.5, 3.5, 4.5),
        7,
        Bytes3::make(2, 3, 4),
        TripleF32::make(2.5, 3.5, 4.5),
    );
    out.push(format!("{many}"));
    out.push(ten(10, 9, 8, 7, 6, 5, 4, 3, 2, 1).to_string());
    out.push(format!("{}", scale_f32(1.5, -2.25) as f64));
    out
}

#[test]
fn structs_cross_the_c_boundary_in_the_jit_and_the_interpreter() {
    let shapes = shapes();
    let text = program(&shapes);
    let m = a0_ir::parse_and_verify(&text, a0_diag::SourceId(0))
        .unwrap_or_else(|d| panic!("{d:?}\n{text}"));
    let want = expected(&shapes);

    if jit_supported() {
        RECORDED.lock().unwrap().clear();
        let code = run_main(&m, symbols(&shapes), vec![], &JitOptions::default()).unwrap();
        assert_eq!(code, 0);
        assert_eq!(*RECORDED.lock().unwrap(), want, "JIT");

        RECORDED.lock().unwrap().clear();
        let mut syms = symbols(&shapes);
        add_thunks(&m, &mut syms).unwrap();
        let code = a0_interp::run_main(m.clone(), syms, vec![])
            .unwrap()
            .unwrap();
        assert_eq!(code, 0);
        assert_eq!(*RECORDED.lock().unwrap(), want, "interpreter");
    }
}

#[test]
fn the_interpreter_needs_thunks_for_struct_calls() {
    let shapes = shapes();
    let m = a0_ir::parse_and_verify(&program(&shapes), a0_diag::SourceId(0)).unwrap();
    let err = a0_interp::Interpreter::new(m, &symbols(&shapes))
        .err()
        .expect("struct parameters need a thunk");
    let text = format!("{err:?}");
    assert!(text.contains("C struct parameters"), "{text}");
}
