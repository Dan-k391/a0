//! The HIR optimizer: inlining of small functions, then simplification, on monomorphized HIR.
//!
//! It runs by default before lowering (`A0_OPT=0` turns it off) and needs nothing from frontends.
//! It complements Cranelift, which optimizes within a function but does not inline across functions
//! and does not know what an `any` holds: once a small helper such as `fn truthy(v: any) -> bool` is
//! inlined at a call with `x as any`, `(x as any) is bool` folds to a constant.
//!
//! Every rewrite keeps the program's observable behavior: evaluation order, side effects, traps and
//! panics. Only expressions without effects are ever dropped or duplicated, and an inlined call
//! evaluates its arguments in order into fresh locals, exactly as the call did.

use std::collections::{HashMap, HashSet};

use crate::builder::visit_children;
use crate::model::*;

/// Functions with at most this many nodes, counted after inlining into them, are inlined (D37).
const INLINE_SIZE: usize = 120;
/// Inlining grows a body to at most this many nodes, or to twice its size if that is more.
const GROWN_SIZE: usize = 4000;
/// Rounds of simplification.
const ROUNDS: u32 = 3;

pub(crate) fn optimize(m: &mut HirModule) {
    let mut o = Opt { m };
    let inlinable = o.inlinable_functions();
    // Callees before their callers: a callee is copied, and measured, as it is after its own
    // inlining, so no copy is inlined into again and sizes cannot compound along a call chain.
    for f in o.bottom_up() {
        for b in 0..o.m.bodies.len() {
            let body = BodyId(b as u32);
            if o.m.bodies[b].func != f {
                continue;
            }
            if let Some(root) = o.m.bodies[b].root {
                let size = o.size(root);
                let mut budget = GROWN_SIZE.max(2 * size) - size;
                o.inline_calls(root, body, &inlinable, &mut budget);
            }
        }
    }
    for _ in 0..ROUNDS {
        for f in 0..o.m.funcs.len() {
            o.simplify_function(FuncId(f as u32));
        }
    }
}

struct Opt<'a> {
    m: &'a mut HirModule,
}

/// How the locals of one function are used.
#[derive(Default)]
struct Uses {
    reads: HashMap<LocalId, u32>,
    writes: HashMap<LocalId, u32>,
    /// Locals used from a closure (they live in boxes; leave them alone).
    captured: HashSet<LocalId>,
}

/// What a local's reads are replaced with.
#[derive(Clone)]
enum Replace {
    Local(LocalId),
    Const(ExprKind),
    /// The local holds `raw as any` for this local `raw` of a known type: tests and unpacking of the
    /// `any` are decided statically. Plain reads still read the local.
    Boxed(LocalId, TypeId),
}

impl Opt<'_> {
    fn ty(&self, e: ExprId) -> TypeId {
        self.m.exprs[e.index()].ty
    }

    fn kind(&self, e: ExprId) -> &ExprKind {
        &self.m.exprs[e.index()].kind
    }

    fn node(&mut self, kind: ExprKind, ty: TypeId, span_of: ExprId) -> ExprId {
        let span = self.m.exprs[span_of.index()].span;
        self.m.exprs.push(ExprData { kind, ty, span });
        ExprId(self.m.exprs.len() as u32 - 1)
    }

    fn children(&self, e: ExprId) -> Vec<ExprId> {
        let mut out = Vec::new();
        visit_children(self.m, e, &mut |c| out.push(c));
        out
    }

    // ---------------------------------------------------------------- inlining

    /// Functions small and simple enough to inline: no closures inside, `return` only where it can
    /// become the value of the inlined block, and no call to themselves.
    fn inlinable_functions(&self) -> HashSet<FuncId> {
        let mut out = HashSet::new();
        for f in 0..self.m.funcs.len() {
            let f = FuncId(f as u32);
            let Some(root) = self.m.body_root(self.m.func_body(f)) else {
                continue;
            };
            // `main` runs once, and a function that never returns is an error path: keeping it a
            // call keeps cold code out of hot functions.
            if self.m.funcs[f.index()].name == "main" || self.m.expr_type(root) == TypeId::NEVER {
                continue;
            }
            let mut size = 0;
            let mut ok = true;
            self.walk(root, &mut |m, e| {
                size += 1;
                match &m.exprs[e.index()].kind {
                    ExprKind::Lambda(_) => ok = false,
                    ExprKind::Call(g, ..) if *g == f => ok = false,
                    _ => {}
                }
            });
            if ok && size <= INLINE_SIZE && self.strippable(root) {
                out.insert(f);
            }
        }
        out
    }

    /// The functions, callees before their callers except around recursion.
    fn bottom_up(&self) -> Vec<FuncId> {
        let n = self.m.funcs.len();
        let mut calls = vec![Vec::new(); n];
        for body in &self.m.bodies {
            if let Some(root) = body.root {
                let callers = &mut calls[body.func.index()];
                self.walk(root, &mut |m, e| {
                    if let ExprKind::Call(g, ..) = m.exprs[e.index()].kind {
                        callers.push(g);
                    }
                });
            }
        }
        let mut order = Vec::with_capacity(n);
        let mut seen = vec![false; n];
        for start in 0..n {
            if seen[start] {
                continue;
            }
            seen[start] = true;
            // (function, how many of its callees have been visited)
            let mut stack = vec![(start, 0)];
            while let Some(&mut (f, ref mut next)) = stack.last_mut() {
                if let Some(&g) = calls[f].get(*next) {
                    *next += 1;
                    if !seen[g.index()] {
                        seen[g.index()] = true;
                        stack.push((g.index(), 0));
                    }
                } else {
                    order.push(FuncId(f as u32));
                    stack.pop();
                }
            }
        }
        order
    }

    /// The number of nodes in the tree `e` (not inside closures).
    fn size(&self, e: ExprId) -> usize {
        let mut n = 0;
        self.walk(e, &mut |_, _| n += 1);
        n
    }

    /// Calls `f` on `e` and every node under it (not inside closures).
    fn walk(&self, e: ExprId, f: &mut dyn FnMut(&HirModule, ExprId)) {
        f(self.m, e);
        for c in self.children(e) {
            self.walk(c, f);
        }
    }

    fn contains_return(&self, e: ExprId) -> bool {
        let mut found = false;
        self.walk(e, &mut |m, x| {
            found |= matches!(m.exprs[x.index()].kind, ExprKind::Return(_));
        });
        found
    }

    /// Whether every path through `e` ends in `return` (so what follows is dead).
    fn always_returns(&self, e: ExprId) -> bool {
        match self.kind(e) {
            ExprKind::Return(_) => true,
            ExprKind::Block(items, tail) => {
                items.iter().any(|&i| self.always_returns(i))
                    || tail.is_some_and(|t| self.always_returns(t))
            }
            ExprKind::If(_, t, Some(f)) | ExprKind::IfLet(_, _, t, Some(f)) => {
                self.always_returns(*t) && self.always_returns(*f)
            }
            _ => false,
        }
    }

    /// Whether the `return`s in a function body are all where [`Opt::strip_returns`] can turn them
    /// into the body's value.
    fn strippable(&self, e: ExprId) -> bool {
        if !self.contains_return(e) {
            return true;
        }
        match self.kind(e) {
            ExprKind::Return(v) => v.is_none_or(|v| !self.contains_return(v)),
            ExprKind::If(c, t, f) => {
                !self.contains_return(*c)
                    && self.strippable(*t)
                    && f.is_none_or(|f| self.strippable(f))
            }
            ExprKind::Block(items, tail) => {
                for (i, &item) in items.iter().enumerate() {
                    if !self.contains_return(item) {
                        continue;
                    }
                    let rest_ok = |o: &Self| {
                        items[i + 1..].iter().all(|&r| o.strippable(r))
                            && tail.is_none_or(|t| o.strippable(t))
                    };
                    return match self.kind(item) {
                        ExprKind::Return(_) | ExprKind::Block(..) if self.always_returns(item) => {
                            self.strippable(item)
                        }
                        ExprKind::If(c, t, None) => {
                            !self.contains_return(*c)
                                && self.always_returns(*t)
                                && self.strippable(*t)
                                && rest_ok(self)
                        }
                        ExprKind::If(_, t, Some(f))
                            if self.always_returns(*t) && self.always_returns(*f) =>
                        {
                            self.strippable(item)
                        }
                        _ => false,
                    };
                }
                tail.is_none_or(|t| self.strippable(t))
            }
            _ => false,
        }
    }

    /// `e` (a copy of a function body) with its `return v`s turned into the value of the body, of
    /// type `ret`. Requires [`Opt::strippable`].
    fn strip_returns(&mut self, e: ExprId, ret: TypeId) -> ExprId {
        if !self.contains_return(e) {
            return e;
        }
        match self.kind(e).clone() {
            ExprKind::Return(Some(v)) => v,
            ExprKind::Return(None) => self.node(ExprKind::Unit, TypeId::UNIT, e),
            ExprKind::If(c, t, f) => {
                let t = self.strip_returns(t, ret);
                let f = f.map(|f| self.strip_returns(f, ret));
                self.node(ExprKind::If(c, t, f), ret, e)
            }
            ExprKind::Block(items, tail) => {
                let i = items
                    .iter()
                    .position(|&x| self.contains_return(x))
                    .unwrap_or(items.len());
                if i == items.len() {
                    let tail = tail.map(|t| self.strip_returns(t, ret));
                    return self.node(ExprKind::Block(items, tail), ret, e);
                }
                let item = items[i];
                let value = match self.kind(item).clone() {
                    ExprKind::If(c, t, None) => {
                        // `if c { return a } rest` is `if c { a } else { rest }`.
                        let rest =
                            self.node(ExprKind::Block(items[i + 1..].to_vec(), tail), ret, e);
                        let rest = self.strip_returns(rest, ret);
                        let t = self.strip_returns(t, ret);
                        self.node(ExprKind::If(c, t, Some(rest)), ret, item)
                    }
                    // A statement that always returns: what follows it is dead.
                    _ => self.strip_returns(item, ret),
                };
                self.node(ExprKind::Block(items[..i].to_vec(), Some(value)), ret, e)
            }
            _ => e,
        }
    }

    /// Replaces calls of inlinable functions under `e`, which belongs to `body`, while the copies
    /// fit in `budget` nodes.
    fn inline_calls(
        &mut self,
        e: ExprId,
        body: BodyId,
        inlinable: &HashSet<FuncId>,
        budget: &mut usize,
    ) {
        for c in self.children(e) {
            self.inline_calls(c, body, inlinable, budget);
        }
        let ExprKind::Call(f, _, args) = self.kind(e).clone() else {
            return;
        };
        // A function that became recursive by inlining stays a call within itself.
        if !inlinable.contains(&f) || self.m.bodies[body.index()].func == f {
            return;
        }
        let callee = self.m.func_body(f);
        let root = self.m.body_root(callee).unwrap();
        let size = self.size(root);
        if size > INLINE_SIZE || size > *budget {
            return;
        }
        *budget -= size;
        let params = self.m.body_params(callee).to_vec();
        let ret = self.m.body_ret(callee);
        let mut locals = HashMap::new();
        let mut items = Vec::new();
        for (&p, &a) in params.iter().zip(&args) {
            let l = self.copy_local(p, body, &mut locals);
            items.push(self.node(ExprKind::Let(l, a), TypeId::UNIT, a));
        }
        let copy = self.copy(root, body, &mut locals);
        let value = self.strip_returns(copy, ret);
        let e_ty = self.ty(e);
        self.m.exprs[e.index()].kind = ExprKind::Block(items, Some(value));
        self.m.exprs[e.index()].ty = e_ty;
    }

    fn copy_local(
        &mut self,
        l: LocalId,
        body: BodyId,
        locals: &mut HashMap<LocalId, LocalId>,
    ) -> LocalId {
        let data = &self.m.locals[l.index()];
        let (name, source_name, ty, span) = (
            data.name.clone(),
            data.source_name.clone(),
            data.ty,
            data.span,
        );
        let id = LocalId(self.m.locals.len() as u32);
        self.m.locals.push(LocalData {
            name: format!("{name}_inl"),
            source_name,
            ty,
            body,
            span,
        });
        locals.insert(l, id);
        id
    }

    /// A copy of the tree `e` from an inlined function, its locals moved into `body`.
    fn copy(&mut self, e: ExprId, body: BodyId, locals: &mut HashMap<LocalId, LocalId>) -> ExprId {
        let data = &self.m.exprs[e.index()];
        let (kind, ty, span) = (data.kind.clone(), data.ty, data.span);
        let kind = match kind {
            ExprKind::Local(l) => ExprKind::Local(locals[&l]),
            ExprKind::SetLocal(l, v) => {
                let v = self.copy(v, body, locals);
                ExprKind::SetLocal(locals[&l], v)
            }
            ExprKind::Let(l, init) => {
                let init = self.copy(init, body, locals);
                ExprKind::Let(self.copy_local(l, body, locals), init)
            }
            ExprKind::IfLet(l, v, t, f) => {
                let v = self.copy(v, body, locals);
                let l = self.copy_local(l, body, locals);
                let t = self.copy(t, body, locals);
                ExprKind::IfLet(l, v, t, f.map(|f| self.copy(f, body, locals)))
            }
            ExprKind::Match(s, arms) => {
                let s = self.copy(s, body, locals);
                let mut out = Vec::new();
                for a in arms {
                    let arm = self.m.arms[a.index()].clone();
                    let binds = arm
                        .binds
                        .iter()
                        .map(|&b| self.copy_local(b, body, locals))
                        .collect();
                    let arm_body = self.copy(arm.body, body, locals);
                    self.m.arms.push(ArmData {
                        variant: arm.variant,
                        binds,
                        body: arm_body,
                    });
                    out.push(ArmId(self.m.arms.len() as u32 - 1));
                }
                ExprKind::Match(s, out)
            }
            other => map_children(&other, &mut |c| self.copy(c, body, locals)),
        };
        self.m.exprs.push(ExprData { kind, ty, span });
        ExprId(self.m.exprs.len() as u32 - 1)
    }

    // ---------------------------------------------------------------- simplification

    fn simplify_function(&mut self, f: FuncId) {
        let bodies: Vec<BodyId> = (0..self.m.bodies.len())
            .map(|b| BodyId(b as u32))
            .filter(|&b| self.m.bodies[b.index()].func == f)
            .collect();
        let uses = self.uses(&bodies);
        self.split_boxed_lets(&bodies, &uses);
        // The split made new locals; count again.
        let uses = self.uses(&bodies);
        let replace = self.replacements(&bodies, &uses);
        for b in bodies {
            if let Some(root) = self.m.bodies[b.index()].root {
                let root = self.simplify(root, &uses, &replace);
                self.m.bodies[b.index()].root = Some(root);
                self.mark_guarded_casts(root, &uses, &HashMap::new());
            }
        }
    }

    /// Marks `x as T` as unchecked where an enclosing `if x is T` (with `x` a local never assigned)
    /// guarantees it succeeds. `known` holds the guarded locals and their types.
    fn mark_guarded_casts(&mut self, e: ExprId, uses: &Uses, known: &HashMap<LocalId, TypeId>) {
        match self.kind(e).clone() {
            ExprKind::Cast(x) => {
                if let ExprKind::Local(l) = *self.kind(x)
                    && self.ty(x) == TypeId::ANY
                    && known.get(&l) == Some(&self.ty(e))
                {
                    self.m.unchecked_casts.insert(e);
                }
            }
            ExprKind::If(c, t, f) => {
                self.mark_guarded_casts(c, uses, known);
                let mut inner = known.clone();
                self.guards(c, uses, &mut inner);
                self.mark_guarded_casts(t, uses, &inner);
                if let Some(f) = f {
                    self.mark_guarded_casts(f, uses, known);
                }
                return;
            }
            _ => {}
        }
        for c in self.children(e) {
            self.mark_guarded_casts(c, uses, known);
        }
    }

    /// The `x is T` facts that hold when `c` is true.
    fn guards(&self, c: ExprId, uses: &Uses, out: &mut HashMap<LocalId, TypeId>) {
        match self.kind(c) {
            ExprKind::Is(x, t) => {
                if let ExprKind::Local(l) = *self.kind(*x)
                    && !uses.writes.contains_key(&l)
                    && !uses.captured.contains(&l)
                {
                    out.insert(l, *t);
                }
            }
            ExprKind::And(a, b) => {
                self.guards(*a, uses, out);
                self.guards(*b, uses, out);
            }
            _ => {}
        }
    }

    fn uses(&self, bodies: &[BodyId]) -> Uses {
        let mut u = Uses::default();
        for &b in bodies {
            let Some(root) = self.m.bodies[b.index()].root else {
                continue;
            };
            self.walk(root, &mut |m, e| {
                let (l, write) = match m.exprs[e.index()].kind {
                    ExprKind::Local(l) => (l, false),
                    ExprKind::SetLocal(l, _) => (l, true),
                    _ => return,
                };
                *if write { &mut u.writes } else { &mut u.reads }
                    .entry(l)
                    .or_insert(0) += 1;
                if m.locals[l.index()].body != b {
                    u.captured.insert(l);
                }
            });
        }
        u
    }

    /// Locals that are copies of another local or of a constant, never assigned and not captured:
    /// their reads can use the original.
    fn replacements(&self, bodies: &[BodyId], uses: &Uses) -> HashMap<LocalId, Replace> {
        let mut out = HashMap::new();
        let fixed = |l: LocalId| !uses.writes.contains_key(&l) && !uses.captured.contains(&l);
        for &b in bodies {
            let Some(root) = self.m.bodies[b.index()].root else {
                continue;
            };
            self.walk(root, &mut |m, e| {
                let ExprKind::Let(l, init) = m.exprs[e.index()].kind else {
                    return;
                };
                if !fixed(l) {
                    return;
                }
                let init_data = &m.exprs[init.index()];
                match &init_data.kind {
                    ExprKind::Local(q)
                        if fixed(*q) && m.locals[q.index()].ty == m.locals[l.index()].ty =>
                    {
                        out.insert(l, Replace::Local(*q));
                    }
                    // `let v = raw as any` for a `raw` of a known type (see `split_boxed_lets`).
                    ExprKind::Cast(x) if init_data.ty == TypeId::ANY => {
                        if let ExprKind::Local(raw) = m.exprs[x.index()].kind
                            && fixed(raw)
                            && decided(m.kind(m.locals[raw.index()].ty))
                        {
                            out.insert(l, Replace::Boxed(raw, m.locals[raw.index()].ty));
                        }
                    }
                    k @ (ExprKind::Int(_)
                    | ExprKind::Float(_)
                    | ExprKind::Bool(_)
                    | ExprKind::Unit)
                        if init_data.ty == m.locals[l.index()].ty =>
                    {
                        out.insert(l, Replace::Const(k.clone()));
                    }
                    _ => {}
                }
            });
        }
        // A copy of a copy reads the original.
        let resolve = |mut l: LocalId| {
            let mut seen = 0;
            while let Some(Replace::Local(q)) = out.get(&l) {
                l = *q;
                seen += 1;
                if seen > 64 {
                    break;
                }
            }
            l
        };
        let keys: Vec<LocalId> = out.keys().copied().collect();
        let mut resolved = HashMap::new();
        for l in keys {
            let r = match &out[&l] {
                Replace::Local(_) => match out.get(&resolve(l)) {
                    Some(Replace::Const(k)) => Replace::Const(k.clone()),
                    _ => Replace::Local(resolve(l)),
                },
                c => c.clone(),
            };
            resolved.insert(l, r);
        }
        resolved
    }

    /// Turns `let v = <... x as any>`, for a `v` never assigned, into `let raw = <... x>;
    /// let v = raw as any`, so that `v is T` and `v as T` need not look inside the `any`.
    fn split_boxed_lets(&mut self, bodies: &[BodyId], uses: &Uses) {
        let mut lets = Vec::new();
        for &b in bodies {
            if let Some(root) = self.m.bodies[b.index()].root {
                self.walk(root, &mut |m, e| {
                    if let ExprKind::Let(l, init) = m.exprs[e.index()].kind {
                        lets.push((e, l, init));
                    }
                });
            }
        }
        for (e, l, init) in lets {
            let fixed = !uses.writes.contains_key(&l) && !uses.captured.contains(&l);
            if !fixed || self.m.locals[l.index()].ty != TypeId::ANY {
                continue;
            }
            // Already split.
            if let ExprKind::Cast(x) = *self.kind(init)
                && matches!(self.kind(x), ExprKind::Local(_))
            {
                continue;
            }
            let Some(t) = self.boxed_source(init) else {
                continue;
            };
            let data = &self.m.locals[l.index()];
            let (name, source_name, body, span) = (
                format!("{}_raw", data.name),
                data.source_name.clone(),
                data.body,
                data.span,
            );
            let raw = LocalId(self.m.locals.len() as u32);
            self.m.locals.push(LocalData {
                name,
                source_name,
                ty: t,
                body,
                span,
            });
            let unboxed = self.strip_box(init, t);
            let let_raw = self.node(ExprKind::Let(raw, unboxed), TypeId::UNIT, e);
            let raw_ref = self.node(ExprKind::Local(raw), t, e);
            let boxed = self.node(ExprKind::Cast(raw_ref), TypeId::ANY, e);
            let let_v = self.node(ExprKind::Let(l, boxed), TypeId::UNIT, e);
            self.m.exprs[e.index()].kind = ExprKind::Block(vec![let_raw, let_v], None);
        }
    }

    /// The type of the value boxed into `any` at the end of `e`, when that is the same type on
    /// every path: `x as any`, possibly as the value of blocks and of both branches of an `if`.
    fn boxed_source(&self, e: ExprId) -> Option<TypeId> {
        match self.kind(e) {
            ExprKind::Cast(x) if self.ty(e) == TypeId::ANY => {
                let t = self.ty(*x);
                decided(self.m.kind(t)).then_some(t)
            }
            ExprKind::Block(_, Some(t)) => self.boxed_source(*t),
            ExprKind::If(_, a, Some(b)) => {
                match (self.ty(*a) == TypeId::NEVER, self.ty(*b) == TypeId::NEVER) {
                    (true, _) => self.boxed_source(*b),
                    (_, true) => self.boxed_source(*a),
                    _ => {
                        let t = self.boxed_source(*a)?;
                        (self.boxed_source(*b)? == t).then_some(t)
                    }
                }
            }
            _ => None,
        }
    }

    /// `e` without its final boxing (see [`Opt::boxed_source`]), of type `t`.
    fn strip_box(&mut self, e: ExprId, t: TypeId) -> ExprId {
        if self.ty(e) == TypeId::NEVER {
            return e;
        }
        match self.kind(e).clone() {
            ExprKind::Cast(x) => x,
            ExprKind::Block(items, Some(tail)) => {
                let tail = self.strip_box(tail, t);
                self.node(ExprKind::Block(items, Some(tail)), t, e)
            }
            ExprKind::If(c, a, Some(b)) => {
                let a = self.strip_box(a, t);
                let b = self.strip_box(b, t);
                self.node(ExprKind::If(c, a, Some(b)), t, e)
            }
            _ => unreachable!("boxed_source accepted it"),
        }
    }

    /// Simplifies the tree under `e` bottom-up and returns what replaces `e`.
    fn simplify(&mut self, e: ExprId, uses: &Uses, replace: &HashMap<LocalId, Replace>) -> ExprId {
        let kind = self.kind(e).clone();
        // Children first.
        let kind = match kind {
            ExprKind::Match(s, arms) => {
                let s = self.simplify(s, uses, replace);
                for &a in &arms {
                    let b = self.m.arms[a.index()].body;
                    self.m.arms[a.index()].body = self.simplify(b, uses, replace);
                }
                ExprKind::Match(s, arms)
            }
            ExprKind::Lambda(b) => {
                // A closure body is simplified with its own function's bodies.
                ExprKind::Lambda(b)
            }
            k => map_children(&k, &mut |c| self.simplify(c, uses, replace)),
        };
        self.m.exprs[e.index()].kind = kind.clone();
        let ty = self.ty(e);
        match kind {
            ExprKind::Local(l) => match replace.get(&l) {
                Some(Replace::Local(q)) => {
                    self.m.exprs[e.index()].kind = ExprKind::Local(*q);
                    e
                }
                Some(Replace::Const(k)) => {
                    self.m.exprs[e.index()].kind = k.clone();
                    e
                }
                Some(Replace::Boxed(..)) | None => e,
            },
            ExprKind::Cast(x) if self.ty(x) == ty => x,
            ExprKind::Cast(x) => match self.kind(x).clone() {
                // `(y as any) as T` with `y: T` is `y`.
                ExprKind::Cast(y) if self.ty(x) == TypeId::ANY && self.ty(y) == ty => y,
                ExprKind::Local(l) => match replace.get(&l) {
                    Some(Replace::Boxed(raw, t)) if *t == ty => {
                        self.m.exprs[e.index()].kind = ExprKind::Local(*raw);
                        e
                    }
                    _ => e,
                },
                _ => e,
            },
            ExprKind::Is(x, t) => {
                let known = match self.kind(x).clone() {
                    ExprKind::Cast(y) if self.ty(x) == TypeId::ANY => {
                        known_test(self.m, self.ty(y), t).map(|v| (v, Some(y)))
                    }
                    ExprKind::Local(l) => match replace.get(&l) {
                        Some(Replace::Boxed(_, yt)) => {
                            known_test(self.m, *yt, t).map(|v| (v, None))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                match known {
                    Some((v, effects)) => {
                        let c = self.node(ExprKind::Bool(v), TypeId::BOOL, e);
                        match effects {
                            Some(y) => self.with_effects_of(y, c),
                            None => c,
                        }
                    }
                    None => e,
                }
            }
            ExprKind::Unary(UnOp::Not, x) => match self.kind(x).clone() {
                ExprKind::Bool(v) => {
                    self.m.exprs[e.index()].kind = ExprKind::Bool(!v);
                    e
                }
                ExprKind::Unary(UnOp::Not, y) => y,
                _ => e,
            },
            ExprKind::And(a, b) => match (self.kind(a).clone(), self.kind(b).clone()) {
                (ExprKind::Bool(true), _) => b,
                (ExprKind::Bool(false), _) => a,
                (_, ExprKind::Bool(true)) => a,
                _ => e,
            },
            ExprKind::Or(a, b) => match (self.kind(a).clone(), self.kind(b).clone()) {
                (ExprKind::Bool(false), _) => b,
                (ExprKind::Bool(true), _) => a,
                (_, ExprKind::Bool(false)) => a,
                _ => e,
            },
            ExprKind::If(c, t, f) => match (self.kind(c).clone(), f) {
                (ExprKind::Bool(true), Some(_)) => t,
                (ExprKind::Bool(false), Some(f)) => f,
                (ExprKind::Bool(true), None) => self.node(ExprKind::Block(vec![t], None), ty, e),
                (ExprKind::Bool(false), None) => self.node(ExprKind::Unit, TypeId::UNIT, e),
                (ExprKind::Unary(UnOp::Not, c2), Some(f)) => {
                    self.m.exprs[e.index()].kind = ExprKind::If(c2, f, Some(t));
                    e
                }
                _ => e,
            },
            ExprKind::Block(items, tail) => self.simplify_block(e, items, tail, uses),
            _ => e,
        }
    }

    /// `result`, after evaluating `effects` if that has any effect.
    fn with_effects_of(&mut self, effects: ExprId, result: ExprId) -> ExprId {
        if self.pure(effects) {
            return result;
        }
        let ty = self.ty(result);
        self.node(ExprKind::Block(vec![effects], Some(result)), ty, result)
    }

    fn simplify_block(
        &mut self,
        e: ExprId,
        items: Vec<ExprId>,
        tail: Option<ExprId>,
        uses: &Uses,
    ) -> ExprId {
        let mut out = Vec::new();
        let mut tail = tail;
        let mut dead = false;
        for item in items {
            if dead {
                break;
            }
            match self.kind(item).clone() {
                // A nested block without a value: its statements, in place.
                ExprKind::Block(inner, None) => {
                    for i in inner {
                        dead |= self.ty(i) == TypeId::NEVER;
                        out.push(i);
                    }
                }
                ExprKind::Let(l, init)
                    if !uses.reads.contains_key(&l)
                        && !uses.writes.contains_key(&l)
                        && !uses.captured.contains(&l) =>
                {
                    // An unused local: keep only the initializer's effects.
                    if !self.pure(init) {
                        dead |= self.ty(init) == TypeId::NEVER;
                        out.push(init);
                    }
                }
                _ if self.pure(item) => {}
                _ => {
                    dead |= self.ty(item) == TypeId::NEVER;
                    out.push(item);
                }
            }
        }
        if dead {
            // Code after something that never finishes does not run.
            tail = None;
        }
        let ty = self.ty(e);
        if out.is_empty()
            && let Some(t) = tail
            && self.ty(t) == ty
        {
            return t;
        }
        self.m.exprs[e.index()].kind = ExprKind::Block(out, tail);
        e
    }

    /// Whether evaluating `e` has no effect besides producing its value (allocation is not an
    /// effect: it cannot be observed): no assignment, call, output, trap or panic.
    fn pure(&self, e: ExprId) -> bool {
        match self.kind(e) {
            ExprKind::Unit
            | ExprKind::Bool(_)
            | ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Null
            | ExprKind::Local(_)
            | ExprKind::Global(_)
            | ExprKind::Func(..)
            | ExprKind::Lambda(_) => true,
            ExprKind::Cast(x) => {
                // Unpacking an `any` checks its type and can panic.
                let from = self.ty(*x);
                from != TypeId::ANY && self.pure(*x)
            }
            ExprKind::Binary(op, a, b) => {
                let traps = matches!(op, BinOp::Div | BinOp::Rem) && self.ty(*a) == TypeId::INT;
                !traps && self.pure(*a) && self.pure(*b)
            }
            ExprKind::Unary(_, a) | ExprKind::Is(a, _) => self.pure(*a),
            ExprKind::And(a, b) | ExprKind::Or(a, b) => self.pure(*a) && self.pure(*b),
            ExprKind::Block(items, tail) => {
                items.iter().all(|&i| self.pure(i)) && tail.is_none_or(|t| self.pure(t))
            }
            _ => false,
        }
    }
}

/// `kind` with each child expression replaced by `f(child)` (match arms are left to the caller).
fn map_children(kind: &ExprKind, f: &mut dyn FnMut(ExprId) -> ExprId) -> ExprKind {
    use ExprKind::*;
    let all = |v: &[ExprId], f: &mut dyn FnMut(ExprId) -> ExprId| v.iter().map(|&x| f(x)).collect();
    match kind {
        Unit | Bool(_) | Int(_) | Float(_) | Str(_) | Null | Local(_) | Global(_) | Func(..)
        | Lambda(_) | NewArray | NewMap | Break | Continue => kind.clone(),
        Let(l, a) => Let(*l, f(*a)),
        SetLocal(l, a) => SetLocal(*l, f(*a)),
        SetGlobal(g, a) => SetGlobal(*g, f(*a)),
        Unary(op, a) => Unary(*op, f(*a)),
        Field(a, i) => Field(f(*a), *i),
        FieldName(a, n) => FieldName(f(*a), n.clone()),
        Cast(a) => Cast(f(*a)),
        Unwrap(a) => Unwrap(f(*a)),
        Is(a, t) => Is(f(*a), *t),
        Loop(a) => Loop(f(*a)),
        SetField(a, i, b) => {
            let a = f(*a);
            SetField(a, *i, f(*b))
        }
        SetFieldName(a, n, b) => {
            let a = f(*a);
            SetFieldName(a, n.clone(), f(*b))
        }
        Binary(op, a, b) => {
            let a = f(*a);
            Binary(*op, a, f(*b))
        }
        And(a, b) => {
            let a = f(*a);
            And(a, f(*b))
        }
        Or(a, b) => {
            let a = f(*a);
            Or(a, f(*b))
        }
        Index(a, b) => {
            let a = f(*a);
            Index(a, f(*b))
        }
        While(a, b) => {
            let a = f(*a);
            While(a, f(*b))
        }
        SetIndex(a, b, c) => {
            let a = f(*a);
            let b = f(*b);
            SetIndex(a, b, f(*c))
        }
        Call(g, targs, args) => Call(*g, targs.clone(), all(args, f)),
        CallExtern(x, args) => CallExtern(*x, all(args, f)),
        New(s, args) => New(*s, all(args, f)),
        Variant(en, v, args) => Variant(*en, *v, all(args, f)),
        Array(args) => Array(all(args, f)),
        Builtin(op, args) => Builtin(*op, all(args, f)),
        CallValue(c, args) => {
            let c = f(*c);
            CallValue(c, all(args, f))
        }
        Match(s, arms) => Match(f(*s), arms.clone()),
        If(c, t, e) => {
            let c = f(*c);
            let t = f(*t);
            If(c, t, e.map(&mut *f))
        }
        IfLet(l, v, t, e) => {
            let v = f(*v);
            let t = f(*t);
            IfLet(*l, v, t, e.map(&mut *f))
        }
        Return(v) => Return(v.map(&mut *f)),
        Block(items, tail) => {
            let items = all(items, f);
            Block(items, tail.map(&mut *f))
        }
    }
}

/// Whether a value of this type, boxed into `any`, has a type known at compile time.
fn decided(k: &TypeKind) -> bool {
    !matches!(
        k,
        TypeKind::Any
            | TypeKind::Nullable(_)
            | TypeKind::Never
            | TypeKind::Error
            | TypeKind::Var(_)
            | TypeKind::Param(..)
    )
}

/// `(y as any) is t` for a `y` of type `yt`, when the type decides it.
fn known_test(m: &HirModule, yt: TypeId, t: TypeId) -> Option<bool> {
    if !decided(m.kind(yt)) {
        return None;
    }
    Some(match (m.kind(yt), m.kind(t)) {
        (TypeKind::Struct(a, _), TypeKind::Struct(b, _)) => a == b,
        (TypeKind::Enum(a, _), TypeKind::Enum(b, _)) => a == b,
        (_, TypeKind::Nullable(_)) => return None,
        _ => yt == t,
    })
}
