//! Type inference support: inference variables, unification, coercion, substitution of type
//! parameters, and resolution of a finished module's types.
//!
//! Unification is invariant: `array<T>` and `array<T?>` are different types. The only implicit
//! conversions are at the top of a type, in [`HirModule::coerce`]: `never` fits every type, and a
//! reference `T` fits `T?`.

use crate::model::*;

impl HirModule {
    pub(crate) fn fresh_var(&mut self) -> TypeId {
        let v = self.vars.len() as u32;
        self.vars.push(None);
        self.intern(TypeKind::Var(v))
    }

    /// `t` with bound variables replaced by what they are bound to. Unbound variables stay.
    pub(crate) fn resolve(&mut self, t: TypeId) -> TypeId {
        let t = self.shallow(t);
        let k = self.types[t.index()].clone();
        let r = |m: &mut HirModule, x: TypeId| m.resolve(x);
        let k = match k {
            TypeKind::Array(e) => TypeKind::Array(r(self, e)),
            TypeKind::Ptr(e) => TypeKind::Ptr(r(self, e)),
            TypeKind::Map(a, b) => TypeKind::Map(r(self, a), r(self, b)),
            TypeKind::Struct(s, args) => {
                TypeKind::Struct(s, args.into_iter().map(|a| r(self, a)).collect())
            }
            TypeKind::Enum(e, args) => {
                TypeKind::Enum(e, args.into_iter().map(|a| r(self, a)).collect())
            }
            TypeKind::Fn(params, ret) => {
                let params = params.into_iter().map(|p| r(self, p)).collect();
                TypeKind::Fn(params, r(self, ret))
            }
            TypeKind::Nullable(inner) => TypeKind::Nullable(r(self, inner)),
            _ => return t,
        };
        self.intern(k)
    }

    /// Whether `t` contains no unbound inference variables.
    pub(crate) fn is_resolved(&self, t: TypeId) -> bool {
        self.first_var(t).is_none()
    }

    /// The first unbound inference variable in `t`.
    pub(crate) fn first_var(&self, t: TypeId) -> Option<TypeId> {
        let t = self.shallow(t);
        match &self.types[t.index()] {
            TypeKind::Var(_) => Some(t),
            TypeKind::Array(e) | TypeKind::Nullable(e) | TypeKind::Ptr(e) => self.first_var(*e),
            TypeKind::Map(a, b) => self.first_var(*a).or_else(|| self.first_var(*b)),
            TypeKind::Struct(_, args) | TypeKind::Enum(_, args) => {
                args.iter().find_map(|&a| self.first_var(a))
            }
            TypeKind::Fn(params, ret) => params
                .iter()
                .find_map(|&p| self.first_var(p))
                .or_else(|| self.first_var(*ret)),
            _ => None,
        }
    }

    fn occurs(&self, v: u32, t: TypeId) -> bool {
        let t = self.shallow(t);
        match &self.types[t.index()] {
            TypeKind::Var(w) => *w == v,
            TypeKind::Array(e) | TypeKind::Nullable(e) | TypeKind::Ptr(e) => self.occurs(v, *e),
            TypeKind::Map(a, b) => self.occurs(v, *a) || self.occurs(v, *b),
            TypeKind::Struct(_, args) | TypeKind::Enum(_, args) => {
                args.iter().any(|&a| self.occurs(v, a))
            }
            TypeKind::Fn(params, ret) => {
                params.iter().any(|&p| self.occurs(v, p)) || self.occurs(v, *ret)
            }
            _ => false,
        }
    }

    /// Makes `a` and `b` the same type by binding inference variables. A failed unification may
    /// have bound some variables already.
    pub(crate) fn unify(&mut self, a: TypeId, b: TypeId) -> bool {
        let (a, b) = (self.shallow(a), self.shallow(b));
        if a == b {
            return true;
        }
        if a == TypeId::ERROR || b == TypeId::ERROR {
            self.absorb_error(a, b);
            return true;
        }
        let (ka, kb) = (self.types[a.index()].clone(), self.types[b.index()].clone());
        match (ka, kb) {
            (TypeKind::Var(v), _) => self.bind(v, b),
            (_, TypeKind::Var(v)) => self.bind(v, a),
            (TypeKind::Array(x), TypeKind::Array(y))
            | (TypeKind::Ptr(x), TypeKind::Ptr(y))
            | (TypeKind::Nullable(x), TypeKind::Nullable(y)) => self.unify(x, y),
            (TypeKind::Map(k1, v1), TypeKind::Map(k2, v2)) => {
                self.unify(k1, k2) && self.unify(v1, v2)
            }
            (TypeKind::Struct(s, xs), TypeKind::Struct(t, ys))
                if s == t && xs.len() == ys.len() =>
            {
                xs.into_iter().zip(ys).all(|(x, y)| self.unify(x, y))
            }
            (TypeKind::Enum(e, xs), TypeKind::Enum(f, ys)) if e == f && xs.len() == ys.len() => {
                xs.into_iter().zip(ys).all(|(x, y)| self.unify(x, y))
            }
            (TypeKind::Fn(p, r), TypeKind::Fn(q, s)) if p.len() == q.len() => {
                p.into_iter().zip(q).all(|(x, y)| self.unify(x, y)) && self.unify(r, s)
            }
            _ => false,
        }
    }

    fn bind(&mut self, v: u32, t: TypeId) -> bool {
        // `never` is the type of expressions with no value; it never fixes another type.
        if t == TypeId::NEVER {
            return true;
        }
        if self.occurs(v, t) {
            return false;
        }
        self.vars[v as usize] = Some(t);
        true
    }

    /// Whether a value of type `found` can be used where `expected` is required, binding inference
    /// variables as needed: `never` fits anything, a reference `T` fits `T?`, and otherwise the types
    /// must unify.
    pub(crate) fn coerce(&mut self, found: TypeId, expected: TypeId) -> bool {
        let (f, e) = (self.shallow(found), self.shallow(expected));
        // `never` on either side comes from code that does not produce a value, or from an error
        // already reported.
        if f == TypeId::NEVER || e == TypeId::NEVER || f == e {
            return true;
        }
        if f == TypeId::ERROR || e == TypeId::ERROR {
            self.absorb_error(f, e);
            return true;
        }
        if let TypeKind::Nullable(inner) = self.types[e.index()]
            && !matches!(
                self.types[f.index()],
                TypeKind::Nullable(_) | TypeKind::Var(_)
            )
        {
            return self.coerce(f, inner);
        }
        // An object of a type that implements an interface is a value of the interface (the
        // same reference); value types are not objects.
        if let TypeKind::Interface(i) = self.types[e.index()]
            && matches!(
                self.types[f.index()],
                TypeKind::Struct(..) | TypeKind::Enum(..) | TypeKind::Param(..)
            )
        {
            return crate::methods::implements(self, f, i);
        }
        self.unify(f, e)
    }

    /// One side is the error type: an inference variable on the other side becomes it too, so the
    /// error does not cause more errors further on.
    fn absorb_error(&mut self, a: TypeId, b: TypeId) {
        for t in [a, b] {
            if let TypeKind::Var(v) = self.types[t.index()] {
                self.vars[v as usize] = Some(TypeId::ERROR);
            }
        }
    }

    /// Whether `a` and `b` are the same type now, without binding anything.
    pub(crate) fn same(&self, a: TypeId, b: TypeId) -> bool {
        let (a, b) = (self.shallow(a), self.shallow(b));
        if a == b {
            return true;
        }
        match (&self.types[a.index()], &self.types[b.index()]) {
            (TypeKind::Array(x), TypeKind::Array(y))
            | (TypeKind::Ptr(x), TypeKind::Ptr(y))
            | (TypeKind::Nullable(x), TypeKind::Nullable(y)) => self.same(*x, *y),
            (TypeKind::Map(k1, v1), TypeKind::Map(k2, v2)) => {
                self.same(*k1, *k2) && self.same(*v1, *v2)
            }
            (TypeKind::Struct(s, xs), TypeKind::Struct(t, ys)) if s == t => self.same_all(xs, ys),
            (TypeKind::Enum(e, xs), TypeKind::Enum(f, ys)) if e == f => self.same_all(xs, ys),
            (TypeKind::Fn(p, r), TypeKind::Fn(q, s)) => {
                p.len() == q.len()
                    && p.iter().zip(q).all(|(&x, &y)| self.same(x, y))
                    && self.same(*r, *s)
            }
            _ => false,
        }
    }

    fn same_all(&self, xs: &[TypeId], ys: &[TypeId]) -> bool {
        xs.len() == ys.len() && xs.iter().zip(ys).all(|(&x, &y)| self.same(x, y))
    }

    /// `t` with the type parameters of `owner` replaced by `args`.
    pub(crate) fn subst(&mut self, t: TypeId, owner: Item, args: &[TypeId]) -> TypeId {
        if args.is_empty() {
            return t;
        }
        let t = self.shallow(t);
        let k = self.types[t.index()].clone();
        let s = |m: &mut HirModule, x: TypeId| m.subst(x, owner, args);
        let k = match k {
            TypeKind::Param(item, i) if item == owner => {
                return args.get(i as usize).copied().unwrap_or(TypeId::ERROR);
            }
            TypeKind::Array(e) => TypeKind::Array(s(self, e)),
            TypeKind::Ptr(e) => TypeKind::Ptr(s(self, e)),
            TypeKind::Map(a, b) => TypeKind::Map(s(self, a), s(self, b)),
            TypeKind::Struct(st, xs) => {
                TypeKind::Struct(st, xs.into_iter().map(|x| s(self, x)).collect())
            }
            TypeKind::Enum(e, xs) => {
                TypeKind::Enum(e, xs.into_iter().map(|x| s(self, x)).collect())
            }
            TypeKind::Fn(params, ret) => {
                let params = params.into_iter().map(|p| s(self, p)).collect();
                TypeKind::Fn(params, s(self, ret))
            }
            TypeKind::Nullable(inner) => TypeKind::Nullable(s(self, inner)),
            _ => return t,
        };
        self.intern(k)
    }

    /// The least type both `a` and `b` fit, binding inference variables if one of them is still
    /// open; `None` if there is none (the branches of a statement-like `if`).
    pub(crate) fn join(&mut self, a: TypeId, b: TypeId) -> Option<TypeId> {
        let (a, b) = (self.shallow(a), self.shallow(b));
        if a == TypeId::ERROR || b == TypeId::ERROR {
            return Some(TypeId::ERROR);
        }
        if a == TypeId::NEVER {
            return Some(b);
        }
        if b == TypeId::NEVER || self.same(a, b) {
            return Some(a);
        }
        for (x, y) in [(a, b), (b, a)] {
            if let TypeKind::Nullable(inner) = self.types[y.index()]
                && self.same(x, inner)
            {
                return Some(y);
            }
        }
        if (!self.is_resolved(a) || !self.is_resolved(b)) && self.unify(a, b) {
            return Some(a);
        }
        None
    }
}
