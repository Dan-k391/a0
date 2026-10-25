//! Methods, interfaces and bounds on type parameters (D52).
//!
//! A method is a function that belongs to a struct, an enum or a value type, with the value it is
//! called on as its first parameter, `self`; `receiver.name(args)` finds it by the receiver's type.
//! An interface lists methods; a struct or enum implements it by declaring so and having them. A
//! value of an interface type is a reference to such an object, whose methods are found at run
//! time; a type parameter bounded by interfaces has their methods, found when monomorphization
//! knows the type.

use a0_diag::codes as code;
use a0_diag::similar_name;

use crate::builder::{HirBuilder, Problem};
use crate::model::*;

/// What a method of `receiver` called `name` resolves to, once the receiver's type is known.
pub(crate) enum Resolved {
    /// A method of a struct, enum or value type, with the type arguments of the call.
    Method(FuncId, Vec<TypeId>),
    /// The `n`th method of an interface.
    Interface(InterfaceId, u32),
    /// A field holding a closure, called.
    Field(u32),
    /// A method of a type parameter's bound: monomorphization finds the method.
    Bound,
    None,
}

/// The owner of methods of type `t`.
pub(crate) fn owner_of(m: &HirModule, t: TypeId) -> Option<Owner> {
    match m.kind(t) {
        TypeKind::Struct(s, _) => Some(Owner::Struct(*s)),
        TypeKind::Enum(e, _) => Some(Owner::Enum(*e)),
        TypeKind::Int => Some(Owner::Value(TypeId::INT)),
        TypeKind::Float => Some(Owner::Value(TypeId::FLOAT)),
        TypeKind::Bool => Some(Owner::Value(TypeId::BOOL)),
        TypeKind::String => Some(Owner::Value(TypeId::STRING)),
        _ => None,
    }
}

/// The method `name` of type `t`, as far as `t` is known.
pub(crate) fn resolve_method(m: &HirModule, t: TypeId, name: &str) -> Resolved {
    let t = m.shallow(t);
    match m.kind(t) {
        TypeKind::Interface(i) => match interface_method(m, *i, name) {
            Some(k) => Resolved::Interface(*i, k),
            None => Resolved::None,
        },
        TypeKind::Param(item, index) => {
            let bounded = m
                .bounds
                .get(&(*item, *index))
                .is_some_and(|bs| bs.iter().any(|&i| interface_method(m, i, name).is_some()));
            if bounded {
                Resolved::Bound
            } else {
                Resolved::None
            }
        }
        _ => {
            if let Some(owner) = owner_of(m, t)
                && let Some(&f) = m.methods.get(&(owner, name.to_string()))
            {
                return Resolved::Method(f, m.type_args(t).to_vec());
            }
            if let TypeKind::Struct(s, _) = m.kind(t)
                && let Some(field) = m.struct_field_index(*s, name)
            {
                return Resolved::Field(field);
            }
            Resolved::None
        }
    }
}

pub(crate) fn interface_method(m: &HirModule, i: InterfaceId, name: &str) -> Option<u32> {
    m.interfaces[i.index()]
        .methods
        .iter()
        .position(|s| s.name == name)
        .map(|k| k as u32)
}

/// The signature of method `name` of the bounds of a type parameter, with `Self` the parameter.
pub(crate) fn bound_method(
    m: &mut HirModule,
    param: TypeId,
    name: &str,
) -> Option<(Vec<TypeId>, TypeId, Option<a0_diag::Span>)> {
    let TypeKind::Param(item, index) = *m.kind(param) else {
        return None;
    };
    let bounds = m.bounds.get(&(item, index)).cloned().unwrap_or_default();
    for i in bounds {
        if let Some(k) = interface_method(m, i, name) {
            let sig = m.interfaces[i.index()].methods[k as usize].clone();
            let owner = Item::Interface(i);
            let params = sig
                .params
                .iter()
                .map(|&p| m.subst(p, owner, &[param]))
                .collect();
            let ret = m.subst(sig.ret, owner, &[param]);
            return Some((params, ret, sig.span));
        }
    }
    None
}

/// Whether a method of interface `i` mentions `Self` beyond its receiver, which an interface
/// value cannot supply (it does not know the type of the object it holds).
pub(crate) fn mentions_self(m: &HirModule, i: InterfaceId, sig: &MethodSig) -> bool {
    fn has(m: &HirModule, t: TypeId, i: InterfaceId) -> bool {
        match m.kind(t) {
            TypeKind::Param(Item::Interface(j), _) => *j == i,
            TypeKind::Array(e) | TypeKind::Nullable(e) => has(m, *e, i),
            TypeKind::Map(k, v) => has(m, *k, i) || has(m, *v, i),
            TypeKind::Struct(_, args) | TypeKind::Enum(_, args) => {
                args.iter().any(|&a| has(m, a, i))
            }
            TypeKind::Fn(ps, r) => ps.iter().any(|&p| has(m, p, i)) || has(m, *r, i),
            _ => false,
        }
    }
    sig.params.iter().any(|&p| has(m, p, i)) || has(m, sig.ret, i)
}

/// Whether values of type `t` have the methods of interface `i`: a struct or enum (or value type)
/// that implements it, a type parameter bounded by it, or the interface itself. Unknown and
/// erroneous types are given the benefit of the doubt (an error is reported elsewhere).
pub(crate) fn implements(m: &HirModule, t: TypeId, i: InterfaceId) -> bool {
    let t = m.shallow(t);
    match m.kind(t) {
        TypeKind::Interface(j) => *j == i,
        TypeKind::Param(item, index) => m
            .bounds
            .get(&(*item, *index))
            .is_some_and(|bs| bs.contains(&i)),
        TypeKind::Var(_) | TypeKind::Error | TypeKind::Never => true,
        _ => owner_of(m, t).is_some_and(|o| m.impls.iter().any(|&(x, j, _)| x == o && j == i)),
    }
}

impl HirBuilder {
    /// Declares an interface; add its methods with [`HirBuilder::add_interface_method`].
    pub fn declare_interface(&mut self, name: &str) -> InterfaceId {
        let i = InterfaceId(self.m.interfaces.len() as u32);
        self.m.interfaces.push(InterfaceDef {
            name: self.qualified(name),
            type_params: vec!["Self".to_string()],
            methods: Vec::new(),
            span: self.span,
        });
        self.add_type_name(name, Item::Interface(i));
        i
    }

    /// `Self` in an interface's methods: the type that implements it.
    pub fn interface_self(&mut self, i: InterfaceId) -> TypeId {
        self.m.intern(TypeKind::Param(Item::Interface(i), 0))
    }

    /// A method the interface requires: its parameters after `self`, and its result.
    pub fn add_interface_method(
        &mut self,
        i: InterfaceId,
        name: &str,
        params: &[TypeId],
        ret: TypeId,
    ) {
        if interface_method(&self.m, i, name).is_some() {
            let first = self.m.interfaces[i.index()]
                .methods
                .iter()
                .find(|s| s.name == name)
                .and_then(|s| s.span);
            self.declared_twice(&format!("the method `{name}`"), first);
            return;
        }
        self.m.interfaces[i.index()].methods.push(MethodSig {
            name: name.to_string(),
            params: params.to_vec(),
            ret,
            span: self.span,
        });
    }

    /// The type of values that implement the interface.
    pub fn interface_type(&mut self, i: InterfaceId) -> TypeId {
        self.m.intern(TypeKind::Interface(i))
    }

    /// Declares a method of `owner` (a struct or enum type, or `int`, `float`, `bool` or
    /// `string`). Its type parameters are the owner's (in the same order and names) followed by
    /// `type_params`; give its signature with [`HirBuilder::set_signature`], with `self` (of type
    /// [`HirBuilder::method_self_type`]) first.
    pub fn declare_method(&mut self, owner: TypeId, name: &str, type_params: &[&str]) -> FuncId {
        let Some(o) = owner_of(&self.m, owner) else {
            let t = self.m.display_type(owner);
            self.misuse(&format!(
                "methods belong to structs, enums and value types, not {t}"
            ));
            return self.declare_generic_function(name, type_params);
        };
        let (short, mut params) = match o {
            Owner::Struct(s) => (
                self.m.structs[s.index()].name.clone(),
                self.m.structs[s.index()].type_params.clone(),
            ),
            Owner::Enum(e) => (
                self.m.enums[e.index()].name.clone(),
                self.m.enums[e.index()].type_params.clone(),
            ),
            Owner::Value(t) => (self.m.type_name(t), Vec::new()),
        };
        // `geo.Circle.area` in module `geo`, which qualifies the name itself.
        let module = self.qualified("");
        let short = short.strip_prefix(&module).unwrap_or(&short).to_string();
        params.extend(type_params.iter().map(|p| p.to_string()));
        let params: Vec<&str> = params.iter().map(String::as_str).collect();
        let f = self.declare_generic_function(&format!("{short}.{name}"), &params);
        if let Some(&first) = self.m.methods.get(&(o, name.to_string())) {
            let span = self.m.funcs[first.index()].span;
            self.declared_twice(&format!("the method `{name}`"), span);
        }
        self.m.methods.insert((o, name.to_string()), f);
        self.m.method_owner.insert(f, o);
        f
    }

    /// The type of `self` in method `f`: its owner, with the method's first type parameters as the
    /// owner's type arguments.
    pub fn method_self_type(&mut self, f: FuncId) -> TypeId {
        match self.m.method_owner.get(&f).copied() {
            Some(Owner::Struct(s)) => {
                let n = self.m.structs[s.index()].type_params.len() as u32;
                let args: Vec<TypeId> = (0..n).map(|i| self.type_param(Item::Func(f), i)).collect();
                self.struct_type_args(s, &args)
            }
            Some(Owner::Enum(e)) => {
                let n = self.m.enums[e.index()].type_params.len() as u32;
                let args: Vec<TypeId> = (0..n).map(|i| self.type_param(Item::Func(f), i)).collect();
                self.enum_type_args(e, &args)
            }
            Some(Owner::Value(t)) => t,
            None => {
                self.misuse("method_self_type of a function that is not a method");
                self.fresh_type()
            }
        }
    }

    /// Declares that `owner` (as for [`HirBuilder::declare_method`]) implements interface `i`; it
    /// must have its methods when the module is finished.
    pub fn implement(&mut self, owner: TypeId, i: InterfaceId) {
        let Some(o) = owner_of(&self.m, owner) else {
            let t = self.m.display_type(owner);
            self.misuse(&format!(
                "only structs, enums and value types implement interfaces, not {t}"
            ));
            return;
        };
        if let Some(&(_, _, first)) = self.m.impls.iter().find(|&&(x, j, _)| x == o && j == i) {
            let p = Problem::new(
                code::DUPLICATE,
                format!(
                    "`{}` is declared twice to implement `{}`",
                    self.m.display_type(owner),
                    self.m.interfaces[i.index()].name
                ),
            )
            .label("declared again here")
            .at(first, "first declared here");
            self.problem(&p);
            return;
        }
        let span = self.span;
        self.m.impls.push((o, i, span));
    }

    /// Requires type parameter `index` of a generic function (or struct, or enum) to implement `i`:
    /// its values then have `i`'s methods.
    pub fn add_bound(&mut self, item: Item, index: u32, i: InterfaceId) {
        if index as usize >= self.m.type_params(item).len() {
            self.misuse("add_bound: no such type parameter");
            return;
        }
        let bounds = self.m.bounds.entry((item, index)).or_default();
        if !bounds.contains(&i) {
            bounds.push(i);
        }
    }

    /// `receiver.name(args)`: a method of the receiver's type (found now if the type is known,
    /// else when the module is finished), a method of an interface value, of a type parameter's
    /// bounds, or a closure in a field of that name.
    pub fn method_call(&mut self, receiver: ExprId, name: &str, args: &[ExprId]) -> ExprId {
        let rt = self.m.shallow(self.m.expr_type(receiver));
        match resolve_method(&self.m, rt, name) {
            Resolved::Method(f, mut targs) => {
                let own = self.m.funcs[f.index()].type_params.len() - targs.len();
                targs.extend((0..own).map(|_| self.m.fresh_var()));
                let mut all = vec![receiver];
                all.extend_from_slice(args);
                self.call_generic(f, &targs, &all)
            }
            Resolved::Interface(i, k) => self.interface_call(receiver, i, k, args),
            Resolved::Field(_) => {
                let callee = self.field_named(receiver, name);
                self.call_value(callee, args)
            }
            Resolved::Bound | Resolved::None => self.push_method_call(receiver, name, args),
        }
    }

    /// A call of the `method`th method of interface `i` on `receiver`, an interface value.
    pub fn interface_call(
        &mut self,
        receiver: ExprId,
        i: InterfaceId,
        method: u32,
        args: &[ExprId],
    ) -> ExprId {
        if method as usize >= self.m.interfaces[i.index()].methods.len() {
            self.misuse("interface_call: no such method");
            return self.error_expr();
        }
        self.push(
            ExprKind::InterfaceCall(receiver, i, method, args.to_vec()),
            TypeId::NEVER,
        )
    }

    fn push_method_call(&mut self, receiver: ExprId, name: &str, args: &[ExprId]) -> ExprId {
        self.push(
            ExprKind::MethodCall(receiver, name.to_string(), args.to_vec()),
            TypeId::NEVER,
        )
    }

    /// The problem with `receiver.name`, a method nothing has.
    pub(crate) fn unknown_method(m: &HirModule, t: TypeId, name: &str) -> Box<Problem> {
        let mut names: Vec<&str> = Vec::new();
        if let Some(o) = owner_of(m, t) {
            names.extend(
                m.methods
                    .keys()
                    .filter(|(x, _)| *x == o)
                    .map(|(_, n)| n.as_str()),
            );
        }
        if let TypeKind::Interface(i) = m.kind(t) {
            names.extend(
                m.interfaces[i.index()]
                    .methods
                    .iter()
                    .map(|s| s.name.as_str()),
            );
        }
        names.sort();
        let p = Problem::new(
            code::UNKNOWN_METHOD,
            format!("no method `{name}` on {}", m.quote_type(t)),
        );
        match similar_name(name, &names) {
            Some(close) => p.similar("a method", name, Some(close)),
            None => p.label("method not found"),
        }
    }

    // ---------------------------------------------------------------- checks at `finish`

    /// Every implementation has the interface's methods, with its signatures (`Self` being the
    /// implementing type), and no type parameters of their own.
    pub(crate) fn check_impls(&mut self) {
        for (owner, i, span) in self.m.impls.clone() {
            let iface = self.m.interfaces[i.index()].clone();
            let owner_name = match owner {
                Owner::Struct(s) => self.m.structs[s.index()].name.clone(),
                Owner::Enum(e) => self.m.enums[e.index()].name.clone(),
                Owner::Value(t) => self.m.type_name(t),
            };
            for sig in &iface.methods {
                let Some(&f) = self.m.methods.get(&(owner, sig.name.clone())) else {
                    let p = Problem::new(
                        code::NOT_IMPLEMENTED,
                        format!(
                            "`{owner_name}` does not have the method `{}` that `{}` requires",
                            sig.name, iface.name
                        ),
                    )
                    .label(format!("`{}` is missing", sig.name))
                    .at(sig.span, "required here");
                    self.problem_at(span, &p);
                    continue;
                };
                let owner_params = match owner {
                    Owner::Struct(s) => self.m.structs[s.index()].type_params.len(),
                    Owner::Enum(e) => self.m.enums[e.index()].type_params.len(),
                    Owner::Value(_) => 0,
                };
                let self_type = self.method_self_type(f);
                let wanted: Vec<TypeId> = sig
                    .params
                    .iter()
                    .map(|&p| self.m.subst(p, Item::Interface(i), &[self_type]))
                    .collect();
                let wanted_ret = self.m.subst(sig.ret, Item::Interface(i), &[self_type]);
                let body = self.m.func_body(f);
                let have: Vec<TypeId> = self.m.body_params(body)[1..]
                    .iter()
                    .map(|&l| self.m.local_type(l))
                    .collect();
                let have_ret = self.m.body_ret(body);
                let own_params = self.m.funcs[f.index()].type_params.len() > owner_params;
                let same = have.len() == wanted.len()
                    && have.iter().zip(&wanted).all(|(&a, &b)| self.m.same(a, b))
                    && self.m.same(have_ret, wanted_ret);
                if own_params || !same {
                    let fmt = |m: &HirModule, ps: &[TypeId], r: TypeId| {
                        let ps: Vec<String> = ps.iter().map(|&p| m.display_type(p)).collect();
                        format!("fn({}) -> {}", ps.join(", "), m.display_type(r))
                    };
                    let p = Problem::new(
                        code::NOT_IMPLEMENTED,
                        format!(
                            "the method `{}` of `{owner_name}` does not match `{}`",
                            sig.name, iface.name
                        ),
                    )
                    .label(format!(
                        "expected `{}`, found `{}`",
                        fmt(&self.m, &wanted, wanted_ret),
                        fmt(&self.m, &have, have_ret)
                    ))
                    .at(sig.span, "required here");
                    let p = if own_params {
                        p.note("a method that implements an interface has no type parameters of its own")
                    } else {
                        p
                    };
                    let at = self.m.funcs[f.index()].span;
                    self.problem_at(at, &p);
                }
            }
        }
    }

    /// Type arguments of calls to generic functions implement the interfaces their parameters are
    /// bounded by.
    pub(crate) fn check_bounds(&mut self) {
        if self.m.bounds.is_empty() {
            return;
        }
        for e in 0..self.m.exprs.len() {
            let (f, targs) = match &self.m.exprs[e].kind {
                ExprKind::Call(f, targs, _) | ExprKind::Func(f, targs) => (*f, targs.clone()),
                _ => continue,
            };
            for (index, &t) in targs.iter().enumerate() {
                let Some(bounds) = self.m.bounds.get(&(Item::Func(f), index as u32)).cloned()
                else {
                    continue;
                };
                for i in bounds {
                    if implements(&self.m, t, i) {
                        continue;
                    }
                    let fname = self.m.func_name(f).to_string();
                    let p = Problem::new(
                        code::NOT_IMPLEMENTED,
                        format!(
                            "{} does not implement `{}`",
                            self.m.quote_type(t),
                            self.m.interfaces[i.index()].name
                        ),
                    )
                    .label(format!(
                        "`{fname}` requires it of `{}`",
                        self.m.type_params(Item::Func(f))[index]
                    ))
                    .at(
                        self.m.funcs[f.index()].span,
                        format!("`{fname}` declared here"),
                    );
                    let span = self.m.exprs[e].span;
                    self.problem_at(span, &p);
                }
            }
        }
    }

    /// The method tables of implementations by owners without type parameters (monomorphization
    /// makes those of the others).
    pub(crate) fn build_vtables(&mut self) {
        self.m.vtables = vtables_of(&self.m);
    }
}

/// The method table of each implementation by an owner without type parameters.
pub(crate) fn vtables_of(m: &HirModule) -> Vec<(Owner, InterfaceId, Vec<FuncId>)> {
    let mut out = Vec::new();
    for &(owner, i, _) in &m.impls {
        let generic = match owner {
            Owner::Struct(s) => !m.structs[s.index()].type_params.is_empty(),
            Owner::Enum(e) => !m.enums[e.index()].type_params.is_empty(),
            Owner::Value(_) => true,
        };
        if generic {
            continue;
        }
        let methods: Option<Vec<FuncId>> = m.interfaces[i.index()]
            .methods
            .iter()
            .map(|s| m.methods.get(&(owner, s.name.clone())).copied())
            .collect();
        if let Some(methods) = methods {
            out.push((owner, i, methods));
        }
    }
    out
}
