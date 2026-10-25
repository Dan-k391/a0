use a0_diag::{SourceId, SourceMap};
use a0_ir::*;

fn parse_text(text: &str) -> Module {
    match parse(text, SourceId(0)) {
        Ok(m) => m,
        Err(d) => {
            let mut sm = SourceMap::new();
            sm.add("input.a0ir", text);
            panic!("parse failed:\n{}", d.render(&sm));
        }
    }
}

fn errors_of(text: &str) -> String {
    let mut sm = SourceMap::new();
    let id = sm.add("t.a0ir", text);
    match parse_and_verify(text, id) {
        Ok(_) => String::new(),
        Err(d) => d.render(&sm),
    }
}

#[test]
fn canonical_text_round_trips_byte_for_byte() {
    let text = include_str!("data/all_ops.a0ir").replace("\r\n", "\n");
    let m = parse_text(&text);
    let diags = verify(&m);
    assert!(diags.is_empty(), "{}", diags.summary());
    assert_eq!(print_module(&m), text);
}

#[test]
fn reparsing_preserves_structure() {
    let text = include_str!("data/all_ops.a0ir").replace("\r\n", "\n");
    let m1 = parse_text(&text);
    let m2 = parse_text(&print_module(&m1));
    assert_eq!(m1.func_count(), m2.func_count());
    assert_eq!(m1.global_count(), m2.global_count());
    for f in 0..m1.func_count() {
        let f = FuncId(f);
        assert_eq!(m1.func_name(f), m2.func_name(f));
        assert_eq!(m1.func_params(f), m2.func_params(f));
        assert_eq!(m1.func_ret(f), m2.func_ret(f));
    }
    assert_eq!(m1.global_bytes(GlobalId(0)), b"Hello, \"world\"\\\n\0");
    let tiny = m1.lookup_global("tiny").unwrap();
    assert_eq!(m1.global_init_bits(tiny), (1e-7f32).to_bits() as u64);
}

#[test]
fn non_canonical_text_is_normalized() {
    let text = "
        ; comments and odd spacing are fine
        module \"m\"
        func @f(%x: i8) -> i8 {   ; trailing comment
            %y: i8 = iconst 255
            %z: i8 = iadd %x,%y
            return %z
        }
    ";
    let m = parse_text(text);
    assert_eq!(
        print_module(&m),
        "module \"m\"\n\nfunc @f(%x: i8) -> i8 {\n  %y: i8 = iconst -1\n  %z: i8 = iadd %x, %y\n  return %z\n}\n"
    );
}

/// Builds a function with unnamed values, clashing name hints and nested control flow.
fn build_sample() -> Module {
    let mut b = Builder::new("sample");
    let printd = b.declare_extern("printd", &[Type::F64], Some(Type::F64));
    let fib = b.declare_function("fib", &[Type::F64], Some(Type::F64));
    let main = b.declare_function("main", &[], Some(Type::I32));
    b.add_data("msg", b"hi\n");

    b.begin_function(fib);
    let n = b.param(0);
    b.name_value(n, "n");
    let r = b.declare_var("r", Type::F64);
    let two = b.fconst(Type::F64, 2.0);
    let small = b.fcmp(FloatCC::Lt, n, two);
    b.if_(small);
    b.set(r, n);
    b.else_();
    let one = b.fconst(Type::F64, 1.0);
    let a = b.fsub(n, one);
    b.name_value(a, "x");
    let fa = b.call(fib, &[a]).unwrap();
    let c = b.fsub(n, two);
    b.name_value(c, "x");
    let fb = b.call(fib, &[c]).unwrap();
    let s = b.fadd(fa, fb);
    b.set(r, s);
    b.end_if();
    let v = b.get(r);
    b.ret(Some(v));
    b.end_function();

    b.begin_function(main);
    let ten = b.fconst(Type::F64, 10.0);
    let f10 = b.call(fib, &[ten]).unwrap();
    b.call(printd, &[f10]);
    let zero = b.iconst(Type::I32, 0);
    b.ret(Some(zero));
    b.end_function();
    b.finish().unwrap()
}

#[test]
fn builder_output_is_canonical() {
    let m = build_sample();
    let expected = "module \"sample\"

extern func @printd(f64) -> f64

func @fib(%n: f64) -> f64 {
  var $r: f64
  %0: f64 = fconst 2.0
  %1: i1 = fcmp lt %n, %0
  if %1 {
    set $r, %n
  } else {
    %2: f64 = fconst 1.0
    %x: f64 = fsub %n, %2
    %3: f64 = call @fib(%x)
    %x.1: f64 = fsub %n, %0
    %4: f64 = call @fib(%x.1)
    %5: f64 = fadd %3, %4
    set $r, %5
  }
  %6: f64 = get $r
  return %6
}

func @main() -> i32 {
  %0: f64 = fconst 10.0
  %1: f64 = call @fib(%0)
  %2: f64 = call @printd(%1)
  %3: i32 = iconst 0
  return %3
}

data @msg = \"hi\\0a\"
";
    assert_eq!(print_module(&m), expected);
    let again = parse_text(&print_module(&m));
    assert_eq!(print_module(&again), expected);
    assert_eq!(
        again.value_name(again.func_param_values(again.lookup_func("fib").unwrap())[0]),
        Some("n")
    );
}

#[test]
fn builder_reports_misuse_instead_of_panicking() {
    let mut b = Builder::new("bad");
    b.else_();
    b.break_();
    let f = b.declare_function("f", &[], None);
    b.begin_function(f);
    b.break_();
    b.loop_();
    b.break_out(1);
    b.end_if();
    b.end_function();
    let err = b.finish().unwrap_err();
    let text = err.summary();
    assert!(
        text.contains("break_out of more loops than are open"),
        "{text}"
    );
    assert!(text.contains("else_ without a matching if_"), "{text}");
    assert!(text.contains("break built outside a function"), "{text}");
    assert!(text.contains("break_ outside a loop"), "{text}");
    assert!(text.contains("end_if without a matching if_"), "{text}");
    assert!(text.contains("ended with unclosed if/loop"), "{text}");
}

#[test]
fn builder_reports_duplicate_symbols() {
    let mut b = Builder::new("dup");
    b.declare_function("f", &[], None);
    b.declare_extern("f", &[Type::I64], None);
    let err = b.finish().unwrap_err();
    assert!(err.summary().contains("symbol @f is already defined"));
}

#[test]
fn is_terminated_tracks_returns_and_ifs() {
    let mut b = Builder::new("t");
    let f = b.declare_function("f", &[Type::I1], Some(Type::I32));
    b.begin_function(f);
    let c = b.param(0);
    assert!(!b.is_terminated());
    b.if_(c);
    let one = b.iconst(Type::I32, 1);
    b.ret(Some(one));
    assert!(b.is_terminated());
    b.else_();
    assert!(!b.is_terminated());
    let two = b.iconst(Type::I32, 2);
    b.ret(Some(two));
    b.end_if();
    // Both branches return, so the function body cannot fall through.
    assert!(b.is_terminated());
    b.end_function();
    assert!(b.finish().is_ok());
}

#[test]
fn verifier_reports_type_errors_with_spans() {
    let text = "module \"m\"
func @f(%a: i64, %b: f64) -> i64 {
  %c: i64 = iadd %a, %b
  return %c
}
";
    let out = errors_of(text);
    assert!(
        out.contains("iadd needs two operands of the same integer type, found i64 and f64"),
        "{out}"
    );
    assert!(out.contains("t.a0ir:3:3"), "{out}");
}

#[test]
fn verifier_reports_scoping_errors() {
    let text = "module \"m\"
func @f(%c: i1) -> i64 {
  if %c {
    %x: i64 = iconst 1
  }
  return %x
}
";
    // The parser already rejects names that are out of scope.
    let out = errors_of(text);
    assert!(out.contains("unknown value %x"), "{out}");

    // The builder can express it; the verifier catches it.
    let mut b = Builder::new("m");
    let f = b.declare_function("f", &[Type::I1], Some(Type::I64));
    b.begin_function(f);
    let c = b.param(0);
    b.if_(c);
    let x = b.iconst(Type::I64, 1);
    b.end_if();
    b.ret(Some(x));
    b.end_function();
    let err = b.finish().unwrap_err().summary();
    assert!(err.contains("%1 is used where it is not visible"), "{err}");
}

#[test]
fn verifier_requires_returns() {
    let text = "module \"m\"
func @f(%c: i1) -> i64 {
  if %c {
    %x: i64 = iconst 1
    return %x
  }
}
";
    let out = errors_of(text);
    assert!(
        out.contains("the end of the function is reachable"),
        "{out}"
    );

    // An infinite loop never falls through.
    let ok = "module \"m\"
func @f() -> i64 {
  loop {
  }
}
";
    assert_eq!(errors_of(ok), "");
}

#[test]
fn verifier_checks_calls_control_and_variables() {
    let text = "module \"m\"
extern func @g(i64) -> i64
func @f(%a: f64) {
  var $v: i64
  %r: i64 = call @g(%a)
  set $v, %a
  break
  return %a
}
";
    let out = errors_of(text);
    assert!(
        out.contains("argument 1 of @g must be i64, found f64"),
        "{out}"
    );
    assert!(
        out.contains("cannot set a variable of type i64 to a value of type f64"),
        "{out}"
    );
    assert!(out.contains("break outside a loop"), "{out}");
    assert!(out.contains("void function returns a value"), "{out}");
}

#[test]
fn verifier_checks_casts() {
    let text = "module \"m\"
func @f(%a: i64) -> i32 {
  %b: i32 = zext %a
  return %b
}
";
    let out = errors_of(text);
    assert!(out.contains("cannot zext from i64 to i32"), "{out}");
}

#[test]
fn parser_reports_syntax_errors() {
    let out = errors_of("module \"m\"\nfunc @f() {\n  %x: i64 = frobnicate\n}\n");
    assert!(
        out.contains("expected an opcode, found 'frobnicate'"),
        "{out}"
    );
    assert!(out.contains("t.a0ir:3:13"), "{out}");

    let out =
        errors_of("module \"m\"\nfunc @f() {\n  %x: i64 = iconst 1\n  %x: i64 = iconst 2\n}\n");
    assert!(out.contains("value name is already used"), "{out}");

    let out = errors_of("module \"m\"\nfunc @f() -> i8 {\n  %x: i8 = iconst 300\n  return %x\n}\n");
    assert!(out.contains("300 does not fit in i8"), "{out}");

    let out = errors_of("module \"m\"\nfunc @f() {\n  iadd %a, %b\n}\n");
    assert!(out.contains("'iadd' produces a value"), "{out}");
}

#[test]
fn parser_resolves_forward_references() {
    let text = "module \"m\"
func @a() -> i64 {
  %r: i64 = call @b()
  return %r
}
func @b() -> i64 {
  %p: ptr = addr @later
  %r: i64 = load %p
  return %r
}
global @later: i64 = 7
";
    assert_eq!(errors_of(text), "");
}

#[test]
fn entry_point_check() {
    let ok =
        parse_text("module \"m\"\nfunc @main() -> i32 {\n  %0: i32 = iconst 0\n  return %0\n}\n");
    assert!(check_entry(&ok).is_empty());
    let bad = parse_text("module \"m\"\nfunc @main() {\n}\n");
    assert!(check_entry(&bad).summary().contains("() -> i32"));
    let none = parse_text("module \"m\"\n");
    assert!(check_entry(&none).summary().contains("no function @main"));
}

#[test]
fn quoted_names_round_trip() {
    let mut b = Builder::new("q");
    let f = b.declare_function("unary-", &[Type::F64], Some(Type::F64));
    b.begin_function(f);
    let p = b.param(0);
    b.name_value(p, "123");
    let v = b.fneg(p);
    b.name_value(v, "with space");
    b.ret(Some(v));
    b.end_function();
    let m = b.finish().unwrap();
    let text = print_module(&m);
    assert!(
        text.contains("func @\"unary-\"(%\"123\": f64) -> f64"),
        "{text}"
    );
    assert!(
        text.contains("%\"with space\": f64 = fneg %\"123\""),
        "{text}"
    );
    let again = parse_text(&text);
    assert_eq!(print_module(&again), text);
}
