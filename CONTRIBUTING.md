# Contributing

The contributing guide is a chapter of the documentation book, so it sits beside the architecture
and method it refers to:

**<https://kanbanr.startr.trade/project/contributing.html>**

or, in this repository: **[docs/src/project/contributing.md](docs/src/project/contributing.md)**.

In short — the bar is the same for everyone, including the maintainer:

- state **why** the work exists and link a charter goal;
- carry at least one **requirement**, in EARS form;
- every requirement has **passing test evidence**;
- no unsupported claims — a measure names the test that checks it.

`kanbanr check --file <your-definition.yaml>` validates a contribution without needing board access.

In the code: every board file is read and written through `kanbanr_core::yaml` (never a YAML crate
directly), and a stored revision hashes `kanbanr_core::hash::canonical_json`, never YAML text.
