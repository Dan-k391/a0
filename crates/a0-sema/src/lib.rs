//! The A0 semantic toolkit: lexical scopes for name resolution, and static types.
//!
//! [`Scopes`] is a stack of scopes holding symbols. It answers the questions every frontend asks while
//! resolving names: is this name already declared in this scope, where does a use resolve to, has the
//! variable been initialized yet, and does a use cross a function boundary (so the variable must be
//! captured by a closure). Each symbol carries a `u64` of frontend data (an IR variable, a slot index,
//! a [`TypeId`]).
//!
//! [`Types`] is the type checker's toolkit: interned types, nominal structs, type variables with
//! unification, and the standard mismatch diagnostic.

mod types;

pub use types::{TypeId, TypeTag, Types};

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct SymbolId(pub u32);

/// What kind of scope this is. Function scopes mark the boundary for closure capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ScopeKind {
    Block,
    Function,
}

/// Where a name resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub symbol: SymbolId,
    /// How many scopes up from the innermost one the symbol was found (0 = innermost).
    pub scope_distance: u32,
    /// Whether the use is inside a function nested in the one that declares the symbol.
    pub crosses_function: bool,
}

struct SymbolData {
    name: String,
    data: u64,
    defined: bool,
    captured: bool,
    function_level: u32,
}

struct Scope {
    kind: ScopeKind,
    names: HashMap<String, SymbolId>,
    order: Vec<SymbolId>,
}

/// A stack of lexical scopes.
#[derive(Default)]
pub struct Scopes {
    symbols: Vec<SymbolData>,
    scopes: Vec<Scope>,
    function_level: u32,
}

impl Scopes {
    pub fn new() -> Scopes {
        Scopes::default()
    }

    pub fn push(&mut self, kind: ScopeKind) {
        if kind == ScopeKind::Function {
            self.function_level += 1;
        }
        self.scopes.push(Scope {
            kind,
            names: HashMap::new(),
            order: Vec::new(),
        });
    }

    /// Closes the innermost scope and returns its symbols in declaration order.
    pub fn pop(&mut self) -> Vec<SymbolId> {
        match self.scopes.pop() {
            Some(scope) => {
                if scope.kind == ScopeKind::Function {
                    self.function_level -= 1;
                }
                scope.order
            }
            None => Vec::new(),
        }
    }

    /// Number of open scopes.
    pub fn depth(&self) -> u32 {
        self.scopes.len() as u32
    }

    /// Number of open function scopes.
    pub fn function_level(&self) -> u32 {
        self.function_level
    }

    /// The kind of the innermost scope, if any scope is open.
    pub fn current_kind(&self) -> Option<ScopeKind> {
        self.scopes.last().map(|s| s.kind)
    }

    /// Declares `name` in the innermost scope (not yet defined). Returns the existing symbol as the
    /// error if the scope already has one with that name. Declaring with no open scope opens a block
    /// scope first.
    pub fn declare(&mut self, name: &str, data: u64) -> Result<SymbolId, SymbolId> {
        if self.scopes.is_empty() {
            self.push(ScopeKind::Block);
        }
        let function_level = self.function_level;
        let scope = self.scopes.last_mut().unwrap();
        if let Some(&existing) = scope.names.get(name) {
            return Err(existing);
        }
        let id = SymbolId(self.symbols.len() as u32);
        self.symbols.push(SymbolData {
            name: name.to_string(),
            data,
            defined: false,
            captured: false,
            function_level,
        });
        scope.names.insert(name.to_string(), id);
        scope.order.push(id);
        Ok(id)
    }

    /// Marks a symbol as initialized (for "used in its own initializer" checks).
    pub fn define(&mut self, s: SymbolId) {
        self.symbols[s.0 as usize].defined = true;
    }

    /// Declares and defines in one step.
    pub fn declare_defined(&mut self, name: &str, data: u64) -> Result<SymbolId, SymbolId> {
        let s = self.declare(name, data)?;
        self.define(s);
        Ok(s)
    }

    pub fn is_defined(&self, s: SymbolId) -> bool {
        self.symbols[s.0 as usize].defined
    }

    /// Finds the innermost declaration of `name`.
    pub fn lookup(&self, name: &str) -> Option<Resolution> {
        for (distance, scope) in self.scopes.iter().rev().enumerate() {
            if let Some(&symbol) = scope.names.get(name) {
                let declared_in = self.symbols[symbol.0 as usize].function_level;
                return Some(Resolution {
                    symbol,
                    scope_distance: distance as u32,
                    crosses_function: declared_in < self.function_level,
                });
            }
        }
        None
    }

    /// Like [`Scopes::lookup`], and marks the symbol as captured when the use crosses a function
    /// boundary.
    pub fn resolve(&mut self, name: &str) -> Option<Resolution> {
        let r = self.lookup(name)?;
        if r.crosses_function {
            self.symbols[r.symbol.0 as usize].captured = true;
        }
        Some(r)
    }

    /// Every name in scope, innermost first, each once (for suggesting a name close to a misspelled
    /// one).
    pub fn visible_names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for scope in self.scopes.iter().rev() {
            for s in scope.order.iter().rev() {
                let name = self.symbols[s.0 as usize].name.as_str();
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }

    /// The symbol declared with `name` in the innermost scope, if any.
    pub fn lookup_in_current(&self, name: &str) -> Option<SymbolId> {
        self.scopes.last().and_then(|s| s.names.get(name).copied())
    }

    pub fn name(&self, s: SymbolId) -> &str {
        &self.symbols[s.0 as usize].name
    }

    pub fn data(&self, s: SymbolId) -> u64 {
        self.symbols[s.0 as usize].data
    }

    pub fn set_data(&mut self, s: SymbolId, data: u64) {
        self.symbols[s.0 as usize].data = data;
    }

    pub fn is_captured(&self, s: SymbolId) -> bool {
        self.symbols[s.0 as usize].captured
    }

    pub fn mark_captured(&mut self, s: SymbolId) {
        self.symbols[s.0 as usize].captured = true;
    }

    /// How many function scopes were open where `s` was declared.
    pub fn symbol_function_level(&self, s: SymbolId) -> u32 {
        self.symbols[s.0 as usize].function_level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadowing_and_duplicates() {
        let mut s = Scopes::new();
        s.push(ScopeKind::Function);
        let a = s.declare_defined("a", 1).unwrap();
        assert_eq!(s.declare("a", 2), Err(a));
        s.push(ScopeKind::Block);
        let inner = s.declare("a", 3).unwrap();
        assert_ne!(inner, a);
        let r = s.lookup("a").unwrap();
        assert_eq!((r.symbol, r.scope_distance), (inner, 0));
        assert!(!s.is_defined(inner));
        assert_eq!(s.pop(), vec![inner]);
        assert_eq!(s.lookup("a").unwrap().symbol, a);
        assert_eq!(s.data(a), 1);
        assert!(s.lookup("b").is_none());
    }

    #[test]
    fn captures_across_functions() {
        let mut s = Scopes::new();
        s.push(ScopeKind::Function);
        let x = s.declare_defined("x", 0).unwrap();
        s.push(ScopeKind::Block);
        // Same function: not a capture.
        assert!(!s.resolve("x").unwrap().crosses_function);
        s.push(ScopeKind::Function);
        let r = s.resolve("x").unwrap();
        assert!(r.crosses_function);
        assert_eq!(r.scope_distance, 2);
        assert!(s.is_captured(x));
        let y = s.declare_defined("y", 0).unwrap();
        assert!(!s.resolve("y").unwrap().crosses_function);
        assert!(!s.is_captured(y));
        assert_eq!(s.function_level(), 2);
        s.pop();
        assert_eq!(s.function_level(), 1);
    }
}
