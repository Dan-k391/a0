// The playground page: two editors, an output pane, and the engine in a worker.

const $ = (id) => document.getElementById(id);
const grammar = $('grammar');
const program = $('program');
const output = $('output');
const status = $('status');
const picker = $('examples');
const run = $('run');
const modes = ['tree', 'hir', 'ir'];

let worker = null;
let examples = [];

function startWorker() {
  worker = new Worker('worker.js', { type: 'module' });
  worker.onmessage = ({ data }) => {
    if (data.kind === 'output') {
      for (const { stream, text } of data.parts) {
        if (stream === 'err') {
          const span = document.createElement('span');
          span.className = 'err';
          span.textContent = text;
          output.append(span);
        } else {
          output.append(text);
        }
      }
    } else if (data.kind === 'exit') {
      const code = data.code === null ? 'failed' : `exit ${data.code}`;
      const how = data.compiled ? ' · compiled' : '';
      status.textContent = `${code} · ${Math.round(data.ms)} ms${how}`;
      running(false);
    }
  };
}

function running(on) {
  run.textContent = on ? 'Stop' : 'Run';
  for (const m of modes) $(m).disabled = on;
}

function start(mode) {
  if (run.textContent === 'Stop') {
    // A program that does not end is stopped with its worker.
    worker.terminate();
    startWorker();
    status.textContent = 'stopped';
    running(false);
    return;
  }
  output.textContent = '';
  status.textContent = 'running…';
  running(true);
  // Programs run compiled to WebAssembly, unless asked to run in the interpreter.
  const interpret = $('interpret').checked;
  worker.postMessage({ mode, grammar: grammar.value, program: program.value, interpret });
}

async function load(example) {
  const [g, p] = await Promise.all(
    [example.grammar, example.program].map((path) => fetch(path).then((r) => r.text())),
  );
  grammar.value = g;
  program.value = p;
  output.textContent = '';
  status.textContent = '';
}

// A link holds the grammar and the program, base64 of their UTF-8 in JSON, after `#code=`.
function encode() {
  const json = JSON.stringify({ grammar: grammar.value, program: program.value });
  const bytes = new TextEncoder().encode(json);
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return `#code=${encodeURIComponent(btoa(binary))}`;
}

function decode(hash) {
  try {
    const binary = atob(decodeURIComponent(hash.slice('#code='.length)));
    const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    return null;
  }
}

function remember() {
  try {
    localStorage.setItem('a0-playground', JSON.stringify({
      grammar: grammar.value,
      program: program.value,
      example: picker.value,
    }));
  } catch {
    // Storage may be unavailable (private windows); the page works without it.
  }
}

function recall() {
  try {
    return JSON.parse(localStorage.getItem('a0-playground'));
  } catch {
    return null;
  }
}

// Tab inserts two spaces rather than leaving the editor.
for (const editor of [grammar, program]) {
  editor.addEventListener('keydown', (e) => {
    if (e.key === 'Tab' && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      editor.setRangeText('  ', editor.selectionStart, editor.selectionEnd, 'end');
    }
  });
  editor.addEventListener('input', remember);
}

document.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    start('run');
  }
});

run.addEventListener('click', () => start('run'));
for (const m of modes) $(m).addEventListener('click', () => start(m));
picker.addEventListener('change', async () => {
  await load(examples[picker.value]);
  remember();
});
$('share').addEventListener('click', async () => {
  const url = `${location.href.split('#')[0]}${encode()}`;
  history.replaceState(null, '', url);
  try {
    await navigator.clipboard.writeText(url);
    status.textContent = 'link copied';
  } catch {
    status.textContent = 'link in the address bar';
  }
});

startWorker();
examples = await fetch('examples.json').then((r) => r.json());
examples.forEach((example, i) => picker.add(new Option(example.name, String(i))));
const shared = location.hash.startsWith('#code=') ? decode(location.hash) : null;
const saved = recall();
if (shared) {
  grammar.value = shared.grammar;
  program.value = shared.program;
} else if (saved && typeof saved.grammar === 'string') {
  grammar.value = saved.grammar;
  program.value = saved.program;
  if (examples[saved.example]) picker.value = saved.example;
} else {
  await load(examples[0]);
}
