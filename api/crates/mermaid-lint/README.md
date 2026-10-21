# mermaid-lint

Lint the Mermaid diagrams in a Markdown file without a browser. It finds the mistakes Mermaid
renderers reject — a `;` that cuts a sequence message in half, punctuation in an unquoted flowchart
label, `end` used as a node id, a `subgraph` or `alt` never closed — and says which diagram and
which file line each one is on. Pure Rust, no dependencies, linear time.

```rust
use mermaid_lint::{blocks, lint_markdown, LintRule};

let markdown = "# Design\n\n```mermaid\nsequenceDiagram\n  A->>B: save; then reply\n```\n";

let found = lint_markdown("design.md", markdown);
assert_eq!(found.len(), 1);
assert_eq!(found[0].rule, LintRule::SemicolonInText);
assert_eq!(found[0].line, 5);
println!("{}", found[0].describe());
// design.md, diagram #1, line 5: `;` ends the statement early, so the rest of the message is
// lost — use a comma or words (semicolon_in_text)

// The blocks themselves, with the file line each starts on.
assert_eq!(blocks(markdown)[0].line, 4);
```

## What it checks

| Rule | Reported when |
|---|---|
| `empty_block` | the block holds no diagram |
| `unknown_type` | the first line names no diagram type (`flowchrt`, `FLOWCHART`, a `title` before the type) |
| `semicolon_in_text` | a `;` with more text after it in a sequence message, note or participant alias, or a class relation label |
| `label_punctuation` | an unquoted flowchart node, subgraph or edge label holds `" \| { } ( ) [ ]`, or a quoted one has text after its closing quote |
| `unbalanced_label` | a label or a quote is never closed |
| `end_as_id` | `end` is used as a flowchart node or subgraph id, or as a message's participant |
| `unbalanced_block` | a `subgraph`, `alt`, `loop`, `opt`, `par`, `critical`, `break`, `rect` or `box` is not closed by `end`; an `end` closes nothing; `else`/`and`/`option` is outside its block; a `{` is never closed |
| `unknown_statement` | a sequence-diagram line is no statement, such as `A->>B` without its `:` |

Blank lines, `%%` comments and directives, and a leading `---` front-matter block are skipped
before the type. Diagram types other than flowchart, sequence, class and state are checked for
their type only.

The usual fix for a label is to quote it: `A["f(x) {y}"]`. For a sequence message, plain words.

## It follows Pele

The rules are held to one property: **they flag only diagrams that
[Pele](https://pele.run) 1.7.0's `parse` rejects, never one it accepts.** A finding a renderer
contradicts would block work on a diagram that draws fine. So some mistakes a stricter renderer
rejects are deliberately not flagged — an unquoted flowchart label holding `;` (`A[x; y]`), or
quotes and braces in a sequence message. The fixture `src/testdata/diagram_lint_cases.json` holds
each case from both sides: the crate's tests check the lint against it, and a script in the
repository parses every case with the real Pele.

It is not a Mermaid grammar. It catches the common mistakes cheaply, where a real parse needs a
JavaScript runtime; a real parse stays the authoritative check.

## Untrusted input

A total function over `&str`: no regex (no ReDoS), no recursion, one pass per line, and at most
`MAX_FINDINGS_PER_FILE` (50) findings per file. Two MiB of Markdown with a thousand diagrams lints
well under a second in a debug build.

## Features

- `serde` — `Serialize` and `Deserialize` on `MermaidBlock`, `LintRule` (as its snake_case name)
  and `Finding`.

## Where it comes from

Extracted from [kanbanr](https://github.com/startr-trade/kanbanr), which lints every diagram written
into a spec or a doc, reports them in `kanbanr doctor`, and can require them clean before an item
moves. It has no dependency on kanbanr.

## License

MIT OR Apache-2.0.
