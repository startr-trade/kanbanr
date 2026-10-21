# HTTP API

`kanbanr serve` runs the view daemon: a small HTTP server over the board folder that answers the
monitor and any tool built on the board. It **reads only** — writes happen in the CLI — has no
login, and binds localhost by default.

Its read routes come in two kinds:

- **Public** routes are the read API, version 1. They are described by an
  [OpenAPI 3.1 document](#the-openapi-document), which the daemon also serves at
  `GET /api/openapi.json`, and they are covered by the [stability promise](../project/stability.md):
  a route, a field or a value documented there keeps its meaning in every 1.x.
- **Internal** routes are the other reads the daemon answers. They work, the CLI and the monitor
  use some of them, and they may change in any release. Build on them at your own risk.

Write routes, which exist only on a daemon started with `--allow-writes`, are part of neither.

## Public routes (v1)

The project views are all under `/api/projects/{project}`; `{code}` is an item's code, such as
`FEAT-001`. The table is generated from the OpenAPI document, which is generated from the Rust
types the routes answer with, so it cannot fall behind them.

<!-- generated from openapi.json: start -->

| Route | Parameters | Answers | What it is |
|---|---|---|---|
| `GET /healthz` | — | JSON `Health` | Liveness probe |
| `GET /api/meta` | — | JSON `Meta` | What this daemon can do, and who an approval here would be attributed to |
| `GET /api/openapi.json` | — | JSON `object` | This description of the public read API |
| `GET /api/projects` | — | JSON `ProjectSummary[]` | The projects on the board |
| `GET /api/projects/{project}` | — | JSON `Project` | A project: its configuration, milestones and items |
| `GET /api/projects/{project}/charter` | — | JSON `Charter` | The project's charter: purpose, goals, scope and constraints |
| `GET /api/projects/{project}/readiness` | — | JSON `ProjectReadiness` | What each live item is missing, and its next stages, keyed by code |
| `GET /api/projects/{project}/features/{code}/readiness` | — | JSON `ReadinessView` | What one item is missing, and what its next stages ask |
| `GET /api/projects/{project}/features/{code}/diagrams` | — | JSON `ItemDiagrams` | The Mermaid blocks of an item's spec and affects_docs, with the lint's findings |
| `GET /api/projects/{project}/review` | — | JSON `ReviewEntry[]` | What is waiting for a person's agreement, in the order to ask it |
| `GET /api/projects/{project}/releases` | — | JSON `ReleaseReport[]` | The project's releases, each with what it ships |
| `GET /api/projects/{project}/sprints` | — | JSON `Sprint[]` | The project's sprints |
| `GET /api/projects/{project}/sprints/{sprint}` | — | JSON `SprintReport` | One sprint's report |
| `GET /api/projects/{project}/adrs` | — | JSON `Adr[]` | The project's architecture decision records |
| `GET /api/projects/{project}/lessons` | `for` (optional), `all` (optional) | JSON `Lesson[]` | The lessons worth showing, most believed first |
| `GET /api/projects/{project}/process` | — | JSON `ProcessStatus` | How the project's workflow stands against the process it was applied from |
| `GET /api/projects/{project}/gates` | — | JSON `object` | The gate each status's moves are judged by, keyed by status: role-addressed gates resolved, the built-in rule included |
| `GET /api/projects/{project}/releases/{version}/cut-preview` | — | JSON `CutPreview[]` | What cutting a release now would ship and carry over, with nothing written |
| `GET /api/projects/{project}/releases/{version}/plan` | `stage` (optional), `reason` (optional) | JSON `ReleasePlan` | What promoting a release would do, with nothing changed: the button, each step and condition, what is missing and, for the cut, what ships. The monitor reads the code repository only when started in the project folder |
| `GET /api/projects/{project}/workflow` | `format=mermaid` | `text/plain` | The workflow as a Mermaid state diagram |
| `GET /api/projects/{project}/gantt` | — | `text/plain` | The schedule as a Mermaid gantt diagram, by milestone |
| `GET /api/projects/{project}/docs` | — | JSON `DocFolder` | The project's documentation tree |
| `GET /api/projects/{project}/docs/content` | `path` | `text/markdown` | A document's text |
| `GET /api/projects/{project}/docs/raw` | `path` | the file's bytes | A documentation file's bytes, such as an image |
| `GET /api/projects/{project}/activity` | `ref` (optional), `ongoing` (optional), `limit` (optional) | JSON `Activity[]` | Recent changes to the project, newest first |
| `GET /api/projects/{project}/features/{code}/export` | `format=md\|json` (optional) | JSON `FeatureExport`, or `text/markdown` | One item, as Markdown, or as JSON with `?format=json` |
| `GET /api/portfolio` | — | JSON `PortfolioView` | The workspace: its programs and their projects |
| `GET /api/portfolio/rollups` | — | JSON `RollupReport` | Progress rolled up by program and project |
| `GET /api/portfolio/board` | — | JSON `BoardReport` | Every project's items on one board, in normalized lanes |
| `GET /api/projects/{project}/events` | — | `text/event-stream` | Server-sent events: `changed` whenever this project changes |
| `GET /api/events` | — | `text/event-stream` | Server-sent events: `changed` whenever anything on the board changes |

<!-- generated from openapi.json: end -->

## Versioning

Version 1 is the paths in use today, under `/api/...`. Additions — a new route, a new field, a new
value where the document lists the values — come in minor releases and do not change the version:
a client must ignore fields it does not know. A change that would break a client is published as a
new version under `/api/v2/...`, with v1 still served for a deprecation period, as the
[stability policy](../project/stability.md) describes. `info.version` in the document names the
version it describes.

## Errors

A failed request answers with a status and a one-line, plain-text reason
(`text/plain; charset=utf-8`):

| Status | When |
|---|---|
| `400` | The request is malformed: a missing or invalid parameter, or an unsupported `format`. |
| `404` | No such project, item, sprint or document — or no route answers the path. |
| `409` | The board was written by a newer kanbanr than the daemon. Upgrade the binary and restart `kanbanr serve`. |
| `500` | The board could not be read. |

A `GET` of a path under `/api` that no route answers is a `404`; releases before the read API was
published answered `400`, "unsupported operation". Any other method on a read route is a `405`.

## Live updates

`GET /api/events` and `GET /api/projects/{project}/events` are
[server-sent event](https://html.spec.whatwg.org/multipage/server-sent-events.html) streams
(`text/event-stream`). Each event is `changed`, and its data is the id of the project whose files
changed, or `*` for a change to the board as a whole (a project added or removed). The
project stream sends only that project's events and `*`. The daemon sends a keep-alive comment
now and then; a client re-reads what it shows when an event arrives.

## CORS and access

The daemon sends permissive CORS headers, so a monitor served from another origin, such as the
standalone bundle each release attaches, can read the API and its SSE streams. CORS is not access
control: anyone who can reach the daemon can read the board. See
[Hosting the monitor elsewhere](../using/the-monitor.md#hosting-the-monitor-elsewhere).

## Internal routes

Served, not covered by the contract, and free to change: the doctor (`/api/doctor`,
`/api/projects/{project}/doctor`), query (`/api/query`, `/api/projects/{project}/query`), the
dependency views (`/ready`, `/blocked`, `/graph`, `/critical-path`, `/features/{code}/impact`, and
their portfolio-wide forms), the portfolio Gantt (`/api/gantt`), trace and Zachman coverage, the
retrospective and report, the issue mirror's settings, the CLAUDE.md block, the process library
(`/api/processes`), the project-wide export (`/api/projects/{project}/export`), `/git/model`,
`/git/doctor`, `/config/method`, the guard rails (`/api/projects/{project}/guards`) and the agent
scopes (`/api/projects/{project}/scopes`, `/scopes/{id}`, `/scopes/{id}/brief` in Markdown, and
`/scopes/drift`, which reads the code repository; FEAT-214). Each answers JSON, except the DOT graph (`?format=dot`) and the Markdown
project export, which are text.

## Writes happen in the CLI (`dispatch`)

All mutations go through `kanbanr-core::dispatch` from the CLI (and the same `(method, path,
body)` shapes the daemon would *read*): create/edit projects, features (never deletable), tasks,
todo-lists, milestones, config, docs — plus `POST /projects/:p/batch` for a bundle in one commit.
Each write appends to the activity log and commits the data repo. The **batch** body is
`{ "operations": [ { "op": "...", ... } ] }` with op types `feature.add`, `feature.edit`,
`feature.move`, `milestone.add`, `todo.add`, `task.add`, `task.state`, `doc.folder`, `doc.write`;
created items may carry a `ref` alias later ops reference; the first failure names its index.

A daemon started with `--allow-writes` takes the same writes over HTTP, under `/api/write/...`,
for the monitor's approval and release actions. Those routes are not part of the read API.

## The OpenAPI document

The document is generated, never edited by hand:

```sh
cargo run -p kanbanr-core --features schema --example openapi          # write it
cargo run -p kanbanr-core --features schema --example openapi -- --check  # CI: fail if stale
```

It is committed as `docs/src/reference/openapi.json`, and the daemon serves those same bytes. A
contract test starts a daemon on a fixture board and checks every public route against it: the
status, the media type, and that each JSON body validates against its schema.

<details>
<summary>openapi.json (v1)</summary>

```json
{{#include openapi.json}}
```

</details>
