//! A reusable command-line driver for languages built on A0.
//!
//! A language describes itself with a [`Language`] (its name, its compiler, its exit codes and any
//! runtime support of its own) and calls [`Language::main`]. The driver provides the commands:
//!
//! ```text
//! lang run <file> [args...]      run the program, compiled into memory (`A0_INTERP=1`: interpret it)
//! lang build <file> [-o output]  compile it to a native executable for the host (`.exe` added
//!                                on Windows)
//! lang run|build|ir|hir          the same for the package the current directory is in
//!                                (`package.rs`; `run -- args` passes arguments)
//! lang update                    take the newest commits of the package's git dependencies
//! lang ir <file>                 print the core IR
//! lang hir <file>                print the HIR (languages that compile to the HIR)
//! lang lsp                       serve the Language Server Protocol on stdin and stdout
//! lang dap                       serve the Debug Adapter Protocol (a debugger) on stdin and stdout
//! lang repl                      a read-eval-print loop
//! lang explain <code>            explain an error code
//! lang <file> [args...]          the same as `run`, if the language says so
//! ```
//!
//! Compile errors are A0 diagnostics, rendered against the source the way rustc renders them, or
//! messages the language formats itself (for a prescribed format, like jlox's). When a program runs,
//! its exit status is passed through; the driver's own failures use the language's [`ExitCodes`].
//!
//! The language server (`lsp`) needs nothing more from a language: its diagnostics come from the
//! compiler, and highlighting, the outline, go-to-definition and hovers from the grammar's `@def`
//! and `@scope` annotations (`docs/grammar.md`) and the HIR the compiler builds.

pub mod dap;
pub mod json;
pub mod lsp;
pub mod package;
pub mod program;
mod repl;
pub mod snapshots;
mod values;

use std::io::{BufRead, Write};
use std::path::Path;
use std::process::ExitCode;

use a0_diag::{Diagnostics, RenderOptions, SourceId, SourceMap, Span};

/// What a compiler produces.
// Made once per compile, so the size of its variants does not matter.
#[allow(clippy::large_enum_variant)]
pub enum Program {
    /// A core IR module with a `@main`.
    Ir(a0_ir::Module),
    /// A HIR module with a `main`, lowered by the driver.
    Hir(a0_hir::HirModule),
}

/// Why compiling failed.
pub enum CompileError {
    /// Diagnostics with spans into the source file.
    Diagnostics(Diagnostics),
    /// Messages in the language's own format, printed one per line.
    Messages(Vec<Message>),
}

/// A compile error in the language's own format, with where it is (for editors).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub span: Option<Span>,
}

impl Message {
    pub fn new(text: &str, span: Option<Span>) -> Message {
        Message {
            text: text.to_string(),
            span,
        }
    }
}

/// How a language with modules compiles a program of several files (`program.rs`): the root file
/// and the files it imports (through the grammar's `@import` rules), each into a module of one HIR
/// builder.
#[derive(Clone, Copy)]
pub struct Modules {
    /// The extension of its source files, without the dot (`"mn"`).
    pub extension: &'static str,
    /// Prepares the builder before any file is compiled (type syntax, terms, warnings).
    pub start: fn(&mut a0_hir::HirBuilder),
    /// Compiles one file into the builder's current module; its imports are already in scope.
    /// Reports problems through the builder.
    pub compile: fn(&mut a0_hir::HirBuilder, &a0_syntax::SyntaxTree),
}

/// How the driver shows the codes of diagnostics.
#[derive(Clone, Copy)]
pub enum Codes {
    /// A0's codes (`E2001`), which `lang explain` explains.
    A0,
    /// No codes.
    Hidden,
    /// The language's own: its code for each of A0's and its own errors' (`None` shows no code).
    /// `lang explain` takes these codes and gives A0's explanation.
    Own(fn(&str) -> Option<&'static str>),
}

impl Codes {
    /// `d` with its codes as the language shows them.
    pub fn apply(self, d: &Diagnostics) -> Diagnostics {
        let mut out = Diagnostics::new();
        for item in d.as_slice() {
            let mut item = item.clone();
            item.code = match self {
                Codes::A0 => item.code,
                Codes::Hidden => String::new(),
                Codes::Own(map) => map(&item.code).unwrap_or_default().to_string(),
            };
            out.push(item);
        }
        out
    }

    /// The explanation of a code as the language shows it.
    pub fn explain(self, code: &str) -> Option<&'static a0_diag::codes::CodeInfo> {
        a0_diag::codes::ALL.iter().find(|c| match self {
            Codes::A0 | Codes::Hidden => c.code == code,
            Codes::Own(map) => map(c.code) == Some(code),
        })
    }
}

/// The exit statuses of the driver's own failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitCodes {
    /// A bad command line.
    pub usage: u8,
    /// The source file cannot be read.
    pub io: u8,
    /// The program has compile errors.
    pub compile: u8,
    /// Lowering, code generation or linking failed (a bug, or a missing system linker).
    pub internal: u8,
}

impl Default for ExitCodes {
    fn default() -> Self {
        ExitCodes {
            usage: 2,
            io: 2,
            compile: 1,
            internal: 1,
        }
    }
}

/// A language, as the driver sees it.
pub struct Language {
    /// The command name, used in messages and the usage text.
    pub name: &'static str,
    /// Compiles one source file. `source` is the file's id in the driver's source map; spans in
    /// diagnostics refer to it.
    pub compile: fn(text: &str, source: SourceId) -> Result<Program, CompileError>,
    pub exit_codes: ExitCodes,
    /// Whether `lang <file>` means `lang run <file>`.
    pub run_by_default: bool,
    /// Interpreter symbols for a language that ships its own C runtime.
    pub host_symbols: Option<fn() -> a0_interp::HostSymbols>,
    /// Static libraries to link into executables (a language's own C runtime).
    pub libraries: Option<fn() -> Vec<a0_native::Library>>,
    /// The language's grammar (the text of its `.a0g` file), for the language server.
    pub grammar: Option<&'static str>,
    /// How error codes are shown.
    pub codes: Codes,
    /// For a language whose programs have several files: how to compile them. `compile` is then
    /// only used for single files given by text (the C API, tests).
    pub modules: Option<Modules>,
    /// For a language whose statements live in a function: the REPL's program, with `{}` where
    /// the statements typed so far go (`"fn main() {\n{}}\n"`). `None` when statements run at the
    /// top level, in the order they come (Lox, Kaleidoscope).
    pub repl_main: Option<&'static str>,
}

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub command: Command,
    pub file: String,
    /// `build -o`
    pub output: Option<String>,
    /// Arguments after the file, passed to the program by `run`.
    pub args: Vec<String>,
    /// `build -g`: describe the executable for the platform's debugger.
    pub debug: bool,
    /// `build --target`: the target to build for, if not the host (D79).
    pub target: Option<String>,
    /// `--locked`: use the package's lock file as it is (D81).
    pub locked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Run,
    Build,
    Ir,
    Hir,
    Lsp,
    /// The Debug Adapter Protocol (`dap.rs`).
    Dap,
    /// The read-eval-print loop (`repl.rs`).
    Repl,
    /// `explain <code>`: the code is the invocation's `file`.
    Explain,
    /// `update`: re-resolve the package's git dependencies and rewrite its lock file.
    Update,
}

/// Whether `run` interprets the program (`A0_INTERP=1`, or a host without native code) rather
/// than compiling it into memory and running it at native speed (D63).
pub fn interpret() -> bool {
    std::env::var("A0_INTERP").is_ok_and(|v| !v.is_empty() && v != "0")
        || !a0_native::jit_supported()
}

/// How usage messages name a command.
fn command_word(command: Command) -> &'static str {
    match command {
        Command::Run => "run",
        Command::Build => "build",
        Command::Ir => "ir",
        Command::Hir => "hir",
        _ => "update",
    }
}

/// Whether `root` (as given to the compiler) is the package's main file.
fn packages_main_is(packages: &package::Packages, root: &str) -> bool {
    packages.main.as_deref() == Some(program::normalize(Path::new(root)).as_path())
}

/// Where `build` writes the executable: `-o`'s name, or the source file's stem, with the platform's
/// suffix added when the name has none (Windows cannot run `fib`, only `fib.exe`).
fn executable_name(file: &str, output: Option<&str>, suffix: &str) -> String {
    match output {
        Some(o) if Path::new(o).extension().is_some() => o.to_string(),
        Some(o) => format!("{o}{suffix}"),
        None => {
            let stem = Path::new(file).file_stem().unwrap_or_default();
            format!("{}{suffix}", stem.to_string_lossy())
        }
    }
}

impl Language {
    /// Runs the driver on the process's command line.
    pub fn main(&self) -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        self.run(&args)
    }

    /// Runs the driver on `args` (without the program name).
    pub fn run(&self, args: &[String]) -> ExitCode {
        ExitCode::from(self.run_status(args))
    }

    /// [`Language::run`], with the exit status as a number (for hosts that are not Rust programs,
    /// `a0_language_main` in the C API). A program that runs ends the process with its own status.
    pub fn run_status(&self, args: &[String]) -> u8 {
        let Some(inv) = self.parse_args(args) else {
            eprint!("{}", self.usage());
            return self.exit_codes.usage;
        };
        if inv.command == Command::Lsp {
            let input = Box::new(std::io::BufReader::new(std::io::stdin()));
            let code = self.serve_lsp(input, &mut std::io::stdout());
            return code;
        }
        if inv.command == Command::Repl {
            return repl::repl(self);
        }
        if inv.command == Command::Dap {
            let input = Box::new(std::io::BufReader::new(std::io::stdin()));
            return dap::serve(self, input, Box::new(std::io::stdout()));
        }
        if inv.command == Command::Explain {
            return match self.codes.explain(&inv.file) {
                Some(c) => {
                    print!("{}", c.describe(&inv.file));
                    0
                }
                None => {
                    eprintln!("{}: no error has the code `{}`", self.name, inv.file);
                    self.exit_codes.usage
                }
            };
        }
        let mut inv = inv;
        let mut sources = SourceMap::new();
        // The package the program is in, if the language has modules (D81): without a file, the
        // one the current directory is in.
        let mut packages = None;
        if let Some(modules) = self.modules
            && inv.file != "-"
        {
            let from = if inv.file.is_empty() {
                std::env::current_dir().unwrap_or_default()
            } else {
                Path::new(&inv.file).to_path_buf()
            };
            match package::find_manifest(&from, self.name) {
                Some(manifest) => {
                    let options = package::LoadOptions {
                        fetch: true,
                        locked: inv.locked,
                        update: inv.command == Command::Update,
                    };
                    let loaded = package::load(
                        &manifest,
                        self.name,
                        modules.extension,
                        options,
                        &mut sources,
                        &mut Vec::new(),
                    );
                    match loaded {
                        Ok(p) => packages = Some(p),
                        Err(d) => {
                            let d = self.codes.apply(&d);
                            eprint!("{}", d.render_with(&sources, &RenderOptions::stderr()));
                            return self.exit_codes.compile;
                        }
                    }
                }
                None if inv.file.is_empty() => {
                    let d: Diagnostics = a0_diag::Diagnostic::error(
                        a0_diag::codes::NO_PACKAGE,
                        &format!(
                            "no {}.toml in this directory or its parents, and no file to {}",
                            self.name,
                            command_word(inv.command)
                        ),
                    )
                    .with_help(&format!(
                        "give a file: `{} {} <file>`",
                        self.name,
                        command_word(inv.command)
                    ))
                    .into();
                    eprint!(
                        "{}",
                        self.codes
                            .apply(&d)
                            .render_with(&sources, &RenderOptions::stderr())
                    );
                    return self.exit_codes.usage;
                }
                None => {}
            }
        }
        if inv.command == Command::Update {
            return 0;
        }
        if inv.file.is_empty() {
            let packages = packages.as_ref().expect("checked above");
            let Some(main) = &packages.main else {
                eprintln!(
                    "{}: the package {} has no main file (src/main.{})",
                    self.name,
                    packages.name,
                    self.modules.map_or("", |m| m.extension)
                );
                return self.exit_codes.usage;
            };
            let cwd = std::env::current_dir().unwrap_or_default();
            inv.file = main
                .strip_prefix(&cwd)
                .unwrap_or(main)
                .to_string_lossy()
                .replace('\\', "/");
            if inv.command == Command::Build && inv.output.is_none() {
                let out = packages.dir.join("build");
                let _ = std::fs::create_dir_all(&out);
                let out = out.join(&packages.name);
                inv.output = Some(
                    out.strip_prefix(&cwd)
                        .unwrap_or(&out)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        // `-` is standard input (the REPL runs its programs this way); imports are found from the
        // current directory then.
        let read = if inv.file == "-" {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map(|_| text)
        } else {
            std::fs::read_to_string(&inv.file)
        };
        let text = match read {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}: cannot read {}: {e}", self.name, inv.file);
                return self.exit_codes.io;
            }
        };
        let name = match inv.file.as_str() {
            "-" if std::env::var_os("A0_REPL").is_some() => "<repl>",
            "-" => "<stdin>",
            file => file,
        };
        let compiled = match self.modules {
            Some(modules) => {
                self.compile_program(&modules, name, &text, packages.as_ref(), &mut sources)
            }
            None => {
                let id = sources.add(name, &text);
                (self.compile)(&text, id)
            }
        };
        let program = match compiled {
            Ok(p) => p,
            Err(CompileError::Diagnostics(d)) => {
                let d = self.codes.apply(&d);
                eprint!("{}", d.render_with(&sources, &RenderOptions::stderr()));
                return self.exit_codes.compile;
            }
            Err(CompileError::Messages(messages)) => {
                for m in messages {
                    eprintln!("{}", m.text);
                }
                return self.exit_codes.compile;
            }
        };
        // The REPL runs its program for each input; warnings would come each time.
        if let Program::Hir(m) = &program
            && !m.warnings().is_empty()
            && std::env::var_os("A0_REPL").is_none()
        {
            let w = self.codes.apply(m.warnings());
            eprint!("{}", w.render_with(&sources, &RenderOptions::stderr()));
            eprintln!();
        }
        let module = match program {
            Program::Hir(m) if inv.command == Command::Hir => {
                print!("{m}");
                return 0;
            }
            Program::Ir(_) if inv.command == Command::Hir => {
                eprintln!(
                    "{}: this language compiles to the core IR, not the HIR",
                    self.name
                );
                return self.exit_codes.usage;
            }
            Program::Ir(m) => m,
            // Debug information describes the program as written: no inlining.
            Program::Hir(m) if inv.debug => {
                match a0_hir::lower_with(&m, a0_hir::LowerOptions { optimize: false }) {
                    Ok(m) => m,
                    Err(d) => return self.internal_error(&d, &sources),
                }
            }
            Program::Hir(m) => match a0_hir::lower(&m) {
                Ok(m) => m,
                Err(d) => return self.internal_error(&d, &sources),
            },
        };
        // Runtime errors say where in the source they happened.
        let mut module = module;
        module.set_sources(sources.clone());
        match inv.command {
            Command::Ir => {
                print!("{module}");
                0
            }
            Command::Run => {
                let symbols = match self.host_symbols {
                    Some(f) => f(),
                    None => a0_interp::HostSymbols::new(),
                };
                let mut argv = vec![inv.file.clone()];
                argv.extend(inv.args);
                if !interpret() {
                    return match a0_native::run_main(
                        &module,
                        symbols,
                        argv,
                        &a0_native::JitOptions::default(),
                    ) {
                        Err(d) => self.internal_error(&d, &sources),
                        Ok(code) => a0_runtime::exit(code),
                    };
                }
                let mut symbols = symbols;
                if let Err(d) = a0_native::add_thunks(&module, &mut symbols) {
                    return self.internal_error(&d, &sources);
                }
                match a0_interp::run_main(module, symbols, argv) {
                    Err(d) => self.internal_error(&d, &sources),
                    Ok(Err(trap)) => trap.report_and_exit(),
                    Ok(Ok(code)) => a0_runtime::exit(code),
                }
            }
            Command::Build => {
                let target = match inv.target.as_deref().map(a0_native::Target::parse) {
                    Some(Ok(t)) => t,
                    Some(Err(e)) => {
                        eprintln!("{}: {e}", self.name);
                        return self.exit_codes.usage;
                    }
                    None => match a0_native::Target::host() {
                        Some(t) => t,
                        None => {
                            eprintln!("{}: native code is not supported on this host", self.name);
                            return self.exit_codes.internal;
                        }
                    },
                };
                let output = executable_name(&inv.file, inv.output.as_deref(), target.exe_suffix());
                let mut opts = a0_native::BuildOptions::default();
                if let Some(libraries) = self.libraries {
                    opts.link.libraries = libraries();
                }
                if inv.debug {
                    opts.debug_info = true;
                    opts.no_optimize = true;
                }
                match a0_native::build_executable(&module, target, Path::new(&output), &opts) {
                    Ok(()) => 0,
                    Err(d) => self.internal_error(&d, &sources),
                }
            }
            Command::Hir
            | Command::Lsp
            | Command::Dap
            | Command::Repl
            | Command::Explain
            | Command::Update => {
                unreachable!("handled above")
            }
        }
    }

    /// Compiles the program whose root file is `root` (with text `text`), loading the files it
    /// imports from disk, and checks its entry point.
    fn compile_program(
        &self,
        modules: &Modules,
        root: &str,
        text: &str,
        packages: Option<&package::Packages>,
        sources: &mut SourceMap,
    ) -> Result<Program, CompileError> {
        let Some(grammar) = self
            .grammar
            .map(|g| a0_syntax::Grammar::from_str(g, SourceId(u32::MAX)))
        else {
            panic!("{}: a language with modules needs its grammar", self.name);
        };
        let grammar = grammar.unwrap_or_else(|d| panic!("{}: {}", self.name, d.summary()));
        let root_path = program::normalize(Path::new(root));
        let mut read = |p: &Path| {
            if *p == root_path {
                Some(text.to_string())
            } else {
                std::fs::read_to_string(p).ok()
            }
        };
        let loaded = program::load(&grammar, modules, root, packages, &mut read, sources)
            .map_err(CompileError::Diagnostics)?;
        let name = match packages {
            Some(p) if packages_main_is(p, root) => p.name.clone(),
            _ => Path::new(root)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        let m = program::compile(&loaded, modules, &name).map_err(CompileError::Diagnostics)?;
        let entry = a0_hir::check_entry(&m);
        if entry.has_errors() {
            return Err(CompileError::Diagnostics(entry));
        }
        Ok(Program::Hir(m))
    }

    /// The package `file` is in, loaded without fetching anything (for the debugger and the
    /// language server; `build` fetches): `None` without a manifest or for a language without
    /// modules. `read` collects the manifests' sources and paths.
    pub fn packages_of(
        &self,
        file: &Path,
        sources: &mut SourceMap,
        read: &mut Vec<(SourceId, std::path::PathBuf)>,
    ) -> Result<Option<package::Packages>, Diagnostics> {
        let Some(modules) = self.modules else {
            return Ok(None);
        };
        let Some(manifest) = package::find_manifest(file, self.name) else {
            return Ok(None);
        };
        package::load(
            &manifest,
            self.name,
            modules.extension,
            package::LoadOptions::default(),
            sources,
            read,
        )
        .map(Some)
    }

    /// Serves the Language Server Protocol until the client exits or the input ends; returns the
    /// exit status (0 after a `shutdown` request).
    pub fn serve_lsp(&self, input: Box<dyn BufRead + Send>, output: &mut dyn Write) -> u8 {
        lsp::serve(self, input, output)
    }

    fn internal_error(&self, d: &Diagnostics, sources: &SourceMap) -> u8 {
        eprint!("{}", d.render_with(sources, &RenderOptions::stderr()));
        self.exit_codes.internal
    }

    /// Parses a command line, or returns `None` if it is not valid.
    pub fn parse_args(&self, args: &[String]) -> Option<Invocation> {
        if let [only] = args
            && matches!(only.as_str(), "lsp" | "dap" | "repl")
        {
            return Some(Invocation {
                command: match only.as_str() {
                    "lsp" => Command::Lsp,
                    "dap" => Command::Dap,
                    _ => Command::Repl,
                },
                file: String::new(),
                output: None,
                args: Vec::new(),
                debug: false,
                target: None,
                locked: false,
            });
        }
        if let [only] = args
            && only == "update"
            && self.modules.is_some()
        {
            return Some(Invocation {
                command: Command::Update,
                file: String::new(),
                output: None,
                args: Vec::new(),
                debug: false,
                target: None,
                locked: false,
            });
        }
        if let [explain, code] = args
            && explain == "explain"
        {
            return Some(Invocation {
                command: Command::Explain,
                file: code.clone(),
                output: None,
                args: Vec::new(),
                debug: false,
                target: None,
                locked: false,
            });
        }
        let (command, rest) = match args.first().map(String::as_str) {
            Some("run") => (Command::Run, &args[1..]),
            Some("build") => (Command::Build, &args[1..]),
            Some("ir") => (Command::Ir, &args[1..]),
            Some("hir") => (Command::Hir, &args[1..]),
            Some(_) if self.run_by_default => (Command::Run, args),
            _ => return None,
        };
        // Without a file (nothing, a flag, or `--` before the program's arguments), a language with
        // modules builds the package the current directory is in.
        let (file, rest) = match rest.split_first() {
            Some((f, more)) if !f.starts_with('-') || f == "-" => (f.clone(), more),
            _ if self.modules.is_some() => match rest.split_first() {
                Some((dashes, more)) if dashes == "--" && command == Command::Run => {
                    (String::new(), more)
                }
                _ => (String::new(), rest),
            },
            _ => return None,
        };
        let mut inv = Invocation {
            command,
            file,
            output: None,
            args: Vec::new(),
            debug: false,
            target: None,
            locked: false,
        };
        match (command, rest) {
            (Command::Run, rest) if !inv.file.is_empty() => inv.args = rest.to_vec(),
            (Command::Run, rest) => {
                // `run --locked -- args`
                let mut rest = rest;
                while let Some((flag, more)) = rest.split_first() {
                    match flag.as_str() {
                        "--locked" if !inv.locked => inv.locked = true,
                        "--" => {
                            inv.args = more.to_vec();
                            break;
                        }
                        _ => {
                            inv.args = rest.to_vec();
                            break;
                        }
                    }
                    rest = more;
                }
            }
            (Command::Build, mut rest) => {
                while let Some((flag, more)) = rest.split_first() {
                    match (flag.as_str(), more) {
                        ("--locked", more) if !inv.locked => {
                            inv.locked = true;
                            rest = more;
                        }
                        ("-o", [out, more @ ..]) if inv.output.is_none() => {
                            inv.output = Some(out.clone());
                            rest = more;
                        }
                        ("-g", more) if !inv.debug => {
                            inv.debug = true;
                            rest = more;
                        }
                        ("--target", [t, more @ ..]) if inv.target.is_none() => {
                            inv.target = Some(t.clone());
                            rest = more;
                        }
                        _ => return None,
                    }
                }
            }
            (_, []) => {}
            _ => return None,
        }
        Some(inv)
    }

    pub fn usage(&self) -> String {
        let n = self.name;
        let run = if self.run_by_default {
            format!(" (also `{n} <file>`)")
        } else {
            String::new()
        };
        let packages = if self.modules.is_some() {
            format!(
                "\n\
                 Without a file, run, build, ir and hir take the package the current directory is\n\
                 in ({n}.toml; `{n} run -- args...`, `build --locked`):\n\
                 \x20 update                 take the newest commits of its git dependencies\n"
            )
        } else {
            String::new()
        };
        format!(
            "usage: {n} <command> <file>\n\n\
             commands:\n\
             \x20 run <file> [args...]   run the program{run}\n\
             \x20 build <file> [-o out] [-g] [--target <target>]\n\
             \x20                        compile it to a native executable (-g: for debuggers;\n\
             \x20                        another target needs its pack: targets/build.py)\n\
             \x20 ir <file>              print the core IR\n\
             \x20 hir <file>             print the high-level IR\n\
             \x20 lsp                    serve the Language Server Protocol on stdin/stdout\n\
             \x20 dap                    serve the Debug Adapter Protocol (debugging) on stdin/stdout\n\
             \x20 explain <code>         explain an error code\n{packages}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(run_by_default: bool) -> Language {
        Language {
            name: "demo",
            compile: |_, _| Err(CompileError::Messages(Vec::new())),
            exit_codes: ExitCodes::default(),
            run_by_default,
            host_symbols: None,
            libraries: None,
            grammar: None,
            codes: Codes::A0,
            modules: None,
            repl_main: None,
        }
    }

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn command_lines_parse() {
        let l = lang(false);
        let inv = l.parse_args(&args("run a.d x y")).unwrap();
        assert_eq!(inv.command, Command::Run);
        assert_eq!(inv.file, "a.d");
        assert_eq!(inv.args, args("x y"));
        let inv = l.parse_args(&args("build a.d -o out")).unwrap();
        assert_eq!(
            (inv.command, inv.output.as_deref()),
            (Command::Build, Some("out"))
        );
        assert_eq!(l.parse_args(&args("ir a.d")).unwrap().command, Command::Ir);
        assert_eq!(
            l.parse_args(&args("hir a.d")).unwrap().command,
            Command::Hir
        );
        assert_eq!(l.parse_args(&args("lsp")).unwrap().command, Command::Lsp);
        let inv = l.parse_args(&args("explain E2001")).unwrap();
        assert_eq!(
            (inv.command, inv.file.as_str()),
            (Command::Explain, "E2001")
        );
        for bad in [
            "",
            "run",
            "a.d",
            "ir a.d extra",
            "build a.d -x out",
            "build a.d -o",
            "lsp a.d",
        ] {
            assert!(
                l.parse_args(&args(bad)).is_none(),
                "{bad:?} should not parse"
            );
        }
    }

    #[test]
    fn executables_get_the_platform_suffix() {
        assert_eq!(executable_name("dir/fib.k", None, ".exe"), "fib.exe");
        assert_eq!(
            executable_name("fib.k", Some("out/fast"), ".exe"),
            "out/fast.exe"
        );
        assert_eq!(
            executable_name("fib.k", Some("fast.bin"), ".exe"),
            "fast.bin"
        );
        assert_eq!(executable_name("fib.k", Some("fast"), ""), "fast");
    }

    #[test]
    fn codes_can_be_hidden_or_the_language_s_own() {
        let mut d = Diagnostics::new();
        d.error("E2001", "mismatched types", None);
        assert_eq!(Codes::A0.apply(&d).get(0).code, "E2001");
        assert_eq!(Codes::Hidden.apply(&d).get(0).code, "");
        let own = Codes::Own(|c| (c == "E2001").then_some("T1"));
        assert_eq!(own.apply(&d).get(0).code, "T1");
        assert_eq!(own.explain("T1").unwrap().title, "mismatched types");
        assert!(own.explain("E2001").is_none());
        assert_eq!(Codes::A0.explain("E2001").unwrap().code, "E2001");
    }

    #[test]
    fn run_can_be_the_default() {
        let l = lang(true);
        let inv = l.parse_args(&args("a.d x")).unwrap();
        assert_eq!((inv.command, inv.file.as_str()), (Command::Run, "a.d"));
        assert_eq!(inv.args, args("x"));
        assert!(l.usage().contains("demo <file>"));
        assert!(!lang(false).usage().contains("demo <file>"));
    }
}
