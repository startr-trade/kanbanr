# Stability: what 1.0 promises

From 1.0.0, kanbanr follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html) on the
surfaces below: nothing listed as **stable** changes incompatibly before 2.0. This page is that
promise, written down so you can rely on it and contributors can check a change against it
(`ADR-0012` on the board records the decision).

Until 1.0.0 the feature set is frozen: the 0.1.x releases take fixes, documentation, and the work
that makes this promise checkable — no new features.

## Stable from 1.0

| Surface | What is kept |
|---|---|
| **The board on disk** | The folder layout (`projects/<id>/` with `config.yaml`, `charter.yaml`, `features/<status>/<CODE>.yaml`, `milestones/`, `docs/`, `lessons.yaml`, `sprints.yaml`, `releases.yaml`), the board's saved processes (`processes/<name>.yaml`), and the meaning of every documented field. A board written by any 1.x opens in every later 1.x. |
| **`config.yaml`** | Statuses, transitions, gates and their check vocabulary, cadence settings, `schema_version`, and the `process` record of where the workflow came from. A workflow file you wrote for 1.0 loads in every 1.x. |
| **Process files** | The workflow file format, its `process:` header (name, description, version), and the personal library's place, `~/.kanbanr/processes/` (or `KANBANR_PROCESSES_DIR`). |
| **The `.kanbanr` marker** | Its fields (`project`, `data_dir`) and how the board is found from it. |
| **CLI commands and flags** | Every documented command and flag keeps its name and meaning. Exit status: `0` on success and non-zero on failure, with `kanbanr check` (and `check --file`) exiting non-zero when something is missing, so scripts and CI can rely on it. |
| **`--json` output** | Every field a command's `--json` prints keeps its name, type and meaning. |
| **The read API** | Version 1 of the public read routes `kanbanr serve` answers: the routes, parameters, media types, fields and documented values that its [OpenAPI document](../reference/http-api.md#the-openapi-document) (`GET /api/openapi.json`) describes. A breaking change is published as v2 under `/api/v2/…`, with v1 kept for a deprecation period. |
| **The Claude Code integration** | The commit trailer (`Refs: kanbanr:FEAT-…`), the hook commands the skill and plugin register, and `kanbanr skill install` / `hooks install` writing them where Claude Code finds them. |

## Not covered

- **Human-readable output** — the wording, layout and colour of what commands print for people.
  Parse `--json`, not the text.
- **The monitor's look** — its pages, layout and URLs other than `/api/…`.
- **The daemon's internal routes** — the reads it answers that the OpenAPI document does not
  list (see [HTTP API](../reference/http-api.md#internal-routes)), and its write routes.
- **The skill's wording** — how SKILL.md phrases its guidance to Claude, as long as the commands it
  drives keep working.
- **Internal crates** — `kanbanr-core`, `kanbanr-server` and `kanbanr-cli` are not libraries;
  they are not published, and their Rust API changes freely. (`ears-classifier` is published and
  versioned on its own.)
- **Anything marked experimental** in its own documentation.
- **Undocumented behaviour**, including the order of items where no order is documented.

## How things change

- **Additions come in minor versions**: a new command, flag, `--json` field, gate check or board
  field. An older kanbanr reads a board with fields it does not know, and when it rewrites the file
  (a move, a task, an edit) it writes those fields back unchanged — so machines sharing a board can
  run different 1.x versions without one deleting what another recorded. See the next rule for a
  newer *format*.
- **A newer board is refused, never misread.** A board that needs a newer kanbanr says so with its
  `schema_version`, and an older binary refuses to open it rather than silently ignoring what it
  does not understand. `schema_version` rises only when an older binary would misread a board.
  Gates once stamped a board 3 and role labels 4. Schema 5 changed how every revision is computed,
  so every board is now stamped 5.
- **Revisions are canonical.** The `rev` stored by an approval, a sign-off or a project's `process`
  record is a hash of the record's canonical JSON: compact, with every key sorted. It does not
  depend on how the YAML is written, so quoting, indentation or a new YAML library never lapses an
  approval. Schema 5 introduced this (FEAT-187). A board from before schema 5 is refused by every
  command until `kanbanr migrate` has run. The migration re-pins each revision from the board's own
  history, in one commit; see [Upgrading a board to schema 5](../releases/schema-5.md).
- **Deprecation before removal.** Something stable is first deprecated in a minor release — the
  changelog says so, and using it prints a warning naming its replacement — and is removed only in
  the next major version.
- **Fixes may change behaviour that was wrong.** A patch release can change behaviour that
  contradicted this documentation; the changelog says what changed and why.
- **Security fixes come first.** If keeping a surface would keep users exposed, it changes in a
  patch release and the changelog says so plainly.

## For contributors

A change that touches a stable surface says so in its board item and in the changelog entry.
If it would break one, it waits for the next major version — or it is redesigned as an addition.
When unsure, ask on the item before building it.
