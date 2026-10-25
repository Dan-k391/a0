//! Construction of HIR modules.
//!
//! Expressions are built bottom-up and typed as they are built, so a frontend can ask for the type of
//! anything it has made. A type that is not known yet is an inference variable
//! ([`HirBuilder::fresh_type`], the elements of `[]`, the type arguments of a generic call), fixed later
//! by unification. A node whose rule needs a type that is still open (a field of an object of unknown
//! type) is typed again by `finish`. A type error is recorded as a diagnostic at the expression it is
//! about and the node gets `never`, which fits everywhere, so one mistake is reported once.

use std::collections::HashSet;

use a0_diag::{Diagnostic, Diagnostics, Span};
use a0_sema::Scopes;

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

/// A name as the text format writes it: as is if it is an identifier and not a keyword, otherwise
/// in backquotes.
pub(crate) fn ident(name: &str) -> std::borrow::Cow<'_, str> {
    if is_identifier(name) {
        name.into()
    } else {
        format!("`{name}`").into()
    }
}

pub(crate) fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&s)
}

/// Builds a [`HirModule`]. See the crate documentation for an example.
pub struct HirBuilder {
    pub(crate) m: HirModule,
    diags: Diagnostics,
    span: Option<Span>,
    /// Nodes whose typing rule needed a type that was still open, with their hints.
    deferred: Vec<(ExprId, TypeId)>,
    // State of the name-based helpers in `frontend.rs`.
    /// Locals by name.
    pub(crate) names: Scopes,
    /// Scopes opened by shadowing in each open block, closed with it.
    pub(crate) shadow_scopes: Vec<u32>,
    /// The bodies being built, innermost last, with the type parameters in scope outside each.
    pub(crate) body_stack: Vec<(BodyId, Vec<(String, TypeId)>)>,
    /// Type parameters in scope, by name.
    pub(crate) type_scope: Vec<(String, TypeId)>,
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
            deferred: Vec::new(),
            names: Scopes::new(),
            shadow_scopes: Vec::new(),
            body_stack: Vec::new(),
            type_scope: Vec::new(),
        }
    }

    pub fn module(&self) -> &HirModule {
        &self.m
    }

    /// The span attached to everything built from now on.
    pub fn set_span(&mut self, span: Option<Span>) {
        self.span = span;
    }

    pub(crate) fn error(&mut self, message: &str) {
        self.diags
            .push(Diagnostic::error("hir.type", message).with_span_opt(self.span));
    }

    fn error_at(&mut self, span: Option<Span>, message: &str) {
        self.diags
            .push(Diagnostic::error("hir.type", message).with_span_opt(span));
    }

    /// Finishes inference, checks the module and returns it, or every problem found.
    pub fn finish(mut self) -> Result<HirModule, Diagnostics> {
        self.solve_deferred();
        self.report_unresolved();
        self.resolve_all();
        self.report_value_nullables();
        let mut diags = self.diags;
        diags.extend(check(&mut self.m));
        // Each combination of type arguments the program uses must make valid types.
        if !diags.has_errors()
            && let Err(d) = crate::mono::monomorphize(&self.m)
        {
            diags.extend(d);
        }
        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(self.m)
        }
    }

    /// Types the deferred nodes again until no more can be typed.
    fn solve_deferred(&mut self) {
        loop {
            let pending = std::mem::take(&mut self.deferred);
            let before = pending.len();
            for (e, hint) in pending {
                let kind = self.m.exprs[e.index()].kind.clone();
                match infer(&mut self.m, &kind, hint) {
                    Ok(t) => {
                        let node = self.m.exprs[e.index()].ty;
                        self.m.unify(node, t);
                    }
                    Err(TypeErr::Open(_)) => self.deferred.push((e, hint)),
                    Err(TypeErr::At(at, msg)) => {
                        let span = self.m.exprs[at.unwrap_or(e).index()].span;
                        self.error_at(span, &msg);
                        let node = self.m.exprs[e.index()].ty;
                        self.m.unify(node, TypeId::ERROR);
                        self.m.exprs[e.index()].ty = TypeId::ERROR;
                    }
                }
            }
            if self.deferred.is_empty() || self.deferred.len() == before {
                break;
            }
        }
        for (e, _) in std::mem::take(&mut self.deferred) {
            let kind = self.m.exprs[e.index()].kind.clone();
            if let Err(TypeErr::Open(at)) = infer(&mut self.m, &kind, TypeId::NEVER) {
                let span = self.m.exprs[at.index()].span;
                self.error_at(
                    span,
                    "the type of this value must be known here; add a type annotation",
                );
            }
            let node = self.m.exprs[e.index()].ty;
            self.m.unify(node, TypeId::ERROR);
            self.m.exprs[e.index()].ty = TypeId::ERROR;
        }
    }

    /// Reports each inference variable that nothing fixed, once, where it first appears.
    fn report_unresolved(&mut self) {
        if self.diags.has_errors() {
            return;
        }
        let mut reported = HashSet::new();
        for i in 0..self.m.exprs.len() {
            let t = self.m.exprs[i].ty;
            if let Some(v) = self.m.first_var(t)
                && reported.insert(v)
            {
                let msg = format!(
                    "cannot infer the type `{}`; add a type annotation",
                    self.m.display_type(t)
                );
                self.error_at(self.m.exprs[i].span, &msg);
            }
        }
        for i in 0..self.m.locals.len() {
            let t = self.m.locals[i].ty;
            if let Some(v) = self.m.first_var(t)
                && reported.insert(v)
            {
                let msg = format!(
                    "cannot infer the type of `{}`; add a type annotation",
                    self.m.locals[i].name
                );
                self.error_at(self.m.locals[i].span, &msg);
            }
        }
    }

    /// Reports `T?` where inference made `T` a value type (`let x = null; x = 5;`).
    fn report_value_nullables(&mut self) {
        let mut reported = HashSet::new();
        for i in 0..self.m.exprs.len() {
            let t = self.m.exprs[i].ty;
            if let Some(inner) = value_nullable(&self.m, t)
                && reported.insert(inner)
            {
                let msg = format!(
                    "only reference types can be nullable, not `{}`",
                    self.m.display_type(inner)
                );
                self.error_at(self.m.exprs[i].span, &msg);
            }
        }
    }

    /// Replaces every inferred type by what inference found, and names in field nodes by indices.
    fn resolve_all(&mut self) {
        let m = &mut self.m;
        for i in 0..m.exprs.len() {
            m.exprs[i].ty = m.resolve(m.exprs[i].ty);
            let kind = match m.exprs[i].kind.clone() {
                ExprKind::Is(e, t) => ExprKind::Is(e, m.resolve(t)),
                ExprKind::Call(f, targs, args) => {
                    let targs = targs.into_iter().map(|t| m.resolve(t)).collect();
                    ExprKind::Call(f, targs, args)
                }
                ExprKind::Func(f, targs) => {
                    let targs = targs.into_iter().map(|t| m.resolve(t)).collect();
                    ExprKind::Func(f, targs)
                }
                ExprKind::FieldName(o, name) => match field_index(m, o, &name) {
                    Some(i) => ExprKind::Field(o, i),
                    None => continue,
                },
                ExprKind::SetFieldName(o, name, v) => match field_index(m, o, &name) {
                    Some(i) => ExprKind::SetField(o, i, v),
                    None => continue,
                },
                _ => continue,
            };
            m.exprs[i].kind = kind;
        }
        for i in 0..m.locals.len() {
            m.locals[i].ty = m.resolve(m.locals[i].ty);
        }
        for i in 0..m.bodies.len() {
            m.bodies[i].ret = m.resolve(m.bodies[i].ret);
        }
    }

    // ---------------------------------------------------------------- types

    /// A type to be inferred from how it is used.
    pub fn fresh_type(&mut self) -> TypeId {
        self.m.fresh_var()
    }

    /// Resolves what is known about `t` so far (inference variables bound by unification).
    pub fn resolved_type(&mut self, t: TypeId) -> TypeId {
        self.m.resolve(t)
    }

    pub fn array_type(&mut self, elem: TypeId) -> TypeId {
        self.m.intern(TypeKind::Array(elem))
    }

    pub fn map_type(&mut self, key: TypeId, value: TypeId) -> TypeId {
        if !matches!(
            self.m.kind(key),
            TypeKind::Int | TypeKind::Bool | TypeKind::String | TypeKind::Var(_)
        ) {
            let k = self.m.display_type(key);
            self.error(&format!("map keys must be int, bool or string, not `{k}`"));
        }
        self.m.intern(TypeKind::Map(key, value))
    }

    pub fn fn_type(&mut self, params: &[TypeId], ret: TypeId) -> TypeId {
        self.m.intern(TypeKind::Fn(params.to_vec(), ret))
    }

    /// `T?`: `t` or null. `t` must be a reference type; a type parameter is checked for each
    /// combination of type arguments the program uses.
    pub fn nullable_type(&mut self, t: TypeId) -> TypeId {
        match self.m.kind(t) {
            TypeKind::Nullable(_) => self.m.shallow(t),
            TypeKind::Var(_) | TypeKind::Param(..) => self.m.intern(TypeKind::Nullable(t)),
            _ if self.m.is_reference(t) => self.m.intern(TypeKind::Nullable(t)),
            _ => {
                let name = self.m.display_type(t);
                self.error(&format!(
                    "only reference types can be nullable, not `{name}`"
                ));
                t
            }
        }
    }

    /// A non-generic struct's type.
    pub fn struct_type(&mut self, s: StructId) -> TypeId {
        self.struct_type_args(s, &[])
    }

    /// A struct's type with type arguments, one per type parameter.
    pub fn struct_type_args(&mut self, s: StructId, args: &[TypeId]) -> TypeId {
        self.arity(Item::Struct(s), args.len());
        self.m.intern(TypeKind::Struct(s, args.to_vec()))
    }

    pub fn enum_type(&mut self, e: EnumId) -> TypeId {
        self.enum_type_args(e, &[])
    }

    pub fn enum_type_args(&mut self, e: EnumId, args: &[TypeId]) -> TypeId {
        self.arity(Item::Enum(e), args.len());
        self.m.intern(TypeKind::Enum(e, args.to_vec()))
    }

    /// The `index`th type parameter of a generic struct, enum or function.
    pub fn type_param(&mut self, owner: Item, index: u32) -> TypeId {
        if index as usize >= self.m.type_params(owner).len() {
            self.error("no such type parameter");
        }
        self.m.intern(TypeKind::Param(owner, index))
    }

    fn arity(&mut self, item: Item, given: usize) {
        let want = self.m.type_params(item).len();
        if want != given {
            let name = match item {
                Item::Struct(s) => self.m.struct_name(s).to_string(),
                Item::Enum(e) => self.m.enum_name(e).to_string(),
                Item::Func(f) => self.m.func_name(f).to_string(),
                _ => String::new(),
            };
            self.error(&format!(
                "`{name}` takes {want} type argument{}, {given} given",
                if want == 1 { "" } else { "s" }
            ));
        }
    }

    /// Fresh inference variables for a generic item's type parameters.
    fn fresh_args(&mut self, item: Item) -> Vec<TypeId> {
        (0..self.m.type_params(item).len())
            .map(|_| self.m.fresh_var())
            .collect()
    }

    // ---------------------------------------------------------------- declarations

    /// Any name is allowed (the text format quotes those that are not identifiers) except an empty
    /// one or one the quoting cannot hold.
    fn item_name(&mut self, name: &str) {
        if name.is_empty() || name.contains(['`', '\n']) {
            self.error(&format!("'{name}' is not a valid name"));
        }
    }

    fn add_type_name(&mut self, name: &str, item: Item) {
        self.item_name(name);
        if self.m.type_names.insert(name.to_string(), item).is_some() {
            self.error(&format!("the type `{name}` is declared twice"));
        }
        self.m.items.push(item);
    }

    fn add_value_name(&mut self, name: &str, item: Item) {
        self.item_name(name);
        if self.m.value_names.insert(name.to_string(), item).is_some() {
            self.error(&format!("`{name}` is declared twice"));
        }
        self.m.items.push(item);
    }

    fn type_param_names(&mut self, params: &[&str]) -> Vec<String> {
        for p in params {
            self.item_name(p);
        }
        params.iter().map(|p| p.to_string()).collect()
    }

    /// Declares a struct; add its fields with [`HirBuilder::add_field`] (structs may refer to
    /// themselves).
    pub fn declare_struct(&mut self, name: &str) -> StructId {
        self.declare_generic_struct(name, &[])
    }

    /// Declares a struct with type parameters; its fields use them through
    /// [`HirBuilder::type_param`].
    pub fn declare_generic_struct(&mut self, name: &str, type_params: &[&str]) -> StructId {
        let s = StructId(self.m.structs.len() as u32);
        let type_params = self.type_param_names(type_params);
        self.m.structs.push(StructDef {
            name: name.to_string(),
            type_params,
            fields: Vec::new(),
            span: self.span,
        });
        self.add_type_name(name, Item::Struct(s));
        s
    }

    pub fn add_field(&mut self, s: StructId, name: &str, ty: TypeId) -> u32 {
        self.item_name(name);
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
        self.declare_generic_enum(name, &[])
    }

    pub fn declare_generic_enum(&mut self, name: &str, type_params: &[&str]) -> EnumId {
        let e = EnumId(self.m.enums.len() as u32);
        let type_params = self.type_param_names(type_params);
        self.m.enums.push(EnumDef {
            name: name.to_string(),
            type_params,
            variants: Vec::new(),
            span: self.span,
        });
        self.add_type_name(name, Item::Enum(e));
        e
    }

    pub fn add_variant(&mut self, e: EnumId, name: &str, fields: &[TypeId]) -> u32 {
        self.item_name(name);
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

    /// Declares a global. Globals start at the zero value of their type, so a reference type must
    /// be nullable.
    pub fn declare_global(&mut self, name: &str, ty: TypeId) -> GlobalId {
        if self.m.is_reference(ty) && !matches!(self.m.kind(ty), TypeKind::Nullable(_)) {
            let t = self.m.display_type(ty);
            self.error(&format!(
                "global '{name}' starts as null, so its type must be nullable (`{t}?`)"
            ));
        }
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
                let tn = self.m.display_type(t);
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
        let f = self.declare_generic_function(name, &[]);
        self.set_signature(f, param_names, params, ret);
        f
    }

    /// Declares a generic function without its signature, so that the signature can use the type
    /// parameters ([`HirBuilder::type_param`]); give it with [`HirBuilder::set_signature`].
    pub fn declare_generic_function(&mut self, name: &str, type_params: &[&str]) -> FuncId {
        let f = FuncId(self.m.funcs.len() as u32);
        let body = BodyId(self.m.bodies.len() as u32);
        let type_params = self.type_param_names(type_params);
        self.m.funcs.push(FuncDef {
            name: name.to_string(),
            type_params,
            body,
            span: self.span,
            local_names: HashSet::new(),
        });
        self.m.bodies.push(BodyData {
            params: Vec::new(),
            param_names: Vec::new(),
            ret: TypeId::UNIT,
            root: None,
            parent: None,
            func: f,
        });
        self.add_value_name(name, Item::Func(f));
        f
    }

    /// Sets the parameters and result type of a function declared with
    /// [`HirBuilder::declare_generic_function`].
    pub fn set_signature(
        &mut self,
        f: FuncId,
        param_names: &[&str],
        params: &[TypeId],
        ret: TypeId,
    ) {
        let body = self.m.funcs[f.index()].body;
        if !self.m.bodies[body.index()].params.is_empty() {
            self.error("a function's signature can only be set once");
            return;
        }
        self.m.bodies[body.index()].ret = ret;
        self.add_params(body, param_names, params);
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
            param_names: Vec::new(),
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
            self.m.bodies[body.index()]
                .param_names
                .push(name.to_string());
        }
    }

    pub fn param(&self, body: BodyId, i: u32) -> LocalId {
        self.m.bodies[body.index()].params[i as usize]
    }

    /// Creates a local of `body`. Names are made unique within the function (`x`, `x_1`, ...) and
    /// adjusted if they are not valid identifiers; locals are referred to by handle, so this only
    /// affects printing. The type may be [`HirBuilder::fresh_type`] to infer it.
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
            span: self.span,
        });
        l
    }

    /// Attaches a body's expression. Every `return` in it must give a value of the body's result
    /// type, and so must the body's own value, unless the result type is `unit` (the value of a
    /// body that returns nothing is discarded).
    pub fn set_root(&mut self, body: BodyId, root: ExprId) {
        self.m.bodies[body.index()].root = Some(root);
        let ret = self.m.bodies[body.index()].ret;
        let rt = self.m.expr_type(root);
        let (want, found) = (self.m.display_type(ret), self.m.display_type(rt));
        if ret != TypeId::UNIT && !self.m.coerce(rt, ret) {
            let span = self.m.exprs[root.index()].span;
            let msg = if self.m.same(rt, TypeId::UNIT) {
                format!("this function can end without returning a value of type `{want}`")
            } else {
                format!("this body evaluates to `{found}` but must return `{want}`")
            };
            self.error_at(span, &msg);
        }
        let mut returns = Vec::new();
        collect_returns(&self.m, root, &mut returns);
        for v in returns {
            let vt = self.m.expr_type(v);
            let msg = mismatch(&self.m, ret, vt);
            if !self.m.coerce(vt, ret) {
                let span = self.m.exprs[v.index()].span;
                self.error_at(span, &msg);
            }
        }
    }

    // ---------------------------------------------------------------- expressions

    /// Adds a node with a given type, without typing it.
    pub(crate) fn push_typed(&mut self, kind: ExprKind, ty: TypeId) -> ExprId {
        self.m.exprs.push(ExprData {
            kind,
            ty,
            span: self.span,
        });
        ExprId(self.m.exprs.len() as u32 - 1)
    }

    fn push(&mut self, kind: ExprKind, hint: TypeId) -> ExprId {
        let e = ExprId(self.m.exprs.len() as u32);
        let ty = match infer(&mut self.m, &kind, hint) {
            Ok(t) => t,
            Err(TypeErr::Open(_)) => {
                self.deferred.push((e, hint));
                self.m.fresh_var()
            }
            Err(TypeErr::At(at, msg)) => {
                let span = match at {
                    Some(x) => self.m.exprs[x.index()].span,
                    None => self.span,
                };
                self.error_at(span, &msg);
                TypeId::ERROR
            }
        };
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

    /// `null` of type `T?`, for a reference type `T` (which may be [`HirBuilder::fresh_type`]).
    pub fn null(&mut self, ty: TypeId) -> ExprId {
        let t = self.nullable_type(ty);
        self.push(ExprKind::Null, t)
    }

    pub fn local(&mut self, l: LocalId) -> ExprId {
        self.push(ExprKind::Local(l), TypeId::NEVER)
    }

    pub fn global(&mut self, g: GlobalId) -> ExprId {
        self.push(ExprKind::Global(g), TypeId::NEVER)
    }

    /// A function used as a closure value; a generic function's type arguments are inferred.
    pub fn func(&mut self, f: FuncId) -> ExprId {
        let targs = self.fresh_args(Item::Func(f));
        self.push(ExprKind::Func(f, targs), TypeId::NEVER)
    }

    /// A generic function used as a closure value, with explicit type arguments.
    pub fn func_generic(&mut self, f: FuncId, type_args: &[TypeId]) -> ExprId {
        self.arity(Item::Func(f), type_args.len());
        self.push(ExprKind::Func(f, type_args.to_vec()), TypeId::NEVER)
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

    /// `object.name = value`; the object's type may still be open.
    pub fn set_field_named(&mut self, object: ExprId, name: &str, value: ExprId) -> ExprId {
        self.push(
            ExprKind::SetFieldName(object, name.to_string(), value),
            TypeId::UNIT,
        )
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

    /// Calls a function; a generic function's type arguments are inferred from the arguments.
    pub fn call(&mut self, f: FuncId, args: &[ExprId]) -> ExprId {
        let targs = self.fresh_args(Item::Func(f));
        self.push(ExprKind::Call(f, targs, args.to_vec()), TypeId::NEVER)
    }

    /// Calls a generic function with explicit type arguments.
    pub fn call_generic(&mut self, f: FuncId, type_args: &[TypeId], args: &[ExprId]) -> ExprId {
        self.arity(Item::Func(f), type_args.len());
        self.push(
            ExprKind::Call(f, type_args.to_vec(), args.to_vec()),
            TypeId::NEVER,
        )
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
        self.push(ExprKind::Lambda(body), TypeId::NEVER)
    }

    /// Constructs a struct; `fields` are in declaration order. A generic struct's type arguments
    /// are inferred from the fields.
    pub fn new_struct(&mut self, s: StructId, fields: &[ExprId]) -> ExprId {
        let args = self.fresh_args(Item::Struct(s));
        let t = self.m.intern(TypeKind::Struct(s, args));
        self.push(ExprKind::New(s, fields.to_vec()), t)
    }

    /// Constructs a generic struct with explicit type arguments.
    pub fn new_struct_generic(
        &mut self,
        s: StructId,
        type_args: &[TypeId],
        fields: &[ExprId],
    ) -> ExprId {
        let t = self.struct_type_args(s, type_args);
        self.push(ExprKind::New(s, fields.to_vec()), t)
    }

    pub fn field(&mut self, object: ExprId, field: u32) -> ExprId {
        self.push(ExprKind::Field(object, field), TypeId::NEVER)
    }

    /// Field access by name. The object's type may still be open; the field is then looked up once
    /// it is known.
    pub fn field_named(&mut self, object: ExprId, name: &str) -> ExprId {
        self.push(ExprKind::FieldName(object, name.to_string()), TypeId::NEVER)
    }

    /// An enum value; a generic enum's type arguments are inferred from the payload and from how the
    /// value is used.
    pub fn variant(&mut self, e: EnumId, v: u32, args: &[ExprId]) -> ExprId {
        let targs = self.fresh_args(Item::Enum(e));
        let t = self.m.intern(TypeKind::Enum(e, targs));
        self.push(ExprKind::Variant(e, v, args.to_vec()), t)
    }

    pub fn variant_generic(
        &mut self,
        e: EnumId,
        type_args: &[TypeId],
        v: u32,
        args: &[ExprId],
    ) -> ExprId {
        let t = self.enum_type_args(e, type_args);
        self.push(ExprKind::Variant(e, v, args.to_vec()), t)
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

    /// An array literal. Its element type is inferred; `[]` gets it from later uses.
    pub fn array(&mut self, elems: &[ExprId]) -> ExprId {
        let elem = self.m.fresh_var();
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
        self.push(ExprKind::Builtin(op, args.to_vec()), TypeId::NEVER)
    }

    /// `e as ty`.
    pub fn cast(&mut self, e: ExprId, ty: TypeId) -> ExprId {
        self.push(ExprKind::Cast(e), ty)
    }

    /// `e!`: the value of a nullable reference; panics with `null reference` if it is null.
    pub fn unwrap(&mut self, e: ExprId) -> ExprId {
        self.push(ExprKind::Unwrap(e), TypeId::NEVER)
    }

    /// `e is ty`, for `e: any`.
    pub fn is(&mut self, e: ExprId, ty: TypeId) -> ExprId {
        self.push(ExprKind::Is(e, ty), TypeId::BOOL)
    }

    pub fn if_(&mut self, cond: ExprId, then: ExprId, els: Option<ExprId>) -> ExprId {
        self.push(ExprKind::If(cond, then, els), TypeId::NEVER)
    }

    /// `if let local = value { then } else { els }`: `value` is a `T?`, and `local` (a `T`, or a
    /// type to infer) holds it in `then` when it is not null.
    pub fn if_let(
        &mut self,
        local: LocalId,
        value: ExprId,
        then: ExprId,
        els: Option<ExprId>,
    ) -> ExprId {
        self.push(ExprKind::IfLet(local, value, then, els), TypeId::NEVER)
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

/// The `T` of a `T?` inside `t` where `T` is a value type.
fn value_nullable(m: &HirModule, t: TypeId) -> Option<TypeId> {
    match m.kind(t) {
        TypeKind::Nullable(inner) => {
            let ok = m.is_reference(*inner)
                || matches!(
                    m.kind(*inner),
                    TypeKind::Param(..) | TypeKind::Var(_) | TypeKind::Error
                );
            if ok {
                value_nullable(m, *inner)
            } else {
                Some(*inner)
            }
        }
        TypeKind::Array(e) => value_nullable(m, *e),
        TypeKind::Map(k, v) => value_nullable(m, *k).or_else(|| value_nullable(m, *v)),
        TypeKind::Struct(_, args) | TypeKind::Enum(_, args) => {
            args.iter().find_map(|&a| value_nullable(m, a))
        }
        TypeKind::Fn(params, ret) => params
            .iter()
            .find_map(|&p| value_nullable(m, p))
            .or_else(|| value_nullable(m, *ret)),
        _ => None,
    }
}

/// The `return` values in a body, not counting nested closures.
fn collect_returns(m: &HirModule, e: ExprId, out: &mut Vec<ExprId>) {
    if let ExprKind::Return(Some(v)) = m.expr(e).kind {
        out.push(v);
    }
    visit_children(m, e, &mut |c| collect_returns(m, c, out));
}

fn field_index(m: &HirModule, object: ExprId, name: &str) -> Option<u32> {
    match m.kind(m.expr_type(object)) {
        TypeKind::Struct(s, _) => m.struct_field_index(*s, name),
        _ => None,
    }
}

/// Why a node could not be typed.
pub(crate) enum TypeErr {
    /// A type error, about the given sub-expression (or the node itself).
    At(Option<ExprId>, String),
    /// The rule needs the type of this sub-expression, which is not inferred yet.
    Open(ExprId),
}

type Typed = Result<TypeId, TypeErr>;

fn fail<T>(msg: String) -> Result<T, TypeErr> {
    Err(TypeErr::At(None, msg))
}

pub(crate) fn mismatch(m: &HirModule, expected: TypeId, found: TypeId) -> String {
    format!(
        "mismatched types: expected `{}`, found `{}`",
        m.display_type(expected),
        m.display_type(found)
    )
}

/// Requires `e` to fit `want`.
fn expect(m: &mut HirModule, e: ExprId, want: TypeId) -> Result<(), TypeErr> {
    let found = m.expr_type(e);
    let msg = mismatch(m, want, found);
    if m.coerce(found, want) {
        Ok(())
    } else {
        Err(TypeErr::At(Some(e), msg))
    }
}

fn args_match(m: &mut HirModule, params: &[TypeId], args: &[ExprId]) -> Result<(), TypeErr> {
    if params.len() != args.len() {
        return fail(format!(
            "expected {} argument{}, found {}",
            params.len(),
            if params.len() == 1 { "" } else { "s" },
            args.len()
        ));
    }
    for (&p, &a) in params.iter().zip(args) {
        expect(m, a, p)?;
    }
    Ok(())
}

/// The parameter and result types of a function, instantiated with type arguments.
fn signature(
    m: &mut HirModule,
    f: FuncId,
    targs: &[TypeId],
) -> Result<(Vec<TypeId>, TypeId), TypeErr> {
    let want = m.funcs[f.index()].type_params.len();
    if want != targs.len() {
        return fail(format!(
            "`{}` takes {want} type arguments, {} given",
            m.func_name(f),
            targs.len()
        ));
    }
    let body = m.func_body(f);
    let params: Vec<TypeId> = m
        .body_params(body)
        .iter()
        .map(|&p| m.local_type(p))
        .collect();
    let ret = m.body_ret(body);
    let owner = Item::Func(f);
    let params = params
        .into_iter()
        .map(|p| m.subst(p, owner, targs))
        .collect();
    Ok((params, m.subst(ret, owner, targs)))
}

fn open_or<T>(m: &HirModule, e: ExprId, msg: impl FnOnce() -> String) -> Result<T, TypeErr> {
    if matches!(m.kind(m.expr_type(e)), TypeKind::Var(_)) {
        Err(TypeErr::Open(e))
    } else {
        Err(TypeErr::At(Some(e), msg()))
    }
}

/// The type of an expression, or why it is ill-typed. `hint` carries the type written in the node
/// itself (the target of `as`, the type of `null`, collections and constructed values); for those
/// nodes the result is the hint.
pub(crate) fn infer(m: &mut HirModule, kind: &ExprKind, hint: TypeId) -> Typed {
    let ty = |m: &HirModule, e: ExprId| m.expr_type(e);
    Ok(match kind {
        ExprKind::Unit => TypeId::UNIT,
        ExprKind::Bool(_) => TypeId::BOOL,
        ExprKind::Int(_) => TypeId::INT,
        ExprKind::Float(_) => TypeId::FLOAT,
        ExprKind::Str(_) => TypeId::STRING,
        ExprKind::Null => match m.kind(hint) {
            TypeKind::Nullable(_) => hint,
            _ => {
                return fail(format!(
                    "`null` needs a nullable type, not `{}`",
                    m.display_type(hint)
                ));
            }
        },
        ExprKind::Local(l) => m.local_type(*l),
        ExprKind::Global(g) => m.global_type(*g),
        ExprKind::Func(f, targs) => {
            let (params, ret) = signature(m, *f, targs)?;
            m.intern(TypeKind::Fn(params, ret))
        }
        ExprKind::Let(l, init) | ExprKind::SetLocal(l, init) => {
            expect(m, *init, m.local_type(*l))?;
            TypeId::UNIT
        }
        ExprKind::SetGlobal(g, value) => {
            expect(m, *value, m.global_type(*g))?;
            TypeId::UNIT
        }
        ExprKind::SetField(object, field, value) => {
            let ft = field_type(m, *object, *field)?;
            expect(m, *value, ft)?;
            TypeId::UNIT
        }
        ExprKind::SetFieldName(object, name, value) => {
            let i = named_field(m, *object, name)?;
            let ft = field_type(m, *object, i)?;
            expect(m, *value, ft)?;
            TypeId::UNIT
        }
        ExprKind::SetIndex(target, index, value) => {
            let (key, elem) = index_types(m, *target)?;
            expect(m, *index, key)?;
            expect(m, *value, elem)?;
            TypeId::UNIT
        }
        ExprKind::Unary(UnOp::Neg, a) => match m.kind(ty(m, *a)) {
            TypeKind::Int | TypeKind::Float | TypeKind::Never | TypeKind::Error => {
                m.shallow(ty(m, *a))
            }
            _ => {
                return open_or(m, *a, || {
                    format!(
                        "operator `-` cannot be applied to `{}`",
                        m.display_type(ty(m, *a))
                    )
                });
            }
        },
        ExprKind::Unary(UnOp::Not, a) => {
            expect(m, *a, TypeId::BOOL)?;
            TypeId::BOOL
        }
        ExprKind::Binary(op, a, b) => binary_type(m, *op, *a, *b)?,
        ExprKind::And(a, b) | ExprKind::Or(a, b) => {
            expect(m, *a, TypeId::BOOL)?;
            expect(m, *b, TypeId::BOOL)?;
            TypeId::BOOL
        }
        ExprKind::Call(f, targs, args) => {
            let (params, ret) = signature(m, *f, targs)?;
            args_match(m, &params, args)?;
            ret
        }
        ExprKind::CallExtern(x, args) => {
            let (params, ret) = (
                m.externs[x.index()].params.clone(),
                m.externs[x.index()].ret,
            );
            args_match(m, &params, args)?;
            ret
        }
        ExprKind::CallValue(callee, args) => {
            let ct = ty(m, *callee);
            match m.kind(ct).clone() {
                TypeKind::Fn(params, ret) => {
                    args_match(m, &params, args)?;
                    ret
                }
                TypeKind::Never => TypeId::NEVER,
                TypeKind::Error => TypeId::ERROR,
                TypeKind::Var(_) => {
                    // An unknown callee is a function of the arguments' types.
                    let params: Vec<TypeId> = args.iter().map(|&a| ty(m, a)).collect();
                    let ret = m.fresh_var();
                    let ft = m.intern(TypeKind::Fn(params, ret));
                    m.unify(ct, ft);
                    ret
                }
                _ => {
                    let msg = format!("`{}` is not a function", m.display_type(ct));
                    return Err(TypeErr::At(Some(*callee), msg));
                }
            }
        }
        ExprKind::Lambda(body) => {
            let params: Vec<TypeId> = m
                .body_params(*body)
                .iter()
                .map(|&p| m.local_type(p))
                .collect();
            let ret = m.body_ret(*body);
            m.intern(TypeKind::Fn(params, ret))
        }
        ExprKind::New(s, fields) => {
            let args = m.type_args(hint).to_vec();
            let def_fields: Vec<(String, TypeId)> = m.structs[s.index()].fields.clone();
            if fields.len() != def_fields.len() {
                return fail(format!(
                    "struct `{}` has {} fields, {} given",
                    m.struct_name(*s),
                    def_fields.len(),
                    fields.len()
                ));
            }
            for ((_, ft), &v) in def_fields.iter().zip(fields) {
                let ft = m.subst(*ft, Item::Struct(*s), &args);
                expect(m, v, ft)?;
            }
            hint
        }
        ExprKind::Field(object, field) => field_type(m, *object, *field)?,
        ExprKind::FieldName(object, name) => {
            let i = named_field(m, *object, name)?;
            field_type(m, *object, i)?
        }
        ExprKind::Variant(e, v, args) => {
            let targs = m.type_args(hint).to_vec();
            let Some(var) = m.enums[e.index()].variants.get(*v as usize) else {
                return fail(format!("enum `{}` has no variant {v}", m.enum_name(*e)));
            };
            let fields: Vec<TypeId> = var.fields.clone();
            let fields: Vec<TypeId> = fields
                .into_iter()
                .map(|f| m.subst(f, Item::Enum(*e), &targs))
                .collect();
            args_match(m, &fields, args)?;
            hint
        }
        ExprKind::Match(scrutinee, arms) => match_type(m, *scrutinee, arms)?,
        ExprKind::Array(elems) => {
            let elem = match m.kind(hint) {
                TypeKind::Array(e) => *e,
                _ => return fail("an array literal needs an array type".to_string()),
            };
            for &e in elems {
                expect(m, e, elem)?;
            }
            hint
        }
        ExprKind::NewArray => {
            if !matches!(m.kind(hint), TypeKind::Array(_)) {
                return fail(format!(
                    "new array needs an array type, not `{}`",
                    m.display_type(hint)
                ));
            }
            hint
        }
        ExprKind::NewMap => {
            if !matches!(m.kind(hint), TypeKind::Map(..)) {
                return fail(format!(
                    "new map needs a map type, not `{}`",
                    m.display_type(hint)
                ));
            }
            hint
        }
        ExprKind::Index(target, index) => {
            let (key, elem) = index_types(m, *target)?;
            expect(m, *index, key)?;
            elem
        }
        ExprKind::Builtin(op, args) => builtin_type(m, *op, args)?,
        ExprKind::Cast(e) => {
            let from = ty(m, *e);
            if matches!(m.kind(from), TypeKind::Var(_)) {
                return Err(TypeErr::Open(*e));
            }
            if !cast_allowed(m, from, hint) {
                return fail(format!(
                    "cannot convert `{}` to `{}`",
                    m.display_type(from),
                    m.display_type(hint)
                ));
            }
            hint
        }
        ExprKind::Unwrap(e) => match m.kind(ty(m, *e)) {
            TypeKind::Nullable(inner) => *inner,
            TypeKind::Never => TypeId::NEVER,
            TypeKind::Error => TypeId::ERROR,
            _ => {
                return open_or(m, *e, || {
                    format!(
                        "`{}` is not nullable, so there is nothing to unwrap",
                        m.display_type(ty(m, *e))
                    )
                });
            }
        },
        ExprKind::Is(e, t) => {
            expect(m, *e, TypeId::ANY)?;
            if !testable(m, *t) {
                return fail(format!(
                    "cannot test whether an any is a `{}`",
                    m.display_type(*t)
                ));
            }
            TypeId::BOOL
        }
        ExprKind::If(cond, then, els) => {
            expect(m, *cond, TypeId::BOOL)?;
            match els {
                // Without else, an if is a statement; the branch's value is discarded.
                None => TypeId::UNIT,
                // Branches of different types make a statement-like if of type unit.
                Some(e) => m.join(ty(m, *then), ty(m, *e)).unwrap_or(TypeId::UNIT),
            }
        }
        ExprKind::IfLet(local, value, then, els) => {
            let vt = ty(m, *value);
            match m.kind(vt).clone() {
                TypeKind::Nullable(inner) => {
                    let lt = m.local_type(*local);
                    if !m.unify(lt, inner) {
                        let msg = format!(
                            "`{}` binds a `{}` as `{}`",
                            m.local_name(*local),
                            m.display_type(inner),
                            m.display_type(lt)
                        );
                        return Err(TypeErr::At(Some(*value), msg));
                    }
                }
                TypeKind::Never | TypeKind::Error => {}
                _ => {
                    return open_or(m, *value, || {
                        format!(
                            "`if let` needs a nullable value, found `{}`",
                            m.display_type(vt)
                        )
                    });
                }
            }
            match els {
                None => TypeId::UNIT,
                Some(e) => m.join(ty(m, *then), ty(m, *e)).unwrap_or(TypeId::UNIT),
            }
        }
        ExprKind::While(cond, _) => {
            expect(m, *cond, TypeId::BOOL)?;
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
            Some(t) => ty(m, *t),
            None if items.iter().any(|&i| ty(m, i) == TypeId::NEVER) => TypeId::NEVER,
            None => TypeId::UNIT,
        },
    })
}

fn named_field(m: &HirModule, object: ExprId, name: &str) -> Result<u32, TypeErr> {
    let ot = m.expr_type(object);
    match m.kind(ot) {
        TypeKind::Struct(s, _) => match m.struct_field_index(*s, name) {
            Some(i) => Ok(i),
            None => Err(TypeErr::At(
                None,
                format!("`{}` has no field `{name}`", m.display_type(ot)),
            )),
        },
        TypeKind::Never | TypeKind::Error => Ok(u32::MAX),
        _ => open_or(m, object, || no_fields(m, ot)),
    }
}

fn no_fields(m: &HirModule, t: TypeId) -> String {
    match m.kind(t) {
        TypeKind::Nullable(_) => format!(
            "`{}` may be null; narrow it (`if let`) or unwrap it (`!`) before using its fields",
            m.display_type(t)
        ),
        _ => format!(
            "`{}` is not a struct, so it has no fields",
            m.display_type(t)
        ),
    }
}

fn field_type(m: &mut HirModule, object: ExprId, field: u32) -> Typed {
    let ot = m.expr_type(object);
    match m.kind(ot).clone() {
        TypeKind::Struct(s, args) => match m.structs[s.index()].fields.get(field as usize) {
            Some(&(_, t)) => Ok(m.subst(t, Item::Struct(s), &args)),
            None => fail(format!(
                "struct `{}` has no field {field}",
                m.struct_name(s)
            )),
        },
        TypeKind::Never => Ok(TypeId::NEVER),
        TypeKind::Error => Ok(TypeId::ERROR),
        _ => open_or(m, object, || no_fields(m, ot)),
    }
}

/// The index and element types of `target[...]`.
fn index_types(m: &mut HirModule, target: ExprId) -> Result<(TypeId, TypeId), TypeErr> {
    let t = m.expr_type(target);
    match m.kind(t) {
        TypeKind::Array(e) => Ok((TypeId::INT, *e)),
        TypeKind::Map(k, v) => Ok((*k, *v)),
        TypeKind::Never => Ok((TypeId::NEVER, TypeId::NEVER)),
        TypeKind::Error => Ok((TypeId::ERROR, TypeId::ERROR)),
        _ => open_or(m, target, || match m.kind(t) {
            TypeKind::Nullable(_) => format!(
                "`{}` may be null; narrow it (`if let`) or unwrap it (`!`) before indexing",
                m.display_type(t)
            ),
            _ => format!("cannot index a value of type `{}`", m.display_type(t)),
        }),
    }
}

fn binary_type(m: &mut HirModule, op: BinOp, a: ExprId, b: ExprId) -> Typed {
    let (ta, tb) = (m.expr_type(a), m.expr_type(b));
    if matches!(op, BinOp::Eq | BinOp::Ne) {
        // Either side may be the nullable one: `x == null`.
        let msg = mismatch(m, ta, tb);
        if !(m.coerce(tb, ta) || m.coerce(ta, tb)) {
            return Err(TypeErr::At(Some(b), msg));
        }
        return Ok(TypeId::BOOL);
    }
    let msg = mismatch(m, ta, tb);
    if ta != TypeId::NEVER && ta != TypeId::ERROR && !m.coerce(tb, ta) {
        return Err(TypeErr::At(Some(b), msg));
    }
    let (t, operand) = if ta == TypeId::NEVER {
        (tb, b)
    } else {
        (ta, a)
    };
    let ok = match m.kind(t) {
        TypeKind::Never | TypeKind::Error => true,
        TypeKind::Int => true,
        TypeKind::Float => op != BinOp::Rem,
        TypeKind::String => matches!(
            op,
            BinOp::Add | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        ),
        TypeKind::Var(_) => return Err(TypeErr::Open(operand)),
        _ => false,
    };
    if !ok {
        return Err(TypeErr::At(
            Some(operand),
            format!(
                "operator `{}` cannot be applied to `{}`",
                op.symbol(),
                m.display_type(t)
            ),
        ));
    }
    Ok(if op.is_comparison() {
        TypeId::BOOL
    } else {
        m.shallow(t)
    })
}

fn match_type(m: &mut HirModule, scrutinee: ExprId, arms: &[ArmId]) -> Typed {
    let st = m.expr_type(scrutinee);
    let (e, eargs) = match m.kind(st).clone() {
        TypeKind::Enum(e, args) => (e, args),
        TypeKind::Never => return Ok(TypeId::NEVER),
        TypeKind::Error => return Ok(TypeId::ERROR),
        _ => {
            return open_or(m, scrutinee, || {
                format!("match needs an enum, found `{}`", m.display_type(st))
            });
        }
    };
    let mut result = TypeId::NEVER;
    let mut covered = vec![false; m.variant_count(e) as usize];
    let mut wildcard = false;
    for &a in arms {
        let (variant, binds, body) = {
            let arm = &m.arms[a.index()];
            (arm.variant, arm.binds.clone(), arm.body)
        };
        match variant {
            Some(v) => {
                let Some(fields) = m.enums[e.index()]
                    .variants
                    .get(v as usize)
                    .map(|v| v.fields.clone())
                else {
                    return fail(format!("enum `{}` has no variant {v}", m.enum_name(e)));
                };
                if binds.len() != fields.len() {
                    return fail(format!(
                        "variant `{}` has {} fields, the arm binds {}",
                        m.variant_name(e, v),
                        fields.len(),
                        binds.len()
                    ));
                }
                for (&b, &f) in binds.iter().zip(&fields) {
                    let f = m.subst(f, Item::Enum(e), &eargs);
                    let lt = m.local_type(b);
                    let msg = format!(
                        "`{}` binds a `{}` as `{}`",
                        m.local_name(b),
                        m.display_type(f),
                        m.display_type(lt)
                    );
                    if !m.unify(lt, f) {
                        return fail(msg);
                    }
                }
                covered[v as usize] = true;
            }
            None => wildcard = true,
        }
        let bt = m.expr_type(body);
        result = m.join(result, bt).unwrap_or(TypeId::UNIT);
    }
    if !wildcard && covered.iter().any(|c| !c) {
        let missing: Vec<&str> = covered
            .iter()
            .enumerate()
            .filter(|(_, c)| !**c)
            .map(|(i, _)| m.variant_name(e, i as u32))
            .collect();
        return fail(format!("match does not cover {}", missing.join(", ")));
    }
    Ok(result)
}

fn builtin_type(m: &mut HirModule, op: Builtin, args: &[ExprId]) -> Typed {
    let arity = match op {
        Builtin::Clock => 0,
        Builtin::Len
        | Builtin::Pop
        | Builtin::Keys
        | Builtin::Str
        | Builtin::Print
        | Builtin::Eprint
        | Builtin::Exit
        | Builtin::Panic => 1,
        Builtin::Push | Builtin::Has | Builtin::Remove | Builtin::ByteAt => 2,
        Builtin::Substr => 3,
    };
    if args.len() != arity {
        return fail(format!(
            "`{}` takes {arity} argument{}, found {}",
            op.name(),
            if arity == 1 { "" } else { "s" },
            args.len()
        ));
    }
    let t0 = args
        .first()
        .map(|&a| m.expr_type(a))
        .unwrap_or(TypeId::UNIT);
    let bad = |m: &HirModule, what: &str| {
        open_or(m, args[0], || {
            format!("`{}` {what}, not `{}`", op.name(), m.display_type(t0))
        })
    };
    Ok(match op {
        Builtin::Len => match m.kind(t0) {
            TypeKind::String
            | TypeKind::Array(_)
            | TypeKind::Map(..)
            | TypeKind::Never
            | TypeKind::Error => TypeId::INT,
            _ => return bad(m, "needs a string, array or map"),
        },
        Builtin::Push | Builtin::Pop => {
            let elem = match m.kind(t0).clone() {
                TypeKind::Array(e) => e,
                TypeKind::Never => TypeId::NEVER,
                TypeKind::Error => TypeId::ERROR,
                TypeKind::Var(_) => {
                    let e = m.fresh_var();
                    let at = m.intern(TypeKind::Array(e));
                    m.unify(t0, at);
                    e
                }
                _ => return bad(m, "needs an array"),
            };
            if op == Builtin::Push {
                if elem != TypeId::NEVER && elem != TypeId::ERROR {
                    expect(m, args[1], elem)?;
                }
                TypeId::UNIT
            } else {
                elem
            }
        }
        Builtin::Has | Builtin::Remove => match m.kind(t0).clone() {
            TypeKind::Map(k, _) => {
                expect(m, args[1], k)?;
                TypeId::BOOL
            }
            TypeKind::Never | TypeKind::Error => TypeId::BOOL,
            _ => return bad(m, "needs a map"),
        },
        Builtin::Keys => match m.kind(t0).clone() {
            TypeKind::Map(k, _) => m.intern(TypeKind::Array(k)),
            TypeKind::Never => TypeId::NEVER,
            TypeKind::Error => TypeId::ERROR,
            _ => return bad(m, "needs a map"),
        },
        Builtin::Substr => {
            expect(m, args[0], TypeId::STRING)?;
            expect(m, args[1], TypeId::INT)?;
            expect(m, args[2], TypeId::INT)?;
            TypeId::STRING
        }
        Builtin::ByteAt => {
            expect(m, args[0], TypeId::STRING)?;
            expect(m, args[1], TypeId::INT)?;
            TypeId::INT
        }
        Builtin::Str | Builtin::Print | Builtin::Eprint => match m.kind(t0) {
            TypeKind::Int
            | TypeKind::Float
            | TypeKind::Bool
            | TypeKind::String
            | TypeKind::Never
            | TypeKind::Error
                if op == Builtin::Str =>
            {
                TypeId::STRING
            }
            TypeKind::Int
            | TypeKind::Float
            | TypeKind::Bool
            | TypeKind::String
            | TypeKind::Never
            | TypeKind::Error => TypeId::UNIT,
            _ => return bad(m, "formats an int, float, bool or string"),
        },
        Builtin::Exit => {
            expect(m, args[0], TypeId::INT)?;
            TypeId::NEVER
        }
        Builtin::Panic => {
            expect(m, args[0], TypeId::STRING)?;
            TypeId::NEVER
        }
        Builtin::Clock => TypeId::FLOAT,
    })
}

/// Types an `any` can be tested for and unpacked to.
pub(crate) fn testable(m: &HirModule, t: TypeId) -> bool {
    match m.kind(t) {
        TypeKind::Unit
        | TypeKind::Bool
        | TypeKind::Int
        | TypeKind::Float
        | TypeKind::String
        | TypeKind::Struct(..)
        | TypeKind::Enum(..) => true,
        TypeKind::Nullable(inner) => testable(m, *inner),
        _ => false,
    }
}

fn cast_allowed(m: &HirModule, from: TypeId, to: TypeId) -> bool {
    let (fk, tk) = (m.kind(from), m.kind(to));
    m.same(from, to)
        || matches!(fk, TypeKind::Never | TypeKind::Error)
        || matches!(
            (fk, tk),
            (TypeKind::Int, TypeKind::Float) | (TypeKind::Float, TypeKind::Int)
        )
        || matches!(tk, TypeKind::Any)
        || (matches!(fk, TypeKind::Any) && testable(m, to))
        || matches!(tk, TypeKind::Nullable(inner) if m.same(from, *inner))
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
        Unit | Bool(_) | Int(_) | Float(_) | Str(_) | Null | Local(_) | Global(_) | Func(..)
        | Lambda(_) | NewArray | NewMap | Break | Continue => {}
        Let(_, a)
        | SetLocal(_, a)
        | SetGlobal(_, a)
        | Unary(_, a)
        | Field(a, _)
        | FieldName(a, _)
        | Cast(a)
        | Unwrap(a)
        | Is(a, _)
        | Loop(a) => f(*a),
        SetField(a, _, b)
        | SetFieldName(a, _, b)
        | Binary(_, a, b)
        | And(a, b)
        | Or(a, b)
        | Index(a, b)
        | While(a, b) => {
            f(*a);
            f(*b);
        }
        SetIndex(a, b, c) => {
            f(*a);
            f(*b);
            f(*c);
        }
        Call(_, _, args)
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
        If(c, t, e) | IfLet(_, c, t, e) => {
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
