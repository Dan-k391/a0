// Tests a built playground (playground/build.sh) the way the page uses it, with Node and the same
// WASI shim: every example runs and prints what its program expects; the other modes, errors and
// stack overflows work.
//
//     node playground/test.mjs target/playground

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { runWasi } from './wasi.js';

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

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
