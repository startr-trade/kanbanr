# Command reference

## Complete command reference

Everything the CLI does, grouped by what you are trying to find out. `--json` works on every read and prints one JSON document. The four exceptions have a named format or a document body as their payload: `export`, `graph` and `gantt` take `--format`, and `doc show` prints the file.

**Setting up and looking around**

| Command | What it does |
|---|---|
| `kanbanr init <name>` | data folder + git repo + identity + project, in one step |
| `kanbanr project init/edit/list/use/delete` | create, rename, select (writes the `.kanbanr` marker) |
| `kanbanr where [--json]` | which board folder this directory uses, and why |
| `kanbanr whoami` / `kanbanr identity` | the commit identity this data folder writes as |
| `kanbanr config show / set-transition / displayed-states / default-state / no-op-states / workflow` | the workflow; `config workflow --preset <name>`, `--from-file`, `--export`, `--write-agreement` ([Processes](../using/processes.md)) |
| `kanbanr config rename-status <OLD> <NEW>` | rename a status and move every item in it |
| `kanbanr config cadence [--sprints on\|off] [--releases on\|off] [--unit points\|days] [--release-stages a,b,c] [--integration-branch B] [--main-branch B] [--git-model on\|off] [--has-ci on\|off]` | switch sprints, releases and the estimate unit on or off for a project; name the stages a release moves through (the last is the cut) and the branches promotion uses; let the git model drive branching (on by default once sprints or releases are chosen); say the project has CI, so the recommended `tag` stage waits for it. Turning releases on with no stages named gives `candidate`, `main` and `tag` ([Release stages](../using/processes.md#release-stages)); a stage that does nothing is warned about |
| `kanbanr config method [--on\|--off] [part…]` | the method — design before build, docs kept current, broken references reported — part by part: shown, turned off, or back on |
| `kanbanr config coverage [--from-file F \| --clear]` | the project's architecture coverage rules: with no flag, each rule with how many of its subjects are uncovered and its source doc; `--from-file` sets them from YAML (refused, naming the rule, if invalid), `--clear` removes them |
| `kanbanr config diagrams [--strict on\|off]` | whether a write whose Mermaid diagrams fail the lint is refused (strict) or written with the problems printed as warnings (the default); with no flag, shows the setting |
| `kanbanr config codes [--set KIND=PREFIX,…] [--unset KIND,…] [--default PREFIX\|""] [--clear]` | which prefix a new item's code gets from its kind (`defect=DEF` codes a defect `DEF-001`, with a counter per prefix); with no flag, each kind's prefix, the default (`FEAT` unless set) and the next code per prefix. A prefix is 2–6 uppercase letters, not `MS`, `ADR`, `SC`, `SP`, `TL` or a milestone's; a bad one is refused, naming the entry. Existing codes are never renamed; unset, every item is `FEAT-` |
| `kanbanr hooks install / status / uninstall` | the Claude Code hooks (session start, stop nudge, test capture, commit guard). The Bash guard, the Write guard and test capture also tell a sub-agent (its calls carry an `agent_id`) from the coordinator: an agent scope applies to agents only, and `scope issue` needs these hooks |
| `kanbanr serve [--ui-dir …] [--allow-writes] [--allow-remote-writes]` | the monitor over this board; read-only unless `--allow-writes`, and then it takes writes only from its own page, or from an https origin named in `KANBANR_ALLOWED_ORIGINS` (comma-separated, such as `https://board.example.com`). With writes on it refuses to start on an address other machines can reach unless `--allow-remote-writes` or `KANBANR_ALLOW_REMOTE_WRITES=1` accepts it. It answers for `localhost`, `127.0.0.1`, `[::1]`, its bound address and the hosts in `KANBANR_ALLOWED_HOSTS` (comma-separated `name[:port]`, for a reverse proxy); any other host gets a 403 |
| `kanbanr open` | open the running monitor in a browser |
| `kanbanr open --check [--json]` | report, opening nothing, whether the monitor is up and accepts writes, and its Review and Releases addresses |
| `kanbanr self-update [--check]` | replace this binary with the release's, checksum-verified (and the skill and editor extension already installed) |
| `kanbanr skill install / status / uninstall` | the Claude Code skill this program carries, in `~/.claude/skills/kanbanr` |
| `kanbanr editor install [--editor codium] [--if-installed]` | this release's VS Code extension, checksum-verified, into the editors on PATH |

**The work**

| Command | What it does |
|---|---|
| `kanbanr board` / `kanbanr feature list / show / add / edit` | the kanban and its items |
| `kanbanr move <CODE> <STATUS> [--override "…"]` | a status change, validated against the workflow and the stage's gate; `--override` passes a gate and records why. Merging an item into a release past its first stage sends the release back to that stage and prints the steps from there; a release tagged and not shipped refuses the merge |
| `kanbanr start <CODE>` / `kanbanr finish <CODE>` | begin (branch, active stage) and end an item, each through its gate |
| `kanbanr milestone add / list / edit / delete` | milestones (a dependency DAG; cycles rejected) |
| `kanbanr todo add / list`, `kanbanr task add / state / list` | persistent todo-lists on an item |
| `kanbanr export <CODE> --format md\|json` | one item, rendered for a human or a machine |
| `kanbanr query "text" [--goal G-1] [--gap …] [--all-projects]` | rich filters plus full text, across projects |
| `kanbanr activity` / `kanbanr events` | the changelog, and the notification event log |
| `kanbanr doc folder / add / tree / list / show / rm` | the documentation tree |

**Process: definitions, agreement and sign-off**

| Command | What it does |
|---|---|
| `kanbanr charter show / set` | why the project exists: purpose, goals, non-goals, constraints |
| `kanbanr feature define <CODE> [--file …] [--template]` | an item's definition: statement, goals, Zachman, requirements and their tests |
| `kanbanr signoff <CODE> <name>` | record a named sign-off a stage's gate asks for, pinned to the current definition |
| `kanbanr signoff <CODE> <name> --withdraw --reason "…"` | takes a sign-off back; the earlier one stays on the record and stops counting |
| `kanbanr ratify <CODE>` | agree after the fact to work started under a recorded override |
| `kanbanr test <CODE> <R-n> <test> planned\|red\|green` / `kanbanr tests` | move a test along its lifecycle; list tracked tests and check each still exists |
| `kanbanr defect <CODE> …` | what a defect cost and where it came from |

**Sprints and releases** (projects with cadence switched on)

| Command | What it does |
|---|---|
| `kanbanr sprint add / edit / remove / list / show / plan / unplan / start / close` | timeboxes: add one, change one that has not started (name, goal, dates, capacity), remove an empty unstarted one, list them, see one with its burndown, plan items in, take items out (`unplan`), start, close (carrying unfinished work) |
| `kanbanr release add / remove / list / plan / cut` | releases: add one, remove an empty one that has not started (`--patch-of <V>` makes a patch of a cut release, branched from its tag, or of a tagged one whose CI failed with `--from-failed-tag [--link URL] [--notes TEXT]`, which files a defect, plans the release's items into the patch and supersedes the release; a new release starts with `release plan`, then promoting it to candidate), list them with their stage, plan items in, cut it (ship what is finished, write notes, carry the rest; `--tag` is promotion to production) |
| `kanbanr release promote <VERSION> [<STAGE>] [--reason … \| --note …]` | move a release to its next stage or the one named ([Release stages](../using/processes.md#release-stages)), doing what the stages it enters say: by default the first makes `release/<VERSION>` and the last is the cut — it fast-forwards main and tags the version. Any missing condition refuses it with its fix, changing nothing; a release with nothing planned into it is refused. Back a stage needs `--reason`; nothing moves back from production. Prints each step and what is next. Run it from the project folder; nothing is pushed |
| `kanbanr release promote <VERSION> [<STAGE>] --plan [--fetch]` | what that promotion would do — the button, each step, each condition with its fix, and for the cut what it tags and ships — changing nothing; `--fetch` fetches main first |
| `kanbanr release notes <VERSION> [--file F \| --edit] [--by NAME]` | a release's notes as they are on `release/<VERSION>`, headed with where they are (`release/v0.3.0 @ 8f995e1: docs/src/releases/v0.3.0.md`); `--file` (`-` for stdin) or `--edit` (`$VISUAL`/`$EDITOR` on a copy) commits new text there in one commit, nothing checked out, and re-applies the notes listings; reading ends with "Not mentioned yet: …" naming the planned items the notes leave out; refused once the release is cut ([Reading and editing the notes](../using/processes.md#reading-and-editing-the-notes)). Notes are read from the path they were first written to, recorded on the release, so changing `config release-prepare --notes` keeps earlier releases readable; a shipped release is read at its tag, and a release recorded earlier is found at any path the setting has named. A miss lists every path and ref looked at. Run it from the project folder |
| `kanbanr release redate <VERSION> [--by NAME]` | date a release's notes and changelog heading today, in one commit onto `release/<VERSION>`, nothing checked out; main moves with it if it holds the release, and the command says to push it again; no commit when nothing is stale; refused once the release is cut ([The release date](../using/processes.md#the-release-date)). The stage that moves main does the same on its own. Run it from the project folder |
| `kanbanr release stages` | each release stage, what it requires and what entering it does, and whether that is the project's own gate or the default, and which do nothing ([Stages that act](../using/processes.md#stages-that-act)) |
| `kanbanr gate record <NAME> (--passed \| --failed) [--commit REV] [--url LINK] [--note TEXT] [--by NAME]` | record the verdict of your CI (or any gate) for a commit, as a new result beside the earlier ones; `--commit` defaults to HEAD, and a pass with no commit named is refused over uncommitted changes. kanbanr asks no server; Claude is refused this command ([Gate results](../using/processes.md#gate-results)) |
| `kanbanr gate list [--name N] [--commit REV]` | the recorded gate results, newest first |
| `kanbanr prepare gate <NAME> (--passed \| --failed) [--commit REV] [--url LINK] [--note TEXT] [--source S]…` | prepare a gate result for the person to confirm on the Releases page; `--commit` resolves as for `gate record`, and the run's link counts as a source. Nothing is recorded until they confirm ([Prepared by Claude](../using/the-monitor.md#prepared-by-claude)) |
| `kanbanr prepare signoff <CODE> <NAME> [--note TEXT] [--doc PATH] --source S…` | prepare a sign-off's note and doc for the Review page |
| `kanbanr prepare promotion <VERSION> [--stage STAGE] --checked TEXT --source S…` | prepare what was checked for a stage confirmed after the tag (default: the plan's next stage) |
| `kanbanr prepare ratify <CODE> --reason TEXT --source S…` | prepare a ratification's reason for the Review page |
| `kanbanr prepare list [--all] [--item CODE] [--release VERSION]` | the prepared records waiting for the person, newest first; `--all` adds the confirmed, rejected, superseded and stale ones, each with why |
| `kanbanr config tag-gate [<NAME>]` | the commit gate whose result judges a release's tag ([Stages after the tag](../using/processes.md#stages-after-the-tag)); with none, the first commit gate judges |
| `kanbanr config gate-docs [<ENTRY>…] [--none]` | the paths that count as documentation when a recorded gate pass is carried to a later commit: a folder ending in `/`, a suffix starting `*`, or a path; with none, goes back to `docs/` and `*.md`; `--none` carries across no change |
| `kanbanr config commit-gates [<NAME>…]` | the gates a stage requiring `gate_passed` needs a passed result of for the release tip; with none, clears them |
| `kanbanr config release-stage <STAGE> [--purpose …] [--requires a,b] [--after a,b] [--on-enter a,b] [--clear]` | a release stage's gate: conditions (`items_finished`, `prepared`, `main_at_release`, `pushed`, `gate_passed`, `tag_passed`, `items_merged`), earlier stages the release must have entered first (`--after`) and actions (`branch`, `prepare`, `move_main`, `tag`), starting from the gate in force; `--clear` goes back to the default |
| `kanbanr config release-prepare [--version-file PATH[=MATCH]]… [--clear-version-files] [--changelog PATH] [--changelog-copy PATH]… [--notes TEMPLATE] [--notes-listing PATH::AFTER::ENTRY]…` | what `prepare` commits onto `release/<V>` ([Preparing a release](../using/processes.md#preparing-a-release)); `\n` in MATCH and ENTRY is a newline; a PATH may be given more than once with different matches, each applied in order; with no flag, shows it |
| `kanbanr retro --sprint <SP-…>` | how a sprint went |

**Dependencies and scheduling**

| Command | What it does |
|---|---|
| `kanbanr ready` / `kanbanr blocked` | what can be started now, and what is waiting on something |
| `kanbanr impact <CODE>` | everything downstream of an item — what breaks if it slips |
| `kanbanr graph [--format dot\|json]` | the dependency graph |
| `kanbanr critical-path` / `kanbanr gantt` | the longest chain, and a Mermaid schedule |
| `kanbanr portfolio …` | cross-project rollups for a program of several boards |

**Keeping it honest**

| Command | What it does |
|---|---|
| `kanbanr readme` | write or refresh the board's README: what it is, its projects, and how to install kanbanr and see it as a board; text outside its markers is kept |
| `kanbanr drift [--since DATE] [--item CODE]` | the docs and items that name something work changed (a flag, command, route, `KANBANR_` variable, setting, stage, status or file) and have not changed since, grouped by text, for the items with commits since `DATE` (from the start of that day, UTC; default: the latest shipped release's tag date, else 60 days back; with `--item`, all of that item's history). `--json` adds when it was generated. It flags what to read, never judges it, and exits 0 whatever it finds. Run it from the project folder: without the code repository it cannot check, and says so |
| `kanbanr doctor` | every broken reference and every gap, across the board — including links in specs and docs to items, requirements, ADRs, goals or docs the board does not have, diagram problems: each Mermaid block in a spec or doc the lint rejects, with its file, diagram number, line and rule, and each coverage rule a view still fails, with its first ten uncovered subjects. Warns only about what each item's current stage asks; what later stages will ask is printed after the warnings as counts |
| `kanbanr check [CODE]` | what one item has not said, and what it cannot yet show |
| `kanbanr review [CODE] [--pending] [--ui]` | the decision brief — one item, all of them, or in the browser |
| `kanbanr approve <CODE> [--by …]` | records agreement, attributed to the board's commit identity, and where it was taken (`conversation` inside a Claude Code session, else `terminal`). If the latest verdict on the same text is the same, it records nothing and prints `already approved by … from the monitor; nothing recorded`; `ratify` and `signoff` do the same |
| `kanbanr unapprove <CODE> --reason "…"` | takes an agreement back, with a reason it refuses to go without; the record of having given it stays |
| `kanbanr capture` | reads a test run's output (run by the hook; you never call it) |
| `kanbanr split-from <CODE> <PARENT>` | records that an item was sliced out of another |
| `kanbanr sources [--write]` | imported items whose source file has gone |
| `kanbanr index` | rebuild the per-project cache from the source-of-truth files |
| `kanbanr migrate [--json]` | bring a board written before schema 5 up to date, once: re-pins every approval, sign-off and process record from the board's history, rewrites the files, one commit; every other command refuses such a board until it has run ([upgrading](../releases/schema-5.md)) |

**Reasoning and evidence**

| Command | What it does |
|---|---|
| `kanbanr why <file>:<line>` | the item, requirement and goal behind a line of code |
| `kanbanr trace <G-…\|CODE\|CODE/R-n\|doc.md>` | what hangs off a goal, an item, a requirement or a doc, the specs and docs that link to it, and what is missing |
| `kanbanr adr new / list / supersede / history` | architecture decisions, linked to the items they affect |
| `kanbanr adr accept / reject --reason` | the user's verdict on a proposed decision |
| `kanbanr lessons [--for CODE]` / `kanbanr lesson add / affirm / contradict` | what the project has learned, and judging it |
| `kanbanr retro <MS-…>` / `kanbanr report` | how a milestone went; flow and quality derived from the record |

**Code and Claude Code**

| Command | What it does |
|---|---|
| `kanbanr git install-hooks / uninstall-hooks / status` | the commit hooks that require each commit to name its item |
| `kanbanr git doctor [--json]` | compare the board with the branches: integrated items not merged, unfinished work on a line, branches naming nothing, release fixes not forwarded; read-only, exits 1 on an error |
| `kanbanr git forward <VERSION>` | merge a release's fixes forward into the current line, naming the items they serve |
| `kanbanr git model [--sprints on\|off] [--releases on\|off] [--json]` | the git model the cadence implies: main, the line finished items integrate into, where items branch, item branches, the lines that take merges only, what kanbanr merges, and that pushing is yours. With `--sprints` or `--releases`, the model that cadence would give, over this board's cadence or, with no board yet, over all-off: setup shows it before applying it |
| `kanbanr commit -m "…"` | commit with the board reference filled in from the branch |
| `kanbanr claude sync` | write the project's charter and stages into `CLAUDE.md` |
| `kanbanr git guard` / `kanbanr claude guard` / `kanbanr git check-msg` / `kanbanr git check-branch` / `kanbanr git check-push` | the checks the hooks run (you never call them); `check-push` is the pre-push hook, which refuses a push from an agent's worktree |
| `kanbanr scope issue <design\|build> <CODE> [--for 8h] [--note …] [--worktree PATH] [--json]` | issue an agent scope: record what one sub-agent may do and print the brief to paste into its brief (FEAT-214). `--worktree PATH` binds the scope to that worktree now, so its brief has no claim step (DEF-023) |
| `kanbanr scope claim <SC-id>` | the agent's first command, in its own worktree: bind the scope to it (a build agent switches to its item's branch) |
| `kanbanr scope here` | which scope applies in this folder, and what it allows |
| `kanbanr scope list [--all] [--item CODE] [--json]` / `scope show <SC-id>` / `scope brief <SC-id>` | the scopes in force (or every one), one scope, its brief |
| `kanbanr scope revoke <SC-id> [--reason done]` / `scope renew <SC-id> --for 4h` | end a scope; extend one (72 hours at most) |
| `kanbanr scope drift [--brief] [--json]` | build agents' commits newer than their item's last task change |

**Sharing**

| Command | What it does |
|---|---|
| `kanbanr remote add / list / remove`, `kanbanr sync` | git remotes for the board (SSH or HTTPS), and an immediate push |
| `kanbanr remote push-policy [auto\|off\|debounce:N]` | when this board is pushed: show it, or set it in the board's own git config |
| `kanbanr mirror enable / disable / status / sync / link / pull` | one-way mirror of items to GitHub issues |
| `kanbanr batch [--dry-run] [--file …]` | many changes in one call and one commit |
