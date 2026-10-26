// Runs the engine (a0-playground.wasm) off the page's thread, so that a program that never ends can
// be stopped by ending the worker. Each run gets a fresh instance: the runtime's state starts anew.
// A program runs compiled to WebAssembly (the engine compiles it, `compiled.js` links it against a
// fresh engine instance), or in the engine's interpreter when the page asks.
import { runWasi } from './wasi.js';
import { runCompiled } from './compiled.js';

const engine = WebAssembly.compileStreaming(fetch('a0-playground.wasm'));
const LIMIT = 1 << 20; // characters of output shown

self.onmessage = async ({ data: { mode, grammar, program, interpret } }) => {
  const started = performance.now();
  let pending = [];
  let shown = 0;
  let lastFlush = started;
  const flush = () => {
    if (pending.length > 0) self.postMessage({ kind: 'output', parts: pending });
    pending = [];
    lastFlush = performance.now();
  };
  const sink = (stream) => {
    const decoder = new TextDecoder();
    return (bytes) => {
      if (shown >= LIMIT) return;
      let text = decoder.decode(bytes, { stream: true });
      if (shown + text.length > LIMIT) text = `${text.slice(0, LIMIT - shown)}\n(output cut short)\n`;
      shown += text.length;
      pending.push({ stream, text });
      // The program runs synchronously, so output is sent in batches as it goes.
      if (performance.now() - lastFlush > 50) flush();
    };
  };
  let code = null;
  const compiled = mode === 'run' && !interpret;
  try {
    if (compiled) {
      // The module comes on the engine's standard output; errors on its standard error.
      const chunks = [];
      code = await runWasi(await engine, {
        args: ['a0-playground', 'wasm', grammar, program],
        stdout: (b) => chunks.push(b),
        stderr: sink('err'),
      });
      if (code === 0) {
        const bytes = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
        let at = 0;
        for (const c of chunks) {
          bytes.set(c, at);
          at += c.length;
        }
        code = await runCompiled(await engine, bytes, {
          args: ['program'],
          stdout: sink('out'),
          stderr: sink('err'),
        });
      }
    } else {
      code = await runWasi(await engine, {
        args: ['a0-playground', mode, grammar, program],
        stdout: sink('out'),
        stderr: sink('err'),
      });
    }
  } catch (e) {
    pending.push({ stream: 'err', text: `\nThe engine failed: ${e}\n` });
  }
  flush();
  self.postMessage({ kind: 'exit', code, compiled, ms: performance.now() - started });
};
