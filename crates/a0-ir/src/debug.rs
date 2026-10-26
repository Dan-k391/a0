//! Where instructions come from, for runtime errors.
//!
//! Both backends give the runtime the same table of locations, so a trap or a panic says where it
//! happened and how the program got there, in the interpreter and in native code alike. A location
//! is a function as traces name it and a place in the source; one that was inlined has a parent,
//! the location of the call it was inlined at, so traces show inlined functions as frames of their
//! own.

use std::collections::HashMap;

use a0_diag::Span;

use crate::module::Module;
use crate::types::{FuncId, InlineId, Inst};

/// One location. `file` is empty (and `line` and `col` 0) when it is not known.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Location {
    /// The function, as stack traces name it.
    pub func: String,
    /// The source file.
    pub file: String,
    /// The line, from 1.
    pub line: u32,
    /// The column, from 1, in characters.
    pub col: u32,
    /// The location this one was inlined at, as an index into the table plus one (0 for none).
    pub parent: u32,
}

/// The locations of a module's instructions.
pub struct Locations {
    /// The locations, which instructions refer to by index.
    pub entries: Vec<Location>,
    /// Per instruction: its location's index plus one, or 0.
    inst: Vec<u32>,
}

impl Locations {
    /// The locations of every instruction of a function that traces show.
    pub fn build(m: &Module) -> Locations {
        let mut b = TableBuilder {
            m,
            out: Locations {
                entries: Vec::new(),
                inst: vec![0; m.insts.len()],
            },
            sites: HashMap::new(),
        };
        // Instructions that share a location are found by numbers, without making its strings.
        // (Copies of one generic function may repeat a location; that costs a few entries.)
        let mut memo: HashMap<MemoKey, u32> = HashMap::new();
        for (i, data) in m.insts.iter().enumerate() {
            let Some(name) = m.func_trace_name(data.func) else {
                continue;
            };
            let place = data.span.map(|s| (s.source.0, s.start));
            let id = match data.inline.option() {
                None => {
                    let key = (data.func.0, 0, place);
                    match memo.get(&key) {
                        Some(&id) => id,
                        None => {
                            let id = b.intern(name.to_string(), data.span, 0);
                            memo.insert(key, id);
                            id
                        }
                    }
                }
                Some(site) => {
                    let parent = b.site(data.func, site);
                    // The callee is the site's, so the parent and the place decide the location.
                    let key = (u32::MAX - site.0, parent, place);
                    match memo.get(&key) {
                        Some(&id) => id,
                        None => {
                            let callee = m.inline_site(site).callee.clone();
                            let id = b.intern(callee, data.span, parent);
                            memo.insert(key, id);
                            id
                        }
                    }
                }
            };
            b.out.inst[i] = id;
        }
        b.out
    }

    /// The location of an instruction, as an id (index plus one; 0 if it has none).
    pub fn of(&self, i: Inst) -> u32 {
        self.inst.get(i.index()).copied().unwrap_or(0)
    }

    /// The location with id `id` (index plus one).
    pub fn get(&self, id: u32) -> Option<&Location> {
        (id as usize)
            .checked_sub(1)
            .and_then(|i| self.entries.get(i))
    }
}

/// A location by numbers: the function (or the inline site), the parent location, and the span's
/// source and start.
type MemoKey = (u32, u32, Option<(u32, u32)>);

struct TableBuilder<'a> {
    m: &'a Module,
    out: Locations,
    /// The location of the call each inline site stands for, per function.
    sites: HashMap<(FuncId, InlineId), u32>,
}

impl TableBuilder<'_> {
    fn intern(&mut self, func: String, span: Option<Span>, parent: u32) -> u32 {
        let (file, line, col) = match (span, self.m.sources()) {
            (Some(s), Some(sources)) if sources.contains(s.source) => {
                let (line, col) = sources.line_col(s.source, s.start);
                (sources.name(s.source).to_string(), line, col)
            }
            _ => (String::new(), 0, 0),
        };
        let loc = Location {
            func,
            file,
            line,
            col,
            parent,
        };
        self.out.entries.push(loc);
        self.out.entries.len() as u32
    }

    /// The location of the call that inline site `site` stands for: in the function that called
    /// the inlined one, which may itself have been inlined.
    fn site(&mut self, f: FuncId, site: InlineId) -> u32 {
        if let Some(&id) = self.sites.get(&(f, site)) {
            return id;
        }
        let data = self.m.inline_site(site).clone();
        let id = match data.parent {
            None => {
                let name = self.m.func_trace_name(f).unwrap_or_default().to_string();
                self.intern(name, data.call, 0)
            }
            Some(outer) => {
                let parent = self.site(f, outer);
                let callee = self.m.inline_site(outer).callee.clone();
                self.intern(callee, data.call, parent)
            }
        };
        self.sites.insert((f, site), id);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Builder, Type};
    use a0_diag::SourceMap;

    #[test]
    fn inlined_instructions_have_the_call_they_were_inlined_at_as_parent() {
        let mut sources = SourceMap::new();
        let text = "fn main() {\n  helper();\n}\nfn helper() {\n  fail();\n}\n";
        let id = sources.add("prog.src", text);
        let at = |needle: &str| {
            let s = text.find(needle).unwrap() as u32;
            Some(Span::new(id, s, s + needle.len() as u32))
        };
        let mut b = Builder::new("m");
        let main = b.declare_function("main", &[], Some(Type::I32));
        b.begin_function(main);
        b.set_span(at("helper()"));
        let site = b.add_inline_site("helper", at("helper()"), None);
        b.set_inline(Some(site));
        b.set_span(at("fail()"));
        let zero = b.iconst(Type::I32, 0);
        b.set_inline(None);
        b.ret(Some(zero));
        b.end_function();
        let mut m = b.finish().unwrap();
        m.set_sources(sources);
        let locs = Locations::build(&m);
        let first = m.region_insts(m.func_body(main).unwrap())[0];
        let loc = locs.get(locs.of(first)).unwrap();
        assert_eq!(
            (loc.func.as_str(), loc.file.as_str(), loc.line, loc.col),
            ("helper", "prog.src", 5, 3)
        );
        let parent = locs.get(loc.parent).unwrap();
        assert_eq!(
            (parent.func.as_str(), parent.line, parent.col),
            ("main", 2, 3)
        );
        assert_eq!(parent.parent, 0);
    }
}
