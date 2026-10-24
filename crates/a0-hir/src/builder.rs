//! Construction of HIR modules.
//!
//! Expressions are built bottom-up and typed as they are built, so a frontend can ask for the type of
//! anything it has made. A type error is recorded as a diagnostic and the offending node gets the type
//! `never`, which fits everywhere, so one mistake is reported once. `finish` runs the checker.

use std::collections::HashSet;

use a0_diag::{Diagnostic, Diagnostics, Span};

use crate::check::check;
use crate::model::*;

/// Names that cannot be used for locals and items because the text format reserves them.
pub const KEYWORDS: &[&str] = &[
    "module",
    "struct",
    "enum",
    "global",
    "extern",
    "fn",
    "let",
    "if",
    "else",
    "while",
    "loop",
    "break",
    "continue",
    "return",
    "match",
    "new",
    "null",
    "true",
    "false",
    "as",
    "is",
    "unit",
    "bool",
    "int",
    "float",
    "string",
    "any",
    "array",
    "map",
    "never",
    "float_bits",
    "_",
    "len",
    "push",
    "pop",
    "has",
    "remove",
    "keys",
    "substr",
    "byte_at",
    "str",
    "print",
    "eprint",
    "exit",
    "panic",
    "clock",
];

pub(crate) fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&s)
}

/// Builds a [`HirModule`]. See the crate documentation for an example.
pub struct HirBuilder {
    m: HirModule,
    diags: Diagnostics,
    span: Option<Span>,
}

impl HirBuilder {
    pub fn new(module_name: &str) -> HirBuilder {
        HirBuilder::from_module(HirModule::new(module_name))
    }

    /// Continues building a module (for example a runtime prelude parsed from text).
    pub fn from_module(m: HirModule) -> HirBuilder {
        HirBuilder {
            m,
            diags: Diagnostics::new(),
            span: None,
        }
    }

    pub fn module(&self) -> &HirModule {
        &self.m
    }

    /// The span attached to everything built from now on.
    pub fn set_span(&mut self, span: Option<Span>) {
        self.span = span;
    }

    fn error(&mut self, message: &str) {
        self.diags
            .push(Diagnostic::error("hir.type", message).with_span_opt(self.span));
    }

    /// Checks the module and returns it, or every problem found.
    pub fn finish(self) -> Result<HirModule, Diagnostics> {
        let mut diags = self.diags;
        diags.extend(check(&self.m));
        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(self.m)
        }
    }

    // ---------------------------------------------------------------- types

    pub fn array_type(&mut self, elem: TypeId) -> TypeId {
        self.m.intern(TypeKind::Array(elem))
    }

    pub fn map_type(&mut self, key: TypeId, value: TypeId) -> TypeId {
        if !matches!(key, TypeId::INT | TypeId::BOOL | TypeId::STRING) {
            let k = self.m.type_name(key);
            self.error(&format!("map keys must be int, bool or string, not {k}"));
        }
        self.m.intern(TypeKind::Map(key, value))
    }

    pub fn fn_type(&mut self, params: &[TypeId], ret: TypeId) -> TypeId {
        self.m.intern(TypeKind::Fn(params.to_vec(), ret))
    }

    pub fn struct_type(&mut self, s: StructId) -> TypeId {
        self.m.intern(TypeKind::Struct(s))
    }

    pub fn enum_type(&mut self, e: EnumId) -> TypeId {
        self.m.intern(TypeKind::Enum(e))
    }

    // ---------------------------------------------------------------- declarations

    fn item_name(&mut self, name: &str) {
        if !is_identifier(name) {
            self.error(&format!(
                "'{name}' is not a valid name (identifiers only, no keywords)"
            ));
        }
    }

    fn add_type_name(&mut self, name: &str, item: Item) {
        self.item_name(name);
        if self.m.type_names.insert(name.to_string(), item).is_some() {
            self.error(&format!("type '{name}' is declared twice"));
        }
        self.m.items.push(item);
    }

    fn add_value_name(&mut self, name: &str, item: Item) {
        self.item_name(name);
        if self.m.value_names.insert(name.to_string(), item).is_some() {
            self.error(&format!("'{name}' is declared twice"));
        }
        self.m.items.push(item);
    }

    /// Declares a struct; add its fields with [`HirBuilder::add_field`] (structs may refer to
    /// themselves).
    pub fn declare_struct(&mut self, name: &str) -> StructId {
        let s = StructId(self.m.structs.len() as u32);
        self.m.structs.push(StructDef {
            name: name.to_string(),
            fields: Vec::new(),
            span: self.span,
        });
        self.add_type_name(name, Item::Struct(s));
        s
    }

    pub fn add_field(&mut self, s: StructId, name: &str, ty: TypeId) -> u32 {
        if !is_identifier(name) {
            self.error(&format!("'{name}' is not a valid field name"));
        }
        let def = &mut self.m.structs[s.index()];
        if def.fields.iter().any(|(n, _)| n == name) {
            let sname = def.name.clone();
            self.error(&format!("struct {sname} has two fields named '{name}'"));
        }
        let def = &mut self.m.structs[s.index()];
        def.fields.push((name.to_string(), ty));
        def.fields.len() as u32 - 1
    }

    pub fn declare_enum(&mut self, name: &str) -> EnumId {
        let e = EnumId(self.m.enums.len() as u32);
        self.m.enums.push(EnumDef {
            name: name.to_string(),
            variants: Vec::new(),
            span: self.span,
        });
        self.add_type_name(name, Item::Enum(e));
        e
    }

    pub fn add_variant(&mut self, e: EnumId, name: &str, fields: &[TypeId]) -> u32 {
        if !is_identifier(name) {
            self.error(&format!("'{name}' is not a valid variant name"));
        }
        if self.m.variant_index(e, name).is_some() {
            let ename = self.m.enum_name(e).to_string();
            self.error(&format!("enum {ename} has two variants named '{name}'"));
        }
        let def = &mut self.m.enums[e.index()];
        def.variants.push(VariantDef {
            name: name.to_string(),
            fields: fields.to_vec(),
        });
        def.variants.len() as u32 - 1
    }

    pub fn declare_global(&mut self, name: &str, ty: TypeId) -> GlobalId {
        let g = GlobalId(self.m.globals.len() as u32);
        self.m.globals.push(GlobalDef {
            name: name.to_string(),
            ty,
            span: self.span,
        });
        self.add_value_name(name, Item::Global(g));
        g
    }

    /// Declares a C function. Its types must be `int`, `float`, `bool` or `unit`.
    pub fn declare_extern(&mut self, name: &str, params: &[TypeId], ret: TypeId) -> ExternId {
        for &t in params.iter().chain([&ret]) {
            if !matches!(t, TypeId::INT | TypeId::FLOAT | TypeId::BOOL | TypeId::UNIT) {
                let tn = self.m.type_name(t);
                self.error(&format!(
                    "extern fn {name} uses {tn}; externs take int, float, bool or unit"
                ));
            }
        }
        let x = ExternId(self.m.externs.len() as u32);
        self.m.externs.push(ExternDef {
            name: name.to_string(),
            params: params.to_vec(),
            ret,
            span: self.span,
        });
        self.add_value_name(name, Item::Extern(x));
        x
    }

    /// Declares a function with named parameters. Build its body expression and attach it with
    /// [`HirBuilder::set_root`] on [`HirBuilder::function_body`].
    pub fn declare_function(
        &mut self,
        name: &str,
        param_names: &[&str],
        params: &[TypeId],
        ret: TypeId,
    ) -> FuncId {
        let f = FuncId(self.m.funcs.len() as u32);
        let body = BodyId(self.m.bodies.len() as u32);
        self.m.funcs.push(FuncDef {
            name: name.to_string(),
            body,
            span: self.span,
            local_names: HashSet::new(),
        });
        self.m.bodies.push(BodyData {
            params: Vec::new(),
            ret,
            root: None,
            parent: None,
            func: f,
        });
        self.add_value_name(name, Item::Func(f));
        self.add_params(body, param_names, params);
        f
    }

    pub fn function_body(&self, f: FuncId) -> BodyId {
        self.m.funcs[f.index()].body
    }

    /// Creates the body of a closure defined inside `parent`; turn it into a value with
    /// [`HirBuilder::lambda`] once its root is set.
    pub fn new_closure_body(
        &mut self,
        parent: BodyId,
        param_names: &[&str],
        params: &[TypeId],
        ret: TypeId,
    ) -> BodyId {
        let func = self.m.bodies[parent.index()].func;
        let body = BodyId(self.m.bodies.len() as u32);
        self.m.bodies.push(BodyData {
            params: Vec::new(),
            ret,
            root: None,
            parent: Some(parent),
            func,
        });
        self.add_params(body, param_names, params);
        body
    }

    fn add_params(&mut self, body: BodyId, names: &[&str], types: &[TypeId]) {
        if names.len() != types.len() {
            self.error("parameter names and types differ in number");
        }
        for (i, &ty) in types.iter().enumerate() {
            let name = names.get(i).copied().unwrap_or("p");
            let l = self.new_local(body, name, ty);
            self.m.bodies[body.index()].params.push(l);
        }
    }

    pub fn param(&self, body: BodyId, i: u32) -> LocalId {
        self.m.bodies[body.index()].params[i as usize]
    }

    /// Creates a local of `body`. Names are made unique within the function (`x`, `x_1`, ...) and
    /// adjusted if they are not valid identifiers; locals are referred to by handle, so this only
    /// affects printing.
    pub fn new_local(&mut self, body: BodyId, name: &str, ty: TypeId) -> LocalId {
        if ty == TypeId::NEVER {
            self.error("a local cannot have type never");
        }
        let func = self.m.bodies[body.index()].func;
        let mut base: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) {
            base.insert(0, 'v');
        }
        if KEYWORDS.contains(&base.as_str()) {
            base.push('_');
        }
        let names = &mut self.m.funcs[func.index()].local_names;
        let mut unique = base.clone();
        let mut n = 1;
        while names.contains(&unique) {
            unique = format!("{base}_{n}");
            n += 1;
        }
        names.insert(unique.clone());
        let l = LocalId(self.m.locals.len() as u32);
        self.m.locals.push(LocalData {
            name: unique,
            ty,
            body,
        });
        l
    }

    pub fn set_root(&mut self, body: BodyId, root: ExprId) {
        self.m.bodies[body.index()].root = Some(root);
    }

    // ---------------------------------------------------------------- expressions

    fn push(&mut self, kind: ExprKind, hint: TypeId) -> ExprId {
        let ty = match infer(&self.m, &kind, hint) {
            Ok(t) => t,
            Err(msg) => {
                self.error(&msg);
                TypeId::NEVER
            }
        };
        let e = ExprId(self.m.exprs.len() as u32);
        self.m.exprs.push(ExprData {
            kind,
            ty,
            span: self.span,
        });
        e
    }

    pub fn type_of(&self, e: ExprId) -> TypeId {
        self.m.expr_type(e)
    }

    pub fn unit(&mut self) -> ExprId {
        self.push(ExprKind::Unit, TypeId::UNIT)
    }

    pub fn bool_lit(&mut self, v: bool) -> ExprId {
        self.push(ExprKind::Bool(v), TypeId::BOOL)
    }

    pub fn int(&mut self, v: i64) -> ExprId {
        self.push(ExprKind::Int(v), TypeId::INT)
    }

    pub fn float(&mut self, v: f64) -> ExprId {
        self.push(ExprKind::Float(v.to_bits()), TypeId::FLOAT)
    }

    pub fn float_bits(&mut self, bits: u64) -> ExprId {
        self.push(ExprKind::Float(bits), TypeId::FLOAT)
    }

    pub fn string(&mut self, s: &str) -> ExprId {
        self.push(ExprKind::Str(s.to_string()), TypeId::STRING)
    }

    /// The null reference of a reference type.
    pub fn null(&mut self, ty: TypeId) -> ExprId {
        self.push(ExprKind::Null, ty)
    }

    pub fn local(&mut self, l: LocalId) -> ExprId {
        self.push(ExprKind::Local(l), TypeId::NEVER)
    }

    pub fn global(&mut self, g: GlobalId) -> ExprId {
        self.push(ExprKind::Global(g), TypeId::NEVER)
    }

    /// A function used as a closure value.
    pub fn func(&mut self, f: FuncId) -> ExprId {
        let body = self.m.func_body(f);
        self.intern_body_type(body);
        self.push(ExprKind::Func(f), TypeId::NEVER)
    }

    fn intern_body_type(&mut self, body: BodyId) -> TypeId {
        let params: Vec<TypeId> = self
            .m
            .body_params(body)
            .iter()
            .map(|&p| self.m.local_type(p))
            .collect();
        let ret = self.m.body_ret(body);
        self.fn_type(&params, ret)
    }

    pub fn let_(&mut self, l: LocalId, init: ExprId) -> ExprId {
        self.push(ExprKind::Let(l, init), TypeId::UNIT)
    }

    pub fn set_local(&mut self, l: LocalId, value: ExprId) -> ExprId {
        self.push(ExprKind::SetLocal(l, value), TypeId::UNIT)
    }

    pub fn set_global(&mut self, g: GlobalId, value: ExprId) -> ExprId {
        self.push(ExprKind::SetGlobal(g, value), TypeId::UNIT)
    }

    pub fn set_field(&mut self, object: ExprId, field: u32, value: ExprId) -> ExprId {
        self.push(ExprKind::SetField(object, field, value), TypeId::UNIT)
    }

    /// `target[index] = value` for arrays and maps.
    pub fn set_index(&mut self, target: ExprId, index: ExprId, value: ExprId) -> ExprId {
        self.push(ExprKind::SetIndex(target, index, value), TypeId::UNIT)
    }

    pub fn unary(&mut self, op: UnOp, a: ExprId) -> ExprId {
        self.push(ExprKind::Unary(op, a), TypeId::NEVER)
    }

    pub fn binary(&mut self, op: BinOp, a: ExprId, b: ExprId) -> ExprId {
        self.push(ExprKind::Binary(op, a, b), TypeId::NEVER)
    }

    pub fn and(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.push(ExprKind::And(a, b), TypeId::BOOL)
    }

    pub fn or(&mut self, a: ExprId, b: ExprId) -> ExprId {
        self.push(ExprKind::Or(a, b), TypeId::BOOL)
    }

    pub fn call(&mut self, f: FuncId, args: &[ExprId]) -> ExprId {
        self.push(ExprKind::Call(f, args.to_vec()), TypeId::NEVER)
    }

    pub fn call_extern(&mut self, x: ExternId, args: &[ExprId]) -> ExprId {
        self.push(ExprKind::CallExtern(x, args.to_vec()), TypeId::NEVER)
    }

    /// Calls a closure value.
    pub fn call_value(&mut self, callee: ExprId, args: &[ExprId]) -> ExprId {
        self.push(ExprKind::CallValue(callee, args.to_vec()), TypeId::NEVER)
    }

    /// A closure made from a body created with [`HirBuilder::new_closure_body`].
    pub fn lambda(&mut self, body: BodyId) -> ExprId {
        self.intern_body_type(body);
        self.push(ExprKind::Lambda(body), TypeId::NEVER)
    }

    /// Constructs a struct; `fields` are in declaration order.
    pub fn new_struct(&mut self, s: StructId, fields: &[ExprId]) -> ExprId {
        self.struct_type(s);
        self.push(ExprKind::New(s, fields.to_vec()), TypeId::NEVER)
    }

    pub fn field(&mut self, object: ExprId, field: u32) -> ExprId {
        self.push(ExprKind::Field(object, field), TypeId::NEVER)
    }

    /// Field access by name; reports an error if the object is not a struct with that field.
    pub fn field_named(&mut self, object: ExprId, name: &str) -> ExprId {
        let idx = match self.m.kind(self.m.expr_type(object)) {
            TypeKind::Struct(s) => self.m.struct_field_index(*s, name),
            _ => None,
        };
        match idx {
            Some(i) => self.field(object, i),
            None => {
                let t = self.m.type_name(self.m.expr_type(object));
                self.error(&format!("{t} has no field '{name}'"));
                self.push(ExprKind::Field(object, u32::MAX), TypeId::NEVER)
            }
        }
    }

    pub fn variant(&mut self, e: EnumId, v: u32, args: &[ExprId]) -> ExprId {
        self.enum_type(e);
        self.push(ExprKind::Variant(e, v, args.to_vec()), TypeId::NEVER)
    }

    /// A `match` arm: `variant` (or `None` for `_`), locals bound to its payload, and the arm body.
    pub fn arm(&mut self, variant: Option<u32>, binds: &[LocalId], body: ExprId) -> ArmId {
        self.m.arms.push(ArmData {
            variant,
            binds: binds.to_vec(),
            body,
        });
        ArmId(self.m.arms.len() as u32 - 1)
    }

    pub fn match_(&mut self, scrutinee: ExprId, arms: &[ArmId]) -> ExprId {
        self.push(ExprKind::Match(scrutinee, arms.to_vec()), TypeId::NEVER)
    }

    /// An array literal; it must not be empty (use [`HirBuilder::new_array`]).
    pub fn array(&mut self, elems: &[ExprId]) -> ExprId {
        let elem = elems
            .first()
            .map(|&e| self.m.expr_type(e))
            .unwrap_or(TypeId::NEVER);
        let ty = self.array_type(elem);
        self.push(ExprKind::Array(elems.to_vec()), ty)
    }

    /// An empty array of type `array_type`.
    pub fn new_array(&mut self, array_type: TypeId) -> ExprId {
        self.push(ExprKind::NewArray, array_type)
    }

    /// An empty map of type `map_type`.
    pub fn new_map(&mut self, map_type: TypeId) -> ExprId {
        self.push(ExprKind::NewMap, map_type)
    }

    pub fn index(&mut self, target: ExprId, index: ExprId) -> ExprId {
        self.push(ExprKind::Index(target, index), TypeId::NEVER)
    }

    pub fn builtin(&mut self, op: Builtin, args: &[ExprId]) -> ExprId {
        if op == Builtin::Keys
            && let Some(&a) = args.first()
            && let TypeKind::Map(k, _) = *self.m.kind(self.m.expr_type(a))
        {
            self.array_type(k);
        }
        self.push(ExprKind::Builtin(op, args.to_vec()), TypeId::NEVER)
    }

    /// `e as ty`.
    pub fn cast(&mut self, e: ExprId, ty: TypeId) -> ExprId {
        self.push(ExprKind::Cast(e), ty)
    }

    /// `e is ty`, for `e: any`.
    pub fn is(&mut self, e: ExprId, ty: TypeId) -> ExprId {
        self.push(ExprKind::Is(e, ty), TypeId::BOOL)
    }

    pub fn if_(&mut self, cond: ExprId, then: ExprId, els: Option<ExprId>) -> ExprId {
        self.push(ExprKind::If(cond, then, els), TypeId::NEVER)
    }

    pub fn while_(&mut self, cond: ExprId, body: ExprId) -> ExprId {
        self.push(ExprKind::While(cond, body), TypeId::UNIT)
    }

    pub fn loop_(&mut self, body: ExprId) -> ExprId {
        self.push(ExprKind::Loop(body), TypeId::NEVER)
    }

    pub fn break_(&mut self) -> ExprId {
        self.push(ExprKind::Break, TypeId::NEVER)
    }

    pub fn continue_(&mut self) -> ExprId {
        self.push(ExprKind::Continue, TypeId::NEVER)
    }

    pub fn return_(&mut self, value: Option<ExprId>) -> ExprId {
        self.push(ExprKind::Return(value), TypeId::NEVER)
    }

    /// `{ items...; tail }`.
    pub fn block(&mut self, items: &[ExprId], tail: Option<ExprId>) -> ExprId {
        self.push(ExprKind::Block(items.to_vec(), tail), TypeId::NEVER)
    }
}

/// Whether a value of type `found` can be used where `expected` is required.
pub(crate) fn fits(found: TypeId, expected: TypeId) -> bool {
    found == expected || found == TypeId::NEVER
}

/// The type of an expression, or why it is ill-typed. `hint` carries the type written in the node
/// itself (the target of `as`, the type of `null` and of empty collections).
pub(crate) fn infer(m: &HirModule, kind: &ExprKind, hint: TypeId) -> Result<TypeId, String> {
    let ty = |e: &ExprId| m.expr_type(*e);
    let name = |t: TypeId| m.type_name(t);
    let expect = |e: &ExprId, want: TypeId, what: &str| -> Result<(), String> {
        if fits(ty(e), want) {
            Ok(())
        } else {
            Err(format!(
                "{what} must be {}, found {}",
                name(want),
                name(ty(e))
            ))
        }
    };
    let args_match = |params: &[TypeId], args: &[ExprId], callee: &str| -> Result<(), String> {
        if params.len() != args.len() {
            return Err(format!(
                "{callee} takes {} arguments, {} given",
                params.len(),
                args.len()
            ));
        }
        for (i, (p, a)) in params.iter().zip(args).enumerate() {
            if !fits(ty(a), *p) {
                return Err(format!(
                    "argument {} of {callee} must be {}, found {}",
                    i + 1,
                    name(*p),
                    name(ty(a))
                ));
            }
        }
        Ok(())
    };
    Ok(match kind {
        ExprKind::Unit => TypeId::UNIT,
        ExprKind::Bool(_) => TypeId::BOOL,
        ExprKind::Int(_) => TypeId::INT,
        ExprKind::Float(_) => TypeId::FLOAT,
        ExprKind::Str(_) => TypeId::STRING,
        ExprKind::Null => {
            if !m.is_reference(hint) {
                return Err(format!("null needs a reference type, not {}", name(hint)));
            }
            hint
        }
        ExprKind::Local(l) => m.local_type(*l),
        ExprKind::Global(g) => m.global_type(*g),
        ExprKind::Func(f) => {
            let body = m.func_body(*f);
            let params: Vec<TypeId> = m
                .body_params(body)
                .iter()
                .map(|&p| m.local_type(p))
                .collect();
            m.type_map
                .get(&TypeKind::Fn(params, m.body_ret(body)))
                .copied()
                .ok_or_else(|| "the function's type was not interned".to_string())?
        }
        ExprKind::Let(l, init) | ExprKind::SetLocal(l, init) => {
            expect(
                init,
                m.local_type(*l),
                &format!("the value of '{}'", m.local_name(*l)),
            )?;
            TypeId::UNIT
        }
        ExprKind::SetGlobal(g, value) => {
            expect(
                value,
                m.global_type(*g),
                &format!("the value of '{}'", m.global_name(*g)),
            )?;
            TypeId::UNIT
        }
        ExprKind::SetField(object, field, value) => {
            let ft = field_type(m, ty(object), *field)?;
            expect(value, ft, "the field's new value")?;
            TypeId::UNIT
        }
        ExprKind::SetIndex(target, index, value) => {
            let (key, elem) = index_types(m, ty(target))?;
            expect(index, key, "the index")?;
            expect(value, elem, "the element")?;
            TypeId::UNIT
        }
        ExprKind::Unary(op, a) => match op {
            UnOp::Neg if matches!(ty(a), TypeId::INT | TypeId::FLOAT | TypeId::NEVER) => ty(a),
            UnOp::Not if fits(ty(a), TypeId::BOOL) => TypeId::BOOL,
            _ => {
                let o = if *op == UnOp::Neg { "-" } else { "!" };
                return Err(format!("cannot apply {o} to {}", name(ty(a))));
            }
        },
        ExprKind::Binary(op, a, b) => binary_type(m, *op, ty(a), ty(b))?,
        ExprKind::And(a, b) | ExprKind::Or(a, b) => {
            expect(a, TypeId::BOOL, "an operand of && or ||")?;
            expect(b, TypeId::BOOL, "an operand of && or ||")?;
            TypeId::BOOL
        }
        ExprKind::Call(f, args) => {
            let body = m.func_body(*f);
            let params: Vec<TypeId> = m
                .body_params(body)
                .iter()
                .map(|&p| m.local_type(p))
                .collect();
            args_match(&params, args, m.func_name(*f))?;
            m.body_ret(body)
        }
        ExprKind::CallExtern(x, args) => {
            let def = &m.externs[x.index()];
            args_match(&def.params, args, &def.name)?;
            def.ret
        }
        ExprKind::CallValue(callee, args) => match m.kind(ty(callee)) {
            TypeKind::Fn(params, ret) => {
                args_match(params, args, "the closure")?;
                *ret
            }
            TypeKind::Never => TypeId::NEVER,
            _ => return Err(format!("cannot call a value of type {}", name(ty(callee)))),
        },
        ExprKind::Lambda(body) => {
            let params: Vec<TypeId> = m
                .body_params(*body)
                .iter()
                .map(|&p| m.local_type(p))
                .collect();
            m.type_map
                .get(&TypeKind::Fn(params, m.body_ret(*body)))
                .copied()
                .ok_or_else(|| "the closure's type was not interned".to_string())?
        }
        ExprKind::New(s, fields) => {
            let def = &m.structs[s.index()];
            if fields.len() != def.fields.len() {
                return Err(format!(
                    "struct {} has {} fields, {} given",
                    def.name,
                    def.fields.len(),
                    fields.len()
                ));
            }
            for ((fname, fty), v) in def.fields.iter().zip(fields) {
                expect(v, *fty, &format!("field '{fname}' of {}", def.name))?;
            }
            m.type_map
                .get(&TypeKind::Struct(*s))
                .copied()
                .ok_or_else(|| "the struct's type was not interned".to_string())?
        }
        ExprKind::Field(object, field) => field_type(m, ty(object), *field)?,
        ExprKind::Variant(e, v, args) => {
            let def = &m.enums[e.index()];
            let Some(var) = def.variants.get(*v as usize) else {
                return Err(format!("enum {} has no variant {v}", def.name));
            };
            args_match(&var.fields, args, &format!("{}::{}", def.name, var.name))?;
            m.type_map
                .get(&TypeKind::Enum(*e))
                .copied()
                .ok_or_else(|| "the enum's type was not interned".to_string())?
        }
        ExprKind::Match(scrutinee, arms) => {
            let st = ty(scrutinee);
            let e = match m.kind(st) {
                TypeKind::Enum(e) => *e,
                TypeKind::Never => return Ok(TypeId::NEVER),
                _ => return Err(format!("match needs an enum, found {}", name(st))),
            };
            let mut result = TypeId::NEVER;
            let mut covered = vec![false; m.variant_count(e) as usize];
            let mut wildcard = false;
            for &a in arms {
                let arm = &m.arms[a.index()];
                match arm.variant {
                    Some(v) => {
                        let Some(fields) = m.enums[e.index()]
                            .variants
                            .get(v as usize)
                            .map(|v| &v.fields)
                        else {
                            return Err(format!("enum {} has no variant {v}", m.enum_name(e)));
                        };
                        if arm.binds.len() != fields.len() {
                            return Err(format!(
                                "variant {} has {} fields, the arm binds {}",
                                m.variant_name(e, v),
                                fields.len(),
                                arm.binds.len()
                            ));
                        }
                        for (&b, &f) in arm.binds.iter().zip(fields) {
                            if m.local_type(b) != f {
                                return Err(format!(
                                    "'{}' binds a {} as {}",
                                    m.local_name(b),
                                    name(f),
                                    name(m.local_type(b))
                                ));
                            }
                        }
                        covered[v as usize] = true;
                    }
                    None => wildcard = true,
                }
                let bt = ty(&arm.body);
                if result == TypeId::NEVER {
                    result = bt;
                } else if bt != TypeId::NEVER && bt != result {
                    // Arms of different types make a statement-like match of type unit.
                    result = TypeId::UNIT;
                }
            }
            if !wildcard && covered.iter().any(|c| !c) {
                let missing: Vec<&str> = covered
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !**c)
                    .map(|(i, _)| m.variant_name(e, i as u32))
                    .collect();
                return Err(format!("match does not cover {}", missing.join(", ")));
            }
            result
        }
        ExprKind::Array(elems) => {
            let Some(first) = elems.first() else {
                return Err("an array literal cannot be empty; use new array<T>".to_string());
            };
            for e in elems {
                expect(e, ty(first), "every array element")?;
            }
            hint
        }
        ExprKind::NewArray => {
            if !matches!(m.kind(hint), TypeKind::Array(_)) {
                return Err(format!("new array needs an array type, not {}", name(hint)));
            }
            hint
        }
        ExprKind::NewMap => {
            if !matches!(m.kind(hint), TypeKind::Map(..)) {
                return Err(format!("new map needs a map type, not {}", name(hint)));
            }
            hint
        }
        ExprKind::Index(target, index) => {
            let (key, elem) = index_types(m, ty(target))?;
            expect(index, key, "the index")?;
            elem
        }
        ExprKind::Builtin(op, args) => builtin_type(m, *op, args)?,
        ExprKind::Cast(e) => {
            let from = ty(e);
            if !cast_allowed(m, from, hint) {
                return Err(format!("cannot convert {} to {}", name(from), name(hint)));
            }
            hint
        }
        ExprKind::Is(e, t) => {
            expect(e, TypeId::ANY, "the operand of is")?;
            if !testable(m, *t) {
                return Err(format!("cannot test whether an any is a {}", name(*t)));
            }
            TypeId::BOOL
        }
        ExprKind::If(cond, then, els) => {
            expect(cond, TypeId::BOOL, "an if condition")?;
            match els {
                // Without else, an if is a statement; the branch's value is discarded.
                None => TypeId::UNIT,
                Some(e) => {
                    let (a, b) = (ty(then), ty(e));
                    if a == TypeId::NEVER {
                        b
                    } else if b == TypeId::NEVER || a == b {
                        a
                    } else {
                        TypeId::UNIT
                    }
                }
            }
        }
        ExprKind::While(cond, _) => {
            expect(cond, TypeId::BOOL, "a while condition")?;
            TypeId::UNIT
        }
        ExprKind::Loop(body) => {
            if contains_break(m, *body) {
                TypeId::UNIT
            } else {
                TypeId::NEVER
            }
        }
        ExprKind::Break | ExprKind::Continue | ExprKind::Return(_) => TypeId::NEVER,
        ExprKind::Block(items, tail) => match tail {
            Some(t) => ty(t),
            None if items.iter().any(|i| ty(i) == TypeId::NEVER) => TypeId::NEVER,
            None => TypeId::UNIT,
        },
    })
}

fn field_type(m: &HirModule, object: TypeId, field: u32) -> Result<TypeId, String> {
    match m.kind(object) {
        TypeKind::Struct(s) => match m.structs[s.index()].fields.get(field as usize) {
            Some((_, t)) => Ok(*t),
            None => Err(format!("struct {} has no field {field}", m.struct_name(*s))),
        },
        TypeKind::Never => Ok(TypeId::NEVER),
        _ => Err(format!("{} has no fields", m.type_name(object))),
    }
}

/// The index and element types of `target[...]`.
fn index_types(m: &HirModule, target: TypeId) -> Result<(TypeId, TypeId), String> {
    match m.kind(target) {
        TypeKind::Array(e) => Ok((TypeId::INT, *e)),
        TypeKind::Map(k, v) => Ok((*k, *v)),
        TypeKind::Never => Ok((TypeId::NEVER, TypeId::NEVER)),
        _ => Err(format!("cannot index a {}", m.type_name(target))),
    }
}

fn binary_type(m: &HirModule, op: BinOp, a: TypeId, b: TypeId) -> Result<TypeId, String> {
    let t = if a == TypeId::NEVER { b } else { a };
    if a != b && a != TypeId::NEVER && b != TypeId::NEVER {
        return Err(format!(
            "operands of {} must have the same type, found {} and {}",
            op.symbol(),
            m.type_name(a),
            m.type_name(b)
        ));
    }
    let ok = match op {
        BinOp::Add => matches!(
            t,
            TypeId::INT | TypeId::FLOAT | TypeId::STRING | TypeId::NEVER
        ),
        BinOp::Sub | BinOp::Mul | BinOp::Div => {
            matches!(t, TypeId::INT | TypeId::FLOAT | TypeId::NEVER)
        }
        BinOp::Rem => matches!(t, TypeId::INT | TypeId::NEVER),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            matches!(
                t,
                TypeId::INT | TypeId::FLOAT | TypeId::STRING | TypeId::NEVER
            )
        }
        BinOp::Eq | BinOp::Ne => true,
    };
    if !ok {
        return Err(format!(
            "cannot apply {} to {}",
            op.symbol(),
            m.type_name(t)
        ));
    }
    Ok(match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => t,
        _ => TypeId::BOOL,
    })
}

fn builtin_type(m: &HirModule, op: Builtin, args: &[ExprId]) -> Result<TypeId, String> {
    let tys: Vec<TypeId> = args.iter().map(|&a| m.expr_type(a)).collect();
    let arity = match op {
        Builtin::Clock => 0,
        Builtin::Len
        | Builtin::Pop
        | Builtin::Keys
        | Builtin::Str
        | Builtin::Print
        | Builtin::Eprint => 1,
        Builtin::Exit | Builtin::Panic => 1,
        Builtin::Push | Builtin::Has | Builtin::Remove | Builtin::ByteAt => 2,
        Builtin::Substr => 3,
    };
    if tys.len() != arity {
        return Err(format!(
            "{} takes {arity} arguments, {} given",
            op.name(),
            tys.len()
        ));
    }
    let bad = |what: &str| Err(format!("{}: {what}", op.name()));
    let kind = |i: usize| m.kind(tys[i]).clone();
    Ok(match op {
        Builtin::Len => match kind(0) {
            TypeKind::String | TypeKind::Array(_) | TypeKind::Map(..) | TypeKind::Never => {
                TypeId::INT
            }
            _ => return bad("needs a string, array or map"),
        },
        Builtin::Push => match kind(0) {
            TypeKind::Array(e) if fits(tys[1], e) => TypeId::UNIT,
            TypeKind::Never => TypeId::UNIT,
            _ => return bad("needs an array and an element of its type"),
        },
        Builtin::Pop => match kind(0) {
            TypeKind::Array(e) => e,
            TypeKind::Never => TypeId::NEVER,
            _ => return bad("needs an array"),
        },
        Builtin::Has | Builtin::Remove => match kind(0) {
            TypeKind::Map(k, _) if fits(tys[1], k) => TypeId::BOOL,
            TypeKind::Never => TypeId::BOOL,
            _ => return bad("needs a map and a key of its type"),
        },
        Builtin::Keys => match kind(0) {
            TypeKind::Map(k, _) => m
                .type_map
                .get(&TypeKind::Array(k))
                .copied()
                .ok_or_else(|| "keys: the array type was not interned".to_string())?,
            _ => return bad("needs a map"),
        },
        Builtin::Substr => {
            if fits(tys[0], TypeId::STRING)
                && fits(tys[1], TypeId::INT)
                && fits(tys[2], TypeId::INT)
            {
                TypeId::STRING
            } else {
                return bad("needs a string, a start and a count");
            }
        }
        Builtin::ByteAt => {
            if fits(tys[0], TypeId::STRING) && fits(tys[1], TypeId::INT) {
                TypeId::INT
            } else {
                return bad("needs a string and an index");
            }
        }
        Builtin::Str => match tys[0] {
            TypeId::INT | TypeId::FLOAT | TypeId::BOOL | TypeId::NEVER => TypeId::STRING,
            _ => return bad("formats an int, a float or a bool"),
        },
        Builtin::Print | Builtin::Eprint => {
            if fits(tys[0], TypeId::STRING) {
                TypeId::UNIT
            } else {
                return bad("needs a string");
            }
        }
        Builtin::Exit => {
            if fits(tys[0], TypeId::INT) {
                TypeId::NEVER
            } else {
                return bad("needs an int exit status");
            }
        }
        Builtin::Panic => {
            if fits(tys[0], TypeId::STRING) {
                TypeId::NEVER
            } else {
                return bad("needs a string message");
            }
        }
        Builtin::Clock => TypeId::FLOAT,
    })
}

/// Types an `any` can be tested for and unpacked to.
fn testable(m: &HirModule, t: TypeId) -> bool {
    matches!(
        m.kind(t),
        TypeKind::Unit
            | TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::String
            | TypeKind::Struct(_)
            | TypeKind::Enum(_)
    )
}

fn cast_allowed(m: &HirModule, from: TypeId, to: TypeId) -> bool {
    from == to
        || from == TypeId::NEVER
        || (from == TypeId::INT && to == TypeId::FLOAT)
        || (from == TypeId::FLOAT && to == TypeId::INT)
        || (to == TypeId::ANY && from != TypeId::NEVER)
        || (from == TypeId::ANY && testable(m, to))
}

/// Whether `e` contains a `break` that targets the innermost loop around it (not a nested loop, not
/// inside a closure).
pub(crate) fn contains_break(m: &HirModule, e: ExprId) -> bool {
    let mut found = false;
    visit_children(m, e, &mut |c| {
        if found {
            return;
        }
        match &m.expr(c).kind {
            ExprKind::Break => found = true,
            ExprKind::Loop(_) | ExprKind::While(..) | ExprKind::Lambda(_) => {}
            _ => found = contains_break(m, c),
        }
    });
    found
}

/// Calls `f` on each direct child expression of `e` (closure bodies are not children).
pub(crate) fn visit_children(m: &HirModule, e: ExprId, f: &mut dyn FnMut(ExprId)) {
    use ExprKind::*;
    match &m.expr(e).kind {
        Unit | Bool(_) | Int(_) | Float(_) | Str(_) | Null | Local(_) | Global(_) | Func(_)
        | Lambda(_) | NewArray | NewMap | Break | Continue => {}
        Let(_, a)
        | SetLocal(_, a)
        | SetGlobal(_, a)
        | Unary(_, a)
        | Field(a, _)
        | Cast(a)
        | Is(a, _)
        | Loop(a) => f(*a),
        SetField(a, _, b) | Binary(_, a, b) | And(a, b) | Or(a, b) | Index(a, b) | While(a, b) => {
            f(*a);
            f(*b);
        }
        SetIndex(a, b, c) => {
            f(*a);
            f(*b);
            f(*c);
        }
        Call(_, args)
        | CallExtern(_, args)
        | New(_, args)
        | Variant(_, _, args)
        | Array(args)
        | Builtin(_, args) => args.iter().for_each(|&a| f(a)),
        CallValue(c, args) => {
            f(*c);
            args.iter().for_each(|&a| f(a));
        }
        Match(s, arms) => {
            f(*s);
            for a in arms {
                f(m.arms[a.index()].body);
            }
        }
        If(c, t, e) => {
            f(*c);
            f(*t);
            if let Some(e) = e {
                f(*e);
            }
        }
        Return(v) => {
            if let Some(v) = v {
                f(*v);
            }
        }
        Block(items, tail) => {
            items.iter().for_each(|&i| f(i));
            if let Some(t) = tail {
                f(*t);
            }
        }
    }
}
