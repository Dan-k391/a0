//! WebAssembly from A0's core IR (D78).
//!
//! Cranelift has no WebAssembly backend, and the core IR is structured (`if`, `loop`, `break n`),
//! so it is translated directly: IR values and variables become locals, a loop becomes a block
//! around a loop, and integers narrower than 32 bits stay in canonical form (sign-extended, `i1` 0
//! or 1) as the interpreter keeps them. Checks the IR defines (division by zero, overflow,
//! `unreachable`) call the runtime's `a0_rt_trap`, as native code does, so messages and exit codes
//! are the same.
//!
//! The module is not a whole program: it links against an instance of A0's runtime built for
//! WebAssembly (the playground's engine, or the runtime module `a0 build` writes beside it). It
//! imports that instance's `memory` and function table, every C function the IR declares, and two
//! numbers: `__memory_base`, where the instance put the bytes the module asked for
//! (`__data_size`: its data and globals, a stack for stack slots, and a stack of call locations),
//! and `__table_base`, where the module's functions go in the table (`__table_size` of them), so
//! that function pointers are table indices. Both sizes are also in the custom section
//! `a0.layout` (two little-endian u32s), for a host that needs them before instantiating.
//! `a0_wasm_main` runs the program's `@main`.
//!
//! Pointers are 32 bits in locals but take 64-bit words in memory, as everywhere else in A0
//! (`A0_WORD_PTR` in `a0rt.h`), so objects look the same to generated code and to the runtime.
//! Stack traces come from a stack of call locations the code keeps (each function's slot holds
//! the location of the call it is making), which the runtime reads through `a0_rt_set_frames`.

use std::collections::HashMap;

use a0_diag::{Diagnostic, Diagnostics};
use a0_ir::{FloatCC, FuncId, GlobalId, Inst, IntCC, Module, Opcode, Region, SymbolKind, Type};
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, CustomSection, DataSection, ElementSection, Elements,
    EntityType, ExportKind, ExportSection, Function, FunctionSection, GlobalSection, GlobalType,
    ImportSection, IndirectNameMap, Instruction as I, MemArg, MemoryType, NameMap, NameSection,
    RefType, TableType, TypeSection, ValType,
};

/// Trap codes (`a0rt.h`).
const TRAP_UNREACHABLE: i32 = 1;
const TRAP_DIVIDE_BY_ZERO: i32 = 2;
const TRAP_INTEGER_OVERFLOW: i32 = 3;
const TRAP_STACK_OVERFLOW: i32 = 4;

/// Calls nested deeper than this stop with a stack overflow, before the engine's own stack does.
pub const MAX_DEPTH: u32 = 10_000;
/// Bytes of the stack that stack slots live on.
const SLOT_STACK: u32 = 1 << 20;

/// Wasm globals of every module, by index.
const G_MEMORY_BASE: u32 = 0;
const G_TABLE_BASE: u32 = 1;
const G_LOC_SP: u32 = 2;
const G_LOC_LIMIT: u32 = 3;
const G_SP: u32 = 4;

fn error(message: &str) -> Diagnostics {
    Diagnostic::error("wasm.codegen", message).into()
}

fn val(t: Type) -> ValType {
    match t {
        Type::I64 => ValType::I64,
        Type::F32 => ValType::F32,
        Type::F64 => ValType::F64,
        // i1, i8, i16, i32 and pointers.
        _ => ValType::I32,
    }
}

/// Whether `t` lives in an i32 local narrower than 32 bits.
fn narrow(t: Type) -> bool {
    matches!(t, Type::I1 | Type::I8 | Type::I16)
}

fn memarg() -> MemArg {
    MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }
}

/// Where the parts of the module's memory are, relative to `__memory_base`.
struct Layout {
    globals: Vec<u32>,
    table: u32,
    locations: u32,
    slots: u32,
    size: u32,
    bytes: Vec<u8>,
}

fn layout(m: &Module, table: &[u8]) -> Layout {
    let mut bytes: Vec<u8> = Vec::new();
    let align = |bytes: &mut Vec<u8>, a: usize| {
        while !bytes.len().is_multiple_of(a) {
            bytes.push(0);
        }
    };
    let mut globals = Vec::new();
    for g in 0..m.global_count() {
        let g = GlobalId(g);
        align(&mut bytes, 8);
        globals.push(bytes.len() as u32);
        match m.global_type(g) {
            None => bytes.extend_from_slice(m.global_bytes(g)),
            Some(t) => {
                // Every scalar global takes a word, as in the interpreter.
                let bits = m.global_init_bits(g);
                let width = match t {
                    Type::I1 | Type::I8 => 1,
                    Type::I16 => 2,
                    Type::I32 | Type::F32 => 4,
                    _ => 8,
                };
                bytes.extend_from_slice(&bits.to_le_bytes()[..width]);
                bytes.extend(std::iter::repeat_n(0, 8 - width));
            }
        }
    }
    align(&mut bytes, 8);
    let table_at = bytes.len() as u32;
    bytes.extend_from_slice(table);
    align(&mut bytes, 16);
    let locations = bytes.len() as u32;
    let slots = locations + MAX_DEPTH * 4;
    let size = slots + SLOT_STACK;
    Layout {
        globals,
        table: table_at,
        locations,
        slots,
        size,
        bytes,
    }
}

/// Function indices: the imported C functions the IR uses, then the defined ones, then the
/// module's own two.
struct Indices {
    /// Per IR function: its wasm function index, if it has one.
    func: Vec<Option<u32>>,
    trap: u32,
    set_frames: u32,
    set_debug: u32,
    frames: u32,
    main: u32,
    count: u32,
}

/// Interns function types.
#[derive(Default)]
struct Types {
    list: Vec<(Vec<ValType>, Vec<ValType>)>,
    index: HashMap<(Vec<ValType>, Vec<ValType>), u32>,
}

impl Types {
    fn get(&mut self, params: Vec<ValType>, results: Vec<ValType>) -> u32 {
        let key = (params, results);
        if let Some(&i) = self.index.get(&key) {
            return i;
        }
        let i = self.list.len() as u32;
        self.list.push(key.clone());
        self.index.insert(key, i);
        i
    }

    fn of_ir(&mut self, params: &[Type], extra: &[ValType], ret: Option<Type>) -> u32 {
        let mut ps: Vec<ValType> = params.iter().map(|&t| val(t)).collect();
        ps.extend_from_slice(extra);
        self.get(ps, ret.map(val).into_iter().collect())
    }
}

/// Compiles a verified module with an `@main` into a WebAssembly module (see the crate's
/// documentation for what it imports and exports).
pub fn compile(m: &Module) -> Result<Vec<u8>, Diagnostics> {
    let diags = a0_ir::verify(m);
    if diags.has_errors() {
        return Err(diags);
    }
    let diags = a0_ir::check_entry(m);
    if diags.has_errors() {
        return Err(diags);
    }
    for f in 0..m.func_count() {
        let f = FuncId(f);
        if m.func_has_struct_params(f) {
            return Err(error(&format!(
                "@{}: C structs passed by value are not supported in WebAssembly yet",
                m.func_name(f)
            )));
        }
    }

    let locations = a0_ir::Locations::build(m);
    let entries: Vec<a0_runtime::debug::Entry> = locations
        .entries
        .iter()
        .map(|l| (l.func.as_str(), l.file.as_str(), l.line, l.col, l.parent))
        .collect();
    let lay = layout(m, &a0_runtime::debug::encode(&entries));

    // Imports: the referenced C functions, and the runtime's functions the code itself calls.
    let referenced = a0_ir::referenced_externs(m);
    let mut types = Types::default();
    let mut imports = ImportSection::new();
    let mut by_name: HashMap<String, u32> = HashMap::new();
    let mut func: Vec<Option<u32>> = vec![None; m.func_count() as usize];
    let mut next = 0u32;
    let reports = |name: &str| -> Vec<ValType> {
        if a0_runtime::reports_location(name) {
            vec![ValType::I64, ValType::I32]
        } else {
            Vec::new()
        }
    };
    let mut names = NameMap::new();
    for f in 0..m.func_count() {
        let fid = FuncId(f);
        if !m.func_is_extern(fid) || !referenced[f as usize] {
            continue;
        }
        let name = m.func_name(fid).to_string();
        if let Some(&i) = by_name.get(&name) {
            func[f as usize] = Some(i);
            continue;
        }
        let ty = types.of_ir(m.func_params(fid), &reports(&name), m.func_ret(fid));
        imports.import("env", &name, EntityType::Function(ty));
        names.append(next, &name);
        by_name.insert(name, next);
        func[f as usize] = Some(next);
        next += 1;
    }
    let mut runtime = |name: &str, params: Vec<ValType>, imports: &mut ImportSection| -> u32 {
        if let Some(&i) = by_name.get(name) {
            return i;
        }
        let ty = types.get(params, Vec::new());
        imports.import("env", name, EntityType::Function(ty));
        names.append(next, name);
        by_name.insert(name.to_string(), next);
        next += 1;
        next - 1
    };
    let trap = runtime(
        "a0_rt_trap",
        vec![ValType::I32, ValType::I64, ValType::I32],
        &mut imports,
    );
    let set_frames = runtime("a0_rt_set_frames", vec![ValType::I32], &mut imports);
    let set_debug = runtime(
        "a0_rt_set_debug",
        vec![ValType::I32, ValType::I32],
        &mut imports,
    );
    let import_count = next;
    imports.import(
        "env",
        "memory",
        EntityType::Memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        }),
    );
    imports.import(
        "env",
        "__indirect_function_table",
        EntityType::Table(TableType {
            element_type: RefType::FUNCREF,
            minimum: 0,
            maximum: None,
            table64: false,
            shared: false,
        }),
    );
    let base = GlobalType {
        val_type: ValType::I32,
        mutable: false,
        shared: false,
    };
    imports.import("env", "__memory_base", EntityType::Global(base));
    imports.import("env", "__table_base", EntityType::Global(base));

    // Defined functions.
    let mut defined = Vec::new();
    for f in 0..m.func_count() {
        let fid = FuncId(f);
        if m.func_is_extern(fid) {
            continue;
        }
        func[f as usize] = Some(next);
        names.append(next, m.func_name(fid));
        defined.push(fid);
        next += 1;
    }
    let frames = next;
    names.append(frames, "a0.wasm.frames");
    let main = next + 1;
    names.append(main, "a0_wasm_main");
    let count = next + 2;
    let idx = Indices {
        func,
        trap,
        set_frames,
        set_debug,
        frames,
        main,
        count,
    };

    let mut functions = FunctionSection::new();
    let mut code = CodeSection::new();
    let mut local_names = IndirectNameMap::new();
    for &f in &defined {
        functions.function(types.of_ir(m.func_params(f), &[], m.func_ret(f)));
        let (body, locals) = FuncGen::new(m, f, &mut types, &idx, &lay, &locations).run();
        code.function(&body);
        local_names.append(idx.func[f.index()].unwrap(), &locals);
    }
    functions.function(types.get(vec![ValType::I32, ValType::I32], vec![ValType::I32]));
    code.function(&frames_function(&lay));
    functions.function(types.get(Vec::new(), vec![ValType::I32]));
    let ir_main = m.lookup_func("main").expect("checked by check_entry");
    code.function(&main_function(
        &idx,
        &lay,
        idx.func[ir_main.index()].unwrap(),
    ));

    let mut type_section = TypeSection::new();
    for (params, results) in &types.list {
        type_section.ty().function(params.clone(), results.clone());
    }

    let mut globals = GlobalSection::new();
    let mutable = GlobalType {
        val_type: ValType::I32,
        mutable: true,
        shared: false,
    };
    // Imported: 0 and 1. Then the location stack's top and limit, and the slot stack's top.
    globals.global(mutable, &ConstExpr::i32_const(0));
    globals.global(mutable, &ConstExpr::i32_const(0));
    globals.global(mutable, &ConstExpr::i32_const(0));
    globals.global(base, &ConstExpr::i32_const(lay.size as i32));
    globals.global(base, &ConstExpr::i32_const(idx.count as i32));

    let mut exports = ExportSection::new();
    exports.export("a0_wasm_main", ExportKind::Func, idx.main);
    exports.export("__data_size", ExportKind::Global, 5);
    exports.export("__table_size", ExportKind::Global, 6);

    // Every function in the table, at `__table_base` + its index.
    let mut elements = ElementSection::new();
    let all: Vec<u32> = (0..idx.count).collect();
    elements.active(
        Some(0),
        &ConstExpr::global_get(G_TABLE_BASE),
        Elements::Functions(std::borrow::Cow::Borrowed(&all)),
    );

    let mut data = DataSection::new();
    data.active(
        0,
        &ConstExpr::global_get(G_MEMORY_BASE),
        lay.bytes.iter().copied(),
    );

    // What a host needs before it can instantiate the module: `__data_size` and `__table_size`.
    let mut sizes = lay.size.to_le_bytes().to_vec();
    sizes.extend_from_slice(&idx.count.to_le_bytes());
    let layout_section = CustomSection {
        name: "a0.layout".into(),
        data: sizes.into(),
    };

    let mut name_section = NameSection::new();
    name_section.functions(&names);
    name_section.locals(&local_names);

    let mut module = wasm_encoder::Module::new();
    module
        .section(&type_section)
        .section(&imports)
        .section(&functions)
        .section(&globals)
        .section(&exports)
        .section(&elements)
        .section(&code)
        .section(&data)
        .section(&layout_section)
        .section(&name_section);
    let _ = import_count;
    Ok(module.finish())
}

/// `size_t frames(uint32_t *out, size_t max)` (`a0_rt_set_frames`): the location of the call each
/// active function is making, from the caller of the innermost one out (the runtime adds the
/// innermost one's own location).
fn frames_function(lay: &Layout) -> Function {
    // Locals: out 0, max 1, n 2, i 3, start 4.
    let mut f = Function::new([(3, ValType::I32)]);
    let ins = &mut f;
    ins.instruction(&I::GlobalGet(G_MEMORY_BASE))
        .instruction(&I::I32Const(lay.locations as i32))
        .instruction(&I::I32Add)
        .instruction(&I::LocalSet(4))
        // i = (loc_sp - start) / 4 - 2
        .instruction(&I::GlobalGet(G_LOC_SP))
        .instruction(&I::LocalGet(4))
        .instruction(&I::I32Sub)
        .instruction(&I::I32Const(2))
        .instruction(&I::I32ShrS)
        .instruction(&I::I32Const(2))
        .instruction(&I::I32Sub)
        .instruction(&I::LocalSet(3))
        .instruction(&I::Block(BlockType::Empty))
        .instruction(&I::Loop(BlockType::Empty))
        // while i >= 0 && n < max
        .instruction(&I::LocalGet(3))
        .instruction(&I::I32Const(0))
        .instruction(&I::I32LtS)
        .instruction(&I::BrIf(1))
        .instruction(&I::LocalGet(2))
        .instruction(&I::LocalGet(1))
        .instruction(&I::I32GeU)
        .instruction(&I::BrIf(1))
        // out[n] = stack[i]
        .instruction(&I::LocalGet(0))
        .instruction(&I::LocalGet(2))
        .instruction(&I::I32Const(2))
        .instruction(&I::I32Shl)
        .instruction(&I::I32Add)
        .instruction(&I::LocalGet(4))
        .instruction(&I::LocalGet(3))
        .instruction(&I::I32Const(2))
        .instruction(&I::I32Shl)
        .instruction(&I::I32Add)
        .instruction(&I::I32Load(memarg()))
        .instruction(&I::I32Store(memarg()))
        .instruction(&I::LocalGet(2))
        .instruction(&I::I32Const(1))
        .instruction(&I::I32Add)
        .instruction(&I::LocalSet(2))
        .instruction(&I::LocalGet(3))
        .instruction(&I::I32Const(1))
        .instruction(&I::I32Sub)
        .instruction(&I::LocalSet(3))
        .instruction(&I::Br(0))
        .instruction(&I::End)
        .instruction(&I::End)
        .instruction(&I::LocalGet(2))
        .instruction(&I::End);
    f
}

/// `a0_wasm_main`: sets up the stacks, gives the runtime the traces' tables, and runs `@main`.
fn main_function(idx: &Indices, lay: &Layout, ir_main: u32) -> Function {
    let mut f = Function::new([]);
    let base = |f: &mut Function, offset: u32| {
        f.instruction(&I::GlobalGet(G_MEMORY_BASE))
            .instruction(&I::I32Const(offset as i32))
            .instruction(&I::I32Add);
    };
    base(&mut f, lay.locations);
    f.instruction(&I::GlobalSet(G_LOC_SP));
    base(&mut f, lay.locations + MAX_DEPTH * 4);
    f.instruction(&I::GlobalSet(G_LOC_LIMIT));
    base(&mut f, lay.slots + SLOT_STACK);
    f.instruction(&I::GlobalSet(G_SP));
    f.instruction(&I::GlobalGet(G_TABLE_BASE))
        .instruction(&I::I32Const(idx.frames as i32))
        .instruction(&I::I32Add)
        .instruction(&I::Call(idx.set_frames));
    base(&mut f, lay.table);
    f.instruction(&I::I32Const(0))
        .instruction(&I::Call(idx.set_debug))
        .instruction(&I::Call(ir_main))
        .instruction(&I::End);
    f
}

/// What a `br` can target.
#[derive(Clone, Copy, PartialEq)]
enum Label {
    Body,
    LoopExit,
    LoopTop,
    If,
}

struct FuncGen<'a> {
    m: &'a Module,
    f: FuncId,
    /// The module's function types, which indirect calls add to.
    types: &'a mut Types,
    idx: &'a Indices,
    lay: &'a Layout,
    locations: &'a a0_ir::Locations,
    out: Function,
    /// Local index of each IR value and variable.
    values: HashMap<a0_ir::Value, u32>,
    vars: HashMap<a0_ir::Var, u32>,
    /// Each stack slot's offset in the frame, and the frame's size.
    slots: HashMap<Inst, u32>,
    frame: u32,
    labels: Vec<Label>,
    /// Locals of the function's own: the frame pointer, the result, scratch.
    fp: u32,
    ret: Option<u32>,
    scratch_i32: u32,
    scratch_i64: u32,
}

impl<'a> FuncGen<'a> {
    fn new(
        m: &'a Module,
        f: FuncId,
        types: &'a mut Types,
        idx: &'a Indices,
        lay: &'a Layout,
        locations: &'a a0_ir::Locations,
    ) -> FuncGen<'a> {
        FuncGen {
            m,
            f,
            types,
            idx,
            lay,
            locations,
            out: Function::new([]),
            values: HashMap::new(),
            vars: HashMap::new(),
            slots: HashMap::new(),
            frame: 0,
            labels: Vec::new(),
            fp: 0,
            ret: None,
            scratch_i32: 0,
            scratch_i64: 0,
        }
    }

    /// The function's body, and the names of its locals.
    fn run(mut self) -> (Function, NameMap) {
        let m = self.m;
        let body = m.func_body(self.f).unwrap();
        let mut locals: Vec<ValType> = Vec::new();
        let mut names = NameMap::new();
        let params = m.func_param_values(self.f).to_vec();
        for (i, &p) in params.iter().enumerate() {
            self.values.insert(p, i as u32);
            names.append(i as u32, &format!("v{}", p.0));
        }
        let first = params.len() as u32;
        let add = |t: ValType, name: String, locals: &mut Vec<ValType>, names: &mut NameMap| {
            let i = first + locals.len() as u32;
            locals.push(t);
            names.append(i, &name);
            i
        };
        for inst in a0_ir::insts_in_text_order(m, body) {
            if let Some(v) = m.inst_result(inst) {
                let i = add(
                    val(m.value_type(v)),
                    format!("v{}", v.0),
                    &mut locals,
                    &mut names,
                );
                self.values.insert(v, i);
            }
            if m.inst_opcode(inst) == Opcode::StackSlot {
                let (size, align) = m.inst_stack_slot(inst);
                let align = align.max(1);
                let offset = self.frame.div_ceil(align) * align;
                self.slots.insert(inst, offset);
                self.frame = offset + size;
            }
        }
        for &v in m.func_vars(self.f) {
            let i = add(
                val(m.var_type(v)),
                format!("var{}", v.0),
                &mut locals,
                &mut names,
            );
            self.vars.insert(v, i);
        }
        self.frame = self.frame.div_ceil(16) * 16;
        self.fp = add(ValType::I32, "fp".into(), &mut locals, &mut names);
        self.ret = m
            .func_ret(self.f)
            .map(|t| add(val(t), "result".into(), &mut locals, &mut names));
        self.scratch_i32 = add(ValType::I32, "t32".into(), &mut locals, &mut names);
        self.scratch_i64 = add(ValType::I64, "t64".into(), &mut locals, &mut names);
        self.out = Function::new_with_locals_types(locals);

        self.prologue();
        self.out.instruction(&I::Block(BlockType::Empty));
        self.labels.push(Label::Body);
        self.region(body);
        self.out.instruction(&I::End);
        self.labels.pop();
        self.epilogue();
        if let Some(r) = self.ret {
            self.out.instruction(&I::LocalGet(r));
        }
        self.out.instruction(&I::End);
        (self.out, names)
    }

    fn ins(&mut self, i: I) -> &mut Self {
        self.out.instruction(&i);
        self
    }

    /// Calls the runtime's trap with `code` at location `loc`, then never comes back.
    fn trap(&mut self, code: i32, loc: u32) {
        let t = self.idx.trap;
        self.ins(I::I32Const(code))
            .ins(I::I64Const(loc as i64))
            .ins(I::I32Const(0))
            .ins(I::Call(t))
            .ins(I::Unreachable);
    }

    fn prologue(&mut self) {
        // The location stack: a slot for this call, unless calls nest too deep.
        self.ins(I::GlobalGet(G_LOC_SP))
            .ins(I::GlobalGet(G_LOC_LIMIT))
            .ins(I::I32GeU)
            .ins(I::If(BlockType::Empty));
        self.trap(TRAP_STACK_OVERFLOW, 0);
        self.ins(I::End)
            .ins(I::GlobalGet(G_LOC_SP))
            .ins(I::I32Const(4))
            .ins(I::I32Add)
            .ins(I::GlobalSet(G_LOC_SP));
        if self.frame > 0 {
            let (frame, fp) = (self.frame as i32, self.fp);
            self.ins(I::GlobalGet(G_SP))
                .ins(I::I32Const(frame))
                .ins(I::I32Sub)
                .ins(I::LocalTee(fp))
                .ins(I::GlobalSet(G_SP));
        }
    }

    fn epilogue(&mut self) {
        if self.frame > 0 {
            let frame = self.frame as i32;
            self.ins(I::LocalGet(self.fp))
                .ins(I::I32Const(frame))
                .ins(I::I32Add)
                .ins(I::GlobalSet(G_SP));
        }
        self.ins(I::GlobalGet(G_LOC_SP))
            .ins(I::I32Const(4))
            .ins(I::I32Sub)
            .ins(I::GlobalSet(G_LOC_SP));
    }

    fn region(&mut self, r: Region) {
        for &i in self.m.region_insts(r) {
            self.inst(i);
        }
    }

    fn get(&mut self, v: a0_ir::Value) {
        let l = self.values[&v];
        self.ins(I::LocalGet(l));
    }

    fn set_result(&mut self, i: Inst) {
        let v = self.m.inst_result(i).unwrap();
        let l = self.values[&v];
        self.ins(I::LocalSet(l));
    }

    /// The value on the stack (computed in an i32) back in canonical form for `t`.
    fn canon(&mut self, t: Type) {
        match t {
            Type::I1 => {
                self.ins(I::I32Const(1)).ins(I::I32And);
            }
            Type::I8 => {
                self.ins(I::I32Extend8S);
            }
            Type::I16 => {
                self.ins(I::I32Extend16S);
            }
            _ => {}
        }
    }

    /// The canonical value on the stack, as its bits unsigned in an i32.
    fn unsigned(&mut self, t: Type) {
        match t {
            Type::I8 => {
                self.ins(I::I32Const(0xff)).ins(I::I32And);
            }
            Type::I16 => {
                self.ins(I::I32Const(0xffff)).ins(I::I32And);
            }
            _ => {}
        }
    }

    fn bits(t: Type) -> i32 {
        match t {
            Type::I1 => 1,
            Type::I8 => 8,
            Type::I16 => 16,
            Type::I64 => 64,
            _ => 32,
        }
    }

    /// Stores the location of a call in this function's slot of the location stack.
    fn mark_call(&mut self, i: Inst) {
        let loc = self.locations.of(i) as i32;
        self.ins(I::GlobalGet(G_LOC_SP))
            .ins(I::I32Const(4))
            .ins(I::I32Sub)
            .ins(I::I32Const(loc))
            .ins(I::I32Store(memarg()));
    }

    /// The depth of the `br` that leaves the `n`th loop out (`exit`) or starts it again.
    fn loop_label(&self, n: u32, exit: bool) -> u32 {
        let mut seen = 0;
        for (pos, &l) in self.labels.iter().enumerate().rev() {
            if l == Label::LoopTop {
                if seen == n {
                    let target = if exit { pos - 1 } else { pos };
                    return (self.labels.len() - 1 - target) as u32;
                }
                seen += 1;
            }
        }
        unreachable!("verified IR breaks out of enclosing loops only")
    }

    fn inst(&mut self, i: Inst) {
        let m = self.m;
        let op = m.inst_opcode(i);
        let args: Vec<a0_ir::Value> = m.inst_args(i).to_vec();
        let ty = |n: usize| m.value_type(args[n]);
        let loc = self.locations.of(i);
        match op {
            Opcode::Iconst => {
                let t = m.value_type(m.inst_result(i).unwrap());
                let v = m.inst_iconst(i);
                if t == Type::I64 {
                    self.ins(I::I64Const(v));
                } else {
                    self.ins(I::I32Const(v as i32));
                }
                self.set_result(i);
            }
            Opcode::Fconst => {
                let t = m.value_type(m.inst_result(i).unwrap());
                let bits = m.inst_fconst_bits(i);
                if t == Type::F32 {
                    self.ins(I::F32Const(f32::from_bits(bits as u32).into()));
                } else {
                    self.ins(I::F64Const(f64::from_bits(bits).into()));
                }
                self.set_result(i);
            }
            Opcode::Null => {
                self.ins(I::I32Const(0));
                self.set_result(i);
            }
            Opcode::StackSlot => {
                let off = self.slots[&i] as i32;
                self.ins(I::LocalGet(self.fp))
                    .ins(I::I32Const(off))
                    .ins(I::I32Add);
                self.set_result(i);
            }
            Opcode::Addr => {
                let sym = m.inst_symbol(i);
                match sym.kind {
                    SymbolKind::Func => {
                        let fidx = self.idx.func[sym.index as usize]
                            .expect("an addressed function is in the module");
                        self.ins(I::GlobalGet(G_TABLE_BASE))
                            .ins(I::I32Const(fidx as i32))
                            .ins(I::I32Add);
                    }
                    SymbolKind::Global => {
                        let off = self.lay.globals[sym.index as usize] as i32;
                        self.ins(I::GlobalGet(G_MEMORY_BASE))
                            .ins(I::I32Const(off))
                            .ins(I::I32Add);
                    }
                }
                self.set_result(i);
            }
            op if op.is_int_binary() => self.int_binary(i, op, ty(0), &args, loc),
            op if op.is_float_binary() => {
                self.get(args[0]);
                self.get(args[1]);
                let f32 = ty(0) == Type::F32;
                self.ins(match (op, f32) {
                    (Opcode::Fadd, true) => I::F32Add,
                    (Opcode::Fsub, true) => I::F32Sub,
                    (Opcode::Fmul, true) => I::F32Mul,
                    (_, true) => I::F32Div,
                    (Opcode::Fadd, false) => I::F64Add,
                    (Opcode::Fsub, false) => I::F64Sub,
                    (Opcode::Fmul, false) => I::F64Mul,
                    (_, false) => I::F64Div,
                });
                self.set_result(i);
            }
            op if op.is_unary() => {
                let t = ty(0);
                match op {
                    Opcode::Fneg => {
                        self.get(args[0]);
                        self.ins(if t == Type::F32 { I::F32Neg } else { I::F64Neg });
                    }
                    Opcode::Ineg if t == Type::I64 => {
                        self.ins(I::I64Const(0));
                        self.get(args[0]);
                        self.ins(I::I64Sub);
                    }
                    Opcode::Ineg => {
                        self.ins(I::I32Const(0));
                        self.get(args[0]);
                        self.ins(I::I32Sub);
                        self.canon(t);
                    }
                    // `not`: of a boolean, of the bits of an integer.
                    _ if t == Type::I64 => {
                        self.get(args[0]);
                        self.ins(I::I64Const(-1)).ins(I::I64Xor);
                    }
                    _ if t == Type::I1 => {
                        self.get(args[0]);
                        self.ins(I::I32Const(1)).ins(I::I32Xor);
                    }
                    _ => {
                        self.get(args[0]);
                        self.ins(I::I32Const(-1)).ins(I::I32Xor);
                    }
                }
                self.set_result(i);
            }
            Opcode::Icmp => {
                let t = ty(0);
                let cc = m.inst_int_cc(i);
                let unsigned = matches!(cc, IntCC::Ult | IntCC::Ule | IntCC::Ugt | IntCC::Uge);
                self.get(args[0]);
                if unsigned {
                    self.unsigned(t);
                }
                self.get(args[1]);
                if unsigned {
                    self.unsigned(t);
                }
                let wide = t == Type::I64;
                self.ins(match (cc, wide) {
                    (IntCC::Eq, false) => I::I32Eq,
                    (IntCC::Ne, false) => I::I32Ne,
                    (IntCC::Slt, false) => I::I32LtS,
                    (IntCC::Sle, false) => I::I32LeS,
                    (IntCC::Sgt, false) => I::I32GtS,
                    (IntCC::Sge, false) => I::I32GeS,
                    (IntCC::Ult, false) => I::I32LtU,
                    (IntCC::Ule, false) => I::I32LeU,
                    (IntCC::Ugt, false) => I::I32GtU,
                    (IntCC::Uge, false) => I::I32GeU,
                    (IntCC::Eq, true) => I::I64Eq,
                    (IntCC::Ne, true) => I::I64Ne,
                    (IntCC::Slt, true) => I::I64LtS,
                    (IntCC::Sle, true) => I::I64LeS,
                    (IntCC::Sgt, true) => I::I64GtS,
                    (IntCC::Sge, true) => I::I64GeS,
                    (IntCC::Ult, true) => I::I64LtU,
                    (IntCC::Ule, true) => I::I64LeU,
                    (IntCC::Ugt, true) => I::I64GtU,
                    (IntCC::Uge, true) => I::I64GeU,
                });
                self.set_result(i);
            }
            Opcode::Fcmp => {
                let f32 = ty(0) == Type::F32;
                self.get(args[0]);
                self.get(args[1]);
                self.ins(match (m.inst_float_cc(i), f32) {
                    (FloatCC::Eq, true) => I::F32Eq,
                    (FloatCC::Ne, true) => I::F32Ne,
                    (FloatCC::Lt, true) => I::F32Lt,
                    (FloatCC::Le, true) => I::F32Le,
                    (FloatCC::Gt, true) => I::F32Gt,
                    (FloatCC::Ge, true) => I::F32Ge,
                    (FloatCC::Eq, false) => I::F64Eq,
                    (FloatCC::Ne, false) => I::F64Ne,
                    (FloatCC::Lt, false) => I::F64Lt,
                    (FloatCC::Le, false) => I::F64Le,
                    (FloatCC::Gt, false) => I::F64Gt,
                    (FloatCC::Ge, false) => I::F64Ge,
                });
                self.set_result(i);
            }
            op if op.is_cast() => {
                let to = m.value_type(m.inst_result(i).unwrap());
                self.get(args[0]);
                self.cast(op, ty(0), to);
                self.set_result(i);
            }
            Opcode::Select => {
                self.get(args[1]);
                self.get(args[2]);
                self.get(args[0]);
                self.ins(I::Select);
                self.set_result(i);
            }
            Opcode::Load => {
                let t = m.value_type(m.inst_result(i).unwrap());
                self.get(args[0]);
                match t {
                    Type::I1 => {
                        self.ins(I::I32Load8U(memarg()))
                            .ins(I::I32Const(1))
                            .ins(I::I32And);
                    }
                    Type::I8 => {
                        self.ins(I::I32Load8S(memarg()));
                    }
                    Type::I16 => {
                        self.ins(I::I32Load16S(memarg()));
                    }
                    Type::I32 => {
                        self.ins(I::I32Load(memarg()));
                    }
                    Type::I64 => {
                        self.ins(I::I64Load(memarg()));
                    }
                    Type::F32 => {
                        self.ins(I::F32Load(memarg()));
                    }
                    Type::F64 => {
                        self.ins(I::F64Load(memarg()));
                    }
                    // A pointer takes a word in memory.
                    Type::Ptr => {
                        self.ins(I::I64Load(memarg())).ins(I::I32WrapI64);
                    }
                }
                self.set_result(i);
            }
            Opcode::Store => {
                let t = ty(0);
                self.get(args[1]);
                self.get(args[0]);
                match t {
                    Type::I1 | Type::I8 => {
                        self.ins(I::I32Store8(memarg()));
                    }
                    Type::I16 => {
                        self.ins(I::I32Store16(memarg()));
                    }
                    Type::I32 => {
                        self.ins(I::I32Store(memarg()));
                    }
                    Type::I64 => {
                        self.ins(I::I64Store(memarg()));
                    }
                    Type::F32 => {
                        self.ins(I::F32Store(memarg()));
                    }
                    Type::F64 => {
                        self.ins(I::F64Store(memarg()));
                    }
                    Type::Ptr => {
                        self.ins(I::I64ExtendI32U).ins(I::I64Store(memarg()));
                    }
                }
            }
            Opcode::Ptradd => {
                self.get(args[0]);
                self.get(args[1]);
                self.ins(I::I32WrapI64).ins(I::I32Add);
                self.set_result(i);
            }
            Opcode::Call => {
                let callee = m.inst_callee(i);
                let fidx =
                    self.idx.func[callee.index()].expect("a called function is in the module");
                self.mark_call(i);
                for &a in &args {
                    self.get(a);
                }
                if m.func_is_extern(callee) && a0_runtime::reports_location(m.func_name(callee)) {
                    self.ins(I::I64Const(loc as i64)).ins(I::I32Const(0));
                }
                self.ins(I::Call(fidx));
                if m.inst_result(i).is_some() {
                    self.set_result(i);
                }
            }
            Opcode::CallIndirect => {
                let sig = m.inst_sig(i);
                self.mark_call(i);
                for &a in &args[1..] {
                    self.get(a);
                }
                self.get(args[0]);
                let ty = self.type_of_sig(m.sig_params(sig), m.sig_ret(sig));
                self.ins(I::CallIndirect {
                    type_index: ty,
                    table_index: 0,
                });
                if m.inst_result(i).is_some() {
                    self.set_result(i);
                }
            }
            Opcode::Get => {
                let l = self.vars[&m.inst_var(i)];
                self.ins(I::LocalGet(l));
                self.set_result(i);
            }
            Opcode::Set => {
                let l = self.vars[&m.inst_var(i)];
                self.get(args[0]);
                self.ins(I::LocalSet(l));
            }
            Opcode::If => {
                self.get(args[0]);
                self.ins(I::If(BlockType::Empty));
                self.labels.push(Label::If);
                self.region(m.inst_region(i, 0).unwrap());
                if let Some(r) = m.inst_region(i, 1) {
                    self.ins(I::Else);
                    self.region(r);
                }
                self.labels.pop();
                self.ins(I::End);
            }
            Opcode::Loop => {
                self.ins(I::Block(BlockType::Empty))
                    .ins(I::Loop(BlockType::Empty));
                self.labels.push(Label::LoopExit);
                self.labels.push(Label::LoopTop);
                self.region(m.inst_region(i, 0).unwrap());
                self.ins(I::Br(0));
                self.labels.pop();
                self.labels.pop();
                self.ins(I::End).ins(I::End);
            }
            Opcode::Break => {
                let d = self.loop_label(m.inst_loop_depth(i), true);
                self.ins(I::Br(d));
            }
            Opcode::Continue => {
                let d = self.loop_label(m.inst_loop_depth(i), false);
                self.ins(I::Br(d));
            }
            Opcode::Return => {
                if let (Some(&v), Some(r)) = (args.first(), self.ret) {
                    self.get(v);
                    self.ins(I::LocalSet(r));
                }
                let d = (self.labels.len() - 1) as u32;
                self.ins(I::Br(d));
            }
            Opcode::Unreachable => self.trap(TRAP_UNREACHABLE, loc),
            _ => unreachable!("verified IR has no other opcodes"),
        }
    }

    fn type_of_sig(&mut self, params: &[Type], ret: Option<Type>) -> u32 {
        self.types.of_ir(params, &[], ret)
    }

    fn int_binary(&mut self, i: Inst, op: Opcode, t: Type, args: &[a0_ir::Value], loc: u32) {
        let wide = t == Type::I64;
        let (a, b) = (args[0], args[1]);
        match op {
            Opcode::Sdiv | Opcode::Srem | Opcode::Udiv | Opcode::Urem => {
                // A divisor of zero traps; so does the most negative number divided by -1.
                self.get(b);
                if wide {
                    self.ins(I::I64Eqz);
                } else {
                    self.ins(I::I32Eqz);
                }
                self.ins(I::If(BlockType::Empty));
                self.trap(TRAP_DIVIDE_BY_ZERO, loc);
                self.ins(I::End);
                if op == Opcode::Sdiv {
                    let min = if wide {
                        i64::MIN
                    } else {
                        -(1i64 << (Self::bits(t) - 1))
                    };
                    self.get(a);
                    if wide {
                        self.ins(I::I64Const(min)).ins(I::I64Eq);
                    } else {
                        self.ins(I::I32Const(min as i32)).ins(I::I32Eq);
                    }
                    self.get(b);
                    if wide {
                        self.ins(I::I64Const(-1)).ins(I::I64Eq);
                    } else {
                        self.ins(I::I32Const(-1)).ins(I::I32Eq);
                    }
                    self.ins(I::I32And).ins(I::If(BlockType::Empty));
                    self.trap(TRAP_INTEGER_OVERFLOW, loc);
                    self.ins(I::End);
                }
                let unsigned = matches!(op, Opcode::Udiv | Opcode::Urem);
                self.get(a);
                if unsigned {
                    self.unsigned(t);
                }
                self.get(b);
                if unsigned {
                    self.unsigned(t);
                }
                self.ins(match (op, wide) {
                    (Opcode::Sdiv, false) => I::I32DivS,
                    (Opcode::Srem, false) => I::I32RemS,
                    (Opcode::Udiv, false) => I::I32DivU,
                    (_, false) => I::I32RemU,
                    (Opcode::Sdiv, true) => I::I64DivS,
                    (Opcode::Srem, true) => I::I64RemS,
                    (Opcode::Udiv, true) => I::I64DivU,
                    (_, true) => I::I64RemU,
                });
                self.canon(t);
            }
            Opcode::Shl | Opcode::Lshr | Opcode::Ashr => {
                self.get(a);
                if op == Opcode::Lshr {
                    self.unsigned(t);
                }
                // The amount, modulo the width (wasm takes 32 and 64 bits modulo already).
                self.get(b);
                if narrow(t) {
                    self.unsigned(t);
                    self.ins(I::I32Const(Self::bits(t))).ins(I::I32RemU);
                }
                self.ins(match (op, wide) {
                    (Opcode::Shl, false) => I::I32Shl,
                    (Opcode::Lshr, false) => I::I32ShrU,
                    (_, false) => I::I32ShrS,
                    (Opcode::Shl, true) => I::I64Shl,
                    (Opcode::Lshr, true) => I::I64ShrU,
                    (_, true) => I::I64ShrS,
                });
                self.canon(t);
            }
            _ => {
                self.get(a);
                self.get(b);
                self.ins(match (op, wide) {
                    (Opcode::Iadd, false) => I::I32Add,
                    (Opcode::Isub, false) => I::I32Sub,
                    (Opcode::Imul, false) => I::I32Mul,
                    (Opcode::And, false) => I::I32And,
                    (Opcode::Or, false) => I::I32Or,
                    (_, false) => I::I32Xor,
                    (Opcode::Iadd, true) => I::I64Add,
                    (Opcode::Isub, true) => I::I64Sub,
                    (Opcode::Imul, true) => I::I64Mul,
                    (Opcode::And, true) => I::I64And,
                    (Opcode::Or, true) => I::I64Or,
                    (_, true) => I::I64Xor,
                });
                self.canon(t);
            }
        }
        self.set_result(i);
    }

    /// Converts the value on the stack from `from` to `to`, as the interpreter's `cast` does.
    fn cast(&mut self, op: Opcode, from: Type, to: Type) {
        let float = |t: Type| matches!(t, Type::F32 | Type::F64);
        match op {
            Opcode::Zext => {
                self.unsigned(from);
                if to == Type::I64 && from != Type::I64 {
                    self.ins(I::I64ExtendI32U);
                } else {
                    self.canon(to);
                }
            }
            Opcode::Sext => {
                if from == Type::I1 {
                    // 1 becomes -1.
                    let t = self.scratch_i32;
                    self.ins(I::LocalSet(t))
                        .ins(I::I32Const(0))
                        .ins(I::LocalGet(t))
                        .ins(I::I32Sub);
                }
                if to == Type::I64 && from != Type::I64 {
                    self.ins(I::I64ExtendI32S);
                } else {
                    self.canon(to);
                }
            }
            Opcode::Trunc => {
                if from == Type::I64 && to != Type::I64 {
                    self.ins(I::I32WrapI64);
                }
                self.canon(to);
            }
            Opcode::Sitofp | Opcode::Uitofp => {
                let unsigned = op == Opcode::Uitofp;
                if unsigned {
                    self.unsigned(from);
                }
                let wide = from == Type::I64;
                self.ins(match (to == Type::F32, wide, unsigned) {
                    (true, false, false) => I::F32ConvertI32S,
                    (true, false, true) => I::F32ConvertI32U,
                    (true, true, false) => I::F32ConvertI64S,
                    (true, true, true) => I::F32ConvertI64U,
                    (false, false, false) => I::F64ConvertI32S,
                    (false, false, true) => I::F64ConvertI32U,
                    (false, true, false) => I::F64ConvertI64S,
                    (false, true, true) => I::F64ConvertI64U,
                });
            }
            Opcode::Fptosi | Opcode::Fptoui => self.float_to_int(from, to, op == Opcode::Fptosi),
            Opcode::Fext => {
                self.ins(I::F64PromoteF32);
            }
            Opcode::Ftrunc => {
                self.ins(I::F32DemoteF64);
            }
            Opcode::Bitcast => match (from, to) {
                (Type::F32, _) if !float(to) => {
                    self.ins(I::I32ReinterpretF32);
                }
                (Type::F64, _) if !float(to) => {
                    self.ins(I::I64ReinterpretF64);
                }
                (_, Type::F32) if !float(from) => {
                    self.ins(I::F32ReinterpretI32);
                }
                (_, Type::F64) if !float(from) => {
                    self.ins(I::F64ReinterpretI64);
                }
                _ => {}
            },
            Opcode::Ptrtoint => {
                if to == Type::I64 {
                    self.ins(I::I64ExtendI32U);
                } else {
                    self.canon(to);
                }
            }
            Opcode::Inttoptr => {
                if from == Type::I64 {
                    self.ins(I::I32WrapI64);
                } else {
                    self.unsigned(from);
                }
            }
            _ => unreachable!(),
        }
    }

    /// Saturating conversion of a float to an integer, NaN to 0, then to `to`'s canonical form.
    fn float_to_int(&mut self, from: Type, to: Type, signed: bool) {
        let f32 = from == Type::F32;
        if to == Type::I1 {
            // Nonzero after truncation is true.
            self.ins(if f32 {
                I::I64TruncSatF32S
            } else {
                I::I64TruncSatF64S
            })
            .ins(I::I64Const(0))
            .ins(I::I64Ne);
            return;
        }
        if to == Type::I64 {
            self.ins(match (f32, signed) {
                (true, true) => I::I64TruncSatF32S,
                (true, false) => I::I64TruncSatF32U,
                (false, true) => I::I64TruncSatF64S,
                (false, false) => I::I64TruncSatF64U,
            });
            return;
        }
        self.ins(match (f32, signed) {
            (true, true) => I::I32TruncSatF32S,
            (true, false) => I::I32TruncSatF32U,
            (false, true) => I::I32TruncSatF64S,
            (false, false) => I::I32TruncSatF64U,
        });
        if to == Type::I32 {
            return;
        }
        // Narrower: saturate to the type's range (signed, or as its unsigned bits), then canonical.
        let bits = Self::bits(to);
        let t = self.scratch_i32;
        let (lo, hi) = if signed {
            (-(1i32 << (bits - 1)), (1i32 << (bits - 1)) - 1)
        } else {
            (0, ((1i64 << bits) - 1) as i32)
        };
        self.ins(I::LocalSet(t));
        if signed {
            // t = min(t, hi), then max(t, lo); `select` gives its first operand when the condition
            // holds.
            self.ins(I::I32Const(hi))
                .ins(I::LocalGet(t))
                .ins(I::LocalGet(t))
                .ins(I::I32Const(hi))
                .ins(I::I32GtS)
                .ins(I::Select)
                .ins(I::LocalSet(t))
                .ins(I::I32Const(lo))
                .ins(I::LocalGet(t))
                .ins(I::LocalGet(t))
                .ins(I::I32Const(lo))
                .ins(I::I32LtS)
                .ins(I::Select);
        } else {
            // min_u(t, hi): the unsigned conversion is never below 0.
            self.ins(I::I32Const(hi))
                .ins(I::LocalGet(t))
                .ins(I::LocalGet(t))
                .ins(I::I32Const(hi))
                .ins(I::I32GtU)
                .ins(I::Select);
        }
        let _ = lo;
        self.canon(to);
    }
}
