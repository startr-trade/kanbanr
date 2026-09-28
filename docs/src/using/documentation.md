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
  sequence, state, etc.), theme-aware:

  ````markdown
  ```mermaid
  flowchart LR
    A[Claude] --> B[kanbanr CLI] --> C[(data/ git repo)]
  ```
  ````

- **Any other diagram tool** — PlantUML, Graphviz/DOT, D2, Excalidraw, draw.io — works too: export
  it to **PNG/SVG** and embed it as an image asset (as above). Mermaid blocks also render natively on
  GitHub, so the same docs look right in your repo.
