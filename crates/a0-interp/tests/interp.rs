use a0_diag::SourceId;
use a0_interp::{HostSymbols, Interpreter, Trap};
use a0_ir::{FuncId, Module};

fn module(text: &str) -> Module {
    a0_ir::parse_and_verify(text, SourceId(0)).unwrap_or_else(|d| panic!("{}", d.summary()))
}

fn interp(text: &str) -> Interpreter {
    Interpreter::new(module(text), &HostSymbols::new()).unwrap_or_else(|d| panic!("{}", d.summary()))
}

fn func(i: &Interpreter, name: &str) -> FuncId {
    i.module().lookup_func(name).unwrap()
}

fn call(i: &mut Interpreter, name: &str, args: &[u64]) -> Result<Option<u64>, Trap> {
    let f = func(i, name);
    i.call(f, args)
}

fn call_f64(i: &mut Interpreter, name: &str, args: &[f64]) -> f64 {
    let raw: Vec<u64> = args.iter().map(|a| a.to_bits()).collect();
    f64::from_bits(call(i, name, &raw).unwrap().unwrap())
}

fn call_i64(i: &mut Interpreter, name: &str, args: &[i64]) -> i64 {
    let raw: Vec<u64> = args.iter().map(|&a| a as u64).collect();
    call(i, name, &raw).unwrap().unwrap() as i64
}

const FIB: &str = "module \"fib\"
func @fib(%n: f64) -> f64 {
  var $r: f64
  %two: f64 = fconst 2.0
  %small: i1 = fcmp lt %n, %two
  if %small {
    set $r, %n
  } else {
    %one: f64 = fconst 1.0
    %a: f64 = fsub %n, %one
    %fa: f64 = call @fib(%a)
    %b: f64 = fsub %n, %two
    %fb: f64 = call @fib(%b)
    %s: f64 = fadd %fa, %fb
    set $r, %s
  }
  %res: f64 = get $r
  return %res
}
";

#[test]
fn recursion_and_variables() {
    let mut i = interp(FIB);
    assert_eq!(call_f64(&mut i, "fib", &[20.0]), 6765.0);
}

#[test]
fn loops_break_and_continue() {
    // Sum of the even numbers below n.
    let mut i = interp(
        "module \"m\"
func @sum_even(%n: i64) -> i64 {
  var $i: i64
  var $acc: i64
  loop {
    %i: i64 = get $i
    %done: i1 = icmp sge %i, %n
    if %done {
      break
    }
    %one: i64 = iconst 1
    %next: i64 = iadd %i, %one
    set $i, %next
    %bit: i64 = and %i, %one
    %odd: i1 = icmp ne %bit, %one
    if %odd {
    } else {
      continue
    }
    %acc: i64 = get $acc
    %acc2: i64 = iadd %acc, %i
    set $acc, %acc2
  }
  %r: i64 = get $acc
  return %r
}
",
    );
    assert_eq!(call_i64(&mut i, "sum_even", &[10]), 2 + 4 + 6 + 8);
    assert_eq!(call_i64(&mut i, "sum_even", &[0]), 0);
}

#[test]
fn integer_semantics() {
    let mut i = interp(
        "module \"m\"
func @wrap8(%a: i8, %b: i8) -> i8 {
  %r: i8 = iadd %a, %b
  return %r
}
func @udiv8(%a: i8, %b: i8) -> i8 {
  %r: i8 = udiv %a, %b
  return %r
}
func @sdiv(%a: i32, %b: i32) -> i32 {
  %r: i32 = sdiv %a, %b
  return %r
}
func @srem(%a: i64, %b: i64) -> i64 {
  %r: i64 = srem %a, %b
  return %r
}
func @shl(%a: i32, %b: i32) -> i32 {
  %r: i32 = shl %a, %b
  return %r
}
func @lshr(%a: i8, %b: i8) -> i8 {
  %r: i8 = lshr %a, %b
  return %r
}
func @ult(%a: i8, %b: i8) -> i1 {
  %r: i1 = icmp ult %a, %b
  return %r
}
func @zext(%a: i8) -> i64 {
  %r: i64 = zext %a
  return %r
}
func @sext1(%a: i1) -> i32 {
  %r: i32 = sext %a
  return %r
}
func @sat(%a: f64) -> i32 {
  %r: i32 = fptosi %a
  return %r
}
",
    );
    // 100 + 100 wraps to -56 in i8.
    assert_eq!(call(&mut i, "wrap8", &[100, 100]).unwrap(), Some(-56i64 as u64));
    // -1 as an unsigned byte is 255; 255 / 2 = 127.
    assert_eq!(call(&mut i, "udiv8", &[-1i64 as u64, 2]).unwrap(), Some(127));
    assert_eq!(call(&mut i, "sdiv", &[-7i64 as u64, 2]).unwrap(), Some(-3i64 as u64));
    assert_eq!(call(&mut i, "sdiv", &[1, 0]).unwrap_err().message, "integer division by zero");
    let min = i32::MIN as i64 as u64;
    assert_eq!(call(&mut i, "sdiv", &[min, -1i64 as u64]).unwrap_err().message, "integer overflow");
    assert_eq!(call(&mut i, "srem", &[i64::MIN as u64, -1i64 as u64]).unwrap(), Some(0));
    // Shift amounts are taken modulo the width.
    assert_eq!(call(&mut i, "shl", &[1, 33]).unwrap(), Some(2));
    assert_eq!(call(&mut i, "lshr", &[-128i64 as u64, 7]).unwrap(), Some(1));
    assert_eq!(call(&mut i, "ult", &[1, -1i64 as u64]).unwrap(), Some(1));
    assert_eq!(call(&mut i, "zext", &[-1i64 as u64]).unwrap(), Some(255));
    assert_eq!(call(&mut i, "sext1", &[1]).unwrap(), Some(-1i64 as u64));
    assert_eq!(call(&mut i, "sat", &[1e20f64.to_bits()]).unwrap(), Some(i32::MAX as u64));
    assert_eq!(call(&mut i, "sat", &[f64::NAN.to_bits()]).unwrap(), Some(0));
}

#[test]
fn memory_globals_and_data() {
    let mut i = interp(
        "module \"m\"
data @bytes = \"\\01\\02\\03\\ff\"
global @counter: i64 = 40
func @bump() -> i64 {
  %p: ptr = addr @counter
  %v: i64 = load %p
  %two: i64 = iconst 2
  %w: i64 = iadd %v, %two
  store %w, %p
  %r: i64 = load %p
  return %r
}
func @byte(%i: i64) -> i8 {
  %p: ptr = addr @bytes
  %q: ptr = ptradd %p, %i
  %b: i8 = load %q
  return %b
}
",
    );
    assert_eq!(call_i64(&mut i, "bump", &[]), 42);
    assert_eq!(call_i64(&mut i, "bump", &[]), 44);
    assert_eq!(call_i64(&mut i, "byte", &[2]), 3);
    assert_eq!(call_i64(&mut i, "byte", &[3]), -1);
}

#[test]
fn calls_into_c() {
    let mut i = interp(
        "module \"m\"
extern func @pow(f64, f64) -> f64
extern func @a0_alloc(i64) -> ptr
extern func @a0_free(ptr)
func @cube(%x: f64) -> f64 {
  %three: f64 = fconst 3.0
  %r: f64 = call @pow(%x, %three)
  return %r
}
func @heap() -> i32 {
  %n: i64 = iconst 16
  %p: ptr = call @a0_alloc(%n)
  %v: i32 = iconst 1234
  %eight: i64 = iconst 8
  %q: ptr = ptradd %p, %eight
  store %v, %q
  %r: i32 = load %q
  call @a0_free(%p)
  return %r
}
",
    );
    assert_eq!(call_f64(&mut i, "cube", &[3.0]), 27.0);
    assert_eq!(call_i64(&mut i, "heap", &[]), 1234);
}

#[test]
fn indirect_calls_to_ir_and_c() {
    let mut i = interp(
        "module \"m\"
extern func @sqrt(f64) -> f64
func @twice(%x: f64) -> f64 {
  %r: f64 = fadd %x, %x
  return %r
}
func @apply(%f: ptr, %x: f64) -> f64 {
  %r: f64 = call_indirect %f(%x) : (f64) -> f64
  return %r
}
func @pick(%which: i1) -> ptr {
  %a: ptr = addr @twice
  %b: ptr = addr @sqrt
  %r: ptr = select %which, %a, %b
  return %r
}
",
    );
    let pick = func(&i, "pick");
    let apply = func(&i, "apply");
    let twice = i.call(pick, &[1]).unwrap().unwrap();
    let sqrt = i.call(pick, &[0]).unwrap().unwrap();
    assert_eq!(f64::from_bits(i.call(apply, &[twice, 21f64.to_bits()]).unwrap().unwrap()), 42.0);
    assert_eq!(f64::from_bits(i.call(apply, &[sqrt, 81f64.to_bits()]).unwrap().unwrap()), 9.0);
}

#[test]
fn deep_recursion_traps_as_stack_overflow() {
    let mut i = interp(
        "module \"m\"
func @down(%n: i64) -> i64 {
  %r: i64 = call @down(%n)
  return %r
}
",
    );
    i.set_max_depth(500);
    let err = call(&mut i, "down", &[1]).unwrap_err();
    assert_eq!(err.message, "stack overflow");
}

#[test]
fn unreachable_traps() {
    let mut i = interp("module \"m\"\nfunc @f() {\n  unreachable\n}\n");
    assert_eq!(call(&mut i, "f", &[]).unwrap_err().code, a0_runtime::trap::UNREACHABLE);
}

#[test]
fn unresolved_externs_are_reported_up_front() {
    let m = module(
        "module \"m\"
extern func @mystery(i64) -> i64
extern func @unused(i64) -> i64
func @f() -> i64 {
  %x: i64 = iconst 1
  %r: i64 = call @mystery(%x)
  return %r
}
",
    );
    let err = Interpreter::new(m, &HostSymbols::new()).err().unwrap();
    let text = err.summary();
    assert!(text.contains("unresolved external function @mystery"), "{text}");
    assert!(!text.contains("@unused"), "{text}");
}

#[test]
fn f32_externs_are_rejected() {
    let m = module(
        "module \"m\"
extern func @sinf(f32) -> f32
func @f() -> f32 {
  %x: f32 = fconst 1.0
  %r: f32 = call @sinf(%x)
  return %r
}
",
    );
    let mut syms = HostSymbols::new();
    syms.add("sinf", 1);
    let err = Interpreter::new(m, &syms).err().unwrap();
    assert!(err.summary().contains("it has f32 parameters"), "{}", err.summary());
}
