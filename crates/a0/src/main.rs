//! `a0`: work with A0 IR files (`.a0ir`, core IR; `.hir`, high-level IR) from the command line.

use std::process::ExitCode;

use a0_diag::{Diagnostics, RenderOptions, SourceMap};
use a0_ir::Module;
use a0_native::Target;

mod lang;
mod new;

const USAGE: &str = "usage: a0 <command> [options] <file>

Files are core IR (.a0ir) or high-level IR (.hir); HIR is lowered to core IR before running.

commands:
  check <file>              parse and verify; print diagnostics
  fmt <file>                print the canonical text form
  lower <file.hir>          print the core IR a HIR module lowers to
  run <file> [-- args...]   interpret @main and exit with its status
  build <file> [-o out] [--target T] [--no-opt] [-g]
                            compile to a native executable (host target only); -g describes
                            it for gdb and lldb (Linux, macOS)
  obj <file> [-o out] [--target T] [--no-opt] [-g]
                            compile to an object file for any supported target
  parse <grammar.a0g> <file>
                            parse a file with a grammar; print the tree or the errors
  explain <code>            explain an error code (E2001)
  new <name> [--dir D] [--ext E] [--a0 PATH]
                            create a language project (grammar, compiler, tests, editor setup)
  lang <grammar.a0g> <command> ...
                            a language defined by its grammar's @hir forms: run, build, hir,
                            ir, lsp or explain, as with any language's driver

targets: x86_64-linux, aarch64-linux, x86_64-macos, aarch64-macos, x86_64-windows,
         aarch64-windows (or full triples); the default is the host
";

fn main() -> ExitCode {
    // Compiling recurses as deep as the program nests (D83).
    std::thread::Builder::new()
        .name("a0".to_string())
        .stack_size(a0_driver::COMPILER_STACK)
        .spawn(main_here)
        .expect("cannot start the compiler's thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn main_here() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    match command.as_str() {
        "check" => check(rest),
        "fmt" => fmt(rest),
        "lower" => lower(rest),
        "run" => run(rest),
        "build" => build(rest, true),
        "obj" => build(rest, false),
        "parse" => parse_with_grammar(rest),
        "new" => new::new(rest),
        "lang" => lang::lang(rest),
        "explain" => match rest {
            [code] => match a0_diag::codes::explain(code) {
                Some(c) => {
                    print!("{}", c.describe(code));
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("a0: no error has the code `{code}`");
                    ExitCode::from(2)
                }
            },
            _ => {
                eprint!("{USAGE}");
                ExitCode::from(2)
            }
        },
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("a0: unknown command '{other}'\n");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Splits `args` at `--`: options and the input file before it, program arguments after.
fn split_program_args(args: &[String]) -> (&[String], &[String]) {
    match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[]),
    }
}

fn single_file(args: &[String]) -> Result<&str, ExitCode> {
    match args {
        [file] => Ok(file),
        _ => {
            eprint!("{USAGE}");
            Err(ExitCode::from(2))
        }
    }
}

fn is_hir(path: &str) -> bool {
    path.ends_with(".hir")
}

fn read(path: &str) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|e| {
        eprintln!("a0: cannot read {path}: {e}");
        ExitCode::from(2)
    })
}

fn load_hir(path: &str, sources: &mut SourceMap) -> Result<a0_hir::HirModule, ExitCode> {
    let text = read(path)?;
    let id = sources.add(path, &text);
    a0_hir::parse(&text, id).map_err(|d| report(sources, &d))
}

/// Reads, parses and verifies an IR file (lowering HIR first), printing diagnostics on failure.
fn load(path: &str, sources: &mut SourceMap) -> Result<Module, ExitCode> {
    load_with(path, sources, a0_hir::LowerOptions::default())
}

/// [`load`], lowering HIR with `options` (debug information wants no inlining).
fn load_with(
    path: &str,
    sources: &mut SourceMap,
    options: a0_hir::LowerOptions,
) -> Result<Module, ExitCode> {
    let mut m = if is_hir(path) {
        let hir = load_hir(path, sources)?;
        a0_hir::lower_with(&hir, options).map_err(|d| report(sources, &d))?
    } else {
        let text = read(path)?;
        let id = sources.add(path, &text);
        a0_ir::parse_and_verify(&text, id).map_err(|d| report(sources, &d))?
    };
    // Runtime errors say where in the file they happened.
    m.set_sources(sources.clone());
    Ok(m)
}

fn report(sources: &SourceMap, diags: &Diagnostics) -> ExitCode {
    eprint!("{}", diags.render_with(sources, &RenderOptions::stderr()));
    ExitCode::from(1)
}

fn check(args: &[String]) -> ExitCode {
    let path = match single_file(args) {
        Ok(p) => p,
        Err(c) => return c,
    };
    match load(path, &mut SourceMap::new()) {
        Ok(_) => ExitCode::SUCCESS,
        Err(c) => c,
    }
}

fn fmt(args: &[String]) -> ExitCode {
    let path = match single_file(args) {
        Ok(p) => p,
        Err(c) => return c,
    };
    if is_hir(path) {
        return match load_hir(path, &mut SourceMap::new()) {
            Ok(m) => {
                print!("{}", a0_hir::print_module(&m));
                ExitCode::SUCCESS
            }
            Err(c) => c,
        };
    }
    match load(path, &mut SourceMap::new()) {
        Ok(m) => {
            print!("{}", a0_ir::print_module(&m));
            ExitCode::SUCCESS
        }
        Err(c) => c,
    }
}

fn lower(args: &[String]) -> ExitCode {
    let path = match single_file(args) {
        Ok(p) => p,
        Err(c) => return c,
    };
    if !is_hir(path) {
        eprintln!("a0: lower takes a .hir file");
        return ExitCode::from(2);
    }
    match load(path, &mut SourceMap::new()) {
        Ok(m) => {
            print!("{}", a0_ir::print_module(&m));
            ExitCode::SUCCESS
        }
        Err(c) => c,
    }
}

fn run(args: &[String]) -> ExitCode {
    let (opts, program_args) = split_program_args(args);
    let path = match single_file(opts) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let mut sources = SourceMap::new();
    let module = match load(path, &mut sources) {
        Ok(m) => m,
        Err(c) => return c,
    };
    let entry = a0_ir::check_entry(&module);
    if entry.has_errors() {
        return report(&sources, &entry);
    }
    let mut argv = vec![path.to_string()];
    argv.extend(program_args.iter().cloned());
    if !a0_driver::interpret() {
        return match a0_native::run_main(
            &module,
            a0_interp::HostSymbols::new(),
            argv,
            &a0_native::JitOptions::default(),
        ) {
            Err(diags) => report(&sources, &diags),
            Ok(code) => a0_runtime::exit(code),
        };
    }
    let mut symbols = a0_interp::HostSymbols::new();
    if let Err(diags) = a0_native::add_thunks(&module, &mut symbols) {
        return report(&sources, &diags);
    }
    match a0_interp::run_main(module, symbols, argv) {
        Err(diags) => report(&sources, &diags),
        Ok(Err(trap)) => trap.report_and_exit(),
        Ok(Ok(code)) => a0_runtime::exit(code),
    }
}

struct BuildArgs {
    file: String,
    output: Option<String>,
    target: Target,
    optimize: bool,
    debug_info: bool,
}

fn parse_build_args(args: &[String]) -> Result<BuildArgs, String> {
    let mut file = None;
    let mut output = None;
    let mut target = Target::host();
    let mut optimize = true;
    let mut debug_info = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-o" => output = Some(it.next().ok_or("-o needs a path")?.clone()),
            "--target" => {
                let name = it.next().ok_or("--target needs a target name")?;
                target = Some(Target::parse(name)?);
            }
            "--no-opt" => optimize = false,
            "-g" => debug_info = true,
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other => {
                if file.replace(other.to_string()).is_some() {
                    return Err("only one input file is allowed".to_string());
                }
            }
        }
    }
    Ok(BuildArgs {
        file: file.ok_or("missing input file")?,
        output,
        target: target.ok_or("the host is not a supported target; pass --target")?,
        optimize,
        debug_info,
    })
}

fn build(args: &[String], executable: bool) -> ExitCode {
    let args = match parse_build_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("a0: {e}\n");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut sources = SourceMap::new();
    let options = if args.debug_info {
        a0_hir::LowerOptions::optimized(false)
    } else {
        a0_hir::LowerOptions::default()
    };
    let module = match load_with(&args.file, &mut sources, options) {
        Ok(m) => m,
        Err(c) => return c,
    };
    let stem = std::path::Path::new(&args.file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "a".to_string());
    let result = if executable {
        let output = args
            .output
            .unwrap_or_else(|| format!("{stem}{}", args.target.exe_suffix()));
        let opts = a0_native::BuildOptions {
            no_optimize: !args.optimize,
            debug_info: args.debug_info,
            ..Default::default()
        };
        a0_native::build_executable(&module, args.target, std::path::Path::new(&output), &opts)
    } else {
        let output = args
            .output
            .unwrap_or_else(|| format!("{stem}{}", args.target.object_suffix()));
        let opts = a0_native::CodegenOptions {
            optimize: args.optimize,
            entry_point: false,
            debug_info: args.debug_info,
        };
        a0_native::compile_object(&module, args.target, &opts).and_then(|bytes| {
            std::fs::write(&output, bytes).map_err(|e| {
                a0_diag::Diagnostic::error("io", &format!("cannot write {output}: {e}")).into()
            })
        })
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(d) => report(&sources, &d),
    }
}

fn parse_with_grammar(args: &[String]) -> ExitCode {
    let [grammar_path, file] = args else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let read = |path: &str| {
        std::fs::read_to_string(path).map_err(|e| {
            eprintln!("a0: cannot read {path}: {e}");
            ExitCode::from(2)
        })
    };
    let (grammar_text, text) = match (read(grammar_path), read(file)) {
        (Ok(g), Ok(t)) => (g, t),
        (Err(c), _) | (_, Err(c)) => return c,
    };
    let mut sources = SourceMap::new();
    let gid = sources.add(grammar_path, &grammar_text);
    let grammar = match a0_syntax::Grammar::from_str(&grammar_text, gid) {
        Ok(g) => g,
        Err(d) => return report(&sources, &d),
    };
    let id = sources.add(file, &text);
    let (tree, diags) = grammar.parse(&text, id);
    println!("{}", tree.to_sexpr(tree.root()));
    if diags.is_empty() {
        ExitCode::SUCCESS
    } else {
        report(&sources, &diags)
    }
}
