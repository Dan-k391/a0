//! The Debug Adapter Protocol (`lang dap`): a debugger for every language, from the interpreter.
//!
//! The program runs in this process, in the interpreter, which tells the debugger of every new
//! source line and of a failure before the program's calls unwind (`Interpreter::new_debug`).
//! Breakpoints and steps decide where it stops; while it is stopped, the stack, its locals and
//! their values (read by the debug types lowering records, `values.rs`) are shown. The program's
//! output becomes `output` events and its standard input is empty (`a0rt.h`, "Debuggers"), since
//! requests come on standard input. Programs are compiled without the HIR optimizer, so that each
//! function keeps its frame.

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use a0_diag::{RenderOptions, SourceId, SourceMap};
use a0_interp::{DebugEvent, Interpreter};

use crate::json::Json;
use crate::lsp::read_message;
use crate::values::{self, View};
use crate::{CompileError, Language, Program};

/// The session: the hook the interpreter calls is a plain function, so it finds it here.
struct Session {
    out: Mutex<Box<dyn Write + Send>>,
    seq: AtomicI64,
    /// Lines with breakpoints, per file (as `file_key` names it).
    breakpoints: Mutex<HashMap<String, BTreeSet<u32>>>,
    pause: AtomicBool,
    stop_on_entry: AtomicBool,
    /// Requests that need the stopped program (stack, variables, steps), for its thread.
    requests: Mutex<Option<Receiver<Json>>>,
    step: Mutex<Step>,
    /// The last line reached: its depth, file and line.
    last: Mutex<Option<(usize, String, u32)>>,
    sources: OnceLock<SourceMap>,
    /// Set when the client disconnects, which ends a program that already ended.
    disconnected: (Mutex<bool>, Condvar),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    None,
    In,
    /// Over the line at this depth: the next line at it or above.
    Over(usize),
    /// Out of the call at this depth.
    Out(usize),
}

static SESSION: OnceLock<Session> = OnceLock::new();

fn session() -> &'static Session {
    SESSION.get().expect("the debug session is set up first")
}

/// Serves the Debug Adapter Protocol on `input` and `output` for one program. The process exits
/// when the session ends.
pub fn serve(
    lang: &Language,
    mut input: Box<dyn BufRead + Send>,
    out: Box<dyn Write + Send>,
) -> u8 {
    let (to_program, requests) = channel();
    let fresh = Session {
        out: Mutex::new(out),
        seq: AtomicI64::new(1),
        breakpoints: Mutex::new(HashMap::new()),
        pause: AtomicBool::new(false),
        stop_on_entry: AtomicBool::new(false),
        requests: Mutex::new(Some(requests)),
        step: Mutex::new(Step::None),
        last: Mutex::new(None),
        sources: OnceLock::new(),
        disconnected: (Mutex::new(false), Condvar::new()),
    };
    if SESSION.set(fresh).is_err() {
        return 1;
    }
    let mut program: Option<Prepared> = None;
    let mut configured = false;
    let mut started = false;
    while let Ok(Some(text)) = read_message(&mut input) {
        let Ok(request) = Json::parse(&text) else {
            continue;
        };
        if request.get("type").as_str() != Some("request") {
            continue;
        }
        let command = request.get("command").as_str().unwrap_or("").to_string();
        let args = request.get("arguments");
        match command.as_str() {
            "initialize" => {
                respond(
                    &request,
                    Json::object([
                        ("supportsConfigurationDoneRequest", true.into()),
                        ("supportsEvaluateForHovers", true.into()),
                        ("supportsTerminateRequest", true.into()),
                    ]),
                );
                event("initialized", Json::object([]));
            }
            "launch" => match prepare(lang, args) {
                Ok(p) => {
                    session().stop_on_entry.store(
                        args.get("stopOnEntry") == &Json::Bool(true),
                        Ordering::SeqCst,
                    );
                    program = Some(p);
                    respond(&request, Json::object([]));
                }
                Err(message) => {
                    output("stderr", &message);
                    fail(&request, "the program has errors");
                    event("terminated", Json::object([]));
                }
            },
            "setBreakpoints" => {
                let path = args.get("source").get("path").as_str().unwrap_or("");
                let lines: BTreeSet<u32> = args
                    .get("breakpoints")
                    .as_array()
                    .iter()
                    .filter_map(|b| b.get("line").as_u32())
                    .collect();
                let verified: Vec<Json> = lines
                    .iter()
                    .map(|&line| Json::object([("verified", true.into()), ("line", line.into())]))
                    .collect();
                session()
                    .breakpoints
                    .lock()
                    .unwrap()
                    .insert(file_key(path), lines);
                respond(&request, Json::object([("breakpoints", verified.into())]));
            }
            "setExceptionBreakpoints" => respond(&request, Json::object([])),
            "configurationDone" => {
                configured = true;
                respond(&request, Json::object([]));
            }
            "threads" => respond(
                &request,
                Json::object([(
                    "threads",
                    vec![Json::object([("id", 1u32.into()), ("name", "main".into())])].into(),
                )]),
            ),
            "pause" => {
                session().pause.store(true, Ordering::SeqCst);
                respond(&request, Json::object([]));
            }
            "disconnect" | "terminate" => {
                // Ending the session ends the program, which runs in this process.
                respond(&request, Json::object([]));
                if command == "terminate" {
                    event("terminated", Json::object([]));
                }
                let (done, signal) = &session().disconnected;
                *done.lock().unwrap() = true;
                signal.notify_all();
                flush_and_exit(0);
            }
            _ if started => {
                let _ = to_program.send(request);
            }
            _ => fail(&request, "the program is not running"),
        }
        if configured
            && !started
            && let Some(p) = program.take()
        {
            started = true;
            start(p);
        }
    }
    // The client went away.
    flush_and_exit(0)
}

/// A program compiled for debugging.
struct Prepared {
    module: a0_ir::Module,
    symbols: a0_interp::HostSymbols,
    args: Vec<String>,
}

/// Compiles the program `launch` names, without the HIR optimizer; errors come rendered.
fn prepare(lang: &Language, args: &Json) -> Result<Prepared, String> {
    let path = args.get("program").as_str().unwrap_or("").to_string();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("{}: cannot read {path}: {e}\n", lang.name))?;
    let mut sources = SourceMap::new();
    let compiled = match lang.modules {
        Some(modules) => lang.compile_program(&modules, &path, &text, &mut sources),
        None => {
            let id: SourceId = sources.add(&path, &text);
            (lang.compile)(&text, id)
        }
    };
    let render = |d: &a0_diag::Diagnostics, sources: &SourceMap| {
        lang.codes
            .apply(d)
            .render_with(sources, &RenderOptions::default())
    };
    let mut module = match compiled {
        Ok(Program::Ir(m)) => m,
        Ok(Program::Hir(m)) => {
            let options = a0_hir::LowerOptions { optimize: false };
            a0_hir::lower_with(&m, options).map_err(|d| render(&d, &sources))?
        }
        Err(CompileError::Diagnostics(d)) => return Err(render(&d, &sources)),
        Err(CompileError::Messages(messages)) => {
            return Err(messages.iter().map(|m| format!("{}\n", m.text)).collect());
        }
    };
    module.set_sources(sources.clone());
    let mut symbols = match lang.host_symbols {
        Some(f) => f(),
        None => a0_interp::HostSymbols::new(),
    };
    a0_native::add_thunks(&module, &mut symbols).map_err(|d| render(&d, &sources))?;
    let _ = session().sources.set(sources);
    let mut argv = vec![path];
    argv.extend(
        args.get("args")
            .as_array()
            .iter()
            .filter_map(|a| a.as_str().map(str::to_string)),
    );
    Ok(Prepared {
        module,
        symbols,
        args: argv,
    })
}

/// Runs the program on a thread of its own with a large stack, as `a0_interp::run_main` does.
fn start(p: Prepared) {
    a0_runtime::set_debugger(
        Some(program_output),
        Some(a0_interp::runtime_failed),
        Some(program_exit),
    );
    std::thread::Builder::new()
        .name("a0-debuggee".to_string())
        .stack_size(1 << 30)
        .spawn(move || {
            let mut interp = match Interpreter::new_debug(p.module, &p.symbols, on_event) {
                Ok(i) => i,
                Err(d) => {
                    let text = d.render_with(
                        session().sources.get().unwrap_or(&SourceMap::new()),
                        &RenderOptions::default(),
                    );
                    output("stderr", &text);
                    a0_runtime::exit(1);
                }
            };
            match interp.run_main(&p.args) {
                Ok(code) => a0_runtime::exit(code),
                Err(trap) => trap.report_and_exit(),
            }
        })
        .expect("cannot start the program's thread");
}

extern "C" fn program_output(stream: i32, bytes: *const u8, len: usize) {
    // SAFETY: the runtime passes the bytes it writes.
    let text = unsafe { std::slice::from_raw_parts(bytes, len) };
    let category = if stream == 2 { "stderr" } else { "stdout" };
    output(category, &String::from_utf8_lossy(text));
}

/// The program ends: the client hears of it, and the process ends once it disconnects (or soon).
extern "C" fn program_exit(code: i32) {
    event(
        "exited",
        Json::object([("exitCode", Json::Number(code as f64))]),
    );
    event("terminated", Json::object([]));
    let (done, signal) = &session().disconnected;
    let guard = done.lock().unwrap();
    let _ = signal.wait_timeout_while(guard, Duration::from_secs(5), |d| !*d);
    let _ = session().out.lock().unwrap().flush();
}

/// What the interpreter tells the debugger, on the program's thread.
fn on_event(interp: &Interpreter, e: DebugEvent) {
    let s = session();
    if e == DebugEvent::Failure {
        let text = interp.debug_failure().unwrap_or_default();
        stop(interp, "exception", Some(&text));
        return;
    }
    let Some(here) = interp
        .debug_location(interp.debug_frame_location(0))
        .cloned()
    else {
        return;
    };
    let depth = interp.debug_frame_count();
    let file = file_key(&here.file);
    // A line reached again without leaving it (a nested block on the same line) is not new.
    let new = {
        let mut last = s.last.lock().unwrap();
        let key = (depth, file.clone(), here.line);
        let new = last.as_ref() != Some(&key);
        *last = Some(key);
        new
    };
    if !new {
        return;
    }
    let reason = if s.pause.swap(false, Ordering::SeqCst) {
        Some("pause")
    } else if s.stop_on_entry.swap(false, Ordering::SeqCst) {
        Some("entry")
    } else if s
        .breakpoints
        .lock()
        .unwrap()
        .get(&file)
        .is_some_and(|lines| lines.contains(&here.line))
    {
        Some("breakpoint")
    } else {
        match *s.step.lock().unwrap() {
            Step::None => None,
            Step::In => Some("step"),
            Step::Over(d) if depth <= d => Some("step"),
            Step::Out(d) if depth < d => Some("step"),
            _ => None,
        }
    };
    if let Some(reason) = reason {
        stop(interp, reason, None);
    }
}

/// What a variable reference stands for while the program is stopped.
enum Target {
    Frame(usize),
    Value(u64, View),
}

/// Stops the program: answers requests about it until one resumes it.
fn stop(interp: &Interpreter, reason: &str, text: Option<&str>) {
    let s = session();
    *s.step.lock().unwrap() = Step::None;
    let mut body = vec![
        ("reason", reason.into()),
        ("threadId", 1u32.into()),
        ("allThreadsStopped", true.into()),
    ];
    if let Some(text) = text {
        body.push(("text", text.into()));
        body.push(("description", format!("panic: {text}").into()));
    }
    event("stopped", object(body));
    let mut targets: Vec<Target> = Vec::new();
    loop {
        let request = {
            let requests = s.requests.lock().unwrap();
            match requests.as_ref().map(Receiver::recv) {
                Some(Ok(r)) => r,
                _ => flush_and_exit(0),
            }
        };
        let args = request.get("arguments");
        let depth = interp.debug_frame_count();
        match request.get("command").as_str().unwrap_or("") {
            "stackTrace" => {
                // Calls of generated code (the entry that sets the program up) have no lines.
                let frames: Vec<Json> = (0..depth)
                    .filter(|&i| i == 0 || interp.debug_frame_location(i) != 0)
                    .map(|i| stack_frame(interp, i))
                    .collect();
                let depth = frames.len();
                respond(
                    &request,
                    Json::object([
                        ("stackFrames", frames.into()),
                        ("totalFrames", (depth as u32).into()),
                    ]),
                );
            }
            "scopes" => {
                let frame = frame_index(args, depth);
                targets.push(Target::Frame(frame));
                let reference = targets.len() as u32;
                let scope = Json::object([
                    ("name", "Locals".into()),
                    ("presentationHint", "locals".into()),
                    ("variablesReference", reference.into()),
                    ("expensive", false.into()),
                ]);
                respond(&request, Json::object([("scopes", vec![scope].into())]));
            }
            "variables" => {
                let reference = args.get("variablesReference").as_u32().unwrap_or(0) as usize;
                let items: Vec<(String, u64, View)> =
                    match reference.checked_sub(1).and_then(|i| targets.get(i)) {
                        Some(&Target::Frame(i)) => locals(interp, i),
                        Some(&Target::Value(w, view)) => values::parts(interp.module(), w, view),
                        None => Vec::new(),
                    };
                let variables: Vec<Json> = items
                    .into_iter()
                    .map(|(name, w, view)| variable(interp, &mut targets, &name, w, view))
                    .collect();
                respond(&request, Json::object([("variables", variables.into())]));
            }
            "evaluate" => {
                let name = args.get("expression").as_str().unwrap_or("").trim();
                let frame = frame_index(args, depth);
                match locals(interp, frame)
                    .into_iter()
                    .find(|(n, _, _)| n == name)
                {
                    Some((_, w, view)) => {
                        let v = variable(interp, &mut targets, name, w, view);
                        respond(
                            &request,
                            Json::object([
                                ("result", v.get("value").clone()),
                                ("type", v.get("type").clone()),
                                ("variablesReference", v.get("variablesReference").clone()),
                            ]),
                        );
                    }
                    None => fail(&request, &format!("no variable `{name}` here")),
                }
            }
            command @ ("continue" | "next" | "stepIn" | "stepOut") => {
                *s.step.lock().unwrap() = match command {
                    "next" => Step::Over(depth),
                    "stepIn" => Step::In,
                    "stepOut" => Step::Out(depth),
                    _ => Step::None,
                };
                let body = if command == "continue" {
                    Json::object([("allThreadsContinued", true.into())])
                } else {
                    Json::object([])
                };
                respond(&request, body);
                return;
            }
            _ => fail(&request, "not supported"),
        }
    }
}

fn frame_index(args: &Json, depth: usize) -> usize {
    let id = args.get("frameId").as_u32().unwrap_or(1) as usize;
    id.saturating_sub(1).min(depth.saturating_sub(1))
}

fn stack_frame(interp: &Interpreter, i: usize) -> Json {
    let m = interp.module();
    let f = interp.debug_frame_function(i);
    let fallback = m.func_trace_name(f).unwrap_or(m.func_name(f)).to_string();
    let mut frame = vec![("id", (i as u32 + 1).into())];
    match interp.debug_location(interp.debug_frame_location(i)) {
        Some(l) if l.line != 0 => {
            frame.push(("name", l.func.as_str().into()));
            frame.push(("line", l.line.into()));
            frame.push(("column", l.col.into()));
            let name = Path::new(&l.file)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // The standard library's code is shown from a copy the editor can open.
            let builtin = SourceMap::builtin_named(&l.file)
                .and_then(|id| crate::lsp::builtin_file(&SourceMap::new(), id));
            let path = match builtin {
                Some(p) => p.to_string_lossy().into_owned(),
                None => std::path::absolute(&l.file)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| l.file.clone()),
            };
            frame.push((
                "source",
                Json::object([("name", name.into()), ("path", path.into())]),
            ));
        }
        _ => {
            frame.push(("name", fallback.into()));
            frame.push(("line", 0u32.into()));
            frame.push(("column", 0u32.into()));
        }
    }
    object(frame)
}

/// The locals of frame `i` that are declared by now: by their debug information, or every named
/// variable of a module that has none (core IR from a frontend). Of locals with the same name, the
/// latest declared wins.
fn locals(interp: &Interpreter, i: usize) -> Vec<(String, u64, View)> {
    let m = interp.module();
    let here = interp
        .debug_location(interp.debug_frame_location(i))
        .cloned();
    let described = m.debug_type_count() > 0;
    let sources = session().sources.get();
    let mut out: Vec<(String, u64, View, (u32, u32))> = Vec::new();
    for (var, w) in interp.debug_frame_vars(i) {
        let (name, view, declared) = match m.var_debug(var) {
            Some((name, t)) => {
                let declared = m.var_debug_span(var).and_then(|span| {
                    let sources = sources?;
                    (sources.name(span.source) == here.as_ref()?.file.as_str())
                        .then(|| sources.line_col(span.source, span.start))
                });
                (name.to_string(), View::Typed(t), declared)
            }
            None if !described => match m.var_name(var) {
                Some(name) => (name.to_string(), View::Raw(m.var_type(var)), None),
                None => continue,
            },
            None => continue,
        };
        // A narrow signed integer is held zero-extended; show its sign.
        let w = match (view, m.var_type(var).bits()) {
            (View::Typed(t), Some(bits))
                if bits < 64 && m.debug_type_kind(t) == a0_ir::DebugKind::Int =>
            {
                (((w << (64 - bits)) as i64) >> (64 - bits)) as u64
            }
            _ => w,
        };
        let position = declared.unwrap_or((0, 0));
        if let Some(h) = &here
            && declared.is_some()
            && position > (h.line, h.col)
        {
            continue;
        }
        match out.iter_mut().find(|(n, ..)| *n == name) {
            Some(existing) if existing.3 <= position => *existing = (name, w, view, position),
            Some(_) => {}
            None => out.push((name, w, view, position)),
        }
    }
    out.into_iter().map(|(n, w, v, _)| (n, w, v)).collect()
}

fn variable(
    interp: &Interpreter,
    targets: &mut Vec<Target>,
    name: &str,
    w: u64,
    view: View,
) -> Json {
    let m = interp.module();
    let reference = if values::has_parts(m, w, view) {
        targets.push(Target::Value(w, view));
        targets.len() as u32
    } else {
        0
    };
    Json::object([
        ("name", name.into()),
        ("value", values::show(m, w, view).into()),
        ("type", values::type_name(m, w, view).into()),
        ("variablesReference", reference.into()),
    ])
}

/// How files are compared: the same file however its path is written.
fn file_key(path: &str) -> String {
    let p = std::fs::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf());
    let key = p.to_string_lossy().into_owned();
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

fn object(members: Vec<(&str, Json)>) -> Json {
    Json::Object(
        members
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn send(mut members: Vec<(&str, Json)>) {
    let s = session();
    let seq = s.seq.fetch_add(1, Ordering::SeqCst);
    members.insert(0, ("seq", Json::Number(seq as f64)));
    let body = object(members).to_string();
    let mut out = s.out.lock().unwrap();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

fn respond(request: &Json, body: Json) {
    send(vec![
        ("type", "response".into()),
        ("request_seq", request.get("seq").clone()),
        ("success", true.into()),
        ("command", request.get("command").clone()),
        ("body", body),
    ]);
}

fn fail(request: &Json, message: &str) {
    send(vec![
        ("type", "response".into()),
        ("request_seq", request.get("seq").clone()),
        ("success", false.into()),
        ("command", request.get("command").clone()),
        ("message", message.into()),
    ]);
}

fn event(name: &str, body: Json) {
    send(vec![
        ("type", "event".into()),
        ("event", name.into()),
        ("body", body),
    ]);
}

fn output(category: &str, text: &str) {
    event(
        "output",
        Json::object([("category", category.into()), ("output", text.into())]),
    );
}

fn flush_and_exit(code: i32) -> ! {
    if let Some(s) = SESSION.get() {
        let _ = s.out.lock().unwrap().flush();
    }
    std::process::exit(code)
}

/// A client of a debug adapter (`server dap`), for languages' tests: sends requests, waits for
/// responses and events, and keeps everything the adapter said.
pub struct Client {
    child: std::process::Child,
    input: std::process::ChildStdin,
    messages: Receiver<Json>,
    seq: u32,
    /// Messages received but not yet waited for.
    pending: std::collections::VecDeque<Json>,
    /// Every message received so far.
    pub seen: Vec<Json>,
}

impl Client {
    /// Starts `server` with `args` (`["dap"]`, or `["lang", "calc.a0g", "dap"]`).
    pub fn start(server: &Path, args: &[&str]) -> std::io::Result<Client> {
        let mut child = std::process::Command::new(server)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let input = child.stdin.take().expect("piped");
        let mut out = std::io::BufReader::new(child.stdout.take().expect("piped"));
        let (tx, messages) = channel();
        std::thread::spawn(move || {
            while let Ok(Some(text)) = read_message(&mut out) {
                if let Ok(m) = Json::parse(&text)
                    && tx.send(m).is_err()
                {
                    break;
                }
            }
        });
        Ok(Client {
            child,
            input,
            messages,
            seq: 0,
            pending: std::collections::VecDeque::new(),
            seen: Vec::new(),
        })
    }

    /// Sends a request and returns its response (panics after a minute without one).
    pub fn request(&mut self, command: &str, arguments: Json) -> Json {
        self.seq += 1;
        let seq = self.seq;
        let message = object(vec![
            ("seq", seq.into()),
            ("type", "request".into()),
            ("command", command.into()),
            ("arguments", arguments),
        ]);
        let _ = self.input.write_all(&crate::lsp::frame(&message));
        let _ = self.input.flush();
        self.wait(|m| {
            m.get("type").as_str() == Some("response") && m.get("request_seq").as_u32() == Some(seq)
        })
    }

    /// Starts debugging `program` with breakpoints on `lines`: initializes, sets the breakpoints,
    /// launches and finishes the configuration. Returns the response to `launch`.
    pub fn launch(&mut self, program: &str, lines: &[u32]) -> Json {
        self.request("initialize", Json::object([("adapterID", "a0".into())]));
        self.event("initialized");
        let breakpoints: Vec<Json> = lines
            .iter()
            .map(|&l| Json::object([("line", l.into())]))
            .collect();
        self.request(
            "setBreakpoints",
            Json::object([
                ("source", Json::object([("path", program.into())])),
                ("breakpoints", breakpoints.into()),
            ]),
        );
        let launch = self.request("launch", Json::object([("program", program.into())]));
        self.request("configurationDone", Json::object([]));
        launch
    }

    /// Resumes the stopped program with `continue`, `next`, `stepIn` or `stepOut`.
    pub fn resume(&mut self, command: &str) {
        self.request(command, Json::object([("threadId", 1u32.into())]));
    }

    /// The stack of the stopped program, innermost first: each frame's name and line.
    pub fn stack(&mut self) -> Vec<(String, u32)> {
        let r = self.request("stackTrace", Json::object([("threadId", 1u32.into())]));
        r.get("body")
            .get("stackFrames")
            .as_array()
            .iter()
            .map(|f| {
                let name = f.get("name").as_str().unwrap_or("").to_string();
                (name, f.get("line").as_u32().unwrap_or(0))
            })
            .collect()
    }

    /// The locals of frame `frame` (1 is the innermost): name, value and the reference of its
    /// parts (0 if it has none).
    pub fn locals(&mut self, frame: u32) -> Vec<(String, String, u32)> {
        let scopes = self.request("scopes", Json::object([("frameId", frame.into())]));
        let reference = scopes.get("body").get("scopes").as_array()[0]
            .get("variablesReference")
            .as_u32()
            .unwrap_or(0);
        self.variables(reference)
    }

    /// The parts of a value (or the locals of a scope) by reference.
    pub fn variables(&mut self, reference: u32) -> Vec<(String, String, u32)> {
        let r = self.request(
            "variables",
            Json::object([("variablesReference", reference.into())]),
        );
        r.get("body")
            .get("variables")
            .as_array()
            .iter()
            .map(|v| {
                (
                    v.get("name").as_str().unwrap_or("").to_string(),
                    v.get("value").as_str().unwrap_or("").to_string(),
                    v.get("variablesReference").as_u32().unwrap_or(0),
                )
            })
            .collect()
    }

    /// Waits for the next event called `name` that has not been waited for yet.
    pub fn event(&mut self, name: &str) -> Json {
        self.wait(|m| {
            m.get("type").as_str() == Some("event") && m.get("event").as_str() == Some(name)
        })
    }

    /// Everything the program printed so far (`output` events in category `category`).
    pub fn output(&self, category: &str) -> String {
        self.seen
            .iter()
            .filter(|m| {
                m.get("event").as_str() == Some("output")
                    && m.get("body").get("category").as_str() == Some(category)
            })
            .filter_map(|m| m.get("body").get("output").as_str())
            .collect()
    }

    fn wait(&mut self, wanted: impl Fn(&Json) -> bool) -> Json {
        // Events can come before the response to the request that caused them.
        if let Some(i) = self.pending.iter().position(&wanted) {
            return self.pending.remove(i).expect("found");
        }
        loop {
            match self.messages.recv_timeout(Duration::from_secs(60)) {
                Ok(m) => {
                    self.seen.push(m.clone());
                    if wanted(&m) {
                        return m;
                    }
                    self.pending.push_back(m);
                }
                Err(_) => panic!("the adapter did not answer; it said:\n{:#?}", self.seen),
            }
        }
    }

    /// Disconnects and waits for the adapter to end.
    pub fn finish(mut self) {
        self.seq += 1;
        let message = object(vec![
            ("seq", self.seq.into()),
            ("type", "request".into()),
            ("command", "disconnect".into()),
            ("arguments", Json::object([])),
        ]);
        let _ = self.input.write_all(&crate::lsp::frame(&message));
        let _ = self.input.flush();
        let _ = self.child.wait();
    }
}
