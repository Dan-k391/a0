//! The HIR checker: every node typed again on the finished module, plus the rules that per-node
//! typing cannot see (returns against the enclosing body, `break` placement, which bodies may use a
//! local, tree shape).

use a0_diag::codes as code;
use a0_diag::{Diagnostic, Diagnostics, Span};

use crate::builder::{Problem, TypeErr, infer, visit_children};
use crate::model::*;

/// Checks a whole module. Typing nodes again may intern types, hence `&mut`.
pub fn check(m: &mut HirModule) -> Diagnostics {
    let mut diags = retype(m);
    let m = &*m;
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
    diags.extend(c.diags);
    diags
}

/// Types every node reachable from a function again. A node typed `never` because of an earlier
/// error, or one whose type could not be inferred (already reported), is skipped.
fn retype(m: &mut HirModule) -> Diagnostics {
    let mut nodes = Vec::new();
    for f in 0..m.funcs.len() {
        reachable(m, m.funcs[f].body, &mut nodes);
    }
    let mut diags = Diagnostics::new();
    for e in nodes {
        let (kind, ty) = (m.exprs[e.index()].kind.clone(), m.exprs[e.index()].ty);
        if ty == TypeId::NEVER || ty == TypeId::ERROR || !m.is_resolved(ty) {
            continue;
        }
        let (at, problem) = match infer(m, &kind, ty) {
            Ok(t) if m.same(t, ty) => continue,
            Ok(t) => (
                e,
                Problem::new(
                    "hir.check",
                    format!(
                        "expression typed {} should be {}",
                        m.display_type(ty),
                        m.display_type(t)
                    ),
                ),
            ),
            Err(TypeErr::At(at, p)) => (at.unwrap_or(e), p),
            Err(TypeErr::Open(_)) => continue,
        };
        let span = m.exprs[at.index()].span;
        diags.push(problem.diagnostic(span, None));
    }
    diags
}

fn reachable(m: &HirModule, body: BodyId, out: &mut Vec<ExprId>) {
    fn walk(m: &HirModule, e: ExprId, out: &mut Vec<ExprId>) {
        out.push(e);
        if let ExprKind::Lambda(b) = m.expr(e).kind {
            reachable(m, b, out);
        }
        visit_children(m, e, &mut |c| walk(m, c, out));
    }
    if let Some(root) = m.body_root(body) {
        walk(m, root, out);
    }
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
                    Diagnostic::error(
                        code::BAD_ENTRY,
                        "`main` must take no parameters and return nothing or an integer",
                    )
                    .with_span_opt(m.funcs[f.index()].span),
                );
            }
        }
        _ => diags.error(code::NO_ENTRY, "the program has no `main` function", None),
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
    /// A malformed module: a bug in the language that built it.
    fn error(&mut self, e: ExprId, message: &str) {
        self.problem(e, &Problem::new("hir.check", message));
    }

    fn problem(&mut self, e: ExprId, p: &Problem) {
        let span = self.m.exprs[e.index()].span;
        self.diags.push(p.diagnostic(span, None));
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
        // `HirBuilder::set_root` has checked the body's value against its result type.
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
            ExprKind::Let(l, _) | ExprKind::IfLet(l, ..) => {
                if m.locals[l.index()].body != body {
                    let msg = format!(
                        "'{}' is declared with let in a different body",
                        m.local_name(*l)
                    );
                    self.error(e, &msg);
                }
            }
            ExprKind::Return(None) if m.body_ret(body) != TypeId::UNIT => {
                let ret = m.quote_type(m.body_ret(body));
                let decl = match m.bodies[body.index()].parent {
                    None => m.funcs[m.bodies[body.index()].func.index()].span,
                    Some(_) => None,
                };
                let p = Problem::new(
                    code::BAD_RESULT,
                    format!(
                        "this {} must return a value of type {ret}",
                        m.term(Term::Function)
                    ),
                )
                .label(format!("returns no value, but {ret} is expected"))
                .at(decl, format!("returns {ret}"));
                self.problem(e, &p);
            }
            ExprKind::Break | ExprKind::Continue if loops == 0 => {
                let what = m.term(match data.kind {
                    ExprKind::Break => Term::Break,
                    _ => Term::Continue,
                });
                let p = Problem::new(code::OUTSIDE_LOOP, format!("{what} outside of a loop"))
                    .label(format!("cannot {what} outside of a loop"));
                self.problem(e, &p);
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
