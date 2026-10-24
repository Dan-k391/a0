// Starts `<command> lsp` for each language in the `a0.languages` setting, the first time a file of
// that language is opened. Everything else (what the server can do) comes from the server.
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

function start(entry) {
  const key = entry.command;
  if (clients.has(key)) {
    return;
  }
  const name = entry.language || entry.command;
  const server = { command: entry.command, args: ['lsp'] };
  const client = new LanguageClient(`a0-${name}`, `${name} (A0)`, { run: server, debug: server }, {
    documentSelector: selector(entry),
  });
  clients.set(key, client);
  client.start().catch((error) => {
    vscode.window.showErrorMessage(`Could not start \`${entry.command} lsp\`: ${error.message}`);
  });
}

function startFor(document) {
  for (const entry of vscode.workspace.getConfiguration('a0').get('languages', [])) {
    if (vscode.languages.match(selector(entry), document) > 0) {
      start(entry);
    }
  }
}

function activate(context) {
  vscode.workspace.textDocuments.forEach(startFor);
  context.subscriptions.push(vscode.workspace.onDidOpenTextDocument(startFor));
}

function deactivate() {
  return Promise.all([...clients.values()].map((client) => client.stop()));
}

module.exports = { activate, deactivate };
