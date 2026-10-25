//! Debug information for the platform's own debuggers (D59): DWARF line tables and the program's
//! functions, so that gdb and lldb set breakpoints by file and line, step by lines and show
//! backtraces in source terms. Variables are not described (their places would need Cranelift's
//! value locations). ELF and Mach-O only: Windows' debuggers read PDB, which A0 does not write.

use cranelift_object::object;
use cranelift_object::object::write::{Object, Relocation, SectionId, SymbolId};
use gimli::write::{
    Address, AttributeValue, DwarfUnit, EndianVec, LineProgram, LineString, Range, RangeList,
    Sections, Writer,
};
use gimli::{Encoding, Format, LineEncoding, LittleEndian};

/// A compiled function: its symbol, name, code size, and its code ranges with location ids
/// (`(start, end, location)`, offsets from its start).
pub(crate) struct Function<'a> {
    pub symbol: SymbolId,
    pub name: &'a str,
    pub size: u32,
    pub ranges: &'a [(u32, u32, u32)],
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
    // A language code of the user range (DW_LANG_lo_user): the languages are A0's.
    unit.set(gimli::DW_AT_language, AttributeValue::Udata(0x8000));
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
