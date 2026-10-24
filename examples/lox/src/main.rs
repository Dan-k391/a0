//! `lox`: run or compile Lox programs, with jlox's command line behavior.
//!
//! `lox file.lox` (or `lox run file.lox`) interprets the program; `lox build file.lox [-o out]` makes a
//! native executable; `lox hir file.lox` prints the HIR and `lox ir file.lox` the core IR. Compile
//! errors exit with 65 and runtime errors with 70, as in jlox.

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: lox [run] <file.lox>
       lox build <file.lox> [-o output]
       lox hir <file.lox>
       lox ir <file.lox>
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, file, output) = match args.as_slice() {
        [f] => ("run", f.as_str(), None),
        [c, f] => (c.as_str(), f.as_str(), None),
        [c, f, o, out] if o == "-o" => (c.as_str(), f.as_str(), Some(out.as_str())),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(64);
        }
    };
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("lox: cannot read {file}: {e}");
            return ExitCode::from(66);
        }
    };
    let hir = match lox::compile_hir(&text) {
        Ok(m) => m,
        Err(errors) => {
            for e in errors {
                eprintln!("{e}");
            }
            return ExitCode::from(lox::EXIT_COMPILE_ERROR);
        }
    };
    if command == "hir" {
        print!("{hir}");
        return ExitCode::SUCCESS;
    }
    let module = match a0_hir::lower(&hir) {
        Ok(m) => m,
        Err(diags) => {
            eprint!("{}", diags.summary());
            return ExitCode::from(1);
        }
    };
    match command {
        "run" => {
            let symbols = a0_interp::HostSymbols::new();
            match a0_interp::run_main(module, symbols, vec![file.to_string()]) {
                Err(diags) => {
                    eprint!("{}", diags.summary());
                    ExitCode::from(1)
                }
                Ok(Err(trap)) => trap.report_and_exit(),
                Ok(Ok(code)) => a0_runtime::exit(code),
            }
        }
        "build" => {
            let target = a0_native::Target::host().expect("unsupported host");
            let stem = Path::new(file).file_stem().unwrap().to_string_lossy();
            let output = output
                .map(str::to_string)
                .unwrap_or_else(|| format!("{stem}{}", target.exe_suffix()));
            let opts = a0_native::BuildOptions::default();
            match a0_native::build_executable(&module, target, Path::new(&output), &opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(diags) => {
                    eprint!("{}", diags.summary());
                    ExitCode::from(1)
                }
            }
        }
        "ir" => {
            print!("{module}");
            ExitCode::SUCCESS
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(64)
        }
    }
}
