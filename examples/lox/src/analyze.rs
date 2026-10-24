//! Static analysis of a Lox syntax tree: jlox's parse-time checks and its resolver.
//!
//! Besides reporting jlox's errors, resolution records where every variable lives (a local
//! declaration or a global), which locals are captured by closures, and for every function the list
//! of outer variables it captures (its upvalues). Code generation relies on all of this.

use std::collections::HashMap;

use a0_sema::{ScopeKind, Scopes};
use a0_syntax::{FieldId, Grammar, KindId, NodeId, SyntaxTree};

use crate::{CompileError, Lines};

pub type DeclId = usize;
pub type FnId = usize;

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
        let f = |n: &str| g.field(n).unwrap_or_else(|| panic!("lox.a0g has no field {n}"));
        let k = |n: &str| g.kind(n).unwrap_or_else(|| panic!("lox.a0g has no kind {n}"));
        let l = |n: &str| g.literal(n).unwrap_or_else(|| panic!("lox.a0g has no literal {n}"));
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
    tree.children(node).into_iter().filter(|&c| !tree.is_token(c)).collect()
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
        } else if kind == n.fun_decl || kind == n.method_kind {
            if let Some(params) = tree.field(node, n.params) {
                for &p in tree.fields(params, n.param).iter().skip(255) {
                    errors.push(lines.error(tree, p, "Can't have more than 255 parameters."));
                }
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
pub enum FnKind {
    Script,
    Function,
    Method,
    Initializer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Local(DeclId),
    Global(u32),
}

pub struct DeclInfo {
    pub name: String,
    pub func: FnId,
    pub captured: bool,
}

pub struct FnInfo {
    /// The `fun_decl` or `method` node (`None` for the script).
    pub node: Option<NodeId>,
    pub kind: FnKind,
    pub name: String,
    pub params: Vec<DeclId>,
    /// Outer variables this function captures, in closure cell order.
    pub upvalues: Vec<DeclId>,
    pub this_decl: Option<DeclId>,
}

/// Everything code generation needs to know about names.
pub struct Analysis {
    pub functions: Vec<FnInfo>,
    pub decls: Vec<DeclInfo>,
    /// Variable uses (`primary` nodes for names, `this` and `super`, and `assign` nodes) to places.
    pub uses: HashMap<NodeId, Place>,
    /// For `super` expressions: the method's `this`.
    pub super_this: HashMap<NodeId, DeclId>,
    /// Declarations (`var_decl`, `fun_decl`, `class_decl`) to places.
    pub defs: HashMap<NodeId, Place>,
    /// `class_decl` nodes with a superclass to their `super` variable.
    pub super_decls: HashMap<NodeId, DeclId>,
    /// `fun_decl` and `method` nodes to functions.
    pub fn_of: HashMap<NodeId, FnId>,
    pub globals: Vec<String>,
    pub clock_global: u32,
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
    global_index: HashMap<String, u32>,
    fn_stack: Vec<FnId>,
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
        a: Analysis {
            functions: vec![FnInfo {
                node: None,
                kind: FnKind::Script,
                name: "script".to_string(),
                params: Vec::new(),
                upvalues: Vec::new(),
                this_decl: None,
            }],
            decls: Vec::new(),
            uses: HashMap::new(),
            super_this: HashMap::new(),
            defs: HashMap::new(),
            super_decls: HashMap::new(),
            fn_of: HashMap::new(),
            globals: Vec::new(),
            clock_global: 0,
        },
        global_index: HashMap::new(),
        fn_stack: vec![0],
        class: ClassKind::None,
    };
    r.a.clock_global = r.global("clock");
    for stmt in statements(tree, tree.root()) {
        r.stmt(stmt);
    }
    if r.errors.is_empty() { Ok(r.a) } else { Err(r.errors) }
}

impl Resolver<'_> {
    fn error(&mut self, token: NodeId, message: &str) {
        let e = self.lines.error(self.tree, token, message);
        self.errors.push(e);
    }

    fn global(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.global_index.get(name) {
            return i;
        }
        let i = self.a.globals.len() as u32;
        self.a.globals.push(name.to_string());
        self.global_index.insert(name.to_string(), i);
        i
    }

    fn current_fn(&self) -> FnId {
        *self.fn_stack.last().unwrap()
    }

    fn new_decl(&mut self, name: &str) -> DeclId {
        self.a.decls.push(DeclInfo {
            name: name.to_string(),
            func: self.current_fn(),
            captured: false,
        });
        self.a.decls.len() - 1
    }

    /// Declares `name` (a token) in the innermost scope, or as a global at the top level. Locals start
    /// out declared but not defined.
    fn declare(&mut self, token: NodeId) -> Place {
        let name = self.tree.text(token);
        if self.scopes.depth() == 0 {
            return Place::Global(self.global(name));
        }
        let d = self.new_decl(name);
        if self.scopes.declare(name, d as u64).is_err() {
            self.error(token, "Already a variable with this name in this scope.");
        }
        Place::Local(d)
    }

    fn define(&mut self, token: NodeId) {
        if self.scopes.depth() == 0 {
            return;
        }
        if let Some(s) = self.scopes.lookup_in_current(self.tree.text(token)) {
            self.scopes.define(s);
        }
    }

    /// Declares a variable that is defined from the start (`this`, `super`, parameters).
    fn declare_defined(&mut self, name: &str) -> DeclId {
        let d = self.new_decl(name);
        let _ = self.scopes.declare_defined(name, d as u64);
        d
    }

    /// Resolves a use of `name`: a local (marking captures along the way) or a global.
    fn resolve_name(&mut self, name: &str) -> Place {
        match self.scopes.resolve(name) {
            Some(r) => {
                let d = self.scopes.data(r.symbol) as DeclId;
                let home = self.a.decls[d].func;
                if home != self.current_fn() {
                    self.a.decls[d].captured = true;
                    // Every function between the use and the declaration passes the variable on.
                    for &f in self.fn_stack.iter().rev() {
                        if f == home {
                            break;
                        }
                        if !self.a.functions[f].upvalues.contains(&d) {
                            self.a.functions[f].upvalues.push(d);
                        }
                    }
                }
                Place::Local(d)
            }
            None => Place::Global(self.global(name)),
        }
    }

    fn stmt(&mut self, node: NodeId) {
        let n = self.n;
        let tree = self.tree;
        let kind = tree.kind(node);
        if kind == n.var_decl {
            let name = tree.field(node, n.name).unwrap();
            let place = self.declare(name);
            self.a.defs.insert(node, place);
            if let Some(init) = tree.field(node, n.init) {
                self.expr(init);
            }
            self.define(name);
        } else if kind == n.fun_decl {
            let name = tree.field(node, n.name).unwrap();
            let place = self.declare(name);
            self.a.defs.insert(node, place);
            self.define(name);
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
            if let Some(init) = tree.field(node, n.init) {
                self.stmt(init);
            }
            if let Some(cond) = tree.field(node, n.cond) {
                self.expr(cond);
            }
            self.stmt(tree.field(node, n.body).unwrap());
            if let Some(incr) = tree.field(node, n.incr) {
                self.expr(incr);
            }
            self.scopes.pop();
        } else if kind == n.return_stmt {
            let keyword = tree.child(node, 0);
            let fn_kind = self.a.functions[self.current_fn()].kind;
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
        let name = tree.text(tree.field(node, n.name).unwrap()).to_string();
        let id = self.a.functions.len();
        self.a.functions.push(FnInfo {
            node: Some(node),
            kind,
            name,
            params: Vec::new(),
            upvalues: Vec::new(),
            this_decl: None,
        });
        self.a.fn_of.insert(node, id);
        self.fn_stack.push(id);
        self.scopes.push(ScopeKind::Function);
        if matches!(kind, FnKind::Method | FnKind::Initializer) {
            let this = self.declare_defined("this");
            self.a.functions[id].this_decl = Some(this);
        }
        if let Some(params) = tree.field(node, n.params) {
            for p in tree.fields(params, n.param) {
                let name = tree.text(p);
                let d = self.new_decl(name);
                match self.scopes.declare_defined(name, d as u64) {
                    Ok(_) => {}
                    Err(_) => self.error(p, "Already a variable with this name in this scope."),
                }
                self.a.functions[id].params.push(d);
            }
        }
        // The body's statements live in the function's own scope, as in jlox.
        let body = tree.field(node, n.body).unwrap();
        for s in statements(tree, body) {
            self.stmt(s);
        }
        self.scopes.pop();
        self.fn_stack.pop();
    }

    fn class_decl(&mut self, node: NodeId) {
        let n = self.n;
        let tree = self.tree;
        let enclosing = self.class;
        self.class = ClassKind::Class;
        let name = tree.field(node, n.name).unwrap();
        let place = self.declare(name);
        self.a.defs.insert(node, place);
        self.define(name);
        let superclass = tree.field(node, n.superclass);
        if let Some(sup) = superclass {
            if tree.text(sup) == tree.text(name) {
                self.error(sup, "A class can't inherit from itself.");
            }
            self.variable_use(sup, sup);
            self.class = ClassKind::Subclass;
            self.scopes.push(ScopeKind::Block);
            let d = self.declare_defined("super");
            self.a.super_decls.insert(node, d);
        }
        for m in tree.fields(node, n.methods) {
            let method_name = tree.text(tree.field(m, n.name).unwrap());
            let kind = if method_name == "init" {
                FnKind::Initializer
            } else {
                FnKind::Method
            };
            self.function(m, kind);
        }
        if superclass.is_some() {
            self.scopes.pop();
        }
        self.class = enclosing;
    }

    /// A variable read: `token` is the name, `node` the expression node to annotate.
    fn variable_use(&mut self, token: NodeId, node: NodeId) {
        let name = self.tree.text(token);
        if let Some(s) = self.scopes.lookup_in_current(name) {
            if !self.scopes.is_defined(s) {
                self.error(token, "Can't read local variable in its own initializer.");
            }
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
            let right = tree.field(node, n.right).unwrap();
            self.expr(right);
            if is_variable(tree, &n, left) {
                let name = tree.text(tree.child(left, 0));
                let place = self.resolve_name(name);
                self.a.uses.insert(node, place);
            } else {
                // A property assignment: resolve the object.
                self.expr(tree.field(left, n.operand).unwrap());
            }
        } else if kind == n.logical || kind == n.binary {
            self.expr(tree.field(node, n.left).unwrap());
            self.expr(tree.field(node, n.right).unwrap());
        } else if kind == n.unary {
            self.expr(tree.field(node, n.operand).unwrap());
        } else if kind == n.call {
            self.expr(tree.field(node, n.operand).unwrap());
            if let Some(args) = tree.field(node, n.args) {
                for a in tree.fields(args, n.arg) {
                    self.expr(a);
                }
            }
        } else if kind == n.get {
            self.expr(tree.field(node, n.operand).unwrap());
        }
    }
}
