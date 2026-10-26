// Runs a program compiled to WebAssembly (`a0-playground wasm`, D78). The module is not a whole
// program: it links against an instance of the engine, which holds A0's runtime and exports its
// functions, memory and function table. This makes a fresh instance (the runtime's state starts
// anew, as for every run), gives the module the memory and table entries it asks for (custom
// section `a0.layout`), and calls its `a0_wasm_main`.

import { Exit, wasiImports } from './wasi.js';

/** Returns the program's exit status; `stdout` and `stderr` receive what it writes. */
export async function runCompiled(engine, program, { args, stdout, stderr }) {
  let memory;
  const imports = wasiImports({ args, stdout, stderr }, () => memory);
  const rt = (await WebAssembly.instantiate(engine, { wasi_snapshot_preview1: imports })).exports;
  memory = rt.memory;
  const module = await WebAssembly.compile(program);
  const layout = new DataView(WebAssembly.Module.customSections(module, 'a0.layout')[0]);
  const dataSize = layout.getUint32(0, true);
  const tableSize = layout.getUint32(4, true);
  const memoryBase = (Number(rt.a0_alloc(BigInt(dataSize + 16))) + 15) & ~15;
  const table = rt.__indirect_function_table;
  const tableBase = table.grow(tableSize);
  const env = {
    memory,
    __indirect_function_table: table,
    __memory_base: new WebAssembly.Global({ value: 'i32', mutable: false }, memoryBase),
    __table_base: new WebAssembly.Global({ value: 'i32', mutable: false }, tableBase),
  };
  for (const imp of WebAssembly.Module.imports(module)) {
    if (imp.kind !== 'function') continue;
    if (typeof rt[imp.name] !== 'function') {
      stderr(new TextEncoder().encode(`the runtime has no function ${imp.name}\n`));
      return 1;
    }
    env[imp.name] = rt[imp.name];
  }
  const instance = await WebAssembly.instantiate(module, { env });
  // The program's arguments, for the runtime.
  const encoder = new TextEncoder();
  const argv = Number(rt.a0_alloc(BigInt(4 * (args.length + 1))));
  args.forEach((a, i) => {
    const s = encoder.encode(`${a}\0`);
    const at = Number(rt.a0_alloc(BigInt(s.length)));
    new Uint8Array(memory.buffer).set(s, at);
    new DataView(memory.buffer).setUint32(argv + 4 * i, at, true);
  });
  try {
    rt.a0_rt_init(args.length, argv);
    const code = instance.exports.a0_wasm_main();
    rt.a0_rt_exit(code);
    return code;
  } catch (e) {
    if (e instanceof Exit) return e.code;
    // The engine's own stack ran out before A0's limit on calls did.
    if (e instanceof RangeError) {
      rt.a0_rt_flush?.();
      stderr(encoder.encode('panic: stack overflow\n'));
      return 101;
    }
    throw e;
  }
}
