# The system representation

This folder describes kanbanr as built, at the commit you are reading it from. It branches, merges
and is tagged with the code it describes, so any branch or release can show the system as it was
there.

It holds:

- **Component docs, as built.** One doc per component: what it does, its interfaces and the code
  it lives in. A change that alters a component updates its doc in the same branch.
- **Views.** Diagrams of how the components fit together, drawn as ```` ```mermaid ```` blocks like
  the book's.
- **Query blocks.** ```` ```katalogr-query ```` blocks that a renderer fills in from the code
  intelligence index, instead of a hand-kept list that drifts.

The front matter each doc carries (which items, ADRs and requirements it covers) comes with
FTR-016, which renders this folder. FTR-004 reads it at a position: a branch, a tag or a commit.

This folder is **not a chapter of the book.** The product guide, which readers of the published
book see, is `docs/guide`; nothing here is listed in its `SUMMARY.md` or published with it. Its
markdown and diagrams are linted as the guide's are (`npm run check:docs` in `web/`, and the
diagram lint test in `kanbanr-core`).
