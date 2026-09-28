# Reasoning, evidence and traceability

## Reasoning, evidence and traceability (MS-006)

The board above records *what* is being built. This layer records **why it exists, what must be
true, and what proves it** — and, deliberately, records nothing it cannot derive.

### The graph

```
charter.purpose
  └── G-2  goal
       └── FEAT-046  item              definition.goals: [G-2]
            ├── R-2  requirement        (EARS text, ISO tag, measured scenario)
            │    └── test               (planned → red → green, stamped with checked_rev)
            ├── FEAT-058  defect        violates: FEAT-046/R-2   introduced_by: FEAT-046
            ├── design/mirror.md        refs: [FEAT-046, R-2, G-2]
            ├── ADR-0003                affects: [FEAT-046]  driven_by: [FEAT-046/R-2]
            └── code / commits          // … (FEAT-046 R-2)  ·  Refs: kanbanr:FEAT-046/R-2
```

Three rules keep it from rotting:

1. **The manifest is canonical** — board yaml, document front-matter, and the commit trailer. An id
   that does not resolve is an *error*, like a dangling dependency.
2. **Inline annotations are a derived convenience** — `(FEAT-046 R-2)` on the module or function
   that owns the behaviour. They survive the refactors that destroy `git blame`; they are never the
   only record.
3. **Views are derived, never stored** (ADR-0002). `kanbanr trace --json` generates the
   traceability manifest for CI or an audit; nothing writes it back. A stored manifest would be a
   third copy of links that already exist, and the copy that goes stale first.

### Modules

| Module | What it owns |
|---|---|
| `charter` | the project's purpose and goals — a side file, absent by default |
| `ears` | the five EARS patterns and the nine ISO/IEC 25010 characteristics; classifies, never rejects |
| `doctor` | every structural check, all warnings except broken references; scoped so it stays readable |
| `report` | flow and quality derived from history, defects and test states |
| `retro` | a wave's account, with changelog-derived spans reported apart from measured ones |
| `lessons` | what was learned, with confidence that decays unless reaffirmed |
| `scm` | the trailer grammar, branch naming and reference validation |
| `adr` | decisions as documents with front-matter; supersede is the one two-sided link |
| `trace` | the downward chain and its gaps, and the derived Zachman view |
| `query`, `graph`, `gantt`, `portfolio`, `mirror`, `eventing` | search, dependencies, scheduling, rollups, the GitHub mirror, notifications |
| `validate`, `hash`, `error`, `docs` | id generation, stable hashing, error taxonomy, the doc tree |

### Two deliberate asymmetries

**Approval is pinned to content, not to time.** `definition.approval.rev` is a stable hash of the
definition with the approval itself *and the test states* excluded — recording evidence must not
lapse an approval, but changing scope must. Editing the definition after a yes therefore reports
"approval lapsed", which is a different thing from "never approved".

**The gates refuse two things and warn about everything else.** A reference that names something
the board does not have, and a commit that names nothing, are refused — both fixable in the message
the author is already writing. Everything else warns, because a check that blocks legitimate work
gets bypassed wholesale (ADR-0004), and a check that fires on everything gets ignored.


## Workflow contract (how Claude uses it)

kanbanr is designed to be the **single system of record** for a project's activity — the only
project information that stays outside it is the raw conversation transcript. The skill
(`skill/kanbanr/SKILL.md`) encodes the behavioral contract: adopt kanbanr for the whole project
on one trigger phrase ("start using kanbanr for this project"); never track work in an
ephemeral/in-session list (use persistent todo-lists on feature items); recover/resume state from
kanbanr at the start of each session; keep it updated before and after every task; review a
feature's spec for staleness when moving it out of `Deferred`; never delete feature items (move
them, e.g. to a no-op state); model every kind of work as a work item (feature/chore/recurring,
ongoing items in a non-displayed status); and prefer one `batch` call to bundle changes. The
durable, file-backed data model makes the project resumable across sessions by design.
