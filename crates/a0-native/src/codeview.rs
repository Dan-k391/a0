//! Debug information for Windows' debuggers (D80): CodeView in `.debug$S` and `.debug$T`, which
//! the linker gathers into a PDB beside the executable. Visual Studio, WinDbg and cdb then set
//! breakpoints by file and line, step by lines, show backtraces in source terms and show
//! variables, as gdb and lldb do with the DWARF of `dwarf.rs`.
//!
//! Variables are read from the stack slots `-g` gives them (D80), relative to the frame pointer.
//! Numbers and booleans are CodeView's simple types; objects are pointers to structures with their
//! fields; strings and arrays are structures too, which the natvis file of [`natvis`] displays as
//! text and as elements.

use std::collections::HashMap;

use a0_ir::{DebugKind, DebugType, Module};
use a0_target::Arch;
use cranelift_object::object;
use cranelift_object::object::write::{Object, Relocation, SymbolId};

use crate::dwarf::{Function, VarPlace};

// Subsections of `.debug$S`.
const DEBUG_S_SYMBOLS: u32 = 0xf1;
const DEBUG_S_LINES: u32 = 0xf2;
const DEBUG_S_STRINGTABLE: u32 = 0xf3;
const DEBUG_S_FILECHKSMS: u32 = 0xf4;
const CV_SIGNATURE_C13: u32 = 4;

// Symbol records.
const S_END: u16 = 0x0006;
const S_FRAMEPROC: u16 = 0x1012;
const S_OBJNAME: u16 = 0x1101;
const S_GPROC32: u16 = 0x1110;
const S_REGREL32: u16 = 0x1111;
const S_UDT: u16 = 0x1108;
const S_COMPILE3: u16 = 0x113c;

// Type records.
const LF_POINTER: u16 = 0x1002;
const LF_PROCEDURE: u16 = 0x1008;
const LF_ARGLIST: u16 = 0x1201;
const LF_FIELDLIST: u16 = 0x1203;
const LF_ARRAY: u16 = 0x1503;
const LF_STRUCTURE: u16 = 0x1505;
const LF_MEMBER: u16 = 0x150d;
const LF_ULONG: u16 = 0x8004;

// Simple types.
const T_VOID: u32 = 0x0003;
const T_RCHAR: u32 = 0x0070;
const T_UQUAD: u32 = 0x0023;

/// A structure that is only declared (its fields come in a later record of the same name).
const FORWARD_REF: u16 = 0x80;
/// A structure with a unique name after its name, by which declarations find the definition
/// (languages may give different types one name).
const HAS_UNIQUE_NAME: u16 = 0x200;

/// Adds `.debug$S` and `.debug$T` describing `functions` to `obj`, a COFF object. Relative file
/// names are relative to `dir`.
pub(crate) fn emit(
    obj: &mut Object,
    m: &Module,
    arch: Arch,
    locations: &a0_ir::Locations,
    functions: &[Function],
    dir: &str,
    object_name: &str,
) -> Result<(), String> {
    let (machine, fp, secrel, section) = match arch {
        // CV_CFL_X64, CV_AMD64_RBP, IMAGE_REL_AMD64_SECREL and _SECTION.
        Arch::X86_64 => (0xd0u16, 334u16, 0x000bu16, 0x000au16),
        // CV_CFL_ARM64, CV_ARM64_FP, IMAGE_REL_ARM64_SECREL and _SECTION.
        Arch::Aarch64 => (0xf6, 79, 0x0008, 0x000d),
    };
    let mut types = Types {
        m,
        records: Vec::new(),
        next: 0x1000,
        words: HashMap::new(),
        structures: HashMap::new(),
        pointers: HashMap::new(),
        defined: Vec::new(),
    };
    let arglist = types.record(LF_ARGLIST, &0u32.to_le_bytes());
    let mut procedure = Vec::new();
    procedure.extend(T_VOID.to_le_bytes());
    procedure.extend([0, 0]); // near C call, no attributes
    procedure.extend(0u16.to_le_bytes());
    procedure.extend(arglist.to_le_bytes());
    let procedure = types.record(LF_PROCEDURE, &procedure);

    // The files, in the string table and the checksum table (without checksums).
    let mut strings = vec![0u8];
    let mut checksums = Vec::new();
    let mut files: HashMap<String, u32> = HashMap::new();
    let mut file_id = |name: &str| -> u32 {
        if let Some(&id) = files.get(name) {
            return id;
        }
        let path = std::path::Path::new(dir).join(name);
        let offset = strings.len() as u32;
        strings.extend(path.to_string_lossy().as_bytes());
        strings.push(0);
        let id = checksums.len() as u32;
        checksums.extend(offset.to_le_bytes());
        checksums.extend([0, 0, 0, 0]); // no checksum, padding
        files.insert(name.to_string(), id);
        id
    };

    let mut symbols = Vec::new();
    // Relocations in the symbols, against a function: (offset, function, whether it is the
    // section index rather than the offset).
    let mut relocations: Vec<(u32, usize, bool)> = Vec::new();
    let mut data = Vec::new();
    data.extend(0u32.to_le_bytes());
    data.extend(object_name.as_bytes());
    data.push(0);
    symbol(&mut symbols, S_OBJNAME, &data);
    let mut compile = Vec::new();
    compile.extend(0u32.to_le_bytes()); // language C, no flags
    compile.extend(machine.to_le_bytes());
    compile.extend([0u8; 16]); // front and back end versions
    compile.extend(b"A0\0");
    symbol(&mut symbols, S_COMPILE3, &compile);

    let mut lines = Vec::new();
    for (i, f) in functions.iter().enumerate() {
        // S_GPROC32: parent, end and next (the linker's), length, debug start and end, type,
        // offset and section (relocated), flags, name.
        let mut proc = Vec::new();
        proc.extend([0u8; 12]);
        proc.extend(f.size.to_le_bytes());
        proc.extend(0u32.to_le_bytes());
        proc.extend(f.size.to_le_bytes());
        proc.extend(procedure.to_le_bytes());
        let at = symbols.len() as u32 + 4 + proc.len() as u32;
        relocations.push((at, i, false));
        relocations.push((at + 4, i, true));
        proc.extend([0u8; 4 + 2 + 1]);
        proc.extend(f.name.as_bytes());
        proc.push(0);
        symbol(&mut symbols, S_GPROC32, &proc);
        // S_FRAMEPROC: the frame pointer is the base of locals and parameters.
        let mut frame = Vec::new();
        frame.extend([0u8; 4 * 5 + 2]);
        frame.extend(((2u32 << 14) | (2u32 << 16)).to_le_bytes());
        symbol(&mut symbols, S_FRAMEPROC, &frame);
        for place in f.vars {
            let Some((name, ty)) = m.var_debug(place.var) else {
                continue;
            };
            let ty = types.variable(ty, place);
            let mut var = Vec::new();
            var.extend((place.fp_offset as i32).to_le_bytes());
            var.extend(ty.to_le_bytes());
            var.extend(fp.to_le_bytes());
            var.extend(name.as_bytes());
            var.push(0);
            symbol(&mut symbols, S_REGREL32, &var);
        }
        symbol(&mut symbols, S_END, &[]);

        // The line table: a block of rows per run of rows in one file.
        let mut blocks: Vec<LineBlock> = Vec::new();
        let mut last = None;
        for &(offset, _, loc) in f.ranges {
            let Some(l) = locations.get(loc).filter(|l| l.line != 0) else {
                continue;
            };
            // A row per run of a line: Windows' debuggers find a line's code by its rows, and call
            // a line with several ambiguous.
            if last == Some((l.file.as_str(), l.line)) {
                continue;
            }
            last = Some((l.file.as_str(), l.line));
            let file = file_id(&l.file) * 8;
            if blocks.last().is_none_or(|(f, _)| *f != file) {
                blocks.push((file, Vec::new()));
            }
            blocks.last_mut().unwrap().1.push((offset, l.line, l.col));
        }
        if blocks.is_empty() {
            continue;
        }
        let mut table = Vec::new();
        table.extend([0u8; 6]); // offset and section, relocated
        table.extend(1u16.to_le_bytes()); // CV_LINES_HAVE_COLUMNS
        table.extend(f.size.to_le_bytes());
        for (file, rows) in &blocks {
            table.extend(file.to_le_bytes());
            table.extend((rows.len() as u32).to_le_bytes());
            table.extend((12 + 12 * rows.len() as u32).to_le_bytes());
            for &(offset, line, _) in rows {
                table.extend(offset.to_le_bytes());
                // The line, no end line, a statement.
                table.extend(((line & 0xff_ffff) | (1 << 31)).to_le_bytes());
            }
            for &(_, _, col) in rows {
                table.extend((col as u16).to_le_bytes());
                table.extend(0u16.to_le_bytes());
            }
        }
        lines.push((i, table));
    }

    // The structures' definitions, which nothing refers to by index: the linker keeps the types
    // symbols use.
    for (name, ty) in std::mem::take(&mut types.defined) {
        let mut udt = ty.to_le_bytes().to_vec();
        udt.extend(name.as_bytes());
        udt.push(0);
        symbol(&mut symbols, S_UDT, &udt);
    }

    // `.debug$S`: the symbols, a line table per function, the file and string tables.
    let mut section_data = CV_SIGNATURE_C13.to_le_bytes().to_vec();
    // Each subsection's data follows its kind and length.
    let symbols_at = section_data.len() as u32 + 8;
    subsection(&mut section_data, DEBUG_S_SYMBOLS, &symbols);
    let mut fixups: Vec<(u32, usize, bool)> = relocations
        .iter()
        .map(|&(at, f, s)| (symbols_at + at, f, s))
        .collect();
    for (f, table) in lines {
        // A line table starts with its function's offset and section.
        let at = section_data.len() as u32 + 8;
        fixups.push((at, f, false));
        fixups.push((at + 4, f, true));
        subsection(&mut section_data, DEBUG_S_LINES, &table);
    }
    subsection(&mut section_data, DEBUG_S_FILECHKSMS, &checksums);
    subsection(&mut section_data, DEBUG_S_STRINGTABLE, &strings);

    let mut type_data = CV_SIGNATURE_C13.to_le_bytes().to_vec();
    type_data.extend(&types.records);

    let debug_s = obj.add_section(Vec::new(), b".debug$S".to_vec(), object::SectionKind::Debug);
    obj.set_section_data(debug_s, section_data, 4);
    let debug_t = obj.add_section(Vec::new(), b".debug$T".to_vec(), object::SectionKind::Debug);
    obj.set_section_data(debug_t, type_data, 4);
    for (offset, f, is_section) in fixups {
        let symbol: SymbolId = functions[f].symbol;
        obj.add_relocation(
            debug_s,
            Relocation {
                offset: offset as u64,
                symbol,
                addend: 0,
                flags: object::RelocationFlags::Coff {
                    typ: object::pe::RelocationType(if is_section { section } else { secrel }),
                },
            },
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A function's rows in one file: the file's offset in the checksum table, and each row's code
/// offset, line and column.
type LineBlock = (u32, Vec<(u32, u32, u32)>);

/// Appends a subsection of `.debug$S`, padded to 4 bytes.
fn subsection(out: &mut Vec<u8>, kind: u32, data: &[u8]) {
    out.extend(kind.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// Appends a symbol record, padded to 4 bytes.
fn symbol(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
    let padding = (4 - (4 + data.len()) % 4) % 4;
    out.extend(((2 + data.len() + padding) as u16).to_le_bytes());
    out.extend(kind.to_le_bytes());
    out.extend(data);
    out.extend(std::iter::repeat_n(0, padding));
}

/// A numeric leaf: small values as they are, others after their kind.
fn numeric(out: &mut Vec<u8>, value: u32) {
    if value < 0x8000 {
        out.extend((value as u16).to_le_bytes());
    } else {
        out.extend(LF_ULONG.to_le_bytes());
        out.extend(value.to_le_bytes());
    }
}

/// Pads a record or a field list entry to 4 bytes with LF_PAD bytes.
fn pad(out: &mut Vec<u8>, start: usize) {
    while !(out.len() - start).is_multiple_of(4) {
        let left = 4 - (out.len() - start) % 4;
        out.push(0xf0 | left as u8);
    }
}

/// The type records of a module's debug types, made as variables need them.
struct Types<'a> {
    m: &'a Module,
    records: Vec<u8>,
    next: u32,
    /// A debug type as a word (or narrower) holds it, by type and size.
    words: HashMap<(DebugType, u32), u32>,
    /// The structure an object or inline struct of a debug type is: its forward declaration.
    structures: HashMap<DebugType, u32>,
    pointers: HashMap<u32, u32>,
    /// The complete structures, by name.
    defined: Vec<(String, u32)>,
}

impl Types<'_> {
    fn record(&mut self, kind: u16, body: &[u8]) -> u32 {
        let start = self.records.len();
        let padding = (4 - (4 + body.len()) % 4) % 4;
        self.records
            .extend(((2 + body.len() + padding) as u16).to_le_bytes());
        self.records.extend(kind.to_le_bytes());
        self.records.extend(body);
        pad(&mut self.records, start);
        let id = self.next;
        self.next += 1;
        id
    }

    fn variable(&mut self, t: DebugType, place: &VarPlace) -> u32 {
        self.word(t, place.size)
    }

    fn pointer(&mut self, to: u32) -> u32 {
        if let Some(&id) = self.pointers.get(&to) {
            return id;
        }
        let mut body = to.to_le_bytes().to_vec();
        // 64-bit pointer (CV_PTR_64), of size 8.
        body.extend((0x0cu32 | (8 << 13)).to_le_bytes());
        let id = self.record(LF_POINTER, &body);
        self.pointers.insert(to, id);
        id
    }

    /// The type of a value of `t` held in `size` bytes (a variable, a field, an element).
    fn word(&mut self, t: DebugType, size: u32) -> u32 {
        if let Some(&id) = self.words.get(&(t, size)) {
            return id;
        }
        let by_size = |types: [u32; 4]| match size {
            1 => types[0],
            2 => types[1],
            4 => types[2],
            _ => types[3],
        };
        let id = match self.m.debug_type_kind(t) {
            DebugKind::Int => by_size([0x68, 0x72, 0x74, 0x76]),
            DebugKind::UInt => by_size([0x69, 0x73, 0x75, 0x77]),
            DebugKind::Float => 0x41,
            DebugKind::Float32 => 0x40,
            DebugKind::Bool => by_size([0x30, 0x31, 0x32, 0x33]),
            DebugKind::Nullable => {
                let elem = self.m.debug_type_elems(t)[0];
                self.word(elem, size)
            }
            _ => {
                let structure = self.structure(t);
                self.pointer(structure)
            }
        };
        self.words.insert((t, size), id);
        id
    }

    fn structure_record(
        &mut self,
        t: DebugType,
        name: &str,
        fields: Option<(u16, u32)>,
        size: u32,
    ) -> u32 {
        let (count, list, property) = match fields {
            Some((count, list)) => (count, list, HAS_UNIQUE_NAME),
            None => (0, 0, FORWARD_REF | HAS_UNIQUE_NAME),
        };
        let mut body = Vec::new();
        body.extend(count.to_le_bytes());
        body.extend(property.to_le_bytes());
        body.extend(list.to_le_bytes());
        body.extend(0u32.to_le_bytes()); // derived from
        body.extend(0u32.to_le_bytes()); // virtual function table shape
        numeric(&mut body, size);
        body.extend(name.as_bytes());
        body.push(0);
        body.extend(format!("a0.{}", t.index()).as_bytes());
        body.push(0);
        self.record(LF_STRUCTURE, &body)
    }

    /// The structure a pointer of type `t` points at, as a forward declaration that its full
    /// record, of the same name, completes.
    fn structure(&mut self, t: DebugType) -> u32 {
        if let Some(&id) = self.structures.get(&t) {
            return id;
        }
        let m = self.m;
        let name = structure_name(m, t);
        let declared = self.structure_record(t, &name, None, 0);
        self.structures.insert(t, declared);
        // Members: name, type, offset; then the structure's size.
        let mut members: Vec<(String, u32, u32)> = Vec::new();
        let size = match m.debug_type_kind(t) {
            DebugKind::Struct | DebugKind::Inline => {
                let mut end = 0;
                for i in 0..m.debug_type_field_count(t) {
                    let (field, ft) = m.debug_type_field(t, i);
                    let (offset, size) = m.debug_type_field_place(t, i);
                    let ty = if m.debug_type_kind(ft) == DebugKind::Inline {
                        self.structure(ft)
                    } else {
                        self.word(ft, size)
                    };
                    members.push((field.to_string(), ty, offset));
                    end = end.max(offset + size);
                }
                end
            }
            DebugKind::String => {
                let mut array = T_RCHAR.to_le_bytes().to_vec();
                array.extend(T_UQUAD.to_le_bytes());
                numeric(&mut array, 1);
                array.push(0);
                let bytes = self.record(LF_ARRAY, &array);
                members.push(("len".to_string(), 0x76, 0));
                members.push(("bytes".to_string(), bytes, 16));
                17
            }
            DebugKind::Array => {
                let elem = m.debug_type_elems(t)[0];
                let elem = self.word(elem, 8);
                let data = self.pointer(elem);
                members.push(("len".to_string(), 0x76, 0));
                members.push(("cap".to_string(), 0x76, 8));
                members.push(("data".to_string(), data, 16));
                24
            }
            DebugKind::Enum => {
                members.push(("variant".to_string(), 0x76, 0));
                8
            }
            DebugKind::Boxed => {
                let elem = m.debug_type_elems(t)[0];
                let elem = self.word(elem, 8);
                members.push(("value".to_string(), elem, 0));
                8
            }
            // Maps, dynamic values, interface values and opaque ones: declared only.
            _ => return declared,
        };
        let mut list = Vec::new();
        for (name, ty, offset) in &members {
            let start = list.len();
            list.extend(LF_MEMBER.to_le_bytes());
            list.extend(3u16.to_le_bytes()); // public
            list.extend(ty.to_le_bytes());
            numeric(&mut list, *offset);
            list.extend(name.as_bytes());
            list.push(0);
            pad(&mut list, start);
        }
        let list = self.record(LF_FIELDLIST, &list);
        let defined = self.structure_record(t, &name, Some((members.len() as u16, list)), size);
        self.defined.push((name, defined));
        declared
    }
}

/// A structure's name: the type's, but a string's in A0's namespace, since WinDbg does not apply
/// visualizers to a type named `string`.
fn structure_name(m: &Module, t: DebugType) -> String {
    match m.debug_type_kind(t) {
        DebugKind::String => format!("a0::{}", m.debug_type_name(t)),
        _ => m.debug_type_name(t).to_string(),
    }
}

/// A natvis file for Visual Studio's and WinDbg's display of the module's strings (as text) and
/// arrays (as their elements); the linker puts it in the PDB.
pub(crate) fn natvis(m: &Module) -> String {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <AutoVisualizer xmlns=\"http://schemas.microsoft.com/vstudio/debugger/natvis/2010\">\n",
    );
    let mut seen = std::collections::HashSet::new();
    for i in 0..m.debug_type_count() {
        let t = DebugType(i);
        let name = structure_name(m, t);
        if !seen.insert(name.clone()) {
            continue;
        }
        match m.debug_type_kind(t) {
            DebugKind::String => out.push_str(&format!(
                "  <Type Name=\"{}\">\n    <DisplayString>{{&amp;bytes[0],[len]s8}}</DisplayString>\n    \
                 <StringView>&amp;bytes[0],[len]s8</StringView>\n  </Type>\n",
                escape(&name)
            )),
            DebugKind::Array => out.push_str(&format!(
                "  <Type Name=\"{}\">\n    <DisplayString>{{{{ len={{len}} }}}}</DisplayString>\n    \
                 <Expand>\n      <ArrayItems>\n        <Size>len</Size>\n        \
                 <ValuePointer>data</ValuePointer>\n      </ArrayItems>\n    </Expand>\n  </Type>\n",
                escape(&name)
            )),
            _ => {}
        }
    }
    out.push_str("</AutoVisualizer>\n");
    out
}
