# Sharing and the live monitor

## Sharing & the live monitor

**Sharing/centralization is the git remote's job** — whoever can pull/push the data repo is in:

```bash
kanbanr remote add origin git@github.com:org/board.git   # SSH (agent or ~/.ssh key) or HTTPS
kanbanr sync                                             # push now
```

How often the board is pushed is a **setting of the board** on this machine, kept in its own git
config — never pushed, never forced on anyone else who clones the code (FEAT-142):

```bash
kanbanr remote push-policy                # show it, where it came from, and how far ahead the board is
kanbanr remote push-policy debounce:10    # the default: push once 10 commits ahead
kanbanr remote push-policy auto           # push after every change
kanbanr remote push-policy off            # only `kanbanr sync` pushes
```

`KANBANR_PUSH=…` overrides the setting for one command. `kanbanr doctor` warns when the board is
further ahead of its remote than its policy allows.

### Conflicts & not losing data

kanbanr is safe by construction: **every change is committed to your local data repo *before* any
remote sync**, so a remote problem can never lose your work. If a push can't go through (the remote
moved on, or a real merge conflict), kanbanr **does not auto-resolve** — it prints a warning with
the exact commands and leaves it to you:

```
kanbanr: remote 'origin': could not push (…). Your change is committed locally, so nothing is
lost. To sync, resolve in the data folder with normal git:
    git -C <data-dir> pull --no-rebase origin <branch>
    git -C <data-dir> push origin <branch>
```

Because the data folder is a **plain git repository**, you resolve exactly as you always do —
`git pull`, fix conflicts (e.g. `git mergetool`), commit the merge, `git push`. Then keep using
`kanbanr` normally. (Tip: for shared data, treat it like code — pull before a work session.)

**The monitor** is a separate, read-only view of your local folder — localhost, no login:

```bash
kanbanr serve                       # built into the binary: no --ui-dir, no Node
kanbanr open                        # opens http://localhost:8080 — just the board
```

Expose it beyond localhost only behind a reverse proxy you control.

### Exposing the monitor beyond localhost

`kanbanr serve` binds `127.0.0.1` and has **no auth or TLS by design** — it's a local, read-only
window onto your own files. To reach it from another machine, **don't change the bind**; instead put
a **reverse proxy** in front that terminates **TLS** and adds **authentication**, proxying to the
unchanged `127.0.0.1:8080`. (kanbanr deliberately ships none of this — sharing is otherwise the git
remote's job; see [the architecture overview](../architecture/overview.md).) Name the proxy's host in
`KANBANR_ALLOWED_HOSTS`. To take decisions through the proxy as well, name the origin the browser
sees in `KANBANR_ALLOWED_ORIGINS`; see below. Two minimal working examples:

**Caddy** (automatic HTTPS + basic auth) — `Caddyfile`:

```caddy
board.example.com {
    basic_auth {
        # generate the hash with: caddy hash-password
        you $2a$14$REPLACE_WITH_BCRYPT_HASH
    }
    reverse_proxy 127.0.0.1:8080
}
```

**nginx** (TLS + basic auth) — server block:

```nginx
# create the password file: htpasswd -c /etc/nginx/.htpasswd you
server {
    listen 443 ssl;
    server_name board.example.com;

    ssl_certificate     /etc/ssl/certs/board.example.com.crt;
    ssl_certificate_key /etc/ssl/private/board.example.com.key;

    location / {
        auth_basic           "kanbanr monitor";
        auth_basic_user_file /etc/nginx/.htpasswd;

        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        # SSE: stream live updates without buffering or timing out
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_read_timeout 1h;
    }
}
```

The proxy enforces who gets in and encrypts the connection. The monitor answers only for the
loopback names (`localhost`, `127.0.0.1`, `[::1]`, on any port, so a port published by Docker works), the address it is bound to and the hosts named in
the `KANBANR_ALLOWED_HOSTS` variable, so name the proxy's host there, as a comma-separated list of
`name[:port]` (for example `KANBANR_ALLOWED_HOSTS=board.example.com kanbanr serve`). A request for any
other host gets a 403 that says so. A monitor started without `--allow-writes` stays read-only, so an
authenticated viewer can't mutate your board through it.

A monitor started with `--allow-writes`, or through `kanbanr review --ui`, takes writes only from its
own page, or from an origin you name (below). At each start it makes a new random token and puts it in the page it serves; a write needs
that token, and when the browser sends an `Origin` it must be the monitor's own. It sends no
cross-origin headers, so no other page open in your browser can read the board through it or record a
decision. A script that posted to a write route without the token is refused, with a message that
says why.

**Writes on an address other machines can reach.** A monitor with writes on refuses to start when its
bind is not `127.0.0.1`, `[::1]` or `localhost`, because it gives its token to any page it serves, so
anyone who can open the page can record an approval, a sign-off or a release. The message names the
address and the two ways to accept it: pass `--allow-remote-writes` to `kanbanr serve`, or set
`KANBANR_ALLOW_REMOTE_WRITES=1`. `kanbanr review --ui` has no flag, so it needs the variable if
`KANBANR_BIND` is reachable. A monitor without `--allow-writes` starts on any address, as before.

**Writes through a TLS proxy.** Bind the monitor to loopback, proxy to it, and name where the
browser's writes come from. The monitor accepts a write when its Origin is its own `http://` host or
one named in `KANBANR_ALLOWED_ORIGINS` (comma-separated, scheme and host and optional port, such as
`https://board.example.com`); it never infers an origin from `X-Forwarded-Proto` or any other header.
Naming a host in `KANBANR_ALLOWED_HOSTS` does not name an origin:

```bash
KANBANR_ALLOWED_HOSTS=board.example.com \
KANBANR_ALLOWED_ORIGINS=https://board.example.com \
  kanbanr serve --allow-writes
```

The proxy goes to the loopback bind, so no `--allow-remote-writes` is needed. nginx's default
`proxy_pass` sends `Host: 127.0.0.1:8080`, which the monitor already answers for; Caddy's
`reverse_proxy` passes the original host, which needs `KANBANR_ALLOWED_HOSTS`. The proxy must
authenticate, because the token proves a write came from the monitor's page, not who the person is.
In a container, pass the same settings with `-e KANBANR_ALLOWED_ORIGINS=https://board.example.com`
(or `environment:` in compose).

### Hosting the monitor elsewhere

Each release attaches the monitor as a plain static bundle, `kanbanr-monitor-<tag>.tar.gz`. It is
the same build the binary embeds, so collaborators and reviewers can view a board from any static
web host without installing kanbanr. Hosting it takes three steps:

1. **Unpack** the archive into a folder your host serves. It holds `index.html`, `config.json` and
   `assets/`, all with relative paths, so any folder works, not only the site root.
2. **Edit `config.json`** to point at a monitor's read API:

   ```json
   {
     "apiBase": "https://board.example.com/api",
     "routing": "hash",
     "readOnly": true
   }
   ```

   - `apiBase` is where `kanbanr serve` answers `/api`. The shipped value is
     `http://localhost:8080/api`.
   - `routing: "hash"` gives addresses like `#/p/<project>/feature/<CODE>`, which a plain static
     host serves with no rewrite rule. Use `"browser"` only if your host answers every path with
     `index.html`.
   - `readOnly: true` hides every decision (approve, ratify, sign off, withdraw, accept or reject a
     decision) and shows the CLI command instead. The bundle never calls a write route. Decisions
     happen in Claude Code, or in a local monitor started with `--allow-writes`.
3. **Serve** the folder, for example with `python3 -m http.server` or any static host.

The bundle contains no secrets. The daemon it reads stays the access boundary:

- it is read-only unless started with `--allow-writes`;
- it binds to localhost by default;
- it sends no cross-origin headers, so a bundle served from another origin cannot read it. Anyone
  who can reach the daemon with an accepted host can read the board.

To publish a board, put the daemon behind a reverse proxy with TLS and authentication, as above,
and serve the bundle and the API from the same origin, with `"apiBase": "/api"`. Name the proxy's
host in `KANBANR_ALLOWED_HOSTS`.

The monitor the binary serves reads the same `config.json`, with the defaults: `/api`, browser
routing, and decisions offered when the daemon allows writes.

### Mirroring to GitHub issues

If collaborators follow your GitHub issues, kanbanr can keep issues in step with the board using
the GitHub CLI (`gh`). It works **one way**: kanbanr is the source of truth, and issues follow it.

```bash
gh auth login                                  # once
kanbanr mirror enable --repo acme/shop         # public repos also need --allow-public
kanbanr mirror sync --all                      # optional: issues for existing open features
```

After that, every kanbanr change updates GitHub:

| In kanbanr | On GitHub |
|---|---|
| New feature | New issue (title, spec, todo-lists as checklists, labels) |
| Title / spec / labels / tasks change | Issue updated |
| Moved to Completed | Issue closed as completed |
| Moved to a no-op state (e.g. Out-of-Scope) | Issue closed as not planned |

- Only features whose issue would actually change call GitHub. If GitHub can't be reached, the
  change is still saved in kanbanr and a warning is printed; `kanbanr mirror sync` catches up.
  `KANBANR_MIRROR=off` pauses the automatic sync for a session.
- Features imported from GitHub issues stay linked to them, so nothing is duplicated.
  `kanbanr mirror link FEAT-012 45` links an existing issue by hand; its content is replaced by
  kanbanr's on the next sync.
- Edits and comments made on GitHub aren't pulled back automatically. `kanbanr mirror pull FEAT-012`
  shows whether the issue was edited since the last sync, its current content, and new comments,
  so you (or Claude) can bring what matters into kanbanr first.
- `kanbanr mirror status` shows what a sync would do without calling GitHub;
  `kanbanr mirror disable` turns the mirror off and keeps the links.

The mirror refuses a public repository unless you pass `--allow-public`, because specs, task lists
and notes become visible to anyone.


## The monitor (read-only) — navigation

```mermaid
flowchart TD
  Home["Home — one tile per project (Dashboard + per-status + Docs links)"]
  Board["Board — a column per displayed status"]
  Status["Status page — features in that status, with their OPEN todo-lists"]
  Feature["Feature page — Specification + todo-list tiles (newest first) + export"]
  Ms["Milestones — list"]
  MsOne["Milestone — its dependencies + its feature items as tiles"]
  Sched["Schedule — milestones grouped by feature status (derived)"]
  Docs["Documentation — folders as tiles, files as links"]
  Home --> Board --> Status --> Feature
  Board --> Feature
  Board --> Ms --> MsOne --> Feature
  Board --> Sched --> MsOne
  Home --> Docs
```

- **Home tile**: project name + description, an **Open dashboard** link, a link per **status**
  (with counts), and a **Documentation** link.
- **Board**: one column per state in `displayed_states`; click a card → that feature's page.
  A card names the one stage the item reaches next, with the same count `kanbanr check` gives for
  it, sign-offs included: "Migration needs 6, 1 yours" means six things stand between the item and
  Migration and one of them is yours (a sign-off or approval); the rest are Claude's. "Waiting on
  you: docs-folded" appears only when nothing but your part remains. The card says nothing about
  stages the item could skip to.
- **Status page**: features in one status, grouped by feature, each showing only its **open**
  todo-lists (those not fully completed), newest first, with their tasks.
- **Feature page** (an *epic*): the **Specification** (rendered markdown) and a **tile per
  todo-list** (newest first), each tile holding that list's tasks, plus markdown/JSON export.
  Add a new todo-list per work session — they persist, so multi-session work is never lost.
  On a project that works in releases, a chip beside the status says which release shipped the
  item ("Released in v0.4.0"), which it is planned for and not released yet, or that it is in none.
  Completed is not released: only that chip says an item has shipped.
- **Milestone page**: the milestones it **depends on**, plus its **feature items as tiles**.
- **Schedule**: a *derived* view — for each displayed status, the milestones that contain
  feature items in that status, dependency-ordered. Nothing to create; it always reflects reality.
- **Documentation**: drill from root folders (tiles) into sub-folders and files (any depth);
  files render as markdown.

Everything is **view-only** and refreshes live (a green dot shows the live connection). A
**light/dark theme toggle** sits in the top bar (it remembers your choice and follows your OS
preference by default), and the layout is **responsive** (usable on a phone) with keyboard-focus and
reduced-motion accessibility niceties.

## What the monitor shows

The board — kanbanr's own, as it is developed — and, in dark mode:

![kanbanr's own board](../images/board.png)

![The same board in dark mode](../images/board-dark.png)

An item: why it exists, the goals it serves, its requirements and the tests that proved them, its
todo-lists and its history. When agents work on it (FEAT-214), an **Agents** section after the
todo-lists shows a banner for each scope in force: the agent's role and scope, its branch, who
issued it and when, the time left, and how many of its commits are ahead of the item's tasks; and
below it the item's scope history, expired and revoked scopes included, with the revoke reason. An
activity line made by an agent reads *you, by the agent for FEAT-214 (SC-007)*, on the item page
and on the board.

In every spec, doc and charter, a reference to the board — `FEAT-012`, `FEAT-012/R-2`, `MS-004`,
`ADR-0005`, `G-1`, `architecture/data.md` — is a link to its page, underlined dotted so it reads
differently from a link out of the board. One that points at nothing the board has is struck
through; `kanbanr doctor` lists the same ones. A requirement link lands on that requirement.

![An item's page](../images/feature.png)

The **Review** page also lists a finished item whose approval lapsed or that gained text since
you approved it, labelled with its status, after all the live decisions. The work is done, but the
definition changed after you approved it: approving again covers what it says now.

Where text was added to an approved definition, or changed or removed, the brief shows those parts
first, marked **added** or **changed**, and collapses the approved text that is unchanged. Adding
keeps the approval and waits for the next decision; changing or removing approved text lapses it. A finished item that was never approved isn't listed.

Before you approve, the **Review** page parses every Mermaid diagram the item's brief covers — its
spec and each doc in its `affects_docs` — with Pele, the library that draws them (FEAT-200). Each
one that does not parse is listed under **Diagrams** with its file, diagram number and line, and a
link to the source; **Approve** (or **Ratify**) stays disabled until all of them parse, and while
the check is still running. Fix the spec and the button comes back on the next live refresh. In a
doc, a diagram that does not draw keeps its source on the page, with the error and its line under
it.

| | | |
|---|---|---|
| ![Review: approve, ratify, sign off](../images/review.png) | ![The charter, goals and lessons](../images/charter.png) | ![The workflow and its gates](../images/workflow.png) |
| **Review** — approve, ratify, sign off | **Charter** — purpose, goals, lessons | **Workflow** — stages and their gates |
| ![Milestones](../images/milestones.png) | ![A milestone](../images/milestone.png) | ![The derived schedule](../images/schedule.png) |
| **Milestones** and their dependencies | **A milestone** and its items | **Schedule** — derived, never stored |
| ![The Gantt](../images/gantt.png) | ![A status page](../images/status.png) | ![The Ongoing stream](../images/ongoing.png) |
| **Gantt** with the critical path | **A status**, grouped | **Ongoing** — work that never completes |
| ![Docs](../images/docs.png) | ![A doc with an embedded image](../images/doc-file.png) | ![A doc with a Mermaid diagram](../images/doc-mermaid.png) |
| **Docs** — a folder tree per project | Embedded images | Live **Mermaid** diagrams |

With sprints and releases switched on ([Processes](processes.md#sprints-and-releases)), the board
opens on the running sprint, and Releases and the Gantt show the cadence. Its **Sprint** filter also
offers **All items** and **Unscheduled**, the items not yet in any sprint:

| | | |
|---|---|---|
| ![A running sprint and its burndown](../images/sprint.png) | ![Releases](../images/releases.png) | ![The Gantt with sprints and releases](../images/gantt-sprints.png) |
| **A sprint** — goal, days left, burndown | **Releases** — scope, progress, notes | **Gantt** with sprints and releases |

### The Workflow page

The Workflow page shows, in one place, how work moves from an item branch to a tagged release and
every guard rail that refuses a step on the way (FEAT-213). Its sections, top to bottom:

- **Process** and the **diagram**: the saved process the workflow came from, and the workflow
  drawn with its gates as notes.
- **Stages**: what entering each stage asks for: conditions, sign-offs, warnings, roles.
- **Cadence**, only when sprints or releases are on: the sprint length and the running sprint with
  its dates (linked to the board, which opens on it); how often a release is cut, the release
  stages in order with the last marked *the cut*, and each unshipped release at its stage (linked
  to Releases).
- **Git model**: the topology, main, the line of integration and where item branches start, where
  production moves, item branch names, the protected branches that take merges only, what kanbanr
  merges and when, and that pushing the code is yours. Whether automation is on, and the command
  that switches it. When `kanbanr git doctor` finds problems, a warning box gives their count and
  the first few.
- **Guard rails**: the method's parts (`config method`); the commit rules (only on an item branch,
  a `Refs:` trailer naming the item, never on a protected branch) and whether the hooks are
  installed; diagram linting and strictness (`config diagrams`); each coverage rule and how many
  subjects it leaves uncovered (`config coverage`); the board's push policy (`remote push-policy`),
  and that kanbanr never pushes code; and **Agent scopes** (FEAT-214): what a design and a build
  scope allow, every scope in force across items (`kanbanr scope revoke`), and whether the
  pre-push hook is installed.

Every line shows its state and the CLI command that changes it. The page changes nothing itself.

The git model and git doctor read the code repository. Start `kanbanr serve` in the project folder
and the page shows the branches it found there. A monitor started anywhere else shows the model
from the cadence alone, without git doctor, and says so. Whether the git hooks are installed is the
CLI's to read: the page shows it as not known and names `kanbanr git status`, which tells.

### Release actions

With releases on, release decisions can be made from the monitor started with writes on
(`kanbanr review --ui`). Each uses the same operation as its CLI command, so the board ends up the
same whichever you use. A read-only monitor shows the command instead.

- **Which release?** When an item's next stage is the end (for example Migration → Operations in
  TOGAF), or a stage that asks for `in_release`, and the item is in no release, the Review page
  asks which planned release it belongs to. There is one button per planned release, plus **New
  release…**, which takes a version and a target date, adds the release, and plans the item into
  it. This is `kanbanr release plan <VERSION> <CODE>`, plus `kanbanr release add` for a new one.
  The question comes after any approval or sign-off the item is waiting on. Once the item is in a
  release, it leaves the queue.
- **Spec changed since sign-off.** A sign-off whose gate asks for spec sections, such as
  design-review, covers the whole spec as it was when you gave it. If the spec changes later, the
  item returns to the Review page showing only the changed lines, with the whole spec on request,
  and you sign again. The item's page marks the sign-off "changed since sign-off".
- **Promote.** Each release on the Releases page shows its stage, with the stage history on hover,
  and one button named for its next step — **Promote to \<next stage\>**, or **Cut \<VERSION\>**
  for the last — until it ships. This is `kanbanr release promote <VERSION>`. Beside the button,
  the page says what the step will do ("fast-forwards main to release/v0.3.0", "tags abc1234
  …"); for the cut, it shows the commit it tags, what ships and what carries over, so one click
  cuts. There is no second confirmation: the stage's conditions keep it safe. Until the plan is
  ready the button is disabled and the page lists what is missing with its fix ("push main
  (`git push origin main`), then wait for CI"), with **Check again** to read the plan once more.
  A release with nothing planned into it shows no button, only how to plan into it. The steps act
  on the code repository, and the monitor uses the repository it was started in. Started anywhere
  else, it says "the monitor was not started in the project folder" and names the CLI command. A
  read-only monitor shows the command and the plan, never the button.
- **Release notes.** A release with a branch has a **Release notes** section, collapsed until
  opened, showing the notes as they are on `release/<V>` — headed "draft on release/v0.3.0 @
  8f995e1", or "as shipped on …" after the cut. Until the cut, **Edit notes** opens the text;
  **Save** commits it onto the branch, nothing checked out, and shows the new commit; **Cancel**
  discards it. This is `kanbanr release notes <VERSION> --edit`. The notes are read from the
  repository the monitor was started in; a read-only monitor shows them with the command instead.
  They are read from the path they were written to, so changing the notes path later leaves earlier
  releases' notes in place; a shipped release's are read at its tag. When none are found, the
  message lists every path and ref looked at.

Across several projects ([Portfolio](portfolio.md)):

![The portfolio: rollups and cross-project lanes](../images/portfolio.png)

The sprint, release and portfolio views come from the demo boards `make screenshots` builds
(`tools/demo/cadence.sh`, `tools/demo/portfolio.sh`), because kanbanr's own board is one project
without sprints.
