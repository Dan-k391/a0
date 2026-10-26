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
    // (program, code, part of the rendered diagnostic)
    let cases: &[(&str, &str, &str)] = &[
        (
            "fn main() -> int { 1 + \"x\" }",
            "E2001",
            "^^^ expected `int`, found `string`",
        ),
        (
            "fn main() -> int { \"x\" }",
            "E2001",
            "`main` returns `int`",
        ),
        (
            "struct P { x: int }\nfn main() -> int { let p = new P { x: 1 }; p.y }",
            "E1003",
            "no field `y` on type `P`",
        ),
        (
            "enum E { A, B }\nfn main() -> int { match E::A { A => 1 } }",
            "E2014",
            "`match` does not cover `B`",
        ),
        (
            "fn main() -> int { let a: array<string> = filled(\"3\", \"x\"); 0 }",
            "E2001",
            "expected `int`, found `string`",
        ),
        (
            "fn main() -> int { let a: array<string> = filled(3, 1); 0 }",
            "E2001",
            "expected `array<string>`, found `array<int>`",
        ),
        (
            "fn main() -> int { let b: u8 = 256; 0 }",
            "E2023",
            "the literal `256` does not fit in `u8`",
        ),
        (
            "fn main() -> int { let b: i8 = -129; 0 }",
            "E2023",
            "`i8` holds -128 to 127",
        ),
        (
            "fn main() -> int { let a: u8 = 1; let b: i32 = 2; a + b; 0 }",
            "E2001",
            "expected `u8`, found `i32`",
        ),
        (
            "fn main() -> int { 1.5 & 2.5; 0 }",
            "E2004",
            "operator `&` cannot be applied to `float`",
        ),
        (
            "inline struct V { s: string }
fn main() -> int { 0 }",
            "E1015",
            "inline struct `V` cannot hold `string` in its field `s`",
        ),
        (
            "inline struct A { b: B }
inline struct B { a: A }
fn main() -> int { 0 }",
            "E1016",
            "inline struct `A` contains itself",
        ),
        (
            "inline struct V { x: int }
fn main() -> int { let v = new V { x: 1 }; v == v; 0 }",
            "E2024",
            "cannot be compared with `==`",
        ),
        (
            "inline struct V { x: int }
fn main() -> int { let v = new V { x: 1 }; v as any; 0 }",
            "E2024",
            "cannot be converted",
        ),
        (
            "fn main() -> int { let s = \"x\"; addr(s); 0 }",
            "E2025",
            "it holds `string`, which is not plain data",
        ),
        (
            "fn main() -> int { addr(1 + 2); 0 }",
            "E2025",
            "it is not a variable, a field or an element",
        ),
        (
            "struct N { v: int }
fn main() -> int { free(new N { v: 1 }); 0 }",
            "E2026",
            "`free` is for programs that manage their memory themselves",
        ),
        (
            "fn main() -> int { break; 0 }",
            "E3001",
            "`break` outside of a loop",
        ),
        (
            "struct P { x: int }\nimpl P {\n  fn get(self: P) -> int { self.x }\n}\nfn main() -> int { let p = new P { x: 1 }; p.gte() }",
            "E1013",
            "no method `gte` on `P`",
        ),
        (
            "struct P { x: int }\ninterface I { fn f(self) -> int; }\nimpl I for P;\nfn main() -> int { 0 }",
            "E2021",
            "`P` does not have the method `f` that `I` requires",
        ),
        (
            "struct P { x: int }\ninterface I { fn f(self) -> int; }\nimpl I for P;\nimpl P {\n  fn f(self: P) -> string { \"\" }\n}\nfn main() -> int { 0 }",
            "E2021",
            "expected `fn() -> int`, found `fn() -> string`",
        ),
        (
            "interface I { fn f(self) -> int; }\nfn g<T: I>(x: T) -> int { x.f() }\nfn main() -> int { g(1) }",
            "E2021",
            "`int` does not implement `I`",
        ),
        (
            "struct P { x: int }\ninterface I { fn same(self, Self) -> bool; }\nimpl I for P;\nimpl P {\n  fn same(self: P, o: P) -> bool { true }\n}\nfn h(i: I) -> bool { i.same(i) }\nfn main() -> int { 0 }",
            "E2022",
            "cannot be called on a value of the interface",
        ),
        (
            "struct P { x: int }\ninterface I { fn f(self) -> int; }\nfn h(i: I) -> int { i.f() }\nfn main() -> int { h(new P { x: 1 }) }",
            "E2001",
            "expected `I`, found `P`",
        ),
        (
            "fn f() -> int { return \"s\"; }\nfn main() -> int { 0 }",
            "E2001",
            "`f` returns `int`",
        ),
        (
            "fn main() -> int { let x = 1; x(2) }",
            "E2003",
            "expected a function, found `int`",
        ),
        (
            "fn main() -> int { if let x = 5 { x } else { 0 } }",
            "E2006",
            "`if let` needs a nullable value, found `int`",
        ),
        (
            "struct P { x: int }\nfn main() -> int { let p = new P { x: 1 }; if let q = p { q.x } else { 0 } }",
            "E2006",
            "`if let` needs a nullable value, found `P`",
        ),
        (
            "fn main() -> int { let a = [1, 2]; a[\"x\"] }",
            "E2001",
            "expected `int`, found `string`",
        ),
        (
            "fn main() -> int { let v = 1 as any; v as array<int>; 0 }",
            "E2008",
            "cannot convert `any` to `array<int>`",
        ),
        (
            "fn main() -> int { 1 is int; 0 }",
            "E2001",
            "expected `any`, found `int`",
        ),
        (
            "fn main() -> int { let m = new map<float, int>; 0 }",
            "E2017",
            "map keys must be integers, booleans or strings",
        ),
        (
            "fn main() -> int { push(1, 2); 0 }",
            "E2001",
            "`push` needs an array, not `int`",
        ),
        (
            "struct P { x: int }\nfn main() -> int { let p = null<P>; p.x }",
            "E2007",
            "this value of type `P?` may be `null`",
        ),
        (
            "struct P { x: int }\nfn main() -> int { let p: P = null<P>; 0 }",
            "E2001",
            "expected `P`, found `P?`",
        ),
        (
            "struct P { x: int }\nglobal g: P;\nfn main() -> int { 0 }",
            "E2010",
            "the global `g` starts as `null`, so its type must be nullable",
        ),
        (
            "fn main() -> int { let n = 1; n! }",
            "E2006",
            "`int` is not nullable",
        ),
        (
            "fn main() -> int { let xs = new array<int>; xs[0] }\nfn f() { let ys = []; }",
            "E2005",
            "cannot infer the type `array<_>`",
        ),
        (
            "fn id<T>(x: T) -> T { x + x }\nfn main() -> int { id(1) }",
            "E2004",
            "operator `+` cannot be applied to `T`",
        ),
        (
            "fn id<T>(x: T) -> T { x }\nfn main() -> int { id(\"s\") }",
            "E2001",
            "`main` returns `int`",
        ),
        (
            "extern fn f(array<string>);\nfn main() -> int { 0 }",
            "hir.builder",
            "C functions take numbers, bool, pointers, strings, arrays of numbers or bool",
        ),
        (
            "inline struct S { name: string }\nextern fn f(S);\nfn main() -> int { 0 }",
            "hir.builder",
            "extern fn f takes S; C functions take numbers",
        ),
        (
            "struct S { x: int }\nextern fn f() -> S;\nfn main() -> int { 0 }",
            "hir.builder",
            "extern fn f returns S; C functions return",
        ),
        (
            "extern fn f() -> array<int>;\nfn main() -> int { 0 }",
            "hir.builder",
            "C functions return numbers, bool, pointers, unit, inline structs of numbers",
        ),
        (
            "extern \"cobol\" fn f();\nfn main() -> int { 0 }",
            "hir.text",
            "unknown ABI",
        ),
    ];
    let mut failures = Vec::new();
    for (src, code, want) in cases {
        let text = format!("module \"m\";\n{src}\n");
        let got = errors(&text);
        if !got.contains(want) || !got.contains("test.hir:") || !got.contains(&format!("[{code}]"))
        {
            failures.push(format!("{src}\n  want: [{code}] {want}\n  got: {got}"));
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
fn builder_misuse_found_by_fuzzing_is_reported() {
    // A closure whose body makes the closure: no pass may loop or overflow its stack on it.
    let mut b = HirBuilder::new("m");
    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let body = b.function_body(main);
    let inner = b.new_closure_body(body, &[], &[], TypeId::UNIT);
    let lambda = b.lambda(inner);
    b.set_root(inner, lambda);
    let zero = b.int(0);
    b.set_root(body, zero);
    let err = b.finish().unwrap_err().summary();
    assert!(
        err.contains("made outside the body it was declared in"),
        "{err}"
    );

    // Too many or too few type arguments are reported, and the types made stay usable.
    let mut b = HirBuilder::new("m");
    let s = b.declare_generic_struct("Box", &["T"]);
    let t = b.type_param(Item::Struct(s), 0);
    b.add_field(s, "value", t);
    let bad = b.type_param(Item::Struct(s), 3);
    assert_eq!(bad, TypeId::ERROR);
    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let body = b.function_body(main);
    let one = b.int(1);
    let made = b.new_struct_generic(s, &[TypeId::INT, TypeId::INT], &[one]);
    let boxed = b.struct_type_args(s, &[TypeId::INT]);
    let l = b.new_local(body, "b", boxed);
    let let_ = b.let_(l, made);
    let zero = b.int(0);
    let block = b.block(&[let_], Some(zero));
    b.set_root(body, block);
    let err = b.finish().unwrap_err().summary();
    assert!(
        err.contains("`Box` takes 1 type argument but 2 were given"),
        "{err}"
    );

    // A type parameter used outside its item.
    let mut b = HirBuilder::new("m");
    let s = b.declare_generic_struct("Box", &["T"]);
    let t = b.type_param(Item::Struct(s), 0);
    b.declare_global("g", t);
    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let body = b.function_body(main);
    let zero = b.int(0);
    b.set_root(body, zero);
    let err = b.finish().unwrap_err().summary();
    assert!(
        err.contains("a type parameter is used outside the item"),
        "{err}"
    );
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
    // Unoptimized: the optimizer would inline the call of the known closure (D84).
    let ir = lower_with(&m, LowerOptions { optimize: false }).unwrap();
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

    // Without inlining, so that the instances are still called (lowering skips unused functions).
    let ir = lower_with(&m, LowerOptions { optimize: false })
        .unwrap()
        .to_string();
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
fn functions_that_end_in_return_are_inlined() {
    // The body's type is `never` (it ends in `return`), but it returns: it is no error path.
    let m = parse_ok(
        "module \"m\";\nfn peek(xs: array<int>, i: int) -> int {\n  if i < len(xs) { return xs[i]; }\n  return -1;\n}\nfn main() -> int {\n  peek([1, 2], 1) + peek([3], 5)\n}\n",
    );
    let ir = lower_with(&m, LowerOptions { optimize: true })
        .unwrap()
        .to_string();
    assert!(!ir.contains("call @hir.peek"), "{ir}");
}

#[test]
fn if_let_bindings_have_their_type_in_the_body() {
    // A `match` needs its scrutinee's enum while it is parsed, and here that depends on `r`.
    parse_ok(
        "module \"m\";
enum E { A, B }
struct R { n: int }
fn main() -> int {
  let e = E::A;
  if let r = null<R> {
    match (if let q = (r as R?) { e } else { e }) { A => 1, _ => 2, }
  } else { 0 }
}
",
    );
}

#[test]
fn inlining_is_bounded_along_call_chains() {
    // Each helper calls the previous one twice: inlining everything would double the code at every
    // step. A callee is measured after its own inlining, so every few helpers the chain stays a call
    // and the code grows by a constant factor, however long the chain.
    let mut text = String::from(
        "module \"m\";
fn h0(x: int) -> int { x + 1 }
",
    );
    for i in 1..=64 {
        text += &format!(
            "fn h{i}(x: int) -> int {{ if x > 9 {{ h{p}(x) }} else {{ h{p}(x + 1) }} }}
",
            p = i - 1
        );
    }
    text += "fn main() -> int { h64(0) }
";
    let m = parse_ok(&text);
    let size = |optimize: bool| {
        lower_with(&m, LowerOptions { optimize })
            .unwrap()
            .to_string()
            .lines()
            .count()
    };
    let (plain, optimized) = (size(false), size(true));
    assert!(
        optimized < 8 * plain,
        "{optimized} lines of IR, {plain} without inlining"
    );
}

#[test]
fn casts_guarded_by_a_type_test_are_not_checked_again() {
    let src = |assign: &str| {
        format!(
            "module \"m\";\nstruct C {{ n: int }}\nfn get(v: any) -> int {{\n  if v is C {{ {assign}(v as C).n }} else {{ 0 }}\n}}\nfn main() -> int {{ let f = get; f(new C {{ n: 1 }} as any) }}\n"
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

#[test]
fn warnings_are_found_only_when_turned_on() {
    let text = "module \"m\";\nfn f(unused_param: int) -> int {\n  let x = 1;\n  let _quiet = 2;\n  return 3;\n  4\n}\nfn main() -> int { f(0) }\n";
    let finish = |warnings: &[Warning]| {
        let mut b = HirBuilder::from_module(parse_ok(text));
        for &w in warnings {
            b.enable_warning(w);
        }
        let m = b.finish().unwrap();
        m.warnings()
            .as_slice()
            .iter()
            .map(|d| format!("{} {}", d.code, d.message))
            .collect::<Vec<_>>()
    };
    assert!(finish(&[]).is_empty());
    assert_eq!(
        finish(&[Warning::UnusedVariable]),
        ["W0001 unused variable: `x`"]
    );
    assert_eq!(
        finish(&[Warning::UnreachableCode]),
        ["W0002 unreachable code"]
    );
    // A program with errors shows only its errors.
    let mut b = HirBuilder::from_module(parse_ok(text));
    b.enable_warning(Warning::UnusedVariable);
    b.report("an error of the language's own");
    let errors = b.finish().unwrap_err();
    assert!(errors.as_slice().iter().all(|d| d.is_error()), "{errors:?}");
}

#[test]
fn modules_have_their_own_names_and_hide_private_items() {
    let mut b = HirBuilder::new("program");
    // A library module with a public and a private function.
    b.begin_module("lib", false);
    let body = |b: &mut HirBuilder, f: FuncId, value: i64| {
        let body = b.function_body(f);
        let v = b.int(value);
        b.set_root(body, v);
    };
    let public = b.declare_function("answer", &[], &[], TypeId::INT);
    body(&mut b, public, 42);
    b.set_public(false);
    let private = b.declare_function("secret", &[], &[], TypeId::INT);
    body(&mut b, private, 7);
    // The root module imports it as `l` and declares a function of the same name as the private one.
    b.begin_module("main", true);
    b.import_module("lib", "l");
    let own = b.declare_function("secret", &[], &[], TypeId::INT);
    body(&mut b, own, 1);
    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let mbody = b.function_body(main);
    b.begin_body(mbody);
    let answer = b.call_member("l", "answer", &[]);
    let mine = b.call_named("secret", &[]);
    let sum = b.binary(BinOp::Add, answer, mine);
    b.end_body();
    b.set_root(mbody, sum);
    let m = b.finish().unwrap();
    assert!(m.lookup_value("lib.answer").is_some() && m.lookup_value("secret").is_some());
    assert_eq!(m.func_name(public), "lib.answer");

    // Private items and unknown ones are reported.
    let mut b = HirBuilder::new("program");
    b.begin_module("lib", false);
    b.set_public(false);
    let hidden = b.declare_function("hidden", &[], &[], TypeId::INT);
    body(&mut b, hidden, 0);
    b.begin_module("main", true);
    b.import_module("lib", "lib");
    b.import_module("nowhere", "n");
    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let mbody = b.function_body(main);
    b.begin_body(mbody);
    let a = b.call_member("lib", "hidden", &[]);
    let c = b.call_member("lib", "hiden", &[]);
    let sum = b.binary(BinOp::Add, a, c);
    b.end_body();
    b.set_root(mbody, sum);
    let codes: Vec<String> = b
        .finish()
        .unwrap_err()
        .as_slice()
        .iter()
        .map(|d| format!("{} {}", d.code, d.message))
        .collect();
    assert_eq!(
        codes,
        [
            "E1010 unknown module `nowhere`",
            "E1011 `hidden` is private to module `lib`",
            "E1001 module `lib` has no public `hiden`",
        ]
    );
}

#[test]
fn the_standard_library_loads_on_import_and_round_trips() {
    let mut b = HirBuilder::new("program");
    b.begin_module("main", true);
    for (i, name) in a0_hir::std_modules().into_iter().enumerate() {
        b.import_module(name, &format!("m{i}"));
    }
    // Importing again, or a module a loaded one used, loads nothing new.
    b.import_item("std", "Result");
    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let body = b.function_body(main);
    b.begin_body(body);
    let text = b.string("a,b");
    let comma = b.string(",");
    let call = b.call_member("m6", "split", &[text, comma]);
    let ty = b.fresh_type();
    let parts = b.let_named("parts", ty, call);
    let zero = b.int(0);
    let root = b.block(&[parts], Some(zero));
    b.end_body();
    b.set_root(body, root);
    let m = b.finish().unwrap_or_else(|d| panic!("{}", d.summary()));
    assert_eq!(
        m.func_name(
            m.lookup_value("std.str.split")
                .and_then(|i| match i {
                    Item::Func(f) => Some(f),
                    _ => None,
                })
                .unwrap()
        ),
        "std.str.split"
    );
    let text = print_module(&m);
    let again = print_module(&parse_ok(&text));
    assert_eq!(text, again);
    // The library's C functions are private to it.
    let mut b = HirBuilder::new("program");
    b.import_module("std.str", "str");
    let _ = b.member("str", "a0_std_find");
    let Err(err) = b.finish() else {
        panic!("a private C function was reachable");
    };
    assert!(err.summary().contains("private"), "{}", err.summary());
}
