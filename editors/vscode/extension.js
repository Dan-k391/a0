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

// `${workspaceFolder}` in a command is the folder of the document (or the first folder): projects
// made with `a0 new` point at the server they build.
function command(entry, document) {
  const folder =
    (document && vscode.workspace.getWorkspaceFolder(document.uri)) ||
    (vscode.workspace.workspaceFolders || [])[0];
  return folder ? entry.command.replace('${workspaceFolder}', folder.uri.fsPath) : entry.command;
}

function start(entry, document) {
  const program = command(entry, document);
  const key = program;
  if (clients.has(key)) {
    return;
  }
  const name = entry.language || entry.command.split(/[\\/]/).pop();
  const server = { command: program, args: ['lsp'] };
  const client = new LanguageClient(`a0-${name}`, `${name} (A0)`, { run: server, debug: server }, {
    documentSelector: selector(entry),
  });
  clients.set(key, client);
  client.start().catch((error) => {
    vscode.window.showErrorMessage(`Could not start \`${program} lsp\`: ${error.message}`);
  });
}

function startFor(document) {
  for (const entry of vscode.workspace.getConfiguration('a0').get('languages', [])) {
    if (vscode.languages.match(selector(entry), document) > 0) {
      start(entry, document);
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
