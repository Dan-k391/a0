//! The HIR data model: interned types, declarations, bodies, locals and a typed expression arena.

use std::collections::{HashMap, HashSet};

use a0_diag::Span;

macro_rules! handle {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(transparent)]
        pub struct $name(pub u32);

        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

handle!(
    /// An interned type.
    TypeId
);
handle!(StructId);
handle!(EnumId);
handle!(GlobalId);
handle!(ExternId);
handle!(FuncId);
handle!(
    /// A function body or a closure body.
    BodyId
);
handle!(LocalId);
handle!(ExprId);
handle!(
    /// One arm of a `match`.
    ArmId
);
handle!(
    /// A module of a program (one file); module 0 is the prelude, which every module sees.
    ModuleId
);

/// A module of a multi-file program (`modules.rs`).
#[derive(Clone)]
pub(crate) struct ModuleDef {
    pub name: String,
    /// Whether its items are named `module.item` (every module but the root file and the prelude).
    pub qualified: bool,
    /// The modules it imports: the name it gives each, the module, and where.
    pub imports: Vec<(String, ModuleId, Option<Span>)>,
    /// Items it imports by name: the name, and the value and the type it may be.
    pub uses: Vec<(String, Option<Item>, Option<Item>)>,
}

impl TypeId {
    pub const UNIT: TypeId = TypeId(0);
    pub const BOOL: TypeId = TypeId(1);
    pub const INT: TypeId = TypeId(2);
    pub const FLOAT: TypeId = TypeId(3);
    pub const STRING: TypeId = TypeId(4);
    pub const ANY: TypeId = TypeId(5);
    /// The type of expressions that never produce a value (`return`, `break`, `panic`, ...).
    pub const NEVER: TypeId = TypeId(6);
    /// The type of an expression that had an error (already reported). It fits everything and
    /// spreads to what is inferred from it, so one mistake is reported once.
    pub const ERROR: TypeId = TypeId(7);
}

/// The shape of a type (see `docs/hir.md`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TypeKind {
    Unit,
    Bool,
    Int,
    Float,
    String,
    Any,
    Never,
    Error,
    Array(TypeId),
    Map(TypeId, TypeId),
    /// A struct and its type arguments (none unless the struct is generic).
    Struct(StructId, Vec<TypeId>),
    Enum(EnumId, Vec<TypeId>),
    Fn(Vec<TypeId>, TypeId),
    /// `T?`: a reference type that may also be null.
    Nullable(TypeId),
    /// The `index`th type parameter of a generic item.
    Param(Item, u32),
    /// An inference variable; `HirModule::vars` says what it is bound to.
    Var(u32),
}

/// The kind of a type, for the public, handle-based API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TypeTag {
    Unit,
    Bool,
    Int,
    Float,
    String,
    Any,
    Never,
    Error,
    Array,
    Map,
    Struct,
    Enum,
    Fn,
    Nullable,
    /// A type parameter of a generic item.
    Param,
    /// A type not inferred yet (only while building).
    Var,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
        }
    }

    pub fn from_symbol(s: &str) -> Option<BinOp> {
        use BinOp::*;
        [Add, Sub, Mul, Div, Rem, Lt, Le, Gt, Ge, Eq, Ne]
            .into_iter()
            .find(|op| op.symbol() == s)
    }

    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum UnOp {
    Neg,
    Not,
}

macro_rules! builtins {
    ($($variant:ident = $name:literal,)*) => {
        /// Built-in operations, called like functions (see `docs/hir.md`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Builtin {
            $($variant,)*
        }

        impl Builtin {
            pub fn name(self) -> &'static str {
                match self {
                    $(Builtin::$variant => $name,)*
                }
            }

            pub fn from_name(s: &str) -> Option<Builtin> {
                match s {
                    $($name => Some(Builtin::$variant),)*
                    _ => None,
                }
            }
        }

        /// The builtins' names.
        pub(crate) const BUILTINS: &[&str] = &[$($name),*];
    };
}

builtins! {
    Len = "len",
    Push = "push",
    Pop = "pop",
    Has = "has",
    Remove = "remove",
    Keys = "keys",
    Substr = "substr",
    ByteAt = "byte_at",
    Str = "str",
    Print = "print",
    Eprint = "eprint",
    Exit = "exit",
    Panic = "panic",
    Clock = "clock",
}

#[derive(Clone, Debug)]
pub(crate) enum ExprKind {
    Unit,
    Bool(bool),
    Int(i64),
    Float(u64),
    Str(String),
    Null,
    Local(LocalId),
    Global(GlobalId),
    Func(FuncId, Vec<TypeId>),
    Let(LocalId, ExprId),
    SetLocal(LocalId, ExprId),
    SetGlobal(GlobalId, ExprId),
    SetField(ExprId, u32, ExprId),
    SetIndex(ExprId, ExprId, ExprId),
    Unary(UnOp, ExprId),
    Binary(BinOp, ExprId, ExprId),
    And(ExprId, ExprId),
    Or(ExprId, ExprId),
    /// A call, with the type arguments of a generic function (empty otherwise).
    Call(FuncId, Vec<TypeId>, Vec<ExprId>),
    CallExtern(ExternId, Vec<ExprId>),
    CallValue(ExprId, Vec<ExprId>),
    Lambda(BodyId),
    New(StructId, Vec<ExprId>),
    Field(ExprId, u32),
    /// A field named before the object's type was known; `finish` turns it into `Field`.
    FieldName(ExprId, String),
    /// Likewise for `SetField`.
    SetFieldName(ExprId, String, ExprId),
    Variant(EnumId, u32, Vec<ExprId>),
    Match(ExprId, Vec<ArmId>),
    Array(Vec<ExprId>),
    NewArray,
    NewMap,
    Index(ExprId, ExprId),
    Builtin(Builtin, Vec<ExprId>),
    Cast(ExprId),
    /// `e!`: the value of a nullable reference, which must not be null.
    Unwrap(ExprId),
    Is(ExprId, TypeId),
    If(ExprId, ExprId, Option<ExprId>),
    /// `if let x = value { then } else { els }`: when `value` (of type `T?`) is not null, `x` (of
    /// type `T`) holds it in `then`.
    IfLet(LocalId, ExprId, ExprId, Option<ExprId>),
    While(ExprId, ExprId),
    Loop(ExprId),
    Break,
    Continue,
    Return(Option<ExprId>),
    Block(Vec<ExprId>, Option<ExprId>),
    /// `throw value`: raises an exception holding `value` (as an `any`).
    Throw(ExprId),
    /// `try { body } catch local { handler }`: runs `handler`, with the exception's value in `local`
    /// (an `any`), if `body` raises one.
    Try(ExprId, LocalId, ExprId),
}

#[derive(Clone)]
pub(crate) struct ExprData {
    pub kind: ExprKind,
    pub ty: TypeId,
    pub span: Option<Span>,
}

/// A call the optimizer inlined: code copied from the callee's body names `callee` in stack traces,
/// followed by the function around the call, at `call`.
#[derive(Clone)]
pub(crate) struct InlineSite {
    pub callee: FuncId,
    pub call: Option<Span>,
    /// The site the function around the call was itself inlined at.
    pub parent: Option<u32>,
}

#[derive(Clone)]
pub(crate) struct ArmData {
    /// The variant matched, or `None` for `_`.
    pub variant: Option<u32>,
    pub binds: Vec<LocalId>,
    pub body: ExprId,
}

#[derive(Clone)]
pub(crate) struct StructDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<(String, TypeId)>,
    /// Where each field was declared.
    pub field_spans: Vec<Option<Span>>,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub(crate) struct VariantDef {
    pub name: String,
    pub fields: Vec<TypeId>,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub(crate) struct EnumDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<VariantDef>,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub(crate) struct GlobalDef {
    pub name: String,
    pub ty: TypeId,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub(crate) struct ExternDef {
    pub name: String,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
    pub abi: Abi,
    pub span: Option<Span>,
}

/// How an `extern fn` takes its arguments and returns its result (D47).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Abi {
    /// A C function: `int`, `float` and `bool` as C's 64-bit integers, doubles and integers, strings
    /// as NUL-terminated `const char *`, arrays of `int`, `float` or `bool` as pointers to their
    /// 64-bit elements, null as `NULL`; a string result is copied.
    C,
    /// A function of A0's runtime (or a language's), which takes and returns A0's own values: a
    /// string is an `A0String *`, an array an `A0Array *`. It may allocate.
    A0,
}

impl Abi {
    /// The name in the text format: `extern "C" fn`, `extern "a0" fn`.
    pub fn name(self) -> &'static str {
        match self {
            Abi::C => "C",
            Abi::A0 => "a0",
        }
    }

    pub fn from_name(s: &str) -> Option<Abi> {
        match s {
            "C" => Some(Abi::C),
            "a0" => Some(Abi::A0),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct FuncDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub body: BodyId,
    pub span: Option<Span>,
    /// Names used by locals anywhere in this function or its closures (kept unique).
    pub local_names: HashSet<String>,
}

#[derive(Clone)]
pub(crate) struct BodyData {
    pub params: Vec<LocalId>,
    /// The parameters' names as the frontend gave them (locals may be renamed to keep them unique).
    pub param_names: Vec<String>,
    pub ret: TypeId,
    pub root: Option<ExprId>,
    /// The enclosing body, for closures.
    pub parent: Option<BodyId>,
    pub func: FuncId,
}

#[derive(Clone)]
pub(crate) struct LocalData {
    /// Unique within the function, for printing.
    pub name: String,
    /// As the frontend named it, for messages.
    pub source_name: String,
    pub ty: TypeId,
    pub body: BodyId,
    pub span: Option<Span>,
}

/// A concept diagnostics name, which a language may call something else
/// ([`crate::HirBuilder::set_term`]). The defaults are the HIR's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Term {
    /// "function"
    Function,
    /// "closure"
    Closure,
    /// "struct"
    Struct,
    /// "enum"
    Enum,
    /// "variant"
    Variant,
    /// "field"
    Field,
    /// "variable"
    Variable,
    /// "`null`", the value of a nullable type that holds nothing.
    Null,
    /// "`!`", which takes the value out of a nullable one.
    Unwrap,
    /// "`if let`", which tests a nullable value and names it when it is not null.
    Narrow,
    /// "`match`"
    Match,
    /// "`break`"
    Break,
    /// "`continue`"
    Continue,
    /// "`?`", which passes a failure (or null) on to the caller.
    Propagate,
}

impl Term {
    pub(crate) fn default_text(self) -> &'static str {
        match self {
            Term::Function => "function",
            Term::Closure => "closure",
            Term::Struct => "struct",
            Term::Enum => "enum",
            Term::Variant => "variant",
            Term::Field => "field",
            Term::Variable => "variable",
            Term::Null => "`null`",
            Term::Unwrap => "`!`",
            Term::Narrow => "`if let`",
            Term::Match => "`match`",
            Term::Break => "`break`",
            Term::Continue => "`continue`",
            Term::Propagate => "`?`",
        }
    }
}

/// Warnings a language can turn on ([`crate::HirBuilder::enable_warning`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Warning {
    /// A variable whose value is never read. Names that start with `_` are exempt, and so are
    /// parameters.
    UnusedVariable,
    /// Code after a `return`, `break`, `continue` or anything else that never finishes.
    UnreachableCode,
}

/// Something a module-level name refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Struct(StructId),
    Enum(EnumId),
    Global(GlobalId),
    Extern(ExternId),
    Func(FuncId),
}

/// A HIR module. Build it with [`crate::HirBuilder`] or [`crate::parse`].
#[derive(Clone)]
pub struct HirModule {
    pub(crate) name: String,
    pub(crate) types: Vec<TypeKind>,
    pub(crate) type_map: HashMap<TypeKind, TypeId>,
    pub(crate) structs: Vec<StructDef>,
    pub(crate) enums: Vec<EnumDef>,
    pub(crate) globals: Vec<GlobalDef>,
    pub(crate) externs: Vec<ExternDef>,
    pub(crate) funcs: Vec<FuncDef>,
    pub(crate) bodies: Vec<BodyData>,
    pub(crate) locals: Vec<LocalData>,
    pub(crate) exprs: Vec<ExprData>,
    pub(crate) arms: Vec<ArmData>,
    /// What each inference variable is bound to.
    pub(crate) vars: Vec<Option<TypeId>>,
    /// How diagnostics write some types, in the language's own syntax (`[{}]` for arrays).
    pub(crate) type_syntax: Vec<(TypeTag, String)>,
    /// What diagnostics call some concepts, in the language's own words.
    pub(crate) terms: Vec<(Term, String)>,
    /// The warnings found when the module was finished.
    pub(crate) warnings: a0_diag::Diagnostics,
    /// The modules of a multi-file program (the prelude first), and the module each item belongs to,
    /// with whether it is public.
    pub(crate) modules: Vec<ModuleDef>,
    pub(crate) item_home: HashMap<Item, (ModuleId, bool)>,
    /// The calls the optimizer inlined, and the site each copied expression came from.
    pub(crate) inline_sites: Vec<InlineSite>,
    pub(crate) inline_of: HashMap<ExprId, u32>,
    /// Casts out of `any` that the optimizer proved cannot fail (an `is` test of the same value
    /// guards them); lowering skips their check.
    pub(crate) unchecked_casts: HashSet<ExprId>,
    /// Declarations in order, for printing.
    pub(crate) items: Vec<Item>,
    pub(crate) type_names: HashMap<String, Item>,
    pub(crate) value_names: HashMap<String, Item>,
}

impl HirModule {
    pub(crate) fn new(name: &str) -> HirModule {
        let mut m = HirModule {
            name: name.to_string(),
            types: Vec::new(),
            type_map: HashMap::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            globals: Vec::new(),
            externs: Vec::new(),
            funcs: Vec::new(),
            bodies: Vec::new(),
            locals: Vec::new(),
            exprs: Vec::new(),
            arms: Vec::new(),
            vars: Vec::new(),
            type_syntax: Vec::new(),
            terms: Vec::new(),
            warnings: a0_diag::Diagnostics::new(),
            inline_sites: Vec::new(),
            inline_of: HashMap::new(),
            modules: vec![ModuleDef {
                name: String::new(),
                qualified: false,
                imports: Vec::new(),
                uses: Vec::new(),
            }],
            item_home: HashMap::new(),
            unchecked_casts: HashSet::new(),
            items: Vec::new(),
            type_names: HashMap::new(),
            value_names: HashMap::new(),
        };
        for k in [
            TypeKind::Unit,
            TypeKind::Bool,
            TypeKind::Int,
            TypeKind::Float,
            TypeKind::String,
            TypeKind::Any,
            TypeKind::Never,
            TypeKind::Error,
        ] {
            m.intern(k);
        }
        m
    }

    pub(crate) fn intern(&mut self, k: TypeKind) -> TypeId {
        if let Some(&t) = self.type_map.get(&k) {
            return t;
        }
        let t = TypeId(self.types.len() as u32);
        self.types.push(k.clone());
        self.type_map.insert(k, t);
        t
    }

    /// The shape of `t`, looking through bound inference variables.
    pub(crate) fn kind(&self, t: TypeId) -> &TypeKind {
        &self.types[self.shallow(t).index()]
    }

    /// `t`, or what it is bound to if it is a bound inference variable.
    pub(crate) fn shallow(&self, mut t: TypeId) -> TypeId {
        while let TypeKind::Var(v) = self.types[t.index()] {
            match self.vars[v as usize] {
                Some(b) => t = b,
                None => break,
            }
        }
        t
    }

    pub(crate) fn expr(&self, e: ExprId) -> &ExprData {
        &self.exprs[e.index()]
    }

    // ---------------------------------------------------------------- public read API

    /// The warnings the builder found (those the language turned on); a driver shows them
    /// before running the program.
    pub fn warnings(&self) -> &a0_diag::Diagnostics {
        &self.warnings
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The module an item belongs to, and whether it is public.
    pub fn home(&self, item: Item) -> (ModuleId, bool) {
        self.item_home
            .get(&item)
            .copied()
            .unwrap_or((ModuleId(0), true))
    }

    pub fn module_count(&self) -> u32 {
        self.modules.len() as u32
    }

    pub fn module_name(&self, m: ModuleId) -> &str {
        &self.modules[m.index()].name
    }

    /// A type by its name in the HIR (`json.Value` for an item of module `json`).
    pub fn lookup_type(&self, name: &str) -> Option<Item> {
        self.type_names.get(name).copied()
    }

    pub fn lookup_value(&self, name: &str) -> Option<Item> {
        self.value_names.get(name).copied()
    }

    pub fn func_count(&self) -> u32 {
        self.funcs.len() as u32
    }

    pub fn func_name(&self, f: FuncId) -> &str {
        &self.funcs[f.index()].name
    }

    pub fn func_body(&self, f: FuncId) -> BodyId {
        self.funcs[f.index()].body
    }

    pub fn body_params(&self, b: BodyId) -> &[LocalId] {
        &self.bodies[b.index()].params
    }

    pub fn body_ret(&self, b: BodyId) -> TypeId {
        self.bodies[b.index()].ret
    }

    pub fn body_root(&self, b: BodyId) -> Option<ExprId> {
        self.bodies[b.index()].root
    }

    pub fn local_count(&self) -> u32 {
        self.locals.len() as u32
    }

    // How many of each handle exist (a handle is valid below its count), for bindings that check
    // handles they did not get from the builder.

    pub fn type_count(&self) -> u32 {
        self.types.len() as u32
    }

    pub fn struct_count(&self) -> u32 {
        self.structs.len() as u32
    }

    pub fn enum_count(&self) -> u32 {
        self.enums.len() as u32
    }

    pub fn global_count(&self) -> u32 {
        self.globals.len() as u32
    }

    pub fn extern_count(&self) -> u32 {
        self.externs.len() as u32
    }

    pub fn body_count(&self) -> u32 {
        self.bodies.len() as u32
    }

    pub fn expr_count(&self) -> u32 {
        self.exprs.len() as u32
    }

    pub fn arm_count(&self) -> u32 {
        self.arms.len() as u32
    }

    /// How tools show an item: a function's signature, a global's type, a struct's or enum's name,
    /// with types in the language's syntax (`fn std.str.find(s: string, part: string) -> int`).
    pub fn item_signature(&self, item: Item) -> String {
        let ret = |t: TypeId| match t {
            TypeId::UNIT => String::new(),
            t => format!(" -> {}", self.display_type(t)),
        };
        match item {
            Item::Func(f) => {
                let body = self.func_body(f);
                let params: Vec<String> = self
                    .body_params(body)
                    .iter()
                    .map(|&l| {
                        let t = self.display_type(self.local_type(l));
                        format!("{}: {t}", self.local_source_name(l))
                    })
                    .collect();
                let tparams = &self.funcs[f.index()].type_params;
                let tparams = match tparams.is_empty() {
                    true => String::new(),
                    false => format!("<{}>", tparams.join(", ")),
                };
                let (name, r) = (self.func_name(f), ret(self.body_ret(body)));
                format!("fn {name}{tparams}({}){r}", params.join(", "))
            }
            Item::Extern(x) => {
                let def = &self.externs[x.index()];
                let params: Vec<String> =
                    def.params.iter().map(|&t| self.display_type(t)).collect();
                format!(
                    "extern fn {}({}){}",
                    def.name,
                    params.join(", "),
                    ret(def.ret)
                )
            }
            Item::Global(g) => format!(
                "{}: {}",
                self.global_name(g),
                self.display_type(self.global_type(g))
            ),
            Item::Struct(s) => format!("struct {}", self.struct_name(s)),
            Item::Enum(e) => format!("enum {}", self.enum_name(e)),
        }
    }

    pub fn local_name(&self, l: LocalId) -> &str {
        &self.locals[l.index()].name
    }

    /// A local's name as the frontend gave it (`local_name` may have a suffix that keeps it
    /// unique).
    pub fn local_source_name(&self, l: LocalId) -> &str {
        &self.locals[l.index()].source_name
    }

    pub fn local_type(&self, l: LocalId) -> TypeId {
        self.locals[l.index()].ty
    }

    /// Where a local was declared, if the builder had a span set.
    pub fn local_span(&self, l: LocalId) -> Option<Span> {
        self.locals[l.index()].span
    }

    pub fn expr_type(&self, e: ExprId) -> TypeId {
        self.exprs[e.index()].ty
    }

    /// The declared item an expression refers to: the function it calls or takes as a value, the
    /// global it reads or writes, the C function it calls, the struct or enum it builds.
    pub fn expr_item(&self, e: ExprId) -> Option<Item> {
        Some(match &self.exprs[e.index()].kind {
            ExprKind::Call(f, ..) | ExprKind::Func(f, _) => Item::Func(*f),
            ExprKind::Global(g) | ExprKind::SetGlobal(g, _) => Item::Global(*g),
            ExprKind::CallExtern(x, _) => Item::Extern(*x),
            ExprKind::New(s, _) => Item::Struct(*s),
            ExprKind::Variant(en, ..) => Item::Enum(*en),
            _ => return None,
        })
    }

    pub fn expr_span(&self, e: ExprId) -> Option<Span> {
        self.exprs[e.index()].span
    }

    pub fn struct_name(&self, s: StructId) -> &str {
        &self.structs[s.index()].name
    }

    pub fn struct_field_count(&self, s: StructId) -> u32 {
        self.structs[s.index()].fields.len() as u32
    }

    pub fn struct_field(&self, s: StructId, i: u32) -> (&str, TypeId) {
        let (n, t) = &self.structs[s.index()].fields[i as usize];
        (n, *t)
    }

    /// The index of a field by name.
    pub fn struct_field_index(&self, s: StructId, name: &str) -> Option<u32> {
        self.structs[s.index()]
            .fields
            .iter()
            .position(|(n, _)| n == name)
            .map(|i| i as u32)
    }

    pub fn enum_name(&self, e: EnumId) -> &str {
        &self.enums[e.index()].name
    }

    pub fn variant_count(&self, e: EnumId) -> u32 {
        self.enums[e.index()].variants.len() as u32
    }

    pub fn variant_name(&self, e: EnumId, v: u32) -> &str {
        &self.enums[e.index()].variants[v as usize].name
    }

    pub fn variant_fields(&self, e: EnumId, v: u32) -> &[TypeId] {
        &self.enums[e.index()].variants[v as usize].fields
    }

    pub fn variant_index(&self, e: EnumId, name: &str) -> Option<u32> {
        self.enums[e.index()]
            .variants
            .iter()
            .position(|v| v.name == name)
            .map(|i| i as u32)
    }

    pub fn global_name(&self, g: GlobalId) -> &str {
        &self.globals[g.index()].name
    }

    pub fn global_type(&self, g: GlobalId) -> TypeId {
        self.globals[g.index()].ty
    }

    pub fn extern_name(&self, x: ExternId) -> &str {
        &self.externs[x.index()].name
    }

    pub fn extern_abi(&self, x: ExternId) -> Abi {
        self.externs[x.index()].abi
    }

    /// Where an item was declared, if it came from text or the builder had a span set.
    pub fn item_span(&self, item: Item) -> Option<Span> {
        match item {
            Item::Struct(s) => self.structs[s.index()].span,
            Item::Enum(e) => self.enums[e.index()].span,
            Item::Global(g) => self.globals[g.index()].span,
            Item::Extern(x) => self.externs[x.index()].span,
            Item::Func(f) => self.funcs[f.index()].span,
        }
    }

    pub fn type_tag(&self, t: TypeId) -> TypeTag {
        match self.kind(t) {
            TypeKind::Unit => TypeTag::Unit,
            TypeKind::Bool => TypeTag::Bool,
            TypeKind::Int => TypeTag::Int,
            TypeKind::Float => TypeTag::Float,
            TypeKind::String => TypeTag::String,
            TypeKind::Any => TypeTag::Any,
            TypeKind::Never => TypeTag::Never,
            TypeKind::Error => TypeTag::Error,
            TypeKind::Array(_) => TypeTag::Array,
            TypeKind::Map(..) => TypeTag::Map,
            TypeKind::Struct(..) => TypeTag::Struct,
            TypeKind::Enum(..) => TypeTag::Enum,
            TypeKind::Fn(..) => TypeTag::Fn,
            TypeKind::Nullable(_) => TypeTag::Nullable,
            TypeKind::Param(..) => TypeTag::Param,
            TypeKind::Var(_) => TypeTag::Var,
        }
    }

    /// The struct a type is an instance of.
    pub fn struct_of(&self, t: TypeId) -> Option<StructId> {
        match self.kind(t) {
            TypeKind::Struct(s, _) => Some(*s),
            _ => None,
        }
    }

    /// The enum a type is an instance of.
    pub fn enum_of(&self, t: TypeId) -> Option<EnumId> {
        match self.kind(t) {
            TypeKind::Enum(e, _) => Some(*e),
            _ => None,
        }
    }

    /// The type arguments of a generic struct or enum type (empty for other types).
    pub fn type_args(&self, t: TypeId) -> &[TypeId] {
        match self.kind(t) {
            TypeKind::Struct(_, args) | TypeKind::Enum(_, args) => args,
            _ => &[],
        }
    }

    /// The type a nullable type makes nullable (`T` for `T?`), or `t` itself.
    pub fn non_null(&self, t: TypeId) -> TypeId {
        match self.kind(t) {
            TypeKind::Nullable(inner) => *inner,
            _ => self.shallow(t),
        }
    }

    /// The names of an item's type parameters (empty unless it is generic).
    pub fn type_params(&self, item: Item) -> &[String] {
        match item {
            Item::Struct(s) => &self.structs[s.index()].type_params,
            Item::Enum(e) => &self.enums[e.index()].type_params,
            Item::Func(f) => &self.funcs[f.index()].type_params,
            Item::Global(_) | Item::Extern(_) => &[],
        }
    }

    /// Whether values of this type are garbage-collected references.
    pub fn is_reference(&self, t: TypeId) -> bool {
        matches!(
            self.kind(t),
            TypeKind::String
                | TypeKind::Array(_)
                | TypeKind::Map(..)
                | TypeKind::Struct(..)
                | TypeKind::Enum(..)
                | TypeKind::Fn(..)
                | TypeKind::Nullable(_)
        )
    }

    fn applied(&self, name: &str, args: &[TypeId], custom: bool) -> String {
        // Diagnostics show names as the language wrote them; the text format quotes them.
        let name = if custom {
            name.into()
        } else {
            crate::builder::ident(name)
        };
        if args.is_empty() {
            return name.into_owned();
        }
        let args: Vec<String> = args.iter().map(|&a| self.format_type(a, custom)).collect();
        format!("{name}<{}>", args.join(", "))
    }

    /// The type's text form, such as `map<string, array<int>>`.
    pub fn type_name(&self, t: TypeId) -> String {
        self.format_type(t, false)
    }

    /// The type as diagnostics show it: like [`HirModule::type_name`], but in the language's own
    /// syntax where the frontend gave one (`HirBuilder::set_type_syntax`).
    pub fn display_type(&self, t: TypeId) -> String {
        self.format_type(t, true)
    }

    /// A type as messages quote it: `` `int` ``, or the language's word for null when it is the
    /// type of a null nothing else determined.
    pub(crate) fn quote_type(&self, t: TypeId) -> String {
        match self.kind(t) {
            TypeKind::Nullable(inner) if matches!(self.kind(*inner), TypeKind::Var(_)) => {
                self.term(Term::Null)
            }
            _ => format!("`{}`", self.display_type(t)),
        }
    }

    /// What messages call `term`.
    pub(crate) fn term(&self, term: Term) -> String {
        match self.terms.iter().find(|(t, _)| *t == term) {
            Some((_, text)) => text.clone(),
            None => term.default_text().to_string(),
        }
    }

    fn format_type(&self, t: TypeId, custom: bool) -> String {
        let name = |t: TypeId| self.format_type(t, custom);
        if custom {
            let tag = self.type_tag(t);
            if let Some((_, template)) = self.type_syntax.iter().find(|(g, _)| *g == tag) {
                let parts: Vec<String> = match self.kind(t) {
                    TypeKind::Array(e) | TypeKind::Nullable(e) => vec![name(*e)],
                    TypeKind::Map(k, v) => vec![name(*k), name(*v)],
                    _ => Vec::new(),
                };
                let mut out = template.clone();
                for p in parts {
                    out = out.replacen("{}", &p, 1);
                }
                return out;
            }
        }
        match self.kind(t) {
            // `unit` is the HIR's own word; messages say `()`, as rustc does.
            TypeKind::Unit if custom => "()".to_string(),
            TypeKind::Unit => "unit".to_string(),
            TypeKind::Bool => "bool".to_string(),
            TypeKind::Int => "int".to_string(),
            TypeKind::Float => "float".to_string(),
            TypeKind::String => "string".to_string(),
            TypeKind::Any => "any".to_string(),
            TypeKind::Never => "never".to_string(),
            TypeKind::Error => "{error}".to_string(),
            TypeKind::Array(e) => format!("array<{}>", name(*e)),
            TypeKind::Map(k, v) => format!("map<{}, {}>", name(*k), name(*v)),
            TypeKind::Struct(s, args) => self.applied(&self.structs[s.index()].name, args, custom),
            TypeKind::Enum(e, args) => self.applied(&self.enums[e.index()].name, args, custom),
            TypeKind::Nullable(inner) => match self.kind(*inner) {
                TypeKind::Fn(..) => format!("({})?", name(*inner)),
                _ => format!("{}?", name(*inner)),
            },
            TypeKind::Param(item, i) => {
                let n = &self.type_params(*item)[*i as usize];
                if custom {
                    n.clone()
                } else {
                    crate::builder::ident(n).into_owned()
                }
            }
            TypeKind::Var(_) => "_".to_string(),
            TypeKind::Fn(params, ret) => {
                let ps: Vec<String> = params.iter().map(|&p| name(p)).collect();
                if *ret == TypeId::UNIT {
                    format!("fn({})", ps.join(", "))
                } else {
                    format!("fn({}) -> {}", ps.join(", "), name(*ret))
                }
            }
        }
    }
}
