//! Debug information for the platform's own debuggers (D59): DWARF line tables, the program's
//! functions and their variables, so that gdb and lldb set breakpoints by file and line, step by
//! lines, show backtraces in source terms and print variables. ELF and Mach-O.
//!
//! With `-g` every source variable has a stack slot that each assignment also stores to (D80), at
//! a fixed offset from the frame pointer; its type comes from the IR's debug types: numbers and
//! booleans as base types, strings as C strings, objects as pointers to structures with their
//! fields, captured variables as what their boxes hold.

use cranelift_object::object;
use cranelift_object::object::write::{Object, Relocation, SectionId, SymbolId};
use std::collections::HashMap;

use a0_ir::{DebugKind, DebugType, Module};
use a0_target::{Arch, Target as Platform};
use gimli::write::{
    Address, AttributeValue, DwarfUnit, EndianVec, Expression, LineProgram, LineString, Range,
    RangeList, Sections, Unit, UnitEntryId, Writer,
};
use gimli::{Encoding, Format, LineEncoding, LittleEndian, Register};

/// A compiled function: its symbol, name, code size, and its code ranges with location ids
/// (`(start, end, location)`, offsets from its start).
pub(crate) struct Function<'a> {
    pub symbol: SymbolId,
    pub name: &'a str,
    pub size: u32,
    pub ranges: &'a [(u32, u32, u32)],
    pub vars: &'a [VarPlace],
}

/// A source variable's stack slot: its offset from the frame pointer and its size in bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VarPlace {
    pub var: a0_ir::Var,
    pub fp_offset: i64,
    pub size: u32,
}

/// A section being written, with the relocations its addresses and offsets need.
#[derive(Clone)]
struct Section {
    data: EndianVec<LittleEndian>,
    relocations: Vec<(u64, u8, Target, i64)>,
}

#[derive(Clone, Copy)]
enum Target {
    /// An index into the functions.
    Symbol(usize),
    Section(gimli::SectionId),
}

impl Section {
    fn new() -> Section {
        Section {
            data: EndianVec::new(LittleEndian),
            relocations: Vec::new(),
        }
    }
}

impl Writer for Section {
    type Endian = LittleEndian;

    fn endian(&self) -> LittleEndian {
        LittleEndian
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn write(&mut self, bytes: &[u8]) -> gimli::write::Result<()> {
        self.data.write(bytes)
    }

    fn write_at(&mut self, offset: usize, bytes: &[u8]) -> gimli::write::Result<()> {
        self.data.write_at(offset, bytes)
    }

    fn write_address(&mut self, address: Address, size: u8) -> gimli::write::Result<()> {
        match address {
            Address::Constant(v) => self.write_udata(v, size),
            Address::Symbol { symbol, addend } => {
                let at = self.len() as u64;
                self.relocations
                    .push((at, size, Target::Symbol(symbol), addend));
                self.write_udata(0, size)
            }
        }
    }

    fn write_offset(
        &mut self,
        val: usize,
        section: gimli::SectionId,
        size: u8,
    ) -> gimli::write::Result<()> {
        let at = self.len() as u64;
        self.relocations
            .push((at, size, Target::Section(section), val as i64));
        self.write_udata(val as u64, size)
    }

    fn write_offset_at(
        &mut self,
        offset: usize,
        val: usize,
        section: gimli::SectionId,
        size: u8,
    ) -> gimli::write::Result<()> {
        self.relocations
            .push((offset as u64, size, Target::Section(section), val as i64));
        self.write_udata_at(offset, val as u64, size)
    }
}

/// Adds the DWARF sections describing `functions` to `obj`. `locations` gives the file, line and
/// column of each location id; relative file names are relative to `dir`.
pub(crate) fn emit(
    obj: &mut Object,
    m: &Module,
    target: Platform,
    locations: &a0_ir::Locations,
    functions: &[Function],
    dir: &str,
) -> Result<(), String> {
    // dsymutil and lldb take the addresses written in Mach-O debug sections as they are, as
    // addresses within the object, and use a relocation only to learn which section an address
    // is in (LLVM writes them that way too). The object writer places the code, so write the
    // object once to learn where each function went; debug sections come after it and leave
    // that in place.
    let macho = obj.format() == object::BinaryFormat::MachO;
    let mut addresses = vec![0; functions.len()];
    if macho {
        use object::{Object as _, ObjectSection as _};
        let bytes = obj.write().map_err(|e| e.to_string())?;
        let file = object::File::parse(&*bytes).map_err(|e| e.to_string())?;
        for (i, f) in functions.iter().enumerate() {
            let symbol = obj.symbol(f.symbol);
            let object::write::SymbolSection::Section(section) = symbol.section else {
                return Err(format!("{} is not in a section", f.name));
            };
            let name = obj.section(section).name().unwrap_or_default();
            let Some(placed) = file.section_by_name(name) else {
                return Err(format!("no section {name} in the object"));
            };
            addresses[i] = placed.address() + symbol.value;
        }
    }
    let encoding = Encoding {
        format: Format::Dwarf32,
        version: 4,
        address_size: 8,
    };
    let main_file = functions
        .iter()
        .flat_map(|f| f.ranges.iter())
        .find_map(|&(_, _, loc)| locations.get(loc).filter(|l| !l.file.is_empty()))
        .map(|l| l.file.clone())
        .unwrap_or_else(|| "program".to_string());
    let mut dwarf = DwarfUnit::new(encoding);
    let mut program = LineProgram::new(
        encoding,
        LineEncoding::default(),
        LineString::String(dir.as_bytes().to_vec()),
        None,
        LineString::String(main_file.as_bytes().to_vec()),
        None,
    );
    let directory = program.default_directory();
    let mut files = std::collections::HashMap::new();
    let mut ranges = Vec::new();
    let root = dwarf.unit.root();
    let fp = match target.arch() {
        Arch::X86_64 => Register(6),
        Arch::Aarch64 => Register(29),
    };
    let mut types = Types {
        m,
        root,
        words: HashMap::new(),
        structures: HashMap::new(),
        base: HashMap::new(),
    };
    for (i, f) in functions.iter().enumerate() {
        let start = Address::Symbol {
            symbol: i,
            addend: 0,
        };
        ranges.push(Range::StartLength {
            begin: start,
            length: f.size as u64,
        });
        let die = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        let entry = dwarf.unit.get_mut(die);
        entry.set(
            gimli::DW_AT_name,
            AttributeValue::String(f.name.as_bytes().to_vec()),
        );
        entry.set(gimli::DW_AT_low_pc, AttributeValue::Address(start));
        entry.set(gimli::DW_AT_high_pc, AttributeValue::Udata(f.size as u64));
        entry.set(gimli::DW_AT_external, AttributeValue::Flag(true));
        entry.set(
            gimli::DW_AT_frame_base,
            AttributeValue::Exprloc({
                let mut e = Expression::new();
                e.op_reg(fp);
                e
            }),
        );
        for place in f.vars {
            let Some((name, ty)) = m.var_debug(place.var) else {
                continue;
            };
            let (ty, location) = types.variable(&mut dwarf.unit, ty, place, fp);
            let var = dwarf.unit.add(die, gimli::DW_TAG_variable);
            let entry = dwarf.unit.get_mut(var);
            entry.set(
                gimli::DW_AT_name,
                AttributeValue::String(name.as_bytes().to_vec()),
            );
            entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(ty));
            entry.set(gimli::DW_AT_location, AttributeValue::Exprloc(location));
        }

        program.begin_sequence(Some(start));
        let mut last = None;
        for &(offset, _, loc) in f.ranges {
            let Some(l) = locations.get(loc).filter(|l| l.line != 0) else {
                continue;
            };
            if last == Some((l.file.as_str(), l.line, l.col)) {
                continue;
            }
            last = Some((l.file.as_str(), l.line, l.col));
            let file = *files.entry(l.file.clone()).or_insert_with(|| {
                program.add_file(
                    LineString::String(l.file.as_bytes().to_vec()),
                    directory,
                    None,
                )
            });
            let row = program.row();
            row.address_offset = offset as u64;
            row.file = file;
            row.line = l.line as u64;
            row.column = l.col as u64;
            program.generate_row();
        }
        program.end_sequence(f.size as u64);
    }
    dwarf.unit.line_program = program;
    let range_list = dwarf.unit.ranges.add(RangeList(ranges));
    let unit = dwarf.unit.get_mut(root);
    unit.set(
        gimli::DW_AT_producer,
        AttributeValue::String(b"A0".to_vec()),
    );
    unit.set(
        gimli::DW_AT_name,
        AttributeValue::String(main_file.as_bytes().to_vec()),
    );
    unit.set(
        gimli::DW_AT_comp_dir,
        AttributeValue::String(dir.as_bytes().to_vec()),
    );
    // C: the types are C's (base types, structures, pointers), and lldb shows no variables of a
    // language it does not know (gdb falls back to C).
    unit.set(
        gimli::DW_AT_language,
        AttributeValue::Language(gimli::DW_LANG_C99),
    );
    unit.set(
        gimli::DW_AT_low_pc,
        AttributeValue::Address(Address::Constant(0)),
    );
    unit.set(
        gimli::DW_AT_ranges,
        AttributeValue::RangeListRef(range_list),
    );

    let mut sections = Sections::new(Section::new());
    dwarf.write(&mut sections).map_err(|e| e.to_string())?;
    let mut written: Vec<(gimli::SectionId, Section)> = Vec::new();
    sections
        .for_each(|id, s| -> Result<(), ()> {
            if s.len() > 0 {
                written.push((id, s.clone()));
            }
            Ok(())
        })
        .map_err(|_| "cannot collect the debug sections".to_string())?;

    // Mach-O does not link DWARF: debuggers read it from the object files, so offsets between its
    // sections need no relocations there. ELF linkers concatenate the sections of every object.
    let segment = obj
        .segment_name(object::write::StandardSegment::Debug)
        .to_vec();
    let mut ids: Vec<(gimli::SectionId, SectionId)> = Vec::new();
    for (id, s) in &written {
        let name = if macho {
            format!("__{}", &id.name()[1..])
        } else {
            id.name().to_string()
        };
        let section = obj.add_section(
            segment.clone(),
            name.into_bytes(),
            object::SectionKind::Debug,
        );
        obj.set_section_data(section, s.data.slice().to_vec(), 1);
        ids.push((*id, section));
    }
    for (id, s) in &written {
        let section = ids.iter().find(|(g, _)| g == id).map(|&(_, s)| s).unwrap();
        for &(offset, size, target, addend) in &s.relocations {
            let (symbol, addend) = match target {
                Target::Symbol(i) if macho => {
                    let object::write::SymbolSection::Section(code) =
                        obj.symbol(functions[i].symbol).section
                    else {
                        unreachable!("checked above");
                    };
                    (obj.section_symbol(code), addresses[i] as i64 + addend)
                }
                Target::Symbol(i) => (functions[i].symbol, addend),
                Target::Section(_) if macho => continue,
                Target::Section(g) => match ids.iter().find(|(other, _)| *other == g) {
                    Some(&(_, other)) => (obj.section_symbol(other), addend),
                    None => continue,
                },
            };
            obj.add_relocation(
                section,
                Relocation {
                    offset,
                    symbol,
                    addend,
                    flags: object::RelocationFlags::Generic {
                        kind: object::RelocationKind::Absolute,
                        encoding: object::RelocationEncoding::Generic,
                        size: size * 8,
                    },
                },
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The DWARF types of a module's debug types, made as variables need them.
struct Types<'a> {
    m: &'a Module,
    root: UnitEntryId,
    /// A debug type as a word (or narrower) holds it, by type and size.
    words: HashMap<(DebugType, u32), UnitEntryId>,
    /// The structure an object or inline struct of a debug type is.
    structures: HashMap<DebugType, UnitEntryId>,
    /// Base types, by name, encoding and size.
    base: HashMap<(String, u8, u32), UnitEntryId>,
}

impl Types<'_> {
    /// A variable's type and location. A captured variable is what its box holds, and a string is
    /// its bytes, which debuggers show as C strings (A0's strings end with a 0).
    fn variable(
        &mut self,
        unit: &mut Unit,
        mut t: DebugType,
        place: &VarPlace,
        fp: Register,
    ) -> (UnitEntryId, Expression) {
        let mut e = Expression::new();
        e.op_breg(fp, place.fp_offset);
        let mut size = place.size;
        loop {
            match self.m.debug_type_kind(t) {
                DebugKind::Boxed => {
                    e.op_deref();
                    t = self.m.debug_type_elems(t)[0];
                    size = 8;
                }
                DebugKind::Nullable => t = self.m.debug_type_elems(t)[0],
                _ => break,
            }
        }
        if self.m.debug_type_kind(t) != DebugKind::String {
            return (self.word(unit, t, size), e);
        }
        // The string's address, or 0, plus the offset of its bytes, as a value.
        e.op_deref();
        e.op(gimli::DW_OP_dup);
        let nonzero = e.op_bra();
        let skip = e.op_skip();
        e.set_target(nonzero, e.next_index());
        e.op_plus_uconst(16);
        e.set_target(skip, e.next_index());
        e.op(gimli::DW_OP_stack_value);
        let char = self.base_type(unit, "char", gimli::DW_ATE_signed_char, 1);
        let pointer = self.pointer(unit, char);
        let name = self.m.debug_type_name(t).to_string();
        let ty = self.add(unit, gimli::DW_TAG_typedef, &name, None);
        unit.get_mut(ty)
            .set(gimli::DW_AT_type, AttributeValue::UnitRef(pointer));
        (ty, e)
    }

    fn add(
        &mut self,
        unit: &mut Unit,
        tag: gimli::DwTag,
        name: &str,
        size: Option<u32>,
    ) -> UnitEntryId {
        let id = unit.add(self.root, tag);
        let entry = unit.get_mut(id);
        if !name.is_empty() {
            entry.set(
                gimli::DW_AT_name,
                AttributeValue::String(name.as_bytes().to_vec()),
            );
        }
        if let Some(size) = size {
            entry.set(gimli::DW_AT_byte_size, AttributeValue::Udata(size as u64));
        }
        id
    }

    fn base_type(
        &mut self,
        unit: &mut Unit,
        name: &str,
        encoding: gimli::DwAte,
        size: u32,
    ) -> UnitEntryId {
        let key = (name.to_string(), encoding.0, size);
        if let Some(&id) = self.base.get(&key) {
            return id;
        }
        let id = self.add(unit, gimli::DW_TAG_base_type, name, Some(size));
        unit.get_mut(id)
            .set(gimli::DW_AT_encoding, AttributeValue::Encoding(encoding));
        self.base.insert(key, id);
        id
    }

    fn pointer(&mut self, unit: &mut Unit, to: UnitEntryId) -> UnitEntryId {
        let id = self.add(unit, gimli::DW_TAG_pointer_type, "", Some(8));
        unit.get_mut(id)
            .set(gimli::DW_AT_type, AttributeValue::UnitRef(to));
        id
    }

    fn member(
        &mut self,
        unit: &mut Unit,
        of: UnitEntryId,
        name: &str,
        ty: UnitEntryId,
        offset: u32,
    ) {
        let id = unit.add(of, gimli::DW_TAG_member);
        let entry = unit.get_mut(id);
        entry.set(
            gimli::DW_AT_name,
            AttributeValue::String(name.as_bytes().to_vec()),
        );
        entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(ty));
        entry.set(
            gimli::DW_AT_data_member_location,
            AttributeValue::Udata(offset as u64),
        );
    }

    /// The type of a value of `t` held in `size` bytes (a variable, a field, an element).
    fn word(&mut self, unit: &mut Unit, t: DebugType, size: u32) -> UnitEntryId {
        if let Some(&id) = self.words.get(&(t, size)) {
            return id;
        }
        let name = self.m.debug_type_name(t).to_string();
        let id = match self.m.debug_type_kind(t) {
            DebugKind::Int => self.base_type(unit, &name, gimli::DW_ATE_signed, size),
            DebugKind::UInt => self.base_type(unit, &name, gimli::DW_ATE_unsigned, size),
            DebugKind::Float => self.base_type(unit, &name, gimli::DW_ATE_float, size),
            DebugKind::Float32 => self.base_type(unit, &name, gimli::DW_ATE_float, 4),
            DebugKind::Bool => self.base_type(unit, &name, gimli::DW_ATE_boolean, size),
            DebugKind::Nullable => {
                let elem = self.m.debug_type_elems(t)[0];
                self.word(unit, elem, size)
            }
            _ => {
                let structure = self.structure(unit, t);
                self.pointer(unit, structure)
            }
        };
        self.words.insert((t, size), id);
        id
    }

    /// The structure a pointer of type `t` points at.
    fn structure(&mut self, unit: &mut Unit, t: DebugType) -> UnitEntryId {
        if let Some(&id) = self.structures.get(&t) {
            return id;
        }
        let m = self.m;
        let name = m.debug_type_name(t).to_string();
        let id = self.add(unit, gimli::DW_TAG_structure_type, &name, None);
        // Before the members, which may point back at it.
        self.structures.insert(t, id);
        let int = self.base_type(unit, "int64_t", gimli::DW_ATE_signed, 8);
        match m.debug_type_kind(t) {
            DebugKind::Struct | DebugKind::Inline => {
                let mut end = 0;
                for i in 0..m.debug_type_field_count(t) {
                    let (field, ft) = m.debug_type_field(t, i);
                    let (offset, size) = m.debug_type_field_place(t, i);
                    // An inline struct in a field is its bytes there.
                    let ty = if m.debug_type_kind(ft) == DebugKind::Inline {
                        self.structure(unit, ft)
                    } else {
                        self.word(unit, ft, size)
                    };
                    self.member(unit, id, field, ty, offset);
                    end = end.max(offset + size);
                }
                unit.get_mut(id)
                    .set(gimli::DW_AT_byte_size, AttributeValue::Udata(end as u64));
            }
            DebugKind::String => {
                let char = self.base_type(unit, "char", gimli::DW_ATE_signed_char, 1);
                let bytes = unit.add(self.root, gimli::DW_TAG_array_type);
                unit.get_mut(bytes)
                    .set(gimli::DW_AT_type, AttributeValue::UnitRef(char));
                unit.add(bytes, gimli::DW_TAG_subrange_type);
                self.member(unit, id, "len", int, 0);
                self.member(unit, id, "bytes", bytes, 16);
                unit.get_mut(id)
                    .set(gimli::DW_AT_byte_size, AttributeValue::Udata(16));
            }
            DebugKind::Array => {
                let elem = m.debug_type_elems(t)[0];
                let elem = self.word(unit, elem, 8);
                let data = self.pointer(unit, elem);
                self.member(unit, id, "len", int, 0);
                self.member(unit, id, "cap", int, 8);
                self.member(unit, id, "data", data, 16);
                unit.get_mut(id)
                    .set(gimli::DW_AT_byte_size, AttributeValue::Udata(24));
            }
            DebugKind::Enum | DebugKind::Boxed => {
                let (field, ty) = if m.debug_type_kind(t) == DebugKind::Enum {
                    ("variant", int)
                } else {
                    let elem = m.debug_type_elems(t)[0];
                    ("value", self.word(unit, elem, 8))
                };
                self.member(unit, id, field, ty, 0);
                unit.get_mut(id)
                    .set(gimli::DW_AT_byte_size, AttributeValue::Udata(8));
            }
            // Maps, dynamic values, interface values and opaque ones: named, contents not
            // described (`lang dap` shows them).
            _ => unit
                .get_mut(id)
                .set(gimli::DW_AT_declaration, AttributeValue::Flag(true)),
        }
        id
    }
}
