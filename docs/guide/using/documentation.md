# Documentation folders

## Documentation folders

Each project carries a nested tree of markdown docs. Organize them into folders, add files (inline
or from disk, at any depth), and browse the tree in the monitor:

```bash
kanbanr doc folder design --name "Design" --description "Architecture & design notes"
kanbanr doc add design/overview.md --content "# Overview\n…"
kanbanr doc add design/customer/portal.md --file ./portal-notes.md   # nested, any depth
kanbanr doc tree
```

### Diagrams & images in docs

Docs are more than text — the viewer renders **diagrams** and **embedded images**:

- **Embedded images.** Add an image as a binary asset, then reference it by a **relative name** from
  a markdown file in the *same folder* (names resolve **per-folder**, so they're meaningful in any
  docs folder — not just one):

  ```bash
  kanbanr doc add design/architecture.png --file ./architecture.png   # store the image asset
  ```
  ```markdown
  <!-- in design/overview.md (same folder) -->
  ![Architecture](architecture.png)
  ```
  The view daemon serves the asset from your data folder — nothing leaves your machine.

- **Mermaid diagrams.** A fenced `mermaid` code block renders to a live diagram (flowchart,
  sequence, state, gantt and the rest of Mermaid's types), drawn by [Pele](https://pele.run) in
  the monitor's light or dark colours:

  ````markdown
  ```mermaid
  flowchart LR
    A[Claude] --> B[kanbanr CLI] --> C[(data/ git repo)]
  ```
  ````

- **Diagrams are checked when they are written** (FEAT-200). Every `mermaid` block in a spec or a
  markdown doc is linted on `feature add`/`edit`, `doc add` and `batch`, and the problems are
  printed as warnings — the write still succeeds:

  ```text
  created FEAT-012 (Planned)
    diagram: FEAT-012 spec, diagram #1, line 14: `;` ends the statement early, so the rest of the message is lost — use a comma or words (semicolon_in_text)
  ```

  `kanbanr doctor` lists every problem on the board, and a gate can require `diagrams_valid` (see
  [Processes](processes.md#the-checks)). A project that wants a broken diagram refused outright turns
  on strict mode with `kanbanr config diagrams --strict on`; a refused write changes nothing, and a
  batch carrying one changes nothing at all.

  The lint catches the common mistakes, each named by its rule: an empty block or one with no
  diagram type (`empty_block`, `unknown_type`); a `;` with text after it in a sequence message, note
  or alias, or a class relation label (`semicolon_in_text`); `" | { } ( ) [ ]` in an unquoted
  flowchart label, or a label never closed (`label_punctuation`, `unbalanced_label`); `end` as a
  node or participant id (`end_as_id`); a `subgraph`, `alt`, `loop` or other block not closed by
  `end`, or unbalanced `{}` (`unbalanced_block`); and a sequence line that is no statement
  (`unknown_statement`). It flags only what [Pele](https://pele.run), which draws the monitor's
  and the book's diagrams, rejects — Pele is the authority, and the review page runs its real
  parse before Approve.

  Two habits avoid nearly all of it: **quote flowchart labels** — `A["f(x) {y}"]`, not `A[f(x)]` —
  and **keep sequence messages to plain words**, without `;`, braces or quotes. GitHub's own Mermaid
  is stricter than Pele: it rejects `A[x; y]`, which Pele draws, so quoting is the advice even where
  the lint is silent.

- **Any other diagram tool** — PlantUML, Graphviz/DOT, D2, Excalidraw, draw.io — works too: export
  it to **PNG/SVG** and embed it as an image asset (as above). Mermaid blocks also render natively on
  GitHub, so the same docs look right in your repo.
