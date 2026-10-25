//! Conveniences for frontends: names in scope and construction by name, with errors reported in
//! the language's terms at the current span.
//!
//! A frontend that resolves names itself (Lox, with jlox's rules) can ignore all of this and build
//! with handles. One that does not opens each body with [`HirBuilder::begin_body`], declares locals
//! with [`HirBuilder::let_named`] or [`HirBuilder::declare_local`], and refers to everything by name:
//! [`HirBuilder::name`], [`HirBuilder::call_named`], [`HirBuilder::assign_named`],
//! [`HirBuilder::named_type`], [`HirBuilder::new_struct_named`] and [`HirBuilder::variant_named`].

use a0_diag::codes as code;
use a0_diag::{Diagnostic, Span, similar_name};
use a0_sema::ScopeKind;

use crate::builder::{HirBuilder, Problem};
use crate::model::*;

impl HirBuilder {
    /// Makes `body` the body being built: its parameters, and a function's type parameters, are in
    /// scope until the matching [`HirBuilder::end_body`]. Bodies nest (a closure inside a function).
    pub fn begin_body(&mut self, body: BodyId) {
        let data = &self.m.bodies[body.index()];
        let (params, names, parent, func) = (
            data.params.clone(),
            data.param_names.clone(),
            data.parent,
            data.func,
        );
        let saved = std::mem::take(&mut self.type_scope);
        if parent.is_none() {
            self.bind_type_params(Item::Func(func));
        } else {
            self.type_scope = saved.clone();
        }
        self.body_stack.push((body, saved));
        self.names.push(ScopeKind::Function);
        self.shadow_scopes.push(0);
        for (name, l) in names.iter().zip(params) {
            let _ = self.names.declare_defined(name, l.0 as u64);
        }
    }

    pub fn end_body(&mut self) {
        self.close_scope();
        if let Some((_, saved)) = self.body_stack.pop() {
            self.type_scope = saved;
        }
    }

    /// Whether a body is being built, as declaring locals needs.
    pub fn in_body(&self) -> bool {
        !self.body_stack.is_empty()
    }

    /// The body being built (opened with [`HirBuilder::begin_body`]).
    pub fn current_body(&self) -> BodyId {
        match self.body_stack.last() {
            Some((b, _)) => *b,
            None => panic!("no body is being built; call begin_body first"),
        }
    }

    /// Opens a block scope; names declared in it disappear at [`HirBuilder::pop_scope`].
    pub fn push_scope(&mut self) {
        self.names.push(ScopeKind::Block);
        self.shadow_scopes.push(0);
    }

    pub fn pop_scope(&mut self) {
        self.close_scope();
    }

    fn close_scope(&mut self) {
        for _ in 0..self.shadow_scopes.pop().unwrap_or(0) {
            self.names.pop();
        }
        self.names.pop();
    }

    /// Whether `name` is already declared in the innermost scope (for languages that forbid
    /// redeclaring a name in the same block; a new declaration otherwise shadows it).
    pub fn is_declared_in_scope(&self, name: &str) -> bool {
        self.names.lookup_in_current(name).is_some()
    }

    /// Declares a local of the current body in the innermost scope. The type may be
    /// [`HirBuilder::fresh_type`].
    pub fn declare_local(&mut self, name: &str, ty: TypeId) -> LocalId {
        let l = self.new_local(self.current_body(), name, ty);
        if self.is_declared_in_scope(name) {
            // Shadowing in the same block: the new name lives in a nested scope until the block ends.
            self.names.push(ScopeKind::Block);
            if let Some(n) = self.shadow_scopes.last_mut() {
                *n += 1;
            }
        }
        let _ = self.names.declare_defined(name, l.0 as u64);
        l
    }

    /// Declares the local that `if let name = value` binds, in the innermost scope (push one for the
    /// `then` branch first). Its type is `value`'s without the `?`; pass it to
    /// [`HirBuilder::if_let`].
    pub fn declare_if_let(&mut self, name: &str, value: ExprId) -> LocalId {
        let t = match self.m.kind(self.m.expr_type(value)) {
            TypeKind::Nullable(inner) => *inner,
            _ => self.fresh_type(),
        };
        self.declare_local(name, t)
    }

    /// `let name: ty = value`, with `name` in scope afterwards (not inside `value`). `ty` may be
    /// [`HirBuilder::fresh_type`] to take the value's type.
    pub fn let_named(&mut self, name: &str, ty: TypeId, value: ExprId) -> ExprId {
        let l = self.declare_local(name, ty);
        self.let_(l, value)
    }

    pub fn lookup_local(&self, name: &str) -> Option<LocalId> {
        self.names
            .lookup(name)
            .map(|r| LocalId(self.names.data(r.symbol) as u32))
    }

    /// A frontend's own error at the current span, without a code.
    pub fn report(&mut self, message: &str) {
        self.report_code("", message);
    }

    /// A frontend's own error at the current span, with a code (the language's, or one of A0's
    /// from `a0_diag::codes`).
    pub fn report_code(&mut self, code: &str, message: &str) {
        self.diags
            .push(Diagnostic::error(code, message).with_span_opt(self.span));
    }

    /// Says what is wrong at the span of the last error reported.
    pub fn report_primary_label(&mut self, message: &str) {
        if let Some(d) = self.diags.last_mut() {
            d.label = message.to_string();
        }
    }

    /// Points the last error reported at a related location, such as an earlier declaration.
    pub fn report_label(&mut self, span: Span, message: &str) {
        if let Some(d) = self.diags.last_mut() {
            d.labels.push(a0_diag::Label {
                span,
                message: message.to_string(),
            });
        }
    }

    pub fn report_note(&mut self, note: &str) {
        if let Some(d) = self.diags.last_mut() {
            d.notes.push(note.to_string());
        }
    }

    pub fn report_help(&mut self, help: &str) {
        if let Some(d) = self.diags.last_mut() {
            d.helps.push(help.to_string());
        }
    }

    /// Suggests replacing `span` with `text` to fix the last error reported.
    pub fn report_fix(&mut self, message: &str, span: Span, text: &str) {
        if let Some(d) = self.diags.last_mut() {
            *d = std::mem::replace(d, Diagnostic::error("", "")).with_fix(message, span, text);
        }
    }

    /// Reports `name` as unknown: a `what` ("name", "function") nothing declares, with the closest
    /// of `candidates` as a suggestion.
    fn unknown(&mut self, code: &'static str, what: &str, name: &str, candidates: &[&str]) {
        let span = self.take_name_span(name);
        let p = Problem::new(code, format!("unknown {what} `{name}`"));
        let p = match similar_name(name, candidates) {
            Some(close) => {
                let kind = self.kind_of_name(&close);
                p.similar(&kind, name, Some(close))
            }
            None => p.label("not found in this scope"),
        };
        // At the name itself when the frontend said where it is (a call's callee).
        self.diags.push(p.diagnostic(span.or(self.span), span));
    }

    /// What a name in scope is, as in "a variable".
    fn kind_of_name(&self, name: &str) -> String {
        if self.lookup_local(name).is_none()
            && self.is_module(name)
            && self.visible(name, false).is_none()
        {
            return "a module".to_string();
        }
        let term = if self.lookup_local(name).is_some() {
            Term::Variable
        } else {
            match self.visible(name, false) {
                Some(Item::Global(_)) => return "a global".to_string(),
                Some(_) => Term::Function,
                None if Builtin::from_name(name).is_some() => Term::Function,
                None => match self.visible(name, true) {
                    Some(Item::Struct(_)) => Term::Struct,
                    Some(Item::Enum(_)) => Term::Enum,
                    _ => return "a type".to_string(),
                },
            }
        };
        let word = self.m.term(term);
        let article = if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
            "an"
        } else {
            "a"
        };
        format!("{article} {word}")
    }

    /// The names a value can be referred to by here: locals (innermost first), globals and
    /// functions, and, for calls, C functions and builtins.
    fn value_candidates(&self, calls: bool, globals_only: bool) -> Vec<String> {
        let mut out: Vec<String> = self
            .names
            .visible_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        for name in self.visible_names(false) {
            let ok = match self.visible(&name, false) {
                Some(Item::Global(_)) => true,
                Some(Item::Func(_)) => !globals_only,
                Some(Item::Extern(_)) => calls,
                _ => false,
            };
            if ok {
                out.push(name);
            }
        }
        if calls {
            out.extend(BUILTINS.iter().map(|b| b.to_string()));
        }
        // A misspelled module before `.`: the frontend took it for a value.
        out.extend(
            self.m.modules[self.module.index()]
                .imports
                .iter()
                .map(|(alias, ..)| alias.clone()),
        );
        out
    }

    /// A placeholder for an expression that could not be built; its type fits everywhere, so it
    /// causes no further errors. Report the reason first.
    pub fn error_expr(&mut self) -> ExprId {
        self.push_typed(ExprKind::Unit, TypeId::ERROR)
    }

    /// The value of a name: a local, a global, or a function as a closure value.
    pub fn name(&mut self, name: &str) -> ExprId {
        if let Some(l) = self.lookup_local(name) {
            return self.local(l);
        }
        match self.visible(name, false) {
            Some(Item::Global(g)) => self.global(g),
            Some(Item::Func(f)) => self.func(f),
            Some(Item::Extern(_)) => {
                self.problem(
                    &Problem::new(
                        code::NOT_A_VALUE,
                        format!(
                            "the C {} `{name}` can only be called",
                            self.m.term(Term::Function)
                        ),
                    )
                    .label("not a value"),
                );
                self.error_expr()
            }
            _ => {
                let candidates = self.value_candidates(false, false);
                let candidates: Vec<&str> = candidates.iter().map(String::as_str).collect();
                self.unknown(code::UNKNOWN_NAME, "name", name, &candidates);
                self.error_expr()
            }
        }
    }

    /// Calls a name: a local or global holding a closure, a function, a C function, or a HIR
    /// builtin (`len`, `push`, ...).
    pub fn call_named(&mut self, name: &str, args: &[ExprId]) -> ExprId {
        if let Some(l) = self.lookup_local(name) {
            let c = self.local(l);
            return self.call_value(c, args);
        }
        match self.visible(name, false) {
            Some(Item::Func(f)) => self.call(f, args),
            Some(Item::Extern(x)) => self.call_extern(x, args),
            Some(Item::Global(g)) => {
                let c = self.global(g);
                self.call_value(c, args)
            }
            _ => match Builtin::from_name(name) {
                Some(op) => self.builtin(op, args),
                None => {
                    let candidates = self.value_candidates(true, false);
                    let candidates: Vec<&str> = candidates.iter().map(String::as_str).collect();
                    let what = self.m.term(Term::Function);
                    self.unknown(code::UNKNOWN_NAME, &what, name, &candidates);
                    self.error_expr()
                }
            },
        }
    }

    /// `name = value` for a local or a global.
    pub fn assign_named(&mut self, name: &str, value: ExprId) -> ExprId {
        if let Some(l) = self.lookup_local(name) {
            return self.set_local(l, value);
        }
        match self.visible(name, false) {
            Some(Item::Global(g)) => self.set_global(g, value),
            Some(Item::Func(f)) => {
                let function = self.m.term(Term::Function);
                let p = Problem::new(
                    code::NOT_ASSIGNABLE,
                    format!("cannot assign to the {function} `{name}`"),
                )
                .label(format!("`{name}` is a {function}"))
                .at(self.m.funcs[f.index()].span, "declared here");
                self.problem(&p);
                self.error_expr()
            }
            Some(Item::Extern(_)) => {
                let function = self.m.term(Term::Function);
                self.problem(&Problem::new(
                    code::NOT_ASSIGNABLE,
                    format!("cannot assign to the {function} `{name}`"),
                ));
                self.error_expr()
            }
            _ => {
                let candidates = self.value_candidates(false, true);
                let candidates: Vec<&str> = candidates.iter().map(String::as_str).collect();
                self.unknown(code::UNKNOWN_NAME, "name", name, &candidates);
                self.error_expr()
            }
        }
    }

    /// Puts the type parameters of a generic item in scope for [`HirBuilder::named_type`], for
    /// declaring its fields, variants or signature. A function's body gets its own through
    /// [`HirBuilder::begin_body`].
    pub fn bind_type_params(&mut self, item: Item) {
        let names = self.m.type_params(item).to_vec();
        self.type_scope = names
            .into_iter()
            .enumerate()
            .map(|(i, n)| (n, self.m.intern(TypeKind::Param(item, i as u32))))
            .collect();
    }

    /// A type by name: a type parameter in scope, a primitive (`unit`, `bool`, `int`, `float`,
    /// `string`, `any`) or a declared struct or enum. A generic struct or enum named without type
    /// arguments gets inferred ones.
    pub fn named_type(&mut self, name: &str, args: &[TypeId]) -> TypeId {
        if let Some(&(_, t)) = self.type_scope.iter().find(|(n, _)| n == name) {
            return t;
        }
        let primitive = match name {
            "unit" => Some(TypeId::UNIT),
            "bool" => Some(TypeId::BOOL),
            "int" => Some(TypeId::INT),
            "float" => Some(TypeId::FLOAT),
            "string" => Some(TypeId::STRING),
            "any" => Some(TypeId::ANY),
            _ => None,
        };
        if let Some(t) = primitive {
            return t;
        }
        let item = match self.visible(name, true) {
            Some(item) => item,
            None => {
                let mut candidates: Vec<String> =
                    self.type_scope.iter().map(|(n, _)| n.clone()).collect();
                candidates.extend(self.visible_names(true));
                candidates.extend(["bool", "int", "float", "string", "any"].map(str::to_string));
                let candidates: Vec<&str> = candidates.iter().map(String::as_str).collect();
                self.unknown(code::UNKNOWN_TYPE, "type", name, &candidates);
                return self.fresh_type();
            }
        };
        self.instantiate_named(item, args)
    }

    /// The type of a struct or enum with type arguments (inferred ones if `args` is empty).
    pub(crate) fn instantiate_named(&mut self, item: Item, args: &[TypeId]) -> TypeId {
        let args = if args.is_empty() {
            (0..self.m.type_params(item).len())
                .map(|_| self.fresh_type())
                .collect()
        } else {
            args.to_vec()
        };
        match item {
            Item::Struct(s) => self.struct_type_args(s, &args),
            Item::Enum(e) => self.enum_type_args(e, &args),
            _ => unreachable!("only structs and enums are types"),
        }
    }

    /// `new S { name: value, ... }` with fields in any order. Values are evaluated in the order
    /// given; unknown, repeated and missing fields are reported.
    pub fn new_struct_named(&mut self, s: StructId, names: &[&str], values: &[ExprId]) -> ExprId {
        let sname = self.m.struct_name(s).to_string();
        let count = self.m.struct_field_count(s) as usize;
        let mut slots: Vec<Option<usize>> = vec![None; count];
        let mut ok = true;
        let field = self.m.term(Term::Field);
        let decl = self.m.structs[s.index()].span;
        for (k, name) in names.iter().enumerate() {
            match self.m.struct_field_index(s, name) {
                Some(i) if slots[i as usize].is_some() => {
                    let first = slots[i as usize].and_then(|j| self.m.expr_span(values[j]));
                    let p = Problem::new(
                        code::FIELD_TWICE,
                        format!("the {field} `{name}` is given twice"),
                    )
                    .at(first, "first given here");
                    let at = self.m.expr_span(values[k]).or(self.span);
                    self.problem_at(at, &p);
                    ok = false;
                }
                Some(i) => slots[i as usize] = Some(k),
                None => {
                    let fields: Vec<String> = (0..count)
                        .map(|i| self.m.struct_field(s, i as u32).0.to_string())
                        .collect();
                    let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
                    let p = Problem::new(
                        code::UNKNOWN_FIELD,
                        format!("`{sname}` has no {field} `{name}`"),
                    )
                    .label(format!("unknown {field}"))
                    .at(decl, format!("`{sname}` declared here"))
                    .similar(
                        &format!("a {field}"),
                        name,
                        similar_name(name, &fields),
                    );
                    self.problem(&p);
                    ok = false;
                }
            }
        }
        let missing: Vec<String> = (0..count)
            .filter(|&i| slots[i].is_none())
            .map(|i| format!("`{}`", self.m.struct_field(s, i as u32).0))
            .collect();
        if !missing.is_empty() {
            let list = match missing.as_slice() {
                [one] => one.clone(),
                [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
                [] => String::new(),
            };
            let what = if missing.len() == 1 {
                field.clone()
            } else {
                format!("{field}s")
            };
            let p = Problem::new(
                code::MISSING_FIELDS,
                format!("missing {what} {list} in `{sname}`"),
            )
            .label(format!("missing {list}"))
            .at(decl, format!("`{sname}` declared here"));
            self.problem(&p);
            ok = false;
        }
        if !ok {
            return self.error_expr();
        }
        let order: Vec<usize> = slots.into_iter().map(Option::unwrap).collect();
        if order.windows(2).all(|w| w[0] < w[1]) || self.body_stack.is_empty() {
            let fields: Vec<ExprId> = order.iter().map(|&k| values[k]).collect();
            return self.new_struct(s, &fields);
        }
        // Out of declaration order: evaluate into temporaries first.
        let body = self.current_body();
        let mut lets = Vec::new();
        let mut temps = Vec::new();
        for &v in values {
            let t = self.type_of(v);
            let l = self.new_local(body, "field", t);
            lets.push(self.let_(l, v));
            temps.push(l);
        }
        let fields: Vec<ExprId> = order.iter().map(|&k| self.local(temps[k])).collect();
        let new = self.new_struct(s, &fields);
        self.block(&lets, Some(new))
    }

    /// `E::name(args)`; an unknown variant is reported.
    pub fn variant_named(&mut self, e: EnumId, name: &str, args: &[ExprId]) -> ExprId {
        match self.m.variant_index(e, name) {
            Some(v) => self.variant(e, v, args),
            None => {
                self.unknown_variant(e, name);
                self.error_expr()
            }
        }
    }

    /// Reports that the enum `e` has no variant `name`, suggesting a close one.
    pub fn unknown_variant(&mut self, e: EnumId, name: &str) {
        let ename = self.m.enum_name(e).to_string();
        let variant = self.m.term(Term::Variant);
        let names: Vec<String> = (0..self.m.variant_count(e))
            .map(|v| self.m.variant_name(e, v).to_string())
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let list: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
        let span = self.take_name_span(name);
        let p = Problem::new(
            code::UNKNOWN_VARIANT,
            format!("`{ename}` has no {variant} `{name}`"),
        )
        .label(format!("unknown {variant}"))
        .at(
            self.m.enums[e.index()].span,
            format!("`{ename}` declared here"),
        )
        .note(format!(
            "the {variant}s of `{ename}` are {}",
            list.join(", ")
        ))
        .similar(&format!("a {variant}"), name, similar_name(name, &names));
        self.diags.push(p.diagnostic(span.or(self.span), span));
    }

    /// How diagnostics write types of this kind, in the language's syntax: `{}` stands for the
    /// element type of an array (`"[{}]"`), the key and value types of a map, or the inner type of
    /// a nullable type (`"{}?"`).
    pub fn set_type_syntax(&mut self, tag: TypeTag, template: &str) {
        self.m.type_syntax.retain(|(t, _)| *t != tag);
        self.m.type_syntax.push((tag, template.to_string()));
    }
}
