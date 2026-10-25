//! Construction of HIR modules.
//!
//! Expressions are built bottom-up and typed as they are built, so a frontend can ask for the type of
//! anything it has made. A type that is not known yet is an inference variable
//! ([`HirBuilder::fresh_type`], the elements of `[]`, the type arguments of a generic call), fixed later
//! by unification. A node whose rule needs a type that is still open (a field of an object of unknown
//! type) is typed again by `finish`. A type error is recorded as a diagnostic at the expression it is
//! about and the node gets `never`, which fits everywhere, so one mistake is reported once.

use std::collections::{HashMap, HashSet};

use a0_diag::codes as code;
use a0_diag::{Diagnostic, Diagnostics, Span};
use a0_sema::Scopes;

use crate::check::check;
use crate::model::*;

/// A problem in a program as the builder reports it: a code, a message, what is wrong at the node
/// itself, related locations, explanations and a likely misspelling.
pub(crate) struct Problem {
    pub code: &'static str,
    pub message: String,
    pub label: String,
    pub labels: Vec<(Option<Span>, String)>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
    /// A declared name close to a misspelled one: what it is ("a field"), the name, and the
    /// misspelled name.
    pub similar: Option<(String, String, String)>,
}

impl Problem {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Box<Problem> {
        Box::new(Problem {
            code,
            message: message.into(),
            label: String::new(),
            labels: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
            similar: None,
        })
    }

    /// What is wrong at the node itself.
    pub(crate) fn label(mut self: Box<Self>, text: impl Into<String>) -> Box<Self> {
        self.label = text.into();
        self
    }

    /// A related location (nothing when its span is unknown).
    pub(crate) fn at(
        mut self: Box<Self>,
        span: Option<Span>,
        text: impl Into<String>,
    ) -> Box<Self> {
        if span.is_some() {
            self.labels.push((span, text.into()));
        }
        self
    }

    pub(crate) fn note(mut self: Box<Self>, text: impl Into<String>) -> Box<Self> {
        self.notes.push(text.into());
        self
    }

    pub(crate) fn help(mut self: Box<Self>, text: impl Into<String>) -> Box<Self> {
        self.helps.push(text.into());
        self
    }

    /// Suggests `name`, a declared `what` ("a field") close to `misspelled`, if there is one.
    pub(crate) fn similar(
        mut self: Box<Self>,
        what: &str,
        misspelled: &str,
        name: Option<String>,
    ) -> Box<Self> {
        self.similar = name.map(|n| (what.to_string(), n, misspelled.to_string()));
        self
    }

    /// The diagnostic, at `span`; `name` is where the misspelled name is, for a suggested fix (when
    /// it is not given, `span` if it is exactly as long as the name).
    pub(crate) fn diagnostic(&self, span: Option<Span>, name: Option<Span>) -> Diagnostic {
        let mut d = Diagnostic::error(self.code, &self.message)
            .with_span_opt(span)
            .with_primary_label(&self.label);
        for (s, text) in &self.labels {
            if let Some(s) = s {
                d = d.with_label(*s, text);
            }
        }
        for n in &self.notes {
            d = d.with_note(n);
        }
        for h in &self.helps {
            d = d.with_help(h);
        }
        if let Some((what, n, misspelled)) = &self.similar {
            let message = format!("{what} with a similar name exists");
            let name = name.or(span.filter(|s| s.len() as usize == misspelled.len()));
            d = match name {
                Some(at) => d.with_fix(&message, at, n),
                None => d.with_help(&format!("{message}: `{n}`")),
            };
        }
        d
    }
}

/// `count` things, as in "1 argument" and "2 arguments".
pub(crate) fn plural(count: usize, thing: &str) -> String {
    if count == 1 {
        format!("1 {thing}")
    } else {
        format!("{count} {thing}s")
    }
}

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
    "interface",
    "impl",
    "for",
    "throw",
    "try",
    "catch",
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
    pub(crate) diags: Diagnostics,
    pub(crate) span: Option<Span>,
    /// Where the name in the next node built by name is, for suggested fixes.
    name_span: Option<Span>,
    /// Where the names of nodes built by name are, for problems found when they are typed again.
    name_spans: HashMap<ExprId, Span>,
    /// Nodes whose typing rule needed a type that was still open, with their hints.
    deferred: Vec<(ExprId, TypeId)>,
    /// The warnings the language turned on.
    warnings: Vec<Warning>,
    /// The module declarations go into, and whether they are public (`modules.rs`).
    pub(crate) module: ModuleId,
    pub(crate) public: bool,
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
            name_span: None,
            name_spans: HashMap::new(),
            deferred: Vec::new(),
            warnings: Vec::new(),
            module: ModuleId(0),
            public: true,
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

    /// Where the name is in what is built next by name (a name, a call, a field, a variant or a
    /// type), when it is not the whole current span: misspelled names then get a fix that editors
    /// can apply. It applies to one node.
    pub fn set_name_span(&mut self, span: Option<Span>) {
        self.name_span = span;
    }

    /// The span of `name` in what is being built: the one the frontend gave, or the current span
    /// if it is exactly as long as the name.
    pub(crate) fn take_name_span(&mut self, name: &str) -> Option<Span> {
        self.name_span
            .take()
            .or(self.span.filter(|s| s.len() as usize == name.len()))
    }

    /// Reports a problem at the current span.
    pub(crate) fn problem(&mut self, p: &Problem) {
        let name = self.name_span.take();
        self.diags.push(p.diagnostic(self.span, name));
    }

    pub(crate) fn problem_at(&mut self, span: Option<Span>, p: &Problem) {
        self.diags.push(p.diagnostic(span, None));
    }

    /// Reports a misuse of the builder, which is a bug in the language rather than in the program.
    pub(crate) fn misuse(&mut self, message: &str) {
        self.problem(&Problem::new("hir.builder", message));
    }

    /// Turns a warning on. Warnings are found when a module without errors is finished, and kept in
    /// it ([`HirModule::warnings`]).
    pub fn enable_warning(&mut self, warning: Warning) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    /// What diagnostics call `term`, in the language's words.
    pub fn set_term(&mut self, term: Term, text: &str) {
        self.m.terms.retain(|(t, _)| *t != term);
        self.m.terms.push((term, text.to_string()));
    }

    /// Where the `index`th parameter of `body` is declared, so that messages about an argument can
    /// point at it.
    pub fn set_param_span(&mut self, body: BodyId, index: u32, span: Span) {
        if let Some(&l) = self.m.bodies[body.index()].params.get(index as usize) {
            self.m.locals[l.index()].span = Some(span);
        }
    }

    /// Finishes inference, checks the module and returns it, or every problem found.
    pub fn finish(self) -> Result<HirModule, Diagnostics> {
        let (m, diags) = self.finish_keeping();
        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(m)
        }
    }

    /// Like [`HirBuilder::finish`], but returns the module even when it has errors, for tools that
    /// show what is known about a program being edited (types on hover, definitions). A module
    /// with errors must not be lowered.
    pub fn finish_keeping(mut self) -> (HirModule, Diagnostics) {
        self.solve_deferred();
        self.report_unresolved();
        self.resolve_all();
        self.check_impls();
        self.check_bounds();
        self.build_vtables();
        self.report_value_nullables();
        let warnings = self.find_warnings();
        let mut diags = self.diags;
        diags.extend(check(&mut self.m));
        // Each combination of type arguments the program uses must make valid types.
        if !diags.has_errors()
            && let Err(d) = crate::mono::monomorphize(&self.m)
        {
            diags.extend(d);
        }
        // As rustc does, warnings wait until the program has no errors: an error often makes code
        // look unused or unreachable.
        if !diags.has_errors() {
            self.m.warnings = warnings;
        }
        (self.m, diags)
    }

    fn find_warnings(&self) -> Diagnostics {
        let mut out = Diagnostics::new();
        if self.warnings.contains(&Warning::UnusedVariable) {
            self.unused_variables(&mut out);
        }
        if self.warnings.contains(&Warning::UnreachableCode) {
            for body in &self.m.bodies {
                if let Some(root) = body.root {
                    self.unreachable(root, &mut out);
                }
            }
        }
        out
    }

    /// Locals whose value nothing reads; parameters and names starting with `_` are left alone.
    fn unused_variables(&self, out: &mut Diagnostics) {
        let m = &self.m;
        let mut read = vec![false; m.locals.len()];
        for e in &m.exprs {
            if let ExprKind::Local(l) = e.kind {
                read[l.index()] = true;
            }
        }
        let params: HashSet<LocalId> = m.bodies.iter().flat_map(|b| b.params.clone()).collect();
        for (i, local) in m.locals.iter().enumerate() {
            let name = &local.source_name;
            if read[i] || params.contains(&LocalId(i as u32)) || name.starts_with('_') {
                continue;
            }
            let Some(span) = local.span else {
                continue;
            };
            let message = format!("unused {}: `{name}`", m.term(Term::Variable));
            let mut d = Diagnostic::warning(code::UNUSED_VARIABLE, &message).with_span(span);
            let help = "if this is intentional, start its name with an underscore";
            d = if span.len() as usize == name.len() {
                d.with_fix(help, span, &format!("_{name}"))
            } else {
                d.with_help(&format!("{help}: `_{name}`"))
            };
            out.push(d);
        }
    }

    /// Code after something that never finishes, once per block and not inside code already
    /// reported.
    fn unreachable(&self, e: ExprId, out: &mut Diagnostics) {
        let m = &self.m;
        if let ExprKind::Block(items, tail) = &m.exprs[e.index()].kind {
            let rest: Vec<ExprId> = items.iter().copied().chain(*tail).collect();
            for (i, &item) in rest.iter().enumerate() {
                self.unreachable(item, out);
                let Some(&next) = rest.get(i + 1) else {
                    break;
                };
                if m.exprs[item.index()].ty != TypeId::NEVER {
                    continue;
                }
                if let Some(span) = m.exprs[next.index()].span {
                    let d = Diagnostic::warning(code::UNREACHABLE, "unreachable code")
                        .with_span(span)
                        .with_primary_label("unreachable")
                        .with_label_opt(
                            m.exprs[item.index()].span,
                            "any code following this expression is unreachable",
                        );
                    out.push(d);
                }
                return;
            }
            return;
        }
        visit_children(m, e, &mut |c| self.unreachable(c, out));
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
                    Err(TypeErr::At(at, p)) => {
                        let span = self.m.exprs[at.unwrap_or(e).index()].span;
                        let name = self.name_spans.get(&e).copied();
                        self.diags.push(p.diagnostic(span, name));
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
                self.problem_at(
                    span,
                    &Problem::new(
                        code::CANNOT_INFER,
                        "the type of this value must be known here",
                    )
                    .label("type must be known at this point")
                    .help("add a type annotation"),
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
                let message = match self.m.kind(t) {
                    TypeKind::Var(_) => "cannot infer the type of this value".to_string(),
                    _ => format!("cannot infer the type {}", self.m.quote_type(t)),
                };
                let p = Problem::new(code::CANNOT_INFER, message).help("add a type annotation");
                self.problem_at(self.m.exprs[i].span, &p);
            }
        }
        for i in 0..self.m.locals.len() {
            let t = self.m.locals[i].ty;
            if let Some(v) = self.m.first_var(t)
                && reported.insert(v)
            {
                let message = format!(
                    "cannot infer the type of `{}`",
                    self.m.locals[i].source_name
                );
                let p = Problem::new(code::CANNOT_INFER, message).help("add a type annotation");
                self.problem_at(self.m.locals[i].span, &p);
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
                let message = format!(
                    "only reference types can be nullable, not {}",
                    self.m.quote_type(inner)
                );
                self.problem_at(
                    self.m.exprs[i].span,
                    &Problem::new(code::VALUE_NOT_NULLABLE, message),
                );
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
                ExprKind::MethodCall(o, name, args) => {
                    let ot = m.resolve(m.expr_type(o));
                    match crate::methods::resolve_method(m, ot, &name) {
                        crate::methods::Resolved::Method(f, targs) => {
                            let targs = targs.into_iter().map(|t| m.resolve(t)).collect();
                            let mut all = vec![o];
                            all.extend(args);
                            ExprKind::Call(f, targs, all)
                        }
                        crate::methods::Resolved::Interface(i, k) => {
                            ExprKind::InterfaceCall(o, i, k, args)
                        }
                        crate::methods::Resolved::Field(fi) => {
                            // The closure in the field, called.
                            let field = ExprId(m.exprs.len() as u32);
                            let ft = match m.kind(ot) {
                                TypeKind::Struct(s, targs) => {
                                    let (s, targs) = (*s, targs.clone());
                                    let t = m.structs[s.index()].fields[fi as usize].1;
                                    let t = m.subst(t, Item::Struct(s), &targs);
                                    m.resolve(t)
                                }
                                _ => continue,
                            };
                            let span = m.exprs[i].span;
                            m.exprs.push(ExprData {
                                kind: ExprKind::Field(o, fi),
                                ty: ft,
                                span,
                            });
                            ExprKind::CallValue(field, args)
                        }
                        crate::methods::Resolved::Bound | crate::methods::Resolved::None => {
                            continue;
                        }
                    }
                }
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
            let k = self.m.quote_type(key);
            self.problem(&Problem::new(
                code::BAD_MAP_KEY,
                format!("map keys must be integers, booleans or strings, not {k}"),
            ));
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
                let name = self.m.quote_type(t);
                self.problem(&Problem::new(
                    code::VALUE_NOT_NULLABLE,
                    format!("only reference types can be nullable, not {name}"),
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
            self.misuse("no such type parameter");
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
            let p = Problem::new(
                code::TYPE_ARGUMENT_COUNT,
                format!(
                    "`{name}` takes {} but {given} {} given",
                    plural(want, "type argument"),
                    if given == 1 { "was" } else { "were" }
                ),
            )
            .label(format!("expected {}", plural(want, "type argument")))
            .at(self.m.item_span(item), format!("`{name}` declared here"));
            self.problem(&p);
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
            self.misuse(&format!("'{name}' is not a valid name"));
        }
    }

    /// Registers a type in the current module; `name` is its name there.
    pub(crate) fn add_type_name(&mut self, name: &str, item: Item) {
        self.item_name(name);
        let key = self.qualified(name);
        if let Some(first) = self.m.type_names.insert(key, item) {
            self.declared_twice(&format!("the type `{name}`"), self.m.item_span(first));
        }
        self.m.item_home.insert(item, (self.module, self.public));
        self.m.items.push(item);
    }

    /// Registers a function, global or C function in the current module.
    fn add_value_name(&mut self, name: &str, item: Item) {
        self.item_name(name);
        let key = self.qualified(name);
        if let Some(first) = self.m.value_names.insert(key, item) {
            self.declared_twice(&format!("`{name}`"), self.m.item_span(first));
        }
        self.m.item_home.insert(item, (self.module, self.public));
        self.m.items.push(item);
    }

    /// Reports a second declaration of `what` at the current span, `first` being the first.
    pub(crate) fn declared_twice(&mut self, what: &str, first: Option<Span>) {
        let p = Problem::new(code::DUPLICATE, format!("{what} is declared twice"))
            .label("declared again here")
            .at(first, "first declared here");
        self.problem(&p);
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
            name: self.qualified(name),
            type_params,
            fields: Vec::new(),
            field_spans: Vec::new(),
            span: self.span,
        });
        self.add_type_name(name, Item::Struct(s));
        s
    }

    pub fn add_field(&mut self, s: StructId, name: &str, ty: TypeId) -> u32 {
        self.item_name(name);
        let def = &self.m.structs[s.index()];
        if let Some(i) = def.fields.iter().position(|(n, _)| n == name) {
            let (what, first) = (
                format!(
                    "the {} `{name}` of `{}`",
                    self.m.term(Term::Field),
                    def.name
                ),
                def.field_spans[i],
            );
            self.declared_twice(&what, first);
        }
        let span = self.span;
        let def = &mut self.m.structs[s.index()];
        def.fields.push((name.to_string(), ty));
        def.field_spans.push(span);
        def.fields.len() as u32 - 1
    }

    pub fn declare_enum(&mut self, name: &str) -> EnumId {
        self.declare_generic_enum(name, &[])
    }

    pub fn declare_generic_enum(&mut self, name: &str, type_params: &[&str]) -> EnumId {
        let e = EnumId(self.m.enums.len() as u32);
        let type_params = self.type_param_names(type_params);
        self.m.enums.push(EnumDef {
            name: self.qualified(name),
            type_params,
            variants: Vec::new(),
            span: self.span,
        });
        self.add_type_name(name, Item::Enum(e));
        e
    }

    pub fn add_variant(&mut self, e: EnumId, name: &str, fields: &[TypeId]) -> u32 {
        self.item_name(name);
        if let Some(v) = self.m.variant_index(e, name) {
            let what = format!(
                "the {} `{name}` of `{}`",
                self.m.term(Term::Variant),
                self.m.enum_name(e)
            );
            let first = self.m.enums[e.index()].variants[v as usize].span;
            self.declared_twice(&what, first);
        }
        let span = self.span;
        let def = &mut self.m.enums[e.index()];
        def.variants.push(VariantDef {
            name: name.to_string(),
            fields: fields.to_vec(),
            span,
        });
        def.variants.len() as u32 - 1
    }

    /// Declares a global. Globals start at the zero value of their type, so a reference type must
    /// be nullable.
    pub fn declare_global(&mut self, name: &str, ty: TypeId) -> GlobalId {
        if self.m.is_reference(ty) && !matches!(self.m.kind(ty), TypeKind::Nullable(_)) {
            let nullable = self.m.intern(TypeKind::Nullable(ty));
            let p = Problem::new(
                code::NULL_NOT_ALLOWED,
                format!(
                    "the global `{name}` starts as {}, so its type must be nullable",
                    self.m.term(Term::Null)
                ),
            )
            .help(format!("make its type {}", self.m.quote_type(nullable)));
            self.problem(&p);
        }
        let g = GlobalId(self.m.globals.len() as u32);
        self.m.globals.push(GlobalDef {
            name: self.qualified(name),
            ty,
            span: self.span,
        });
        self.add_value_name(name, Item::Global(g));
        g
    }

    /// Declares a C function. Its types must be `int`, `float`, `bool` or `unit`.
    pub fn declare_extern(&mut self, name: &str, params: &[TypeId], ret: TypeId) -> ExternId {
        self.declare_extern_abi(name, params, ret, Abi::C)
    }

    /// Declares a function written in C, called through `abi`. A C function takes `int`, `float`,
    /// `bool`, strings and arrays of `int`, `float` or `bool` (or null for `T?`), and returns one of
    /// the first three, `unit` or a string (`string?` if it can return `NULL`); a function of the
    /// runtime takes and returns anything. Modules may declare the same C function, with the same
    /// signature.
    pub fn declare_extern_abi(
        &mut self,
        name: &str,
        params: &[TypeId],
        ret: TypeId,
        abi: Abi,
    ) -> ExternId {
        if abi == Abi::C {
            for &t in params {
                if !self.c_param(t) {
                    let tn = self.m.display_type(t);
                    self.misuse(&format!(
                        "extern fn {name} takes {tn}; C functions take int, float, bool, strings \
                         and arrays of int, float or bool"
                    ));
                }
            }
            let string_result =
                ret == TypeId::STRING || self.nullable_of(ret) == Some(TypeId::STRING);
            if !(string_result
                || matches!(
                    ret,
                    TypeId::INT | TypeId::FLOAT | TypeId::BOOL | TypeId::UNIT
                ))
            {
                let tn = self.m.display_type(ret);
                self.misuse(&format!(
                    "extern fn {name} returns {tn}; C functions return int, float, bool, unit or a string"
                ));
            }
        }
        if let Some(other) = self.m.externs.iter().find(|x| x.name == name)
            && (other.params != params || other.ret != ret || other.abi != abi)
        {
            self.misuse(&format!("extern fn {name} is declared with two signatures"));
        }
        let x = ExternId(self.m.externs.len() as u32);
        self.m.externs.push(ExternDef {
            name: name.to_string(),
            params: params.to_vec(),
            ret,
            abi,
            span: self.span,
        });
        self.add_value_name(name, Item::Extern(x));
        x
    }

    fn nullable_of(&self, t: TypeId) -> Option<TypeId> {
        match self.m.kind(t) {
            TypeKind::Nullable(inner) => Some(*inner),
            _ => None,
        }
    }

    /// Whether a C function can take a `t`.
    fn c_param(&self, t: TypeId) -> bool {
        if matches!(t, TypeId::INT | TypeId::FLOAT | TypeId::BOOL) {
            return true;
        }
        let t = self.nullable_of(t).unwrap_or(t);
        match self.m.kind(t) {
            TypeKind::String => true,
            TypeKind::Array(e) => matches!(*e, TypeId::INT | TypeId::FLOAT | TypeId::BOOL),
            _ => false,
        }
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
            name: self.qualified(name),
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
            self.misuse("a function's signature can only be set once");
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
            self.misuse("parameter names and types differ in number");
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
            self.misuse("a local cannot have type never");
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
            source_name: name.to_string(),
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
        let want = self.m.quote_type(ret);
        // Where the result type comes from: the function's declaration (closures have none).
        let data = &self.m.bodies[body.index()];
        let (decl, fname) = match data.parent {
            None => {
                let f = &self.m.funcs[data.func.index()];
                (f.span, format!("`{}`", f.name))
            }
            Some(_) => (None, String::new()),
        };
        if ret != TypeId::UNIT && !self.m.coerce(rt, ret) {
            let mut value = root;
            while let ExprKind::Block(_, Some(tail)) = self.m.exprs[value.index()].kind {
                value = tail;
            }
            let span = self.m.exprs[value.index()].span;
            let p = if self.m.same(rt, TypeId::UNIT) {
                Problem::new(
                    code::BAD_RESULT,
                    format!(
                        "this {} can end without returning a value of type {want}",
                        self.m.term(Term::Function)
                    ),
                )
                .label(format!("expected {want}"))
            } else {
                mismatch(&self.m, ret, rt)
            };
            let p = p.at(decl, format!("{fname} returns {want}"));
            self.problem_at(span, &p);
        }
        let mut returns = Vec::new();
        collect_returns(&self.m, root, &mut returns);
        for v in returns {
            let vt = self.m.expr_type(v);
            if self.m.coerce(vt, ret) {
                continue;
            }
            let p = if ret == TypeId::UNIT {
                Problem::new(code::MISMATCHED_TYPES, "mismatched types")
                    .label(format!(
                        "expected no value, found {}",
                        self.m.quote_type(vt)
                    ))
                    .at(decl, format!("{fname} returns no value"))
            } else {
                mismatch(&self.m, ret, vt).at(decl, format!("{fname} returns {want}"))
            };
            let span = self.m.exprs[v.index()].span;
            self.problem_at(span, &p);
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

    pub(crate) fn push(&mut self, kind: ExprKind, hint: TypeId) -> ExprId {
        let e = ExprId(self.m.exprs.len() as u32);
        let ty = match infer(&mut self.m, &kind, hint) {
            Ok(t) => t,
            Err(TypeErr::Open(_)) => {
                self.deferred.push((e, hint));
                self.m.fresh_var()
            }
            Err(TypeErr::At(at, p)) => {
                let span = match at {
                    Some(x) => self.m.exprs[x.index()].span,
                    None => self.span,
                };
                self.diags.push(p.diagnostic(span, self.name_span));
                TypeId::ERROR
            }
        };
        if let Some(name) = self.name_span.take() {
            self.name_spans.insert(e, name);
        }
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

    /// `throw value`: raises an exception that holds `value` (of any type, as an `any`); it unwinds
    /// to the innermost `try` around it, in this function or a caller, or ends the program with
    /// `panic: uncaught exception` and the stack trace where it was thrown.
    pub fn throw(&mut self, value: ExprId) -> ExprId {
        self.push(ExprKind::Throw(value), TypeId::NEVER)
    }

    /// `try { body } catch local { handler }`: the value of `body`, or, if `body` raises an exception,
    /// of `handler`, with the exception's value in `local` (an `any` local, declared for the
    /// handler).
    pub fn try_catch(&mut self, body: ExprId, local: LocalId, handler: ExprId) -> ExprId {
        self.push(ExprKind::Try(body, local, handler), TypeId::NEVER)
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
    /// A problem, about the given sub-expression (or the node itself).
    At(Option<ExprId>, Box<Problem>),
    /// The rule needs the type of this sub-expression, which is not inferred yet.
    Open(ExprId),
}

type Typed = Result<TypeId, TypeErr>;

fn fail<T>(p: Box<Problem>) -> Result<T, TypeErr> {
    Err(TypeErr::At(None, p))
}

pub(crate) fn mismatch(m: &HirModule, expected: TypeId, found: TypeId) -> Box<Problem> {
    Problem::new(code::MISMATCHED_TYPES, "mismatched types").label(format!(
        "expected {}, found {}",
        m.quote_type(expected),
        m.quote_type(found)
    ))
}

/// Requires `e` to fit `want`.
fn expect(m: &mut HirModule, e: ExprId, want: TypeId) -> Result<(), TypeErr> {
    let found = m.expr_type(e);
    let mut p = mismatch(m, want, found);
    // A value that may be null where one that cannot is needed: say how to get at it.
    if let TypeKind::Nullable(inner) = m.kind(found)
        && m.same(*inner, want)
        && !matches!(m.expr(e).kind, ExprKind::Null)
    {
        p = p.help(format!(
            "check that it is not {} with {}, or unwrap it with {}",
            m.term(Term::Null),
            m.term(Term::Narrow),
            m.term(Term::Unwrap)
        ));
    }
    if m.coerce(found, want) {
        Ok(())
    } else {
        Err(TypeErr::At(Some(e), p))
    }
}

/// Adds a related location to a problem: where the expected type came from, for example.
fn because<T>(
    r: Result<T, TypeErr>,
    span: Option<Span>,
    text: impl FnOnce() -> String,
) -> Result<T, TypeErr> {
    r.map_err(|err| match err {
        TypeErr::At(at, p) => TypeErr::At(at, p.at(span, text())),
        open => open,
    })
}

/// What is being called, for messages about its arguments.
struct Callee {
    /// "this function", "`len`"
    what: String,
    /// Where it is declared, and its parameters.
    decl: Option<Span>,
    params: Vec<Option<Span>>,
}

impl Callee {
    fn unknown(what: String) -> Callee {
        Callee {
            what,
            decl: None,
            params: Vec::new(),
        }
    }

    /// A function's declaration and its parameters' (parameters declared at the function's own
    /// span are not pointed at separately).
    fn function(m: &HirModule, f: FuncId) -> Callee {
        let decl = m.funcs[f.index()].span;
        let params = m
            .body_params(m.func_body(f))
            .iter()
            .map(|&p| m.local_span(p).filter(|s| Some(*s) != decl))
            .collect();
        Callee {
            what: format!("this {}", m.term(Term::Function)),
            decl,
            params,
        }
    }
}

fn args_match(
    m: &mut HirModule,
    params: &[TypeId],
    args: &[ExprId],
    callee: &Callee,
) -> Result<(), TypeErr> {
    if params.len() != args.len() {
        let p = Problem::new(
            code::ARGUMENT_COUNT,
            format!(
                "{} takes {} but {} supplied",
                callee.what,
                plural(params.len(), "argument"),
                match args.len() {
                    1 => "1 argument was".to_string(),
                    n => format!("{n} arguments were"),
                }
            ),
        )
        .label(format!("expected {}", plural(params.len(), "argument")))
        .at(callee.decl, "declared here");
        return fail(p);
    }
    for (i, (&p, &a)) in params.iter().zip(args).enumerate() {
        let param = callee.params.get(i).copied().flatten();
        because(expect(m, a, p), param, || {
            "parameter declared here".to_string()
        })?;
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
        let name = m.func_name(f);
        return fail(
            Problem::new(
                code::TYPE_ARGUMENT_COUNT,
                format!(
                    "`{name}` takes {} but {} {} given",
                    plural(want, "type argument"),
                    targs.len(),
                    if targs.len() == 1 { "was" } else { "were" }
                ),
            )
            .label(format!("expected {}", plural(want, "type argument")))
            .at(m.funcs[f.index()].span, format!("`{name}` declared here")),
        );
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

fn open_or<T>(m: &HirModule, e: ExprId, p: impl FnOnce() -> Box<Problem>) -> Result<T, TypeErr> {
    if matches!(m.kind(m.expr_type(e)), TypeKind::Var(_)) {
        Err(TypeErr::Open(e))
    } else {
        Err(TypeErr::At(Some(e), p()))
    }
}

/// A note for a problem about a value of type parameter `t`, which may be any type.
fn param_note(m: &HirModule, t: TypeId, p: Box<Problem>) -> Box<Problem> {
    match m.kind(t) {
        TypeKind::Param(..) => p.note(format!(
            "{} is a type parameter, so it may be any type",
            m.quote_type(t)
        )),
        _ => p,
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
                return fail(Problem::new(
                    "hir.builder",
                    format!(
                        "`null` needs a nullable type, not `{}`",
                        m.display_type(hint)
                    ),
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
            let r = expect(m, *init, m.local_type(*l));
            because(r, m.local_span(*l), || "expected due to this".to_string())?;
            TypeId::UNIT
        }
        ExprKind::SetGlobal(g, value) => {
            let r = expect(m, *value, m.global_type(*g));
            because(r, m.globals[g.index()].span, || {
                "expected due to this".to_string()
            })?;
            TypeId::UNIT
        }
        ExprKind::SetField(object, field, value) => {
            let ft = field_type(m, *object, *field)?;
            let decl = field_span(m, *object, *field);
            because(expect(m, *value, ft), decl, || field_declared(m))?;
            TypeId::UNIT
        }
        ExprKind::SetFieldName(object, name, value) => {
            let i = named_field(m, *object, name)?;
            let ft = field_type(m, *object, i)?;
            let decl = field_span(m, *object, i);
            because(expect(m, *value, ft), decl, || field_declared(m))?;
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
                return open_or(m, *a, || bad_operand(m, "-", ty(m, *a)));
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
            args_match(m, &params, args, &Callee::function(m, *f))?;
            ret
        }
        ExprKind::CallExtern(x, args) => {
            let (params, ret) = (
                m.externs[x.index()].params.clone(),
                m.externs[x.index()].ret,
            );
            let callee = Callee {
                what: format!("this {}", m.term(Term::Function)),
                decl: m.externs[x.index()].span,
                params: Vec::new(),
            };
            args_match(m, &params, args, &callee)?;
            ret
        }
        ExprKind::CallValue(callee, args) => {
            let ct = ty(m, *callee);
            match m.kind(ct).clone() {
                TypeKind::Fn(params, ret) => {
                    let what = format!("this {}", m.term(Term::Closure));
                    args_match(m, &params, args, &Callee::unknown(what))?;
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
                    let function = m.term(Term::Function);
                    let mut p = Problem::new(
                        code::NOT_CALLABLE,
                        format!("expected a {function}, found {}", m.quote_type(ct)),
                    )
                    .label(format!("this is not a {function}"));
                    if let ExprKind::Local(l) = m.expr(*callee).kind {
                        let name = m.local_source_name(l);
                        let text = format!("`{name}` has type {}", m.quote_type(ct));
                        p = p.at(m.local_span(l), text);
                    }
                    return Err(TypeErr::At(Some(*callee), p));
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
                let p = Problem::new(
                    code::MISSING_FIELDS,
                    format!(
                        "`{}` has {} but {} {} given",
                        m.struct_name(*s),
                        plural(def_fields.len(), &m.term(Term::Field)),
                        fields.len(),
                        if fields.len() == 1 { "is" } else { "are" }
                    ),
                )
                .at(m.structs[s.index()].span, "declared here");
                return fail(p);
            }
            for (i, ((_, ft), &v)) in def_fields.iter().zip(fields).enumerate() {
                let ft = m.subst(*ft, Item::Struct(*s), &args);
                let decl = m.structs[s.index()].field_spans[i];
                because(expect(m, v, ft), decl, || field_declared(m))?;
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
                return fail(Problem::new(
                    "hir.builder",
                    format!("enum `{}` has no variant {v}", m.enum_name(*e)),
                ));
            };
            let (fields, decl): (Vec<TypeId>, Option<Span>) = (var.fields.clone(), var.span);
            let fields: Vec<TypeId> = fields
                .into_iter()
                .map(|f| m.subst(f, Item::Enum(*e), &targs))
                .collect();
            let callee = Callee {
                what: format!("this {}", m.term(Term::Variant)),
                decl,
                params: Vec::new(),
            };
            args_match(m, &fields, args, &callee)?;
            hint
        }
        ExprKind::Match(scrutinee, arms) => match_type(m, *scrutinee, arms)?,
        ExprKind::Array(elems) => {
            let elem = match m.kind(hint) {
                TypeKind::Array(e) => *e,
                _ => {
                    return fail(Problem::new(
                        "hir.builder",
                        "an array literal needs an array type",
                    ));
                }
            };
            for &e in elems {
                expect(m, e, elem)?;
            }
            hint
        }
        ExprKind::NewArray => {
            if !matches!(m.kind(hint), TypeKind::Array(_)) {
                return fail(Problem::new(
                    "hir.builder",
                    format!(
                        "new array needs an array type, not `{}`",
                        m.display_type(hint)
                    ),
                ));
            }
            hint
        }
        ExprKind::NewMap => {
            if !matches!(m.kind(hint), TypeKind::Map(..)) {
                return fail(Problem::new(
                    "hir.builder",
                    format!("new map needs a map type, not `{}`", m.display_type(hint)),
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
                return fail(Problem::new(
                    code::BAD_CONVERSION,
                    format!(
                        "cannot convert {} to {}",
                        m.quote_type(from),
                        m.quote_type(hint)
                    ),
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
                    Problem::new(
                        code::NOT_NULLABLE,
                        format!("{} is not nullable", m.quote_type(ty(m, *e))),
                    )
                    .label(format!("cannot be {}", m.term(Term::Null)))
                    .help(format!("remove the {}", m.term(Term::Unwrap)))
                });
            }
        },
        ExprKind::Is(e, t) => {
            expect(m, *e, TypeId::ANY)?;
            if !testable(m, *t) {
                return fail(Problem::new(
                    code::BAD_TYPE_TEST,
                    format!(
                        "cannot test whether a value of type {} holds {}",
                        m.quote_type(TypeId::ANY),
                        m.quote_type(*t)
                    ),
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
                        let p =
                            mismatch(m, lt, inner).at(m.local_span(*local), "expected due to this");
                        return Err(TypeErr::At(Some(*value), p));
                    }
                }
                TypeKind::Never | TypeKind::Error => {}
                _ => {
                    return open_or(m, *value, || {
                        Problem::new(
                            code::NOT_NULLABLE,
                            format!(
                                "{} needs a nullable value, found {}",
                                m.term(Term::Narrow),
                                m.quote_type(vt)
                            ),
                        )
                        .label(format!("this is never {}", m.term(Term::Null)))
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
        ExprKind::MethodCall(receiver, name, args) => method_call_type(m, *receiver, name, args)?,
        ExprKind::InterfaceCall(receiver, i, k, args) => {
            let want = m.intern(TypeKind::Interface(*i));
            expect(m, *receiver, want)?;
            interface_call_type(m, *i, *k, args)?
        }
        // Lowering boxes the value, whose type `finish` requires to be known like every other.
        ExprKind::Throw(_) => TypeId::NEVER,
        ExprKind::Try(body, local, handler) => {
            let lt = m.local_type(*local);
            if !m.unify(lt, TypeId::ANY) {
                let p = mismatch(m, TypeId::ANY, lt).at(m.local_span(*local), "declared here");
                return Err(TypeErr::At(Some(*handler), p));
            }
            m.join(ty(m, *body), ty(m, *handler))
                .unwrap_or(TypeId::UNIT)
        }
        ExprKind::Block(items, tail) => match tail {
            Some(t) => ty(m, *t),
            None if items.iter().any(|&i| ty(m, i) == TypeId::NEVER) => TypeId::NEVER,
            None => TypeId::UNIT,
        },
    })
}

/// The type of `receiver.name(args)` (see `methods.rs`).
fn method_call_type(m: &mut HirModule, receiver: ExprId, name: &str, args: &[ExprId]) -> Typed {
    use crate::methods::{Resolved, bound_method, resolve_method};
    let rt = m.shallow(m.expr_type(receiver));
    match m.kind(rt) {
        TypeKind::Var(_) => return Err(TypeErr::Open(receiver)),
        TypeKind::Never => return Ok(TypeId::NEVER),
        TypeKind::Error => return Ok(TypeId::ERROR),
        _ => {}
    }
    match resolve_method(m, rt, name) {
        Resolved::Method(f, mut targs) => {
            let own = m.funcs[f.index()].type_params.len() - targs.len();
            if own > 0 {
                // Fresh type arguments here would not survive to `finish`; `method_call` gives them
                // when the receiver's type is known.
                return fail(Problem::new(
                    code::CANNOT_INFER,
                    format!("the type of the value `{name}` is called on must be known here"),
                ));
            }
            targs.truncate(m.funcs[f.index()].type_params.len());
            let (params, ret) = signature(m, f, &targs)?;
            expect(m, receiver, params[0])?;
            let mut callee = Callee::function(m, f);
            if !callee.params.is_empty() {
                callee.params.remove(0);
            }
            args_match(m, &params[1..], args, &callee)?;
            Ok(ret)
        }
        Resolved::Interface(i, k) => interface_call_type(m, i, k, args),
        Resolved::Field(fi) => {
            let ft = field_type(m, receiver, fi)?;
            match m.kind(ft).clone() {
                TypeKind::Fn(params, ret) => {
                    let what = format!("this {}", m.term(Term::Closure));
                    args_match(m, &params, args, &Callee::unknown(what))?;
                    Ok(ret)
                }
                _ => fail(HirBuilder::unknown_method(m, rt, name)),
            }
        }
        Resolved::Bound => {
            let Some((params, ret, decl)) = bound_method(m, rt, name) else {
                return fail(HirBuilder::unknown_method(m, rt, name));
            };
            let callee = Callee {
                what: format!("the method `{name}`"),
                decl,
                params: Vec::new(),
            };
            args_match(m, &params, args, &callee)?;
            Ok(ret)
        }
        Resolved::None => {
            let p = HirBuilder::unknown_method(m, rt, name);
            fail(param_note(m, rt, p))
        }
    }
}

/// The type of a call of the `k`th method of interface `i` on an interface value.
fn interface_call_type(m: &mut HirModule, i: InterfaceId, k: u32, args: &[ExprId]) -> Typed {
    let sig = m.interfaces[i.index()].methods[k as usize].clone();
    if crate::methods::mentions_self(m, i, &sig) {
        return fail(
            Problem::new(
                code::NOT_DYNAMIC,
                format!(
                    "the method `{}` of `{}` cannot be called on a value of the interface",
                    sig.name,
                    m.interfaces[i.index()].name
                ),
            )
            .label("the type of the value is not known here")
            .at(sig.span, "it uses `Self`, the type of the value")
            .help("call it through a type parameter bounded by the interface instead"),
        );
    }
    let callee = Callee {
        what: format!("the method `{}`", sig.name),
        decl: sig.span,
        params: Vec::new(),
    };
    args_match(m, &sig.params, args, &callee)?;
    Ok(sig.ret)
}

fn named_field(m: &HirModule, object: ExprId, name: &str) -> Result<u32, TypeErr> {
    let ot = m.expr_type(object);
    match m.kind(ot) {
        TypeKind::Struct(s, _) => match m.struct_field_index(*s, name) {
            Some(i) => Ok(i),
            None => {
                let fields: Vec<&str> = m.structs[s.index()]
                    .fields
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect();
                let field = m.term(Term::Field);
                let mut p = Problem::new(
                    code::UNKNOWN_FIELD,
                    format!("no {field} `{name}` on type {}", m.quote_type(ot)),
                )
                .label(format!("unknown {field}"))
                .similar(
                    &format!("a {field}"),
                    name,
                    a0_diag::similar_name(name, &fields),
                );
                if !fields.is_empty() {
                    let list: Vec<String> = fields.iter().map(|f| format!("`{f}`")).collect();
                    p = p.note(format!(
                        "the {field}s of {} are {}",
                        m.quote_type(ot),
                        list.join(", ")
                    ));
                }
                Err(TypeErr::At(None, p))
            }
        },
        TypeKind::Never | TypeKind::Error => Ok(u32::MAX),
        _ => open_or(m, object, || no_fields(m, ot)),
    }
}

fn no_fields(m: &HirModule, t: TypeId) -> Box<Problem> {
    match m.kind(t) {
        TypeKind::Nullable(_) => maybe_null(m, t),
        _ => Problem::new(
            code::NO_FIELDS,
            format!(
                "{} is not a {}, so it has no {}s",
                m.quote_type(t),
                m.term(Term::Struct),
                m.term(Term::Field)
            ),
        ),
    }
}

/// A value of nullable type `t` used as if it could not be null.
fn maybe_null(m: &HirModule, t: TypeId) -> Box<Problem> {
    let null = m.term(Term::Null);
    Problem::new(
        code::MAYBE_NULL,
        format!("this value of type {} may be {null}", m.quote_type(t)),
    )
    .label(format!("may be {null}"))
    .help(format!(
        "check that it is not {null} with {}, or unwrap it with {}",
        m.term(Term::Narrow),
        m.term(Term::Unwrap)
    ))
}

/// Where the `field`th field of the struct `object` has is declared.
fn field_span(m: &HirModule, object: ExprId, field: u32) -> Option<Span> {
    match m.kind(m.expr_type(object)) {
        TypeKind::Struct(s, _) => m.structs[s.index()]
            .field_spans
            .get(field as usize)
            .copied()
            .flatten(),
        _ => None,
    }
}

fn field_declared(m: &HirModule) -> String {
    format!("{} declared here", m.term(Term::Field))
}

fn field_type(m: &mut HirModule, object: ExprId, field: u32) -> Typed {
    let ot = m.expr_type(object);
    match m.kind(ot).clone() {
        TypeKind::Struct(s, args) => match m.structs[s.index()].fields.get(field as usize) {
            Some(&(_, t)) => Ok(m.subst(t, Item::Struct(s), &args)),
            None => fail(Problem::new(
                "hir.builder",
                format!("struct `{}` has no field {field}", m.struct_name(s)),
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
            TypeKind::Nullable(_) => maybe_null(m, t),
            _ => Problem::new(
                code::NOT_INDEXABLE,
                format!("cannot index a value of type {}", m.quote_type(t)),
            ),
        }),
    }
}

fn binary_type(m: &mut HirModule, op: BinOp, a: ExprId, b: ExprId) -> Typed {
    let (ta, tb) = (m.expr_type(a), m.expr_type(b));
    let left = |m: &HirModule| format!("expected because this is {}", m.quote_type(ta));
    let a_span = m.expr(a).span;
    if matches!(op, BinOp::Eq | BinOp::Ne) {
        // Either side may be the nullable one: `x == null`.
        let p = mismatch(m, ta, tb).at(a_span, left(m));
        if !(m.coerce(tb, ta) || m.coerce(ta, tb)) {
            return Err(TypeErr::At(Some(b), p));
        }
        return Ok(TypeId::BOOL);
    }
    let p = mismatch(m, ta, tb).at(a_span, left(m));
    if ta != TypeId::NEVER && ta != TypeId::ERROR && !m.coerce(tb, ta) {
        return Err(TypeErr::At(Some(b), p));
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
        return Err(TypeErr::At(Some(operand), bad_operand(m, op.symbol(), t)));
    }
    Ok(if op.is_comparison() {
        TypeId::BOOL
    } else {
        m.shallow(t)
    })
}

fn bad_operand(m: &HirModule, op: &str, t: TypeId) -> Box<Problem> {
    let p = Problem::new(
        code::BAD_OPERAND,
        format!("operator `{op}` cannot be applied to {}", m.quote_type(t)),
    )
    .label(format!("this is {}", m.quote_type(t)));
    param_note(m, t, p)
}

fn match_type(m: &mut HirModule, scrutinee: ExprId, arms: &[ArmId]) -> Typed {
    let st = m.expr_type(scrutinee);
    let (e, eargs) = match m.kind(st).clone() {
        TypeKind::Enum(e, args) => (e, args),
        TypeKind::Never => return Ok(TypeId::NEVER),
        TypeKind::Error => return Ok(TypeId::ERROR),
        _ => {
            return open_or(m, scrutinee, || {
                Problem::new(
                    code::NOT_AN_ENUM,
                    format!(
                        "{} needs a value of an {}, found {}",
                        m.term(Term::Match),
                        m.term(Term::Enum),
                        m.quote_type(st)
                    ),
                )
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
                    return fail(Problem::new(
                        "hir.builder",
                        format!("enum `{}` has no variant {v}", m.enum_name(e)),
                    ));
                };
                let decl = m.enums[e.index()].variants[v as usize].span;
                if binds.len() != fields.len() {
                    let p = Problem::new(
                        code::ARGUMENT_COUNT,
                        format!(
                            "the {} `{}` has {} but the pattern binds {}",
                            m.term(Term::Variant),
                            m.variant_name(e, v),
                            plural(fields.len(), "value"),
                            binds.len()
                        ),
                    )
                    .at(decl, "declared here");
                    return fail(p);
                }
                for (&b, &f) in binds.iter().zip(&fields) {
                    let f = m.subst(f, Item::Enum(e), &eargs);
                    let lt = m.local_type(b);
                    let p = mismatch(m, lt, f).at(decl, "declared here");
                    if !m.unify(lt, f) {
                        return fail(p);
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
        let missing: Vec<u32> = (0..covered.len() as u32)
            .filter(|&i| !covered[i as usize])
            .collect();
        let names: Vec<String> = missing
            .iter()
            .map(|&v| format!("`{}`", m.variant_name(e, v)))
            .collect();
        let list = match names.as_slice() {
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
            [] => String::new(),
        };
        let mut p = Problem::new(
            code::NON_EXHAUSTIVE,
            format!("{} does not cover {list}", m.term(Term::Match)),
        )
        .label(format!("{list} not covered"));
        for &v in &missing {
            p = p.at(m.enums[e.index()].variants[v as usize].span, "not covered");
        }
        let p = p.help("add an arm for each, or one that matches anything");
        return Err(TypeErr::At(Some(scrutinee), p));
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
        let callee = Callee::unknown(format!("`{}`", op.name()));
        return args_match(m, &vec![TypeId::ERROR; arity], args, &callee).map(|_| TypeId::ERROR);
    }
    let t0 = args
        .first()
        .map(|&a| m.expr_type(a))
        .unwrap_or(TypeId::UNIT);
    let bad = |m: &HirModule, what: &str| {
        open_or(m, args[0], || {
            Problem::new(
                code::MISMATCHED_TYPES,
                format!("`{}` {what}, not {}", op.name(), m.quote_type(t0)),
            )
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
        | TypeKind::Enum(..)
        | TypeKind::Interface(_) => true,
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
        || matches!(
            (fk, tk),
            (TypeKind::Struct(..) | TypeKind::Enum(..), TypeKind::Interface(i))
                if crate::methods::implements(m, from, *i)
        )
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
        | Loop(a)
        | Throw(a) => f(*a),
        SetField(a, _, b)
        | SetFieldName(a, _, b)
        | Binary(_, a, b)
        | And(a, b)
        | Or(a, b)
        | Index(a, b)
        | While(a, b)
        | Try(a, _, b) => {
            f(*a);
            f(*b);
        }
        SetIndex(a, b, c) => {
            f(*a);
            f(*b);
            f(*c);
        }
        MethodCall(r, _, args) | InterfaceCall(r, _, _, args) => {
            f(*r);
            args.iter().for_each(|&a| f(a));
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
