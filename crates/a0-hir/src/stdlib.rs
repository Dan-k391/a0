//! The standard library (D48): modules written in the HIR text format (`std/*.hir`) over C
//! functions of the runtime (`a0-runtime/c/std.c`), which every language can offer. A module is
//! added to a program the first time the program imports it (`import_module("std.fs", "fs")`),
//! so programs pay only for what they use.

use crate::builder::HirBuilder;
use crate::model::*;
use crate::text;

/// The modules: name, text, and the modules it uses.
const MODULES: &[(&str, &str, &[&str])] = &[
    ("std", include_str!("../std/std.hir"), &[]),
    ("std.io", include_str!("../std/io.hir"), &[]),
    ("std.fs", include_str!("../std/fs.hir"), &["std"]),
    ("std.env", include_str!("../std/env.hir"), &[]),
    ("std.time", include_str!("../std/time.hir"), &[]),
    ("std.math", include_str!("../std/math.hir"), &[]),
    ("std.str", include_str!("../std/str.hir"), &["std"]),
];

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
    MODULES.iter().find(|m| m.0 == name).map(|m| m.1)
}

impl HirBuilder {
    /// Adds the standard library's module `name` (and the modules it uses) unless the program has
    /// it already. Declarations the frontend makes afterwards go where they went before.
    pub(crate) fn load_std(&mut self, name: &str) -> Option<ModuleId> {
        if let Some(m) = self.lookup_module(name) {
            return Some(m);
        }
        let &(_, source, uses) = MODULES.iter().find(|m| m.0 == name)?;
        if !self.body_stack.is_empty() {
            self.misuse(&format!(
                "the standard library's {name} is imported inside a body; import it before"
            ));
            return None;
        }
        for &used in uses {
            self.load_std(used);
        }
        // The library has no spans, so warnings never point into it.
        let saved = (self.module, self.public, self.span);
        let type_scope = std::mem::take(&mut self.type_scope);
        let id = self.begin_module(name, false);
        if let Err(d) = text::include(self, source, None, true) {
            panic!("the standard library's {name} is invalid:\n{}", d.summary());
        }
        (self.module, self.public, self.span) = saved;
        self.type_scope = type_scope;
        Some(id)
    }
}
