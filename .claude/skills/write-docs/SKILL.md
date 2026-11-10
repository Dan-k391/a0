---
name: write-docs
description: Write or rewrite A0's documentation (README, docs/, the book, the tutorial) so that a reader who has never seen A0 understands it on the first read. Use when adding or changing any page a user reads.
---

# Write docs people understand

The goal is understanding, not style. A page is good when a reader who has never seen A0 can
follow it once, top to bottom, and do what it describes.

## Know the reader and the kind of page

Decide which kind of page you are writing before you write it (the Diátaxis framework), and keep
each page to one kind.

| Kind | Reader | Pages | Shape |
|---|---|---|---|
| Tutorial | new, learning | `README.md`, `docs/book/`, `docs/tutorial.md` | steps they follow, each with what they will see |
| How-to | has a task | how-to guides in `docs/book/` | the goal in the title, then the steps, nothing else |
| Reference | looking something up | `ir.md`, `hir.md`, `grammar.md`, `std.md`, `diagnostics.md`, `c-api.md`, `python.md` | complete and dense is fine, but open with an example |
| Explanation | wants to know why | `decisions.md`, design sections | the problem, the choice, what it costs |
| Maintainers | works on A0 | `milestones.md`, `releases.md`, `AGENTS.md`, skills | this skill does not apply |

## Rules

1. **Example first.** Open every page, and every section that introduces something, with the
   smallest thing the reader can run or read, and what it does. Explain the rule after that.
2. **One idea per sentence.** Split a sentence if it has brackets inside brackets, a semicolon, or
   a colon followed by a list. Put three or more options in a list or a table.
3. **Define a term before you use it.** The first time a page says HIR, JIT, target pack, grammar
   rule or any other A0 word, give a plain one-line meaning or link to the page that defines it.
4. **Say what the reader does and what they will see.** Commands go in code blocks, followed by
   what they print or make. Copy real output from running them; never write output from memory.
5. **Lead with the reader's goal, not the implementation.** Write "To build for Windows, ...", not
   "`Target::parse` accepts ...". Name a file, crate or function only when the reader needs to
   open it. Decision numbers (D46) belong in `decisions.md` and maintainer pages, not user pages.
6. **No notes to maintainers in user pages**: no "Status: implemented", no "Keep this document in
   sync". Those rules live in `AGENTS.md`.
7. **Check every fact against the code or by running it.** Don't copy a claim from another page;
   pages drift.
8. **One home per fact.** Link to it instead of repeating it, so a change needs one edit.

Longer is fine when it is clearer. Cut words that carry nothing ("simply", "just", "note that"),
not words that carry steps.

## A0's checks on its docs

- `docs/tutorial.md`'s listings are `ANCHOR` regions of Kaleidoscope's code, and its commands run.
  Change the code, then `A0_BLESS=1 cargo test -p kaleidoscope --test tutorial`, then reread the
  prose around each listing.
- `docs/diagnostics.md`'s table of error codes is tested against `a0_diag::codes`.
- `crates/a0/tests/docs.rs` checks that relative links point to files that exist and that
  `docs/SUMMARY.md` lists every page in `docs/book/`. A new book page goes into `SUMMARY.md`.
- Docs describe what `docs/compatibility.md` promises; rewording a page must not change a promise.

Run `cargo test -p a0-cli --test docs` (and the tutorial test, if you touched it) before
committing.

## Before you finish

- Run every command on the page, in a fresh directory, as written.
- Read the page top to bottom as someone who has never seen A0. Stop at each term they would not
  know, each step that assumes something not yet said, and each sentence you had to read twice,
  and fix it.
- Look again at sentences over 30 words; most should be split.
