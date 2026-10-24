# A0 languages for VS Code

Starts the language server of each language built on A0 (`<command> lsp`) and gives VS Code its
diagnostics, highlighting, outline, go to definition and hovers. See `docs/editors.md` in the A0
repository for what the server does and how to add a language.

```
npm install
code --extensionDevelopmentPath=. path/to/folder
```

The `a0.languages` setting lists the languages: `command` is the language's driver, `extensions` and
`language` select its files. The defaults expect `lox`, `minnow` and `kaleidoscope` on the `PATH`.
