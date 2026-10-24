//! Static analysis of a Lox syntax tree: jlox's parse-time checks and its resolver.
//!
//! Besides reporting jlox's errors, resolution records where every variable lives: a local
//! declaration, or a global looked up by name at run time.

use std::collections::HashMap;

use a0_sema::{ScopeKind, Scopes};
use a0_syntax::{FieldId, Grammar, KindId, NodeId, SyntaxTree};

use crate::{CompileError, Lines};

pub type DeclId = usize;

/// Field and kind ids of the Lox grammar.
#[derive(Clone, Copy)]
pub struct Names {
    pub name: FieldId,
    pub superclass: FieldId,
    pub methods: FieldId,
    pub params: FieldId,
    pub param: FieldId,
    pub body: FieldId,
    pub init: FieldId,
    pub expr: FieldId,
    pub cond: FieldId,
    pub incr: FieldId,
    pub then: FieldId,
    pub else_: FieldId,
    pub value: FieldId,
    pub left: FieldId,
    pub op: FieldId,
    pub right: FieldId,
    pub operand: FieldId,
    pub args: FieldId,
    pub arg: FieldId,
    pub inner: FieldId,
    pub method: FieldId,
    pub class_decl: KindId,
    pub fun_decl: KindId,
    pub method_kind: KindId,
    pub var_decl: KindId,
    pub expr_stmt: KindId,
    pub for_stmt: KindId,
    pub if_stmt: KindId,
    pub print_stmt: KindId,
    pub return_stmt: KindId,
    pub while_stmt: KindId,
    pub block: KindId,
    pub assign: KindId,
    pub logical: KindId,
    pub binary: KindId,
    pub unary: KindId,
    pub call: KindId,
    pub get: KindId,
    pub primary: KindId,
    pub identifier: KindId,
    pub number: KindId,
    pub string: KindId,
    pub kw_true: KindId,
    pub kw_false: KindId,
    pub kw_nil: KindId,
    pub kw_this: KindId,
    pub kw_super: KindId,
    pub kw_and: KindId,
    pub kw_or: KindId,
}

impl Names {
    pub fn new(g: &Grammar) -> Names {
        let f = |n: &str| {
            g.field(n)
                .unwrap_or_else(|| panic!("lox.a0g has no field {n}"))
        };
        let k = |n: &str| {
            g.kind(n)
                .unwrap_or_else(|| panic!("lox.a0g has no kind {n}"))
        };
        let l = |n: &str| {
            g.literal(n)
                .unwrap_or_else(|| panic!("lox.a0g has no literal {n}"))
        };
        Names {
            name: f("name"),
            superclass: f("superclass"),
            methods: f("methods"),
            params: f("params"),
            param: f("param"),
            body: f("body"),
            init: f("init"),
            expr: f("expr"),
            cond: f("cond"),
            incr: f("incr"),
            then: f("then"),
            else_: f("else"),
            value: f("value"),
            left: f("left"),
            op: f("op"),
            right: f("right"),
            operand: f("operand"),
            args: f("args"),
            arg: f("arg"),
            inner: f("inner"),
            method: f("method"),
            class_decl: k("class_decl"),
            fun_decl: k("fun_decl"),
            method_kind: k("method"),
            var_decl: k("var_decl"),
            expr_stmt: k("expr_stmt"),
            for_stmt: k("for_stmt"),
            if_stmt: k("if_stmt"),
            print_stmt: k("print_stmt"),
            return_stmt: k("return_stmt"),
            while_stmt: k("while_stmt"),
            block: k("block"),
            assign: k("assign"),
            logical: k("logical"),
            binary: k("binary"),
            unary: k("unary"),
            call: k("call"),
            get: k("get"),
            primary: k("primary"),
            identifier: k("IDENTIFIER"),
            number: k("NUMBER"),
            string: k("STRING"),
            kw_true: l("true"),
            kw_false: l("false"),
            kw_nil: l("nil"),
            kw_this: l("this"),
            kw_super: l("super"),
            kw_and: l("and"),
            kw_or: l("or"),
        }
    }
}

/// The leftmost token under `node`.
pub fn first_token(tree: &SyntaxTree, mut node: NodeId) -> NodeId {
    while !tree.is_token(node) && tree.child_count(node) > 0 {
        node = tree.child(node, 0);
    }
    node
}

/// The non-token children of a node (statements in a block, declarations in the program).
pub fn statements(tree: &SyntaxTree, node: NodeId) -> Vec<NodeId> {
    tree.children(node)
        .into_iter()
        .filter(|&c| !tree.is_token(c))
        .collect()
}

/// jlox reports these while parsing: invalid assignment targets and more than 255 arguments or
/// parameters.
pub fn parse_time_checks(tree: &SyntaxTree, n: &Names, lines: &Lines) -> Vec<CompileError> {
    let mut errors = Vec::new();
    let mut stack = vec![tree.root()];
    while let Some(node) = stack.pop() {
        if tree.is_token(node) {
            continue;
        }
        let kind = tree.kind(node);
        if kind == n.assign {
            let left = tree.field(node, n.left).unwrap();
            if !is_variable(tree, n, left) && tree.kind(left) != n.get {
                let op = tree.field(node, n.op).unwrap();
                errors.push(lines.error(tree, op, "Invalid assignment target."));
            }
        } else if kind == n.call {
            if let Some(args) = tree.field(node, n.args) {
                for &arg in tree.fields(args, n.arg).iter().skip(255) {
                    let tok = first_token(tree, arg);
                    errors.push(lines.error(tree, tok, "Can't have more than 255 arguments."));
                }
            }
        } else if (kind == n.fun_decl || kind == n.method_kind)
            && let Some(params) = tree.field(node, n.params)
        {
            for &p in tree.fields(params, n.param).iter().skip(255) {
                errors.push(lines.error(tree, p, "Can't have more than 255 parameters."));
            }
        }
        for c in tree.children(node).into_iter().rev() {
            stack.push(c);
        }
    }
    errors
}

/// Whether `node` is a plain variable reference (`primary` holding an IDENTIFIER).
pub fn is_variable(tree: &SyntaxTree, n: &Names, node: NodeId) -> bool {
    tree.kind(node) == n.primary
        && tree.child_count(node) == 1
        && tree.kind(tree.child(node, 0)) == n.identifier
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FnKind {
    Script,
    Function,
    Method,
    Initializer,
}

/// Where a variable lives: a local declaration, or a global looked up by name at run time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Local(DeclId),
    Global,
}

/// What code generation needs to know about names. Nothing about closures: the HIR captures
/// whatever outer locals a function uses.
#[derive(Default)]
pub struct Analysis {
    /// Variable uses (`primary` nodes for names, `this` and `super`, `assign` nodes, and superclass
    /// name tokens) to places.
    pub uses: HashMap<NodeId, Place>,
    /// Declarations (`var_decl`, `fun_decl` and `class_decl` nodes, parameter tokens) to places.
    pub defs: HashMap<NodeId, Place>,
    /// Methods to their `this`.
    pub this_decls: HashMap<NodeId, DeclId>,
    /// `class_decl` nodes with a superclass to their `super` variable.
    pub super_decls: HashMap<NodeId, DeclId>,
    /// `super` expressions to the method's `this`.
    pub super_this: HashMap<NodeId, DeclId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClassKind {
    None,
    Class,
    Subclass,
}

struct Resolver<'a> {
    tree: &'a SyntaxTree,
    n: Names,
    lines: &'a Lines<'a>,
    scopes: Scopes,
    errors: Vec<CompileError>,
    a: Analysis,
    decls: DeclId,
    functions: Vec<FnKind>,
    class: ClassKind,
}

/// jlox's resolver. Returns the analysis, or the resolution errors.
pub fn resolve(tree: &SyntaxTree, n: &Names, lines: &Lines) -> Result<Analysis, Vec<CompileError>> {
    let mut r = Resolver {
        tree,
        n: *n,
        lines,
        scopes: Scopes::new(),
        errors: Vec::new(),
        a: Analysis::default(),
        decls: 0,
        functions: vec![FnKind::Script],
        class: ClassKind::None,
    };
    for stmt in statements(tree, tree.root()) {
        r.stmt(stmt);
    }
    if r.errors.is_empty() {
        Ok(r.a)
    } else {
        Err(r.errors)
    }
}

impl Resolver<'_> {
    fn error(&mut self, token: NodeId, message: &str) {
        let e = self.lines.error(self.tree, token, message);
        self.errors.push(e);
    }

    /// Declares `token` in the innermost scope, or as a global at the top level. Locals start out
    /// declared but not defined, unless `defined`.
    fn declare(&mut self, token: NodeId, defined: bool) -> Place {
        if self.scopes.depth() == 0 {
            return Place::Global;
        }
        let d = self.declare_name(self.tree.text(token), defined);
        if d.is_none() {
            self.error(token, "Already a variable with this name in this scope.");
        }
        Place::Local(d.unwrap_or(0))
    }

    fn declare_name(&mut self, name: &str, defined: bool) -> Option<DeclId> {
        let d = self.decls;
        self.decls += 1;
        let declared = if defined {
            self.scopes.declare_defined(name, d as u64)
        } else {
            self.scopes.declare(name, d as u64)
        };
        declared.ok().map(|_| d)
    }

    fn define(&mut self, token: NodeId) {
        if let Some(s) = self.scopes.lookup_in_current(self.tree.text(token)) {
            self.scopes.define(s);
        }
    }

    fn resolve_name(&mut self, name: &str) -> Place {
        match self.scopes.resolve(name) {
            Some(r) => Place::Local(self.scopes.data(r.symbol) as DeclId),
            None => Place::Global,
        }
    }

    fn stmt(&mut self, node: NodeId) {
        let n = self.n;
        let tree = self.tree;
        let kind = tree.kind(node);
        if kind == n.var_decl {
            let name = tree.field(node, n.name).unwrap();
            let place = self.declare(name, false);
            self.a.defs.insert(node, place);
            if let Some(init) = tree.field(node, n.init) {
                self.expr(init);
            }
            self.define(name);
        } else if kind == n.fun_decl {
            let place = self.declare(tree.field(node, n.name).unwrap(), true);
            self.a.defs.insert(node, place);
            self.function(node, FnKind::Function);
        } else if kind == n.class_decl {
            self.class_decl(node);
        } else if kind == n.print_stmt {
            self.expr(tree.field(node, n.value).unwrap());
        } else if kind == n.expr_stmt {
            self.expr(tree.field(node, n.expr).unwrap());
        } else if kind == n.block {
            self.scopes.push(ScopeKind::Block);
            for s in statements(tree, node) {
                self.stmt(s);
            }
            self.scopes.pop();
        } else if kind == n.if_stmt {
            self.expr(tree.field(node, n.cond).unwrap());
            self.stmt(tree.field(node, n.then).unwrap());
            if let Some(e) = tree.field(node, n.else_) {
                self.stmt(e);
            }
        } else if kind == n.while_stmt {
            self.expr(tree.field(node, n.cond).unwrap());
            self.stmt(tree.field(node, n.body).unwrap());
        } else if kind == n.for_stmt {
            // jlox desugars `for` into a block holding the initializer and a while loop.
            self.scopes.push(ScopeKind::Block);
            for f in [n.init, n.cond, n.body, n.incr] {
                match tree.field(node, f) {
                    Some(s) if f == n.init || f == n.body => self.stmt(s),
                    Some(e) => self.expr(e),
                    None => {}
                }
            }
            self.scopes.pop();
        } else if kind == n.return_stmt {
            let keyword = tree.child(node, 0);
            let fn_kind = *self.functions.last().unwrap();
            if fn_kind == FnKind::Script {
                self.error(keyword, "Can't return from top-level code.");
            }
            if let Some(value) = tree.field(node, n.value) {
                if fn_kind == FnKind::Initializer {
                    self.error(keyword, "Can't return a value from an initializer.");
                }
                self.expr(value);
            }
        }
    }

    fn function(&mut self, node: NodeId, kind: FnKind) {
        let n = self.n;
        let tree = self.tree;
        self.functions.push(kind);
        self.scopes.push(ScopeKind::Function);
        if kind != FnKind::Function {
            let this = self.declare_name("this", true).unwrap();
            self.a.this_decls.insert(node, this);
        }
        if let Some(params) = tree.field(node, n.params) {
            for p in tree.fields(params, n.param) {
                let place = self.declare(p, true);
                self.a.defs.insert(p, place);
            }
        }
        // The body's statements live in the function's own scope, as in jlox.
        for s in statements(tree, tree.field(node, n.body).unwrap()) {
            self.stmt(s);
        }
        self.scopes.pop();
        self.functions.pop();
    }

    fn class_decl(&mut self, node: NodeId) {
        let n = self.n;
        let tree = self.tree;
        let enclosing = self.class;
        self.class = ClassKind::Class;
        let name = tree.field(node, n.name).unwrap();
        let place = self.declare(name, true);
        self.a.defs.insert(node, place);
        let superclass = tree.field(node, n.superclass);
        if let Some(sup) = superclass {
            if tree.text(sup) == tree.text(name) {
                self.error(sup, "A class can't inherit from itself.");
            }
            self.variable_use(sup, sup);
            self.class = ClassKind::Subclass;
            self.scopes.push(ScopeKind::Block);
            let d = self.declare_name("super", true).unwrap();
            self.a.super_decls.insert(node, d);
        }
        for m in tree.fields(node, n.methods) {
            let init = tree.text(tree.field(m, n.name).unwrap()) == "init";
            self.function(
                m,
                if init {
                    FnKind::Initializer
                } else {
                    FnKind::Method
                },
            );
        }
        if superclass.is_some() {
            self.scopes.pop();
        }
        self.class = enclosing;
    }

    /// A variable read: `token` is the name, `node` the expression node to annotate.
    fn variable_use(&mut self, token: NodeId, node: NodeId) {
        let name = self.tree.text(token);
        if let Some(s) = self.scopes.lookup_in_current(name)
            && !self.scopes.is_defined(s)
        {
            self.error(token, "Can't read local variable in its own initializer.");
        }
        let place = self.resolve_name(name);
        self.a.uses.insert(node, place);
    }

    fn expr(&mut self, node: NodeId) {
        let n = self.n;
        let tree = self.tree;
        let kind = tree.kind(node);
        if kind == n.primary {
            let first = tree.child(node, 0);
            let fk = tree.kind(first);
            if fk == n.identifier {
                self.variable_use(first, node);
            } else if fk == n.kw_this {
                if self.class == ClassKind::None {
                    self.error(first, "Can't use 'this' outside of a class.");
                    return;
                }
                let place = self.resolve_name("this");
                self.a.uses.insert(node, place);
            } else if fk == n.kw_super {
                if self.class == ClassKind::None {
                    self.error(first, "Can't use 'super' outside of a class.");
                    return;
                } else if self.class != ClassKind::Subclass {
                    self.error(first, "Can't use 'super' in a class with no superclass.");
                    return;
                }
                let place = self.resolve_name("super");
                self.a.uses.insert(node, place);
                if let Place::Local(this) = self.resolve_name("this") {
                    self.a.super_this.insert(node, this);
                }
            } else if let Some(inner) = tree.field(node, n.inner) {
                self.expr(inner);
            }
        } else if kind == n.assign {
            let left = tree.field(node, n.left).unwrap();
            self.expr(tree.field(node, n.right).unwrap());
            if is_variable(tree, &n, left) {
                let place = self.resolve_name(tree.text(tree.child(left, 0)));
                self.a.uses.insert(node, place);
            } else {
                // A property assignment: resolve the object.
                self.expr(tree.field(left, n.operand).unwrap());
            }
        } else if kind == n.logical || kind == n.binary {
            self.expr(tree.field(node, n.left).unwrap());
            self.expr(tree.field(node, n.right).unwrap());
        } else if kind == n.unary || kind == n.get {
            self.expr(tree.field(node, n.operand).unwrap());
        } else if kind == n.call {
            self.expr(tree.field(node, n.operand).unwrap());
            if let Some(args) = tree.field(node, n.args) {
                for a in tree.fields(args, n.arg) {
                    self.expr(a);
                }
            }
        }
    }
}
