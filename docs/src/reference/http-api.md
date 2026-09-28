# HTTP API

## HTTP API (the view daemon — read-only, no auth)

The daemon serves **only reads** (writes happen in the CLI, not over HTTP). There is no auth; it
binds localhost by default.

| Method & path | Purpose |
|---|---|
| `GET /healthz` | liveness/readiness probe |
| `GET /api/projects` | project summaries (home tiles) |
| `GET /api/projects/:p` | full project (config, features, milestones) |
| `GET /api/projects/:p/features/:code/export?format=md\|json` | Claude-ready export |
| `GET /api/projects/:p/docs` · `…/docs/content?path=` | docs tree / a doc's markdown |
| `GET /api/projects/:p/activity` | recent activity (the changelog) |
| `GET /api/projects/:p/events` · `GET /api/events` | SSE change streams |

### Writes happen in the CLI (`dispatch`)

All mutations go through `kanbanr-core::dispatch` from the CLI (and the same `(method, path,
body)` shapes the daemon would *read*): create/edit projects, features (never deletable), tasks,
todo-lists, milestones, config, docs — plus `POST /projects/:p/batch` for a bundle in one commit.
Each write appends to the activity log and commits the data repo. The **batch** body is
`{ "operations": [ { "op": "...", ... } ] }` with op types `feature.add`, `feature.edit`,
`feature.move`, `milestone.add`, `todo.add`, `task.add`, `task.state`, `doc.folder`, `doc.write`;
created items may carry a `ref` alias later ops reference; the first failure names its index.
