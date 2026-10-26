//! Packages (D81): a program's manifest, its dependencies, and the lock file that pins them.
//!
//! A package is a directory with a manifest named after the language (`minnow.toml`):
//!
//! ```toml
//! [package]
//! name = "app"
//! version = "0.1.0"
//! main = "src/main.mn"     # the program (default: src/main.<ext>, or main.<ext>)
//! lib = "src/lib.mn"       # what `import app` gives other packages (default: src/lib.<ext>)
//!
//! [dependencies]
//! json = { path = "../json" }
//! http = { git = "https://example.com/http.git", tag = "v1.2" }
//! ```
//!
//! A file imports a dependency's library as `import json` and its other modules as
//! `import json.parser` (a path from the directory of its library file). Git dependencies are
//! fetched with the `git` command into a cache (`A0_HOME`, by default `~/.a0`) and pinned in the
//! lock file beside the manifest (`minnow.lock`), so that a build is the same wherever and
//! whenever it runs: the same sources give the same executable, byte for byte.

// Reading a manifest stops at its first error, so the size of the error does not matter.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use a0_diag::codes as code;
use a0_diag::{Diagnostic, Diagnostics, SourceId, SourceMap, Span};

use crate::program::normalize;

// ------------------------------------------------------------------------------------------------
// A TOML subset: what manifests and lock files use.

/// A TOML value with where it is.
#[derive(Clone, Debug)]
pub(crate) struct Value {
    pub kind: ValueKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum ValueKind {
    Str(String),
    Table(Vec<Entry>),
    /// A number, a boolean or an array, which manifests do not use: what it is, for messages.
    Other(&'static str),
}

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub key: String,
    pub key_span: Span,
    pub value: Value,
}

/// A `[table]` of a document; the entries before the first header are the table named "".
#[derive(Clone, Debug)]
pub(crate) struct Section {
    pub name: String,
    pub span: Span,
    pub entries: Vec<Entry>,
}

struct Toml<'a> {
    text: &'a str,
    pos: usize,
    source: SourceId,
}

type Parsed<T> = Result<T, Diagnostic>;

impl Toml<'_> {
    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.source, start as u32, end as u32)
    }

    fn error(&self, start: usize, message: &str, label: &str) -> Diagnostic {
        let end = (start + 1).min(self.text.len()).max(start);
        Diagnostic::error(code::MANIFEST_SYNTAX, message)
            .with_span(self.span(start, end))
            .with_primary_label(label)
    }

    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    /// Skips spaces, tabs and comments, and with `newlines` line ends too.
    fn skip(&mut self, newlines: bool) {
        while let Some(c) = self.peek() {
            match c {
                ' ' | '\t' => self.pos += 1,
                '\r' | '\n' if newlines => self.pos += 1,
                '#' => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.pos += self.peek().unwrap().len_utf8();
                    }
                }
                _ => break,
            }
        }
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn document(&mut self) -> Parsed<Vec<Section>> {
        let mut sections = vec![Section {
            name: String::new(),
            span: self.span(0, 0),
            entries: Vec::new(),
        }];
        loop {
            self.skip(true);
            let Some(c) = self.peek() else {
                return Ok(sections);
            };
            if c == '[' {
                let start = self.pos;
                self.pos += 1;
                self.skip(false);
                let (name, _) = self.key()?;
                self.skip(false);
                if !self.eat(']') {
                    return Err(self.error(self.pos, "expected `]`", "the table's name ends here"));
                }
                if sections.iter().any(|s| s.name == name) {
                    return Err(Diagnostic::error(
                        code::MANIFEST_SYNTAX,
                        &format!("the table `[{name}]` appears twice"),
                    )
                    .with_span(self.span(start, self.pos))
                    .with_primary_label("again here"));
                }
                sections.push(Section {
                    name,
                    span: self.span(start, self.pos),
                    entries: Vec::new(),
                });
            } else {
                let entry = self.entry()?;
                let entries = &mut sections.last_mut().unwrap().entries;
                if let Some(first) = entries.iter().find(|e| e.key == entry.key) {
                    return Err(Diagnostic::error(
                        code::MANIFEST_SYNTAX,
                        &format!("the key `{}` appears twice", entry.key),
                    )
                    .with_span(entry.key_span)
                    .with_primary_label("again here")
                    .with_label(first.key_span, "first here"));
                }
                entries.push(entry);
            }
            self.skip(false);
            if self.peek().is_some_and(|c| c != '\n' && c != '\r') {
                return Err(self.error(
                    self.pos,
                    "expected the end of the line",
                    "one entry per line",
                ));
            }
        }
    }

    fn key(&mut self) -> Parsed<(String, Span)> {
        let start = self.pos;
        if self.peek() == Some('"') {
            let value = self.string()?;
            let ValueKind::Str(s) = value.kind else {
                unreachable!()
            };
            return Ok((s, value.span));
        }
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.error(start, "expected a key", "a name or a quoted string"));
        }
        Ok((
            self.text[start..self.pos].to_string(),
            self.span(start, self.pos),
        ))
    }

    fn entry(&mut self) -> Parsed<Entry> {
        let (key, key_span) = self.key()?;
        self.skip(false);
        if !self.eat('=') {
            return Err(self.error(self.pos, "expected `=` after the key", "expected `=`"));
        }
        self.skip(false);
        let value = self.value()?;
        Ok(Entry {
            key,
            key_span,
            value,
        })
    }

    fn string(&mut self) -> Parsed<Value> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error(start, "this string is never closed", "starts here"));
            };
            self.pos += c.len_utf8();
            match c {
                '"' => break,
                '\n' => {
                    return Err(self.error(start, "this string is never closed", "starts here"));
                }
                '\\' => {
                    let at = self.pos - 1;
                    let e = self
                        .peek()
                        .ok_or_else(|| self.error(at, "unknown escape", ""))?;
                    self.pos += e.len_utf8();
                    out.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '"' => '"',
                        '\\' => '\\',
                        _ => {
                            return Err(self.error(
                                at,
                                &format!("unknown escape `\\{e}`"),
                                "not an escape",
                            ));
                        }
                    });
                }
                c => out.push(c),
            }
        }
        Ok(Value {
            kind: ValueKind::Str(out),
            span: self.span(start, self.pos),
        })
    }

    fn value(&mut self) -> Parsed<Value> {
        let start = self.pos;
        match self.peek() {
            Some('"') => self.string(),
            Some('[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip(true);
                    if self.eat(']') {
                        break;
                    }
                    items.push(self.value()?);
                    self.skip(true);
                    if !self.eat(',') {
                        self.skip(true);
                        if !self.eat(']') {
                            return Err(self.error(self.pos, "expected `,` or `]`", "here"));
                        }
                        break;
                    }
                }
                let _ = items;
                Ok(Value {
                    kind: ValueKind::Other("an array"),
                    span: self.span(start, self.pos),
                })
            }
            Some('{') => {
                self.pos += 1;
                let mut entries: Vec<Entry> = Vec::new();
                self.skip(false);
                if !self.eat('}') {
                    loop {
                        self.skip(false);
                        let entry = self.entry()?;
                        if entries.iter().any(|e| e.key == entry.key) {
                            return Err(Diagnostic::error(
                                code::MANIFEST_SYNTAX,
                                &format!("the key `{}` appears twice", entry.key),
                            )
                            .with_span(entry.key_span)
                            .with_primary_label("again here"));
                        }
                        entries.push(entry);
                        self.skip(false);
                        if self.eat('}') {
                            break;
                        }
                        if !self.eat(',') {
                            return Err(self.error(self.pos, "expected `,` or `}`", "here"));
                        }
                    }
                }
                Ok(Value {
                    kind: ValueKind::Table(entries),
                    span: self.span(start, self.pos),
                })
            }
            Some(c) if c.is_ascii_digit() || c == '-' || c == '+' => {
                self.pos += 1;
                while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '_') {
                    self.pos += 1;
                }
                let digits = self.text[start..self.pos].replace('_', "");
                digits.parse::<i64>().map_err(|_| {
                    self.error(start, "this number is not an integer A0 reads", "here")
                })?;
                Ok(Value {
                    kind: ValueKind::Other("a number"),
                    span: self.span(start, self.pos),
                })
            }
            _ => {
                for word in ["true", "false"] {
                    if self.text[self.pos..].starts_with(word) {
                        self.pos += word.len();
                        return Ok(Value {
                            kind: ValueKind::Other("a boolean"),
                            span: self.span(start, self.pos),
                        });
                    }
                }
                Err(self.error(
                    start,
                    "expected a value",
                    "a string in double quotes, a number, a boolean, an array or a table",
                ))
            }
        }
    }
}

/// Parses a TOML document of the subset manifests use.
pub(crate) fn parse_toml(text: &str, source: SourceId) -> Result<Vec<Section>, Diagnostic> {
    Toml {
        text,
        pos: 0,
        source,
    }
    .document()
}

fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ------------------------------------------------------------------------------------------------
// Manifests.

/// Where a dependency comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A directory, relative to the manifest's.
    Path(String),
    /// A git repository and what to check out of it.
    Git { url: String, reference: GitRef },
}

/// What of a git repository a dependency wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GitRef {
    /// The repository's default branch.
    Default,
    Branch(String),
    Tag(String),
    Rev(String),
}

impl GitRef {
    /// How the lock file writes it after the URL.
    fn suffix(&self) -> String {
        match self {
            GitRef::Default => String::new(),
            GitRef::Branch(b) => format!("#branch={b}"),
            GitRef::Tag(t) => format!("#tag={t}"),
            GitRef::Rev(r) => format!("#rev={r}"),
        }
    }

    /// What `git rev-parse` resolves.
    fn spec(&self) -> String {
        match self {
            GitRef::Default => "HEAD".to_string(),
            GitRef::Branch(b) => format!("refs/heads/{b}"),
            GitRef::Tag(t) => format!("refs/tags/{t}"),
            GitRef::Rev(r) => r.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Dependency {
    pub name: String,
    pub span: Span,
    pub origin: Origin,
}

/// A package's manifest.
#[derive(Clone, Debug)]
pub(crate) struct Manifest {
    pub name: String,
    pub main: Option<(String, Span)>,
    pub lib: Option<(String, Span)>,
    pub dependencies: Vec<Dependency>,
}

fn manifest_error(message: &str, span: Span, label: &str) -> Diagnostic {
    Diagnostic::error(code::MANIFEST_KEY, message)
        .with_span(span)
        .with_primary_label(label)
}

fn string_of(e: &Entry) -> Result<String, Diagnostic> {
    let found = match &e.value.kind {
        ValueKind::Str(s) => return Ok(s.clone()),
        ValueKind::Table(_) => "a table",
        ValueKind::Other(what) => what,
    };
    Err(manifest_error(
        &format!("`{}` must be a string, not {found}", e.key),
        e.value.span,
        "expected a string in double quotes",
    ))
}

/// Reads a manifest whose text was added to `sources` as `source`.
pub(crate) fn parse_manifest(
    text: &str,
    source: SourceId,
    dir: &Path,
) -> Result<Manifest, Diagnostics> {
    let sections = parse_toml(text, source)?;
    let mut diags = Diagnostics::new();
    let mut manifest = Manifest {
        name: String::new(),
        main: None,
        lib: None,
        dependencies: Vec::new(),
    };
    let mut has_package = false;
    for section in &sections {
        match section.name.as_str() {
            "" => {
                if let Some(e) = section.entries.first() {
                    diags.push(
                        manifest_error(
                            &format!("`{}` is outside any table", e.key),
                            e.key_span,
                            "not in a table",
                        )
                        .with_help("put it under `[package]` or `[dependencies]`"),
                    );
                }
            }
            "package" => {
                has_package = true;
                for e in &section.entries {
                    let result = match e.key.as_str() {
                        "name" => string_of(e).map(|s| manifest.name = s),
                        "version" => string_of(e).map(|_| ()),
                        "main" => string_of(e).map(|s| manifest.main = Some((s, e.value.span))),
                        "lib" => string_of(e).map(|s| manifest.lib = Some((s, e.value.span))),
                        _ => Err(manifest_error(
                            &format!("unknown key `{}` in `[package]`", e.key),
                            e.key_span,
                            "not a key of `[package]`",
                        )
                        .with_note("`[package]` takes `name`, `version`, `main` and `lib`")),
                    };
                    if let Err(d) = result {
                        diags.push(d);
                    }
                }
                if manifest.name.is_empty() && !diags.has_errors() {
                    diags.push(
                        manifest_error(
                            "the package has no name",
                            section.span,
                            "this table needs `name = \"...\"`",
                        )
                        .with_fix(
                            "name the package",
                            Span::new(source, section.span.end, section.span.end),
                            &format!(
                                "\nname = {}",
                                quoted(
                                    &dir.file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| "app".to_string())
                                )
                            ),
                        ),
                    );
                }
            }
            "dependencies" => {
                for e in &section.entries {
                    match dependency(e) {
                        Ok(d) => manifest.dependencies.push(d),
                        Err(d) => diags.push(d),
                    }
                }
            }
            other => diags.push(
                manifest_error(
                    &format!("unknown table `[{other}]`"),
                    section.span,
                    "not a table of the manifest",
                )
                .with_note("a manifest has `[package]` and `[dependencies]`"),
            ),
        }
    }
    if !has_package && !diags.has_errors() {
        diags.push(
            Diagnostic::error(code::MANIFEST_KEY, "the manifest has no `[package]` table")
                .with_span(Span::new(source, 0, 0))
                .with_primary_label("expected `[package]` with the package's name"),
        );
    }
    if diags.has_errors() {
        Err(diags)
    } else {
        Ok(manifest)
    }
}

fn dependency(e: &Entry) -> Result<Dependency, Diagnostic> {
    let ValueKind::Table(entries) = &e.value.kind else {
        return Err(manifest_error(
            &format!(
                "the dependency `{}` says neither a path nor a git repository",
                e.key
            ),
            e.value.span,
            "expected `{ path = \"...\" }` or `{ git = \"...\" }`",
        )
        .with_note(
            "dependencies come from a directory or a git repository; there is no registry",
        ));
    };
    let mut path = None;
    let mut git = None;
    let mut reference = GitRef::Default;
    let mut reference_key: Option<&Entry> = None;
    for item in entries {
        let s = string_of(item)?;
        match item.key.as_str() {
            "path" => path = Some(s),
            "git" => git = Some(s),
            "branch" | "tag" | "rev" => {
                if let Some(first) = reference_key {
                    return Err(manifest_error(
                        "a git dependency takes one of `branch`, `tag` and `rev`",
                        item.key_span,
                        "a second one",
                    )
                    .with_label(first.key_span, "the first"));
                }
                reference_key = Some(item);
                reference = match item.key.as_str() {
                    "branch" => GitRef::Branch(s),
                    "tag" => GitRef::Tag(s),
                    _ => GitRef::Rev(s),
                };
            }
            other => {
                return Err(manifest_error(
                    &format!("unknown key `{other}` in a dependency"),
                    item.key_span,
                    "not a key of a dependency",
                )
                .with_note("a dependency takes `path`, or `git` with `branch`, `tag` or `rev`"));
            }
        }
    }
    let origin = match (path, git) {
        (Some(p), None) => {
            if let Some(r) = reference_key {
                return Err(manifest_error(
                    &format!("`{}` is for git dependencies", r.key),
                    r.key_span,
                    "this dependency is a path",
                ));
            }
            Origin::Path(p)
        }
        (None, Some(url)) => Origin::Git { url, reference },
        (Some(_), Some(_)) => {
            return Err(manifest_error(
                &format!(
                    "the dependency `{}` has both a path and a git repository",
                    e.key
                ),
                e.value.span,
                "give one of them",
            ));
        }
        (None, None) => {
            return Err(manifest_error(
                &format!(
                    "the dependency `{}` says neither a path nor a git repository",
                    e.key
                ),
                e.value.span,
                "expected `path = \"...\"` or `git = \"...\"`",
            ));
        }
    };
    Ok(Dependency {
        name: e.key.clone(),
        span: e.key_span,
        origin,
    })
}

/// The manifest of the package `file` (a file or a directory) is in: the nearest
/// `<language>.toml` in it or its parents.
pub fn find_manifest(file: &Path, language: &str) -> Option<PathBuf> {
    let start = normalize(file);
    let mut dir = if start.is_dir() {
        Some(start.as_path())
    } else {
        start.parent()
    };
    while let Some(d) = dir {
        let candidate = d.join(format!("{language}.toml"));
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

// ------------------------------------------------------------------------------------------------
// Lock files.

/// The commits git dependencies are pinned to, by `url#reference`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Lock {
    pub commits: std::collections::BTreeMap<String, String>,
}

impl Lock {
    fn read(path: &Path, sources: &mut SourceMap) -> Result<Lock, Diagnostics> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(Lock::default());
        };
        let source = sources.add(&path.to_string_lossy(), &text);
        let mut lock = Lock::default();
        for section in parse_toml(&text, source)? {
            if section.name != "git" {
                continue;
            }
            for e in section.entries {
                if let ValueKind::Str(commit) = e.value.kind {
                    lock.commits.insert(e.key, commit);
                }
            }
        }
        Ok(lock)
    }

    fn text(&self, language: &str) -> String {
        let mut out = format!(
            "# Written by `{language} build`: the commits git dependencies are pinned to. Commit it.\n"
        );
        if !self.commits.is_empty() {
            out.push_str("\n[git]\n");
            for (key, commit) in &self.commits {
                out.push_str(&format!("{} = {}\n", quoted(key), quoted(commit)));
            }
        }
        out
    }
}

// ------------------------------------------------------------------------------------------------
// Git.

/// Where fetched repositories are kept: `A0_HOME`, or `.a0` in the home directory.
fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("A0_HOME") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".a0")
}

/// A short, stable name for a URL (FNV-1a), for directories in the cache.
fn url_key(url: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let name: String = url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or("repo")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    format!("{name}-{h:016x}")
}

fn git(args: &[&str], dir: Option<&Path>) -> Result<String, String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let out = cmd
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The commit `reference` names in the repository at `url`, fetching it into the cache if
/// `fetch` (or if `locked` gives one, only when the cache lacks it).
fn resolve_git(
    url: &str,
    reference: &GitRef,
    locked: Option<&str>,
    fetch: bool,
    language: &str,
) -> Result<String, String> {
    let db = cache_dir().join("git").join("db").join(url_key(url));
    let has = |commit: &str| {
        db.exists()
            && git(
                &["cat-file", "-e", &format!("{commit}^{{commit}}")],
                Some(&db),
            )
            .is_ok()
    };
    if let Some(commit) = locked
        && has(commit)
    {
        return Ok(commit.to_string());
    }
    if !fetch {
        return Err(format!(
            "{url} is not fetched yet: `{language} build` fetches it"
        ));
    }
    eprintln!("{language}: fetching {url}");
    if db.exists() {
        git(
            &[
                "fetch",
                "--quiet",
                "--force",
                "--tags",
                "origin",
                "+refs/heads/*:refs/heads/*",
            ],
            Some(&db),
        )?;
    } else {
        std::fs::create_dir_all(db.parent().unwrap()).map_err(|e| e.to_string())?;
        git(
            &["clone", "--quiet", "--bare", url, &db.to_string_lossy()],
            None,
        )?;
    }
    if let Some(commit) = locked {
        return if has(commit) {
            Ok(commit.to_string())
        } else {
            Err(format!(
                "the repository has no commit {commit}, which the lock file pins"
            ))
        };
    }
    git(
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{}^{{commit}}", reference.spec()),
        ],
        Some(&db),
    )
    .map_err(|_| match reference {
        GitRef::Default => "the repository has no commits".to_string(),
        GitRef::Branch(b) => format!("the repository has no branch `{b}`"),
        GitRef::Tag(t) => format!("the repository has no tag `{t}`"),
        GitRef::Rev(r) => format!("the repository has no commit `{r}`"),
    })
}

/// The files of `commit`, checked out once into the cache.
fn checkout(url: &str, commit: &str) -> Result<PathBuf, String> {
    let key = url_key(url);
    let db = cache_dir().join("git").join("db").join(&key);
    let dir = cache_dir()
        .join("git")
        .join("checkouts")
        .join(&key)
        .join(&commit[..commit.len().min(12)]);
    let done = dir.join(".a0-checkout");
    if done.exists() {
        return Ok(dir);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    git(
        &[
            &format!("--git-dir={}", db.to_string_lossy()),
            &format!("--work-tree={}", dir.to_string_lossy()),
            "checkout",
            "--quiet",
            "--force",
            commit,
            "--",
            ".",
        ],
        None,
    )?;
    std::fs::write(&done, commit).map_err(|e| e.to_string())?;
    Ok(dir)
}

// ------------------------------------------------------------------------------------------------
// The packages of a program.

/// One package of a program: where its modules are and what it depends on.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Package {
    /// The name its modules have in the program (`json`, `json.parser`); empty for the root
    /// package, whose modules have their own names.
    pub prefix: String,
    /// The directory module paths start from: the main or library file's.
    pub src: PathBuf,
    /// How diagnostics and traces name that directory.
    pub shown: String,
    /// The file `import <prefix>` loads.
    pub lib: Option<PathBuf>,
    /// Its dependencies: their names in its imports, and their packages.
    pub deps: Vec<(String, usize)>,
}

/// The packages of a program, the root package first.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Packages {
    pub list: Vec<Package>,
    /// The root package's main file, if it has one.
    pub main: Option<PathBuf>,
    /// The root package's name.
    pub name: String,
    /// The root package's directory.
    pub dir: PathBuf,
}

/// How [`load`] may get dependencies.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct LoadOptions {
    /// Fetch git dependencies that are not in the cache (builds do, the language server does not).
    pub fetch: bool,
    /// Fail rather than change the lock file (`--locked`).
    pub locked: bool,
    /// Ignore the lock file and take each git reference's newest commit (`update`).
    pub update: bool,
}

/// The first of `candidates` (relative to `dir`) that is a file.
fn first_file(dir: &Path, candidates: &[String]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|c| normalize(&dir.join(c)))
        .find(|p| p.is_file())
}

/// Loads the package whose manifest is `manifest_path` and every package it depends on; fetches
/// and pins git dependencies as `options` allow, writing the lock file when it changes. Problems
/// are diagnostics against the manifests, whose texts are added to `sources`; `read` collects
/// each manifest's source and path (for editors).
pub fn load(
    manifest_path: &Path,
    language: &str,
    extension: &str,
    options: LoadOptions,
    sources: &mut SourceMap,
    read: &mut Vec<(SourceId, PathBuf)>,
) -> Result<Packages, Diagnostics> {
    let manifest_path = normalize(manifest_path);
    let root_dir = manifest_path
        .parent()
        .unwrap_or(Path::new(""))
        .to_path_buf();
    let lock_path = root_dir.join(format!("{language}.lock"));
    let old_lock = if options.update {
        Lock::default()
    } else {
        Lock::read(&lock_path, sources)?
    };
    let mut lock = Lock::default();
    let mut diags = Diagnostics::new();
    let mut packages = Packages {
        dir: root_dir.clone(),
        ..Packages::default()
    };
    // Packages by directory, and the manifests still to read: (directory, how it is shown, the
    // name its modules get, the package's index).
    let mut by_dir: HashMap<PathBuf, usize> = HashMap::new();
    let mut queue: Vec<(PathBuf, String, usize)> = Vec::new();
    by_dir.insert(root_dir.clone(), 0);
    packages.list.push(Package {
        prefix: String::new(),
        src: root_dir.clone(),
        shown: String::new(),
        lib: None,
        deps: Vec::new(),
    });
    queue.push((root_dir.clone(), String::new(), 0));
    let mut prefixes: HashMap<String, usize> = HashMap::new();
    let mut next = 0;
    while next < queue.len() {
        let (dir, shown, index) = queue[next].clone();
        next += 1;
        let path = dir.join(format!("{language}.toml"));
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                diags.error("io", &format!("cannot read {}: {e}", path.display()), None);
                continue;
            }
        };
        let shown_manifest = if shown.is_empty() {
            format!("{language}.toml")
        } else {
            format!("{shown}/{language}.toml")
        };
        let source = sources.add(&shown_manifest, &text);
        read.push((source, path.clone()));
        let manifest = match parse_manifest(&text, source, &dir) {
            Ok(m) => m,
            Err(d) => {
                diags.extend(d);
                continue;
            }
        };
        let ext = extension;
        // Where the package's modules are: its main file's directory for the root package, its
        // library's for others.
        let main = match &manifest.main {
            Some((m, _)) => Some(normalize(&dir.join(m))),
            None => first_file(&dir, &[format!("src/main.{ext}"), format!("main.{ext}")]),
        };
        let lib = match &manifest.lib {
            Some((l, _)) => Some(normalize(&dir.join(l))),
            None => first_file(&dir, &[format!("src/lib.{ext}"), format!("lib.{ext}")]),
        };
        if index == 0 {
            packages.name = manifest.name.clone();
            packages.main = main.clone();
            if let (Some((m, span)), Some(file)) = (&manifest.main, &main)
                && !file.is_file()
            {
                diags.push(
                    Diagnostic::error(
                        code::DEPENDENCY_NOT_FOUND,
                        &format!("the main file {m} does not exist"),
                    )
                    .with_span(*span)
                    .with_primary_label("no such file"),
                );
            }
        }
        let anchor = if index == 0 {
            main.as_ref().or(lib.as_ref())
        } else {
            lib.as_ref()
        };
        let src = anchor
            .and_then(|f| f.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| {
                if dir.join("src").is_dir() {
                    dir.join("src")
                } else {
                    dir.clone()
                }
            });
        let src_shown = match src.strip_prefix(&dir) {
            Ok(rel) if rel.as_os_str().is_empty() => shown.clone(),
            Ok(rel) => join_shown(&shown, &rel.to_string_lossy()),
            Err(_) => shown.clone(),
        };
        {
            let p = &mut packages.list[index];
            p.src = src;
            p.shown = src_shown;
            p.lib = lib;
        }
        for dep in &manifest.dependencies {
            let (dep_dir, dep_shown) = match &dep.origin {
                Origin::Path(p) => (normalize(&dir.join(p)), join_shown(&shown, p)),
                Origin::Git { url, reference } => {
                    let key = format!("{url}{}", reference.suffix());
                    let pinned = old_lock.commits.get(&key).map(String::as_str);
                    if options.locked && pinned.is_none() {
                        diags.push(
                            Diagnostic::error(
                                code::LOCK_OUT_OF_DATE,
                                &format!("the lock file does not pin `{}`", dep.name),
                            )
                            .with_span(dep.span)
                            .with_primary_label("not in the lock file")
                            .with_note(&format!(
                                "`--locked` keeps {language}.lock as it is; build without it once"
                            )),
                        );
                        continue;
                    }
                    let resolved = resolve_git(url, reference, pinned, options.fetch, language)
                        .and_then(|commit| checkout(url, &commit).map(|dir| (commit, dir)));
                    match resolved {
                        Ok((commit, checkout_dir)) => {
                            lock.commits.insert(key, commit.clone());
                            (
                                checkout_dir,
                                format!("{}@{}", dep.name, &commit[..commit.len().min(12)]),
                            )
                        }
                        Err(why) => {
                            diags.push(
                                Diagnostic::error(
                                    code::DEPENDENCY_FETCH,
                                    &format!("cannot fetch the dependency `{}`", dep.name),
                                )
                                .with_span(dep.span)
                                .with_primary_label("from this repository")
                                .with_note(&why),
                            );
                            continue;
                        }
                    }
                }
            };
            if !dep_dir.join(format!("{language}.toml")).is_file() {
                diags.push(
                    Diagnostic::error(
                        code::DEPENDENCY_NOT_FOUND,
                        &format!("the dependency `{}` is not a package", dep.name),
                    )
                    .with_span(dep.span)
                    .with_primary_label(&format!("no {language}.toml in {dep_shown}")),
                );
                continue;
            }
            let dep_index = match by_dir.get(&dep_dir) {
                Some(&i) => i,
                None => {
                    let i = packages.list.len();
                    // Module names are unique in a program: a second package of one name gets
                    // a number.
                    let count = prefixes.entry(dep.name.clone()).or_insert(0);
                    let prefix = if *count == 0 {
                        dep.name.clone()
                    } else {
                        format!("{}#{count}", dep.name)
                    };
                    *count += 1;
                    packages.list.push(Package {
                        prefix,
                        src: dep_dir.clone(),
                        shown: dep_shown.clone(),
                        lib: None,
                        deps: Vec::new(),
                    });
                    by_dir.insert(dep_dir.clone(), i);
                    queue.push((dep_dir, dep_shown, i));
                    i
                }
            };
            packages.list[index]
                .deps
                .push((dep.name.clone(), dep_index));
        }
    }
    if diags.has_errors() {
        return Err(diags);
    }
    // The lock file pins exactly the git dependencies in use.
    if lock != old_lock || (options.update && !lock.commits.is_empty()) {
        if options.locked {
            let mut d = Diagnostics::new();
            d.error(
                code::LOCK_OUT_OF_DATE,
                &format!("{language}.lock pins dependencies the manifests no longer name"),
                None,
            );
            return Err(d);
        }
        let had_file = lock_path.exists();
        if !lock.commits.is_empty() || had_file {
            let _ = std::fs::write(&lock_path, lock.text(language));
        }
    }
    Ok(packages)
}

/// `shown`/`rel`, with `/` between them, for diagnostics.
fn join_shown(shown: &str, rel: &str) -> String {
    let rel = rel.replace('\\', "/");
    let joined = if shown.is_empty() {
        rel
    } else {
        format!("{shown}/{rel}")
    };
    // `a/../b` as `b`, for paths as people write them.
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "." | "" => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(text: &str) -> Result<Manifest, String> {
        let mut sources = SourceMap::new();
        let id = sources.add("m.toml", text);
        parse_manifest(text, id, Path::new("/p/app")).map_err(|d| d.render(&sources))
    }

    #[test]
    fn manifests_parse() {
        let m = manifest(
            "# an app\n[package]\nname = \"app\"  # its name\nversion = \"0.1.0\"\n\n\
             [dependencies]\njson = { path = \"../json\" }\n\
             http = { git = \"https://example.com/http.git\", tag = \"v1\" }\n",
        )
        .unwrap();
        assert_eq!(m.name, "app");
        assert_eq!(
            m.dependencies
                .iter()
                .map(|d| (d.name.as_str(), d.origin.clone()))
                .collect::<Vec<_>>(),
            [
                ("json", Origin::Path("../json".to_string())),
                (
                    "http",
                    Origin::Git {
                        url: "https://example.com/http.git".to_string(),
                        reference: GitRef::Tag("v1".to_string())
                    }
                )
            ]
        );
    }

    #[test]
    fn manifest_errors_point_at_the_problem() {
        for (text, wanted) in [
            ("[package]\nname = app\n", "expected a value"),
            (
                "[package]\nname = \"a\"\nname = \"b\"\n",
                "the key `name` appears twice",
            ),
            (
                "[package]\nname = \"a\"\nauthor = \"me\"\n",
                "unknown key `author` in `[package]`",
            ),
            ("[package]\nversion = \"1\"\n", "the package has no name"),
            ("[pakage]\nname = \"a\"\n", "unknown table `[pakage]`"),
            ("name = \"a\"\n", "`name` is outside any table"),
            (
                "[package]\nname = \"a\"\n[dependencies]\njson = \"1.0\"\n",
                "says neither a path nor a git repository",
            ),
            (
                "[package]\nname = \"a\"\n[dependencies]\nj = { path = \"x\", tag = \"v\" }\n",
                "`tag` is for git dependencies",
            ),
            (
                "[package]\nname = \"a\"\n[dependencies]\nj = { git = \"u\", tag = \"v\", rev = \"r\" }\n",
                "one of `branch`, `tag` and `rev`",
            ),
            (
                "[package]\nname = \"a\" x\n",
                "expected the end of the line",
            ),
            ("[package]\nname = \"a\n", "this string is never closed"),
        ] {
            let err = manifest(text).unwrap_err();
            assert!(err.contains(wanted), "{text:?}: {err}");
        }
    }

    #[test]
    fn lock_files_round_trip() {
        let mut lock = Lock::default();
        lock.commits
            .insert("https://x/a.git#tag=v1".to_string(), "abc".to_string());
        let text = lock.text("demo");
        let mut sources = SourceMap::new();
        let id = sources.add("demo.lock", &text);
        let sections = parse_toml(&text, id).unwrap();
        assert_eq!(sections[1].name, "git");
        assert_eq!(sections[1].entries[0].key, "https://x/a.git#tag=v1");
    }

    #[test]
    fn shown_paths_are_normal() {
        assert_eq!(join_shown("", "../json"), "../json");
        assert_eq!(join_shown("../json", "../util/src"), "../util/src");
        assert_eq!(join_shown("a", "./b"), "a/b");
    }

    #[test]
    fn cache_names_are_stable() {
        assert_eq!(
            url_key("https://example.com/x/http.git"),
            url_key("https://example.com/x/http.git")
        );
        assert!(url_key("https://example.com/x/http.git").starts_with("http-"));
        assert_ne!(url_key("a/http"), url_key("b/http"));
    }

    #[test]
    fn mutated_manifests_never_panic() {
        // Manifests and lock files are input like any other (D84): mutations of valid ones must
        // give a manifest or diagnostics that render, never a panic. A0_FUZZ_ITERATIONS sets how
        // many (crates/a0/tests/fuzz.rs fuzzes the other inputs).
        let seeds = [
            "# an app\n[package]\nname = \"app\"  # its name\nversion = \"0.1.0\"\nmain = \"src/main.mn\"\n\n\
             [dependencies]\njson = { path = \"../json\" }\n\
             http = { git = \"https://example.com/http.git\", tag = \"v1\" }\n\
             util = { git = \"u\", branch = \"main\" }\nx = { git = \"u\", rev = \"abc\" }\n",
            "[package]\nname = \"lib\"\nlib = \"src/lib.mn\"\nflags = [1, true, \"s\", [], {}]\n",
            "# generated\n[git]\n\"https://x/a.git#tag=v1\" = \"abc\"\n",
        ];
        let words = [
            "[",
            "]",
            "{",
            "}",
            "\"",
            "=",
            ",",
            "#",
            "\n",
            "\\",
            "\\u{1F600}",
            "\\x",
            "é",
            "0",
            "-1",
            "true",
            "package",
            "dependencies",
            "path",
            "git",
            "[[",
            "]]",
            "'",
            ".",
        ];
        let iterations: u64 = std::env::var("A0_FUZZ_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000);
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n.max(1) as u64) as usize
        };
        for _ in 0..iterations {
            let mut text = seeds[next(seeds.len())].to_string();
            for _ in 0..1 + next(3) {
                let at = text.floor_char_boundary(next(text.len() + 1));
                let end = text.ceil_char_boundary(at + next(12));
                match next(4) {
                    0 => text.replace_range(at..end, ""),
                    1 => text.insert_str(at, words[next(words.len())]),
                    2 => {
                        let piece = text[at..end].to_string();
                        text.insert_str(text.floor_char_boundary(next(text.len() + 1)), &piece);
                    }
                    _ => text.replace_range(at..end, words[next(words.len())]),
                }
            }
            let mut sources = SourceMap::new();
            let id = sources.add("m.toml", &text);
            let result = std::panic::catch_unwind(|| {
                if let Err(d) = parse_manifest(&text, id, Path::new("/p/app")) {
                    d.render(&sources);
                }
            });
            assert!(result.is_ok(), "panicked on {text:?}");
        }
    }
}
