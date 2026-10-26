// Tests a built playground (playground/build.sh) the way the page uses it, with Node and the same
// WASI shim: every example runs and prints what its program expects, interpreted and compiled to
// WebAssembly; the other modes, errors and stack overflows work; and every core IR and HIR program
// of the differential corpus (tests/programs, tests/hir), compiled to WebAssembly and linked against
// the engine's runtime, prints, fails and exits as it expects (run from the repository's root).
//
//     node playground/test.mjs target/playground

import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { runWasi } from './wasi.js';
import { runCompiled } from './compiled.js';

const site = process.argv[2] ?? 'target/playground';
const engine = await WebAssembly.compile(readFileSync(join(site, 'a0-playground.wasm')));
const read = (path) => readFileSync(join(site, path), 'utf8');
let failures = 0;

async function run(mode, grammar, program) {
  const out = [];
  const err = [];
  const decoder = new TextDecoder();
  const code = await runWasi(engine, {
    args: ['a0-playground', mode, grammar, program],
    stdout: (b) => out.push(decoder.decode(b)),
    stderr: (b) => err.push(decoder.decode(b)),
  });
  return { code, stdout: out.join(''), stderr: err.join('') };
}

function check(name, ok, detail) {
  if (ok) {
    console.log(`ok   ${name}`);
  } else {
    console.log(`FAIL ${name}\n${detail}`);
    failures++;
  }
}

// What a program says it prints: the comment lines after `# expect-stdout:` (`;` in Lispy).
function expected(text) {
  const lines = text.split('\n');
  const start = lines.findIndex((l) => /^[#;] expect-stdout:$/.test(l));
  return lines
    .slice(start + 1)
    .filter((l) => l !== '')
    .map((l) => `${l.replace(/^[#;] ?/, '')}\n`)
    .join('');
}

for (const example of JSON.parse(read('examples.json'))) {
  const program = read(example.program);
  const r = await run('run', read(example.grammar), program);
  check(example.name, r.code === 0 && r.stdout === expected(program), JSON.stringify(r, null, 2));
}

const calc = read('examples/calc/calc.a0g');
let r = await run('tree', calc, 'print 1 + 2');
check('tree', r.code === 0 && r.stdout.startsWith('(program (print_stmt'), r.stdout + r.stderr);
r = await run('hir', calc, 'print 1');
check('hir', r.code === 0 && r.stdout.includes('fn main()'), r.stdout + r.stderr);
r = await run('ir', calc, 'print 1');
check('ir', r.code === 0 && r.stdout.includes('func @main'), r.stdout + r.stderr);
r = await run('run', calc, 'print nope(1)');
check(
  'errors',
  r.code === 1 && r.stderr.includes('error[E1001]: unknown function `nope`') &&
    r.stderr.includes('--> program:1:7'),
  r.stderr,
);
r = await run('run', 'grammar g; program = missing;', '');
check('grammar errors', r.code === 1 && r.stderr.includes('--> grammar:1:22'), r.stderr);
r = await run('run', calc, 'fn down(n) {\n  return 1 + down(n - 1)\n}\nprint "before"\nprint down(1)\n');
check(
  'stack overflow',
  r.code === 101 && r.stdout === 'before\n' && r.stderr.startsWith('panic: stack overflow'),
  JSON.stringify(r, null, 2),
);

// Compiled to WebAssembly by the engine, then run against a fresh instance of it.
async function compile(args) {
  const chunks = [];
  const err = [];
  const code = await runWasi(engine, {
    args: ['a0-playground', ...args],
    stdout: (b) => chunks.push(b),
    stderr: (b) => err.push(new TextDecoder().decode(b)),
  });
  if (code !== 0) return { error: err.join('') };
  const bytes = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
  let at = 0;
  for (const c of chunks) {
    bytes.set(c, at);
    at += c.length;
  }
  return { bytes };
}

async function runBytes(bytes, args) {
  const out = [];
  const err = [];
  const decoder = new TextDecoder();
  const code = await runCompiled(engine, bytes, {
    args,
    stdout: (b) => out.push(decoder.decode(b)),
    stderr: (b) => err.push(decoder.decode(b)),
  });
  return { code, stdout: out.join(''), stderr: err.join('') };
}

for (const example of JSON.parse(read('examples.json'))) {
  const program = read(example.program);
  const c = await compile(['wasm', read(example.grammar), program]);
  const r = c.bytes ? await runBytes(c.bytes, ['program']) : { error: c.error };
  check(`${example.name} (compiled)`, r.code === 0 && r.stdout === expected(program), JSON.stringify(r, null, 2));
}

// What a corpus program expects: `expect-stdout:`, `expect-stderr:` and `expect-exit:` lines after
// `;` or `//` (crates/a0/tests/programs.rs).
function expectations(text) {
  const e = { stdout: '', stderr: '', exit: 0 };
  for (const line of text.split('\n')) {
    const m = /^(?:;|\/\/) expect-(stdout|stderr|exit):(?: (.*))?$/.exec(line.trimEnd());
    if (!m) continue;
    if (m[1] === 'exit') e.exit = Number(m[2]);
    else e[m[1]] += `${m[2] ?? ''}\n`;
  }
  return e;
}

// Programs that need what WebAssembly does not have here: threads, C structs by value, and C
// functions declared with 64-bit integers for C types that are 32 bits on wasm32 (`size_t`, `long`).
const unsupported = new Set([
  'threads.hir', 'c_structs.hir', 'c_structs.a0ir', 'ffi.hir', 'pointers.hir', 'arrays_filled.hir',
]);
for (const dir of ['tests/programs', 'tests/hir']) {
  for (const name of readdirSync(dir).sort()) {
    if (!/\.(a0ir|hir)$/.test(name) || unsupported.has(name)) continue;
    const file = `${dir}/${name}`;
    const text = readFileSync(file, 'utf8').replaceAll('\r\n', '\n');
    const want = expectations(text);
    const c = await compile(['wasm-file', file, text]);
    if (!c.bytes) {
      check(`${file} (compiled)`, false, c.error);
      continue;
    }
    const r = await runBytes(c.bytes, [file]);
    check(
      `${file} (compiled)`,
      r.code === want.exit && r.stdout === want.stdout && r.stderr === want.stderr,
      `got ${JSON.stringify(r, null, 2)}\nwanted ${JSON.stringify(want, null, 2)}`,
    );
  }
}

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
