//! Completion, find-references and rename, for any language: from the grammar's indexes of the
//! program's files (`@def`, `@scope`), the imports between them (`@import`), and, for members after
//! `.`, the types of the HIR. Across files a top-level item is found where it is imported: as
//! `alias.name`, or by its name where the file imports it unqualified. It is all syntax, so rename
//! works on programs with type errors too.

use std::collections::BTreeMap;

use a0_diag::{SourceId, Span};
use a0_hir::{HirModule, TypeId};
use a0_syntax::{SymbolKind, SyntaxIndex, TokenClass};

use super::{Document, FileInfo, Server, declaration_header, hir_type};
use crate::json::Json;

/// The protocol's `CompletionItemKind` numbers.
fn completion_kind(kind: SymbolKind) -> u32 {
    match kind {
        SymbolKind::Function => 3,
        SymbolKind::Method => 2,
        SymbolKind::Class => 7,
        SymbolKind::Struct => 22,
        SymbolKind::Enum => 13,
        SymbolKind::Interface => 8,
        SymbolKind::Variant => 20,
        SymbolKind::Field => 5,
        SymbolKind::Variable | SymbolKind::Parameter => 6,
        SymbolKind::Constant => 21,
        SymbolKind::TypeParameter => 25,
        SymbolKind::Type => 7,
        SymbolKind::Module => 9,
    }
}

const KEYWORD: u32 = 14;
const MODULE: u32 = 9;

/// What a name at a position stands for.
enum Target {
    /// Symbols of one file of the program: a local, or the candidates for a member name.
    Local(usize, Vec<u32>),
    /// A top-level item, which other files may import.
    Global(usize, u32),
}

fn token_text(file: &FileInfo, t: u32) -> &str {
    let token = file.index.token(t);
    &file.text[token.start as usize..token.end as usize]
}

/// The global symbol of a file that is called `name`.
fn global_named(file: &FileInfo, name: &str) -> Option<u32> {
    (0..file.index.symbol_count()).find(|&s| {
        file.index.symbol_is_global(s)
            && !file.index.symbol_kind(s).is_member()
            && file.index.symbol_name(s) == name
    })
}

/// For a token after `.` or `::`, the name before them (`alias` in `alias.name`).
fn qualifier(file: &FileInfo, t: u32) -> Option<&str> {
    let dot = t.checked_sub(1)?;
    let name = t.checked_sub(2)?;
    let dot_text = token_text(file, dot);
    (matches!(dot_text, "." | "::") && file.index.token(name).class == TokenClass::Name)
        .then(|| token_text(file, name))
}

impl Server<'_> {
    pub(super) fn completion(&self, params: &Json) -> Json {
        let Some((doc, offset)) = self.position(params) else {
            return Json::Null;
        };
        let Some(index) = &doc.index else {
            return Json::Null;
        };
        let text = &doc.text;
        // The name being typed, if any, starts here.
        let start = text[..offset as usize]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .last()
            .map_or(offset, |(i, _)| i as u32);
        let before = text[..start as usize].trim_end();
        let mut items: BTreeMap<String, Json> = BTreeMap::new();
        let mut add = |label: &str, kind: u32, detail: Option<String>| {
            let mut item = vec![
                ("label".to_string(), label.into()),
                ("kind".to_string(), kind.into()),
            ];
            if let Some(detail) = detail {
                item.push(("detail".to_string(), detail.into()));
            }
            items
                .entry(label.to_string())
                .or_insert_with(|| Json::Object(item));
        };
        let this = doc.program.iter().find(|f| f.source == doc.source);
        if let Some(receiver) = before
            .strip_suffix("::")
            .or_else(|| before.strip_suffix('.'))
        {
            // A member: of a module, of a value whose type the HIR knows, or any member at all.
            let receiver_end = receiver.trim_end().len() as u32;
            let receiver_token = index
                .token_at(receiver_end)
                .filter(|&t| index.token(t).end == receiver_end)
                .filter(|&t| index.token(t).class == TokenClass::Name);
            let receiver_name = receiver_token.map(|t| {
                let token = index.token(t);
                &text[token.start as usize..token.end as usize]
            });
            let module = this.and_then(|f| {
                let name = receiver_name?;
                f.imports.iter().find(|(alias, ..)| alias == name)
            });
            if let Some(&(_, file, _)) = module {
                let file = &doc.program[file];
                for s in 0..file.index.symbol_count() {
                    if file.index.symbol_is_global(s) && !file.index.symbol_kind(s).is_member() {
                        let kind = file.index.symbol_kind(s);
                        let header =
                            declaration_header(&file.text, file.index.symbol_extent(s), kind);
                        add(
                            file.index.symbol_name(s),
                            completion_kind(kind),
                            Some(header),
                        );
                    }
                }
            } else if let Some(members) = receiver_token
                .and_then(|t| index.targets(t).first().copied())
                .and_then(|s| Some((doc.hir.as_ref()?, s)))
                .and_then(|(m, s)| Some((m, local_type(m, doc, index, s)?)))
                .map(|(m, t)| m.members(t))
                .filter(|members| !members.is_empty())
            {
                for (name, detail, method) in members {
                    add(&name, if method { 2 } else { 5 }, Some(detail));
                }
            } else {
                for s in 0..index.symbol_count() {
                    let kind = index.symbol_kind(s);
                    if kind.is_member() {
                        add(index.symbol_name(s), completion_kind(kind), None);
                    }
                }
            }
        } else {
            for s in index.visible_at(start) {
                let kind = index.symbol_kind(s);
                let detail = match kind {
                    SymbolKind::Variable | SymbolKind::Parameter | SymbolKind::Constant => {
                        doc.hir.as_ref().and_then(|m| hir_type(m, doc, index, s))
                    }
                    _ => Some(declaration_header(text, index.symbol_extent(s), kind)),
                };
                add(index.symbol_name(s), completion_kind(kind), detail);
            }
            if let Some(f) = this {
                for (alias, file, names) in &f.imports {
                    add(alias, MODULE, None);
                    let file = &doc.program[*file];
                    for (name, _) in names {
                        if let Some(s) = global_named(file, name) {
                            add(name, completion_kind(file.index.symbol_kind(s)), None);
                        }
                    }
                }
            }
            for keyword in self.keywords() {
                add(&keyword, KEYWORD, None);
            }
        }
        Json::object([
            ("isIncomplete", false.into()),
            ("items", items.into_values().collect::<Vec<Json>>().into()),
        ])
    }

    /// The words the grammar uses as keywords.
    fn keywords(&self) -> Vec<String> {
        let Some(g) = &self.grammar else {
            return Vec::new();
        };
        (0..g.kind_count())
            .filter_map(|k| g.literal_text(a0_syntax::KindId(k)))
            .filter(|t| {
                t.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && t.chars().next().is_some_and(|c| !c.is_ascii_digit())
            })
            .map(str::to_string)
            .collect()
    }

    pub(super) fn references(&self, params: &Json) -> Json {
        let Some((doc, offset)) = self.position(params) else {
            return Json::Null;
        };
        let declarations = params.get("context").get("includeDeclaration") != &Json::Bool(false);
        let Some(target) = target_at(doc, offset) else {
            return Json::Null;
        };
        let locations: Vec<Json> = occurrences(doc, &target, declarations)
            .into_iter()
            .map(|(file, start, end)| {
                let f = &doc.program[file];
                Json::object([
                    ("uri", f.uri.as_str().into()),
                    ("range", f.lines.range(&f.text, start, end)),
                ])
            })
            .collect();
        locations.into()
    }

    pub(super) fn prepare_rename(&self, params: &Json) -> Result<Json, String> {
        let (doc, offset) = self.position(params).ok_or("the document is not open")?;
        let target = target_at(doc, offset).ok_or("there is nothing to rename here")?;
        check_unambiguous(doc, &target)?;
        let (file, t) = token_of(doc, offset).ok_or("there is nothing to rename here")?;
        let f = &doc.program[file];
        let token = f.index.token(t);
        Ok(Json::object([
            ("range", f.lines.range(&f.text, token.start, token.end)),
            ("placeholder", token_text(f, t).into()),
        ]))
    }

    pub(super) fn rename(&self, params: &Json) -> Result<Json, String> {
        let (doc, offset) = self.position(params).ok_or("the document is not open")?;
        let new_name = params.get("newName").as_str().ok_or("no new name")?;
        if !doc.loaded {
            return Err(
                "the program has syntax errors; fix them to rename across its files".into(),
            );
        }
        let target = target_at(doc, offset).ok_or("there is nothing to rename here")?;
        check_unambiguous(doc, &target)?;
        let (file, t) = token_of(doc, offset).ok_or("there is nothing to rename here")?;
        if !self.is_name(token_text(&doc.program[file], t), new_name) {
            return Err(format!("`{new_name}` is not a name in {}", self.lang.name));
        }
        let mut changes: BTreeMap<usize, Vec<Json>> = BTreeMap::new();
        for (file, start, end) in occurrences(doc, &target, true) {
            let f = &doc.program[file];
            changes.entry(file).or_default().push(Json::object([
                ("range", f.lines.range(&f.text, start, end)),
                ("newText", new_name.into()),
            ]));
        }
        let changes: Vec<(String, Json)> = changes
            .into_iter()
            .map(|(file, edits)| (doc.program[file].uri.clone(), edits.into()))
            .collect();
        Ok(Json::object([("changes", Json::Object(changes))]))
    }

    /// Whether `new` lexes as one token of the same kind as `old` (a name, not a keyword).
    fn is_name(&self, old: &str, new: &str) -> bool {
        let Some(g) = &self.grammar else {
            return false;
        };
        let kinds = |text: &str| -> Vec<a0_syntax::KindId> {
            g.lex(text, SourceId(u32::MAX))
                .0
                .iter()
                .filter(|t| t.kind != a0_syntax::KindId::EOF)
                .map(|t| t.kind)
                .collect()
        };
        let (old, new) = (kinds(old), kinds(new));
        old.len() == 1 && new == old
    }
}

/// The file of the program the document is, and the token at `offset` in it.
fn token_of(doc: &Document, offset: u32) -> Option<(usize, u32)> {
    let file = doc.program.iter().position(|f| f.source == doc.source)?;
    let f = &doc.program[file];
    let t = f.index.token_at(offset)?;
    (f.index.token(t).class == TokenClass::Name).then_some((file, t))
}

/// What the name at `offset` stands for.
fn target_at(doc: &Document, offset: u32) -> Option<Target> {
    let (file, t) = token_of(doc, offset)?;
    let f = &doc.program[file];
    let targets = f.index.targets(t);
    if let [s] = targets
        && f.index.symbol_is_global(*s)
        && !f.index.symbol_kind(*s).is_member()
    {
        return Some(Target::Global(file, *s));
    }
    if !targets.is_empty() {
        return Some(Target::Local(file, targets.to_vec()));
    }
    // Another file's item: `alias.name`, or a name the file imports unqualified.
    let name = token_text(f, t);
    let token = f.index.token(t);
    let imported = if token.member {
        let alias = qualifier(f, t)?;
        f.imports
            .iter()
            .find(|(a, ..)| a == alias)
            .map(|&(_, file, _)| file)
    } else {
        f.imports
            .iter()
            .find(|(_, _, names)| names.iter().any(|(n, _)| n == name))
            .map(|&(_, file, _)| file)
    }?;
    let s = global_named(&doc.program[imported], name)?;
    Some(Target::Global(imported, s))
}

/// Rename refuses a member name that the indexes cannot tell from another type's member.
fn check_unambiguous(doc: &Document, target: &Target) -> Result<(), String> {
    match target {
        Target::Local(file, symbols) => {
            let f = &doc.program[*file];
            if symbols.len() > 1 {
                return Err(format!(
                    "several declarations are called `{}`; rename it where it is declared",
                    f.index.symbol_name(symbols[0])
                ));
            }
            let s = symbols[0];
            if f.index.symbol_kind(s).is_member() {
                let shared = (0..f.index.token_count())
                    .any(|t| f.index.targets(t).len() > 1 && f.index.targets(t).contains(&s));
                if shared {
                    return Err(format!(
                        "other types have a member called `{}` too, and its uses cannot be told apart",
                        f.index.symbol_name(s)
                    ));
                }
            }
            Ok(())
        }
        Target::Global(..) => Ok(()),
    }
}

/// Where a target is named: `(file, start, end)` for each occurrence, its declarations included if
/// `declarations`.
fn occurrences(doc: &Document, target: &Target, declarations: bool) -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    let in_file = |file: usize, symbols: &[u32], out: &mut Vec<(usize, u32, u32)>| {
        let f = &doc.program[file];
        for t in 0..f.index.token_count() {
            let token = f.index.token(t);
            if !f.index.targets(t).iter().any(|s| symbols.contains(s)) {
                continue;
            }
            if token.declares.is_some() && !declarations {
                continue;
            }
            out.push((file, token.start, token.end));
        }
    };
    match target {
        Target::Local(file, symbols) => in_file(*file, symbols, &mut out),
        Target::Global(defining, s) => {
            in_file(*defining, &[*s], &mut out);
            let name = doc.program[*defining].index.symbol_name(*s);
            for (g, file) in doc.program.iter().enumerate() {
                for (alias, imported, names) in &file.imports {
                    if imported != defining {
                        continue;
                    }
                    let unqualified = names.iter().filter(|(n, _)| n == name);
                    for (_, span) in unqualified.clone() {
                        out.push((g, span.start, span.end));
                    }
                    let unqualified = unqualified.count() > 0;
                    for t in 0..file.index.token_count() {
                        let token = file.index.token(t);
                        if token.class != TokenClass::Name || token_text(file, t) != name {
                            continue;
                        }
                        let found = if token.member {
                            qualifier(file, t) == Some(alias)
                        } else {
                            unqualified && file.index.targets(t).is_empty()
                        };
                        if found {
                            out.push((g, token.start, token.end));
                        }
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The HIR type of a local or parameter symbol, as hovers find it.
fn local_type(m: &HirModule, doc: &Document, index: &SyntaxIndex, s: u32) -> Option<TypeId> {
    let span = index.symbol_span(s);
    let covers = |outer: Option<Span>| {
        outer.filter(|o| o.source == doc.source && o.start <= span.start && span.end <= o.end)
    };
    let name = index.symbol_name(s);
    (0..m.local_count())
        .map(a0_hir::LocalId)
        .filter_map(|l| {
            let outer = covers(m.local_span(l))?;
            (m.local_source_name(l) == name).then_some((outer.end - outer.start, l))
        })
        .min_by_key(|&(size, _)| size)
        .map(|(_, l)| m.local_type(l))
}
