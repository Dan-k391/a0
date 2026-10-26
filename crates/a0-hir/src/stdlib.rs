//! The standard library (D48): modules written in the HIR text format (`std/*.hir`) over C
//! functions of the runtime (`a0-runtime/c/std.c`), which every language can offer. A module is
//! added to a program the first time the program imports it (`import_module("std.fs", "fs")`),
//! so programs pay only for what they use.

use a0_diag::SourceId;

use crate::builder::HirBuilder;
use crate::model::*;
use crate::text;

/// The modules: name, file, text, and the modules it uses.
const MODULES: &[(&str, &str, &str, &[&str])] = &[
    ("std", "std.hir", include_str!("../std/std.hir"), &[]),
    ("std.io", "io.hir", include_str!("../std/io.hir"), &[]),
    ("std.fs", "fs.hir", include_str!("../std/fs.hir"), &["std"]),
    ("std.env", "env.hir", include_str!("../std/env.hir"), &[]),
    ("std.time", "time.hir", include_str!("../std/time.hir"), &[]),
    ("std.math", "math.hir", include_str!("../std/math.hir"), &[]),
    (
        "std.str",
        "str.hir",
        include_str!("../std/str.hir"),
        &["std"],
    ),
    (
        "std.thread",
        "thread.hir",
        include_str!("../std/thread.hir"),
        &["std"],
    ),
];

/// The source of a module of the standard library, a built-in source every `SourceMap` knows
/// (`a0_diag::register_builtin_source`), named `<std>/fs.hir`: diagnostics, stack traces and
/// definitions can point into it.
pub fn std_source(name: &str) -> Option<SourceId> {
    let index = MODULES.iter().position(|m| m.0 == name)?;
    let (_, file, text, _) = MODULES[index];
    Some(a0_diag::register_builtin_source(
        index as u32,
        &format!("<std>/{file}"),
        text,
    ))
}

/// The names of the standard library's modules.
pub fn std_modules() -> Vec<&'static str> {
    MODULES.iter().map(|m| m.0).collect()
}

/// Whether `name` is a module of the standard library.
pub fn is_std_module(name: &str) -> bool {
    MODULES.iter().any(|m| m.0 == name)
}

/// The text of a module of the standard library.
pub fn std_module_text(name: &str) -> Option<&'static str> {
    MODULES.iter().find(|m| m.0 == name).map(|m| m.2)
}

impl HirBuilder {
    /// Adds the standard library's module `name` (and the modules it uses) unless the program has
    /// it already. Declarations the frontend makes afterwards go where they went before.
    pub(crate) fn load_std(&mut self, name: &str) -> Option<ModuleId> {
        if let Some(m) = self.lookup_module(name) {
            return Some(m);
        }
        let &(_, _, text, uses) = MODULES.iter().find(|m| m.0 == name)?;
        if !self.body_stack.is_empty() {
            self.misuse(&format!(
                "the standard library's {name} is imported inside a body; import it before"
            ));
            return None;
        }
        for &used in uses {
            self.load_std(used);
        }
        let saved = (self.module, self.public, self.span);
        let type_scope = std::mem::take(&mut self.type_scope);
        let id = self.begin_module(name, false);
        if let Err(d) = text::include(self, text, std_source(name), true) {
            panic!("the standard library's {name} is invalid:\n{}", d.summary());
        }
        (self.module, self.public, self.span) = saved;
        self.type_scope = type_scope;
        Some(id)
    }
}
