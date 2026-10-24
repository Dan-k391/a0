//! `kaleidoscope`: run or compile Kaleidoscope programs.

use std::path::Path;
use std::process::ExitCode;

use a0_diag::SourceMap;

const USAGE: &str = "usage: kaleidoscope <command> <file.k> [-o output]

commands:
  run     interpret the program
  build   compile to a native executable
  ir      print the A0 IR
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, file, output) = match args.as_slice() {
        [c, f] => (c.as_str(), f.as_str(), None),
        [c, f, o, out] if o == "-o" => (c.as_str(), f.as_str(), Some(out.as_str())),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("kaleidoscope: cannot read {file}: {e}");
            return ExitCode::from(2);
        }
    };
    let mut sources = SourceMap::new();
    let id = sources.add(file, &text);
    let module = match kaleidoscope::compile(&text, id) {
        Ok(m) => m,
        Err(diags) => {
            eprint!("{}", diags.render(&sources));
            return ExitCode::from(1);
        }
    };
    match command {
        "ir" => {
            print!("{module}");
            ExitCode::SUCCESS
        }
        "run" => {
            let args = vec![file.to_string()];
            match a0_interp::run_main(module, kaleidoscope::host_symbols(), args) {
                Err(diags) => {
                    eprint!("{}", diags.render(&sources));
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
            let mut opts = a0_native::BuildOptions::default();
            opts.link.libraries.push(kaleidoscope::runtime_library());
            match a0_native::build_executable(&module, target, Path::new(&output), &opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(diags) => {
                    eprint!("{}", diags.render(&sources));
                    ExitCode::from(1)
                }
            }
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
