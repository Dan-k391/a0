//! The canonical text form of a module. `parse(print(m))` reproduces `m` exactly.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::module::{Item, Module};
use crate::types::*;

/// Prints the whole module in canonical form.
pub fn print_module(m: &Module) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "module {}", quote_string(m.name().as_bytes()));
    for item in m.items() {
        out.push('\n');
        match *item {
            Item::Global(g) => print_global(&mut out, m, g),
            Item::Func(f) => print_func(&mut out, m, f),
        }
    }
    out
}

impl std::fmt::Display for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&print_module(self))
    }
}

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&print_module(self))
    }
}

fn print_global(out: &mut String, m: &Module, g: GlobalId) {
    let name = sym_name(m.global_name(g));
    match m.global_type(g) {
        None => {
            let _ = writeln!(out, "data {name} = {}", quote_string(m.global_bytes(g)));
        }
        Some(ty) => {
            let init = format_scalar(ty, m.global_init_bits(g));
            let _ = writeln!(out, "global {name}: {ty} = {init}");
        }
    }
}

fn print_func(out: &mut String, m: &Module, f: FuncId) {
    let name = sym_name(m.func_name(f));
    let ret = match m.func_ret(f) {
        Some(t) => format!(" -> {t}"),
        None => String::new(),
    };
    let Some(body) = m.func_body(f) else {
        let params: Vec<&str> = m.func_params(f).iter().map(|t| t.name()).collect();
        let _ = writeln!(out, "extern func {name}({}){ret}", params.join(", "));
        return;
    };
    let names = Names::for_function(m, f);
    let params: Vec<String> = m
        .func_param_values(f)
        .iter()
        .map(|&v| format!("{}: {}", names.value(v), m.value_type(v)))
        .collect();
    let _ = writeln!(out, "func {name}({}){ret} {{", params.join(", "));
    for &var in m.func_vars(f) {
        let _ = writeln!(out, "  var {}: {}", names.var(var), m.var_type(var));
    }
    print_region(out, m, &names, body, 1);
    out.push_str("}\n");
}

fn print_region(out: &mut String, m: &Module, names: &Names, r: Region, depth: usize) {
    for &inst in m.region_insts(r) {
        indent(out, depth);
        out.push_str(&inst_text(m, names, inst));
        match m.inst_opcode(inst) {
            Opcode::If => {
                out.push_str(" {\n");
                if let Some(then) = m.inst_region(inst, 0) {
                    print_region(out, m, names, then, depth + 1);
                }
                indent(out, depth);
                out.push('}');
                if let Some(els) = m.inst_region(inst, 1) {
                    out.push_str(" else {\n");
                    print_region(out, m, names, els, depth + 1);
                    indent(out, depth);
                    out.push('}');
                }
            }
            Opcode::Loop => {
                out.push_str(" {\n");
                if let Some(body) = m.inst_region(inst, 0) {
                    print_region(out, m, names, body, depth + 1);
                }
                indent(out, depth);
                out.push('}');
            }
            _ => {}
        }
        out.push('\n');
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// One instruction without nested regions, as it appears on its line (no indentation, no newline).
/// `if` and `loop` print only their header (`if %c`, `loop`).
pub(crate) fn inst_text(m: &Module, names: &Names, inst: Inst) -> String {
    let op = m.inst_opcode(inst);
    let args = m.inst_args(inst);
    let a = |i: usize| names.value(args.get(i).copied().unwrap_or(Value::NONE));
    let operands = match op {
        Opcode::Iconst => {
            let ty = m.inst_result(inst).map(|v| m.value_type(v)).unwrap_or(Type::I64);
            format_scalar(ty, m.inst_iconst(inst) as u64)
        }
        Opcode::Fconst => {
            let ty = m.inst_result(inst).map(|v| m.value_type(v)).unwrap_or(Type::F64);
            format_scalar(ty, m.inst_fconst_bits(inst))
        }
        Opcode::Null | Opcode::Break | Opcode::Continue | Opcode::Unreachable | Opcode::Loop => {
            String::new()
        }
        Opcode::Addr => sym_name(m.symbol_name(m.inst_symbol(inst))),
        Opcode::Icmp => format!("{} {}, {}", m.inst_int_cc(inst).name(), a(0), a(1)),
        Opcode::Fcmp => format!("{} {}, {}", m.inst_float_cc(inst).name(), a(0), a(1)),
        Opcode::Call => {
            let callee = sym_name(m.func_name(m.inst_callee(inst)));
            let list: Vec<String> = args.iter().map(|&v| names.value(v)).collect();
            format!("{callee}({})", list.join(", "))
        }
        Opcode::CallIndirect => {
            let list: Vec<String> = args[1..].iter().map(|&v| names.value(v)).collect();
            let sig = m.inst_sig(inst);
            let params: Vec<&str> = m.sig_params(sig).iter().map(|t| t.name()).collect();
            let ret = match m.sig_ret(sig) {
                Some(t) => format!(" -> {t}"),
                None => String::new(),
            };
            format!("{}({}) : ({}){ret}", a(0), list.join(", "), params.join(", "))
        }
        Opcode::Get => names.var(m.inst_var(inst)),
        Opcode::Set => format!("{}, {}", names.var(m.inst_var(inst)), a(0)),
        _ => {
            let list: Vec<String> = args.iter().map(|&v| names.value(v)).collect();
            list.join(", ")
        }
    };
    let mut text = String::new();
    if let Some(r) = m.inst_result(inst) {
        let _ = write!(text, "{}: {} = ", names.value(r), m.value_type(r));
    }
    text.push_str(op.name());
    if !operands.is_empty() {
        text.push(' ');
        text.push_str(&operands);
    }
    text
}

/// Printed names of the values and variables of one function. Named entities print their name;
/// unnamed ones are numbered in order of appearance.
pub(crate) struct Names {
    values: HashMap<Value, String>,
    vars: HashMap<Var, String>,
}

impl Names {
    pub(crate) fn for_function(m: &Module, f: FuncId) -> Names {
        let mut names = Names {
            values: HashMap::new(),
            vars: HashMap::new(),
        };
        let mut defs: Vec<Value> = m.func_param_values(f).to_vec();
        if let Some(body) = m.func_body(f) {
            for inst in insts_in_text_order(m, body) {
                defs.extend(m.inst_result(inst));
            }
        }
        let mut next_value = 0u32;
        for v in defs {
            let text = match m.value_name(v) {
                Some(n) => format!("%{}", ident(n)),
                None => {
                    next_value += 1;
                    format!("%{}", next_value - 1)
                }
            };
            names.values.insert(v, text);
        }
        let mut next_var = 0u32;
        for &var in m.func_vars(f) {
            let text = match m.var_name(var) {
                Some(n) => format!("${}", ident(n)),
                None => {
                    next_var += 1;
                    format!("${}", next_var - 1)
                }
            };
            names.vars.insert(var, text);
        }
        names
    }

    pub(crate) fn value(&self, v: Value) -> String {
        self.values
            .get(&v)
            .cloned()
            .unwrap_or_else(|| format!("%<invalid {}>", v.0))
    }

    pub(crate) fn var(&self, v: Var) -> String {
        self.vars
            .get(&v)
            .cloned()
            .unwrap_or_else(|| format!("$<invalid {}>", v.0))
    }
}

/// All instructions under `r` in the order they appear in the text: each region's instructions in
/// order, with nested regions at the point where their owner appears.
pub fn insts_in_text_order(m: &Module, r: Region) -> Vec<Inst> {
    fn walk(m: &Module, r: Region, out: &mut Vec<Inst>) {
        for &i in m.region_insts(r) {
            out.push(i);
            for n in 0..2 {
                if let Some(sub) = m.inst_region(i, n) {
                    walk(m, sub, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(m, r, &mut out);
    out
}

/// Whether `s` can be printed without quotes.
pub fn is_bare_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

fn ident(s: &str) -> String {
    if is_bare_ident(s) {
        s.to_string()
    } else {
        quote_string(s.as_bytes())
    }
}

fn sym_name(s: &str) -> String {
    format!("@{}", ident(s))
}

/// Quotes bytes: printable ASCII stays as is, `"` and `\` are escaped, everything else is `\XX`.
pub fn quote_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('"');
    for &b in bytes {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:02x}");
            }
        }
    }
    out.push('"');
    out
}

/// Formats a constant of type `ty` given its stored bits.
pub(crate) fn format_scalar(ty: Type, bits: u64) -> String {
    match ty {
        Type::F64 => format_f64(bits),
        Type::F32 => format_f32(bits as u32),
        Type::Ptr => {
            if bits == 0 {
                "null".to_string()
            } else {
                format!("{}", bits as i64)
            }
        }
        _ => format!("{}", bits as i64),
    }
}

fn format_f64(bits: u64) -> String {
    let f = f64::from_bits(bits);
    if f.is_nan() {
        if bits == F64_CANONICAL_NAN {
            "nan".to_string()
        } else {
            format!("nan(0x{bits:016x})")
        }
    } else if f.is_infinite() {
        if f > 0.0 { "inf" } else { "-inf" }.to_string()
    } else {
        format!("{f:?}")
    }
}

fn format_f32(bits: u32) -> String {
    let f = f32::from_bits(bits);
    if f.is_nan() {
        if bits == F32_CANONICAL_NAN {
            "nan".to_string()
        } else {
            format!("nan(0x{bits:08x})")
        }
    } else if f.is_infinite() {
        if f > 0.0 { "inf" } else { "-inf" }.to_string()
    } else {
        format!("{f:?}")
    }
}
