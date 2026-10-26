//! Static types for frontends: interned types, nominal structs, type variables with unification, and
//! the standard type-mismatch diagnostic.
//!
//! A frontend creates the types its language has ([`Types::named`] for `int` or `string`,
//! [`Types::array`], [`Types::function`], [`Types::declare_struct`]) and checks expressions with
//! [`Types::expect`]. Where a type is not known yet (the elements of `[]`, an unannotated variable), a
//! [`Types::fresh_var`] stands for it; unification fills it in from later uses, and
//! [`Types::resolve`] gives the final type.
//!
//! Types are interned, so two structurally equal types built from the same parts have the same id.
//! A type containing a variable is not equal to what the variable was later bound to, though: compare
//! with [`Types::same`] or [`Types::unify`], not `==`.

use std::collections::HashMap;

use a0_diag::{Diagnostic, Diagnostics, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct TypeId(pub u32);

impl TypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The shape of a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum TypeTag {
    /// A type known by name only: `int`, `string`, ... Equal names are the same type.
    Named,
    Array,
    Function,
    /// A declared struct. Every declaration is a distinct type, even with the same name.
    Struct,
    /// A type variable that is not bound (yet).
    Var,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Named(String),
    Array(TypeId),
    Function(Vec<TypeId>, TypeId),
    Struct(u32),
    Var(u32),
}

struct StructDef {
    name: String,
    fields: Vec<(String, TypeId)>,
}

/// A table of types.
#[derive(Default)]
pub struct Types {
    kinds: Vec<Kind>,
    interned: HashMap<Kind, TypeId>,
    structs: Vec<StructDef>,
    /// What each variable is bound to.
    bindings: Vec<Option<TypeId>>,
}

impl Types {
    pub fn new() -> Types {
        Types::default()
    }

    fn intern(&mut self, k: Kind) -> TypeId {
        if let Some(&t) = self.interned.get(&k) {
            return t;
        }
        let t = TypeId(self.kinds.len() as u32);
        self.kinds.push(k.clone());
        self.interned.insert(k, t);
        t
    }

    // ---------------------------------------------------------------- building

    /// The type called `name` (a language's primitive types).
    pub fn named(&mut self, name: &str) -> TypeId {
        self.intern(Kind::Named(name.to_string()))
    }

    pub fn array(&mut self, elem: TypeId) -> TypeId {
        self.intern(Kind::Array(elem))
    }

    pub fn function(&mut self, params: &[TypeId], ret: TypeId) -> TypeId {
        self.intern(Kind::Function(params.to_vec(), ret))
    }

    /// Declares a new struct type with no fields yet.
    pub fn declare_struct(&mut self, name: &str) -> TypeId {
        let s = self.structs.len() as u32;
        self.structs.push(StructDef {
            name: name.to_string(),
            fields: Vec::new(),
        });
        self.intern(Kind::Struct(s))
    }

    /// Adds a field to a struct and returns its index, or `None` if the struct already has a field
    /// with that name (or `s` is not a struct).
    pub fn add_field(&mut self, s: TypeId, name: &str, ty: TypeId) -> Option<u32> {
        let def = self.struct_def_mut(s)?;
        if def.fields.iter().any(|(n, _)| n == name) {
            return None;
        }
        def.fields.push((name.to_string(), ty));
        Some(def.fields.len() as u32 - 1)
    }

    /// A new, unbound type variable.
    pub fn fresh_var(&mut self) -> TypeId {
        let v = self.bindings.len() as u32;
        self.bindings.push(None);
        self.intern(Kind::Var(v))
    }

    // ---------------------------------------------------------------- reading

    /// Follows variable bindings until reaching a type that is not a bound variable.
    pub fn shallow(&self, mut t: TypeId) -> TypeId {
        while let Kind::Var(v) = self.kinds[t.index()] {
            match self.bindings[v as usize] {
                Some(b) => t = b,
                None => break,
            }
        }
        t
    }

    /// The shape of `t`, looking through bound variables.
    pub fn tag(&self, t: TypeId) -> TypeTag {
        match self.kinds[self.shallow(t).index()] {
            Kind::Named(_) => TypeTag::Named,
            Kind::Array(_) => TypeTag::Array,
            Kind::Function(..) => TypeTag::Function,
            Kind::Struct(_) => TypeTag::Struct,
            Kind::Var(_) => TypeTag::Var,
        }
    }

    /// The element type of an array type.
    pub fn elem(&self, t: TypeId) -> Option<TypeId> {
        match self.kinds[self.shallow(t).index()] {
            Kind::Array(e) => Some(e),
            _ => None,
        }
    }

    /// The parameter types of a function type (empty for other types).
    pub fn params(&self, t: TypeId) -> &[TypeId] {
        match &self.kinds[self.shallow(t).index()] {
            Kind::Function(p, _) => p,
            _ => &[],
        }
    }

    /// The result type of a function type.
    pub fn ret(&self, t: TypeId) -> Option<TypeId> {
        match self.kinds[self.shallow(t).index()] {
            Kind::Function(_, r) => Some(r),
            _ => None,
        }
    }

    fn struct_def(&self, t: TypeId) -> Option<&StructDef> {
        match self.kinds[self.shallow(t).index()] {
            Kind::Struct(s) => Some(&self.structs[s as usize]),
            _ => None,
        }
    }

    fn struct_def_mut(&mut self, t: TypeId) -> Option<&mut StructDef> {
        match self.kinds[self.shallow(t).index()] {
            Kind::Struct(s) => Some(&mut self.structs[s as usize]),
            _ => None,
        }
    }

    /// The name of a named or struct type.
    pub fn name(&self, t: TypeId) -> Option<&str> {
        match &self.kinds[self.shallow(t).index()] {
            Kind::Named(n) => Some(n),
            Kind::Struct(s) => Some(&self.structs[*s as usize].name),
            _ => None,
        }
    }

    pub fn field_count(&self, s: TypeId) -> u32 {
        self.struct_def(s).map_or(0, |d| d.fields.len() as u32)
    }

    pub fn field_index(&self, s: TypeId, name: &str) -> Option<u32> {
        let d = self.struct_def(s)?;
        d.fields
            .iter()
            .position(|(n, _)| n == name)
            .map(|i| i as u32)
    }

    pub fn field_name(&self, s: TypeId, i: u32) -> &str {
        &self.struct_def(s).expect("not a struct").fields[i as usize].0
    }

    pub fn field_type(&self, s: TypeId, i: u32) -> TypeId {
        self.struct_def(s).expect("not a struct").fields[i as usize].1
    }

    // ---------------------------------------------------------------- unification

    /// `t` with every bound variable replaced by what it is bound to. Unbound variables stay.
    pub fn resolve(&mut self, t: TypeId) -> TypeId {
        let t = self.shallow(t);
        match self.kinds[t.index()].clone() {
            Kind::Array(e) => {
                let e = self.resolve(e);
                self.array(e)
            }
            Kind::Function(params, ret) => {
                let params: Vec<TypeId> = params.into_iter().map(|p| self.resolve(p)).collect();
                let ret = self.resolve(ret);
                self.function(&params, ret)
            }
            _ => t,
        }
    }

    /// Whether `t` contains no unbound variables.
    pub fn is_complete(&self, t: TypeId) -> bool {
        match &self.kinds[self.shallow(t).index()] {
            Kind::Var(_) => false,
            Kind::Array(e) => self.is_complete(*e),
            Kind::Function(params, ret) => {
                params.iter().all(|&p| self.is_complete(p)) && self.is_complete(*ret)
            }
            _ => true,
        }
    }

    /// Whether `a` and `b` are the same type now, without binding anything.
    pub fn same(&self, a: TypeId, b: TypeId) -> bool {
        let (a, b) = (self.shallow(a), self.shallow(b));
        if a == b {
            return true;
        }
        match (&self.kinds[a.index()], &self.kinds[b.index()]) {
            (Kind::Array(x), Kind::Array(y)) => self.same(*x, *y),
            (Kind::Function(p, r), Kind::Function(q, s)) => {
                p.len() == q.len()
                    && p.iter().zip(q).all(|(&x, &y)| self.same(x, y))
                    && self.same(*r, *s)
            }
            _ => false,
        }
    }

    fn occurs(&self, v: u32, t: TypeId) -> bool {
        match &self.kinds[self.shallow(t).index()] {
            Kind::Var(w) => *w == v,
            Kind::Array(e) => self.occurs(v, *e),
            Kind::Function(params, ret) => {
                params.iter().any(|&p| self.occurs(v, p)) || self.occurs(v, *ret)
            }
            _ => false,
        }
    }

    /// Makes `a` and `b` the same type by binding variables, or returns `false` if they cannot be.
    /// A failed unification may have bound some variables already.
    pub fn unify(&mut self, a: TypeId, b: TypeId) -> bool {
        let (a, b) = (self.shallow(a), self.shallow(b));
        if a == b {
            return true;
        }
        match (self.kinds[a.index()].clone(), self.kinds[b.index()].clone()) {
            (Kind::Var(v), _) => self.bind(v, b),
            (_, Kind::Var(v)) => self.bind(v, a),
            (Kind::Array(x), Kind::Array(y)) => self.unify(x, y),
            (Kind::Function(p, r), Kind::Function(q, s)) => {
                p.len() == q.len()
                    && p.iter().zip(&q).all(|(&x, &y)| self.unify(x, y))
                    && self.unify(r, s)
            }
            _ => false,
        }
    }

    fn bind(&mut self, v: u32, t: TypeId) -> bool {
        if self.occurs(v, t) {
            return false;
        }
        self.bindings[v as usize] = Some(t);
        true
    }

    // ---------------------------------------------------------------- diagnostics

    /// The type as source code would write it: `int`, `[int]`, `fn(int, bool) -> string`, a struct's
    /// name, and `_` for a type not known yet.
    pub fn display(&self, t: TypeId) -> String {
        match &self.kinds[self.shallow(t).index()] {
            Kind::Named(n) => n.clone(),
            Kind::Struct(s) => self.structs[*s as usize].name.clone(),
            Kind::Var(_) => "_".to_string(),
            Kind::Array(e) => format!("[{}]", self.display(*e)),
            Kind::Function(params, ret) => {
                let params: Vec<String> = params.iter().map(|&p| self.display(p)).collect();
                format!("fn({}) -> {}", params.join(", "), self.display(*ret))
            }
        }
    }

    /// Checks that an expression of type `found` can be used where `expected` is required, unifying
    /// them. On a mismatch, reports `expected `T`, found `U`` (code `type.mismatch`) and returns
    /// `false`.
    pub fn expect(
        &mut self,
        found: TypeId,
        expected: TypeId,
        span: Option<Span>,
        diags: &mut Diagnostics,
    ) -> bool {
        // Describe the types before unifying: a failed unification can bind variables halfway.
        let message = format!(
            "mismatched types: expected `{}`, found `{}`",
            self.display(expected),
            self.display(found)
        );
        if self.unify(found, expected) {
            return true;
        }
        diags.push(Diagnostic::error("type.mismatch", &message).with_span_opt(span));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_are_interned_and_displayed() {
        let mut t = Types::new();
        let int = t.named("int");
        assert_eq!(int, t.named("int"));
        let ints = t.array(int);
        assert_eq!(ints, t.array(int));
        let f = t.function(&[int, ints], int);
        assert_eq!(t.tag(f), TypeTag::Function);
        assert_eq!(t.params(f), &[int, ints]);
        assert_eq!(t.ret(f), Some(int));
        assert_eq!(t.display(f), "fn(int, [int]) -> int");
        assert_eq!(t.elem(ints), Some(int));
        assert_eq!(t.elem(int), None);
    }

    #[test]
    fn structs_are_nominal() {
        let mut t = Types::new();
        let int = t.named("int");
        let a = t.declare_struct("P");
        let b = t.declare_struct("P");
        assert_ne!(a, b);
        assert!(!t.unify(a, b));
        assert_eq!(t.add_field(a, "x", int), Some(0));
        assert_eq!(t.add_field(a, "y", int), Some(1));
        assert_eq!(t.add_field(a, "x", int), None);
        assert_eq!(t.field_index(a, "y"), Some(1));
        assert_eq!(t.field_index(a, "z"), None);
        assert_eq!((t.field_name(a, 0), t.field_type(a, 0)), ("x", int));
        assert_eq!(t.field_count(b), 0);
        assert_eq!(t.name(a), Some("P"));
    }

    #[test]
    fn variables_unify_and_resolve() {
        let mut t = Types::new();
        let int = t.named("int");
        let string = t.named("string");
        let v = t.fresh_var();
        let vs = t.array(v);
        assert_eq!(t.display(vs), "[_]");
        assert!(!t.is_complete(vs));
        let ints = t.array(int);
        assert!(!t.same(vs, ints));
        assert!(t.unify(vs, ints));
        assert!(t.same(vs, ints));
        assert_eq!(t.resolve(vs), ints);
        assert!(t.is_complete(vs));
        assert!(!t.unify(v, string));
        // Occurs check: a variable cannot contain itself.
        let w = t.fresh_var();
        let ws = t.array(w);
        assert!(!t.unify(w, ws));
        // Functions unify parameter by parameter.
        let x = t.fresh_var();
        let f = t.function(&[x], x);
        let g = t.function(&[string], string);
        assert!(t.unify(f, g));
        assert_eq!(t.resolve(x), string);
        let h = t.function(&[], string);
        assert!(!t.unify(f, h));
    }

    #[test]
    fn mismatches_are_reported() {
        let mut t = Types::new();
        let int = t.named("int");
        let float = t.named("float");
        let mut diags = Diagnostics::new();
        assert!(t.expect(int, int, None, &mut diags));
        assert!(!t.expect(float, int, None, &mut diags));
        let d = diags.as_slice();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "type.mismatch");
        assert_eq!(
            d[0].message,
            "mismatched types: expected `int`, found `float`"
        );
    }
}
