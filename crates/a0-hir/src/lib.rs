//! The A0 high-level IR (HIR).
//!
//! A typed expression tree with strings, arrays, maps, structs, enums, closures and `any`, managed by
//! a precise garbage collector. It is lowered to the core IR (`a0-ir`) plus calls into the default
//! runtime, so it runs in the interpreter and natively without changes. See `docs/hir.md`.
//!
//! ```
//! use a0_hir::{BinOp, Builtin, HirBuilder, TypeId};
//!
//! let mut b = HirBuilder::new("demo");
//! let main = b.declare_function("main", &[], &[], TypeId::INT);
//! let body = b.function_body(main);
//! let hello = b.string("hello, ");
//! let world = b.string("world\n");
//! let text = b.binary(BinOp::Add, hello, world);
//! let print = b.builtin(Builtin::Print, &[text]);
//! let zero = b.int(0);
//! let root = b.block(&[print], Some(zero));
//! b.set_root(body, root);
//! let module = b.finish().unwrap();
//! assert!(module.to_string().contains(r#"print("hello, " + "world\n");"#));
//! let ir = a0_hir::lower(&module).unwrap();
//! assert!(ir.lookup_func("main").is_some());
//! ```

mod builder;
mod check;
pub mod dsl;
mod frontend;
mod lower;
mod methods;
mod model;
mod modules;
mod mono;
mod opt;
mod print;
mod stdlib;
mod text;
mod types;

pub use builder::{HirBuilder, KEYWORDS};
pub use check::{check, check_entry};
pub use lower::{LowerOptions, lower, lower_with};
pub use model::{
    Abi, ArmId, BinOp, BodyId, Builtin, EnumId, ExprId, ExternId, FuncId, GlobalId, HirModule,
    InterfaceId, Item, LocalId, ModuleId, StructId, Term, TypeId, TypeTag, UnOp, Warning,
};
pub use print::print_module;
pub use stdlib::{is_std_module, std_module_text, std_modules};
pub use text::parse;
