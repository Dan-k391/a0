//! The IR verifier. It reports every problem it finds as a diagnostic and never panics.

use std::collections::HashSet;

use a0_diag::{Diagnostic, Diagnostics};

use crate::module::{Module, region_terminates};
use crate::print::{Names, inst_text};
use crate::types::*;

/// Checks a whole module. An empty result means the module is well-formed.
pub fn verify(m: &Module) -> Diagnostics {
    let mut diags = Diagnostics::new();
    for g in 0..m.global_count() {
        let g = GlobalId(g);
        if m.global_name(g).is_empty() {
            diags.push(
                Diagnostic::error("ir.name", "global has an empty name")
                    .with_span_opt(m.global_span(g)),
            );
        }
        if m.global_type(g) == Some(Type::Ptr) && m.global_init_bits(g) != 0 {
            diags.push(
                Diagnostic::error(
                    "ir.type",
                    &format!(
                        "pointer global @{} must be initialized to null",
                        m.global_name(g)
                    ),
                )
                .with_span_opt(m.global_span(g)),
            );
        }
    }
    for f in 0..m.func_count() {
        let f = FuncId(f);
        if m.func_name(f).is_empty() {
            diags.push(
                Diagnostic::error("ir.name", "function has an empty name")
                    .with_span_opt(m.func_span(f)),
            );
        }
        if m.func_body(f).is_some() {
            FuncVerifier::new(m, f, &mut diags).run();
        }
    }
    diags
}

/// Checks that the module has a valid entry point: a defined `func @main() -> i32`.
pub fn check_entry(m: &Module) -> Diagnostics {
    let mut diags = Diagnostics::new();
    match m.lookup_func("main") {
        None => diags.error("ir.entry", "module has no function @main", None),
        Some(f) => {
            if m.func_is_extern(f) {
                diags.error(
                    "ir.entry",
                    "@main must be defined in the module",
                    m.func_span(f),
                );
            } else if !m.func_params(f).is_empty() || m.func_ret(f) != Some(Type::I32) {
                diags.error(
                    "ir.entry",
                    "@main must have the signature () -> i32",
                    m.func_span(f),
                );
            }
        }
    }
    diags
}

struct FuncVerifier<'a> {
    m: &'a Module,
    f: FuncId,
    names: Names,
    diags: &'a mut Diagnostics,
    visible: HashSet<Value>,
    loop_depth: u32,
}

impl<'a> FuncVerifier<'a> {
    fn new(m: &'a Module, f: FuncId, diags: &'a mut Diagnostics) -> FuncVerifier<'a> {
        FuncVerifier {
            m,
            f,
            names: Names::for_function(m, f),
            diags,
            visible: HashSet::new(),
            loop_depth: 0,
        }
    }

    fn fname(&self) -> &str {
        self.m.func_name(self.f)
    }

    fn func_error(&mut self, code: &str, msg: &str) {
        let msg = format!("in @{}: {msg}", self.fname());
        let span = self.m.func_span(self.f);
        self.diags
            .push(Diagnostic::error(code, &msg).with_span_opt(span));
    }

    fn inst_error(&mut self, inst: Inst, code: &str, msg: &str) {
        let msg = format!("in @{}: {msg}", self.fname());
        let note = format!("instruction: {}", inst_text(self.m, &self.names, inst));
        let d = Diagnostic::error(code, &msg)
            .with_span_opt(self.m.inst_span(inst))
            .with_note(&note);
        self.diags.push(d);
    }

    fn run(&mut self) {
        let m = self.m;
        self.check_names();
        for &p in m.func_param_values(self.f) {
            self.visible.insert(p);
        }
        let Some(body) = m.func_body(self.f) else {
            return;
        };
        self.region(body);
        if m.func_ret(self.f).is_some() && !region_terminates(m, body) {
            self.func_error(
                "ir.return",
                "the end of the function is reachable but it must return a value",
            );
        }
    }

    fn check_names(&mut self) {
        let m = self.m;
        let mut seen = HashSet::new();
        let mut values: Vec<Value> = m.func_param_values(self.f).to_vec();
        if let Some(body) = m.func_body(self.f) {
            for inst in crate::print::insts_in_text_order(m, body) {
                values.extend(m.inst_result(inst));
            }
        }
        for v in values {
            if let Some(name) = m.value_name(v)
                && !seen.insert(name.to_string())
            {
                self.func_error(
                    "ir.name",
                    &format!("value name %{name} is used more than once"),
                );
            }
        }
        let mut seen = HashSet::new();
        for &var in m.func_vars(self.f) {
            if let Some(name) = m.var_name(var)
                && !seen.insert(name.to_string())
            {
                self.func_error(
                    "ir.name",
                    &format!("variable name ${name} is used more than once"),
                );
            }
        }
    }

    fn region(&mut self, r: Region) {
        let insts = self.m.region_insts(r).to_vec();
        let mut defined = Vec::new();
        for inst in insts {
            self.inst(inst);
            if let Some(v) = self.m.inst_result(inst) {
                self.visible.insert(v);
                defined.push(v);
            }
        }
        for v in defined {
            self.visible.remove(&v);
        }
    }

    fn ty(&self, v: Value) -> Type {
        self.m.value_type(v)
    }

    fn inst(&mut self, inst: Inst) {
        let m = self.m;
        let op = m.inst_opcode(inst);
        let args = m.inst_args(inst).to_vec();
        for &a in &args {
            if a.index() >= m.values.len() || m.value_func(a) != self.f {
                self.inst_error(inst, "ir.scope", "operand belongs to another function");
                return;
            }
            if !self.visible.contains(&a) {
                let name = self.names.value(a);
                self.inst_error(
                    inst,
                    "ir.scope",
                    &format!("{name} is used where it is not visible (it must be defined earlier in this region or an enclosing one)"),
                );
                return;
            }
        }
        let result = m.inst_result(inst).map(|v| self.ty(v));
        let expect_args = |n: usize| args.len() == n;
        let arity_ok = match op {
            Opcode::Iconst
            | Opcode::Fconst
            | Opcode::Null
            | Opcode::Addr
            | Opcode::Get
            | Opcode::Loop
            | Opcode::Break
            | Opcode::Continue
            | Opcode::Unreachable => expect_args(0),
            Opcode::Set | Opcode::If | Opcode::Load => expect_args(1),
            op if op.is_unary() || op.is_cast() => expect_args(1),
            Opcode::Icmp | Opcode::Fcmp | Opcode::Store | Opcode::Ptradd => expect_args(2),
            op if op.is_int_binary() || op.is_float_binary() => expect_args(2),
            Opcode::Select => expect_args(3),
            Opcode::Return => args.len() <= 1,
            Opcode::Call => true,
            Opcode::CallIndirect => !args.is_empty(),
            _ => true,
        };
        if !arity_ok {
            self.inst_error(
                inst,
                "ir.type",
                &format!("wrong number of operands for {op}"),
            );
            return;
        }
        let produces = match op {
            Opcode::Store
            | Opcode::Set
            | Opcode::If
            | Opcode::Loop
            | Opcode::Break
            | Opcode::Continue
            | Opcode::Return
            | Opcode::Unreachable => Some(false),
            Opcode::Call | Opcode::CallIndirect => None,
            _ => Some(true),
        };
        match (produces, result) {
            (Some(true), None) => {
                self.inst_error(inst, "ir.type", &format!("{op} must produce a value"));
                return;
            }
            (Some(false), Some(_)) => {
                self.inst_error(inst, "ir.type", &format!("{op} does not produce a value"));
                return;
            }
            _ => {}
        }

        let t = |i: usize| self.ty(args[i]);
        match op {
            Opcode::Iconst => {
                if !result.unwrap().is_int() {
                    self.inst_error(inst, "ir.type", "iconst must produce an integer type");
                }
            }
            Opcode::Fconst => {
                let ty = result.unwrap();
                if !ty.is_float() {
                    self.inst_error(inst, "ir.type", "fconst must produce a float type");
                } else if ty == Type::F32 && m.inst_fconst_bits(inst) > u32::MAX as u64 {
                    self.inst_error(inst, "ir.type", "f32 constant has more than 32 bits");
                }
            }
            Opcode::Null => self.expect_result(inst, result, Type::Ptr),
            Opcode::Addr => {
                let sym = m.inst_symbol(inst);
                let valid = match sym.kind {
                    SymbolKind::Func => sym.index < m.func_count(),
                    SymbolKind::Global => sym.index < m.global_count(),
                };
                if !valid {
                    self.inst_error(inst, "ir.symbol", "addr refers to an unknown symbol");
                }
                self.expect_result(inst, result, Type::Ptr);
            }
            op if op.is_int_binary() => {
                if t(0) != t(1) || !t(0).is_int() {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!(
                            "{op} needs two operands of the same integer type, found {} and {}",
                            t(0),
                            t(1)
                        ),
                    );
                } else {
                    self.expect_result(inst, result, t(0));
                }
            }
            op if op.is_float_binary() => {
                if t(0) != t(1) || !t(0).is_float() {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!(
                            "{op} needs two operands of the same float type, found {} and {}",
                            t(0),
                            t(1)
                        ),
                    );
                } else {
                    self.expect_result(inst, result, t(0));
                }
            }
            Opcode::Ineg | Opcode::Not => {
                if !t(0).is_int() {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!("{op} needs an integer operand, found {}", t(0)),
                    );
                } else {
                    self.expect_result(inst, result, t(0));
                }
            }
            Opcode::Fneg => {
                if !t(0).is_float() {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!("fneg needs a float operand, found {}", t(0)),
                    );
                } else {
                    self.expect_result(inst, result, t(0));
                }
            }
            Opcode::Icmp => {
                if t(0) != t(1) || !(t(0).is_int() || t(0) == Type::Ptr) {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!("icmp needs two integers or two pointers of the same type, found {} and {}", t(0), t(1)),
                    );
                }
                self.expect_result(inst, result, Type::I1);
            }
            Opcode::Fcmp => {
                if t(0) != t(1) || !t(0).is_float() {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!(
                            "fcmp needs two floats of the same type, found {} and {}",
                            t(0),
                            t(1)
                        ),
                    );
                }
                self.expect_result(inst, result, Type::I1);
            }
            op if op.is_cast() => self.cast(inst, op, t(0), result.unwrap()),
            Opcode::Select => {
                if t(0) != Type::I1 {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!("select condition must be i1, found {}", t(0)),
                    );
                } else if t(1) != t(2) {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!(
                            "select operands must have the same type, found {} and {}",
                            t(1),
                            t(2)
                        ),
                    );
                } else {
                    self.expect_result(inst, result, t(1));
                }
            }
            Opcode::Load => {
                if t(0) != Type::Ptr {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!("load address must be ptr, found {}", t(0)),
                    );
                }
            }
            Opcode::Store => {
                if t(1) != Type::Ptr {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!("store address must be ptr, found {}", t(1)),
                    );
                }
            }
            Opcode::Ptradd => {
                if t(0) != Type::Ptr || t(1) != Type::I64 {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!(
                            "ptradd needs a ptr and an i64 offset, found {} and {}",
                            t(0),
                            t(1)
                        ),
                    );
                }
                self.expect_result(inst, result, Type::Ptr);
            }
            Opcode::Call => {
                let callee = m.inst_callee(inst);
                if callee.index() >= m.funcs.len() {
                    self.inst_error(inst, "ir.symbol", "call to an unknown function");
                    return;
                }
                let params = m.func_params(callee).to_vec();
                let ret = m.func_ret(callee);
                let name = format!("@{}", m.func_name(callee));
                self.check_call(inst, &name, &params, ret, &args, result);
            }
            Opcode::CallIndirect => {
                if t(0) != Type::Ptr {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!("call_indirect callee must be ptr, found {}", t(0)),
                    );
                }
                let sig = m.inst_sig(inst);
                if sig.index() >= m.sigs.len() {
                    self.inst_error(inst, "ir.type", "call_indirect has an invalid signature");
                    return;
                }
                let params = m.sig_params(sig).to_vec();
                let ret = m.sig_ret(sig);
                self.check_call(inst, "the callee", &params, ret, &args[1..], result);
            }
            Opcode::Get | Opcode::Set => {
                let var = m.inst_var(inst);
                if var.index() >= m.vars.len() || m.var_func(var) != self.f {
                    self.inst_error(inst, "ir.scope", "variable belongs to another function");
                    return;
                }
                let vt = m.var_type(var);
                if op == Opcode::Get {
                    self.expect_result(inst, result, vt);
                } else if t(0) != vt {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!(
                            "cannot set a variable of type {vt} to a value of type {}",
                            t(0)
                        ),
                    );
                }
            }
            Opcode::If => {
                if t(0) != Type::I1 {
                    self.inst_error(
                        inst,
                        "ir.type",
                        &format!("if condition must be i1, found {}", t(0)),
                    );
                }
                match m.inst_region(inst, 0) {
                    Some(then) => self.region(then),
                    None => self.inst_error(inst, "ir.control", "if has no then-region"),
                }
                if let Some(els) = m.inst_region(inst, 1) {
                    self.region(els);
                }
            }
            Opcode::Loop => match m.inst_region(inst, 0) {
                Some(body) => {
                    self.loop_depth += 1;
                    self.region(body);
                    self.loop_depth -= 1;
                }
                None => self.inst_error(inst, "ir.control", "loop has no body"),
            },
            Opcode::Break | Opcode::Continue => {
                if self.loop_depth == 0 {
                    self.inst_error(inst, "ir.control", &format!("{op} outside a loop"));
                } else if m.inst_loop_depth(inst) >= self.loop_depth {
                    self.inst_error(
                        inst,
                        "ir.control",
                        &format!(
                            "{op} {} leaves more loops than are open",
                            m.inst_loop_depth(inst)
                        ),
                    );
                }
            }
            Opcode::Return => {
                let expected = m.func_ret(self.f);
                match (expected, args.first()) {
                    (None, Some(_)) => {
                        self.inst_error(inst, "ir.return", "void function returns a value");
                    }
                    (Some(t), None) => {
                        self.inst_error(
                            inst,
                            "ir.return",
                            &format!("return needs a value of type {t}"),
                        );
                    }
                    (Some(t), Some(&v)) if self.ty(v) != t => {
                        self.inst_error(
                            inst,
                            "ir.return",
                            &format!("returns {} but the function returns {t}", self.ty(v)),
                        );
                    }
                    _ => {}
                }
            }
            Opcode::Unreachable => {}
            _ => {}
        }
    }

    fn expect_result(&mut self, inst: Inst, result: Option<Type>, expected: Type) {
        if let Some(found) = result
            && found != expected
        {
            let op = self.m.inst_opcode(inst);
            self.inst_error(
                inst,
                "ir.type",
                &format!("{op} produces {expected}, but the result is declared as {found}"),
            );
        }
    }

    fn check_call(
        &mut self,
        inst: Inst,
        callee: &str,
        params: &[Type],
        ret: Option<Type>,
        args: &[Value],
        result: Option<Type>,
    ) {
        if params.len() != args.len() {
            self.inst_error(
                inst,
                "ir.type",
                &format!(
                    "{callee} takes {} arguments but {} were given",
                    params.len(),
                    args.len()
                ),
            );
            return;
        }
        for (i, (&p, &a)) in params.iter().zip(args).enumerate() {
            if self.ty(a) != p {
                self.inst_error(
                    inst,
                    "ir.type",
                    &format!(
                        "argument {} of {callee} must be {p}, found {}",
                        i + 1,
                        self.ty(a)
                    ),
                );
            }
        }
        match (ret, result) {
            (None, Some(_)) => self.inst_error(
                inst,
                "ir.type",
                &format!("{callee} returns nothing, so the call has no result"),
            ),
            (Some(t), None) => self.inst_error(
                inst,
                "ir.type",
                &format!("{callee} returns {t}; give the call a result"),
            ),
            (Some(t), Some(r)) if t != r => self.inst_error(
                inst,
                "ir.type",
                &format!("{callee} returns {t}, but the result is declared as {r}"),
            ),
            _ => {}
        }
    }

    fn cast(&mut self, inst: Inst, op: Opcode, from: Type, to: Type) {
        let int_bits = |t: Type| if t.is_int() { t.bits() } else { None };
        let ok = match op {
            Opcode::Zext | Opcode::Sext => {
                matches!((int_bits(from), int_bits(to)), (Some(a), Some(b)) if a < b)
            }
            Opcode::Trunc => matches!((int_bits(from), int_bits(to)), (Some(a), Some(b)) if a > b),
            Opcode::Sitofp | Opcode::Uitofp => from.is_int() && to.is_float(),
            Opcode::Fptosi | Opcode::Fptoui => from.is_float() && to.is_int(),
            Opcode::Fext => from == Type::F32 && to == Type::F64,
            Opcode::Ftrunc => from == Type::F64 && to == Type::F32,
            Opcode::Bitcast => matches!(
                (from, to),
                (Type::I32, Type::F32)
                    | (Type::F32, Type::I32)
                    | (Type::I64, Type::F64)
                    | (Type::F64, Type::I64)
            ),
            Opcode::Ptrtoint => from == Type::Ptr && to.is_int(),
            Opcode::Inttoptr => from.is_int() && to == Type::Ptr,
            _ => false,
        };
        if !ok {
            self.inst_error(inst, "ir.type", &format!("cannot {op} from {from} to {to}"));
        }
    }
}
