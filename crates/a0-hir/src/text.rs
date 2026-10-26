//! Parsing the HIR text format, using A0's own grammar layer (`hir.a0g`) and scopes from `a0-sema`.

use a0_diag::codes as code;
use a0_diag::{Diagnostic, Diagnostics, SourceId};
use a0_sema::{ScopeKind, Scopes};
use a0_syntax::{Grammar, NodeId, SyntaxTree};

use crate::builder::HirBuilder;
use crate::model::*;

const GRAMMAR: &str = include_str!("hir.a0g");

fn grammar() -> Grammar {
    match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("hir.a0g is invalid:\n{}", d.summary()),
    }
}

/// Parses a module from text (`source` is its id in the caller's `SourceMap`, for spans) and checks
/// it.
pub fn parse(text: &str, source: SourceId) -> Result<HirModule, Diagnostics> {
    let g = grammar();
    let (tree, diags) = g.parse(text, source);
    if diags.has_errors() {
        return Err(diags);
    }
    let root = tree.root();
    let name_node = tree.field_named(root, "name").unwrap();
    let mut b = HirBuilder::new(&unquote(tree.text(name_node)));
    let diags = TextParser::new(&tree, &mut b, Some(source), false).module(root);
    if diags.has_errors() {
        return Err(diags);
    }
    b.finish()
}

/// Adds the items of a module in the text format to the builder's current module, with spans in
/// `source` if given. Its own items and the prelude's are known by their names, other modules' by
/// their full names (`` `std.Result` ``). In a `library` (the standard library, `stdlib.rs`), a C
/// function whose name starts with `a0_` is private to the module and everything else is public.
pub(crate) fn include(
    b: &mut HirBuilder,
    text: &str,
    source: Option<SourceId>,
    library: bool,
) -> Result<(), Diagnostics> {
    let g = grammar();
    let (tree, diags) = g.parse(text, source.unwrap_or(SourceId(u32::MAX)));
    if diags.has_errors() {
        return Err(diags);
    }
    let diags = TextParser::new(&tree, b, source, library).module(tree.root());
    if diags.has_errors() {
        return Err(diags);
    }
    Ok(())
}

impl HirBuilder {
    /// Adds the items of a module in the text format (its `module` line is ignored) to the current
    /// module, for code a language keeps as text, such as a runtime written in HIR that uses the
    /// standard library (import its modules first; the text names their items in full,
    /// `` `std.fs.read` ``). `source` is the text's id in the caller's `SourceMap`, for spans.
    pub fn add_text(&mut self, text: &str, source: SourceId) -> Result<(), Diagnostics> {
        include(self, text, Some(source), false)
    }
}

/// The sized number types by name, and `i64` and `f64` for `int` and `float`.
pub(crate) fn numeric_type(name: &str) -> Option<TypeId> {
    match name {
        "i64" => Some(TypeId::INT),
        "f64" => Some(TypeId::FLOAT),
        "f32" => Some(TypeId::F32),
        _ => SIZED_INTS.iter().find(|s| s.0 == name).map(|s| s.1),
    }
}

/// Decodes a string literal (with its quotes).
fn unquote(lit: &str) -> String {
    let inner = &lit[1..lit.len() - 1];
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('u') => {
                let rest: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let hex = rest.trim_start_matches('{');
                if let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

struct Abort;
type R<T> = Result<T, Abort>;

struct TextParser<'a> {
    tree: &'a SyntaxTree,
    b: &'a mut HirBuilder,
    /// Whether what is built gets the spans of the text.
    spans: bool,
    /// Whether the text is a module of the standard library (see `include`).
    library: bool,
    diags: Diagnostics,
    scopes: Scopes,
    body: Option<BodyId>,
    /// Scopes opened by `let` shadowing in each open block, to close when the block ends.
    extra_scopes: Vec<u32>,
    /// The type parameters of the item being parsed, by name.
    tparams: Vec<(String, TypeId)>,
}

impl<'a> TextParser<'a> {
    fn new(
        tree: &'a SyntaxTree,
        b: &'a mut HirBuilder,
        source: Option<SourceId>,
        library: bool,
    ) -> TextParser<'a> {
        TextParser {
            tree,
            b,
            spans: source.is_some(),
            library,
            diags: Diagnostics::new(),
            scopes: Scopes::new(),
            body: None,
            extra_scopes: Vec::new(),
            tparams: Vec::new(),
        }
    }

    /// The type a name means here: one of the current module's, or one known by its full name.
    fn type_item(&self, name: &str) -> Option<Item> {
        self.b
            .lookup_type_name(name)
            .or_else(|| self.b.module().lookup_type(name))
    }

    fn value_item(&self, name: &str) -> Option<Item> {
        self.b
            .lookup_value_name(name)
            .or_else(|| self.b.module().lookup_value(name))
    }

    fn error<T>(&mut self, node: NodeId, code: &str, message: &str) -> R<T> {
        self.diags
            .push(Diagnostic::error(code, message).with_span(self.tree.span(node)));
        Err(Abort)
    }

    /// A token's text; a backquoted name without its backquotes.
    fn text(&self, n: NodeId) -> &str {
        let t = self.tree.text(n);
        match t.strip_prefix('`') {
            Some(inner) => inner.strip_suffix('`').unwrap_or(inner),
            None => t,
        }
    }

    fn field(&self, n: NodeId, name: &str) -> Option<NodeId> {
        self.tree.field_named(n, name)
    }

    fn fields(&self, n: NodeId, name: &str) -> Vec<NodeId> {
        self.tree.fields_named(n, name)
    }

    fn kind(&self, n: NodeId) -> &str {
        self.tree.kind_name(n)
    }

    fn at(&mut self, n: NodeId) {
        let span = self.spans.then(|| self.tree.span(n));
        self.b.set_span(span);
    }

    /// Attributes a declaration to its name, which is what messages point at.
    fn at_name(&mut self, n: NodeId) {
        let name = self.field(n, "name").unwrap_or(n);
        self.at(name);
    }

    // ---------------------------------------------------------------- items

    /// Builds the items of the module; returns the problems with the text.
    fn module(mut self, root: NodeId) -> Diagnostics {
        let items: Vec<NodeId> = self
            .tree
            .children(root)
            .into_iter()
            .filter(|&c| !self.tree.is_token(c))
            .collect();
        // Pass 1: type names, so any declaration can refer to any type.
        for &item in &items {
            self.at_name(item);
            let name = self.field(item, "name").map(|n| self.text(n).to_string());
            let tparams = self.type_param_names(item);
            let tparams: Vec<&str> = tparams.iter().map(String::as_str).collect();
            match (self.kind(item), name) {
                ("struct_decl", Some(n)) => {
                    let s = self.b.declare_generic_struct(&n, &tparams);
                    if self.field(item, "inline").is_some() {
                        self.b.set_struct_inline(s);
                    }
                }
                ("enum_decl", Some(n)) => {
                    self.b.declare_generic_enum(&n, &tparams);
                }
                ("interface_decl", Some(n)) => {
                    self.b.declare_interface(&n);
                }
                _ => {}
            }
        }
        // Pass 2: fields, variants and signatures.
        let mut funcs = Vec::new();
        for &item in &items {
            self.at(item);
            if self.library {
                let private = self.kind(item) == "extern_decl"
                    && self
                        .field(item, "name")
                        .is_some_and(|n| self.text(n).starts_with("a0_"));
                self.b.set_public(!private);
            }
            let _ = self.declaration(item, &mut funcs);
            self.tparams.clear();
        }
        if self.library {
            self.b.set_public(true);
        }
        // Pass 3: bodies, with their function's type parameters in scope.
        for (f, item) in funcs {
            self.bind_type_params(Item::Func(f));
            let _ = self.function_body(f, item);
            self.tparams.clear();
        }
        self.diags
    }

    fn type_param_names(&self, item: NodeId) -> Vec<String> {
        let Some(tp) = self
            .tree
            .children(item)
            .into_iter()
            .find(|&c| self.kind(c) == "type_params")
        else {
            return Vec::new();
        };
        self.fields(tp, "tparam")
            .into_iter()
            .map(|p| self.text(self.field(p, "name").unwrap()).to_string())
            .collect()
    }

    /// The bounds written on `item`'s type parameters, which are the parameters of `owner` from
    /// `skip` on.
    fn bounds(&mut self, item: NodeId, owner: Item, skip: u32) -> R<()> {
        let Some(tp) = self
            .tree
            .children(item)
            .into_iter()
            .find(|&c| self.kind(c) == "type_params")
        else {
            return Ok(());
        };
        for (k, p) in self.fields(tp, "tparam").into_iter().enumerate() {
            for b in self.fields(p, "bound") {
                let name = self.text(b).to_string();
                let Some(Item::Interface(i)) = self.type_item(&name) else {
                    return self.error(
                        b,
                        code::UNKNOWN_TYPE,
                        &format!("unknown interface '{name}'"),
                    );
                };
                self.b.add_bound(owner, skip + k as u32, i);
            }
        }
        Ok(())
    }

    /// The type an `impl` line names: a struct, an enum, a value type or (before `for`) an
    /// interface.
    fn impl_type(&mut self, node: NodeId) -> R<Result<TypeId, InterfaceId>> {
        let Some(n) = self.field(node, "name") else {
            return Ok(Ok(match self.tree.text(self.tree.child(node, 0)) {
                "int" => TypeId::INT,
                "float" => TypeId::FLOAT,
                "bool" => TypeId::BOOL,
                _ => TypeId::STRING,
            }));
        };
        let name = self.text(n).to_string();
        match self.type_item(&name) {
            Some(Item::Interface(i)) => Ok(Err(i)),
            Some(item @ (Item::Struct(_) | Item::Enum(_))) => {
                Ok(Ok(self.b.instantiate_named(item, &[])))
            }
            _ => self.error(n, code::UNKNOWN_TYPE, &format!("unknown type '{name}'")),
        }
    }

    /// The signature of a function or method `f` declared by `item`, with its parameters' spans.
    fn signature(&mut self, f: FuncId, item: NodeId) -> R<()> {
        self.bind_type_params(Item::Func(f));
        let (names, types) = self.params(item)?;
        let ret = self.ret(item)?;
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.at_name(item);
        self.b.set_signature(f, &refs, &types, ret);
        let body = self.b.function_body(f);
        let params: Vec<NodeId> = self
            .tree
            .children(item)
            .into_iter()
            .filter(|&c| self.kind(c) == "param")
            .collect();
        for (i, p) in params.into_iter().enumerate() {
            if self.spans {
                let span = self.tree.span(self.field(p, "name").unwrap_or(p));
                self.b.set_param_span(body, i as u32, span);
            }
        }
        Ok(())
    }

    /// Puts the type parameters of `owner` in scope for the types that follow.
    fn bind_type_params(&mut self, owner: Item) {
        let names = self.b.module().type_params(owner).to_vec();
        self.tparams = names
            .into_iter()
            .enumerate()
            .map(|(i, n)| (n, self.b.type_param(owner, i as u32)))
            .collect();
    }

    fn declaration(&mut self, item: NodeId, funcs: &mut Vec<(FuncId, NodeId)>) -> R<()> {
        let name = self
            .field(item, "name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        match self.kind(item) {
            "struct_decl" => {
                let Some(Item::Struct(s)) = self.type_item(&name) else {
                    return Ok(());
                };
                self.bind_type_params(Item::Struct(s));
                for f in self.tree.children(item) {
                    if self.kind(f) == "field" {
                        let fname = self.text(self.field(f, "name").unwrap()).to_string();
                        let t = self.ty(self.field(f, "type").unwrap())?;
                        self.at_name(f);
                        self.b.add_field(s, &fname, t);
                    }
                }
            }
            "enum_decl" => {
                let Some(Item::Enum(e)) = self.type_item(&name) else {
                    return Ok(());
                };
                self.bind_type_params(Item::Enum(e));
                for v in self.tree.children(item) {
                    if self.kind(v) == "variant" {
                        let vname = self.text(self.field(v, "name").unwrap()).to_string();
                        let mut tys = Vec::new();
                        for t in self.fields(v, "type") {
                            tys.push(self.ty(t)?);
                        }
                        self.at_name(v);
                        self.b.add_variant(e, &vname, &tys);
                    }
                }
            }
            "global_decl" => {
                let t = self.ty(self.field(item, "type").unwrap())?;
                self.at_name(item);
                self.b.declare_global(&name, t);
            }
            "extern_decl" => {
                let mut params = Vec::new();
                for t in self.fields(item, "param_type") {
                    params.push(self.ty(t)?);
                }
                let ret = self.ret(item)?;
                let abi = match self.field(item, "abi") {
                    None => Abi::C,
                    Some(n) => match Abi::from_name(&unquote(self.text(n))) {
                        Some(abi) => abi,
                        None => {
                            return self.error(
                                n,
                                "hir.text",
                                "unknown ABI: externs are \"C\" or \"a0\"",
                            );
                        }
                    },
                };
                self.at_name(item);
                self.b.declare_extern_abi(&name, &params, ret, abi);
            }
            "fn_decl" => {
                let tparams = self.type_param_names(item);
                let tparams: Vec<&str> = tparams.iter().map(String::as_str).collect();
                self.at_name(item);
                let f = self.b.declare_generic_function(&name, &tparams);
                self.bounds(item, Item::Func(f), 0)?;
                self.signature(f, item)?;
                funcs.push((f, item));
            }
            "interface_decl" => {
                let Some(Item::Interface(i)) = self.type_item(&name) else {
                    return Ok(());
                };
                let me = self.b.interface_self(i);
                for sig in self.fields(item, "method") {
                    self.tparams = vec![("Self".to_string(), me)];
                    let mut params = Vec::new();
                    for t in self.fields(sig, "param_type") {
                        params.push(self.ty(t)?);
                    }
                    let ret = self.ret(sig)?;
                    self.at_name(sig);
                    let mname = self.text(self.field(sig, "name").unwrap()).to_string();
                    self.b.add_interface_method(i, &mname, &params, ret);
                }
            }
            "impl_decl" => {
                let owner = self.impl_type(self.field(item, "owner").unwrap())?;
                if let Some(target) = self.field(item, "target") {
                    let (Err(i), Ok(t)) = (owner, self.impl_type(target)?) else {
                        return self.error(
                            item,
                            code::UNKNOWN_TYPE,
                            "expected `impl Interface for Type;`",
                        );
                    };
                    self.at(item);
                    self.b.implement(t, i);
                    return Ok(());
                }
                let Ok(owner) = owner else {
                    return self.error(item, code::UNKNOWN_TYPE, "methods belong to a type");
                };
                // A method's own type parameters come after its owner's.
                let skip = self.b.module().type_args(owner).len() as u32;
                for method in self.fields(item, "method") {
                    let mname = self.text(self.field(method, "name").unwrap()).to_string();
                    let own = self.type_param_names(method);
                    let own: Vec<&str> = own.iter().map(String::as_str).collect();
                    self.at_name(method);
                    let f = self.b.declare_method(owner, &mname, &own);
                    self.bounds(method, Item::Func(f), skip)?;
                    self.signature(f, method)?;
                    funcs.push((f, method));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn params(&mut self, node: NodeId) -> R<(Vec<String>, Vec<TypeId>)> {
        let mut names = Vec::new();
        let mut types = Vec::new();
        for c in self.tree.children(node) {
            if self.kind(c) == "param" {
                names.push(self.text(self.field(c, "name").unwrap()).to_string());
                types.push(self.ty(self.field(c, "type").unwrap())?);
            }
        }
        Ok((names, types))
    }

    fn ret(&mut self, node: NodeId) -> R<TypeId> {
        match self.field(node, "ret") {
            Some(t) => self.ty(t),
            None => Ok(TypeId::UNIT),
        }
    }

    fn ty(&mut self, node: NodeId) -> R<TypeId> {
        let t = self.base_type(self.field(node, "base").unwrap())?;
        if self.field(node, "nullable").is_some() {
            self.at(node);
            return Ok(self.b.nullable_type(t));
        }
        Ok(t)
    }

    /// The type arguments written after `node`'s name, if any.
    fn type_args(&mut self, node: NodeId) -> R<Option<Vec<TypeId>>> {
        let Some(ta) = self
            .tree
            .children(node)
            .into_iter()
            .find(|&c| self.kind(c) == "type_args")
        else {
            return Ok(None);
        };
        let mut out = Vec::new();
        for t in self.fields(ta, "targ") {
            out.push(self.ty(t)?);
        }
        Ok(Some(out))
    }

    fn base_type(&mut self, node: NodeId) -> R<TypeId> {
        let first = self.tree.child(node, 0);
        Ok(match self.text(first) {
            "unit" => TypeId::UNIT,
            "bool" => TypeId::BOOL,
            "int" => TypeId::INT,
            "float" => TypeId::FLOAT,
            "string" => TypeId::STRING,
            "any" => TypeId::ANY,
            "never" => TypeId::NEVER,
            "array" => {
                let e = self.ty(self.field(node, "elem").unwrap())?;
                self.b.array_type(e)
            }
            "map" => {
                let k = self.ty(self.field(node, "key").unwrap())?;
                let v = self.ty(self.field(node, "value").unwrap())?;
                self.at(node);
                self.b.map_type(k, v)
            }
            "fn" => {
                let mut ps = Vec::new();
                for t in self.fields(node, "param_type") {
                    ps.push(self.ty(t)?);
                }
                let r = self.ret(node)?;
                self.b.fn_type(&ps, r)
            }
            "(" => return self.ty(self.field(node, "inner").unwrap()),
            name => {
                let name = name.to_string();
                if let Some((_, t)) = self.tparams.iter().find(|(n, _)| *n == name) {
                    return Ok(*t);
                }
                let args = self.type_args(node)?.unwrap_or_default();
                self.at(node);
                match self.type_item(&name) {
                    Some(Item::Struct(s)) => self.b.struct_type_args(s, &args),
                    Some(Item::Enum(e)) => self.b.enum_type_args(e, &args),
                    Some(Item::Interface(i)) if args.is_empty() => self.b.interface_type(i),
                    // Not keywords, so that they stay free as names.
                    None if name == "object" && args.is_empty() => TypeId::OBJECT,
                    None if args.is_empty() && numeric_type(&name).is_some() => {
                        numeric_type(&name).unwrap()
                    }
                    _ => {
                        return self.error(
                            node,
                            code::UNKNOWN_TYPE,
                            &format!("unknown type '{name}'"),
                        );
                    }
                }
            }
        })
    }

    // ---------------------------------------------------------------- bodies

    fn function_body(&mut self, f: FuncId, item: NodeId) -> R<()> {
        let body = self.b.function_body(f);
        let block = self.field(item, "body").unwrap();
        let root = self.with_body(body, item, block)?;
        self.b.set_root(body, root);
        Ok(())
    }

    /// Parses `block` as the root of `body`, with the parameters of `decl` (a function or closure)
    /// in scope under the names written there; locals may have been renamed to keep them unique.
    fn with_body(&mut self, body: BodyId, decl: NodeId, block: NodeId) -> R<ExprId> {
        let outer = self.body.replace(body);
        self.scopes.push(ScopeKind::Function);
        let params = self.b.module().body_params(body).to_vec();
        let names: Vec<String> = self
            .tree
            .children(decl)
            .into_iter()
            .filter(|&c| self.kind(c) == "param")
            .map(|c| self.text(self.field(c, "name").unwrap()).to_string())
            .collect();
        for (name, p) in names.iter().zip(params) {
            let _ = self.scopes.declare_defined(name, p.0 as u64);
        }
        let r = self.block(block);
        self.scopes.pop();
        self.body = outer;
        r
    }

    fn block(&mut self, node: NodeId) -> R<ExprId> {
        self.scopes.push(ScopeKind::Block);
        self.extra_scopes.push(0);
        let r = self.block_items(node);
        for _ in 0..self.extra_scopes.pop().unwrap() {
            self.scopes.pop();
        }
        self.scopes.pop();
        r
    }

    fn block_items(&mut self, node: NodeId) -> R<ExprId> {
        let mut items = Vec::new();
        for c in self.tree.children(node) {
            match self.kind(c) {
                "let_stmt" => items.push(self.let_stmt(c)?),
                "expr_stmt" | "blocklike_stmt" => {
                    items.push(self.expr(self.field(c, "e").unwrap())?)
                }
                _ => {}
            }
        }
        let tail = match self.field(node, "tail") {
            Some(t) => Some(self.expr(t)?),
            None => None,
        };
        self.at(node);
        Ok(self.b.block(&items, tail))
    }

    fn let_stmt(&mut self, node: NodeId) -> R<ExprId> {
        let name_node = self.field(node, "name").unwrap();
        let name = self.text(name_node).to_string();
        // The initializer is resolved before the new name is in scope.
        let value = self.expr(self.field(node, "value").unwrap())?;
        let ty = match self.field(node, "type") {
            Some(t) => self.ty(t)?,
            None => self.b.type_of(value),
        };
        if ty == TypeId::NEVER {
            return self.error(
                node,
                code::CANNOT_INFER,
                "cannot infer the type of this let; write it",
            );
        }
        let local = self.b.new_local(self.body.unwrap(), &name, ty);
        if self.scopes.lookup_in_current(&name).is_some() {
            // Shadowing in the same block: the new name lives in a nested scope.
            self.scopes.push(ScopeKind::Block);
            *self.extra_scopes.last_mut().unwrap() += 1;
        }
        let _ = self.scopes.declare_defined(&name, local.0 as u64);
        self.at(node);
        Ok(self.b.let_(local, value))
    }

    /// The name in a `name_expr` (without type arguments).
    fn name_of(&self, name_expr: NodeId) -> String {
        self.text(self.field(name_expr, "name").unwrap())
            .to_string()
    }

    fn lookup_local(&self, name: &str) -> Option<LocalId> {
        self.scopes
            .lookup(name)
            .map(|r| LocalId(self.scopes.data(r.symbol) as u32))
    }

    fn args(&mut self, node: NodeId) -> R<Vec<ExprId>> {
        let mut out = Vec::new();
        if let Some(args) = self.field(node, "args") {
            for a in self.fields(args, "arg") {
                out.push(self.expr(a)?);
            }
        }
        Ok(out)
    }

    fn expr(&mut self, node: NodeId) -> R<ExprId> {
        let kind = self.kind(node).to_string();
        let e = match kind.as_str() {
            "primary" => return self.primary(node),
            "assign" => self.assign(node)?,
            "or" | "and" => {
                let a = self.expr(self.field(node, "left").unwrap())?;
                let b = self.expr(self.field(node, "right").unwrap())?;
                self.at(node);
                if kind == "or" {
                    self.b.or(a, b)
                } else {
                    self.b.and(a, b)
                }
            }
            "binary" => {
                let op = match self.text(self.field(node, "op").unwrap()) {
                    "shl" => BinOp::Shl,
                    "shr" => BinOp::Shr,
                    op => BinOp::from_symbol(op).unwrap(),
                };
                let a = self.expr(self.field(node, "left").unwrap())?;
                let b = self.expr(self.field(node, "right").unwrap())?;
                self.at(node);
                self.b.binary(op, a, b)
            }
            "unary" => self.unary(node)?,
            "cast" => {
                let a = self.expr(self.field(node, "operand").unwrap())?;
                let t = self.ty(self.field(node, "ty").unwrap())?;
                self.at(node);
                self.b.cast(a, t)
            }
            "is" => {
                let a = self.expr(self.field(node, "operand").unwrap())?;
                let t = self.ty(self.field(node, "ty").unwrap())?;
                self.at(node);
                self.b.is(a, t)
            }
            "call" => self.call(node)?,
            "field" => {
                let o = self.expr(self.field(node, "operand").unwrap())?;
                let name = self.text(self.field(node, "name").unwrap()).to_string();
                self.at(node);
                self.b.field_named(o, &name)
            }
            "unwrap" => {
                let o = self.expr(self.field(node, "operand").unwrap())?;
                self.at(node);
                self.b.unwrap(o)
            }
            "index" => {
                let t = self.expr(self.field(node, "operand").unwrap())?;
                let i = self.expr(self.field(node, "index").unwrap())?;
                self.at(node);
                self.b.index(t, i)
            }
            other => {
                // Block-like forms appear directly as statements.
                return self.primary_form(node, other);
            }
        };
        Ok(e)
    }

    fn unary(&mut self, node: NodeId) -> R<ExprId> {
        let op = self.text(self.field(node, "op").unwrap()).to_string();
        let operand = self.field(node, "operand").unwrap();
        // `-5` and `-1.5` are literals, not negations, so printed constants read back unchanged.
        if op == "-" && self.kind(operand) == "primary" {
            let inner = self.tree.child(operand, 0);
            if self.kind(inner) == "literal" {
                let tok = self.tree.child(inner, 0);
                let text = self.text(tok).to_string();
                match self.tree.kind_name(tok) {
                    "INT" => {
                        self.at(node);
                        return match text.parse::<u64>() {
                            Ok(v) if v <= i64::MAX as u64 + 1 => {
                                Ok(self.b.int((v as i64).wrapping_neg()))
                            }
                            _ => self.error(
                                node,
                                code::INVALID_TOKEN,
                                "integer literal out of range",
                            ),
                        };
                    }
                    "FLOAT" => {
                        self.at(node);
                        let v: f64 = text.parse().unwrap_or(0.0);
                        return Ok(self.b.float(-v));
                    }
                    _ => {}
                }
            }
        }
        let a = self.expr(operand)?;
        self.at(node);
        Ok(self.b.unary(
            match op.as_str() {
                "-" => UnOp::Neg,
                "~" => UnOp::BitNot,
                _ => UnOp::Not,
            },
            a,
        ))
    }

    fn assign(&mut self, node: NodeId) -> R<ExprId> {
        let target = self.field(node, "left").unwrap();
        let value_node = self.field(node, "right").unwrap();
        match self.kind(target) {
            "primary" if self.kind(self.tree.child(target, 0)) == "name_expr" => {
                let name = self.name_of(self.tree.child(target, 0));
                let value = self.expr(value_node)?;
                self.at(node);
                if let Some(l) = self.lookup_local(&name) {
                    return Ok(self.b.set_local(l, value));
                }
                match self.value_item(&name) {
                    Some(Item::Global(g)) => Ok(self.b.set_global(g, value)),
                    _ => self.error(
                        target,
                        code::NOT_ASSIGNABLE,
                        &format!("cannot assign to '{name}'"),
                    ),
                }
            }
            "field" => {
                let o = self.expr(self.field(target, "operand").unwrap())?;
                let name = self.text(self.field(target, "name").unwrap()).to_string();
                let value = self.expr(value_node)?;
                self.at(node);
                Ok(self.b.set_field_named(o, &name, value))
            }
            "index" => {
                let t = self.expr(self.field(target, "operand").unwrap())?;
                let i = self.expr(self.field(target, "index").unwrap())?;
                let value = self.expr(value_node)?;
                self.at(node);
                Ok(self.b.set_index(t, i, value))
            }
            _ => self.error(target, code::NOT_ASSIGNABLE, "invalid assignment target"),
        }
    }

    fn call(&mut self, node: NodeId) -> R<ExprId> {
        let callee = self.field(node, "operand").unwrap();
        // A bare name may be a builtin, a function or an extern; anything else is a closure value.
        if self.kind(callee) == "primary" && self.kind(self.tree.child(callee, 0)) == "name_expr" {
            let name_node = self.tree.child(callee, 0);
            let name = self.name_of(name_node);
            let targs = self.type_args(name_node)?;
            // A backquoted name is always a declared item, never a builtin.
            let quoted = self.tree.text(name_node).starts_with('`');
            if self.lookup_local(&name).is_none() {
                if let Some(op) = Builtin::from_name(&name).filter(|_| !quoted) {
                    let args = self.args(node)?;
                    self.at(node);
                    return Ok(self.b.builtin(op, &args));
                }
                match self.value_item(&name) {
                    Some(Item::Func(f)) => {
                        let args = self.args(node)?;
                        self.at(node);
                        return Ok(match targs {
                            Some(t) => self.b.call_generic(f, &t, &args),
                            None => self.b.call(f, &args),
                        });
                    }
                    Some(Item::Extern(x)) => {
                        let args = self.args(node)?;
                        self.at(node);
                        return Ok(self.b.call_extern(x, &args));
                    }
                    _ => {}
                }
            }
        }
        // `o.m(x)` calls a method (`(o.f)(x)` calls a closure in a field).
        if self.kind(callee) == "field" {
            let o = self.expr(self.field(callee, "operand").unwrap())?;
            let name = self.text(self.field(callee, "name").unwrap()).to_string();
            let args = self.args(node)?;
            self.at(node);
            return Ok(self.b.method_call(o, &name, &args));
        }
        let c = self.expr(callee)?;
        let args = self.args(node)?;
        self.at(node);
        Ok(self.b.call_value(c, &args))
    }

    fn primary(&mut self, node: NodeId) -> R<ExprId> {
        let inner = self.tree.child(node, 0);
        let kind = self.kind(inner).to_string();
        self.primary_form(inner, &kind)
    }

    fn primary_form(&mut self, node: NodeId, kind: &str) -> R<ExprId> {
        self.at(node);
        Ok(match kind {
            "unit_lit" => self.b.unit(),
            "paren" => return self.expr(self.field(node, "inner").unwrap()),
            "literal" => {
                let tok = self.tree.child(node, 0);
                let text = self.text(tok).to_string();
                match self.tree.kind_name(tok) {
                    "INT" => match text.parse::<i64>() {
                        Ok(v) => self.b.int(v),
                        Err(_) => {
                            return self.error(
                                node,
                                code::INVALID_TOKEN,
                                "integer literal out of range",
                            );
                        }
                    },
                    "FLOAT" => self.b.float(text.parse().unwrap_or(0.0)),
                    "STRING" => self.b.string(&unquote(&text)),
                    _ => self.b.bool_lit(text == "true"),
                }
            }
            "float_bits" => {
                let hex = self.text(self.field(node, "bits").unwrap()).to_string();
                match u64::from_str_radix(&hex[2..], 16) {
                    Ok(bits) => self.b.float_bits(bits),
                    Err(_) => return self.error(node, code::INVALID_TOKEN, "invalid float bits"),
                }
            }
            "null_lit" => {
                let t = self.ty(self.field(node, "type").unwrap())?;
                self.at(node);
                self.b.null(t)
            }
            "new_expr" => return self.new_expr(node),
            "array_lit" => {
                let elems = self.args(node)?;
                self.at(node);
                self.b.array(&elems)
            }
            "variant_expr" => {
                let ename = self.text(self.field(node, "enum").unwrap()).to_string();
                let vname = self.text(self.field(node, "name").unwrap()).to_string();
                let Some(Item::Enum(e)) = self.type_item(&ename) else {
                    return self.error(
                        node,
                        code::UNKNOWN_TYPE,
                        &format!("unknown enum '{ename}'"),
                    );
                };
                let Some(v) = self.b.module().variant_index(e, &vname) else {
                    return self.error(
                        node,
                        code::UNKNOWN_VARIANT,
                        &format!("enum {ename} has no variant '{vname}'"),
                    );
                };
                let targs = self.type_args(node)?;
                let args = self.args(node)?;
                self.at(node);
                match targs {
                    Some(t) => self.b.variant_generic(e, &t, v, &args),
                    None => self.b.variant(e, v, &args),
                }
            }
            "lambda" => {
                let (names, types) = self.params(node)?;
                let ret = self.ret(node)?;
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                let body = self
                    .b
                    .new_closure_body(self.body.unwrap(), &refs, &types, ret);
                let root = self.with_body(body, node, self.field(node, "body").unwrap())?;
                self.b.set_root(body, root);
                self.at(node);
                self.b.lambda(body)
            }
            "if_expr" => {
                let e = |p: &mut Self| -> R<Option<ExprId>> {
                    Ok(match p.field(node, "else") {
                        Some(n) if p.kind(n) == "if_expr" => Some(p.primary_form(n, "if_expr")?),
                        Some(n) => Some(p.block(n)?),
                        None => None,
                    })
                };
                if let Some(bind) = self.field(node, "bind") {
                    let value = self.expr(self.field(node, "value").unwrap())?;
                    let name = self.text(bind).to_string();
                    // The binding's type is the value's without `?`, known in the body unless the
                    // value's type is still open (then it is settled when the node is typed).
                    let lt = match self.b.module().kind(self.b.type_of(value)) {
                        TypeKind::Nullable(inner) => *inner,
                        _ => self.b.fresh_type(),
                    };
                    self.at(bind);
                    let l = self.b.new_local(self.body.unwrap(), &name, lt);
                    self.scopes.push(ScopeKind::Block);
                    let _ = self.scopes.declare_defined(&name, l.0 as u64);
                    let t = self.block(self.field(node, "then").unwrap());
                    self.scopes.pop();
                    let t = t?;
                    let els = e(self)?;
                    self.at(node);
                    self.b.if_let(l, value, t, els)
                } else {
                    let c = self.expr(self.field(node, "cond").unwrap())?;
                    let t = self.block(self.field(node, "then").unwrap())?;
                    let els = e(self)?;
                    self.at(node);
                    self.b.if_(c, t, els)
                }
            }
            "while_expr" => {
                let c = self.expr(self.field(node, "cond").unwrap())?;
                let body = self.block(self.field(node, "body").unwrap())?;
                self.at(node);
                self.b.while_(c, body)
            }
            "loop_expr" => {
                let body = self.block(self.field(node, "body").unwrap())?;
                self.at(node);
                self.b.loop_(body)
            }
            "match_expr" => return self.match_expr(node),
            "block" => return self.block(node),
            "break_expr" => self.b.break_(),
            "continue_expr" => self.b.continue_(),
            "return_expr" => {
                let v = match self.field(node, "value") {
                    Some(v) => Some(self.expr(v)?),
                    None => None,
                };
                self.at(node);
                self.b.return_(v)
            }
            "throw_expr" => {
                let v = self.expr(self.field(node, "value").unwrap())?;
                self.at(node);
                self.b.throw(v)
            }
            "try_expr" => {
                let body = self.block(self.field(node, "body").unwrap())?;
                let bind = self.field(node, "bind").unwrap();
                let name = self.text(bind).to_string();
                self.at(bind);
                let l = self.b.new_local(self.body.unwrap(), &name, TypeId::ANY);
                self.scopes.push(ScopeKind::Block);
                let _ = self.scopes.declare_defined(&name, l.0 as u64);
                let handler = self.block(self.field(node, "handler").unwrap());
                self.scopes.pop();
                let handler = handler?;
                self.at(node);
                self.b.try_catch(body, l, handler)
            }
            "name_expr" => {
                let name = self.name_of(node);
                if let Some(l) = self.lookup_local(&name) {
                    return Ok(self.b.local(l));
                }
                let targs = self.type_args(node)?;
                self.at(node);
                match self.value_item(&name) {
                    Some(Item::Global(g)) => self.b.global(g),
                    Some(Item::Func(f)) => match targs {
                        Some(t) => self.b.func_generic(f, &t),
                        None => self.b.func(f),
                    },
                    _ => {
                        return self.error(
                            node,
                            code::UNKNOWN_NAME,
                            &format!("unknown name '{name}'"),
                        );
                    }
                }
            }
            other => return self.error(node, code::SYNTAX, &format!("unexpected {other}")),
        })
    }

    fn new_expr(&mut self, node: NodeId) -> R<ExprId> {
        if let Some(s) = self
            .tree
            .children(node)
            .into_iter()
            .find(|&c| self.kind(c) == "new_struct")
        {
            let name = self.text(self.field(s, "name").unwrap()).to_string();
            let Some(Item::Struct(sid)) = self.type_item(&name) else {
                return self.error(s, code::UNKNOWN_TYPE, &format!("unknown struct '{name}'"));
            };
            let count = self.b.module().struct_field_count(sid) as usize;
            let mut values: Vec<Option<ExprId>> = vec![None; count];
            for init in self.tree.children(s) {
                if self.kind(init) != "field_init" {
                    continue;
                }
                let fname = self.text(self.field(init, "name").unwrap()).to_string();
                let v = self.expr(self.field(init, "value").unwrap())?;
                match self.b.module().struct_field_index(sid, &fname) {
                    Some(i) if values[i as usize].is_none() => values[i as usize] = Some(v),
                    Some(_) => {
                        return self.error(
                            init,
                            code::FIELD_TWICE,
                            &format!("field '{fname}' is given twice"),
                        );
                    }
                    None => {
                        return self.error(
                            init,
                            code::UNKNOWN_FIELD,
                            &format!("struct {name} has no field '{fname}'"),
                        );
                    }
                }
            }
            let mut fields = Vec::new();
            for (i, v) in values.into_iter().enumerate() {
                match v {
                    Some(v) => fields.push(v),
                    None => {
                        let fname = self.b.module().struct_field(sid, i as u32).0.to_string();
                        return self.error(
                            s,
                            code::MISSING_FIELDS,
                            &format!("missing field '{fname}'"),
                        );
                    }
                }
            }
            let targs = self.type_args(s)?;
            self.at(node);
            return Ok(match targs {
                Some(t) => self.b.new_struct_generic(sid, &t, &fields),
                None => self.b.new_struct(sid, &fields),
            });
        }
        let coll = self
            .tree
            .children(node)
            .into_iter()
            .find(|&c| self.kind(c) == "new_collection")
            .unwrap();
        let t = self.ty(self.field(coll, "type").unwrap())?;
        self.at(node);
        Ok(match self.b.module().type_tag(t) {
            TypeTag::Array => self.b.new_array(t),
            TypeTag::Map => self.b.new_map(t),
            TypeTag::Object => self.b.new_object(),
            _ => {
                return self.error(
                    node,
                    code::SYNTAX,
                    "new needs a struct literal, an array, map or object type",
                );
            }
        })
    }

    fn match_expr(&mut self, node: NodeId) -> R<ExprId> {
        let scrutinee = self.expr(self.field(node, "scrutinee").unwrap())?;
        let st = self.b.type_of(scrutinee);
        let e = match self.b.module().kind(st) {
            TypeKind::Enum(e, _) => *e,
            _ => return self.error(node, code::NOT_AN_ENUM, "match needs an enum"),
        };
        let mut arms = Vec::new();
        for arm in self.tree.children(node) {
            if self.kind(arm) != "arm" {
                continue;
            }
            let pattern = self.field(arm, "pattern").unwrap();
            let body_node = self.field(arm, "body").unwrap();
            self.scopes.push(ScopeKind::Block);
            let result = (|| -> R<ArmId> {
                let variant = match self.field(pattern, "variant") {
                    None => None,
                    Some(vn) => {
                        let vname = self.text(vn).to_string();
                        match self.b.module().variant_index(e, &vname) {
                            Some(v) => Some(v),
                            None => {
                                return self.error(
                                    vn,
                                    code::UNKNOWN_VARIANT,
                                    &format!("no variant '{vname}'"),
                                );
                            }
                        }
                    }
                };
                let mut binds = Vec::new();
                if let Some(v) = variant {
                    let count = self.b.module().variant_fields(e, v).len();
                    let bind_nodes = self.fields(pattern, "bind");
                    if bind_nodes.len() != count {
                        return self.error(
                            pattern,
                            code::ARGUMENT_COUNT,
                            &format!("this variant has {count} fields"),
                        );
                    }
                    for bn in bind_nodes {
                        let bname = self.text(bn).to_string();
                        // The binding's type comes from the scrutinee's type when the match is typed.
                        let ft = self.b.fresh_type();
                        self.at(bn);
                        let l = self.b.new_local(self.body.unwrap(), &bname, ft);
                        let _ = self.scopes.declare_defined(&bname, l.0 as u64);
                        binds.push(l);
                    }
                }
                let body = self.expr(body_node)?;
                Ok(self.b.arm(variant, &binds, body))
            })();
            self.scopes.pop();
            arms.push(result?);
        }
        self.at(node);
        Ok(self.b.match_(scrutinee, &arms))
    }
}
