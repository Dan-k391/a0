//! The A0 core IR.
//!
//! A structured, typed IR with mutable variables instead of hand-built SSA. See `docs/ir.md` for the
//! full description.
//!
//! ```
//! use a0_ir::{Builder, FloatCC, Type};
//!
//! let mut b = Builder::new("demo");
//! let max = b.declare_function("max", &[Type::F64, Type::F64], Some(Type::F64));
//! b.begin_function(max);
//! let (x, y) = (b.param(0), b.param(1));
//! let result = b.declare_var("result", Type::F64);
//! b.set(result, y);
//! let bigger = b.fcmp(FloatCC::Gt, x, y);
//! b.if_(bigger);
//! b.set(result, x);
//! b.end_if();
//! let r = b.get(result);
//! b.ret(Some(r));
//! b.end_function();
//! let module = b.finish().expect("valid IR");
//! assert!(module.to_string().contains("fcmp gt"));
//! ```

mod builder;
mod debug;
mod module;
mod parse;
mod print;
mod types;
mod verify;

pub use builder::Builder;
pub use debug::{Location, Locations};
pub use module::{InlineSite, Item, Module, region_terminates};
pub use parse::{parse, parse_and_verify};
pub use print::{
    insts_in_text_order, is_bare_ident, print_module, quote_string, referenced_externs,
};
pub use types::*;
pub use verify::{check_entry, verify};
