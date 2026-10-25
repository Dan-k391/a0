// Starts `<command> lsp` (or `<command> <args>`) for each language in the `a0.languages` setting,
// the first time a file of that language is opened, and `<command> dap` to debug one of its
// programs. Everything else (what the server and the debugger can do) comes from them.
const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');

const clients = new Map();

function selector(entry) {
  const filters = (entry.extensions || []).map((ext) => ({ scheme: 'file', pattern: `**/*${ext}` }));
  if (entry.language) {
    filters.push({ scheme: 'file', language: entry.language });
  }
  return filters;
}

// `${workspaceFolder}` in a command or its arguments is the folder of the document (or the first
// folder): projects made with `a0 new` point at the server they build, and languages defined by a
// grammar alone at their grammar (`a0 lang ${workspaceFolder}/calc.a0g lsp`).
function expand(text, document) {
  const folder =
    (document && vscode.workspace.getWorkspaceFolder(document.uri)) ||
    (vscode.workspace.workspaceFolders || [])[0];
  return folder ? text.split('${workspaceFolder}').join(folder.uri.fsPath) : text;
}

function start(entry, document) {
  const program = expand(entry.command, document);
  const args = (entry.args || ['lsp']).map((arg) => expand(arg, document));
  const key = [program, ...args].join(' ');
  if (clients.has(key)) {
    return;
  }
  const name = entry.language || entry.command.split(/[\\/]/).pop();
  const server = { command: program, args };
  const client = new LanguageClient(`a0-${name}`, `${name} (A0)`, { run: server, debug: server }, {
    documentSelector: selector(entry),
  });
  clients.set(key, client);
  client.start().catch((error) => {
    vscode.window.showErrorMessage(`Could not start \`${key}\`: ${error.message}`);
  });
}

function startFor(document) {
  for (const entry of vscode.workspace.getConfiguration('a0').get('languages', [])) {
    if (vscode.languages.match(selector(entry), document) > 0) {
      start(entry, document);
    }
  }
}

// Debugging (`a0` launch configurations) runs `<command> dap`: the entry of `a0.languages` whose
// extensions match the program, with `lsp` in its `args` replaced by `dap`.
const debugAdapters = {
  createDebugAdapterDescriptor(session) {
    const program = session.configuration.program || '';
    const entry = vscode.workspace
      .getConfiguration('a0')
      .get('languages', [])
      .find((e) => (e.extensions || []).some((ext) => program.endsWith(ext)));
    if (!entry) {
      throw new Error(`No language in the a0.languages setting has the extension of ${program}`);
    }
    const document = vscode.workspace.textDocuments.find((d) => d.uri.fsPath === program);
    const args = (entry.args || ['lsp']).map((arg) => (arg === 'lsp' ? 'dap' : expand(arg, document)));
    return new vscode.DebugAdapterExecutable(expand(entry.command, document), args);
  },
};

function activate(context) {
  vscode.workspace.textDocuments.forEach(startFor);
  context.subscriptions.push(vscode.workspace.onDidOpenTextDocument(startFor));
  context.subscriptions.push(vscode.debug.registerDebugAdapterDescriptorFactory('a0', debugAdapters));
}

function deactivate() {
  return Promise.all([...clients.values()].map((client) => client.stop()));
}

module.exports = { activate, deactivate };
