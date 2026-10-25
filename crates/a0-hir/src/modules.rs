//! Modules: programs of several files, each with its own names.
//!
//! Module 0 is the prelude: everything declared before the first [`HirBuilder::begin_module`], seen
//! by every module. Each file of a program is a module begun with `begin_module`. The root file's
//! items keep their names (so its `main` is the program's entry point); another module's items are
//! named `module.item` in the HIR, in the IR and in stack traces. A module sees its own items, the
//! prelude, the items it imports by name, and through [`HirBuilder::member`] and its siblings the
//! public items of the modules it imports.

use a0_diag::codes as code;
use a0_diag::similar_name;

use crate::builder::{HirBuilder, Problem};
use crate::model::*;

impl HirBuilder {
    /// Starts a module: declarations from now on belong to it, and are public until
    /// [`HirBuilder::set_public`] says otherwise. `root` is the program's main file, whose items
    /// keep their names.
    pub fn begin_module(&mut self, name: &str, root: bool) -> ModuleId {
        let id = ModuleId(self.m.modules.len() as u32);
        self.m.modules.push(ModuleDef {
            name: name.to_string(),
            qualified: !root,
            imports: Vec::new(),
            uses: Vec::new(),
        });
        self.module = id;
        self.public = true;
        id
    }

    /// The module declarations go into now.
    pub fn current_module(&self) -> ModuleId {
        self.module
    }

    /// Whether declarations from now on can be used by modules that import this one.
    pub fn set_public(&mut self, public: bool) {
        self.public = public;
    }

    /// The module begun with `name`.
    pub fn lookup_module(&self, name: &str) -> Option<ModuleId> {
        self.m
            .modules
            .iter()
            .position(|m| m.name == name && m.qualified)
            .map(|i| ModuleId(i as u32))
    }

    /// Lets the current module refer to the public items of module `name` as `alias.item`.
    pub fn import_module(&mut self, name: &str, alias: &str) {
        let Some(target) = self.lookup_module(name).or_else(|| self.load_std(name)) else {
            self.problem(
                &Problem::new(code::UNKNOWN_MODULE, format!("unknown module `{name}`"))
                    .label("no module of this name"),
            );
            return;
        };
        let current = &self.m.modules[self.module.index()];
        if let Some(&(_, _, first)) = current.imports.iter().find(|(a, ..)| a == alias) {
            let p = Problem::new(code::DUPLICATE, format!("`{alias}` is imported twice"))
                .label("imported again here")
                .at(first, "first imported here");
            self.problem(&p);
            return;
        }
        let span = self.span;
        let current = &mut self.m.modules[self.module.index()];
        current.imports.push((alias.to_string(), target, span));
    }

    /// Imports the public item `name` of module `module` into the current module, where it is
    /// then known by its name alone.
    pub fn import_item(&mut self, module: &str, name: &str) {
        let Some(target) = self.lookup_module(module).or_else(|| self.load_std(module)) else {
            self.problem(
                &Problem::new(code::UNKNOWN_MODULE, format!("unknown module `{module}`"))
                    .label("no module of this name"),
            );
            return;
        };
        let value = self.public_item(target, name, false);
        let ty = self.public_item(target, name, true);
        if value.is_none() && ty.is_none() {
            self.unknown_member(target, name);
            return;
        }
        let current = &mut self.m.modules[self.module.index()];
        current.uses.push((name.to_string(), value, ty));
    }

    /// Whether `alias` names a module the current module imports.
    pub fn is_module(&self, alias: &str) -> bool {
        self.imported(alias).is_some()
    }

    fn imported(&self, alias: &str) -> Option<ModuleId> {
        self.m.modules[self.module.index()]
            .imports
            .iter()
            .find(|(a, ..)| a == alias)
            .map(|&(_, m, _)| m)
    }

    /// The name an item of module `m` called `name` has in the HIR.
    pub(crate) fn qualified_in(&self, m: ModuleId, name: &str) -> String {
        let def = &self.m.modules[m.index()];
        if def.qualified {
            format!("{}.{name}", def.name)
        } else {
            name.to_string()
        }
    }

    /// The name a declaration called `name` gets in the current module.
    pub(crate) fn qualified(&self, name: &str) -> String {
        self.qualified_in(self.module, name)
    }

    /// The item of module `m` called `name`, if it exists, public or not.
    fn item_of(&self, m: ModuleId, name: &str, types: bool) -> Option<Item> {
        let key = self.qualified_in(m, name);
        let map = if types {
            &self.m.type_names
        } else {
            &self.m.value_names
        };
        map.get(&key).copied().filter(|&i| self.m.home(i).0 == m)
    }

    fn public_item(&self, m: ModuleId, name: &str, types: bool) -> Option<Item> {
        self.item_of(m, name, types).filter(|&i| self.m.home(i).1)
    }

    /// What `name` means in the current module, as a value or a type: its own item, an item it
    /// imported by name, or one of the prelude.
    pub(crate) fn visible(&self, name: &str, types: bool) -> Option<Item> {
        if let Some(i) = self.item_of(self.module, name, types) {
            return Some(i);
        }
        let uses = &self.m.modules[self.module.index()].uses;
        if let Some(&(_, value, ty)) = uses.iter().find(|(n, ..)| n == name)
            && let Some(i) = if types { ty } else { value }
        {
            return Some(i);
        }
        if self.module.0 != 0 {
            return self.item_of(ModuleId(0), name, types);
        }
        None
    }

    /// The names the current module can see, for suggestions.
    pub(crate) fn visible_names(&self, types: bool) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let map = if types {
            &self.m.type_names
        } else {
            &self.m.value_names
        };
        let mine = &self.m.modules[self.module.index()];
        for (qualified, &item) in map {
            let (home, _) = self.m.home(item);
            if home == self.module || home.0 == 0 {
                let def = &self.m.modules[home.index()];
                let short = match def.qualified {
                    true => qualified
                        .strip_prefix(&format!("{}.", def.name))
                        .unwrap_or(qualified),
                    false => qualified,
                };
                out.push(short.to_string());
            }
        }
        out.extend(mine.uses.iter().map(|(n, ..)| n.clone()));
        out.sort();
        out.dedup();
        out
    }

    /// The item `alias.name`: a public item of the module the current module imports as `alias`.
    /// Reports what is wrong (an unknown module, an unknown or private item) and returns `None`.
    pub fn member_item(&mut self, alias: &str, name: &str, types: bool) -> Option<Item> {
        let Some(target) = self.imported(alias) else {
            let aliases: Vec<String> = self.m.modules[self.module.index()]
                .imports
                .iter()
                .map(|(a, ..)| a.clone())
                .collect();
            let aliases: Vec<&str> = aliases.iter().map(String::as_str).collect();
            let p = Problem::new(code::UNKNOWN_MODULE, format!("unknown module `{alias}`"))
                .label("not imported here")
                .similar("a module", alias, similar_name(alias, &aliases));
            self.problem(&p);
            return None;
        };
        match self.item_of(target, name, types) {
            Some(item) if self.m.home(item).1 => Some(item),
            Some(item) => {
                let module = self.m.modules[target.index()].name.clone();
                let p = Problem::new(
                    code::PRIVATE,
                    format!("`{name}` is private to module `{module}`"),
                )
                .label("private")
                .at(self.m.item_span(item), "declared here without being public");
                let span = self.take_name_span(name);
                self.diags.push(p.diagnostic(span.or(self.span), None));
                None
            }
            None => {
                self.unknown_member(target, name);
                None
            }
        }
    }

    fn unknown_member(&mut self, target: ModuleId, name: &str) {
        let module = self.m.modules[target.index()].name.clone();
        let mut names: Vec<String> = Vec::new();
        for map in [&self.m.value_names, &self.m.type_names] {
            for (qualified, &item) in map {
                let (home, public) = self.m.home(item);
                if home == target && public {
                    let short = qualified
                        .strip_prefix(&format!("{module}."))
                        .unwrap_or(qualified);
                    names.push(short.to_string());
                }
            }
        }
        names.sort();
        let candidates: Vec<&str> = names.iter().map(String::as_str).collect();
        let span = self.take_name_span(name);
        let p = Problem::new(
            code::UNKNOWN_NAME,
            format!("module `{module}` has no public `{name}`"),
        );
        let p = match similar_name(name, &candidates) {
            Some(close) => p.similar("a public item", name, Some(close)),
            None => p.label("not found in the module"),
        };
        self.diags.push(p.diagnostic(span.or(self.span), span));
    }

    /// The value `alias.name`: a public function (as a closure value) or global of an imported
    /// module.
    pub fn member(&mut self, alias: &str, name: &str) -> ExprId {
        match self.member_item(alias, name, false) {
            Some(Item::Global(g)) => self.global(g),
            Some(Item::Func(f)) => self.func(f),
            Some(Item::Extern(_)) => {
                self.problem(&Problem::new(
                    code::NOT_A_VALUE,
                    format!("the C function `{alias}.{name}` can only be called"),
                ));
                self.error_expr()
            }
            _ => self.error_expr(),
        }
    }

    /// Calls `alias.name(args)`: a public function, C function or global closure of an imported
    /// module.
    pub fn call_member(&mut self, alias: &str, name: &str, args: &[ExprId]) -> ExprId {
        match self.member_item(alias, name, false) {
            Some(Item::Func(f)) => self.call(f, args),
            Some(Item::Extern(x)) => self.call_extern(x, args),
            Some(Item::Global(g)) => {
                let c = self.global(g);
                self.call_value(c, args)
            }
            _ => self.error_expr(),
        }
    }

    /// The type `alias.name`, with type arguments (inferred ones if `args` is empty).
    pub fn member_type(&mut self, alias: &str, name: &str, args: &[TypeId]) -> TypeId {
        match self.member_item(alias, name, true) {
            Some(item) => self.instantiate_named(item, args),
            None => self.fresh_type(),
        }
    }

    /// What a type name means here: a struct or an enum of the current module, one it imported by
    /// name, or one of the prelude.
    pub fn lookup_type_name(&self, name: &str) -> Option<Item> {
        self.visible(name, true)
    }

    /// What a value name (a function, a global, a C function) means here.
    pub fn lookup_value_name(&self, name: &str) -> Option<Item> {
        self.visible(name, false)
    }
}
