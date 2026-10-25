//! `lang repl`: a read-eval-print loop for every language (D61).
//!
//! The inputs so far make a program: in the order they were typed, or, for a language whose
//! statements live in `main` (`Language::repl_main`), its declarations followed by `main` with the
//! statements. Each new input is added, checked, and the whole program runs again in a process of
//! its own (`lang run -`; the runtime keeps global state, so a process runs one program); what it
//! prints beyond what the program printed before is shown. An input that does not compile, or that
//! makes the program fail, is dropped, so the program always runs. An input that ends early (an
//! open `{`) waits for more lines; an empty line submits it as it is.

use std::io::{BufRead, IsTerminal, Read, Write};
use std::process::{Command, Stdio};

use a0_diag::{RenderOptions, SourceId, SourceMap};
use a0_syntax::Grammar;

use crate::{CompileError, Language};

const HELP: &str = "\
Type declarations and statements of the language: each runs together with the ones before it,
and what it prints is shown. A line that leaves something open continues on the next line.
  :show   print the program so far
  :reset  start over
  :quit   leave (so does the end of the input)
";

/// The program the inputs make.
#[derive(Default)]
struct Session {
    /// Each input, and whether it is a declaration (only told apart with `repl_main`).
    inputs: Vec<(String, bool)>,
    /// What the program printed when it last ran.
    out: Vec<u8>,
    err: Vec<u8>,
}

impl Session {
    /// The program, with `extra` added.
    fn text(&self, lang: &Language, extra: Option<(&str, bool)>) -> String {
        let inputs = self
            .inputs
            .iter()
            .map(|(text, decl)| (text.as_str(), *decl))
            .chain(extra);
        match lang.repl_main {
            Some(main) => {
                let (declarations, statements): (Vec<_>, Vec<_>) = inputs.partition(|i| i.1);
                let mut text: String = declarations.iter().map(|i| i.0).collect();
                let body: String = statements.iter().map(|i| i.0).collect();
                text.push_str(&main.replace("{}", &body));
                text
            }
            None => inputs.map(|i| i.0).collect(),
        }
    }
}

/// Runs the loop on standard input and output; returns the exit status.
pub(crate) fn repl(lang: &Language) -> u8 {
    let interactive = std::io::stdin().is_terminal();
    if interactive {
        println!(
            "{} {} (A0): :help for help, :quit to leave",
            lang.name,
            env!("CARGO_PKG_VERSION")
        );
    }
    let grammar = lang
        .grammar
        .and_then(|g| Grammar::from_str(g, SourceId(u32::MAX)).ok());
    let mut session = Session::default();
    let mut pending = String::new();
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        if interactive {
            print!("{}", if pending.is_empty() { "> " } else { ". " });
            let _ = std::io::stdout().flush();
        }
        let Some(Ok(line)) = lines.next() else {
            break;
        };
        if pending.is_empty() {
            match line.trim() {
                ":quit" | ":q" | ":exit" => break,
                ":help" => {
                    print!("{HELP}");
                    continue;
                }
                ":show" => {
                    let text = session.text(lang, None);
                    for (i, l) in text.lines().enumerate() {
                        println!("{:4} | {l}", i + 1);
                    }
                    continue;
                }
                ":reset" => {
                    session = Session::default();
                    continue;
                }
                "" => continue,
                _ => {}
            }
        }
        pending.push_str(&line);
        pending.push('\n');
        // With `repl_main`, an input that the language takes at the top level, after the
        // declarations so far, is a declaration. Either reading may still be unfinished.
        let mut declarations: String = session
            .inputs
            .iter()
            .filter(|i| i.1)
            .map(|i| i.0.as_str())
            .collect();
        declarations.push_str(&pending);
        let as_statement = session.text(lang, Some((&pending, false)));
        if let Some(g) = &grammar {
            let unfinished = ends_early(g, &as_statement)
                || (lang.repl_main.is_some() && ends_early(g, &declarations));
            if unfinished && !line.trim().is_empty() {
                continue;
            }
        }
        let declaration = lang.repl_main.is_some()
            && grammar
                .as_ref()
                .is_some_and(|g| !g.parse(&declarations, SourceId(0)).1.has_errors());
        let text = session.text(lang, Some((&pending, declaration)));
        let input = std::mem::take(&mut pending);
        if let Err(errors) = check(lang, &text) {
            eprint!("{errors}");
            continue;
        }
        let Some((ok, out, err)) = run(&text) else {
            eprintln!("{}: cannot run the program", lang.name);
            continue;
        };
        show(&session.out, &out, &mut std::io::stdout());
        show(&session.err, &err, &mut std::io::stderr());
        if ok {
            session.inputs.push((input, declaration));
            session.out = out;
            session.err = err;
        }
    }
    if interactive {
        println!();
    }
    0
}

/// Whether the only syntax errors of `text` are at its end: an input that is not finished yet.
fn ends_early(g: &Grammar, text: &str) -> bool {
    let (_, diags) = g.parse(text, SourceId(0));
    let end = text.trim_end().len() as u32;
    diags.has_errors()
        && diags
            .as_slice()
            .iter()
            .all(|d| d.span.is_some_and(|s| s.start >= end))
}

/// Compiles the program, as `lang run` would; errors come rendered.
fn check(lang: &Language, text: &str) -> Result<(), String> {
    let mut sources = SourceMap::new();
    let compiled = match lang.modules {
        Some(modules) => lang.compile_program(&modules, "<repl>", text, &mut sources),
        None => {
            let id = sources.add("<repl>", text);
            (lang.compile)(text, id)
        }
    };
    match compiled {
        Ok(_) => Ok(()),
        Err(CompileError::Diagnostics(d)) => Err(lang
            .codes
            .apply(&d)
            .render_with(&sources, &RenderOptions::stderr())),
        Err(CompileError::Messages(ms)) => {
            Err(ms.iter().map(|m| format!("{}\n", m.text)).collect())
        }
    }
}

/// Runs the program in a process of its own: whether it succeeded, and what it printed.
fn run(text: &str) -> Option<(bool, Vec<u8>, Vec<u8>)> {
    let exe = std::env::current_exe().ok()?;
    // The command this loop runs in, with `run -` for `repl` (`a0 lang calc.a0g repl` runs
    // `a0 lang calc.a0g run -`).
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.last().is_some_and(|a| a == "repl") {
        args.pop();
    }
    let mut child = Command::new(exe)
        .args(&args)
        .args(["run", "-"])
        .env("A0_REPL", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let text = text.to_string();
    let writer = std::thread::spawn(move || stdin.write_all(text.as_bytes()));
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let errors = std::thread::spawn(move || {
        let mut err = Vec::new();
        let _ = stderr.read_to_end(&mut err);
        err
    });
    let _ = stdout.read_to_end(&mut out);
    err.extend(errors.join().ok()?);
    let _ = writer.join();
    let status = child.wait().ok()?;
    Some((status.success(), out, err))
}

/// Writes what `now` adds to `before`; all of it if the program printed something else this time.
fn show(before: &[u8], now: &[u8], to: &mut dyn Write) {
    let new = now.strip_prefix(before).unwrap_or(now);
    let _ = to.write_all(new);
    let _ = to.flush();
}
