//! Name resolution and type checking, on `a0-sema`'s scopes and types.
//!
//! The checker gives every expression a type and every name its meaning. Types that are not known
//! where they appear (the elements of `[]`) are type variables, filled in by later uses through
//! unification; an expression with an error gets a fresh variable too, so one mistake does not cause
//! a cascade of mismatches.

use std::collections::HashMap;

use a0_diag::{Diagnostic, Diagnostics};
use a0_sema::{ScopeKind, Scopes, TypeId, TypeTag, Types};
use a0_syntax::{NodeId, SyntaxTree};

/// Functions every program can call.
pub const BUILTINS: &[&str] = &["print", "len", "push", "pop", "str"];

/// What a name refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Name {
    Local(usize),
    Func(usize),
    Builtin(&'static str),
}

pub struct Local {
    pub name: String,
    pub ty: TypeId,
}

pub struct Func {
    pub node: NodeId,
    pub name: String,
    pub ty: TypeId,
    pub params: Vec<usize>,
}

pub struct Struct {
    pub name: String,
    pub ty: TypeId,
}

/// Everything code generation needs.
pub struct Checked {
    pub types: Types,
    pub int: TypeId,
    pub float: TypeId,
    pub bool: TypeId,
    pub string: TypeId,
    pub unit: TypeId,
    pub structs: Vec<Struct>,
    pub funcs: Vec<Func>,
    pub locals: Vec<Local>,
    /// `name_expr` nodes to what they name.
    pub names: HashMap<NodeId, Name>,
    /// `let_stmt`, `for_stmt` and `param` nodes to the locals they declare.
    pub decls: HashMap<NodeId, usize>,
    /// The type of every expression node, resolved.
    pub expr_types: HashMap<NodeId, TypeId>,
}

impl Checked {
    pub fn ty(&self, node: NodeId) -> TypeId {
        self.expr_types[&node]
    }
}

struct Checker<'a> {
    tree: &'a SyntaxTree,
    c: Checked,
    diags: Diagnostics,
    scopes: Scopes,
    struct_names: HashMap<String, usize>,
    func_names: HashMap<String, usize>,
    /// The result type of the function being checked, and how many loops enclose the statement.
    ret: TypeId,
    loops: u32,
}

/// The non-token children of a node.
pub fn statements(tree: &SyntaxTree, node: NodeId) -> Vec<NodeId> {
    let mut out = tree.children(node);
    out.retain(|&c| !tree.is_token(c));
    out
}

/// The children of `node` that are `kind` nodes (rules used without a field label).
pub fn children_of_kind(tree: &SyntaxTree, node: NodeId, kind: &str) -> Vec<NodeId> {
    let mut out = tree.children(node);
    out.retain(|&c| !tree.is_token(c) && tree.kind_name(c) == kind);
    out
}

/// Skips the wrapper nodes the grammar puts around alternatives (`primary`, `type`).
pub fn unwrap(tree: &SyntaxTree, mut node: NodeId) -> NodeId {
    while matches!(tree.kind_name(node), "primary" | "type") {
        node = tree.child(node, 0);
    }
    node
}

/// Checks a parsed program.
pub fn check(tree: &SyntaxTree) -> Result<Checked, Diagnostics> {
    let mut types = Types::new();
    let c = Checked {
        int: types.named("int"),
        float: types.named("float"),
        bool: types.named("bool"),
        string: types.named("string"),
        unit: types.named("unit"),
        types,
        structs: Vec::new(),
        funcs: Vec::new(),
        locals: Vec::new(),
        names: HashMap::new(),
        decls: HashMap::new(),
        expr_types: HashMap::new(),
    };
    let unit = c.unit;
    let mut k = Checker {
        tree,
        c,
        diags: Diagnostics::new(),
        scopes: Scopes::new(),
        struct_names: HashMap::new(),
        func_names: HashMap::new(),
        ret: unit,
        loops: 0,
    };
    k.program();
    if k.diags.has_errors() {
        Err(k.diags)
    } else {
        Ok(k.c)
    }
}

impl<'a> Checker<'a> {
    fn error(&mut self, node: NodeId, message: &str) {
        let span = self.tree.span(node);
        self.diags
            .push(Diagnostic::error("minnow.check", message).with_span(span));
    }

    fn field(&self, node: NodeId, name: &str) -> Option<NodeId> {
        self.tree.field_named(node, name)
    }

    fn text(&self, node: NodeId) -> &'a str {
        self.tree.text(node)
    }

    fn display(&self, t: TypeId) -> String {
        self.c.types.display(t)
    }

    fn expect(&mut self, found: TypeId, expected: TypeId, node: NodeId) -> bool {
        let span = self.tree.span(node);
        self.c
            .types
            .expect(found, expected, Some(span), &mut self.diags)
    }

    /// Whether `t` is one of the named types in `names`.
    fn is(&self, t: TypeId, names: &[&str]) -> bool {
        self.c.types.tag(t) == TypeTag::Named && names.contains(&self.c.types.name(t).unwrap_or(""))
    }

    // ---------------------------------------------------------------- items

    fn program(&mut self) {
        let tree = self.tree;
        let items = statements(tree, tree.root());
        // Structs first, so fields and signatures can name them in any order.
        for &item in &items {
            if tree.kind_name(item) == "struct_decl" {
                let name_node = self.field(item, "name").unwrap();
                let name = self.text(name_node).to_string();
                if self.struct_names.contains_key(&name) || self.is_primitive(&name) {
                    self.error(name_node, &format!("the type `{name}` is defined twice"));
                    continue;
                }
                let ty = self.c.types.declare_struct(&name);
                self.struct_names.insert(name.clone(), self.c.structs.len());
                self.c.structs.push(Struct { name, ty });
            }
        }
        for &item in &items {
            if tree.kind_name(item) != "struct_decl" {
                continue;
            }
            let name = self.text(self.field(item, "name").unwrap()).to_string();
            let Some(&s) = self.struct_names.get(&name) else {
                continue;
            };
            let ty = self.c.structs[s].ty;
            for f in children_of_kind(tree, item, "field") {
                let fname = self.field(f, "name").unwrap();
                let fty = self.ty(self.field(f, "type").unwrap());
                if self.c.types.add_field(ty, self.text(fname), fty).is_none() {
                    let message = format!("`{name}` has two fields named `{}`", self.text(fname));
                    self.error(fname, &message);
                }
            }
        }
        for &item in &items {
            if tree.kind_name(item) == "fn_decl" {
                self.declare_function(item);
            }
        }
        for f in 0..self.c.funcs.len() {
            let node = self.c.funcs[f].node;
            let ty = self.c.funcs[f].ty;
            let params = self.c.types.params(ty).to_vec();
            let ret = self.c.types.ret(ty).unwrap();
            let name = self.field(node, "name").unwrap();
            self.c.funcs[f].params = self.function(node, &params, ret, name);
        }
        match self.func_names.get("main") {
            Some(&f) => {
                let ty = self.c.funcs[f].ty;
                let ret = self.c.types.ret(ty).unwrap();
                if !self.c.types.params(ty).is_empty() || !self.is(ret, &["unit", "int"]) {
                    let name = self.field(self.c.funcs[f].node, "name").unwrap();
                    self.error(
                        name,
                        "`main` must take no parameters and return nothing or `int`",
                    );
                }
            }
            None => self.diags.push(Diagnostic::error(
                "minnow.check",
                "the program has no `main` function",
            )),
        }
        // Empty arrays whose element type nothing determined.
        if !self.diags.has_errors() {
            let mut unknown: Vec<NodeId> = self
                .c
                .expr_types
                .iter()
                .filter(|&(&n, &t)| {
                    self.tree.kind_name(n) == "array_lit" && !self.c.types.is_complete(t)
                })
                .map(|(&n, _)| n)
                .collect();
            unknown.sort();
            for n in unknown {
                self.error(n, "cannot infer the element type of this array; annotate the variable, as in `let xs: [int] = [];`");
            }
        }
        let resolved: Vec<(NodeId, TypeId)> = self.c.expr_types.clone().into_iter().collect();
        for (n, t) in resolved {
            let t = self.c.types.resolve(t);
            self.c.expr_types.insert(n, t);
        }
        for l in 0..self.c.locals.len() {
            self.c.locals[l].ty = self.c.types.resolve(self.c.locals[l].ty);
        }
    }

    fn is_primitive(&self, name: &str) -> bool {
        matches!(name, "int" | "float" | "bool" | "string")
    }

    fn declare_function(&mut self, item: NodeId) {
        let name_node = self.field(item, "name").unwrap();
        let name = self.text(name_node).to_string();
        let params: Vec<TypeId> = children_of_kind(self.tree, item, "param")
            .into_iter()
            .map(|p| self.ty(self.field(p, "type").unwrap()))
            .collect();
        let ret = match self.field(item, "ret") {
            Some(r) => self.ty(r),
            None => self.c.unit,
        };
        let ty = self.c.types.function(&params, ret);
        if self.func_names.contains_key(&name) || BUILTINS.contains(&name.as_str()) {
            self.error(
                name_node,
                &format!("the function `{name}` is defined twice"),
            );
            return;
        }
        self.func_names.insert(name.clone(), self.c.funcs.len());
        self.c.funcs.push(Func {
            node: item,
            name,
            ty,
            params: Vec::new(),
        });
    }

    /// A type written in the source.
    fn ty(&mut self, node: NodeId) -> TypeId {
        let node = unwrap(self.tree, node);
        match self.tree.kind_name(node) {
            "array_type" => {
                let elem = self.ty(self.field(node, "elem").unwrap());
                self.c.types.array(elem)
            }
            "fn_type" => {
                let params: Vec<TypeId> = self
                    .tree
                    .fields_named(node, "param_type")
                    .into_iter()
                    .map(|p| self.ty(p))
                    .collect();
                let ret = match self.field(node, "ret") {
                    Some(r) => self.ty(r),
                    None => self.c.unit,
                };
                self.c.types.function(&params, ret)
            }
            _ => {
                let name_node = self.field(node, "name").unwrap();
                let name = self.text(name_node);
                if self.is_primitive(name) {
                    return self.c.types.named(name);
                }
                match self.struct_names.get(name) {
                    Some(&s) => self.c.structs[s].ty,
                    None => {
                        let message = format!("unknown type `{name}`");
                        self.error(name_node, &message);
                        self.c.types.fresh_var()
                    }
                }
            }
        }
    }

    /// Checks a function or closure body and returns its parameters' locals.
    fn function(&mut self, node: NodeId, params: &[TypeId], ret: TypeId, at: NodeId) -> Vec<usize> {
        let saved = (self.ret, self.loops);
        (self.ret, self.loops) = (ret, 0);
        self.scopes.push(ScopeKind::Function);
        let locals = children_of_kind(self.tree, node, "param")
            .into_iter()
            .zip(params)
            .map(|(p, &t)| self.declare(self.field(p, "name").unwrap(), t, p))
            .collect();
        let body = self.field(node, "body").unwrap();
        for s in statements(self.tree, body) {
            self.stmt(s);
        }
        if !self.c.types.same(ret, self.c.unit) && !always_returns(self.tree, body) {
            let message = format!(
                "this function can end without returning a value of type `{}`",
                self.display(ret)
            );
            self.error(at, &message);
        }
        self.scopes.pop();
        (self.ret, self.loops) = saved;
        locals
    }

    fn declare(&mut self, name: NodeId, ty: TypeId, decl: NodeId) -> usize {
        let l = self.c.locals.len();
        let text = self.text(name).to_string();
        if self.scopes.declare_defined(&text, l as u64).is_err() {
            self.error(name, &format!("`{text}` is already defined in this scope"));
        }
        self.c.locals.push(Local { name: text, ty });
        self.c.decls.insert(decl, l);
        l
    }

    // ---------------------------------------------------------------- statements

    fn block(&mut self, node: NodeId) {
        self.scopes.push(ScopeKind::Block);
        for s in statements(self.tree, node) {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, node: NodeId) {
        let tree = self.tree;
        match tree.kind_name(node) {
            "let_stmt" => {
                let value = self.field(node, "value").unwrap();
                let mut t = self.expr(value);
                if let Some(ty) = self.field(node, "type") {
                    let declared = self.ty(ty);
                    self.expect(t, declared, value);
                    t = declared;
                }
                self.declare(self.field(node, "name").unwrap(), t, node);
            }
            "if_stmt" => {
                self.condition(self.field(node, "cond").unwrap());
                self.block(self.field(node, "then").unwrap());
                if let Some(e) = self.field(node, "else") {
                    if tree.kind_name(e) == "if_stmt" {
                        self.stmt(e);
                    } else {
                        self.block(e);
                    }
                }
            }
            "while_stmt" => {
                self.condition(self.field(node, "cond").unwrap());
                self.loop_body(self.field(node, "body").unwrap(), None);
            }
            "for_stmt" => {
                let iterable = self.field(node, "iterable").unwrap();
                let t = self.expr(iterable);
                let elem = self.c.types.fresh_var();
                let array = self.c.types.array(elem);
                self.expect(t, array, iterable);
                self.loop_body(self.field(node, "body").unwrap(), Some((node, elem)));
            }
            "return_stmt" => match self.field(node, "value") {
                Some(v) => {
                    let t = self.expr(v);
                    self.expect(t, self.ret, v);
                }
                None if !self.c.types.same(self.ret, self.c.unit) => {
                    let message = format!(
                        "this function must return a value of type `{}`",
                        self.display(self.ret)
                    );
                    self.error(node, &message);
                }
                None => {}
            },
            "break_stmt" | "continue_stmt" if self.loops == 0 => {
                self.error(
                    node,
                    "`break` and `continue` can only be used inside a loop",
                );
            }
            "block" => self.block(node),
            "expr_stmt" => {
                let target = self.field(node, "expr").unwrap();
                let t = self.expr(target);
                if let Some(value) = self.field(node, "value") {
                    if !self.is_place(target) {
                        self.error(target, "cannot assign to this expression");
                    }
                    let v = self.expr(value);
                    self.expect(v, t, value);
                }
            }
            _ => {}
        }
    }

    /// A loop body, with the loop variable of a `for` in its own scope.
    fn loop_body(&mut self, body: NodeId, var: Option<(NodeId, TypeId)>) {
        self.loops += 1;
        self.scopes.push(ScopeKind::Block);
        if let Some((node, elem)) = var {
            self.declare(self.field(node, "name").unwrap(), elem, node);
        }
        self.block(body);
        self.scopes.pop();
        self.loops -= 1;
    }

    fn condition(&mut self, node: NodeId) {
        let t = self.expr(node);
        self.expect(t, self.c.bool, node);
    }

    /// Whether an expression can be assigned to: a local variable, a field or an element.
    fn is_place(&self, node: NodeId) -> bool {
        let node = unwrap(self.tree, node);
        match self.tree.kind_name(node) {
            "name_expr" => matches!(self.c.names.get(&node), Some(Name::Local(_))),
            "get" | "index" => true,
            _ => false,
        }
    }

    // ---------------------------------------------------------------- expressions

    fn expr(&mut self, node: NodeId) -> TypeId {
        let inner = unwrap(self.tree, node);
        let t = self.expr_inner(inner);
        self.c.expr_types.insert(inner, t);
        self.c.expr_types.insert(node, t);
        t
    }

    fn expr_inner(&mut self, node: NodeId) -> TypeId {
        let tree = self.tree;
        match tree.kind_name(node) {
            "literal" => {
                let tok = tree.child(node, 0);
                match tree.kind_name(tok) {
                    "INT" => {
                        if self.text(tok).parse::<i64>().is_err() {
                            self.error(tok, "this integer does not fit in 64 bits");
                        }
                        self.c.int
                    }
                    "FLOAT" => self.c.float,
                    "STRING" => self.c.string,
                    _ => self.c.bool,
                }
            }
            "name_expr" => {
                let name = self.text(tree.child(node, 0)).to_string();
                match self.lookup(&name) {
                    Some(n @ Name::Local(l)) => {
                        self.c.names.insert(node, n);
                        self.c.locals[l].ty
                    }
                    Some(n @ Name::Func(f)) => {
                        self.c.names.insert(node, n);
                        self.c.funcs[f].ty
                    }
                    Some(Name::Builtin(_)) => {
                        let message = format!("the builtin `{name}` can only be called");
                        self.error(node, &message);
                        self.c.types.fresh_var()
                    }
                    None => {
                        self.error(node, &format!("unknown name `{name}`"));
                        self.c.types.fresh_var()
                    }
                }
            }
            "paren" => self.expr(self.field(node, "inner").unwrap()),
            "array_lit" => {
                let elem = self.c.types.fresh_var();
                if let Some(args) = self.field(node, "args") {
                    for a in tree.fields_named(args, "arg") {
                        let t = self.expr(a);
                        self.expect(t, elem, a);
                    }
                }
                self.c.types.array(elem)
            }
            "new_expr" => self.new_expr(node),
            "closure" => {
                let params: Vec<TypeId> = children_of_kind(tree, node, "param")
                    .into_iter()
                    .map(|p| self.ty(self.field(p, "type").unwrap()))
                    .collect();
                let ret = match self.field(node, "ret") {
                    Some(r) => self.ty(r),
                    None => self.c.unit,
                };
                self.function(node, &params, ret, tree.child(node, 0));
                self.c.types.function(&params, ret)
            }
            "binary" => self.binary(node),
            "logical" => {
                for side in ["left", "right"] {
                    let e = self.field(node, side).unwrap();
                    self.condition(e);
                }
                self.c.bool
            }
            "unary" => {
                let op = self.text(self.field(node, "op").unwrap()).to_string();
                let operand = self.field(node, "operand").unwrap();
                let t = self.expr(operand);
                if self.unknown(t, operand) {
                    return t;
                }
                let ok = if op == "!" {
                    self.is(t, &["bool"])
                } else {
                    self.is(t, &["int", "float"])
                };
                if !ok {
                    return self.bad_operand(node, &op, t);
                }
                t
            }
            "cast" => {
                let from = self.expr(self.field(node, "operand").unwrap());
                let to = self.ty(self.field(node, "type").unwrap());
                if self.unknown(from, node) {
                    return to;
                }
                let numeric = self.is(from, &["int", "float"]) && self.is(to, &["int", "float"]);
                if !numeric && !self.c.types.same(from, to) {
                    let message = format!(
                        "cannot cast `{}` to `{}`",
                        self.display(from),
                        self.display(to)
                    );
                    self.error(node, &message);
                }
                to
            }
            "call" => self.call(node),
            "get" => {
                let object = self.field(node, "operand").unwrap();
                let t = self.expr(object);
                let name = self.field(node, "name").unwrap();
                if self.unknown(t, object) {
                    return self.c.types.fresh_var();
                }
                match self.c.types.field_index(t, self.text(name)) {
                    Some(i) => self.c.types.field_type(t, i),
                    None => {
                        let message = if self.c.types.tag(t) == TypeTag::Struct {
                            format!("`{}` has no field `{}`", self.display(t), self.text(name))
                        } else {
                            format!("`{}` is not a struct, so it has no fields", self.display(t))
                        };
                        self.error(name, &message);
                        self.c.types.fresh_var()
                    }
                }
            }
            "index" => {
                let target = self.field(node, "operand").unwrap();
                let t = self.expr(target);
                let elem = self.c.types.fresh_var();
                let array = self.c.types.array(elem);
                self.expect(t, array, target);
                let index = self.field(node, "index").unwrap();
                let i = self.expr(index);
                self.expect(i, self.c.int, index);
                elem
            }
            _ => self.c.types.fresh_var(),
        }
    }

    fn lookup(&self, name: &str) -> Option<Name> {
        if let Some(r) = self.scopes.lookup(name) {
            return Some(Name::Local(self.scopes.data(r.symbol) as usize));
        }
        if let Some(&f) = self.func_names.get(name) {
            return Some(Name::Func(f));
        }
        BUILTINS
            .iter()
            .find(|&&b| b == name)
            .map(|&b| Name::Builtin(b))
    }

    /// Whether `t` is still unknown where an operation needs it. After an earlier error, an unknown
    /// type is that error's doing and is not reported again.
    fn unknown(&mut self, t: TypeId, node: NodeId) -> bool {
        if self.c.types.tag(t) != TypeTag::Var {
            return false;
        }
        if !self.diags.has_errors() {
            self.error(
                node,
                "the type of this value must be known here; add a type annotation",
            );
        }
        true
    }

    fn bad_operand(&mut self, node: NodeId, op: &str, t: TypeId) -> TypeId {
        let message = format!("operator `{op}` cannot be applied to `{}`", self.display(t));
        self.error(node, &message);
        self.c.types.fresh_var()
    }

    fn binary(&mut self, node: NodeId) -> TypeId {
        let op = self.text(self.field(node, "op").unwrap()).to_string();
        let left = self.field(node, "left").unwrap();
        let right = self.field(node, "right").unwrap();
        let lt = self.expr(left);
        let rt = self.expr(right);
        self.expect(rt, lt, right);
        let allowed: &[&str] = match op.as_str() {
            "+" => &["int", "float", "string"],
            "-" | "*" | "/" => &["int", "float"],
            "%" => &["int"],
            "<" | "<=" | ">" | ">=" => &["int", "float", "string"],
            _ => return self.c.bool,
        };
        if self.unknown(lt, left) {
            return self.c.types.fresh_var();
        }
        if !self.is(lt, allowed) {
            return self.bad_operand(node, &op, lt);
        }
        if matches!(op.as_str(), "<" | "<=" | ">" | ">=") {
            self.c.bool
        } else {
            lt
        }
    }

    fn new_expr(&mut self, node: NodeId) -> TypeId {
        let name_node = self.field(node, "name").unwrap();
        let name = self.text(name_node).to_string();
        let inits = children_of_kind(self.tree, node, "field_init");
        let Some(&s) = self.struct_names.get(&name) else {
            self.error(name_node, &format!("unknown struct `{name}`"));
            for init in inits {
                self.expr(self.field(init, "value").unwrap());
            }
            return self.c.types.fresh_var();
        };
        let ty = self.c.structs[s].ty;
        let mut seen = vec![false; self.c.types.field_count(ty) as usize];
        for init in inits {
            let fname = self.field(init, "name").unwrap();
            let value = self.field(init, "value").unwrap();
            let t = self.expr(value);
            match self.c.types.field_index(ty, self.text(fname)) {
                Some(i) if seen[i as usize] => {
                    let message = format!("the field `{}` is given twice", self.text(fname));
                    self.error(fname, &message);
                }
                Some(i) => {
                    seen[i as usize] = true;
                    let ft = self.c.types.field_type(ty, i);
                    self.expect(t, ft, value);
                }
                None => {
                    let message = format!("`{name}` has no field `{}`", self.text(fname));
                    self.error(fname, &message);
                }
            }
        }
        let missing: Vec<String> = (0..seen.len())
            .filter(|&i| !seen[i])
            .map(|i| format!("`{}`", self.c.types.field_name(ty, i as u32)))
            .collect();
        if !missing.is_empty() {
            let message = format!("missing fields in `{name}`: {}", missing.join(", "));
            self.error(name_node, &message);
        }
        ty
    }

    fn call(&mut self, node: NodeId) -> TypeId {
        let tree = self.tree;
        let callee = self.field(node, "operand").unwrap();
        let args = match self.field(node, "args") {
            Some(a) => tree.fields_named(a, "arg"),
            None => Vec::new(),
        };
        let inner = unwrap(tree, callee);
        if tree.kind_name(inner) == "name_expr"
            && let Some(Name::Builtin(b)) = self.lookup(self.text(tree.child(inner, 0)))
        {
            self.c.names.insert(inner, Name::Builtin(b));
            return self.builtin(node, b, &args);
        }
        let t = self.expr(callee);
        let arg_types: Vec<TypeId> = args.iter().map(|&a| self.expr(a)).collect();
        if self.unknown(t, callee) {
            return self.c.types.fresh_var();
        }
        if self.c.types.tag(t) != TypeTag::Function {
            let message = format!("`{}` is not a function", self.display(t));
            self.error(callee, &message);
            return self.c.types.fresh_var();
        }
        let params = self.c.types.params(t).to_vec();
        if params.len() != args.len() {
            let message = format!(
                "expected {} argument{}, found {}",
                params.len(),
                if params.len() == 1 { "" } else { "s" },
                args.len()
            );
            self.error(node, &message);
        } else {
            for ((&a, &at), &p) in args.iter().zip(&arg_types).zip(&params) {
                self.expect(at, p, a);
            }
        }
        self.c.types.ret(t).unwrap()
    }

    fn builtin(&mut self, node: NodeId, b: &str, args: &[NodeId]) -> TypeId {
        let arg_types: Vec<TypeId> = args.iter().map(|&a| self.expr(a)).collect();
        let arity = if b == "push" { 2 } else { 1 };
        if args.len() != arity {
            let message = format!(
                "`{b}` takes {arity} argument{}, found {}",
                if arity == 1 { "" } else { "s" },
                args.len()
            );
            self.error(node, &message);
            return self.c.types.fresh_var();
        }
        let (a, t) = (args[0], arg_types[0]);
        if matches!(b, "print" | "str" | "len") && self.unknown(t, a) {
            return if b == "str" {
                self.c.string
            } else if b == "len" {
                self.c.int
            } else {
                self.c.unit
            };
        }
        match b {
            "print" | "str" => {
                let printable: &[&str] = if b == "print" {
                    &["int", "float", "bool", "string"]
                } else {
                    &["int", "float", "bool"]
                };
                if !self.is(t, printable) {
                    let message = format!("`{b}` does not accept `{}`", self.display(t));
                    self.error(a, &message);
                }
                if b == "print" {
                    self.c.unit
                } else {
                    self.c.string
                }
            }
            "len" => {
                if !self.is(t, &["string"]) && self.c.types.tag(t) != TypeTag::Array {
                    let message = format!("`len` does not accept `{}`", self.display(t));
                    self.error(a, &message);
                }
                self.c.int
            }
            _ => {
                // push(array, value) and pop(array)
                let elem = self.c.types.fresh_var();
                let array = self.c.types.array(elem);
                self.expect(t, array, a);
                if b == "push" {
                    self.expect(arg_types[1], elem, args[1]);
                    self.c.unit
                } else {
                    elem
                }
            }
        }
    }
}

/// Whether every path through a statement ends in `return`.
pub fn always_returns(tree: &SyntaxTree, node: NodeId) -> bool {
    match tree.kind_name(node) {
        "return_stmt" => true,
        "block" => statements(tree, node)
            .into_iter()
            .any(|s| always_returns(tree, s)),
        "if_stmt" => {
            let then = tree.field_named(node, "then").unwrap();
            tree.field_named(node, "else")
                .is_some_and(|e| always_returns(tree, then) && always_returns(tree, e))
        }
        _ => false,
    }
}
