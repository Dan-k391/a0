//! Syntax trees produced by [`crate::Grammar::parse`].

use std::fmt::Write as _;
use std::sync::Arc;

use a0_diag::{SourceId, Span};

use crate::grammar::{FieldId, KindId, Names};

/// A node in a [`SyntaxTree`]: a token (leaf) or a rule/operator node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    pub const NONE: NodeId = NodeId(u32::MAX);

    pub fn is_none(self) -> bool {
        self.0 == u32::MAX
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NodeData {
    pub kind: KindId,
    pub start: u32,
    pub end: u32,
    pub children_start: u32,
    pub children_len: u32,
    pub is_token: bool,
}

/// A concrete syntax tree. Every token that was parsed is a leaf; nodes carry their kind (the rule or
/// operator that produced them), their span and their children, which may be labeled with fields.
#[derive(Clone)]
pub struct SyntaxTree {
    pub(crate) text: String,
    pub(crate) source: SourceId,
    pub(crate) nodes: Vec<NodeData>,
    pub(crate) children: Vec<(NodeId, FieldId)>,
    pub(crate) root: NodeId,
    pub(crate) names: Arc<Names>,
}

impl SyntaxTree {
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// The number of nodes; `NodeId`s are below it.
    pub fn node_count(&self) -> u32 {
        self.nodes.len() as u32
    }

    pub fn field_name(&self, field: FieldId) -> &str {
        &self.names.fields[field.0 as usize]
    }

    pub fn source(&self) -> SourceId {
        self.source
    }

    /// The same tree for the same text under another source id (a tree kept from an earlier
    /// source map, `a0_driver::program::TreeCache`): spans are made from it.
    pub fn set_source(&mut self, source: SourceId) {
        self.source = source;
    }

    /// The full source text the tree was parsed from.
    pub fn source_text(&self) -> &str {
        &self.text
    }

    pub fn kind(&self, n: NodeId) -> KindId {
        self.nodes[n.0 as usize].kind
    }

    pub fn kind_name(&self, n: NodeId) -> &str {
        &self.names.kinds[self.kind(n).0 as usize]
    }

    pub fn is_token(&self, n: NodeId) -> bool {
        self.nodes[n.0 as usize].is_token
    }

    pub fn is_error(&self, n: NodeId) -> bool {
        self.kind(n) == KindId::ERROR
    }

    pub fn span(&self, n: NodeId) -> Span {
        let d = &self.nodes[n.0 as usize];
        Span::new(self.source, d.start, d.end)
    }

    /// The source text covered by `n`.
    pub fn text(&self, n: NodeId) -> &str {
        let d = &self.nodes[n.0 as usize];
        &self.text[d.start as usize..d.end as usize]
    }

    pub fn child_count(&self, n: NodeId) -> u32 {
        self.nodes[n.0 as usize].children_len
    }

    pub fn child(&self, n: NodeId, i: u32) -> NodeId {
        self.child_entry(n, i).0
    }

    /// The field label of the `i`th child, if it has one.
    pub fn child_field(&self, n: NodeId, i: u32) -> Option<FieldId> {
        let f = self.child_entry(n, i).1;
        (f != FieldId::NONE).then_some(f)
    }

    fn child_entry(&self, n: NodeId, i: u32) -> (NodeId, FieldId) {
        let d = &self.nodes[n.0 as usize];
        assert!(i < d.children_len, "child index out of range");
        self.children[(d.children_start + i) as usize]
    }

    /// All children of `n`.
    pub fn children(&self, n: NodeId) -> Vec<NodeId> {
        let d = &self.nodes[n.0 as usize];
        self.children[d.children_start as usize..(d.children_start + d.children_len) as usize]
            .iter()
            .map(|&(c, _)| c)
            .collect()
    }

    /// The first child labeled with `field`, if any.
    pub fn field(&self, n: NodeId, field: FieldId) -> Option<NodeId> {
        let d = &self.nodes[n.0 as usize];
        self.children[d.children_start as usize..(d.children_start + d.children_len) as usize]
            .iter()
            .find(|&&(_, f)| f == field)
            .map(|&(c, _)| c)
    }

    /// All children labeled with `field`, in order.
    pub fn fields(&self, n: NodeId, field: FieldId) -> Vec<NodeId> {
        let d = &self.nodes[n.0 as usize];
        self.children[d.children_start as usize..(d.children_start + d.children_len) as usize]
            .iter()
            .filter(|&&(_, f)| f == field)
            .map(|&(c, _)| c)
            .collect()
    }

    /// Like [`SyntaxTree::field`], looking the field up by name.
    pub fn field_named(&self, n: NodeId, name: &str) -> Option<NodeId> {
        let f = self.field_id(name)?;
        self.field(n, f)
    }

    /// Like [`SyntaxTree::fields`], looking the field up by name.
    pub fn fields_named(&self, n: NodeId, name: &str) -> Vec<NodeId> {
        match self.field_id(name) {
            Some(f) => self.fields(n, f),
            None => Vec::new(),
        }
    }

    pub fn field_id(&self, name: &str) -> Option<FieldId> {
        self.names
            .fields
            .iter()
            .position(|f| f == name)
            .map(|i| FieldId(i as u32))
    }

    /// The first child (token or node) of the given kind.
    pub fn child_of_kind(&self, n: NodeId, kind: KindId) -> Option<NodeId> {
        self.children(n).into_iter().find(|&c| self.kind(c) == kind)
    }

    /// The children of `n` that are nodes, not tokens (the statements of a block, the items of a
    /// program).
    pub fn nodes(&self, n: NodeId) -> Vec<NodeId> {
        let mut out = self.children(n);
        out.retain(|&c| !self.is_token(c));
        out
    }

    /// The children of `n` that are nodes of the rule `kind`: the unlabeled repetitions in
    /// `(param ("," param)*)`.
    pub fn nodes_of_kind(&self, n: NodeId, kind: &str) -> Vec<NodeId> {
        let mut out = self.children(n);
        out.retain(|&c| !self.is_token(c) && self.kind_name(c) == kind);
        out
    }

    /// Whether any node in the tree is an error node.
    pub fn has_errors(&self) -> bool {
        self.nodes.iter().any(|d| d.kind == KindId::ERROR)
    }

    /// A compact s-expression rendering, for tests and debugging:
    /// `(print_stmt 'print' value=(binary left='1' op='+' right='2') ';')`.
    pub fn to_sexpr(&self, n: NodeId) -> String {
        let mut out = String::new();
        self.sexpr_into(n, &mut out);
        out
    }

    fn sexpr_into(&self, n: NodeId, out: &mut String) {
        if self.is_token(n) {
            let _ = write!(out, "'{}'", self.text(n));
            return;
        }
        out.push('(');
        out.push_str(self.kind_name(n));
        for i in 0..self.child_count(n) {
            out.push(' ');
            if let Some(f) = self.child_field(n, i) {
                out.push_str(&self.names.fields[f.0 as usize]);
                out.push('=');
            }
            self.sexpr_into(self.child(n, i), out);
        }
        out.push(')');
    }
}
