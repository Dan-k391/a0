//! A reusable command-line driver for languages built on A0.
//!
//! A language describes itself with a [`Language`] (its name, its compiler, its exit codes and any
//! runtime support of its own) and calls [`Language::main`]. The driver provides the commands:
//!
//! ```text
//! lang run <file> [args...]      interpret the program
//! lang build <file> [-o output]  compile it to a native executable for the host (`.exe` added
//!                                on Windows)
//! lang ir <file>                 print the core IR
//! lang hir <file>                print the HIR (languages that compile to the HIR)
//! lang lsp                       serve the Language Server Protocol on stdin and stdout
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

pub mod json;
pub mod lsp;
pub mod snapshots;

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Run,
    Build,
    Ir,
    Hir,
    Lsp,
    /// `explain <code>`: the code is the invocation's `file`.
    Explain,
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
        let Some(inv) = self.parse_args(args) else {
            eprint!("{}", self.usage());
            return ExitCode::from(self.exit_codes.usage);
        };
        if inv.command == Command::Lsp {
            let stdin = std::io::stdin();
            let code = self.serve_lsp(&mut stdin.lock(), &mut std::io::stdout());
            return ExitCode::from(code);
        }
        if inv.command == Command::Explain {
            return match self.codes.explain(&inv.file) {
                Some(c) => {
                    print!("{}", c.describe(&inv.file));
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("{}: no error has the code `{}`", self.name, inv.file);
                    ExitCode::from(self.exit_codes.usage)
                }
            };
        }
        let text = match std::fs::read_to_string(&inv.file) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}: cannot read {}: {e}", self.name, inv.file);
                return ExitCode::from(self.exit_codes.io);
            }
        };
        let mut sources = SourceMap::new();
        let id = sources.add(&inv.file, &text);
        let program = match (self.compile)(&text, id) {
            Ok(p) => p,
            Err(CompileError::Diagnostics(d)) => {
                let d = self.codes.apply(&d);
                eprint!("{}", d.render_with(&sources, &RenderOptions::stderr()));
                return ExitCode::from(self.exit_codes.compile);
            }
            Err(CompileError::Messages(messages)) => {
                for m in messages {
                    eprintln!("{}", m.text);
                }
                return ExitCode::from(self.exit_codes.compile);
            }
        };
        if let Program::Hir(m) = &program
            && !m.warnings().is_empty()
        {
            let w = self.codes.apply(m.warnings());
            eprint!("{}", w.render_with(&sources, &RenderOptions::stderr()));
            eprintln!();
        }
        let module = match program {
            Program::Hir(m) if inv.command == Command::Hir => {
                print!("{m}");
                return ExitCode::SUCCESS;
            }
            Program::Ir(_) if inv.command == Command::Hir => {
                eprintln!(
                    "{}: this language compiles to the core IR, not the HIR",
                    self.name
                );
                return ExitCode::from(self.exit_codes.usage);
            }
            Program::Ir(m) => m,
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
                ExitCode::SUCCESS
            }
            Command::Run => {
                let symbols = match self.host_symbols {
                    Some(f) => f(),
                    None => a0_interp::HostSymbols::new(),
                };
                let mut argv = vec![inv.file.clone()];
                argv.extend(inv.args);
                match a0_interp::run_main(module, symbols, argv) {
                    Err(d) => self.internal_error(&d, &sources),
                    Ok(Err(trap)) => trap.report_and_exit(),
                    Ok(Ok(code)) => a0_runtime::exit(code),
                }
            }
            Command::Build => {
                let Some(target) = a0_native::Target::host() else {
                    eprintln!("{}: native code is not supported on this host", self.name);
                    return ExitCode::from(self.exit_codes.internal);
                };
                let output = executable_name(&inv.file, inv.output.as_deref(), target.exe_suffix());
                let mut opts = a0_native::BuildOptions::default();
                if let Some(libraries) = self.libraries {
                    opts.link.libraries = libraries();
                }
                match a0_native::build_executable(&module, target, Path::new(&output), &opts) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(d) => self.internal_error(&d, &sources),
                }
            }
            Command::Hir | Command::Lsp | Command::Explain => unreachable!("handled above"),
        }
    }

    /// Serves the Language Server Protocol until the client exits or the input ends; returns the
    /// exit status (0 after a `shutdown` request).
    pub fn serve_lsp(&self, input: &mut dyn BufRead, output: &mut dyn Write) -> u8 {
        lsp::serve(self, input, output)
    }

    fn internal_error(&self, d: &Diagnostics, sources: &SourceMap) -> ExitCode {
        eprint!("{}", d.render_with(sources, &RenderOptions::stderr()));
        ExitCode::from(self.exit_codes.internal)
    }

    /// Parses a command line, or returns `None` if it is not valid.
    pub fn parse_args(&self, args: &[String]) -> Option<Invocation> {
        if args.len() == 1 && args[0] == "lsp" {
            return Some(Invocation {
                command: Command::Lsp,
                file: String::new(),
                output: None,
                args: Vec::new(),
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
        let (file, rest) = rest.split_first()?;
        let mut inv = Invocation {
            command,
            file: file.clone(),
            output: None,
            args: Vec::new(),
        };
        match (command, rest) {
            (Command::Run, rest) => inv.args = rest.to_vec(),
            (Command::Build, [o, out]) if o == "-o" => inv.output = Some(out.clone()),
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
        format!(
            "usage: {n} <command> <file>\n\n\
             commands:\n\
             \x20 run <file> [args...]   interpret the program{run}\n\
             \x20 build <file> [-o out]  compile it to a native executable\n\
             \x20 ir <file>              print the core IR\n\
             \x20 hir <file>             print the high-level IR\n\
             \x20 lsp                    serve the Language Server Protocol on stdin/stdout\n\
             \x20 explain <code>         explain an error code\n"
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
