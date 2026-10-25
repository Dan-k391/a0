# Editor support

Every language built with `a0-driver` has a language server: `lang lsp` speaks the Language Server
Protocol on stdin and stdout. A language needs no code for it. It passes its grammar to the driver
(`Language { grammar: Some(GRAMMAR), .. }`) and marks declarations and scopes in the grammar with
`@def` and `@scope` (`docs/grammar.md`).

| Feature | Where it comes from |
|---|---|
| Diagnostics | the language's compiler, as it reports errors on the command line: spans become ranges, labels related information, and codes are shown as the language chooses (`docs/diagnostics.md`) |
| Quick fixes (code actions) | the fixes diagnostics suggest, such as a declared name close to a misspelled one |
| Highlighting (semantic tokens) | the grammar: keywords, operators, numbers, strings and comments from its tokens; functions, types, parameters, variables and members from `@def` |
| Outline (document symbols) | `@def`: declarations nested in the declarations around them, without parameters and local variables |
| Go to definition | `@def` and `@scope`: lexical resolution, every member of that name after `.` or `::` |
| Hover | the declaration's first line and kind, plus its type when the compiler builds HIR with spans (Minnow: `variable total: int`) |

For a language with modules, a document is analyzed as the main file of its program: the files it
imports are read from documents open in the editor or from disk, their diagnostics are published
for them, and go-to-definition on another module's item (`util.twice`) opens that file; hovering
over one, or over the standard library's (`str.split`), shows its signature. Types on hover and
definitions keep working while the program has errors.

The server keeps whole documents (full sync), re-analyzes a document when it changes, and survives
a panicking compiler by reporting it as a diagnostic. Positions are UTF-16 code units, as the protocol
requires. Tests drive it with `a0_driver::lsp::session` (see `examples/*/tests/lsp.rs`).

## VS Code

`editors/vscode` is a small extension that starts `<command> lsp` for every entry of the
`a0.languages` setting. It declares Lox (`.lox`), Minnow (`.mn`) and Kaleidoscope (`.k`):

```
cargo install --path examples/lox        # or set the command to target/release/lox
cd editors/vscode && npm install
code --extensionDevelopmentPath=editors/vscode some-folder
```

To keep it installed, package it with `npx @vscode/vsce package` and install the `.vsix`. A new
language is one entry in `a0.languages`:

```json
"a0.languages": [
  { "language": "lox", "extensions": [".lox"], "command": "lox" },
  { "extensions": [".mylang"], "command": "D:/mylang/target/release/mylang" }
]
```

`${workspaceFolder}` in a command is the folder of the file being edited, which is how a project
made with `a0 new` points at the language server it builds (its `.vscode/settings.json`). An entry's
`args` replace `lsp`, for a language defined by its grammar alone (`docs/grammar.md`), whose server
is `a0 lang <grammar> lsp`:

```json
{ "extensions": [".calc"], "command": "a0", "args": ["lang", "${workspaceFolder}/calc.a0g", "lsp"] }
```

VS Code colors names (functions, types, parameters) from semantic tokens with any theme, but keywords,
strings, numbers and comments only through a TextMate grammar or a `semanticTokenScopes` mapping; the
extension maps them for every language.

## Other editors

Any LSP client works. Neovim (0.11 or later):

```lua
vim.lsp.config('lox', { cmd = { 'lox', 'lsp' }, filetypes = { 'lox' } })
vim.lsp.enable('lox')
vim.filetype.add({ extension = { lox = 'lox' } })
```

Helix (`languages.toml`):

```toml
[language-server.lox]
command = "lox"
args = ["lsp"]

[[language]]
name = "lox"
scope = "source.lox"
file-types = ["lox"]
language-servers = ["lox"]
```

## Debugging

The extension also adds a debugger of type `a0`, which runs `<command> dap` for the program's
language (`docs/debugging.md`):

```json
{ "type": "a0", "request": "launch", "name": "Debug the current file", "program": "${file}" }
```

For languages the extension does not declare, `"debug.allowBreakpointsEverywhere": true` lets VS
Code take breakpoints in their files.

## Limits

Resolution is lexical and knows no types, so `p.x` offers every member named `x`, and names a language
declares without a grammar rule (Lox's fields, assigned in methods) have no definition. Renaming,
completion and find-references are not implemented yet; all three can be built on `SyntaxIndex`.
