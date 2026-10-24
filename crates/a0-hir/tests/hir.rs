use std::path::Path;

use a0_diag::SourceMap;
use a0_hir::*;

fn parse_ok(text: &str) -> HirModule {
    let mut sm = SourceMap::new();
    let id = sm.add("test.hir", text);
    match parse(text, id) {
        Ok(m) => m,
        Err(d) => panic!("{}", d.render(&sm)),
    }
}

fn errors(text: &str) -> String {
    let mut sm = SourceMap::new();
    let id = sm.add("test.hir", text);
    match parse(text, id) {
        Ok(_) => String::new(),
        Err(d) => d.render(&sm),
    }
}

#[test]
fn corpus_round_trips_through_text() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/hir");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "hir") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let first = print_module(&parse_ok(&text));
        let second = print_module(&parse_ok(&first));
        assert_eq!(first, second, "{} does not round-trip", path.display());
        lower(&parse_ok(&first)).unwrap_or_else(|d| panic!("{}: {}", path.display(), d.summary()));
        count += 1;
    }
    assert!(count >= 5);
}

#[test]
fn canonical_text_is_stable() {
    let text = r#"module "m";

struct P { x: int, next: P? }

enum E { A(int), B }

global g: any;

extern fn putchar(int) -> int;

fn f(p: P, e: E) -> int {
  let n: int = -5;
  let s: string = "a\"b\\c\n";
  if n < 0 {
    n = -n;
  }
  while n > 0 {
    n = n - 1;
  }
  let c: fn(int) -> int = fn(x: int) -> int {
    x + n
  };
  g = ((1 + 2) * 3) as any;
  match e {
    A(v) => v,
    B => c(p.x),
  }
}

fn main() -> int {
  f(new P { x: 1, next: null<P> }, E::B)
}
"#;
    let m = parse_ok(text);
    assert_eq!(print_module(&m), text);
}

#[test]
fn precedence_survives_printing() {
    let text = "module \"m\";\n\nfn main() -> int {\n  let a: int = (1 + 2) * 3;\n  let b: int = 1 + 2 * 3;\n  let c: int = 1 - (2 - 3);\n  let d: bool = !(a < b) || a == b && c > 0;\n  let e: float = -(1 as float) * 2.0;\n  0\n}\n";
    let m = parse_ok(text);
    assert_eq!(print_module(&m), text);
}

#[test]
fn type_errors_are_reported_with_locations() {
    let cases: &[(&str, &str)] = &[
        (
            "fn main() -> int { 1 + \"x\" }",
            "mismatched types: expected `int`, found `string`",
        ),
        (
            "fn main() -> int { \"x\" }",
            "this body evaluates to `string` but must return `int`",
        ),
        (
            "struct P { x: int }\nfn main() -> int { let p = new P { x: 1 }; p.y }",
            "`P` has no field `y`",
        ),
        (
            "enum E { A, B }\nfn main() -> int { match E::A { A => 1 } }",
            "match does not cover B",
        ),
        (
            "fn main() -> int { break; 0 }",
            "`break` and `continue` can only be used inside a loop",
        ),
        (
            "fn f() -> int { return \"s\"; }\nfn main() -> int { 0 }",
            "mismatched types: expected `int`, found `string`",
        ),
        (
            "fn main() -> int { let x = 1; x(2) }",
            "`int` is not a function",
        ),
        (
            "fn main() -> int { let a = [1, 2]; a[\"x\"] }",
            "mismatched types: expected `int`, found `string`",
        ),
        (
            "fn main() -> int { let v = 1 as any; v as array<int>; 0 }",
            "cannot convert `any` to `array<int>`",
        ),
        (
            "fn main() -> int { 1 is int; 0 }",
            "mismatched types: expected `any`, found `int`",
        ),
        (
            "fn main() -> int { let m = new map<float, int>; 0 }",
            "map keys must be int, bool or string",
        ),
        (
            "fn main() -> int { push(1, 2); 0 }",
            "`push` needs an array, not `int`",
        ),
        (
            "struct P { x: int }\nfn main() -> int { let p = null<P>; p.x }",
            "`P?` may be null; unwrap it (`!`) or check it for null before using its fields",
        ),
        (
            "struct P { x: int }\nfn main() -> int { let p: P = null<P>; 0 }",
            "mismatched types: expected `P`, found `P?`",
        ),
        (
            "struct P { x: int }\nglobal g: P;\nfn main() -> int { 0 }",
            "global 'g' starts as null, so its type must be nullable (`P?`)",
        ),
        (
            "fn main() -> int { let n = 1; n! }",
            "`int` is not nullable",
        ),
        (
            "fn main() -> int { let xs = new array<int>; xs[0] }\nfn f() { let ys = []; }",
            "cannot infer the type `array<_>`",
        ),
        (
            "fn id<T>(x: T) -> T { x + x }\nfn main() -> int { id(1) }",
            "operator `+` cannot be applied to `T`",
        ),
        (
            "fn id<T>(x: T) -> T { x }\nfn main() -> int { id(\"s\") }",
            "this body evaluates to `string` but must return `int`",
        ),
        (
            "extern fn f(string);\nfn main() -> int { 0 }",
            "externs take int, float, bool or unit",
        ),
    ];
    let mut failures = Vec::new();
    for (src, want) in cases {
        let text = format!("module \"m\";\n{src}\n");
        let got = errors(&text);
        if !got.contains(want) || !got.contains("test.hir:") {
            failures.push(format!("{src}\n  want: {want}\n  got: {got}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn name_and_syntax_errors() {
    let e = errors("module \"m\";\nfn main() -> int { y }\n");
    assert!(e.contains("unknown name 'y'"), "{e}");
    let e = errors("module \"m\";\nfn main() -> int { let p = new Q { }; 0 }\n");
    assert!(e.contains("unknown struct 'Q'"), "{e}");
    let e =
        errors("module \"m\";\nstruct P { x: int }\nfn main() -> int { let p = new P { }; 0 }\n");
    assert!(e.contains("missing field 'x'"), "{e}");
    let e = errors("module \"m\";\nfn f(x: Missing) {}\nfn main() -> int { 0 }\n");
    assert!(e.contains("unknown type 'Missing'"), "{e}");
    let e = errors("module \"m\";\nfn main() -> int { let x = ; 0 }\n");
    assert!(e.contains("expected an expression"), "{e}");
    assert!(e.contains("test.hir:2:28"), "{e}");
}

#[test]
fn builder_checks_structure() {
    // An expression used twice is rejected: HIR is a tree.
    let mut b = HirBuilder::new("m");
    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let body = b.function_body(main);
    let one = b.int(1);
    let sum = b.binary(BinOp::Add, one, one);
    b.set_root(body, sum);
    let err = b.finish().unwrap_err().summary();
    assert!(err.contains("used in more than one place"), "{err}");

    // A local of one function cannot be used by another.
    let mut b = HirBuilder::new("m");
    let f = b.declare_function("f", &["x"], &[TypeId::INT], TypeId::INT);
    let g = b.declare_function("g", &[], &[], TypeId::INT);
    let x = b.param(b.function_body(f), 0);
    let fx = b.local(x);
    let fb = b.function_body(f);
    b.set_root(fb, fx);
    let gx = b.local(x);
    let gb = b.function_body(g);
    b.set_root(gb, gx);
    let err = b.finish().unwrap_err().summary();
    assert!(
        err.contains("used outside the function that declares it"),
        "{err}"
    );

    // Items may have any name; the text format quotes keywords and non-identifiers. Locals are
    // renamed instead.
    let mut b = HirBuilder::new("m");
    let f = b.declare_function("len", &[], &[], TypeId::INT);
    let body = b.function_body(f);
    let one = b.int(1);
    b.set_root(body, one);
    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let body = b.function_body(main);
    let call = b.call(f, &[]);
    b.set_root(body, call);
    let text = b.finish().unwrap().to_string();
    assert!(
        text.contains("fn `len`() -> int") && text.contains("  `len`()"),
        "{text}"
    );
    assert_eq!(parse_ok(&text).to_string(), text);
    let mut b = HirBuilder::new("m");
    let f = b.declare_function(
        "f",
        &["match", "x y"],
        &[TypeId::INT, TypeId::INT],
        TypeId::UNIT,
    );
    let body = b.function_body(f);
    let unit = b.unit();
    b.set_root(body, unit);
    let m = b.finish().unwrap();
    assert!(m.to_string().contains("fn f(match_: int, x_y: int)"), "{m}");
}

#[test]
fn lowering_needs_an_entry_point() {
    let m = parse_ok("module \"m\";\nfn helper() -> int { 1 }\n");
    let err = lower(&m).unwrap_err().summary();
    assert!(err.contains("the program has no `main` function"), "{err}");
}

#[test]
fn lowered_closures_take_their_environment_first() {
    let m = parse_ok(
        "module \"m\";\nfn main() -> int {\n  let n = 1;\n  let f = fn(x: int) -> int { x + n };\n  f(41)\n}\n",
    );
    let ir = lower(&m).unwrap();
    let text = ir.to_string();
    assert!(
        text.contains("func @hir.main.closure0(%0: ptr, %1: i64) -> i64"),
        "{text}"
    );
    assert!(text.contains("call_indirect"), "{text}");
}

#[test]
fn builders_infer_types_and_generics_are_monomorphized() {
    // fn id<T>(x: T) -> T { x }, called with an int and a string; `xs` is `[]` until a push.
    let mut b = HirBuilder::new("m");
    let id = b.declare_generic_function("id", &["T"]);
    let t = b.type_param(Item::Func(id), 0);
    b.set_signature(id, &["x"], &[t], t);
    let body = b.function_body(id);
    let x = b.param(body, 0);
    let root = b.local(x);
    b.set_root(body, root);

    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let body = b.function_body(main);
    let one = b.int(1);
    let n = b.call(id, &[one]);
    let s = b.string("s");
    let s = b.call(id, &[s]);
    let s_len = b.builtin(Builtin::Len, &[s]);
    let elem = b.fresh_type();
    let xs_ty = b.array_type(elem);
    let xs = b.new_local(body, "xs", xs_ty);
    let empty = b.array(&[]);
    let bind = b.let_(xs, empty);
    let xs_ref = b.local(xs);
    let two = b.int(2);
    let pushed = b.call(id, &[two]);
    let push = b.builtin(Builtin::Push, &[xs_ref, pushed]);
    let sum = b.binary(BinOp::Add, n, s_len);
    let root = b.block(&[bind, push], Some(sum));
    b.set_root(body, root);
    let m = b.finish().unwrap_or_else(|d| panic!("{}", d.summary()));
    assert_eq!(m.type_name(m.local_type(xs)), "array<int>");

    let ir = lower(&m).unwrap().to_string();
    assert!(ir.contains("func @\"hir.id<int>\"("), "{ir}");
    assert!(ir.contains("func @\"hir.id<string>\"("), "{ir}");
}

#[test]
fn reused_nodes_and_open_types_are_reported() {
    let mut b = HirBuilder::new("m");
    let main = b.declare_function("main", &[], &[], TypeId::UNIT);
    let body = b.function_body(main);
    let t = b.fresh_type();
    let l = b.new_local(body, "unknown", t);
    let e = b.local(l);
    let f = b.field_named(e, "x");
    let root = b.block(&[f], None);
    b.set_root(body, root);
    let err = b.finish().unwrap_err().summary();
    assert!(
        err.contains("the type of this value must be known here"),
        "{err}"
    );
}

#[test]
fn the_optimizer_inlines_small_functions_and_folds_any_tests() {
    let m = parse_ok(
        "module \"m\";\nfn truthy(v: any) -> bool {\n  if v is unit { false } else if v is bool { v as bool } else { true }\n}\nfn main() -> int {\n  let n = 3;\n  if truthy((n > 2) as any) { 1 } else { 0 }\n}\n",
    );
    let main_of = |optimize: bool| {
        let ir = lower_with(&m, LowerOptions { optimize })
            .unwrap()
            .to_string();
        let start = ir.find("func @hir.main(").unwrap();
        let end = ir[start..].find("\n}\n").unwrap();
        ir[start..start + end].to_string()
    };
    let plain = main_of(false);
    assert!(plain.contains("call @hir.truthy"), "{plain}");
    let optimized = main_of(true);
    // Inlined, and `((n > 2) as any) is bool` is known, so no `any` is ever built or tested.
    assert!(!optimized.contains("call @hir.truthy"), "{optimized}");
    assert!(!optimized.contains("select"), "{optimized}");
    assert!(!optimized.contains("a0_any"), "{optimized}");
}

#[test]
fn casts_guarded_by_a_type_test_are_not_checked_again() {
    let src = |assign: &str| {
        format!(
            "module \"m\";\nstruct C {{ n: int }}\nfn get(v: any) -> int {{\n  if v is C {{ {assign}(v as C).n }} else {{ 0 }}\n}}\nfn main() -> int {{ get(new C {{ n: 1 }} as any) }}\n"
        )
    };
    let get_of = |src: &str, optimize: bool| {
        let ir = lower_with(&parse_ok(src), LowerOptions { optimize })
            .unwrap()
            .to_string();
        let start = ir.find("func @hir.get(").unwrap();
        let end = ir[start..].find("\n}\n").unwrap();
        ir[start..start + end].to_string()
    };
    let guarded = src("");
    assert!(get_of(&guarded, false).contains("a0_any_cast_failed"));
    let optimized = get_of(&guarded, true);
    assert!(!optimized.contains("a0_any_cast_failed"), "{optimized}");
    // Assigning `v` between the test and the cast voids the guarantee.
    let assigned = src("v = 1 as any; ");
    let optimized = get_of(&assigned, true);
    assert!(optimized.contains("a0_any_cast_failed"), "{optimized}");
}
