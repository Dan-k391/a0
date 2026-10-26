//! B0's compiler never crashes (D85): programs mutated from `tests/programs` compile to HIR and
//! lower to core IR, or give diagnostics, which render; a panic is a failure. `A0_FUZZ_ITERATIONS`
//! (300 by default) and `A0_FUZZ_SEED` run it longer.

use std::path::Path;

use a0_diag::{RenderOptions, SourceMap};

fn programs() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/programs");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "b0"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect()
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A program with one mutation or a few: spans deleted or duplicated, words of other programs put
/// in.
fn mutate(seeds: &[String], words: &[&str], state: &mut u64) -> String {
    let mut next = |n: usize| {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        (*state % n.max(1) as u64) as usize
    };
    let mut s = seeds[next(seeds.len())].clone();
    for _ in 0..1 + next(3) {
        let at = s.floor_char_boundary(next(s.len() + 1));
        let end = s.ceil_char_boundary(at + 1 + next(12));
        match next(4) {
            0 => s.replace_range(at..end, ""),
            1 => {
                let piece = s[at..end].to_string();
                let to = s.floor_char_boundary(next(s.len() + 1));
                s.insert_str(to, &piece);
            }
            2 => s.replace_range(at..end, words[next(words.len())]),
            _ => s.insert_str(at, &format!(" {} ", words[next(words.len())])),
        }
    }
    s
}

#[test]
fn mutated_programs_never_crash_the_compiler() {
    let seeds = programs();
    let text = seeds.join(" ");
    let mut words: Vec<&str> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    words.extend([
        "{", "}", "(", ")", ";", "unsafe", "throw", "?", "!", "..", "[", "]", "null",
    ]);
    words.sort();
    words.dedup();
    let iterations = env_u64("A0_FUZZ_ITERATIONS", 300);
    let first = env_u64("A0_FUZZ_SEED", 1);
    for seed in first..first + iterations {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let program = mutate(&seeds, &words, &mut state);
        let input = program.clone();
        // On the compiler's stack (D83), as the driver compiles.
        let result = std::thread::Builder::new()
            .stack_size(a0_driver::COMPILER_STACK)
            .spawn(move || {
                let mut sources = SourceMap::new();
                let id = sources.add("fuzz.b0", &input);
                match b0::compile(&input, id) {
                    Ok(m) => {
                        if let Err(d) = a0_hir::lower(&m) {
                            assert!(
                                !d.as_slice().iter().any(|d| d.code == "hir.lower"),
                                "lowering made invalid IR: {}",
                                d.summary()
                            );
                        }
                    }
                    Err(d) => {
                        d.render_with(&sources, &RenderOptions::default());
                    }
                }
            })
            .unwrap()
            .join();
        if result.is_err() {
            let path = std::env::temp_dir().join(format!("b0-fuzz-{seed}.b0"));
            std::fs::write(&path, &program).unwrap();
            panic!(
                "seed {seed} crashed the compiler; the program is {}",
                path.display()
            );
        }
    }
}
