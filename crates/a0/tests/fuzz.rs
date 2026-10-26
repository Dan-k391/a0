//! Fuzzing: inputs mutated from the corpus must never crash A0 (D84).
//!
//! Each target takes the files A0 is tested with, mutates them (deletes, duplicates and swaps spans
//! and lines, splices in pieces of other files and words from all of them) and feeds them to what
//! reads them: the core IR's parser, verifier and both code generators; the HIR's parser, checker,
//! optimizer and lowering; grammars, the programs written in them and the languages a grammar
//! defines alone (D55). Errors are fine and rendered; a panic, a hang, IR that lowering made and the
//! verifier rejects, an internal code generator error, or text that does not print back the same is
//! a failure. The C API is fuzzed in `crates/a0-capi/tests/fuzz.rs` and manifests in `a0-driver`.
//!
//! `A0_FUZZ_ITERATIONS=n` runs `n` inputs per target (150 by default) and `A0_FUZZ_SEED=s` starts at
//! seed `s`. A failing input is written to the temporary directory and named in the failure.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use a0_diag::{Diagnostics, RenderOptions, SourceMap};

/// xorshift64*, seeded through splitmix64.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rng((z ^ (z >> 31)) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Words and punctuation of the seeds, and values at the edges of what numbers can hold.
fn dictionary(seeds: &[String]) -> Vec<String> {
    let mut words: Vec<String> = [
        "0",
        "-1",
        "9223372036854775807",
        "-9223372036854775808",
        "18446744073709551616",
        "1e308",
        "nan",
        "\"",
        "\\",
        "{",
        "}",
        "(",
        ")",
        "[",
        "]",
        "<",
        ">",
        ";",
        ",",
        ":",
        ".",
        "@",
        "->",
        "é",
        "\u{1F600}",
        "\t",
        "\n",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut seen = std::collections::HashSet::new();
    for s in seeds {
        for w in s.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')) {
            if !w.is_empty() && w.len() < 32 && seen.insert(w) {
                words.push(w.to_string());
            }
        }
    }
    words
}

/// A byte position in `s` at a character boundary.
fn boundary(s: &str, r: &mut Rng) -> usize {
    let mut i = r.below(s.len() + 1);
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A span `[a, b)` of `s` of at most `max` bytes, at character boundaries.
fn span(s: &str, r: &mut Rng, max: usize) -> (usize, usize) {
    let a = boundary(s, r);
    let mut b = (a + 1 + r.below(max)).min(s.len());
    while !s.is_char_boundary(b) {
        b += 1;
    }
    (a, b)
}

/// `seeds[i]` with one mutation or a few.
fn mutate(seeds: &[String], i: usize, words: &[String], r: &mut Rng) -> String {
    let mut s = seeds[i].clone();
    // Mostly one mutation, so that more inputs get past the parser to the stages behind it.
    let mut n = 1;
    while n < 6 && r.below(3) == 0 {
        n += 1;
    }
    for _ in 0..n {
        match r.below(9) {
            0 => {
                let (a, b) = span(&s, r, 16);
                s.replace_range(a..b, "");
            }
            1 => {
                let (a, b) = span(&s, r, 64);
                let piece = s[a..b].to_string();
                let at = boundary(&s, r);
                s.insert_str(at, &piece);
            }
            2 | 3 => {
                let (a, b) = span(&s, r, 8);
                let w = &words[r.below(words.len())];
                s.replace_range(a..b, w);
            }
            4 => {
                let at = boundary(&s, r);
                let w = &words[r.below(words.len())];
                s.insert_str(at, &format!(" {w} "));
            }
            5 => {
                let mut lines: Vec<&str> = s.split('\n').collect();
                let (a, b) = (r.below(lines.len()), r.below(lines.len()));
                lines.swap(a, b);
                s = lines.join("\n");
            }
            6 => {
                let mut lines: Vec<&str> = s.split('\n').collect();
                let a = r.below(lines.len());
                lines.remove(a);
                s = lines.join("\n");
            }
            7 => {
                let other = &seeds[r.below(seeds.len())];
                let (a, b) = span(other, r, 200);
                let at = boundary(&s, r);
                s.insert_str(at, &other[a..b]);
            }
            _ => {
                let at = boundary(&s, r);
                let c = char::from(b' ' + r.below(95) as u8);
                s.insert(at, c);
            }
        }
    }
    s
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The files with one of `extensions` under each of `dirs` (relative to the repository), sorted.
fn corpus(dirs: &[&str], extensions: &[&str]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dir in dirs {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(root().join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| extensions.iter().any(|x| e == *x))
            })
            .collect();
        entries.sort();
        for p in entries {
            let text = std::fs::read_to_string(&p).unwrap();
            out.push((p.file_name().unwrap().to_string_lossy().into_owned(), text));
        }
    }
    out
}

/// How many inputs got how far, by stage: whether mutations still reach the later stages.
static REACHED: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
/// Stages as each target counts them: IR is parsed, verified and compiled; HIR parsed, lowered
/// (twice: optimized and not) and compiled; a grammar read, its program parsed, a DSL compiled.
const STAGES: [&str; 4] = ["parsed", "verified or lowered", "compiled", "-"];

fn reached(stage: usize) {
    REACHED[stage].fetch_add(1, Ordering::Relaxed);
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Runs `check` on mutated inputs: each on a thread with the compiler's stack (D83), within ten
/// seconds. `check` returns an error for a failure; a panic is one too.
fn fuzz(target: &str, seeds: &[String], check: fn(&str) -> Result<(), String>) {
    let words = dictionary(seeds);
    fuzz_with(
        target,
        |r| mutate(seeds, r.below(seeds.len()), &words, r),
        check,
    );
}

/// [`fuzz`] with inputs from `make`.
fn fuzz_with(
    target: &str,
    mut make: impl FnMut(&mut Rng) -> String,
    check: fn(&str) -> Result<(), String>,
) {
    let iterations = env_u64("A0_FUZZ_ITERATIONS", 150);
    let first = env_u64("A0_FUZZ_SEED", 1);
    for n in &REACHED {
        n.store(0, Ordering::Relaxed);
    }
    for seed in first..first + iterations {
        let mut r = Rng::new(seed);
        let input = make(&mut r);
        let (tx, rx) = mpsc::channel();
        let text = input.clone();
        std::thread::Builder::new()
            .stack_size(a0_driver::COMPILER_STACK)
            .spawn(move || {
                let result = std::panic::catch_unwind(|| check(&text))
                    .unwrap_or_else(|p| Err(format!("panic: {}", panic_text(&*p))));
                tx.send(result).ok();
            })
            .unwrap();
        let failure = match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => continue,
            Ok(Err(e)) => e,
            Err(_) => "no answer after 10 seconds".to_string(),
        };
        let path = std::env::temp_dir().join(format!("a0-fuzz-{target}-{seed}.txt"));
        std::fs::write(&path, &input).unwrap();
        panic!(
            "fuzz target {target}, seed {seed}: {failure}\ninput: {}",
            path.display()
        );
    }
    // Tests running at once share the counters: run one at a time to read them.
    let reached: Vec<String> = (0..3)
        .map(|i| format!("{} {}", REACHED[i].load(Ordering::Relaxed), STAGES[i]))
        .collect();
    eprintln!("fuzz {target}: {iterations} inputs, {}", reached.join(", "));
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "(no message)".to_string())
}

/// Renders diagnostics, which must not fail whatever their spans.
fn render(sources: &SourceMap, diags: &Diagnostics) {
    let text = diags.render_with(sources, &RenderOptions::default());
    assert!(!text.is_empty() || diags.is_empty());
}

/// Compiles a verified module with both code generators: an error is fine, an internal one is not.
fn codegen(m: &a0_ir::Module) -> Result<(), String> {
    if let Some(target) = a0_native::Target::host() {
        let opts = a0_native::CodegenOptions::default();
        if let Err(d) = a0_native::compile_object(m, target, &opts) {
            let s = d.summary();
            if s.contains("internal code generator error") {
                return Err(s);
            }
        } else {
            reached(2);
        }
    }
    a0_wasm::compile(m).ok();
    Ok(())
}

fn ir_text(text: &str) -> Result<(), String> {
    let mut sources = SourceMap::new();
    let id = sources.add("fuzz.a0ir", text);
    if a0_ir::parse(text, id).is_ok() {
        reached(0);
    }
    let m = match a0_ir::parse_and_verify(text, id) {
        Ok(m) => m,
        Err(d) => {
            render(&sources, &d);
            return Ok(());
        }
    };
    reached(1);
    // What the verifier accepts prints as text that parses and verifies to the same text.
    let printed = a0_ir::print_module(&m);
    let again = a0_ir::parse_and_verify(&printed, id)
        .map_err(|d| format!("printed IR does not parse back: {}", d.summary()))?;
    if a0_ir::print_module(&again) != printed {
        return Err("printed IR does not print back the same".to_string());
    }
    codegen(&m)
}

fn hir_text(text: &str) -> Result<(), String> {
    let mut sources = SourceMap::new();
    let id = sources.add("fuzz.hir", text);
    let m = match a0_hir::parse(text, id) {
        Ok(m) => m,
        Err(d) => {
            render(&sources, &d);
            return Ok(());
        }
    };
    reached(0);
    let printed = a0_hir::print_module(&m);
    let again = a0_hir::parse(&printed, id)
        .map_err(|d| format!("printed HIR does not parse back: {}", d.summary()))?;
    if a0_hir::print_module(&again) != printed {
        return Err("printed HIR does not print back the same".to_string());
    }
    for optimize in [true, false] {
        match a0_hir::lower_with(&m, a0_hir::LowerOptions { optimize }) {
            Ok(ir) => {
                reached(1);
                codegen(&ir)?
            }
            Err(d) => {
                if d.as_slice().iter().any(|d| d.code == "hir.lower") {
                    return Err(d.summary());
                }
                render(&sources, &d);
            }
        }
    }
    Ok(())
}

#[test]
fn ir_text_never_crashes() {
    let seeds: Vec<String> = corpus(&["tests/programs"], &["a0ir"])
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    fuzz("ir", &seeds, ir_text);
}

#[test]
fn hir_text_never_crashes() {
    let mut seeds: Vec<String> = corpus(&["tests/hir"], &["hir"])
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    seeds.extend(a0_hir::std_modules().iter().map(|m| {
        a0_hir::std_module_text(m)
            .expect("a standard module")
            .to_string()
    }));
    fuzz("hir", &seeds, hir_text);
}

/// The example languages' grammars, each with its programs.
fn languages() -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for (lang, ext) in [
        ("calc", "calc"),
        ("lispy", "lisp"),
        ("kaleidoscope", "k"),
        ("lox", "lox"),
        ("minnow", "mn"),
        ("tin", "tin"),
    ] {
        let grammar =
            std::fs::read_to_string(root().join(format!("examples/{lang}/{lang}.a0g"))).unwrap();
        let dirs = [
            format!("examples/{lang}/programs"),
            format!("examples/{lang}/tests/programs"),
            format!("examples/{lang}/tests/a0"),
        ];
        let dirs: Vec<&str> = dirs
            .iter()
            .map(String::as_str)
            .filter(|d| root().join(d).is_dir())
            .collect();
        let programs: Vec<String> = corpus(&dirs, &[ext]).into_iter().map(|(_, t)| t).collect();
        assert!(!programs.is_empty(), "no programs for {lang}");
        out.push((grammar, programs));
    }
    out
}

/// A grammar, mutated or not, and a program, mutated or not: the grammar is read and checked, the
/// program parsed with it, and a grammar with `@hir` forms compiles the tree.
fn grammar_and_program(grammar: &str, program: &str) -> Result<(), String> {
    let mut sources = SourceMap::new();
    let gid = sources.add("fuzz.a0g", grammar);
    let g = match a0_syntax::Grammar::from_str(grammar, gid) {
        Ok(g) => g,
        Err(d) => {
            render(&sources, &d);
            return Ok(());
        }
    };
    let forms = a0_hir::dsl::check_forms(&g);
    render(&sources, &forms);
    let id = sources.add("fuzz.src", program);
    reached(0);
    let (tree, diags) = g.parse(program, id);
    render(&sources, &diags);
    if !diags.has_errors() {
        reached(1);
    }
    tree.to_sexpr(tree.root());
    if diags.has_errors() || forms.has_errors() || !grammar.contains("@hir") {
        return Ok(());
    }
    match a0_hir::dsl::compile(&g, &tree, "fuzz") {
        Ok(m) => {
            reached(2);
            if let Err(d) = a0_hir::lower(&m) {
                if d.as_slice().iter().any(|d| d.code == "hir.lower") {
                    return Err(d.summary());
                }
                render(&sources, &d);
            }
        }
        Err(d) => render(&sources, &d),
    }
    Ok(())
}

#[test]
fn grammars_never_crash() {
    // Inputs are a grammar and a program of its language; one of them, or both, is mutated.
    let languages = languages();
    let grammars: Vec<String> = languages.iter().map(|(g, _)| g.clone()).collect();
    let grammar_words = dictionary(&grammars);
    let program_words: Vec<Vec<String>> = languages.iter().map(|(_, p)| dictionary(p)).collect();
    fuzz_with(
        "grammar",
        |r| {
            let lang = r.below(languages.len());
            let programs = &languages[lang].1;
            let (grammar, program) = match r.below(3) {
                0 => (mutate(&grammars, lang, &grammar_words, r), None),
                1 => (grammars[lang].clone(), None),
                _ => (
                    mutate(&grammars, lang, &grammar_words, r),
                    Some(programs[r.below(programs.len())].clone()),
                ),
            };
            let program = program.unwrap_or_else(|| {
                let i = r.below(programs.len());
                mutate(programs, i, &program_words[lang], r)
            });
            format!("{grammar}{SEPARATOR}{program}")
        },
        |input| {
            let (grammar, program) = input.split_once(SEPARATOR).unwrap_or((input, ""));
            grammar_and_program(grammar, program)
        },
    );
}

/// Separates the grammar and the program in an input of [`grammars_never_crash`].
const SEPARATOR: &str = "\n\u{0}program\u{0}\n";

#[test]
fn corpus_seeds_pass_unmutated() {
    // The checks above hold for the inputs they start from.
    for (name, text) in corpus(&["tests/programs"], &["a0ir"]) {
        ir_text(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    for (name, text) in corpus(&["tests/hir"], &["hir"]) {
        hir_text(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    for (grammar, programs) in languages() {
        for p in programs {
            grammar_and_program(&grammar, &p).unwrap();
        }
    }
}
