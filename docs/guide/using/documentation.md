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
    A[the agent] --> B[kanbanr CLI] --> C[(data/ git repo)]
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

### Query blocks (katalogr)

A doc can ask katalogr, the code index, for a view of the
code. katalogr is optional: nothing in kanbanr needs it, and every page works without it
(FTR-016, ADR-0014). The ask is a fenced block whose info string is `katalogr-query`; its body is one
line, an operation and its arguments:

````markdown
```katalogr-query
structure --level file --max-nodes 40
```
````

- **Operations.** `structure`, `contracts`, `cycles` (no argument), `callers`, `flow`, `impact`,
  `lineage`, `process` (one symbol) and `path` (from, to). A flag is `--name value` or
  `--name=value`, given once; a word with spaces is wrapped in double quotes.
- **What the monitor does.** The doc page asks `katalogr api` at the project's current position (the
  commit and whether the checkout is clean) and draws the answer as a diagram, with the position, whether
  it is exact, and the view's ID under it, and the one-line source below that. At most 20 blocks per
  doc are asked.
- **When katalogr is not there.** The block keeps its source and one plain sentence says why:
  "katalogr is not running at `<address>` — the block shows its source", "the katalogr at … is too old: it
  has no `views` capability", "no katalogr context holds `<position>`", or "this block is not a query:
  `<reason>`". Where katalogr is is environment, never board config: `KANBANR_KATALOGR_URL` (default
  `http://127.0.0.1:7878`), or `kanbanr serve --katalogr-url`. `kanbanr katalogr status` says whether
  it answers and at which position a block would be asked.
- **Checked when written.** `doc add`, `PUT …/docs/content` and a batch `doc.write` print each bad
  block as `query block: docs/<path>, line N: <reason>` and still write; `kanbanr doctor` lists the
  same under "Query blocks".
- **Release snapshots.** When `katalogr` is installed, the cut that tags a release runs
  `katalogr view snapshot` and stores the result under `docs/releases/<V>/views`, in the same board
  commit as the tag's record, and the release notes gain a "System views" section. The Releases page and the
  `promote` output name the repos the snapshot mentions: check them before you push a board that is
  public. With no katalogr, or a failed snapshot, the release ships and says why it has no views.

**The contract with katalogr.** The two tools share a few files, kept byte for byte under
`fixtures/contract/` at the repository root: `position-v1.json` (the position text), `views-v1.json`
(the view answer and the catalogue of operations), `query-block-v1.md` (the block grammar above) and,
read rather than copied, the three view names and link template of katalogr's `web-links-v1.json`. The
board's alias format, version 1, is part of it too: an item's own file carries `aliases:` directly after
`code`, so katalogr resolves a `kanbanr:<CODE>` reference under an item's earlier codes. kanbanr checks
a capability by name, never a version number. The link is outside the 1.0 promise until it has been used
([Stability](../project/stability.md)).
