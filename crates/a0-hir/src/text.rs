//! Parsing the HIR text format, using A0's own grammar layer (`hir.a0g`) and scopes from `a0-sema`.

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
    let mut p = TextParser {
        tree: &tree,
        b: HirBuilder::new(&unquote(tree.text(name_node))),
        diags: Diagnostics::new(),
        scopes: Scopes::new(),
        body: None,
        extra_scopes: Vec::new(),
    };
    p.module(root);
    if p.diags.has_errors() {
        return Err(p.diags);
    }
    p.b.finish()
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
    b: HirBuilder,
    diags: Diagnostics,
    scopes: Scopes,
    body: Option<BodyId>,
    /// Scopes opened by `let` shadowing in each open block, to close when the block ends.
    extra_scopes: Vec<u32>,
}

impl TextParser<'_> {
    fn error<T>(&mut self, node: NodeId, message: &str) -> R<T> {
        self.diags
            .push(Diagnostic::error("hir.syntax", message).with_span(self.tree.span(node)));
        Err(Abort)
    }

    fn text(&self, n: NodeId) -> &str {
        self.tree.text(n)
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
        self.b.set_span(Some(self.tree.span(n)));
    }

    // ---------------------------------------------------------------- items

    fn module(&mut self, root: NodeId) {
        let items: Vec<NodeId> = self
            .tree
            .children(root)
            .into_iter()
            .filter(|&c| !self.tree.is_token(c))
            .collect();
        // Pass 1: type names, so any declaration can refer to any type.
        for &item in &items {
            self.at(item);
            let name = self.field(item, "name").map(|n| self.text(n).to_string());
            match (self.kind(item), name) {
                ("struct_decl", Some(n)) => {
                    self.b.declare_struct(&n);
                }
                ("enum_decl", Some(n)) => {
                    self.b.declare_enum(&n);
                }
                _ => {}
            }
        }
        // Pass 2: fields, variants and signatures.
        let mut funcs = Vec::new();
        for &item in &items {
            self.at(item);
            let _ = self.declaration(item, &mut funcs);
        }
        // Pass 3: bodies.
        for (f, item) in funcs {
            let _ = self.function_body(f, item);
        }
    }

    fn declaration(&mut self, item: NodeId, funcs: &mut Vec<(FuncId, NodeId)>) -> R<()> {
        let name = self
            .field(item, "name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        match self.kind(item) {
            "struct_decl" => {
                let Some(Item::Struct(s)) = self.b.module().lookup_type(&name) else {
                    return Ok(());
                };
                for f in self.tree.children(item) {
                    if self.kind(f) == "field" {
                        let fname = self.text(self.field(f, "name").unwrap()).to_string();
                        let t = self.ty(self.field(f, "type").unwrap())?;
                        self.at(f);
                        self.b.add_field(s, &fname, t);
                    }
                }
            }
            "enum_decl" => {
                let Some(Item::Enum(e)) = self.b.module().lookup_type(&name) else {
                    return Ok(());
                };
                for v in self.tree.children(item) {
                    if self.kind(v) == "variant" {
                        let vname = self.text(self.field(v, "name").unwrap()).to_string();
                        let mut tys = Vec::new();
                        for t in self.fields(v, "type") {
                            tys.push(self.ty(t)?);
                        }
                        self.at(v);
                        self.b.add_variant(e, &vname, &tys);
                    }
                }
            }
            "global_decl" => {
                let t = self.ty(self.field(item, "type").unwrap())?;
                self.at(item);
                self.b.declare_global(&name, t);
            }
            "extern_decl" => {
                let mut params = Vec::new();
                for t in self.fields(item, "param_type") {
                    params.push(self.ty(t)?);
                }
                let ret = self.ret(item)?;
                self.at(item);
                self.b.declare_extern(&name, &params, ret);
            }
            "fn_decl" => {
                let (names, types) = self.params(item)?;
                let ret = self.ret(item)?;
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                self.at(item);
                let f = self.b.declare_function(&name, &refs, &types, ret);
                funcs.push((f, item));
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
            name => match self.b.module().lookup_type(name) {
                Some(Item::Struct(s)) => self.b.struct_type(s),
                Some(Item::Enum(e)) => self.b.enum_type(e),
                _ => return self.error(node, &format!("unknown type '{name}'")),
            },
        })
    }

    // ---------------------------------------------------------------- bodies

    fn function_body(&mut self, f: FuncId, item: NodeId) -> R<()> {
        let body = self.b.function_body(f);
        let block = self.field(item, "body").unwrap();
        let root = self.with_body(body, block)?;
        self.b.set_root(body, root);
        Ok(())
    }

    /// Parses `block` as the root of `body`, with its parameters in scope.
    fn with_body(&mut self, body: BodyId, block: NodeId) -> R<ExprId> {
        let outer = self.body.replace(body);
        self.scopes.push(ScopeKind::Function);
        for &p in self.b.module().body_params(body).to_vec().iter() {
            let name = self.b.module().local_name(p).to_string();
            let _ = self.scopes.declare_defined(&name, p.0 as u64);
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
            return self.error(node, "cannot infer the type of this let; write it");
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
                let op = BinOp::from_symbol(self.text(self.field(node, "op").unwrap())).unwrap();
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
                            _ => self.error(node, "integer literal out of range"),
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
        Ok(self
            .b
            .unary(if op == "-" { UnOp::Neg } else { UnOp::Not }, a))
    }

    fn assign(&mut self, node: NodeId) -> R<ExprId> {
        let target = self.field(node, "left").unwrap();
        let value_node = self.field(node, "right").unwrap();
        match self.kind(target) {
            "primary" if self.kind(self.tree.child(target, 0)) == "name_expr" => {
                let name = self.text(target).to_string();
                let value = self.expr(value_node)?;
                self.at(node);
                if let Some(l) = self.lookup_local(&name) {
                    return Ok(self.b.set_local(l, value));
                }
                match self.b.module().lookup_value(&name) {
                    Some(Item::Global(g)) => Ok(self.b.set_global(g, value)),
                    _ => self.error(target, &format!("cannot assign to '{name}'")),
                }
            }
            "field" => {
                let o = self.expr(self.field(target, "operand").unwrap())?;
                let name = self.text(self.field(target, "name").unwrap()).to_string();
                let value = self.expr(value_node)?;
                let idx = match self.b.module().kind(self.b.type_of(o)) {
                    TypeKind::Struct(s) => self.b.module().struct_field_index(*s, &name),
                    _ => None,
                };
                self.at(node);
                match idx {
                    Some(i) => Ok(self.b.set_field(o, i, value)),
                    None => self.error(target, &format!("no field '{name}'")),
                }
            }
            "index" => {
                let t = self.expr(self.field(target, "operand").unwrap())?;
                let i = self.expr(self.field(target, "index").unwrap())?;
                let value = self.expr(value_node)?;
                self.at(node);
                Ok(self.b.set_index(t, i, value))
            }
            _ => self.error(target, "invalid assignment target"),
        }
    }

    fn call(&mut self, node: NodeId) -> R<ExprId> {
        let callee = self.field(node, "operand").unwrap();
        // A bare name may be a builtin, a function or an extern; anything else is a closure value.
        if self.kind(callee) == "primary" && self.kind(self.tree.child(callee, 0)) == "name_expr" {
            let name = self.text(callee).to_string();
            if self.lookup_local(&name).is_none() {
                if let Some(op) = Builtin::from_name(&name) {
                    let args = self.args(node)?;
                    self.at(node);
                    return Ok(self.b.builtin(op, &args));
                }
                match self.b.module().lookup_value(&name) {
                    Some(Item::Func(f)) => {
                        let args = self.args(node)?;
                        self.at(node);
                        return Ok(self.b.call(f, &args));
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
                        Err(_) => return self.error(node, "integer literal out of range"),
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
                    Err(_) => return self.error(node, "invalid float bits"),
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
                let Some(Item::Enum(e)) = self.b.module().lookup_type(&ename) else {
                    return self.error(node, &format!("unknown enum '{ename}'"));
                };
                let Some(v) = self.b.module().variant_index(e, &vname) else {
                    return self.error(node, &format!("enum {ename} has no variant '{vname}'"));
                };
                let args = self.args(node)?;
                self.at(node);
                self.b.variant(e, v, &args)
            }
            "lambda" => {
                let (names, types) = self.params(node)?;
                let ret = self.ret(node)?;
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                let body = self
                    .b
                    .new_closure_body(self.body.unwrap(), &refs, &types, ret);
                let root = self.with_body(body, self.field(node, "body").unwrap())?;
                self.b.set_root(body, root);
                self.at(node);
                self.b.lambda(body)
            }
            "if_expr" => {
                let c = self.expr(self.field(node, "cond").unwrap())?;
                let t = self.block(self.field(node, "then").unwrap())?;
                let e = match self.field(node, "else") {
                    Some(n) if self.kind(n) == "if_expr" => Some(self.primary_form(n, "if_expr")?),
                    Some(n) => Some(self.block(n)?),
                    None => None,
                };
                self.at(node);
                self.b.if_(c, t, e)
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
            "name_expr" => {
                let name = self.text(node).to_string();
                if let Some(l) = self.lookup_local(&name) {
                    return Ok(self.b.local(l));
                }
                match self.b.module().lookup_value(&name) {
                    Some(Item::Global(g)) => self.b.global(g),
                    Some(Item::Func(f)) => self.b.func(f),
                    _ => return self.error(node, &format!("unknown name '{name}'")),
                }
            }
            other => return self.error(node, &format!("unexpected {other}")),
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
            let Some(Item::Struct(sid)) = self.b.module().lookup_type(&name) else {
                return self.error(s, &format!("unknown struct '{name}'"));
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
                    Some(_) => return self.error(init, &format!("field '{fname}' is given twice")),
                    None => {
                        return self.error(init, &format!("struct {name} has no field '{fname}'"));
                    }
                }
            }
            let mut fields = Vec::new();
            for (i, v) in values.into_iter().enumerate() {
                match v {
                    Some(v) => fields.push(v),
                    None => {
                        let fname = self.b.module().struct_field(sid, i as u32).0.to_string();
                        return self.error(s, &format!("missing field '{fname}'"));
                    }
                }
            }
            self.at(node);
            return Ok(self.b.new_struct(sid, &fields));
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
            _ => {
                return self.error(
                    node,
                    "new needs a struct literal, an array type or a map type",
                );
            }
        })
    }

    fn match_expr(&mut self, node: NodeId) -> R<ExprId> {
        let scrutinee = self.expr(self.field(node, "scrutinee").unwrap())?;
        let st = self.b.type_of(scrutinee);
        let e = match self.b.module().kind(st) {
            TypeKind::Enum(e) => *e,
            _ => return self.error(node, "match needs an enum"),
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
                            None => return self.error(vn, &format!("no variant '{vname}'")),
                        }
                    }
                };
                let mut binds = Vec::new();
                if let Some(v) = variant {
                    let fields = self.b.module().variant_fields(e, v).to_vec();
                    let bind_nodes = self.fields(pattern, "bind");
                    if bind_nodes.len() != fields.len() {
                        return self.error(
                            pattern,
                            &format!("this variant has {} fields", fields.len()),
                        );
                    }
                    for (bn, ft) in bind_nodes.into_iter().zip(fields) {
                        let bname = self.text(bn).to_string();
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
