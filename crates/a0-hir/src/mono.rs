//! Monomorphization: a module with generic functions, structs and enums becomes one without, with a
//! copy of each generic item for every combination of type arguments the program uses. Lowering (and
//! later passes) only ever see the result, so frontends never substitute types themselves.

use std::collections::HashMap;

use a0_diag::{Diagnostic, Diagnostics};

use crate::model::*;

/// The module without generics, or `None` if it has none. Fails if an instance makes a `T?` of a
/// value type (`T?` in a generic item used with `T = int`).
pub(crate) fn monomorphize(m: &HirModule) -> Result<Option<HirModule>, Diagnostics> {
    let generic = m.structs.iter().any(|s| !s.type_params.is_empty())
        || m.enums.iter().any(|e| !e.type_params.is_empty())
        || m.funcs.iter().any(|f| !f.type_params.is_empty());
    if !generic {
        return Ok(None);
    }
    let mut mono = Mono {
        m,
        out: HirModule::new(&m.name),
        origin: HashMap::new(),
        structs: HashMap::new(),
        enums: HashMap::new(),
        funcs: HashMap::new(),
        globals: HashMap::new(),
        externs: HashMap::new(),
        queue: Vec::new(),
        current: None,
        diags: Diagnostics::new(),
    };
    // Interfaces keep their ids. Their signatures are not needed any more: interface calls carry
    // their types, and `Self` has no instance.
    for (k, i) in m.interfaces.iter().enumerate() {
        mono.out.interfaces.push(InterfaceDef {
            name: i.name.clone(),
            type_params: i.type_params.clone(),
            methods: i
                .methods
                .iter()
                .map(|s| MethodSig {
                    name: s.name.clone(),
                    params: Vec::new(),
                    ret: TypeId::UNIT,
                    span: s.span,
                })
                .collect(),
            span: i.span,
        });
        mono.register_type(&i.name, Item::Interface(InterfaceId(k as u32)));
    }
    for item in &m.items {
        match *item {
            Item::Struct(s) if m.structs[s.index()].type_params.is_empty() => {
                mono.struct_instance(s, &[]);
            }
            Item::Enum(e) if m.enums[e.index()].type_params.is_empty() => {
                mono.enum_instance(e, &[]);
            }
            Item::Global(g) => {
                let def = &m.globals[g.index()];
                let ty = mono.ty(def.ty, &Subst::default());
                let id = GlobalId(mono.out.globals.len() as u32);
                mono.out.globals.push(GlobalDef {
                    name: def.name.clone(),
                    ty,
                    span: def.span,
                });
                mono.register_value(&def.name, Item::Global(id));
                mono.globals.insert(g, id);
            }
            Item::Extern(x) => {
                let def = &m.externs[x.index()];
                let id = ExternId(mono.out.externs.len() as u32);
                let none = Subst::default();
                let params = def.params.iter().map(|&t| mono.ty(t, &none)).collect();
                let ret = mono.ty(def.ret, &none);
                mono.out.externs.push(ExternDef {
                    name: def.name.clone(),
                    params,
                    ret,
                    abi: def.abi,
                    span: def.span,
                });
                mono.register_value(&def.name, Item::Extern(id));
                mono.externs.insert(x, id);
            }
            Item::Func(f) if m.funcs[f.index()].type_params.is_empty() => {
                mono.func_instance(f, &[]);
            }
            _ => {}
        }
    }
    // Copy the functions used, then the methods of every instance that implements an interface
    // (which may use more), until nothing is left.
    loop {
        while let Some((f, args, out_f)) = mono.queue.pop() {
            mono.current = Some(out_f);
            mono.copy_function(f, &args, out_f);
        }
        if !mono.instantiate_vtables() {
            break;
        }
    }
    if mono.diags.has_errors() {
        return Err(mono.diags);
    }
    Ok(Some(mono.out))
}

/// The type arguments in force: those of the function being copied (and, for struct fields and
/// variant payloads, those of the type).
#[derive(Clone, Default)]
struct Subst {
    owner: Option<Item>,
    args: Vec<TypeId>,
}

struct Mono<'a> {
    m: &'a HirModule,
    out: HirModule,
    /// What each struct and enum of the output is an instance of.
    origin: HashMap<Owner, (Owner, Vec<TypeId>)>,
    structs: HashMap<(StructId, Vec<TypeId>), StructId>,
    enums: HashMap<(EnumId, Vec<TypeId>), EnumId>,
    funcs: HashMap<(FuncId, Vec<TypeId>), FuncId>,
    globals: HashMap<GlobalId, GlobalId>,
    externs: HashMap<ExternId, ExternId>,
    /// Function instances declared but not copied yet.
    queue: Vec<(FuncId, Vec<TypeId>, FuncId)>,
    /// The instance being copied, for errors.
    current: Option<FuncId>,
    diags: Diagnostics,
}

/// A body being copied: where its locals went.
struct Copy {
    subst: Subst,
    locals: HashMap<LocalId, LocalId>,
    func: FuncId,
}

impl Mono<'_> {
    fn register_type(&mut self, name: &str, item: Item) {
        self.out.type_names.insert(name.to_string(), item);
        self.out.items.push(item);
    }

    fn register_value(&mut self, name: &str, item: Item) {
        self.out.value_names.insert(name.to_string(), item);
        self.out.items.push(item);
    }

    /// The name of an instance: the item's own name, with its type arguments if it has any.
    fn instance_name(&self, name: &str, args: &[TypeId]) -> String {
        if args.is_empty() {
            return name.to_string();
        }
        let args: Vec<String> = args.iter().map(|&a| self.out.type_name(a)).collect();
        format!("{name}<{}>", args.join(", "))
    }

    /// `t` (a type of the input module) in the output module, with type parameters replaced.
    fn ty(&mut self, t: TypeId, subst: &Subst) -> TypeId {
        let k = self.m.kind(t).clone();
        let k = match k {
            TypeKind::Unit => return TypeId::UNIT,
            TypeKind::Bool => return TypeId::BOOL,
            TypeKind::Int => return TypeId::INT,
            TypeKind::Float => return TypeId::FLOAT,
            TypeKind::String => return TypeId::STRING,
            TypeKind::Any => return TypeId::ANY,
            TypeKind::Never => return TypeId::NEVER,
            TypeKind::Object => return TypeId::OBJECT,
            TypeKind::SizedInt(..) | TypeKind::F32 => return t,
            // A finished module has no inference variables; this is not reached.
            TypeKind::Var(_) | TypeKind::Error => return TypeId::UNIT,
            TypeKind::Param(owner, i) => {
                debug_assert_eq!(Some(owner), subst.owner);
                return subst.args[i as usize];
            }
            TypeKind::Array(e) => TypeKind::Array(self.ty(e, subst)),
            TypeKind::Map(a, b) => TypeKind::Map(self.ty(a, subst), self.ty(b, subst)),
            TypeKind::Nullable(inner) => {
                let inner = self.ty(inner, subst);
                if matches!(self.out.kind(inner), TypeKind::Nullable(_)) {
                    // `T?` with `T = string?` is `string?`.
                    return inner;
                }
                if !self.out.is_reference(inner) {
                    self.value_nullable(inner);
                }
                TypeKind::Nullable(inner)
            }
            TypeKind::Fn(params, ret) => {
                let params = params.into_iter().map(|p| self.ty(p, subst)).collect();
                TypeKind::Fn(params, self.ty(ret, subst))
            }
            TypeKind::Struct(s, args) => {
                let args: Vec<TypeId> = args.into_iter().map(|a| self.ty(a, subst)).collect();
                TypeKind::Struct(self.struct_instance(s, &args), Vec::new())
            }
            TypeKind::Enum(e, args) => {
                let args: Vec<TypeId> = args.into_iter().map(|a| self.ty(a, subst)).collect();
                TypeKind::Enum(self.enum_instance(e, &args), Vec::new())
            }
            TypeKind::Interface(i) => TypeKind::Interface(i),
        };
        self.out.intern(k)
    }

    fn value_nullable(&mut self, inner: TypeId) {
        let (name, span) = match self.current {
            Some(f) => (
                format!("`{}`", self.out.func_name(f)),
                self.out.funcs[f.index()].span,
            ),
            None => ("a generic type".to_string(), None),
        };
        let msg = format!(
            "{name} makes a nullable `{}`, but only reference types can be nullable",
            self.out.display_type(inner)
        );
        if !self.diags.as_slice().iter().any(|d| d.message == msg) {
            self.diags.push(
                Diagnostic::error(a0_diag::codes::VALUE_NOT_NULLABLE, &msg).with_span_opt(span),
            );
        }
    }

    /// Makes the method tables of the instances that implement interfaces and have none yet;
    /// returns whether it made any.
    fn instantiate_vtables(&mut self) -> bool {
        let mut made = false;
        let instances: Vec<(Owner, (Owner, Vec<TypeId>))> =
            self.origin.iter().map(|(&o, x)| (o, x.clone())).collect();
        for (out_owner, (owner, args)) in instances {
            for &(o, i, _) in &self.m.impls {
                if o != owner
                    || self
                        .out
                        .vtables
                        .iter()
                        .any(|v| v.0 == out_owner && v.1 == i)
                {
                    continue;
                }
                let names: Vec<String> = self.m.interfaces[i.index()]
                    .methods
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                let mut methods = Vec::new();
                for name in names {
                    // The builder checked that the implementation is complete.
                    let Some(&f) = self.m.methods.get(&(owner, name)) else {
                        continue;
                    };
                    methods.push(self.func_instance(f, &args));
                }
                self.out.vtables.push((out_owner, i, methods));
                made = true;
            }
        }
        made
    }

    /// `receiver.name(args)` where the receiver's type was a type parameter, now `t`: a call of
    /// the method of `t`'s type, or of an interface.
    fn method_call(
        &mut self,
        t: TypeId,
        name: &str,
        receiver: ExprId,
        args: Vec<ExprId>,
    ) -> ExprKind {
        if let TypeKind::Interface(i) = *self.out.kind(t)
            && let Some(k) = crate::methods::interface_method(self.m, i, name)
        {
            return ExprKind::InterfaceCall(receiver, i, k, args);
        }
        let (owner, targs) = match self.out.kind(t) {
            TypeKind::Struct(s, _) => self.origin[&Owner::Struct(*s)].clone(),
            TypeKind::Enum(e, _) => self.origin[&Owner::Enum(*e)].clone(),
            _ => match crate::methods::owner_of(&self.out, t) {
                Some(o) => (o, Vec::new()),
                None => unreachable!("bounds are checked before monomorphization"),
            },
        };
        let f = self.m.methods[&(owner, name.to_string())];
        let f = self.func_instance(f, &targs);
        let mut all = vec![receiver];
        all.extend(args);
        ExprKind::Call(f, Vec::new(), all)
    }

    fn struct_instance(&mut self, s: StructId, args: &[TypeId]) -> StructId {
        if let Some(&id) = self.structs.get(&(s, args.to_vec())) {
            return id;
        }
        let def = &self.m.structs[s.index()];
        let id = StructId(self.out.structs.len() as u32);
        let name = self.instance_name(&def.name, args);
        self.out.structs.push(StructDef {
            name: name.clone(),
            type_params: Vec::new(),
            fields: Vec::new(),
            field_spans: def.field_spans.clone(),
            span: def.span,
        });
        self.register_type(&name, Item::Struct(id));
        // Registered before its fields, so a struct can contain itself.
        self.structs.insert((s, args.to_vec()), id);
        self.origin
            .insert(Owner::Struct(id), (Owner::Struct(s), args.to_vec()));
        let subst = Subst {
            owner: Some(Item::Struct(s)),
            args: args.to_vec(),
        };
        let fields: Vec<(String, TypeId)> = def.fields.clone();
        let fields = fields
            .into_iter()
            .map(|(n, t)| (n, self.ty(t, &subst)))
            .collect();
        self.out.structs[id.index()].fields = fields;
        id
    }

    fn enum_instance(&mut self, e: EnumId, args: &[TypeId]) -> EnumId {
        if let Some(&id) = self.enums.get(&(e, args.to_vec())) {
            return id;
        }
        let def = &self.m.enums[e.index()];
        let id = EnumId(self.out.enums.len() as u32);
        let name = self.instance_name(&def.name, args);
        self.out.enums.push(EnumDef {
            name: name.clone(),
            type_params: Vec::new(),
            variants: Vec::new(),
            span: def.span,
        });
        self.register_type(&name, Item::Enum(id));
        self.enums.insert((e, args.to_vec()), id);
        self.origin
            .insert(Owner::Enum(id), (Owner::Enum(e), args.to_vec()));
        let subst = Subst {
            owner: Some(Item::Enum(e)),
            args: args.to_vec(),
        };
        let mut variants = Vec::new();
        for v in &def.variants {
            let fields = v.fields.iter().map(|&t| self.ty(t, &subst)).collect();
            variants.push(VariantDef {
                name: v.name.clone(),
                fields,
                span: v.span,
            });
        }
        self.out.enums[id.index()].variants = variants;
        id
    }

    /// The instance of `f` for `args` (already output types), declared now and copied later.
    fn func_instance(&mut self, f: FuncId, args: &[TypeId]) -> FuncId {
        if let Some(&id) = self.funcs.get(&(f, args.to_vec())) {
            return id;
        }
        let def = &self.m.funcs[f.index()];
        let id = FuncId(self.out.funcs.len() as u32);
        let body = BodyId(self.out.bodies.len() as u32);
        let name = self.instance_name(&def.name, args);
        self.out.funcs.push(FuncDef {
            name: name.clone(),
            type_params: Vec::new(),
            body,
            span: def.span,
            local_names: def.local_names.clone(),
        });
        self.out.bodies.push(BodyData {
            params: Vec::new(),
            param_names: Vec::new(),
            ret: TypeId::UNIT,
            root: None,
            parent: None,
            func: id,
            name: None,
        });
        self.register_value(&name, Item::Func(id));
        self.funcs.insert((f, args.to_vec()), id);
        self.queue.push((f, args.to_vec(), id));
        id
    }

    fn copy_function(&mut self, f: FuncId, args: &[TypeId], out_f: FuncId) {
        let mut c = Copy {
            subst: Subst {
                owner: Some(Item::Func(f)),
                args: args.to_vec(),
            },
            locals: HashMap::new(),
            func: out_f,
        };
        let body = self.m.funcs[f.index()].body;
        let out_body = self.out.funcs[out_f.index()].body;
        self.copy_body(&mut c, body, out_body);
    }

    /// Copies the parameters, result type and expression of `body` into `out_body`.
    fn copy_body(&mut self, c: &mut Copy, body: BodyId, out_body: BodyId) {
        let data = &self.m.bodies[body.index()];
        let params: Vec<LocalId> = data.params.clone();
        let (ret, root) = (data.ret, data.root);
        let params = params
            .into_iter()
            .map(|p| self.local(c, p, out_body))
            .collect();
        let ret = self.ty(ret, &c.subst);
        let root = root.map(|r| self.expr(c, r, out_body));
        let b = &mut self.out.bodies[out_body.index()];
        b.params = params;
        b.ret = ret;
        b.root = root;
    }

    fn local(&mut self, c: &mut Copy, l: LocalId, out_body: BodyId) -> LocalId {
        if let Some(&id) = c.locals.get(&l) {
            return id;
        }
        let data = &self.m.locals[l.index()];
        let (name, source_name, ty, span) = (
            data.name.clone(),
            data.source_name.clone(),
            data.ty,
            data.span,
        );
        let ty = self.ty(ty, &c.subst);
        let id = LocalId(self.out.locals.len() as u32);
        self.out.locals.push(LocalData {
            name,
            source_name,
            ty,
            body: out_body,
            span,
        });
        c.locals.insert(l, id);
        id
    }

    /// A local referred to (not declared) by an expression: declared by an enclosing body.
    fn local_ref(&mut self, c: &mut Copy, l: LocalId) -> LocalId {
        match c.locals.get(&l) {
            Some(&id) => id,
            // A `let` in a nested block is visited before its uses, so this is not reached.
            None => panic!("a local is used before its declaration was copied"),
        }
    }

    fn exprs(&mut self, c: &mut Copy, es: &[ExprId], body: BodyId) -> Vec<ExprId> {
        es.iter().map(|&e| self.expr(c, e, body)).collect()
    }

    fn expr(&mut self, c: &mut Copy, e: ExprId, body: BodyId) -> ExprId {
        let data = &self.m.exprs[e.index()];
        let (kind, ty, span) = (data.kind.clone(), data.ty, data.span);
        let ty = self.ty(ty, &c.subst);
        let kind = match kind {
            ExprKind::Unit => ExprKind::Unit,
            ExprKind::Bool(v) => ExprKind::Bool(v),
            ExprKind::Int(v) => ExprKind::Int(v),
            ExprKind::Float(v) => ExprKind::Float(v),
            ExprKind::Str(s) => ExprKind::Str(s),
            ExprKind::Null => ExprKind::Null,
            ExprKind::Local(l) => ExprKind::Local(self.local_ref(c, l)),
            ExprKind::Global(g) => ExprKind::Global(self.globals[&g]),
            ExprKind::Func(f, targs) => {
                let targs: Vec<TypeId> = targs.iter().map(|&t| self.ty(t, &c.subst)).collect();
                ExprKind::Func(self.func_instance(f, &targs), Vec::new())
            }
            ExprKind::Let(l, init) => {
                // The initializer cannot see the new local, so copy it first.
                let init = self.expr(c, init, body);
                ExprKind::Let(self.local(c, l, body), init)
            }
            ExprKind::SetLocal(l, v) => {
                let v = self.expr(c, v, body);
                ExprKind::SetLocal(self.local_ref(c, l), v)
            }
            ExprKind::SetGlobal(g, v) => {
                ExprKind::SetGlobal(self.globals[&g], self.expr(c, v, body))
            }
            ExprKind::SetField(o, f, v) => {
                ExprKind::SetField(self.expr(c, o, body), f, self.expr(c, v, body))
            }
            ExprKind::SetIndex(t, i, v) => ExprKind::SetIndex(
                self.expr(c, t, body),
                self.expr(c, i, body),
                self.expr(c, v, body),
            ),
            ExprKind::Unary(op, a) => ExprKind::Unary(op, self.expr(c, a, body)),
            ExprKind::Binary(op, a, b) => {
                ExprKind::Binary(op, self.expr(c, a, body), self.expr(c, b, body))
            }
            ExprKind::And(a, b) => ExprKind::And(self.expr(c, a, body), self.expr(c, b, body)),
            ExprKind::Or(a, b) => ExprKind::Or(self.expr(c, a, body), self.expr(c, b, body)),
            ExprKind::Call(f, targs, args) => {
                let targs: Vec<TypeId> = targs.iter().map(|&t| self.ty(t, &c.subst)).collect();
                let f = self.func_instance(f, &targs);
                ExprKind::Call(f, Vec::new(), self.exprs(c, &args, body))
            }
            ExprKind::CallExtern(x, args) => {
                ExprKind::CallExtern(self.externs[&x], self.exprs(c, &args, body))
            }
            ExprKind::CallValue(callee, args) => {
                let callee = self.expr(c, callee, body);
                ExprKind::CallValue(callee, self.exprs(c, &args, body))
            }
            ExprKind::Lambda(b) => {
                let out_b = BodyId(self.out.bodies.len() as u32);
                self.out.bodies.push(BodyData {
                    params: Vec::new(),
                    param_names: Vec::new(),
                    ret: TypeId::UNIT,
                    root: None,
                    parent: Some(body),
                    func: c.func,
                    name: self.m.bodies[b.index()].name.clone(),
                });
                self.copy_body(c, b, out_b);
                ExprKind::Lambda(out_b)
            }
            ExprKind::New(_, fields) => {
                let TypeKind::Struct(s, _) = *self.out.kind(ty) else {
                    unreachable!("a struct literal has a struct type")
                };
                ExprKind::New(s, self.exprs(c, &fields, body))
            }
            ExprKind::Field(o, f) => ExprKind::Field(self.expr(c, o, body), f),
            ExprKind::FieldName(..) | ExprKind::SetFieldName(..) => {
                unreachable!("finish resolves fields named before their object's type was known")
            }
            ExprKind::Variant(_, v, args) => {
                let TypeKind::Enum(en, _) = *self.out.kind(ty) else {
                    unreachable!("a variant has an enum type")
                };
                ExprKind::Variant(en, v, self.exprs(c, &args, body))
            }
            ExprKind::Match(s, arms) => {
                let s = self.expr(c, s, body);
                let mut out_arms = Vec::new();
                for a in arms {
                    let arm = &self.m.arms[a.index()];
                    let (variant, binds, arm_body) = (arm.variant, arm.binds.clone(), arm.body);
                    let binds = binds.into_iter().map(|l| self.local(c, l, body)).collect();
                    let arm_body = self.expr(c, arm_body, body);
                    self.out.arms.push(ArmData {
                        variant,
                        binds,
                        body: arm_body,
                    });
                    out_arms.push(ArmId(self.out.arms.len() as u32 - 1));
                }
                ExprKind::Match(s, out_arms)
            }
            ExprKind::Array(elems) => ExprKind::Array(self.exprs(c, &elems, body)),
            ExprKind::NewArray => ExprKind::NewArray,
            ExprKind::NewMap => ExprKind::NewMap,
            ExprKind::NewObject => ExprKind::NewObject,
            ExprKind::Index(t, i) => ExprKind::Index(self.expr(c, t, body), self.expr(c, i, body)),
            ExprKind::Builtin(op, args) => ExprKind::Builtin(op, self.exprs(c, &args, body)),
            ExprKind::Cast(a) => ExprKind::Cast(self.expr(c, a, body)),
            ExprKind::Unwrap(a) => ExprKind::Unwrap(self.expr(c, a, body)),
            ExprKind::Is(a, t) => {
                let a = self.expr(c, a, body);
                ExprKind::Is(a, self.ty(t, &c.subst))
            }
            ExprKind::If(cond, t, e) => {
                let cond = self.expr(c, cond, body);
                let t = self.expr(c, t, body);
                ExprKind::If(cond, t, e.map(|e| self.expr(c, e, body)))
            }
            ExprKind::IfLet(l, v, t, e) => {
                let v = self.expr(c, v, body);
                let l = self.local(c, l, body);
                let t = self.expr(c, t, body);
                ExprKind::IfLet(l, v, t, e.map(|e| self.expr(c, e, body)))
            }
            ExprKind::While(cond, b) => {
                ExprKind::While(self.expr(c, cond, body), self.expr(c, b, body))
            }
            ExprKind::Loop(b) => ExprKind::Loop(self.expr(c, b, body)),
            ExprKind::Break => ExprKind::Break,
            ExprKind::Continue => ExprKind::Continue,
            ExprKind::Return(v) => ExprKind::Return(v.map(|v| self.expr(c, v, body))),
            ExprKind::Throw(v) => ExprKind::Throw(self.expr(c, v, body)),
            ExprKind::MethodCall(r, name, args) => {
                let receiver_type = self.ty(self.m.expr_type(r), &c.subst);
                let r = self.expr(c, r, body);
                let args = self.exprs(c, &args, body);
                self.method_call(receiver_type, &name, r, args)
            }
            ExprKind::InterfaceCall(r, i, k, args) => {
                let r = self.expr(c, r, body);
                ExprKind::InterfaceCall(r, i, k, self.exprs(c, &args, body))
            }
            ExprKind::Try(b, l, h) => {
                let b = self.expr(c, b, body);
                let l = self.local(c, l, body);
                ExprKind::Try(b, l, self.expr(c, h, body))
            }
            ExprKind::Block(items, tail) => {
                let items = self.exprs(c, &items, body);
                ExprKind::Block(items, tail.map(|t| self.expr(c, t, body)))
            }
        };
        let id = ExprId(self.out.exprs.len() as u32);
        self.out.exprs.push(ExprData { kind, ty, span });
        id
    }
}
