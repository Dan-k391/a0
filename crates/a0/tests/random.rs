//! Random HIR programs: optimized and unoptimized code must agree in every mode.
//!
//! A seeded generator writes well-typed programs that exercise what the optimizer rewrites: small
//! functions with early returns (inlined), arguments with effects, `any` boxed and tested, closures
//! over assigned locals, loops with `break` and `continue`, recursion, enums, structs, nullable
//! references, arrays, maps and strings, and now and then a runtime error. Each program runs
//! interpreted, natively and interpreted under GC stress, each with the optimizer on and off, and all
//! six runs must print the same and exit the same way.
//!
//! `A0_RANDOM_PROGRAMS=n` runs `n` programs (24 by default) and `A0_RANDOM_SEED=s` starts at seed
//! `s`. A failing program is kept in the temporary directory and named in the failure;
//! `A0_RANDOM_KEEP=1` keeps them all.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// xorshift64*, seeded through splitmix64 so that nearby seeds give unrelated programs.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rng((z ^ (z >> 31)) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    Int,
    Bool,
    Float,
    Str,
    Any,
    Arr,
    Map,
    Rec,
    Opt,
    Shape,
    Fun,
    Unit,
}

use Ty::*;

/// The types of parameters, results and locals.
const VALUES: [Ty; 11] = [Int, Bool, Float, Str, Any, Arr, Map, Rec, Opt, Shape, Fun];

impl Ty {
    fn syntax(self) -> &'static str {
        match self {
            Int => "int",
            Bool => "bool",
            Float => "float",
            Str => "string",
            Any => "any",
            Arr => "array<int>",
            Map => "map<string, int>",
            Rec => "R",
            Opt => "R?",
            Shape => "Shape",
            Fun => "fn(int) -> int",
            Unit => "unit",
        }
    }
}

/// Helpers every program starts with; small, so the optimizer inlines them.
const PRELUDE: &str = r#"module "random";

struct R { n: int, s: string }
enum Shape { Dot, Num(int), Txt(string) }
interface Sized { fn size(self) -> int; fn twice(self) -> int; }

impl Sized for R;
impl Sized for Shape;

impl R {
  fn size(self: R) -> int {
    self.n + len(self.s)
  }
  fn twice(self: R) -> int {
    2 * self.size()
  }
}

impl Shape {
  fn size(self: Shape) -> int {
    match self { Dot => 0, Num(n) => n, Txt(t) => len(t), }
  }
  fn twice(self: Shape) -> int {
    2 * self.size()
  }
}

fn size_of(x: Sized) -> int {
  x.size()
}

fn twice_of<T: Sized>(x: T) -> int {
  x.twice()
}

fn note(n: int) -> int {
  print("<" + str(n) + ">");
  n
}

fn clip(s: string) -> string {
  if len(s) > 12 { return substr(s, len(s) - 12, 12); }
  s
}

fn aget(a: array<int>, i: int) -> int {
  if i < 0 || i >= len(a) { return -1; }
  a[i]
}

fn aset(a: array<int>, i: int, v: int) {
  if i >= 0 && i < len(a) { a[i] = v; }
}

fn mget(m: map<string, int>, k: string) -> int {
  if has(m, k) { m[k] } else { 0 }
}

fn show_any(v: any) -> string {
  if v is int { "i" + str(v as int) }
  else if v is bool { "b" + str(v as bool) }
  else if v is float { "f" + str(v as float) }
  else if v is string { "s" + (v as string) }
  else if v is R { "R" + str((v as R).n) }
  else if v is unit { "()" }
  else { "?" }
}

fn show_arr(a: array<int>) -> string {
  let out = "[";
  let i = 0;
  while i < len(a) {
    if i > 0 { out = out + ","; }
    out = out + str(a[i]);
    i = i + 1;
  }
  out + "]"
}

fn show_map(m: map<string, int>) -> string {
  let out = "{";
  let ks = keys(m);
  let i = 0;
  while i < len(ks) {
    out = out + ks[i] + ":" + str(m[ks[i]]) + ";";
    i = i + 1;
  }
  out + "}"
}

fn show_shape(s: Shape) -> string {
  match s { Dot => "Dot", Num(n) => "Num" + str(n), Txt(t) => "Txt" + t, }
}

fn show_opt(o: R?) -> string {
  if let r = o { "R" + str(r.n) + r.s } else { "null" }
}
"#;

/// The string that shows a local of type `ty` named `name`.
fn show(ty: Ty, name: &str) -> String {
    match ty {
        Int | Bool | Float => format!("str({name})"),
        Str => name.to_string(),
        Any => format!("show_any({name})"),
        Arr => format!("show_arr({name})"),
        Map => format!("show_map({name})"),
        Rec | Opt => format!("show_opt({name})"),
        Shape => format!("show_shape({name})"),
        Fun => format!("str({name}(5))"),
        Unit => "\"()\"".to_string(),
    }
}

struct Func {
    name: String,
    params: Vec<Ty>,
    ret: Ty,
    /// The first parameter is recursion fuel: callers pass 0 to 3 and it only decreases.
    fuel: bool,
    /// How many calls one call of it makes at most, counting itself.
    cost: u64,
}

#[derive(Clone)]
struct Local {
    name: String,
    ty: Ty,
    mutable: bool,
}

/// Where code is being generated.
#[derive(Clone)]
struct Ctx {
    locals: Vec<Local>,
    /// The result type `return` needs, or `None` where `return` is not allowed.
    ret: Option<Ty>,
    in_loop: bool,
    in_closure: bool,
    /// Whether a `try` of this function is around the code, so that it may throw.
    in_try: bool,
    /// How many times the code runs per call of the function (loops multiply it).
    times: u64,
}

/// A call costs at most this much more of a function's budget.
const BUDGET: u64 = 3000;
/// A function at most this costly may be a closure value, since calls of closures are not counted.
const CHEAP: u64 = 4;

struct Gen {
    rng: Rng,
    funcs: Vec<Func>,
    /// The function being generated: its index and its cost so far.
    current: usize,
    cost: u64,
    fresh: u32,
}

impl Gen {
    fn name(&mut self, prefix: &str) -> String {
        self.fresh += 1;
        format!("{prefix}{}", self.fresh)
    }

    fn locals_of(&self, ctx: &Ctx, ty: Ty, mutable: bool) -> Vec<String> {
        ctx.locals
            .iter()
            .filter(|l| l.ty == ty && (l.mutable || !mutable))
            .map(|l| l.name.clone())
            .collect()
    }

    /// Functions `current` may call that return `ty`, within the budget at `times`.
    fn callees(&self, ty: Ty, times: u64) -> Vec<usize> {
        (0..self.current)
            .filter(|&f| {
                self.funcs[f].ret == ty && self.cost + self.funcs[f].cost * times <= BUDGET
            })
            .collect()
    }

    fn call(&mut self, f: usize, ctx: &Ctx, d: u32) -> String {
        self.cost += self.funcs[f].cost * ctx.times;
        let params = self.funcs[f].params.clone();
        let mut args = Vec::new();
        for (i, &p) in params.iter().enumerate() {
            if i == 0 && self.funcs[f].fuel {
                args.push(self.rng.below(4).to_string());
            } else {
                args.push(self.expr(p, ctx, d));
            }
        }
        format!("{}({})", self.funcs[f].name, args.join(", "))
    }

    fn leaf(&mut self, ty: Ty, ctx: &Ctx) -> String {
        let locals = self.locals_of(ctx, ty, false);
        if !locals.is_empty() && self.rng.chance(60) {
            return self
                .rng
                .pick(&locals.iter().map(String::as_str).collect::<Vec<_>>())
                .to_string();
        }
        let r = &mut self.rng;
        match ty {
            Int => r
                .pick(&[
                    "0",
                    "1",
                    "2",
                    "3",
                    "7",
                    "-1",
                    "-5",
                    "100",
                    "9223372036854775807",
                ])
                .to_string(),
            Bool => r.pick(&["true", "false"]).to_string(),
            Float => r
                .pick(&["0.0", "1.5", "-2.25", "1000000.0", "0.1", "-0.0"])
                .to_string(),
            Str => r
                .pick(&["\"\"", "\"a\"", "\"xy\"", "\"hello\""])
                .to_string(),
            Any => r
                .pick(&[
                    "(1 as any)",
                    "(\"s\" as any)",
                    "(true as any)",
                    "(() as any)",
                    "(2.5 as any)",
                ])
                .to_string(),
            Arr => r.pick(&["[1, 2, 3]", "new array<int>", "[0]"]).to_string(),
            Map => "new map<string, int>".to_string(),
            Rec => r
                .pick(&["new R { n: 1, s: \"r\" }", "new R { n: -4, s: \"\" }"])
                .to_string(),
            Opt => r
                .pick(&["null<R>", "(new R { n: 2, s: \"o\" } as R?)"])
                .to_string(),
            Shape => r
                .pick(&["Shape::Dot", "Shape::Num(4)", "Shape::Txt(\"t\")"])
                .to_string(),
            Fun => r
                .pick(&["fn(x: int) -> int { x + 1 }", "fn(x: int) -> int { x * x }"])
                .to_string(),
            Unit => "()".to_string(),
        }
    }

    /// An expression of type `ty`, nested at most `d` deep.
    fn expr(&mut self, ty: Ty, ctx: &Ctx, d: u32) -> String {
        if d == 0 || self.rng.chance(25) {
            return self.leaf(ty, ctx);
        }
        let d = d - 1;
        // Shared forms first: a call, an `if`, a `match`, an `if let`.
        let callees = if ctx.in_closure {
            Vec::new()
        } else {
            self.callees(ty, ctx.times)
        };
        match self.rng.below(12) {
            0 if !callees.is_empty() => {
                let f = self.rng.pick(&callees);
                return self.call(f, ctx, d);
            }
            1 => {
                let c = self.expr(Bool, ctx, d);
                let t = self.branch(ty, ctx, d);
                let f = self.branch(ty, ctx, d);
                return format!("(if {c} {{ {t} }} else {{ {f} }})");
            }
            2 => {
                let s = self.expr(Shape, ctx, d);
                let (x, t) = (self.name("v"), self.name("v"));
                let mut num = ctx.clone();
                num.locals.push(Local {
                    name: x.clone(),
                    ty: Int,
                    mutable: false,
                });
                let mut txt = ctx.clone();
                txt.locals.push(Local {
                    name: t.clone(),
                    ty: Str,
                    mutable: false,
                });
                let a = self.expr(ty, ctx, d);
                let b = self.expr(ty, &num, d);
                if self.rng.chance(30) {
                    return format!("(match {s} {{ Num({x}) => {b}, _ => {a}, }})");
                }
                let c = self.expr(ty, &txt, d);
                return format!("(match {s} {{ Dot => {a}, Num({x}) => {b}, Txt({t}) => {c}, }})");
            }
            3 => {
                let o = self.expr(Opt, ctx, d);
                let r = self.name("v");
                let mut inner = ctx.clone();
                inner.locals.push(Local {
                    name: r.clone(),
                    ty: Rec,
                    mutable: false,
                });
                let t = self.branch(ty, &inner, d);
                let f = self.branch(ty, ctx, d);
                return format!("(if let {r} = {o} {{ {t} }} else {{ {f} }})");
            }
            _ => {}
        }
        match ty {
            Int => match self.rng.below(15) {
                0..=2 => {
                    let op = self.rng.pick(&["+", "-", "*"]);
                    format!(
                        "({} {op} {})",
                        self.expr(Int, ctx, d),
                        self.expr(Int, ctx, d)
                    )
                }
                3 => {
                    let op = self.rng.pick(&["/", "%"]);
                    let by = self.rng.pick(&["1", "2", "3", "7", "-3"]);
                    format!("({} {op} {by})", self.expr(Int, ctx, d))
                }
                4 => {
                    let of = self.rng.pick(&[Str, Arr, Map]);
                    format!("len({})", self.expr(of, ctx, d))
                }
                5 => format!("note({})", self.expr(Int, ctx, d)),
                6 => format!("({}).n", self.expr(Rec, ctx, d)),
                7 => format!("({} as int)", self.expr(Float, ctx, d)),
                8 => {
                    let v = self.expr(Any, ctx, d);
                    format!("(if {v} is int {{ {v} as int }} else {{ 0 }})")
                }
                9 => format!(
                    "aget({}, {})",
                    self.expr(Arr, ctx, d),
                    self.expr(Int, ctx, d)
                ),
                10 => format!(
                    "mget({}, {})",
                    self.expr(Map, ctx, d),
                    self.expr(Str, ctx, d)
                ),
                // Closures call no closures, so none can call itself.
                11 => {
                    let funs = match ctx.in_closure {
                        true => Vec::new(),
                        false => self.locals_of(ctx, Fun, false),
                    };
                    match funs.is_empty() {
                        true => self.leaf(Int, ctx),
                        false => {
                            let f = self
                                .rng
                                .pick(&funs.iter().map(String::as_str).collect::<Vec<_>>())
                                .to_string();
                            self.cost += CHEAP * ctx.times;
                            format!("{f}({})", self.expr(Int, ctx, d))
                        }
                    }
                }
                // Methods: called directly, through an interface value, and through a bound.
                12 => {
                    let of = self.rng.pick(&[Rec, Shape]);
                    let x = self.expr(of, ctx, d);
                    match self.rng.below(3) {
                        0 => format!("({x}).size()"),
                        1 => format!("size_of({x})"),
                        _ => format!("twice_of({x})"),
                    }
                }
                13 => {
                    let x = self.expr(Any, ctx, d);
                    format!("(if {x} is Sized {{ ({x} as Sized).twice() }} else {{ 1 }})")
                }
                _ => format!("byte_at(\"hello\", {})", self.rng.below(5)),
            },
            Bool => match self.rng.below(9) {
                0 => {
                    let op = self.rng.pick(&["<", "<=", "==", "!=", ">"]);
                    format!(
                        "({} {op} {})",
                        self.expr(Int, ctx, d),
                        self.expr(Int, ctx, d)
                    )
                }
                1 => {
                    let op = self.rng.pick(&["<", "=="]);
                    format!(
                        "({} {op} {})",
                        self.expr(Str, ctx, d),
                        self.expr(Str, ctx, d)
                    )
                }
                2 => format!(
                    "({} < {})",
                    self.expr(Float, ctx, d),
                    self.expr(Float, ctx, d)
                ),
                3 => format!("!({})", self.expr(Bool, ctx, d)),
                4 => {
                    let op = self.rng.pick(&["&&", "||"]);
                    format!(
                        "({} {op} {})",
                        self.expr(Bool, ctx, d),
                        self.expr(Bool, ctx, d)
                    )
                }
                5 => {
                    let t = self
                        .rng
                        .pick(&["int", "bool", "float", "string", "R", "unit"]);
                    format!("({} is {t})", self.expr(Any, ctx, d))
                }
                6 => format!("({} == {})", self.expr(Any, ctx, d), self.expr(Any, ctx, d)),
                7 => format!("({} == null<R>)", self.expr(Opt, ctx, d)),
                _ => format!(
                    "has({}, {})",
                    self.expr(Map, ctx, d),
                    self.expr(Str, ctx, d)
                ),
            },
            Float => match self.rng.below(3) {
                0 => {
                    let op = self.rng.pick(&["+", "-", "*", "/"]);
                    format!(
                        "({} {op} {})",
                        self.expr(Float, ctx, d),
                        self.expr(Float, ctx, d)
                    )
                }
                1 => format!("({} as float)", self.expr(Int, ctx, d)),
                _ => format!("(-({}))", self.expr(Float, ctx, d)),
            },
            Str => match self.rng.below(7) {
                0 | 1 => format!(
                    "clip({} + {})",
                    self.expr(Str, ctx, d),
                    self.expr(Str, ctx, d)
                ),
                2 => {
                    let of = self.rng.pick(&[Int, Bool, Float]);
                    format!("str({})", self.expr(of, ctx, d))
                }
                3 => format!("({}).s", self.expr(Rec, ctx, d)),
                4 => format!("show_any({})", self.expr(Any, ctx, d)),
                5 => format!("show_shape({})", self.expr(Shape, ctx, d)),
                _ => {
                    let v = self.expr(Any, ctx, d);
                    format!("(if {v} is string {{ {v} as string }} else {{ \"-\" }})")
                }
            },
            Any => {
                let from = self.rng.pick(&[Int, Bool, Float, Str, Rec, Unit, Any]);
                format!("({} as any)", self.expr(from, ctx, d))
            }
            Arr => format!("[{}, {}]", self.expr(Int, ctx, d), self.expr(Int, ctx, d)),
            Map => self.leaf(Map, ctx),
            Rec => format!(
                "new R {{ n: {}, s: {} }}",
                self.expr(Int, ctx, d),
                self.expr(Str, ctx, d)
            ),
            Opt => format!("({} as R?)", self.expr(Rec, ctx, d)),
            Shape => match self.rng.below(3) {
                0 => format!("Shape::Num({})", self.expr(Int, ctx, d)),
                1 => format!("Shape::Txt({})", self.expr(Str, ctx, d)),
                _ => "Shape::Dot".to_string(),
            },
            Fun => {
                let named: Vec<usize> = (0..self.current)
                    .filter(|&f| {
                        let f = &self.funcs[f];
                        f.params == [Int] && f.ret == Int && f.cost <= CHEAP && !f.fuel
                    })
                    .collect();
                if !named.is_empty() && self.rng.chance(30) {
                    return self.funcs[self.rng.pick(&named)].name.clone();
                }
                self.closure(ctx, d)
            }
            Unit => "()".to_string(),
        }
    }

    /// The body of an `if` branch: sometimes statements before the value.
    fn branch(&mut self, ty: Ty, ctx: &Ctx, d: u32) -> String {
        if d == 0 || self.rng.chance(70) {
            return self.expr(ty, ctx, d);
        }
        let mut inner = ctx.clone();
        let stmts = self.stmts(&mut inner, 1, d.min(1));
        let value = self.expr(ty, &inner, d);
        format!("{stmts} {value}")
    }

    /// A closure that may assign the locals it captures and return early. It calls no functions,
    /// so calling it costs nothing against the budget.
    fn closure(&mut self, ctx: &Ctx, d: u32) -> String {
        let x = self.name("v");
        let mut inner = ctx.clone();
        inner.in_closure = true;
        inner.in_loop = false;
        inner.in_try = false;
        inner.ret = Some(Int);
        inner.locals.push(Local {
            name: x.clone(),
            ty: Int,
            mutable: true,
        });
        let mut body = String::new();
        let ints = self.locals_of(&inner, Int, true);
        if !ints.is_empty() && self.rng.chance(50) {
            let target = self
                .rng
                .pick(&ints.iter().map(String::as_str).collect::<Vec<_>>())
                .to_string();
            body += &format!(
                "{target} = {} + {target}; ",
                self.expr(Int, &inner, d.min(1))
            );
        }
        if self.rng.chance(30) {
            body += &format!(
                "if {} {{ return {}; }} ",
                self.expr(Bool, &inner, d.min(1)),
                self.expr(Int, &inner, 1)
            );
        }
        let value = self.expr(Int, &inner, d.min(2));
        format!("fn({x}: int) -> int {{ {body}{value} }}")
    }

    /// Up to `n` statements, which may declare locals in `ctx`.
    fn stmts(&mut self, ctx: &mut Ctx, n: usize, d: u32) -> String {
        let count = self.rng.below(n + 1);
        let mut out = Vec::new();
        for _ in 0..count {
            out.push(self.stmt(ctx, d));
        }
        out.join(" ")
    }

    fn block(&mut self, ctx: &Ctx, n: usize, d: u32) -> String {
        let mut inner = ctx.clone();
        let body = self.stmts(&mut inner, n, d);
        format!("{{ {body} }}")
    }

    fn stmt(&mut self, ctx: &mut Ctx, d: u32) -> String {
        let e = 3;
        loop {
            match self.rng.below(17) {
                0..=2 => {
                    let ty = self.rng.pick(&VALUES);
                    let name = self.name("v");
                    let init = self.expr(ty, ctx, e);
                    ctx.locals.push(Local {
                        name: name.clone(),
                        ty,
                        mutable: true,
                    });
                    return if self.rng.chance(50) {
                        format!("let {name}: {} = {init};", ty.syntax())
                    } else {
                        format!("let {name} = {init};")
                    };
                }
                3 | 4 => {
                    let ty = self.rng.pick(&VALUES);
                    let targets = self.locals_of(ctx, ty, true);
                    if targets.is_empty() {
                        continue;
                    }
                    let t = self
                        .rng
                        .pick(&targets.iter().map(String::as_str).collect::<Vec<_>>())
                        .to_string();
                    let v = self.expr(ty, ctx, e);
                    return match ty {
                        Str => format!("{t} = clip({v});"),
                        _ => format!("{t} = {v};"),
                    };
                }
                5 => return format!("print(\"[\" + {} + \"]\");", self.expr(Str, ctx, e)),
                6 if d > 0 => {
                    let c = self.expr(Bool, ctx, e);
                    let t = self.block(ctx, 3, d - 1);
                    if self.rng.chance(50) {
                        return format!("if {c} {t}");
                    }
                    let f = self.block(ctx, 3, d - 1);
                    return format!("if {c} {t} else {f}");
                }
                7 if d > 0 && ctx.times < 100 => {
                    let i = self.name("i");
                    let k = self.rng.below(5) + 1;
                    let mut inner = ctx.clone();
                    inner.in_loop = true;
                    inner.times *= k as u64 + 1;
                    inner.locals.push(Local {
                        name: i.clone(),
                        ty: Int,
                        mutable: false,
                    });
                    let body = self.stmts(&mut inner, 4, d - 1);
                    if self.rng.chance(50) {
                        return format!("let {i} = 0; while {i} < {k} {{ {i} = {i} + 1; {body} }}");
                    }
                    return format!(
                        "let {i} = 0; loop {{ {i} = {i} + 1; if {i} > {k} {{ break; }} {body} }}"
                    );
                }
                8 if ctx.in_loop => {
                    let c = self.expr(Bool, ctx, 1);
                    let what = self.rng.pick(&["break", "continue"]);
                    return format!("if {c} {{ {what}; }}");
                }
                9 if ctx.ret.is_some() => {
                    let c = self.expr(Bool, ctx, 1);
                    return match ctx.ret.unwrap() {
                        Unit => format!("if {c} {{ return; }}"),
                        ty => format!("if {c} {{ return {}; }}", self.expr(ty, ctx, 2)),
                    };
                }
                10 => {
                    let arrays = self.locals_of(ctx, Arr, false);
                    if arrays.is_empty() {
                        continue;
                    }
                    let a = self
                        .rng
                        .pick(&arrays.iter().map(String::as_str).collect::<Vec<_>>())
                        .to_string();
                    return match self.rng.below(3) {
                        0 => format!("push({a}, {});", self.expr(Int, ctx, e)),
                        1 => format!("if len({a}) > 0 {{ pop({a}); }}"),
                        _ => format!(
                            "aset({a}, {}, {});",
                            self.expr(Int, ctx, 1),
                            self.expr(Int, ctx, e)
                        ),
                    };
                }
                11 => {
                    let maps = self.locals_of(ctx, Map, false);
                    if maps.is_empty() {
                        continue;
                    }
                    let m = self
                        .rng
                        .pick(&maps.iter().map(String::as_str).collect::<Vec<_>>())
                        .to_string();
                    let k = self.rng.pick(&["\"a\"", "\"b\"", "\"xy\""]);
                    return match self.rng.below(3) {
                        0 => format!("remove({m}, {k});"),
                        _ => format!("{m}[{k}] = {};", self.expr(Int, ctx, e)),
                    };
                }
                12 => {
                    let recs = self.locals_of(ctx, Rec, false);
                    if recs.is_empty() {
                        continue;
                    }
                    let r = self
                        .rng
                        .pick(&recs.iter().map(String::as_str).collect::<Vec<_>>())
                        .to_string();
                    return match self.rng.below(2) {
                        0 => format!("{r}.n = {};", self.expr(Int, ctx, e)),
                        _ => format!("{r}.s = clip({});", self.expr(Str, ctx, e)),
                    };
                }
                13 if d > 0 => {
                    let s = self.expr(Shape, ctx, 2);
                    let x = self.name("v");
                    let mut num = ctx.clone();
                    num.locals.push(Local {
                        name: x.clone(),
                        ty: Int,
                        mutable: false,
                    });
                    let a = self.block(&num, 2, d - 1);
                    let b = self.block(ctx, 2, d - 1);
                    return format!("match {s} {{ Num({x}) => {a}, _ => {b}, }}");
                }
                14 if !ctx.in_closure => {
                    let ty = self.rng.pick(&[Int, Str, Unit, Bool]);
                    let callees = self.callees(ty, ctx.times);
                    if callees.is_empty() {
                        continue;
                    }
                    let f = self.rng.pick(&callees);
                    return format!("{};", self.call(f, ctx, 2));
                }
                // Exceptions, thrown inside a `try` of the same function (possibly from inside its
                // loops, past `break` and `continue` to outer loops, or with `return`).
                15 if d > 0 => {
                    let mut inner = ctx.clone();
                    inner.in_try = true;
                    let body = self.stmts(&mut inner, 3, d - 1);
                    let e = self.name("e");
                    let mut handler = ctx.clone();
                    handler.locals.push(Local {
                        name: e.clone(),
                        ty: Any,
                        mutable: false,
                    });
                    let rest = self.stmts(&mut handler, 2, d - 1);
                    return format!(
                        "try {{ {body} }} catch {e} {{ print(\"[caught \" + show_any({e}) + \"]\"); {rest} }}"
                    );
                }
                16 if ctx.in_try => {
                    let c = self.expr(Bool, ctx, 1);
                    let ty = self.rng.pick(&VALUES);
                    let v = self.expr(ty, ctx, 2);
                    return format!("if {c} {{ throw {v}; }}");
                }
                _ => continue,
            }
        }
    }

    fn function(&mut self, index: usize) -> String {
        let fuel = self.rng.chance(20);
        let mut params = Vec::new();
        if fuel {
            params.push(Int);
        }
        for _ in 0..self.rng.below(3) {
            params.push(self.rng.pick(&VALUES));
        }
        let ret = if self.rng.chance(15) {
            Unit
        } else {
            self.rng.pick(&VALUES)
        };
        let name = format!("f{index}");
        self.current = index;
        self.cost = 1;
        let mut ctx = Ctx {
            locals: Vec::new(),
            ret: Some(ret),
            in_loop: false,
            in_closure: false,
            in_try: false,
            times: 1,
        };
        let mut sig = Vec::new();
        for (i, &p) in params.iter().enumerate() {
            let pname = format!("p{i}");
            sig.push(format!("{pname}: {}", p.syntax()));
            ctx.locals.push(Local {
                name: pname,
                ty: p,
                mutable: !(fuel && i == 0),
            });
        }
        // Small functions are what the optimizer inlines; some get longer bodies.
        let n = if self.rng.chance(60) { 2 } else { 6 };
        // The recursive call comes first, so it sees only the parameters.
        let at_entry = ctx.clone();
        let mut body = self.stmts(&mut ctx, n, 2);
        if fuel {
            let mut args = vec!["p0 - 1".to_string()];
            for &p in &params[1..] {
                args.push(self.expr(p, &at_entry, 1));
            }
            let call = format!("{name}({})", args.join(", "));
            let used = match ret {
                Unit => format!("{call};"),
                _ => format!("let {} = {call};", self.name("v")),
            };
            body = format!("if p0 > 0 {{ {used} }} {body}");
        }
        let tail = match ret {
            Unit => String::new(),
            ty => self.expr(ty, &ctx, 3),
        };
        let cost = if fuel { self.cost * 4 } else { self.cost };
        self.funcs.push(Func {
            name: name.clone(),
            params,
            ret,
            fuel,
            cost,
        });
        let ret_syntax = match ret {
            Unit => String::new(),
            ty => format!(" -> {}", ty.syntax()),
        };
        format!(
            "fn {name}({}){ret_syntax} {{\n  {body}\n  {tail}\n}}\n",
            sig.join(", ")
        )
    }

    /// A runtime error at the end of `main`, in a function the optimizer inlines.
    fn failure(&mut self) -> (String, String) {
        let (def, call) = match self.rng.below(7) {
            0 => (
                "fn fail(a: array<int>, i: int) -> int { a[i] }",
                "fail([1, 2], 2)",
            ),
            1 => ("fn fail(n: int) -> int { 10 / n }", "fail(len(\"\"))"),
            2 => ("fn fail(o: R?) -> int { o!.n }", "fail(null<R>)"),
            3 => ("fn fail(v: any) -> int { v as int }", "fail(\"s\" as any)"),
            4 => (
                "fn fail(n: int) -> int { if n > 0 { panic(\"boom \" + str(n)); } n }",
                "fail(3)",
            ),
            5 => (
                "fn fail(a: array<int>) -> int { pop(a) }",
                "fail(new array<int>)",
            ),
            _ => (
                "fn fail(m: map<string, int>) -> int { m[\"none\"] }",
                "fail(new map<string, int>)",
            ),
        };
        (def.to_string(), format!("print(str({call}) + \"\\n\");"))
    }

    fn program(&mut self) -> String {
        let mut out = String::from(PRELUDE);
        let n = 6 + self.rng.below(8);
        for i in 0..n {
            out.push('\n');
            out += &self.function(i);
        }
        self.current = n;
        self.cost = 1;
        let mut ctx = Ctx {
            locals: Vec::new(),
            ret: None,
            in_loop: false,
            in_closure: false,
            in_try: false,
            times: 1,
        };
        let mut main = Vec::new();
        for i in 0..n {
            if self.funcs[i].ret == Unit {
                main.push(format!(
                    "print(\"f{i}\\n\"); {}",
                    self.call(i, &ctx, 2) + ";"
                ));
                continue;
            }
            let r = format!("r{i}");
            let call = self.call(i, &ctx, 2);
            let ty = self.funcs[i].ret;
            main.push(format!("let {r} = {call};"));
            main.push(format!("print(\"f{i} \" + {} + \"\\n\");", show(ty, &r)));
            ctx.locals.push(Local {
                name: r,
                ty,
                mutable: true,
            });
            if self.rng.chance(30) {
                main.push(self.stmt(&mut ctx, 1));
            }
        }
        if self.rng.chance(30) {
            let (def, call) = self.failure();
            out += &format!("\n{def}\n");
            main.push(call);
        }
        let status = match self.rng.below(4) {
            0 => format!("len({}) % 4", self.expr(Str, &ctx, 1)),
            _ => "0".to_string(),
        };
        out += &format!(
            "\nfn main() -> int {{\n  {}\n  {status}\n}}\n",
            main.join("\n  ")
        );
        out
    }
}

fn generate(seed: u64) -> String {
    Gen {
        rng: Rng::new(seed),
        funcs: Vec::new(),
        current: 0,
        cost: 0,
        fresh: 0,
    }
    .program()
}

/// Runs a command, killing it after a minute so that a miscompiled loop fails the test.
fn run(cmd: &mut Command) -> Output {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let stdout = std::thread::spawn(move || {
        let mut v = Vec::new();
        out.read_to_end(&mut v).ok();
        v
    });
    let stderr = std::thread::spawn(move || {
        let mut v = Vec::new();
        err.read_to_end(&mut v).ok();
        v
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(60) {
            child.kill().ok();
            break child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

/// What a run printed and how it ended.
fn outcome(out: &Output) -> String {
    format!(
        "exit {:?}\n--- stdout\n{}--- stderr\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Runs `file` in all six modes; a description of each that differs from the first.
fn differences(file: &Path, dir: &Path) -> Vec<String> {
    let a0 = env!("CARGO_BIN_EXE_a0");
    let mut runs: Vec<(String, Output)> = Vec::new();
    for opt in ["0", "1"] {
        for stress in ["0", "1"] {
            let out = run(Command::new(a0)
                .arg("run")
                .arg(file)
                .env("A0_OPT", opt)
                .env("A0_GC_STRESS", stress));
            runs.push((
                format!("interpreted, A0_OPT={opt}, A0_GC_STRESS={stress}"),
                out,
            ));
        }
        let exe = dir.join(format!(
            "{}-{opt}{}",
            file.file_stem().unwrap().to_string_lossy(),
            std::env::consts::EXE_SUFFIX
        ));
        let build = run(Command::new(a0)
            .arg("build")
            .arg(file)
            .arg("-o")
            .arg(&exe)
            .env("A0_OPT", opt));
        let out = if build.status.success() {
            run(&mut Command::new(&exe))
        } else {
            build
        };
        runs.push((format!("native, A0_OPT={opt}"), out));
    }
    let reference = outcome(&runs[0].1);
    let mut out = Vec::new();
    // A program the generator got wrong fails to compile: that is a bug in the generator.
    if runs[0].1.status.code() == Some(1)
        && !runs[0].1.stderr.is_empty()
        && runs[0].1.stdout.is_empty()
    {
        out.push(format!("does not compile:\n{reference}"));
    }
    for (mode, run) in &runs[1..] {
        let o = outcome(run);
        if o != reference {
            out.push(format!(
                "{mode} differs from {}:\n{o}\nexpected:\n{reference}",
                runs[0].0
            ));
        }
    }
    out
}

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn optimized_and_unoptimized_random_programs_agree_in_every_mode() {
    let count = env_number("A0_RANDOM_PROGRAMS", 24);
    let first = env_number("A0_RANDOM_SEED", 0);
    let dir: PathBuf = std::env::temp_dir().join(format!("a0-random-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let seeds: Vec<u64> = (first..first + count).collect();
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8);
    let keep = std::env::var("A0_RANDOM_KEEP").is_ok_and(|v| v == "1");
    let failures = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for chunk in seeds.chunks(seeds.len().div_ceil(workers).max(1)) {
            let (dir, failures) = (&dir, &failures);
            s.spawn(move || {
                for &seed in chunk {
                    let file = dir.join(format!("seed{seed}.hir"));
                    std::fs::write(&file, generate(seed)).unwrap();
                    let diffs = differences(&file, dir);
                    if diffs.is_empty() && !keep {
                        std::fs::remove_file(&file).ok();
                    } else {
                        failures.lock().unwrap().push(format!(
                            "seed {seed} ({}):\n{}",
                            file.display(),
                            diffs.join("\n")
                        ));
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    if keep {
        println!("programs kept in {}", dir.display());
    } else if failures.is_empty() {
        std::fs::remove_dir_all(&dir).ok();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn the_generator_is_deterministic_and_varied() {
    assert_eq!(generate(7), generate(7));
    assert_ne!(generate(7), generate(8));
}
