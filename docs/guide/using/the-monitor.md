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

The image's health check asks for `/healthz` as `localhost`, so a probe of your own should send
`Host: localhost` too.

To host the monitor itself for a team, read-only and kept current from each board's git remote,
use the Kubernetes module or the compose file in `deploy/`; see
[Hosting a read-only monitor](hosting.md).

The monitor the binary serves reads the same `config.json`, with the defaults: `/api`, browser
routing, and decisions offered when the daemon allows writes.

### Several boards on one server

One read-only monitor can show the projects of several boards together:

```
kanbanr serve --board /srv/boards/kanbanr --board /srv/boards/katalogr
```

Use it for a public site that shows boards with different writers. Each board stays its own git
repository, with its own remote and its own people who may push to it. A data folder is the
sharing boundary (`ADR-0007`), and this keeps it: the server reads each board, writes none, and
resolves nothing from one board to another.

- **Read-only.** `--board` refuses `--allow-writes` and `--allow-remote-writes`, so no decision
  can be taken on such a site. Decisions stay with each board's own writers, in Claude Code or a
  local monitor.
- **Each project from its own board.** A project's items, docs, history, approvals, drift and
  retrospective are read from the board that holds it. A change pulled into one board refreshes the
  pages open on its projects.
- **Names.** A board is named after the last folder of its path as you wrote it (`kanbanr` and
  `katalogr` above). Two boards with one name, a path that is not a board (it has no `projects/`
  folder) and one project on two boards are refused, and the server does not start.
- **The first board wins.** When a project appears on a second board after the server started,
  the first board keeps serving it, the server prints a note once, and `/api/boards` lists the
  project under the second board's `shadowed`.
- **Nothing resolves across boards.** A dependency on another board's project, such as
  `katalogr:FTR-001` on the kanbanr board, is shown as external and is never resolved: the item
  that names it stays blocked, whatever the other board says. The portfolio and the other
  workspace-wide reads show one board at a time, the first unless the address names another with
  `?board=<name>` (`/portfolio?board=katalogr`). A view that resolves across boards is `FEAT-074`,
  not built.

With several boards the home page has one section per board: the board's name as a heading, a
line saying when it last changed (or why its history can't be read), then its projects as tiles and
a Portfolio link for that board. A dependency on another board's project is shown as its chip
followed by the board's name and "not resolved here".

`/api/boards` lists the boards, each with its projects and its last commit
([HTTP API](../reference/http-api.md#several-boards)). On two copies of kanbanr's own board (one
project of about 400 items each), its first answer after a start took 4 to 7 ms, and 49 ms the
first time the folders were read from disk (a debug build on a busy 12-core machine, November
2026). Later answers come from the daemon's cache until a board changes.

A data folder served without `--board` is a list of one board, and the monitor shows it as it
always did.

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

A project's pages are grouped into areas: **Overview, Work, Plan, Evidence, Architecture and
Records** (FTR-047). On a wide screen the top bar is one line: an area with several pages is a
menu button ("Work ▾", "Architecture ▾") that opens its list under it, and an area with one page
is a plain link named for the area. Only one menu is open at a time; Escape, a click outside or
following a link closes it. The area you are in is underlined, and the page you are on is marked
in its menu. An area shows only once it has a page: Evidence has none yet, and
**Releases** shows only on a project that works in releases.

```mermaid
flowchart LR
  Bar["Top bar — one item per area"]
  Work["Work ▾"]
  Plan["Plan ▾"]
  Arch["Architecture ▾"]
  Rec["Records ▾"]
  Board["Board — a column per displayed status"]
  Review["Review — what waits on you"]
  Ms["Milestones — list, newest first"]
  Sched["Schedule — milestones by feature status"]
  Gantt["Gantt"]
  Rel["Releases — on a project in releases"]
  Wf["Workflow"]
  Base["Baseline — the engineering baseline"]
  Charter["Charter"]
  Docs["Docs — folders as tiles, files as links"]
  Cat["Catalog — what the project must document"]
  Status["Status page"]
  Feature["Feature page"]
  MsOne["Milestone page"]
  Over["Overview — what needs you, what is happening now"]
  Bar --> Over
  Bar --> Work
  Work --> Items["Items — filtered list"]
  Bar --> Plan
  Bar --> Arch
  Bar --> Rec
  Work --> Board
  Work --> Review
  Work --> Ms
  Plan --> Sched
  Plan --> Gantt
  Plan --> Rel
  Arch --> Wf
  Arch --> Base
  Rec --> Charter
  Rec --> Docs
  Rec --> Cat
  Board --> Status --> Feature
  Board --> Feature
  Ms --> MsOne --> Feature
```

An item, status or milestone page belongs to the area it is reached from: on an item's page, Work
is underlined and Board is marked; on a document, Records and Docs.

**On a phone** (720 px wide or less) the areas fold behind one **≡ Menu** button, with the page you
are on named beside it ("kanbanr · Board"). The button opens a full-width panel that lists every
area as a heading with all its pages under it, and **All projects** (Projects, Portfolio) with
them. Nothing in the top bar is cut off, and every page can be reached. The theme toggle stays
outside the menu, always visible.

- **Home tile**: project name + description, an **Open dashboard** link, a link per **status**
  (with counts), and a **Documentation** link.
- **Overview**: the project's landing page, at `/p/<project>`; see
  [The Overview and Needs you](#the-overview-and-needs-you).
- **Board**: at `/p/<project>/board`: one column per state in `displayed_states`; click a card → that feature's page.
  A card names the one stage the item reaches next, with the same count `kanbanr check` gives for
  it, sign-offs included: "Migration needs 6, 1 yours" means six things stand between the item and
  Migration and one of them is yours (a sign-off or approval); the rest are Claude's. "Waiting on
  you: docs-folded" appears only when nothing but your part remains. The card says nothing about
  stages the item could skip to.
- **Review: texts to check.** Under an item's docs-folded sign-off, the page lists the docs and
  items that name something the item changed and have not changed since, and says Claude settles
  them. You fill in nothing: the sign-off appears once none is left, and "Checked by Claude" lists
  what Claude confirmed or corrected, with why.
- **Review: a lapsed docs-folded.** When the item's branch gets a commit after its docs-folded
  sign-off, the page asks for the sign-off again and says why: "The earlier docs-folded sign-off
  lapsed: feat/… has commits after sign-off 'docs-folded' was given."
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

Everything is **view-only** and refreshes live: a green dot with the word **Live** shows the live
connection, and while it is down the dot turns grey and says **Reconnecting…**. A **light/dark
theme toggle** sits in the top bar (it remembers your choice and follows your OS preference by
default), and the layout is **responsive** (on a phone the navigation is one menu, as above).

The monitor meets **WCAG 2.1 AA**: every colour pair reaches 4.5:1 for text and 3:1 for borders
and the focus ring, in both themes; no text is smaller than 13 px; the keyboard focus ring always
shows; each page has one h1, with cards, Board columns and rows as headings; and icons are hidden
from screen readers while every input is labelled. Checks in CI hold this bar
(`check:a11y-contrast`, `check:a11y-type`, `check:a11y-focus` and `check:a11y-structure`), and
motion is reduced when your system asks for it.

### The Overview and Needs you

A project opens on its **Overview** at `/p/<project>` (FTR-048). The Board moved to
`/p/<project>/board`; the Work menu still names it. When the monitor serves exactly one project,
`/` goes straight to that project's Overview; with two or more, Home lists them as before. The
Overview leads with what needs you, then what is happening, then how much the board holds:

- **Needs you**: what waits on you, in four sections: Decisions (each Review group), Releases (a
  release at a stage that waits for you), Health (red tests, doctor errors, a repository that has
  drifted) and Records (a retrospective that is due). Each line has a count and links to the page
  that takes it. Only your own decisions count toward the total; health is Claude's to fix, so
  its line says "Ask Claude to fix the 22 doctor errors." instead of linking.
- **Now**: the running sprint with its days left, each release in flight with the steps done,
  current and next, and the open milestones.
- **What the board holds**: items by kind (untyped counted apart), requirements, tests by state
  and kind, requirement coverage, flow (completed, cycle time, rework, defects) and records. Every
  count links to the list it counts; the flow numbers that have no list are tiles that say
  "Ask Claude for the flow report."
- **Goals**, **Recent decisions** (ADRs, approvals, sign-offs, release stages, shipped releases,
  newest first, in words) and **Activity**.

Every page also carries a **Needs you** badge in the top bar with the same total; it opens a
drawer with the same lines. **Work → Items** (`/p/<project>/items`) lists items filtered by kind,
status, milestone, goal, sprint, release or evidence (red, planned, stale, unproven, proven,
untested); the filters are in the address, so a link from the Overview opens the list it counted.
The data comes from `GET /api/projects/{project}/overview`, `…/attention` and `…/items`; see the
[HTTP API](../reference/http-api.md).

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

An override Claude made past a gate waits on the **Review** page to be ratified, even on an item
you approved: the entry shows each one as "Moved Test → Review on 2026-11-12 by Claude, past its
gate: …" above **Ratify**. Until you ratify it, the item does not ship. Each override waits on
its own: one Claude made after you ratified an earlier one, at the same definition, is listed
again, and ratifying records a new ratification. When Approve or Ratify is disabled, the sentence
under it says what is missing and who provides it (the diagram check running, a diagram to fix, or
a page to reload).

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
| **Milestones**, newest first, and their dependencies | **A milestone** and its items | **Schedule** — derived, never stored |
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

### The Plan page's Flow panel

The **Plan** page's Current tab opens on the **Flow** panel (FTR-061), above the timeline. It plans
agent-built work by its task graph, in agent-hours, for the running sprint (else every open item),
the same plan `kanbanr plan` prints:

- one line of totals: the remaining work, the critical path's length and how many tasks it runs
  over, and the most tasks that can run at once ("Remaining 46.5 agent-h · critical path 12.0 h
  over 7 tasks · up to 6 at once"). Its critical path runs over tasks; the timeline below marks the
  items' own;
- **Running**, each task with its scope and its time against its estimate ("1.5 h of 2.0 h"),
  marked **over** past it; **Ready**, the critical-path tasks marked, each with its remaining chain;
  **Blocked**, each task with every reason it waits, in words (a task it waits on, files another
  running task writes, its item not at the build stage);
- the forecast: one row per lane count, "4 lanes · 2 build slots → 14.0 h" in hours from now, with
  "(slots allow 2)" when the build slots bound the lanes that help, and the critical path as the
  least it can take however many lanes run;
- notes: the tasks with no estimate (counted as 1 h), the items with no tasks, and the build share
  the forecast assumed or measured.

A forecast is never a date. The only dates on the panel are the ones a person set, a running
sprint's last day and a release's target, shown as chips labelled "(set by the person)". With no
tasks to plan, the panel says so and offers the sentence to ask Claude to break the sprint's items
into tasks.

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

### The Baseline page

The Baseline page, under **Architecture**, shows the project's engineering baseline (FTR-033): the
practice packs it follows, each `current`, `changed` in its library or `removed`; each proposed
revision as a panel with its answers, the entries it adds and replaces, the release-stage gates it
adds and what depends on them; the areas grouped accepted, proposed, open (deferred ones marked
"decide later") and not applicable with the reason, each entry with its kind, its ADR and the items
that name it; the charter's own constraints under **Also in the charter**; and the history of
accepted revisions. **Accept** on a proposed revision is the page's one decision, and a revision
can be withdrawn with a reason. On a read-only monitor the panel names the command instead
(`kanbanr baseline accept REV-n`). See [The engineering baseline](baseline.md).

### The Catalog page

The Catalog page, under **Records** after Docs, shows the project's content catalog (FTR-005): what
the project must document, by question and perspective. The **gaps** come first, in one sentence
("Nobody answers: when for the operator; who for the builder"). Then the rows, grouped **System**,
**Components** and **Items**, each on a line with chips: whether it applies (a row switched off
says `off:` and its reason, greyed), its source's state (present, stub, missing, available, needs
katalogr), its notations, and each protection, in force or not configured, with the reason as its
title. A written source links to its doc (a folder to the Docs page at that folder); a generated
source links to the page that shows it, such as the Charter, Gantt or Releases. A repository doc
shows its path. With no facts the page says so and names `kanbanr facts set`; with no catalog it
names `kanbanr catalog init`. The page is read-only: Claude keeps the catalog. See
[Living architecture docs](living-docs.md#what-a-project-documents-the-content-catalog).

### Docs at a position

The Docs page, and the repository doc pages that open from it, read a doc **at the position you
pick** (FTR-004). A position is where a doc is read from: `main`, the working tree, an item, a
sprint, a release or a commit. The picker is a select in the header of the Docs page, the repository
docs page and each repository doc page (and nowhere else: an item page has none, because its
repository doc links already open at the item). It lists `main`, the working tree, each sprint and
each release, with the reason beside one that cannot be read (a sprint with no branch yet, a release
with no tag); a text field beside it takes an item code, a commit or a position text. Your choice is
kept for the session and carried on every repository doc link, so moving between docs keeps the
position.

Repository docs are the files under `docs/` in the code repository. The page at `/p/<project>/repo`
lists them at the position, and `/p/<project>/repo/<path>` shows one. A link in a spec or a doc such
as `[docs/guide/using/x.md](docs/guide/using/x.md)` opens there, at the item's position on an item
page and at the current pick elsewhere. A link under `/p/<project>/docs/file/` is a board doc and
never changes.

Every doc page says what it read, in one line above the text:

- `docs/guide/using/x.md · at FTR-004 · feat/FTR-004-read-docs @ 1a2b3c4 · from git (merge commit recorded on the item)`:
  the item's merge commit when it has been integrated, else its branch tip. The text is the commit's
  own, whatever is checked out.
- `docs/guide/using/x.md · working tree @ 6a973c6+1f3e… · from disk, with uncommitted changes`: the
  files on disk, the only position that shows an edit you have not committed.
- `board doc · one record, the same at every position`: a board doc does not depend on the
  position.

A doc that is not there says so and whether `main` has it: "docs/system/store.md is not in the
repository at v0.6.0 (tag v0.6.0 @ 9fe1d20). main has it." The picker stays on the page to choose
another position. Without a code repository (a monitor started outside the project folder), these
pages say that no checkout holds the position, as `kanbanr view docs --at` does. Reading never
changes the repository: nothing is checked out and no branch moves.

On an item page, each sign-off that recorded a position adds "docs read at 1a2b3c4" and a link to
each doc it read, opened at that commit, so the reviewer sees exactly what was signed.

### How the monitor stays fast

A page reads only what it shows (DEF-081). The header, the links in specs and docs, and the item
lists read one compact index of the board: every item as a card, with no specifications or
histories. An item page reads that item alone, and a status page only the items in that status.
Every page shares one copy of each read, however many parts of it ask. The daemon answers each
read once per change to the board and gzips it. The browser then asks again only to be told
nothing changed. When the CLI changes the board, the daemon says which parts changed, and the
page re-reads just those parts. The Review page reads its group counts first, then the open
group's rows, and a row's evidence only when the row is opened. The Releases page reads its tab
counts first, then the open tab's releases, and a release's items only when its row is opened (see
[The Releases page](#the-releases-page)). The Schedule page still reads the whole board until its
own rebuild lands.

### The Releases page

On a project that works in releases, the Releases page has three tabs (FTR-041), each labelled with
its count:

- **Current**: releases at a stage that are not shipped yet — a candidate, a release on main, one
  tagged and waiting for its tag workflow, a patch in progress.
- **Planned**: releases not yet at a stage.
- **Released**: shipped releases, and superseded ones marked "Superseded by vX".

The page opens on Current when a release is in flight, else on Planned, else on Released; an empty
tab is hidden unless all are. Each tab lists its releases newest first by version (see **Promote** below).

Each release is one row that opens like an accordion. A tab opens its first release and keeps the
rest closed. The row's header has two lines:

- **First line:** the version and name, then the chips — state, stage ("not yet a candidate"
  before the first), target, shipped date, how much is finished ("3/4 finished (75%)"), how many
  items were carried over, "Superseded by vX" and "tagged, not released yet" — and the stage
  stepper: one dot per stage joined by a line, with the date and who under each stage reached. A
  stage that was undone shows as not reached.
- **Second line:** for a release that is not shipped, its next step: the button and what it will do
  (see [Release actions](#release-actions)). A closed Planned release shows its next step as text
  ("next: Promote v0.9.0 to candidate") and asks nothing of the code repository until you open it.

The opened row's body starts with a full-width table of the release's items: ID (linked to the
item), title, kind, status, sprint, tests green out of named, its approval, design-review and
docs-folded sign-offs as pills, and where a carried item went. Titles get the width and do not
wrap. Every column sorts, the table has its own filter, and **Export CSV** saves it. Below the
table come **Remove** for a planned release that can still be removed, the rest of the release's
button (what is missing with its fix, the cut preview, **Check again**, and the tag's panel once it
is tagged), **Prepared, not required**, the **Release notes** section, collapsed until opened, and
the link to the release's notes doc. A closed row reads none of this.

The **Filter** box above the list filters the open releases' tables, and hides closed releases
whose version or name does not match. It never reads a closed release's items to search them.

On a narrow screen (below 720 px) the table keeps ID, title and status, and each row's **More**
shows the other columns; below 640 px the header stacks into one column.

The URL keeps the page as you left it, so a link can point at one release: `?tab=planned` is the
tab, the hash lists the open releases (`#v0.8.0,v0.7.1`), `?q=` is the filter, and `?t.sort=` and
`?t.dir=` the table's sort. `/p/<project>/releases?tab=current#v0.8.0` opens v0.8.0's row.

### Which kanbanr the monitor runs

The header shows the kanbanr version and commit the monitor runs, for example `v0.6.0 · 1138833`
(FTR-036). Every write to the board records the newest kanbanr that wrote it in `writer.yaml` at
the board's root. When that writer is newer than the monitor, a warning under the header names
both versions: decisions taken on that monitor follow older rules than the work was built for.
Restart the monitor from the newer kanbanr before taking sign-offs. For two builds of the same
version, the monitor compares commits in the project's code repository; outside it, only a higher
version warns. A board no stamping kanbanr has written shows no warning.

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
- **Promote.** The Releases page lists releases newest first by version (v0.10.0 above v0.9.0, a
  patch such as v0.5.1 above v0.5.0, a pre-release just below its release; versions that are not
  numbers come last, in the order they were added), as `kanbanr release list` does. Each release's
  header shows its stage on the stepper and, on its second line, one button that names the release and its
  next step — **Promote \<VERSION\> to \<next stage\>**, or **Cut \<VERSION\>** for the last —
  until it ships. This is `kanbanr release promote <VERSION>`. Beside the button, the page says
  what the step will do ("fast-forwards main to release/v0.3.0", "tags abc1234 …"); for the cut,
  it shows the commit it tags, what ships and what carries over. A candidate whose release-prepare
  configuration changed since it was prepared also offers **Promote \<VERSION\> to candidate
  again**, named for what it redoes (`kanbanr release promote <VERSION> candidate`); it is shown
  only while entering candidate again would change something. A press asks before it acts: the
  button turns into the question ("Promote v0.6.0 to candidate?"), the same button and
  **Cancel**. The second press promotes the release the button names; Cancel leaves it where it
  was. A prepared confirmation and a confirmation after the tag keep their one press. After a
  promotion the page names the way back: a stage taken by mistake is undone with
  `kanbanr release undo <VERSION> --reason …` while nothing is pushed or tagged. Until the plan is
  ready the button is disabled and says why beside it, in the same words as its tooltip: the first
  thing missing with its fix ("release/v0.2.0 already exists in … and is not main's tip: 2
  commit(s) ahead, 0 behind. Delete it if it is a leftover …"). In the release's body, below its
  item table, the page lists everything that is missing with its fix ("push main (`git push origin main`), then wait for CI"), with
  **Check again** to read the plan once more. The **again** button says why it is disabled the same
  way.
  A release with nothing planned into it shows no button, only how to plan into it. The steps act
  on the code repository, and the monitor uses the repository it was started in. Started anywhere
  else, it says "the monitor was not started in the project folder" and names the CLI command. A
  read-only monitor shows the command and the plan, never the button.
- **Shipping untested work.** When the cut's one missing condition is that some shipping
  requirements have tests not run, the release's body lists them (`FEAT-005/R-2`, …) with one
  checkbox, "Ship these unverified; the notes will name them". Ticking it enables **Cut**, which
  sends exactly that list; if the list changed since the page read it, the cut is refused and you
  look again. The notes on the board gain a "Not verified in this release" section. This is
  `kanbanr release promote <VERSION> --accept-untested …`.
- **Tagged, waiting for the tag workflow.** When the tag stage is the last and a gate judges the
  tag, the cut leaves the release "tagged, not released yet", still in Current. Its body keeps the
  tag's panel, and its header a button, "Waiting for the tag workflow", that stays off until a passed result is recorded for the
  tag's commit. Confirming Claude's prepared result (or recording it with **Record result**) ships
  the release. A failed result keeps it tagged and the panel offers the patch release. This is
  `kanbanr gate record <gate> --passed --commit <tag>`, or `kanbanr release promote <VERSION>`.
- **Prepared, not required.** A record Claude prepared for a release that no form on its row
  shows, such as a gate result for a gate no release stage asks for, or a confirmation for a stage
  other than the next one, is listed in the release's body under **Prepared, not required**,
  below its item table and the rest of its button, and above its notes. A gate result no stage asks for reads "no stage asks for
  \<gate\>". Each has its own button (**Record \<gate\> passed on \<commit\>**), **Edit** and
  **Reject**, as in [Prepared by Claude](#prepared-by-claude), so nothing waits unseen. A prepared
  gate result goes stale once its release is cut after it was prepared, or its release branch
  moves past its commit; it then shows why and has no button. `kanbanr prepare list --release
  <VERSION>` lists the same records. An answered one names its release only by the tag commit, so
  `--all --release <VERSION>` can miss an answered result whose commit is only on the release
  branch.
- **Release notes.** A release with a branch has a **Release notes** section, collapsed until
  opened, showing the notes as they are on `release/<V>` — headed "draft on release/v0.3.0 @
  8f995e1", or "as shipped on …" after the cut. Until the cut, **Edit notes** opens the text;
  **Save** commits it onto the branch, nothing checked out, and shows the new commit; **Cancel**
  discards it. This is `kanbanr release notes <VERSION> --edit`. The notes are read from the
  repository the monitor was started in; a read-only monitor shows them with the command instead.
  Under the notes the panel shows the version's section of the changelog ("Changelog section
  (CHANGELOG.md)"), read-only; when there is none it says so. Ask Claude to change it
  (`kanbanr release notes <VERSION> --changelog FILE`).
  They are read from the path they were written to, so changing the notes path later leaves earlier
  releases' notes in place; a shipped release's are read at its tag. When none are found, the
  message lists every path and ref looked at.

### The Review page

The Review page groups what waits for you by the decision it needs (FTR-038), one tab per group,
each with its count: **Approve**, **Ratify**, a **Sign off** group per sign-off the process names
(**Sign off design-review**, **Sign off docs-folded**, a release's sign-off), **Which release**,
**Manual checks**, **Delegations**, **Delegated decisions** and **Proposed ADRs**. An empty group
has no tab. The page opens on the largest group, or on the one its link names.

- **Rows.** Each row is one item on one line: its code, its title, what is being decided, and pills
  for what needs your eye ("approval lapsed", "spec changed since sign-off", "added since
  approval", "prepared", the batch it is in), with its release and sprint. Under it, one line says
  what your click records. Open a row to see its evidence in place: the definition's changes since
  you last approved, the spec diff, the diagrams, the texts Claude settled, a record Claude
  prepared with its sources. Only an opened row's evidence is read.
- **Selecting.** A checkbox per row and **Select all** in the group's header pick rows in that
  group only; opening another group starts a new selection, so one confirmation is always one
  decision. A row that cannot be decided yet says why, and who provides what is missing ("Claude
  settles 2 texts first: ask Claude to check them"), and cannot be ticked. In the Approve and
  Ratify groups a row can be ticked once its diagrams have been checked and parse. Rows Claude
  prepared arrive ticked; untick any you do not want.
- **Filter and sort.** The header's filter box searches the group by code or title, and its sort
  orders the rows by release, last change, sprint or code. The header stays in view as the rows
  scroll.
- **One button, asked first.** The group's button names the action and the count, such as
  **Approve 3** or **Sign off docs-folded 5**. A press turns the header into the question naming
  the items ("Approve FTR-041, FTR-043 and DEF-071?"), the same button and **Cancel**. The second
  press records the decision for each of them: one verdict per item, each pinned to the definition
  you saw and recorded as you from the monitor, all in one commit. An item that changed since the
  page read it is refused alone, and a line under the header says so ("approved 2 of 3; FTR-043:
  …"); the others go through. When the selection is exactly one batch Claude prepared, the button
  reads **Confirm all N** (and **Reject all N** for its records) and confirms the batch the same
  way. The Delegated decisions group also has **Undo N**, and selected prepared rows have **Reject
  N prepared**. A decision that needs a reason or your own observation stays in its row: an ADR's
  Reject, Undo with a reason, and Passed or Failed on a manual check with nothing prepared.
- **Links.** The group and the open rows are in the address, such as
  `/p/<project>/review?group=docs-folded#DEF-066`, so Claude can link you to exactly the decision
  it is asking for. Your selection is never in a link.
- **Read-only.** A read-only monitor shows the same groups and rows with no checkboxes or
  buttons; each row's evidence gives the command that takes the decision in your own terminal.

`kanbanr review` in the terminal is unchanged.

### Manual checks

A manual test that is not yet green waits on the **Review** page, in the **Manual checks** group: the
item, the requirement and its text, the check to make, and its state ("not run", or "failed" with
who failed it and when). Do the check, then click **Passed** or **Failed**. **Add a note** opens an
optional note; nothing has to be typed. The click records the result as you, the same record
`kanbanr test` makes in your own terminal. Claude is refused a manual result, since it says a
person checked; if you tell Claude the result, it prepares the record filled in and you confirm it
here. A failed check is recorded as failed and no defect is filed for you: ask Claude to file one.
A read-only monitor shows the `kanbanr test` command instead. A check opens once its item reaches
its build stage: before then nothing exists to check, so it is listed apart under **Not due yet**
with no Passed or Failed, and kanbanr refuses a result for it from any page or terminal.

The checks are grouped under their item (FTR-057): one heading per item, reading its code, title,
status, release and how many checks it has, with its checks beneath it. The release being shipped
comes first, under **Holding v0.8.0**: the release at a stage, newest first when more than one is in
flight, or else the next planned release with items. A check that holds that release's next
promotion carries a **holds Promote v0.8.0 to main** pill, and so does its item's heading; those
items come first in their section. Every other item follows under **Other items**, collapsed while
a release is being shipped. When an item has two or more prepared results, its heading offers
**Confirm 2 prepared for FTR-041**: the first press selects exactly those checks and asks, naming
them; the second records them together, as one decision.

### Prepared by Claude

When Claude already has the evidence for a decision, it prepares the record instead of asking you
to fill in a form (FTR-034): a sign-off's note and the doc it covered, the reason for a
ratification, what was checked before a stage after the tag, or a CI result with its run's link.
On the Review page the record's row sits in the group of the decision it prepares, ticked, with
the record in its evidence; on the Releases page it sits beside the decision's one button. It is
shown filled in, headed "Prepared
by Claude on 12 Nov 2026 from CI run 4512, release tip 9f078bd", with each source as a link: a
board doc or item opens here, a web link opens in a new tab, and a commit is shown as its hash.

- **The button** confirms it as it stands. The verdict is made as you, through the same route the
  form or the command uses, with the same checks: a definition, spec or release branch that moved
  since the page was shown is refused, and nothing is recorded.
- **Edit** turns the fields you may change into inputs, filled in with what Claude prepared; the
  same button then confirms your version.
- **Reject** takes an optional reason and records that you turned it down. Nothing else changes;
  ask Claude to prepare it again.

Both parts are kept as files under the project's `prepared/` folder: what Claude prepared, with
its sources, and your answer, with each field you changed. Neither is rewritten. A record whose
decision was taken some other way, or whose item or release moved on, shows why and has no button;
a newer one for the same decision replaces it. A read-only monitor shows the record and the
command that takes the same decision in your own terminal. Claude can prepare a record but can
never confirm or reject one.

### Planning from the pages

You can plan items from the pages instead of asking Claude in the conversation (FTR-049). On the
Plan page, open a row: each item in its table has a checkbox, and one **Plan into…** button acts on
the selection, with the sprints and releases that are not frozen as targets. You can also drag an
item's table row onto a sprint or release row of the Plan page. On the Releases page, an item
table's selection has **Move to…**, with the other planned releases as targets.

Every route asks first. The first press (or the drop) shows the question, for example "Plan
FTR-044 and DEF-076 into SP-004?", with a button of the same words and Cancel; the second press
records it. A drop never records anything by itself.

What is recorded is your decision, as you, from the monitor: one commit, one file under
`planning/`. It does not change any plan. The row shows it as **pending: Claude carries it out at
its next turn**, and the Overview lists it under Decisions without counting it in the badge, since
it waits for Claude. Claude then checks dependencies, frozen scope, capacity and dates, and either
plans the items (the outcome shows as carried) or declines with its reason and the nearest valid
move, shown on the item's row with a one-press "Ask for v0.9.0 instead". Claude can never record a
planning decision, and you can carry out your own in a terminal with `--decision`. A read-only
monitor shows pending decisions but not the buttons.

### Decisions here

**Decisions here**, a collapsed section above the Review page's groups, says where this project's
decisions are taken (FTR-028). Delegations Claude proposed and decisions Claude took under one
are rows in the **Delegations** and **Delegated decisions** groups, each row opening to the panel
below:

- **The mode**, `monitor` or `delegated`, with **Change**. After a change the panel lists what is
  waiting and where it is taken now: here, or by Claude under a delegation that covers it.
- **Delegations.** A proposal from Claude is shown like a prepared record, with what it covers and
  until when: **Grant** grants it exactly as shown (if it changed since the page loaded, the grant
  is refused), **Reject** discards it. A granted one shows its end and **Revoke**, which needs a
  reason. A delegation whose grant kanbanr did not write (the file was edited by hand, or changed
  outside a commit) is marked "not granted through kanbanr", and nothing is taken under it. One
  whose grant kanbanr cannot check, because it cannot read the board's git history (most often a
  folder owned by another user), is marked "can't be checked" with the reason and the fix, and
  nothing is taken under it until it can be. In delegated mode a granted one also says
  whether the Claude Code allow rule auto mode needs is in place ("auto mode: allowed"), or names
  the missing rule and the command that adds it, `kanbanr claude permissions --write`.
- **Delegated decisions to review**: what Claude decided under a delegation, on which item, under
  which delegation and when, with **Keep** and **Undo** and an optional reason. Undo withdraws the
  approval or sign-off.

A read-only monitor shows the same panel with the command that does each thing in your own
terminal instead of the buttons.

Across several projects ([Portfolio](portfolio.md)):

![The portfolio: rollups and cross-project lanes](../images/portfolio.png)

The sprint, release and portfolio views come from the demo boards `make screenshots` builds
(`tools/demo/cadence.sh`, `tools/demo/portfolio.sh`), because kanbanr's own board is one project
without sprints.
