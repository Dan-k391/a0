//! `a0`: work with A0 IR files from the command line.

use std::process::ExitCode;

use a0_diag::{Diagnostics, SourceMap};
use a0_ir::Module;

const USAGE: &str = "usage: a0 <command> [options] <file.a0ir>

commands:
  check <file>              parse and verify; print diagnostics
  fmt <file>                print the canonical text form
  run <file> [-- args...]   interpret @main and exit with its status
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    match command.as_str() {
        "check" => check(rest),
        "fmt" => fmt(rest),
        "run" => run(rest),
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

/// Reads, parses and verifies an IR file, printing diagnostics on failure.
fn load(path: &str, sources: &mut SourceMap) -> Result<Module, ExitCode> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("a0: cannot read {path}: {e}");
            return Err(ExitCode::from(2));
        }
    };
    let id = sources.add(path, &text);
    a0_ir::parse_and_verify(&text, id).map_err(|d| report(sources, &d))
}

fn report(sources: &SourceMap, diags: &Diagnostics) -> ExitCode {
    eprint!("{}", diags.render(sources));
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
    match a0_interp::run_main(module, a0_interp::HostSymbols::new(), argv) {
        Err(diags) => report(&sources, &diags),
        Ok(Err(trap)) => trap.report_and_exit(),
        Ok(Ok(code)) => a0_runtime::exit(code),
    }
}
