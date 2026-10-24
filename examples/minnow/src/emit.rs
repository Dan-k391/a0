//! HIR generation for checked programs. Minnow's types map one to one onto HIR types, so the HIR
//! provides everything else: closures, arrays, structs and garbage collection.

use std::collections::{HashMap, HashSet};

use a0_diag::Diagnostics;
use a0_hir::{
    BinOp, BodyId, Builtin, ExprId, FuncId, HirBuilder, HirModule, KEYWORDS, LocalId, StructId,
    TypeId as HirType, UnOp,
};
use a0_sema::{TypeId, TypeTag};
use a0_syntax::{NodeId, SyntaxTree};

use crate::check::{Checked, Name, children_of_kind, statements, unwrap};
use crate::unescape;

struct Emit<'a> {
    tree: &'a SyntaxTree,
    c: &'a Checked,
    b: HirBuilder,
    structs: HashMap<TypeId, StructId>,
    funcs: Vec<FuncId>,
    locals: HashMap<usize, LocalId>,
    /// The body being built.
    body: BodyId,
}

/// A HIR name for a Minnow name: the same, unless that is a HIR keyword or already taken.
fn hir_name(name: &str, taken: &mut HashSet<String>) -> String {
    let mut n = name.to_string();
    while KEYWORDS.contains(&n.as_str()) || taken.contains(&n) {
        n.push('_');
    }
    taken.insert(n.clone());
    n
}

/// Builds the HIR module for a checked program.
pub fn emit(tree: &SyntaxTree, c: &Checked) -> Result<HirModule, Diagnostics> {
    let mut b = HirBuilder::new("minnow");
    let mut type_names = HashSet::new();
    let mut value_names = HashSet::from(["main".to_string()]);
    let structs = c
        .structs
        .iter()
        .map(|s| (s.ty, b.declare_struct(&hir_name(&s.name, &mut type_names))))
        .collect();
    let mut e = Emit {
        tree,
        c,
        b,
        structs,
        funcs: Vec::new(),
        locals: HashMap::new(),
        body: BodyId(0),
    };
    for s in &c.structs {
        for i in 0..c.types.field_count(s.ty) {
            let t = e.hir_type(c.types.field_type(s.ty, i));
            e.b.add_field(e.structs[&s.ty], c.types.field_name(s.ty, i), t);
        }
    }
    for f in &c.funcs {
        let params: Vec<HirType> = c
            .types
            .params(f.ty)
            .iter()
            .map(|&t| e.hir_type(t))
            .collect();
        let ret = e.hir_type(c.types.ret(f.ty).unwrap());
        let names: Vec<&str> = f
            .params
            .iter()
            .map(|&l| c.locals[l].name.as_str())
            .collect();
        let name = if f.name == "main" {
            f.name.clone()
        } else {
            hir_name(&f.name, &mut value_names)
        };
        let id = e.b.declare_function(&name, &names, &params, ret);
        e.funcs.push(id);
    }
    for (i, f) in c.funcs.iter().enumerate() {
        e.body = e.b.function_body(e.funcs[i]);
        for (k, &l) in f.params.iter().enumerate() {
            let p = e.b.param(e.body, k as u32);
            e.locals.insert(l, p);
        }
        let ret = c.types.ret(f.ty).unwrap();
        let root = e.body_block(f.node, ret);
        e.b.set_root(e.body, root);
    }
    e.b.finish()
}

impl Emit<'_> {
    fn field(&self, node: NodeId, name: &str) -> NodeId {
        self.tree.field_named(node, name).unwrap()
    }

    fn hir_type(&mut self, t: TypeId) -> HirType {
        let types = &self.c.types;
        match types.tag(t) {
            TypeTag::Named => match types.name(t).unwrap() {
                "int" => HirType::INT,
                "float" => HirType::FLOAT,
                "bool" => HirType::BOOL,
                "string" => HirType::STRING,
                _ => HirType::UNIT,
            },
            TypeTag::Array => {
                let e = self.hir_type(types.elem(t).unwrap());
                self.b.array_type(e)
            }
            TypeTag::Function => {
                let params: Vec<TypeId> = types.params(t).to_vec();
                let ret = types.ret(t).unwrap();
                let params: Vec<HirType> = params.into_iter().map(|p| self.hir_type(p)).collect();
                let ret = self.hir_type(ret);
                self.b.fn_type(&params, ret)
            }
            TypeTag::Struct => {
                let s = self.structs[&types.shallow(t)];
                self.b.struct_type(s)
            }
            // The checker reports every type it could not infer.
            TypeTag::Var => HirType::UNIT,
        }
    }

    /// The body of a function or closure. The checker has made sure that a function with a result
    /// returns on every path, so falling off the end is unreachable.
    fn body_block(&mut self, node: NodeId, ret: TypeId) -> ExprId {
        let mut items = Vec::new();
        for s in statements(self.tree, self.field(node, "body")) {
            self.stmt(s, &mut items);
        }
        let tail = if self.c.types.same(ret, self.c.unit) {
            None
        } else {
            let message = self.b.string("missing return");
            Some(self.b.builtin(Builtin::Panic, &[message]))
        };
        self.b.block(&items, tail)
    }

    fn block(&mut self, node: NodeId) -> ExprId {
        let mut items = Vec::new();
        if self.tree.kind_name(node) == "block" {
            for s in statements(self.tree, node) {
                self.stmt(s, &mut items);
            }
        } else {
            self.stmt(node, &mut items);
        }
        self.b.block(&items, None)
    }

    fn new_local(&mut self, l: usize) -> LocalId {
        let local = &self.c.locals[l];
        let t = self.hir_type(local.ty);
        let id = self.b.new_local(self.body, &local.name, t);
        self.locals.insert(l, id);
        id
    }

    fn stmt(&mut self, node: NodeId, out: &mut Vec<ExprId>) {
        let tree = self.tree;
        let e = match tree.kind_name(node) {
            "let_stmt" => {
                let value = self.expr(self.field(node, "value"));
                let l = self.new_local(self.c.decls[&node]);
                self.b.let_(l, value)
            }
            "if_stmt" => {
                let cond = self.expr(self.field(node, "cond"));
                let then = self.block(self.field(node, "then"));
                let els = tree.field_named(node, "else").map(|e| self.block(e));
                self.b.if_(cond, then, els)
            }
            "while_stmt" => {
                let cond = self.expr(self.field(node, "cond"));
                let body = self.block(self.field(node, "body"));
                self.b.while_(cond, body)
            }
            "for_stmt" => self.for_stmt(node),
            "return_stmt" => {
                let value = tree.field_named(node, "value").map(|v| self.expr(v));
                self.b.return_(value)
            }
            "break_stmt" => self.b.break_(),
            "continue_stmt" => self.b.continue_(),
            "block" => self.block(node),
            "expr_stmt" => {
                let target = self.field(node, "expr");
                match tree.field_named(node, "value") {
                    Some(value) => self.assign(target, value),
                    None => self.expr(target),
                }
            }
            _ => return,
        };
        out.push(e);
    }

    /// `for x in items { body }` is
    /// `{ let a = items; let i = 0; while i < len(a) { let x = a[i]; i = i + 1; body } }`.
    fn for_stmt(&mut self, node: NodeId) -> ExprId {
        let iterable = self.field(node, "iterable");
        let array_type = self.hir_type(self.c.ty(iterable));
        let items = self.expr(iterable);
        let a = self.b.new_local(self.body, "items", array_type);
        let i = self.b.new_local(self.body, "i", HirType::INT);
        let bind_a = self.b.let_(a, items);
        let zero = self.b.int(0);
        let bind_i = self.b.let_(i, zero);

        let (i_ref, a_ref) = (self.b.local(i), self.b.local(a));
        let len = self.b.builtin(Builtin::Len, &[a_ref]);
        let cond = self.b.binary(BinOp::Lt, i_ref, len);
        let (i_ref, a_ref) = (self.b.local(i), self.b.local(a));
        let elem = self.b.index(a_ref, i_ref);
        let x = self.new_local(self.c.decls[&node]);
        let bind_x = self.b.let_(x, elem);
        let (i_ref, one) = (self.b.local(i), self.b.int(1));
        let next = self.b.binary(BinOp::Add, i_ref, one);
        let step = self.b.set_local(i, next);
        let body = self.block(self.field(node, "body"));
        let body = self.b.block(&[bind_x, step, body], None);
        let lp = self.b.while_(cond, body);
        self.b.block(&[bind_a, bind_i, lp], None)
    }

    fn assign(&mut self, target: NodeId, value: NodeId) -> ExprId {
        let target = unwrap(self.tree, target);
        match self.tree.kind_name(target) {
            "name_expr" => {
                let Name::Local(l) = self.c.names[&target] else {
                    unreachable!("the checker allows assigning to locals only")
                };
                let v = self.expr(value);
                self.b.set_local(self.locals[&l], v)
            }
            "get" => {
                let operand = self.field(target, "operand");
                let s = self.c.ty(operand);
                let name = self.tree.text(self.field(target, "name"));
                let i = self.c.types.field_index(s, name).unwrap();
                let object = self.expr(operand);
                let v = self.expr(value);
                self.b.set_field(object, i, v)
            }
            _ => {
                let array = self.expr(self.field(target, "operand"));
                let index = self.expr(self.field(target, "index"));
                let v = self.expr(value);
                self.b.set_index(array, index, v)
            }
        }
    }

    fn expr(&mut self, node: NodeId) -> ExprId {
        let tree = self.tree;
        let node = unwrap(tree, node);
        match tree.kind_name(node) {
            "literal" => {
                let tok = tree.child(node, 0);
                let text = tree.text(tok);
                match tree.kind_name(tok) {
                    "INT" => self.b.int(text.parse().unwrap()),
                    "FLOAT" => self.b.float(text.parse().unwrap()),
                    "STRING" => {
                        let s = unescape(&text[1..text.len() - 1]).unwrap();
                        self.b.string(&s)
                    }
                    _ => self.b.bool_lit(text == "true"),
                }
            }
            "name_expr" => match self.c.names[&node] {
                Name::Local(l) => self.b.local(self.locals[&l]),
                Name::Func(f) => self.b.func(self.funcs[f]),
                Name::Builtin(_) => unreachable!("builtins are only called"),
            },
            "paren" => self.expr(self.field(node, "inner")),
            "array_lit" => match tree.field_named(node, "args") {
                Some(args) => {
                    let elems: Vec<ExprId> = tree
                        .fields_named(args, "arg")
                        .into_iter()
                        .map(|a| self.expr(a))
                        .collect();
                    self.b.array(&elems)
                }
                None => {
                    let t = self.hir_type(self.c.ty(node));
                    self.b.new_array(t)
                }
            },
            "new_expr" => self.new_expr(node),
            "closure" => {
                let t = self.c.ty(node);
                let params: Vec<TypeId> = self.c.types.params(t).to_vec();
                let ret = self.c.types.ret(t).unwrap();
                let hir_params: Vec<HirType> = params.iter().map(|&p| self.hir_type(p)).collect();
                let hir_ret = self.hir_type(ret);
                let param_nodes = children_of_kind(tree, node, "param");
                let names: Vec<&str> = param_nodes
                    .iter()
                    .map(|&p| tree.text(self.field(p, "name")))
                    .collect();
                let body = self
                    .b
                    .new_closure_body(self.body, &names, &hir_params, hir_ret);
                for (k, p) in param_nodes.iter().enumerate() {
                    let local = self.b.param(body, k as u32);
                    self.locals.insert(self.c.decls[p], local);
                }
                let outer = std::mem::replace(&mut self.body, body);
                let root = self.body_block(node, ret);
                self.b.set_root(body, root);
                self.body = outer;
                self.b.lambda(body)
            }
            "binary" => {
                let op = BinOp::from_symbol(tree.text(self.field(node, "op"))).unwrap();
                let l = self.expr(self.field(node, "left"));
                let r = self.expr(self.field(node, "right"));
                self.b.binary(op, l, r)
            }
            "logical" => {
                let l = self.expr(self.field(node, "left"));
                let r = self.expr(self.field(node, "right"));
                if tree.text(self.field(node, "op")) == "&&" {
                    self.b.and(l, r)
                } else {
                    self.b.or(l, r)
                }
            }
            "unary" => {
                let op = if tree.text(self.field(node, "op")) == "-" {
                    UnOp::Neg
                } else {
                    UnOp::Not
                };
                let operand = self.expr(self.field(node, "operand"));
                self.b.unary(op, operand)
            }
            "cast" => {
                let operand = self.field(node, "operand");
                let (from, to) = (self.c.ty(operand), self.c.ty(node));
                let e = self.expr(operand);
                if self.c.types.same(from, to) {
                    e
                } else {
                    let t = self.hir_type(to);
                    self.b.cast(e, t)
                }
            }
            "call" => self.call(node),
            "get" => {
                let object = self.expr(self.field(node, "operand"));
                let name = tree.text(self.field(node, "name"));
                self.b.field_named(object, name)
            }
            _ => {
                // `index`
                let array = self.expr(self.field(node, "operand"));
                let index = self.expr(self.field(node, "index"));
                self.b.index(array, index)
            }
        }
    }

    /// `new S { ... }`. Fields are evaluated in source order; the HIR takes them in declaration
    /// order, so out-of-order fields go through temporaries.
    fn new_expr(&mut self, node: NodeId) -> ExprId {
        let ty = self.c.ty(node);
        let s = self.structs[&self.c.types.shallow(ty)];
        let inits = children_of_kind(self.tree, node, "field_init");
        let index: Vec<u32> = inits
            .iter()
            .map(|&f| {
                let name = self.tree.text(self.field(f, "name"));
                self.c.types.field_index(ty, name).unwrap()
            })
            .collect();
        let in_order = index.windows(2).all(|w| w[0] < w[1]);
        let mut items = Vec::new();
        let mut fields = vec![None; inits.len()];
        for (&f, &i) in inits.iter().zip(&index) {
            let value = self.field(f, "value");
            let v = self.expr(value);
            fields[i as usize] = Some(if in_order {
                v
            } else {
                let t = self.hir_type(self.c.ty(value));
                let tmp = self.b.new_local(self.body, "field", t);
                items.push(self.b.let_(tmp, v));
                self.b.local(tmp)
            });
        }
        let fields: Vec<ExprId> = fields.into_iter().map(Option::unwrap).collect();
        let new = self.b.new_struct(s, &fields);
        if items.is_empty() {
            new
        } else {
            self.b.block(&items, Some(new))
        }
    }

    fn call(&mut self, node: NodeId) -> ExprId {
        let tree = self.tree;
        let callee = unwrap(tree, self.field(node, "operand"));
        let args: Vec<NodeId> = match tree.field_named(node, "args") {
            Some(a) => tree.fields_named(a, "arg"),
            None => Vec::new(),
        };
        let name = self.c.names.get(&callee).copied();
        if let Some(Name::Builtin(b)) = name {
            return self.builtin(b, &args);
        }
        // A named function is called directly; anything else is a closure value.
        let closure = match name {
            Some(Name::Func(_)) => None,
            _ => Some(self.expr(callee)),
        };
        let args: Vec<ExprId> = args.into_iter().map(|a| self.expr(a)).collect();
        match (closure, name) {
            (Some(c), _) => self.b.call_value(c, &args),
            (None, Some(Name::Func(f))) => self.b.call(self.funcs[f], &args),
            (None, _) => unreachable!(),
        }
    }

    fn builtin(&mut self, name: &str, args: &[NodeId]) -> ExprId {
        let arg_type = self.c.ty(args[0]);
        let values: Vec<ExprId> = args.iter().map(|&a| self.expr(a)).collect();
        match name {
            "print" => {
                let text = if self.c.types.same(arg_type, self.c.string) {
                    values[0]
                } else {
                    self.b.builtin(Builtin::Str, &values)
                };
                let newline = self.b.string("\n");
                let line = self.b.binary(BinOp::Add, text, newline);
                self.b.builtin(Builtin::Print, &[line])
            }
            "len" => self.b.builtin(Builtin::Len, &values),
            "push" => self.b.builtin(Builtin::Push, &values),
            "pop" => self.b.builtin(Builtin::Pop, &values),
            _ => self.b.builtin(Builtin::Str, &values),
        }
    }
}
