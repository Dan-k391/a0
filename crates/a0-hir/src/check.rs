//! The HIR checker: the rules that per-node typing cannot see (returns against the enclosing body,
//! `break` placement, which bodies may use a local, tree shape), plus re-typing every node.

use a0_diag::{Diagnostic, Diagnostics, Span};

use crate::builder::{fits, infer, visit_children};
use crate::model::*;

/// Checks a whole module.
pub fn check(m: &HirModule) -> Diagnostics {
    let mut c = Checker {
        m,
        diags: Diagnostics::new(),
        uses: vec![0; m.exprs.len()],
        body_uses: vec![0; m.bodies.len()],
    };
    for f in 0..m.funcs.len() {
        let body = m.funcs[f].body;
        c.body(body, m.funcs[f].span);
    }
    for (i, &n) in c.uses.iter().enumerate() {
        if n > 1 {
            let span = m.exprs[i].span;
            c.diags.push(
                Diagnostic::error(
                    "hir.tree",
                    "an expression is used in more than one place; build a new node instead",
                )
                .with_span_opt(span),
            );
            break;
        }
    }
    for (i, &n) in c.body_uses.iter().enumerate() {
        if n == 0 && m.bodies[i].parent.is_some() {
            c.diags.push(Diagnostic::error(
                "hir.tree",
                "a closure body is never turned into a closure",
            ));
        }
    }
    c.diags
}

/// Checks that the module has an entry point: `fn main() -> int` or `fn main()`.
pub fn check_entry(m: &HirModule) -> Diagnostics {
    let mut diags = Diagnostics::new();
    match m.lookup_value("main") {
        Some(Item::Func(f)) => {
            let body = m.func_body(f);
            let ret = m.body_ret(body);
            if !m.body_params(body).is_empty() || !matches!(ret, TypeId::INT | TypeId::UNIT) {
                diags.push(
                    Diagnostic::error("hir.entry", "main must be fn main() -> int or fn main()")
                        .with_span_opt(m.funcs[f.index()].span),
                );
            }
        }
        _ => diags.error("hir.entry", "the module has no fn main", None),
    }
    diags
}

struct Checker<'a> {
    m: &'a HirModule,
    diags: Diagnostics,
    uses: Vec<u32>,
    body_uses: Vec<u32>,
}

impl Checker<'_> {
    fn error(&mut self, e: ExprId, message: &str) {
        let span = self.m.exprs[e.index()].span;
        self.diags
            .push(Diagnostic::error("hir.check", message).with_span_opt(span));
    }

    fn body(&mut self, body: BodyId, span: Option<Span>) {
        let m = self.m;
        let Some(root) = m.body_root(body) else {
            self.diags.push(
                Diagnostic::error("hir.check", "a function or closure has no body")
                    .with_span_opt(span),
            );
            return;
        };
        let ret = m.body_ret(body);
        let rt = m.expr_type(root);
        if ret != TypeId::UNIT && !fits(rt, ret) {
            let msg = format!(
                "the body evaluates to {} but must return {}",
                m.type_name(rt),
                m.type_name(ret)
            );
            self.error(root, &msg);
        }
        self.expr(root, body, 0, false);
    }

    /// Whether `local` may be used from `body`: it belongs to it or to an enclosing body.
    fn visible(&self, local: LocalId, mut body: BodyId) -> bool {
        let owner = self.m.locals[local.index()].body;
        loop {
            if body == owner {
                return true;
            }
            match self.m.bodies[body.index()].parent {
                Some(p) => body = p,
                None => return false,
            }
        }
    }

    /// `item` is true for the statements of a block, the only place a `let` may appear.
    fn expr(&mut self, e: ExprId, body: BodyId, loops: u32, item: bool) {
        let m = self.m;
        self.uses[e.index()] += 1;
        let data = m.expr(e);
        if matches!(data.kind, ExprKind::Let(..)) && !item {
            self.error(e, "let can only be a statement of a block");
        }
        // Re-type the node; a node typed `never` because of an earlier error is not reported again.
        if data.ty != TypeId::NEVER {
            match infer(m, &data.kind, data.ty) {
                Ok(t) if t == data.ty => {}
                Ok(t) => {
                    let msg = format!(
                        "expression typed {} should be {}",
                        m.type_name(data.ty),
                        m.type_name(t)
                    );
                    self.error(e, &msg);
                }
                Err(msg) => self.error(e, &msg),
            }
        }
        match &data.kind {
            ExprKind::Local(l) | ExprKind::SetLocal(l, _) => {
                if !self.visible(*l, body) {
                    let msg = format!(
                        "'{}' is used outside the function that declares it",
                        m.local_name(*l)
                    );
                    self.error(e, &msg);
                }
            }
            ExprKind::Let(l, _) => {
                if m.locals[l.index()].body != body {
                    let msg = format!(
                        "'{}' is declared with let in a different body",
                        m.local_name(*l)
                    );
                    self.error(e, &msg);
                }
            }
            ExprKind::Return(v) => {
                let ret = m.body_ret(body);
                match v {
                    Some(v) if ret != TypeId::UNIT && !fits(m.expr_type(*v), ret) => {
                        let msg = format!(
                            "return {} from a body returning {}",
                            m.type_name(m.expr_type(*v)),
                            m.type_name(ret)
                        );
                        self.error(e, &msg);
                    }
                    None if ret != TypeId::UNIT => {
                        let msg = format!("return needs a value of type {}", m.type_name(ret));
                        self.error(e, &msg);
                    }
                    _ => {}
                }
            }
            ExprKind::Break | ExprKind::Continue if loops == 0 => {
                self.error(e, "break or continue outside a loop");
            }
            ExprKind::Lambda(b) => {
                self.body_uses[b.index()] += 1;
                if m.bodies[b.index()].parent != Some(body) {
                    self.error(e, "a closure is used outside the body it was created in");
                }
                self.body(*b, data.span);
                return;
            }
            ExprKind::Match(_, arms) => {
                for a in arms {
                    for &l in &m.arms[a.index()].binds {
                        if m.locals[l.index()].body != body {
                            self.error(e, "a match binding belongs to a different body");
                        }
                    }
                }
            }
            _ => {}
        }
        let inner_loops = match &data.kind {
            ExprKind::While(..) | ExprKind::Loop(_) => loops + 1,
            _ => loops,
        };
        let items: Vec<ExprId> = match &data.kind {
            ExprKind::Block(items, _) => items.clone(),
            _ => Vec::new(),
        };
        let mut children = Vec::new();
        visit_children(m, e, &mut |c| children.push(c));
        for c in children {
            // The condition of a while is outside its loop.
            let depth = match &data.kind {
                ExprKind::While(cond, _) if *cond == c => loops,
                _ => inner_loops,
            };
            self.expr(c, body, depth, items.contains(&c));
        }
    }
}
