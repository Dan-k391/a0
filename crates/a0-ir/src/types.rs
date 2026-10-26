//! Value types, condition codes, opcodes and entity handles.

/// The type of an IR value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Type {
    I1,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Ptr,
}

impl Type {
    pub const ALL: [Type; 8] = [
        Type::I1,
        Type::I8,
        Type::I16,
        Type::I32,
        Type::I64,
        Type::F32,
        Type::F64,
        Type::Ptr,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Type::I1 => "i1",
            Type::I8 => "i8",
            Type::I16 => "i16",
            Type::I32 => "i32",
            Type::I64 => "i64",
            Type::F32 => "f32",
            Type::F64 => "f64",
            Type::Ptr => "ptr",
        }
    }

    pub fn from_name(s: &str) -> Option<Type> {
        Type::ALL.into_iter().find(|t| t.name() == s)
    }

    pub fn is_int(self) -> bool {
        matches!(
            self,
            Type::I1 | Type::I8 | Type::I16 | Type::I32 | Type::I64
        )
    }

    pub fn is_float(self) -> bool {
        matches!(self, Type::F32 | Type::F64)
    }

    /// Bit width of integer and float types; `None` for `ptr`, whose width depends on the target.
    pub fn bits(self) -> Option<u32> {
        match self {
            Type::I1 => Some(1),
            Type::I8 => Some(8),
            Type::I16 => Some(16),
            Type::I32 => Some(32),
            Type::I64 => Some(64),
            Type::F32 => Some(32),
            Type::F64 => Some(64),
            Type::Ptr => None,
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Integer comparison condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum IntCC {
    Eq,
    Ne,
    Slt,
    Sle,
    Sgt,
    Sge,
    Ult,
    Ule,
    Ugt,
    Uge,
}

impl IntCC {
    pub const ALL: [IntCC; 10] = [
        IntCC::Eq,
        IntCC::Ne,
        IntCC::Slt,
        IntCC::Sle,
        IntCC::Sgt,
        IntCC::Sge,
        IntCC::Ult,
        IntCC::Ule,
        IntCC::Ugt,
        IntCC::Uge,
    ];

    pub fn name(self) -> &'static str {
        match self {
            IntCC::Eq => "eq",
            IntCC::Ne => "ne",
            IntCC::Slt => "slt",
            IntCC::Sle => "sle",
            IntCC::Sgt => "sgt",
            IntCC::Sge => "sge",
            IntCC::Ult => "ult",
            IntCC::Ule => "ule",
            IntCC::Ugt => "ugt",
            IntCC::Uge => "uge",
        }
    }

    pub fn from_name(s: &str) -> Option<IntCC> {
        IntCC::ALL.into_iter().find(|c| c.name() == s)
    }

    fn from_u64(x: u64) -> IntCC {
        IntCC::ALL[x as usize]
    }
}

/// Float comparison condition. All are ordered (false if either operand is NaN) except `Ne`,
/// which is true when the operands are unordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FloatCC {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl FloatCC {
    pub const ALL: [FloatCC; 6] = [
        FloatCC::Eq,
        FloatCC::Ne,
        FloatCC::Lt,
        FloatCC::Le,
        FloatCC::Gt,
        FloatCC::Ge,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FloatCC::Eq => "eq",
            FloatCC::Ne => "ne",
            FloatCC::Lt => "lt",
            FloatCC::Le => "le",
            FloatCC::Gt => "gt",
            FloatCC::Ge => "ge",
        }
    }

    pub fn from_name(s: &str) -> Option<FloatCC> {
        FloatCC::ALL.into_iter().find(|c| c.name() == s)
    }

    fn from_u64(x: u64) -> FloatCC {
        FloatCC::ALL[x as usize]
    }
}

pub(crate) fn int_cc_from_imm(x: u64) -> IntCC {
    IntCC::from_u64(x)
}

pub(crate) fn float_cc_from_imm(x: u64) -> FloatCC {
    FloatCC::from_u64(x)
}

macro_rules! opcodes {
    ($($variant:ident = $name:literal,)*) => {
        /// Instruction opcode. See `docs/ir.md` for the meaning of each.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Opcode {
            $($variant,)*
        }

        impl Opcode {
            pub const ALL: &'static [Opcode] = &[$(Opcode::$variant,)*];

            /// The spelling used in the text format.
            pub fn name(self) -> &'static str {
                match self {
                    $(Opcode::$variant => $name,)*
                }
            }

            pub fn from_name(s: &str) -> Option<Opcode> {
                match s {
                    $($name => Some(Opcode::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

opcodes! {
    Iconst = "iconst",
    Fconst = "fconst",
    Null = "null",
    Addr = "addr",
    Iadd = "iadd",
    Isub = "isub",
    Imul = "imul",
    Sdiv = "sdiv",
    Udiv = "udiv",
    Srem = "srem",
    Urem = "urem",
    And = "and",
    Or = "or",
    Xor = "xor",
    Shl = "shl",
    Lshr = "lshr",
    Ashr = "ashr",
    Ineg = "ineg",
    Not = "not",
    Fadd = "fadd",
    Fsub = "fsub",
    Fmul = "fmul",
    Fdiv = "fdiv",
    Fneg = "fneg",
    Icmp = "icmp",
    Fcmp = "fcmp",
    Zext = "zext",
    Sext = "sext",
    Trunc = "trunc",
    Sitofp = "sitofp",
    Uitofp = "uitofp",
    Fptosi = "fptosi",
    Fptoui = "fptoui",
    Fext = "fext",
    Ftrunc = "ftrunc",
    Bitcast = "bitcast",
    Ptrtoint = "ptrtoint",
    Inttoptr = "inttoptr",
    Select = "select",
    Load = "load",
    Store = "store",
    Ptradd = "ptradd",
    Call = "call",
    CallIndirect = "call_indirect",
    Get = "get",
    Set = "set",
    If = "if",
    Loop = "loop",
    Break = "break",
    Continue = "continue",
    Return = "return",
    Unreachable = "unreachable",
}

impl Opcode {
    /// Two integer operands, integer result of the same type.
    pub fn is_int_binary(self) -> bool {
        use Opcode::*;
        matches!(
            self,
            Iadd | Isub | Imul | Sdiv | Udiv | Srem | Urem | And | Or | Xor | Shl | Lshr | Ashr
        )
    }

    /// Two float operands, float result of the same type.
    pub fn is_float_binary(self) -> bool {
        matches!(
            self,
            Opcode::Fadd | Opcode::Fsub | Opcode::Fmul | Opcode::Fdiv
        )
    }

    pub fn is_unary(self) -> bool {
        matches!(self, Opcode::Ineg | Opcode::Not | Opcode::Fneg)
    }

    pub fn is_cast(self) -> bool {
        use Opcode::*;
        matches!(
            self,
            Zext | Sext
                | Trunc
                | Sitofp
                | Uitofp
                | Fptosi
                | Fptoui
                | Fext
                | Ftrunc
                | Bitcast
                | Ptrtoint
                | Inttoptr
        )
    }

    /// Control never continues to the next instruction in the same region.
    pub fn is_terminator(self) -> bool {
        matches!(
            self,
            Opcode::Break | Opcode::Continue | Opcode::Return | Opcode::Unreachable
        )
    }

    /// Integer division and remainder trap on a zero divisor.
    pub fn can_trap(self) -> bool {
        matches!(
            self,
            Opcode::Sdiv | Opcode::Udiv | Opcode::Srem | Opcode::Urem | Opcode::Unreachable
        )
    }
}

impl std::fmt::Display for Opcode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

macro_rules! entity {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(transparent)]
        pub struct $name(pub u32);

        impl $name {
            /// Sentinel meaning "none".
            pub const NONE: $name = $name(u32::MAX);

            pub fn index(self) -> usize {
                self.0 as usize
            }

            pub fn is_none(self) -> bool {
                self.0 == u32::MAX
            }

            pub fn is_some(self) -> bool {
                self.0 != u32::MAX
            }

            /// `None` for the sentinel, `Some(self)` otherwise.
            pub fn option(self) -> Option<$name> {
                if self.is_none() { None } else { Some(self) }
            }
        }
    };
}

entity!(
    /// A function, defined or extern.
    FuncId
);
entity!(
    /// A `data` or `global` item.
    GlobalId
);
entity!(
    /// An interned function signature.
    SigId
);
entity!(
    /// An SSA value: a parameter or an instruction result. Unique in the module.
    Value
);
entity!(
    /// A mutable function-local variable. Unique in the module.
    Var
);
entity!(
    /// An instruction. Unique in the module.
    Inst
);
entity!(
    /// An ordered list of instructions: a function body or a nested `if`/`loop` region.
    Region
);
entity!(
    /// A place where a source-level function was inlined, for stack traces.
    InlineId
);
entity!(
    /// A source-level type, for debuggers (see [`DebugKind`]).
    DebugType
);

/// What a source-level value is, so that a debugger can show it (`docs/ir.md`, "Debug
/// information"). Values are 64-bit words as variables, struct fields and array elements hold
/// them: references are pointers to the runtime's objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DebugKind {
    Int,
    Float,
    Bool,
    /// A pointer to an `A0String`.
    String,
    /// A pointer to an `A0Array` of the first element type.
    Array,
    /// A pointer to an `A0Map` from the first element type to the second.
    Map,
    /// A pointer to an object whose words are the fields.
    Struct,
    /// A pointer to an object whose first word is the variant's index, then its fields.
    Enum,
    /// The first element type, or 0 for null.
    Nullable,
    /// A dynamic value (`a0rt.h`, "Dynamic values").
    Any,
    /// A pointer to a box whose first word holds a value of the first element type.
    Boxed,
    /// A pointer to an object of a type the object itself says (an interface value): its
    /// descriptor's name is a struct's, or an enum's variant's as `Enum::Variant`.
    Object,
    /// Shown by name only (closures, foreign pointers).
    Opaque,
    /// An unsigned integer.
    UInt,
    /// A 32-bit float (in the low 32 bits of a word).
    Float32,
}

impl DebugKind {
    pub const ALL: &'static [DebugKind] = &[
        DebugKind::Int,
        DebugKind::Float,
        DebugKind::Bool,
        DebugKind::String,
        DebugKind::Array,
        DebugKind::Map,
        DebugKind::Struct,
        DebugKind::Enum,
        DebugKind::Nullable,
        DebugKind::Any,
        DebugKind::Boxed,
        DebugKind::Object,
        DebugKind::Opaque,
        DebugKind::UInt,
        DebugKind::Float32,
    ];

    pub fn name(self) -> &'static str {
        match self {
            DebugKind::Int => "int",
            DebugKind::Float => "float",
            DebugKind::Bool => "bool",
            DebugKind::String => "string",
            DebugKind::Array => "array",
            DebugKind::Map => "map",
            DebugKind::Struct => "struct",
            DebugKind::Enum => "enum",
            DebugKind::Nullable => "nullable",
            DebugKind::Any => "any",
            DebugKind::Boxed => "boxed",
            DebugKind::Object => "object",
            DebugKind::Opaque => "opaque",
            DebugKind::UInt => "uint",
            DebugKind::Float32 => "float32",
        }
    }

    pub fn from_name(s: &str) -> Option<DebugKind> {
        DebugKind::ALL.iter().copied().find(|k| k.name() == s)
    }
}

/// What kind of item a [`Symbol`] names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SymbolKind {
    Func,
    Global,
}

/// A reference to a module item by kind and index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub index: u32,
}

impl Symbol {
    pub fn func(f: FuncId) -> Symbol {
        Symbol {
            kind: SymbolKind::Func,
            index: f.0,
        }
    }

    pub fn global(g: GlobalId) -> Symbol {
        Symbol {
            kind: SymbolKind::Global,
            index: g.0,
        }
    }

    pub fn as_func(self) -> Option<FuncId> {
        (self.kind == SymbolKind::Func).then_some(FuncId(self.index))
    }

    pub fn as_global(self) -> Option<GlobalId> {
        (self.kind == SymbolKind::Global).then_some(GlobalId(self.index))
    }

    pub(crate) fn to_imm(self) -> u64 {
        ((self.kind as u64) << 32) | self.index as u64
    }

    pub(crate) fn from_imm(x: u64) -> Symbol {
        let kind = if x >> 32 == 0 {
            SymbolKind::Func
        } else {
            SymbolKind::Global
        };
        Symbol {
            kind,
            index: x as u32,
        }
    }
}

/// Canonical NaN bit patterns, printed as plain `nan`.
pub const F64_CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;
pub const F32_CANONICAL_NAN: u32 = 0x7fc0_0000;

/// Normalizes an integer constant to the canonical stored form for `ty`: sign-extended from the
/// type's width, except `i1`, which is stored as 0 or 1.
pub fn canonical_int(ty: Type, value: i64) -> i64 {
    match ty {
        Type::I1 => value & 1,
        Type::I8 => value as i8 as i64,
        Type::I16 => value as i16 as i64,
        Type::I32 => value as i32 as i64,
        _ => value,
    }
}
