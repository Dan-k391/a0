# Editor support

Every language built on A0 works in editors such as VS Code, Neovim and Helix. It gets errors as
you type, quick fixes, highlighting, an outline, go to definition, hovers, completion, find
references and rename, with no editor code of its own. The language's command has a language
server built in: `lang lsp`, where `lang` is the command, such as `minnow` or `lox`. A language
server is a program that an editor starts and asks about the code being edited, through the
[Language Server Protocol](https://microsoft.github.io/language-server-protocol/) (LSP).

## What you get

Here is a program in Minnow, one of A0's example languages ([the language guides](book/languages.md)):

```
struct Point { x: int, y: int }
enum Shape { Circle(float), Square(float) }

fn area(s: Shape) -> float {
  return match s {
    Circle(r) => 3.14 * r * r,
    Square(w) => w * w,
  };
}

fn main() {
  let p = new Point { x: 1, y: 2 };
  let total = p.x + p.y;
  print(str(area(Shape::Circle(1.0))));
  print(str(total));
}
```

Hover over `total`, and the editor shows its declaration and the type Minnow worked out for it:

````
```minnow
let total = p.x + p.y;
```
variable `total`: `int`
````

Hover over the `y` of `p.y`, and it shows the field:

````
```minnow
y: int
```
field `y`: `int`
````

The outline lists `Point` (with `x` and `y`), `Shape` (with `Circle` and `Square`), `area` and
`main`. Go to definition on `Circle` in `Shape::Circle(1.0)` jumps to the variant on line 2, and
on `area` to the function on line 4.

Errors appear as you type. In this program, `cuont` is misspelled:

```
fn main() {
  let count = 1;
  print(cuont);
}
```

The editor underlines `cuont` as an unknown name, with the code E1001. It also offers a quick fix,
"a variable with a similar name exists: `count`", which replaces `cuont` with `count`.

These hovers, outline, definitions and fix are what Minnow's own tests check, in
`examples/minnow/tests/lsp.rs`.

## Use it in VS Code

A0 has a small VS Code extension in `editors/vscode`. It starts each language's server, and its
debugger too ([debugging](debugging.md)).

1. Put the language's command on your `PATH`. For Lox, from the A0 repository:

   ```
   cargo install --path examples/lox
   ```

2. Install the extension's dependencies, and start VS Code with the extension loaded:

   ```
   cd editors/vscode
   npm install
   code --extensionDevelopmentPath=. path/to/your/folder
   ```

3. Open a `.lox` file. VS Code starts `lox lsp` the first time you open one.

To keep the extension installed instead of loading it each time, package it with
`npx @vscode/vsce package` in `editors/vscode`. Then install the `.vsix` file it makes, with
**Extensions: Install from VSIX...** in VS Code.

The extension knows five languages, and expects their commands on the `PATH`:

| Language | Files | Command |
|---|---|---|
| B0 | `.b0` | `b0` |
| Lox | `.lox` | `lox` |
| Minnow | `.mn` | `minnow` |
| Kaleidoscope | `.k` | `kaleidoscope` |
| Tin | `.tin` | `tin` |

### Add your language

The setting `a0.languages` lists the languages the extension starts servers for. Add one entry for
your language to your settings:

```json
"a0.languages": [
  { "language": "lox", "extensions": [".lox"], "command": "lox" },
  { "extensions": [".mylang"], "command": "D:/mylang/target/release/mylang" }
]
```

Each entry has these fields:

| Field | Meaning |
|---|---|
| `command` | The language's command. Required. The extension runs it as `command lsp`. |
| `extensions` | The file endings that belong to the language. |
| `language` | A VS Code language id, another way to pick the files. |
| `args` | Arguments to run the command with, in place of `lsp`. |

`${workspaceFolder}` in `command` or `args` stands for the workspace folder that holds the file
(or the first folder open). A project made with `a0 new` uses it in its `.vscode/settings.json`,
to point at the server it builds:

```json
{ "extensions": [".mylang"], "command": "${workspaceFolder}/target/debug/mylang" }
```

A language defined by its grammar alone ([grammars](grammar.md#languages-without-host-code-hir))
has its server in `a0 lang <grammar> lsp`, so its entry gives `args`:

```json
{ "extensions": [".calc"], "command": "a0", "args": ["lang", "${workspaceFolder}/calc.a0g", "lsp"] }
```

A language written in Python ([A0 from Python](python.md)) runs Python:

```json
{ "extensions": [".pb"], "command": "python", "args": ["${workspaceFolder}/pebble.py", "lsp"] }
```

### Colors

VS Code colors names (functions, types, parameters) from what the server says about them, with any
color theme. It colors keywords, strings, numbers and comments only when something maps them to
the theme's colors. The extension does that for every language, so they are colored too.

## Use it in other editors

Any editor with an LSP client works. Start the server as `lang lsp`.

Neovim (0.11 or later):

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

## What each feature needs from a language

A language gets the server by giving the driver its grammar:
`Language::new(name, compile).grammar(GRAMMAR)`, where `GRAMMAR` is the text of its `.a0g` file.
The grammar marks where names are declared with `@def`, and where scopes begin with `@scope`
([grammars](grammar.md#names-def-and-scope)). Without a grammar, the server gives only diagnostics
and quick fixes.

Each feature comes from something the language already has:

| Feature | Where it comes from |
|---|---|
| Diagnostics (errors and warnings) | The language's compiler, as on the command line. Each label of a diagnostic becomes related information, and its code is shown as the language chooses ([diagnostics](diagnostics.md)). |
| Quick fixes | The fixes the diagnostics suggest, such as a declared name close to a misspelled one. |
| Highlighting | The grammar. Keywords, operators, numbers, strings and comments come from its tokens. Functions, types, parameters, variables and members come from `@def`. |
| Outline | `@def`: each declaration, inside the declarations around it. Parameters and local variables are left out. |
| Go to definition | `@def` and `@scope`, which say what each name refers to. After `.` or `::`, every member of that name. |
| Hover | The declaration's first line and its kind. Its type, too, when the compiler builds the high-level IR with spans. The high-level IR ([HIR](hir.md)) is the typed form of the program a compiler gives A0, and a span says where in the source each part came from. |
| Completion | The names in scope at the cursor, the modules and names the file imports, and the grammar's keywords. After `p.`: the fields and methods of `p`'s type, when the HIR knows it. After `alias.`: the items of that module. |
| Find references | Every name that refers to the same declaration. In the declaration's own file, the grammar's scopes find them. In other files of the program, they are the places that import it, as `alias.name` or as the bare name. |
| Rename | The same places as find references, changed in every file. |

Rename refuses in three cases:

- The new name is a keyword, or anything else that is not a name in the language.
- The name is a member that another type has too, since their uses cannot be told apart.
- The program has syntax errors, which keep its files from loading.

### Programs of several files

For a language with modules ([packages](packages.md) and `@import` in
[grammars](grammar.md#imports-import)), the server analyzes a document as the main file of its
program:

- It reads the files the document imports from the editor's open documents, or from disk.
- It reports the errors of each imported file in that file.
- Go to definition on another module's item (`util.twice`) opens that module's file.
- Hovering over another module's item, or over one of the standard library's (`str.split`), shows
  its signature.
- In a package, imports go through the package and its dependencies. Git dependencies come from
  the cache that `lang build` downloaded them into. Errors in a package's manifest are reported in
  the manifest.

Types on hover, and definitions, keep working while the program has errors.

## How the server works

- It speaks the protocol on standard input and output, and keeps a whole copy of each open
  document.
- An edit only re-reads the document's names. The whole program is analyzed again when edits pause
  for 200 ms, or before a request that needs it: hover, definition, references and rename. Fifty
  edits in a row are one analysis, so typing stays fast in large programs.
- Completion answers from the last analysis, since a line being typed (`p.`) rarely compiles.
- When the language's compiler panics, the server reports it as an error in the document
  ("internal error in the ... compiler") and keeps running.
- Positions count UTF-16 code units, as the protocol requires.

A language tests its server with `a0_driver::lsp::session` and `session_requests`, which send a
session of requests and return everything the server answered. Each example language has one, in
`examples/*/tests/lsp.rs`.

## Limits

The server finds what a name refers to from the grammar alone, without types. So:

- `p.x` goes to every member named `x`, of any type. Completion is the exception: it asks the HIR
  for `p`'s type.
- A name that a language declares without a grammar rule has no definition. Lox's fields are an
  example, since they are created by assigning to them in methods.
- A member name that several types declare cannot be renamed.
