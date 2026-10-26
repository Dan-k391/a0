//! Programs of several files, for languages with modules ([`crate::Modules`]).
//!
//! The root file's imports (found with the grammar's `@import` annotations) are loaded, theirs in
//! turn, and every file is compiled in dependency order into one HIR builder, each into a module of
//! its own: the root file's names stay as they are, and another file's are `module.item`. A module
//! name is resolved from its package's source directory (`text.json` is `text/json.<ext>` there;
//! without a package, the root file's directory), or names a dependency (`json` is the package
//! `json`'s library, `json.parser` a module of it, `package.rs`); a string path is resolved from
//! the importing file's directory. Imports may not form a cycle. Modules named `std` or `std.*`
//! are the standard library's (`a0_hir::std_modules`), never files.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use a0_diag::codes as code;
use a0_diag::{Diagnostic, Diagnostics, SourceId, SourceMap};
use a0_hir::{HirBuilder, HirModule};
use a0_syntax::{Grammar, Import, SyntaxTree};

use crate::Modules;
use crate::package::{Package, Packages};

/// One file of a program.
pub struct ProgramFile {
    /// Absolute, without `.` and `..`.
    pub path: PathBuf,
    pub source: SourceId,
    /// The module's name: the path from the root file's directory, with `.` between segments.
    pub module: String,
    pub tree: SyntaxTree,
    /// Its imports, each with the index of the file it imports.
    pub imports: Vec<(Import, usize)>,
    /// Its imports of the standard library's modules.
    pub library: Vec<Import>,
}

/// Syntax trees kept between loads of programs (the language server's, D82): a file whose text has
/// not changed since it was last parsed is not parsed again.
#[derive(Default)]
pub struct TreeCache {
    files: HashMap<PathBuf, (String, SyntaxTree)>,
    hits: usize,
}

impl TreeCache {
    /// How many times a file's tree was reused.
    pub fn hits(&self) -> usize {
        self.hits
    }
}

/// The files of a program, in the order they are compiled: every file after the files it imports,
/// so the root file last.
pub struct Program {
    pub files: Vec<ProgramFile>,
}

/// `path` made absolute, with `.` and `..` resolved without looking at the file system (so files
/// that only exist in an editor resolve too).
pub fn normalize(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for c in absolute.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Loads the program whose root file is `root` (as written by the user, which is how diagnostics
/// name it; other files are named relative to it), in the root package of `packages` if given.
/// `read` gives a file's text, or `None` if it cannot be read. Syntax errors in any file, missing
/// modules and import cycles are reported.
pub fn load(
    grammar: &Grammar,
    modules: &Modules,
    root: &str,
    packages: Option<&Packages>,
    read: &mut dyn FnMut(&Path) -> Option<String>,
    sources: &mut SourceMap,
) -> Result<Program, Diagnostics> {
    load_cached(grammar, modules, root, packages, read, sources, None)
}

/// [`load`], taking unchanged files' trees from `cache` and keeping new ones there.
pub fn load_cached(
    grammar: &Grammar,
    modules: &Modules,
    root: &str,
    packages: Option<&Packages>,
    read: &mut dyn FnMut(&Path) -> Option<String>,
    sources: &mut SourceMap,
    mut cache: Option<&mut TreeCache>,
) -> Result<Program, Diagnostics> {
    let root_path = normalize(Path::new(root));
    let root_dir = root_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let shown_dir = Path::new(root).parent().map(display).unwrap_or_default();
    // Without a package, the root file's directory is the one package's.
    let alone;
    let packages = match packages {
        Some(p) => p,
        None => {
            alone = Packages {
                list: vec![Package {
                    prefix: String::new(),
                    src: root_dir.clone(),
                    shown: shown_dir.clone(),
                    lib: None,
                    deps: Vec::new(),
                }],
                ..Packages::default()
            };
            &alone
        }
    };
    let mut diags = Diagnostics::new();
    let Some(text) = read(&root_path) else {
        diags.error("io", &format!("cannot read {root}"), None);
        return Err(diags);
    };
    // Every file found so far, by index: its path, its name in diagnostics, its text, its package
    // and its module's name.
    let root_module = module_name(&packages.list[0], &root_path, 0);
    let mut found: Vec<(PathBuf, String, String, usize, String)> =
        vec![(root_path, root.to_string(), text, 0, root_module)];
    let mut by_path: HashMap<PathBuf, usize> = HashMap::new();
    by_path.insert(found[0].0.clone(), 0);
    let mut files: Vec<ProgramFile> = Vec::new();
    while files.len() < found.len() {
        let index = files.len();
        let (path, shown, text, package, module) = found[index].clone();
        let source = sources.add(&shown, &text);
        let cached = cache
            .as_deref()
            .and_then(|c| c.files.get(&path))
            .filter(|(old, _)| *old == text)
            .map(|(_, tree)| tree.clone());
        let tree = match cached {
            Some(mut tree) => {
                tree.set_source(source);
                if let Some(c) = cache.as_deref_mut() {
                    c.hits += 1;
                }
                tree
            }
            None => {
                let (tree, parse) = grammar.parse(&text, source);
                // Only trees without errors are kept: their diagnostics need not be.
                if !parse.has_errors()
                    && let Some(c) = cache.as_deref_mut()
                {
                    c.files.insert(path.clone(), (text.clone(), tree.clone()));
                }
                diags.extend(parse);
                tree
            }
        };
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let shown_base = Path::new(&shown)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut imports = Vec::new();
        let mut library = Vec::new();
        for import in a0_syntax::imports(grammar, &tree) {
            if !import.literal && a0_hir::is_std_module(&import.module) {
                library.push(import);
                continue;
            }
            if !import.literal && import.module.starts_with("std.") {
                let modules = a0_hir::std_modules().join(", ");
                diags.push(
                    Diagnostic::error(
                        code::UNKNOWN_MODULE,
                        &format!("the standard library has no module `{}`", import.module),
                    )
                    .with_span(import.path_span)
                    .with_primary_label("not a module of the standard library")
                    .with_note(&format!("its modules are {modules}")),
                );
                continue;
            }
            let (target, shown_target, target_package) = if import.literal {
                let rel = Path::new(&import.file);
                (
                    normalize(&dir.join(rel)),
                    display(&shown_base.join(rel)),
                    package,
                )
            } else {
                match resolve(packages, package, &import.module, modules.extension) {
                    Ok(found) => found,
                    Err(why) => {
                        diags.push(
                            Diagnostic::error(
                                code::UNKNOWN_MODULE,
                                &format!("cannot find module `{}`", import.module),
                            )
                            .with_span(import.path_span)
                            .with_primary_label("no file for this module")
                            .with_note(&why),
                        );
                        continue;
                    }
                }
            };
            if let Some(&known) = by_path.get(&target) {
                imports.push((import, known));
                continue;
            }
            match read(&target) {
                Some(text) => {
                    let name = module_name(&packages.list[target_package], &target, found.len());
                    by_path.insert(target.clone(), found.len());
                    imports.push((import, found.len()));
                    found.push((target, shown_target, text, target_package, name));
                }
                None => diags.push(
                    Diagnostic::error(
                        code::UNKNOWN_MODULE,
                        &format!("cannot find module `{}`", import.module),
                    )
                    .with_span(import.path_span)
                    .with_primary_label("no file for this module")
                    .with_note(&format!("looked for {shown_target}")),
                ),
            }
        }
        files.push(ProgramFile {
            path,
            source,
            module,
            tree,
            imports,
            library,
        });
    }
    if diags.has_errors() {
        return Err(diags);
    }
    let order = dependency_order(&files)?;
    let new_index: HashMap<usize, usize> = order
        .iter()
        .enumerate()
        .map(|(new, &old)| (old, new))
        .collect();
    let mut slots: Vec<Option<ProgramFile>> = files.into_iter().map(Some).collect();
    let mut sorted = Vec::new();
    for old in order {
        let mut f = slots[old].take().unwrap();
        for (_, target) in &mut f.imports {
            *target = new_index[target];
        }
        sorted.push(f);
    }
    Ok(Program { files: sorted })
}

/// Where the module `module` imported by a file of package `from` is: its file, how diagnostics
/// name it, and its package. Its first name may be a dependency's.
fn resolve(
    packages: &Packages,
    from: usize,
    module: &str,
    extension: &str,
) -> Result<(PathBuf, String, usize), String> {
    let segments: Vec<&str> = module.split('.').collect();
    let package = &packages.list[from];
    if let Some(&(_, dep)) = package.deps.iter().find(|(name, _)| name == segments[0]) {
        let target = &packages.list[dep];
        if segments.len() == 1 {
            let Some(lib) = &target.lib else {
                return Err(format!(
                    "the package `{}` has no library file (lib.{extension})",
                    segments[0]
                ));
            };
            let rel = lib
                .strip_prefix(&target.src)
                .map(display)
                .unwrap_or_default();
            return Ok((lib.clone(), join(&target.shown, &rel), dep));
        }
        let rel = format!("{}.{extension}", segments[1..].join("/"));
        return Ok((
            normalize(&target.src.join(&rel)),
            join(&target.shown, &rel),
            dep,
        ));
    }
    let rel = format!("{}.{extension}", segments.join("/"));
    Ok((
        normalize(&package.src.join(&rel)),
        join(&package.shown, &rel),
        from,
    ))
}

fn join(dir: &str, rel: &str) -> String {
    if dir.is_empty() {
        rel.to_string()
    } else {
        format!("{dir}/{rel}")
    }
}

/// The name of the module in `path`, of `package`: its path from the package's directory without
/// the extension, segments joined with `.`, after the package's own name (its library file is the
/// package's name alone); a file outside the directory is named after its stem, made unique.
fn module_name(package: &Package, path: &Path, index: usize) -> String {
    if package.lib.as_deref() == Some(path) && !package.prefix.is_empty() {
        return package.prefix.clone();
    }
    let stem = path.with_extension("");
    let local = match stem.strip_prefix(&package.src) {
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("."),
        Err(_) => {
            return format!(
                "{}#{index}",
                stem.file_name().unwrap_or_default().to_string_lossy()
            );
        }
    };
    if package.prefix.is_empty() {
        local
    } else {
        format!("{}.{local}", package.prefix)
    }
}

/// A path as diagnostics show it: with `/` on every system.
fn display(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The files in compile order (each after what it imports), or the import cycle.
fn dependency_order(files: &[ProgramFile]) -> Result<Vec<usize>, Diagnostics> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        New,
        Open,
        Done,
    }
    let mut state = vec![State::New; files.len()];
    let mut order = Vec::new();
    // (file, next import to look at)
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    state[0] = State::Open;
    while let Some(&mut (f, ref mut next)) = stack.last_mut() {
        let Some((import, target)) = files[f].imports.get(*next) else {
            state[f] = State::Done;
            order.push(f);
            stack.pop();
            continue;
        };
        *next += 1;
        let target = *target;
        match state[target] {
            State::Done => {}
            State::New => {
                state[target] = State::Open;
                stack.push((target, 0));
            }
            State::Open => {
                // The files on the stack from `target` up import each other in a circle.
                let start = stack.iter().position(|&(g, _)| g == target).unwrap();
                let chain: Vec<&str> = stack[start..]
                    .iter()
                    .map(|&(g, _)| files[g].module.as_str())
                    .chain([files[target].module.as_str()])
                    .collect();
                let mut d = Diagnostic::error(
                    code::IMPORT_CYCLE,
                    &format!("import cycle: {}", chain.join(" -> ")),
                )
                .with_span(import.path_span)
                .with_primary_label("this import closes the cycle");
                for &(g, i) in &stack[start..stack.len() - 1] {
                    let (other, _) = &files[g].imports[i - 1];
                    d = d.with_label(other.path_span, "imported here");
                }
                return Err(d.into());
            }
        }
    }
    Ok(order)
}

/// Compiles a loaded program into one HIR module.
pub fn compile(program: &Program, modules: &Modules, name: &str) -> Result<HirModule, Diagnostics> {
    let (m, diags) = analyze(program, modules, name);
    if diags.has_errors() {
        Err(diags)
    } else {
        Ok(m)
    }
}

/// Compiles a loaded program, keeping the module even when it has errors (for the language
/// server).
pub fn analyze(program: &Program, modules: &Modules, name: &str) -> (HirModule, Diagnostics) {
    let mut b = HirBuilder::new(name);
    (modules.start)(&mut b);
    let root = program.files.len() - 1;
    for (i, file) in program.files.iter().enumerate() {
        b.set_span(None);
        b.begin_module(&file.module, i == root);
        for import in &file.library {
            b.set_span(Some(import.span));
            b.import_module(&import.module, &import.alias);
            for (item, span) in &import.names {
                b.set_span(Some(*span));
                b.import_item(&import.module, item);
            }
        }
        for (import, target) in &file.imports {
            let module = &program.files[*target].module;
            b.set_span(Some(import.span));
            b.import_module(module, &import.alias);
            for (item, span) in &import.names {
                b.set_span(Some(*span));
                b.import_item(module, item);
            }
        }
        b.set_public(true);
        (modules.compile)(&mut b, &file.tree);
    }
    b.finish_keeping()
}
