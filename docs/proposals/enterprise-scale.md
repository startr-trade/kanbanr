# Proposal: Enterprise-Scale Coordination (multi-project, cross-dependencies)

**Status:** Proposal / handoff for the primary implementation session
**Audience:** the VS Code session working in this repo
**Goal:** evolve kanbanr from a single-project tracker into a portfolio-scale coordination
system — many projects, cross-project dependencies, derived "what's ready/blocked", and safe
concurrent use — **without abandoning the git-backed, human-readable, file-per-entity design**,
which is a genuine strength.

---

## 0. Preserve these (they are why it works)

- **Git-backed, plain YAML/MD, file-per-entity.** Diffs, history, offline, no DB to operate.
- **Status-as-folder** layout; **spec in a separate `.md`**.
- **Batch = one git commit.** Atomic-ish bundles.
- **Read-only view daemon** already exists (`kanbanr-server`) with an auth concept
  (`security.yaml` / JWT) — a foothold for a write daemon later.

The recommendations *layer on top*; they do not replace the store.

---

## 1. Current architecture (as built) and the scale limits

| Area | Where | Limit at enterprise scale |
|---|---|---|
| Model has no layer above Project | `store.rs::Project`, `projects/<id>/` flat | No program/portfolio grouping or cross-project rollup |
| `depends_on` is **project-local** | `models.rs` (`FeatureItem.depends_on`, `Milestone.depends_on`); `validate.rs::validate_dag` builds the node set from **one** project | **No cross-project dependencies** — the headline gap |
| Dependencies are stored but **never derived into state** | nothing consumes `depends_on` for readiness | No "blocked/ready", no critical path, no impact analysis |
| No ownership | `FeatureItem` has `labels` only; "who" = git commit author | Can't coordinate by owner/team or see workload |
| Full-project load on **every** op | `store.rs::load` reads every status folder + **every spec `.md`** (`read_features`) | O(N) file reads per mutation |
| Batch reloads per op | `store.rs::apply_batch` calls `self.add_feature`/`set_feature_attrs`/… each of which calls `self.load(id)` again | A K-op bundle ≈ **K+ full project loads** |
| Per-mutation commit **and push** | `backend.rs:86-98` → `git::commit_local` + `git::sync_all` every call; **no lock** | Two sessions interleave writes → git divergence; push-per-write is slow |
| Query surface is thin | CLI filters by status/milestone only | No filter by label/owner/dep-state, no full-text, no cross-project view |

---

## 2. Recommendations by theme (code-anchored)

### A. Cross-project dependencies (the core ask)

1. **Qualified references.** Allow a dependency entry to be `"<project>:<code>"`
   (e.g. `simplr-cluster:FEAT-004`). A bare `FEAT-004` keeps meaning "same project".
   No schema change to the YAML — `depends_on: Vec<String>` already holds strings; only the
   *interpretation* and *validation* change.
2. **Global graph resolver.** Add `graph.rs` in `kanbanr-core` that can load **N projects**
   (lazily, metadata-only — see §D) and build a combined `(node_id, [dep_id])` set where
   `node_id = "project:code"`. Generalize `validate.rs::validate_dag` to run over this global set so
   **cycle detection and existence checks span projects**.
3. **Validation touchpoints.** `set_feature_attrs` / `add_milestone` / `edit_milestone` currently
   call the project-local validators; when a dep is qualified, route through the global resolver
   instead. Reject a qualified dep whose project or code doesn't exist.
4. **Referential integrity.** Add `kanbanr doctor` (portfolio-wide) that flags dangling
   cross-project refs (referenced project/code renamed or moved).

### B. Derived dependency state — the coordination payoff

Dependencies are only useful if they *drive a view*. On top of the global graph:

- **Readiness:** a node is **blocked** if any (transitive or direct — pick direct first) dep is not
  in a terminal/`Completed`/no-op state; **ready** when all deps are `Completed`.
- New read commands: `kanbanr ready [--project|--portfolio]`, `kanbanr blocked`,
  `kanbanr graph [--format dot|json]`, `kanbanr critical-path`, `kanbanr impact <id>`
  (downstream closure: "if this slips, what's affected").
- Surface "**what's unblocked now**" in the web view — the single highest-value coordination screen.
- Pure read-side; no storage change.

### C. Hierarchy above Project: Program / Portfolio

- Add a root-level `workspace.yaml` (or `portfolios/<id>.yaml`) declaring **programs** and which
  **projects** belong to them, plus portfolio metadata. Projects stay exactly as they are; this is
  an **index**, not a move.
- Enables cross-project **board / schedule / roadmap** and **progress rollups**
  (milestone % → project % → program % → portfolio %), computed from existing task/feature state.
- `project.rs::resolve_project` stays; add `resolve_portfolio` + `--portfolio` / `--all-projects`
  flags on read commands.

### D. Performance & scale of the store

1. **Load the project once per batch.** Refactor `apply_batch` to `load(id)` a single mutable
   `Project`, apply every op **in-memory**, validate once, then persist only changed files. This
   removes the K× reload and is a safe, high-impact win. (Today each sub-call reloads.)
2. **Lazy spec loading.** `read_features` eagerly reads every `features-spec/<code>.md`. Split
   metadata load (yaml only) from spec load; board/list/graph/validation never need spec bodies.
   Load a spec on demand (`feature show`, `export`).
3. **Per-project index.** Maintain a small `index.yaml` (code → status/milestone/labels/owner/deps,
   no spec) so listing, the global graph, and rollups read **one file per project** instead of N.
   Rebuildable from the source of truth; treat as a cache.

### E. Concurrency & the "primary session" problem

Today every CLI call commits **and pushes**, with no lock — two VS Code sessions on the same data
dir will diverge.

1. **Write lock (cheap, immediate).** Take an OS advisory file lock on the data dir (or per-project)
   around the mutate→commit sequence in `backend.rs`. Serializes concurrent writers safely.
2. **Single-writer daemon (the real answer).** The dispatch layer is already a REST-ish
   `method + path + body` interface (`dispatch::dispatch`), and `kanbanr-server` already runs.
   Promote it to accept **writes** over a local socket/HTTP (reuse the JWT/`security.yaml` auth),
   hold projects in memory, **serialize all mutations**, and let every session/agent be a thin
   client. This is how "one primary session coordinates many" should work — make the daemon the
   single writer, not one of the VS Code windows.
3. **Debounce the push.** Keep per-mutation **commit** (cheap, local) but move `git::sync_all`
   (network push/pull) off the hot path — debounce to every N seconds / on idle, or a `kanbanr sync`
   command + background pusher. Per-write push is the main latency and conflict source at scale.

### F. Ownership, querying, reporting (coordination ergonomics)

- **Add `assignee`/`owner` and `team`** to `FeatureItem` (optional `Option<String>` fields;
  backward-compatible with `#[serde(default)]`). Optional `people.yaml`/`teams.yaml` registry at the
  portfolio level. Enables by-owner/by-team views and WIP/workload.
- **Richer query**: `kanbanr query` with filters on label/owner/team/kind/priority/due-range/
  **dep-state (ready|blocked)** + **full-text** over titles/specs, with `--all-projects`.
- **Reporting**: rollup %; aging/at-risk (due vs status); burndown/velocity from `created_at`/
  `updated_at` + the existing `activity` changelog.
- **Eventing (optional)**: emit on state change (esp. a dep reaching `Completed` → dependents become
  ready) via a webhook/hook so cross-team handoffs notify.

### G. Schema versioning & access scope

- Add a `schema_version` to `config.yaml` and a tiny migration step so the model can grow safely
  across a large existing dataset.
- **Access scope** for multi-team: today the whole data dir is one git repo / one remote set. Two
  options to document and pick: (a) keep the **monorepo** and scope writes via the daemon's auth;
  (b) **per-project git repos** (each `projects/<id>` its own repo or submodule) so teams own their
  data and the portfolio index ties them together. (a) is less disruptive; (b) gives real per-team
  access boundaries.

---

## 3. Suggested phasing

**Phase 1 — make scale safe & cross-deps real (highest value):**
- D1 (batch loads once) · A1–A3 (qualified cross-project deps + global graph) ·
  B (ready/blocked/graph commands) · E1 (write lock).

**Phase 2 — coordinate across projects:**
- C (portfolio index + cross-project board/rollups) · F (owner/team + query + reporting) ·
  D2–D3 (lazy specs + per-project index).

**Phase 3 — operate at scale:**
- E2 (single-writer daemon) · E3 (debounced push) · B-extended (critical path/impact) ·
  scheduling/dates · F-eventing · G (versioning + access scope) · `doctor`.

Phase 1 alone delivers the headline ask (cross-project dependencies + "what's ready now") and makes
concurrent two-session use safe.

---

## 4. Backward compatibility

- `depends_on` stays `Vec<String>`; qualified refs are a superset of today's bare codes.
- New `FeatureItem` fields use `#[serde(default)]` → old YAML loads unchanged.
- `workspace.yaml` / `index.yaml` are **additive**; absence = today's single-project behavior.
- The daemon is opt-in; local CLI mode keeps working.

---

_Generated as a recommendation for the primary session. Validate the cited line references against
HEAD before implementing — they reflect the repo as read during analysis._
