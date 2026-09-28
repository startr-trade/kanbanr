# Set up in 60 seconds

## Set up (60 seconds, no server)

```bash
# Install (macOS/Linux) — one binary, monitor included, nothing else to build
curl -fsSL https://github.com/startr-trade/kanbanr/releases/latest/download/install.sh | sh

kanbanr init my-app --author "You" --email you@example.com  # data dir + git repo + identity + project
```

From source instead: `make install`, which builds the SPA, installs the binary and links the skill.
Version pinning, checksums, rate limits, published targets and the glibc floor are in
**[INSTALL.md](INSTALL.md)**.

`init` creates the data dir (a git repo), sets your commit identity, scaffolds a project, and
selects it here (a `.kanbanr` marker). If this folder **already** names a board, `init` refuses
rather than repointing it — the marker is the only link between a project and its board, and
overwriting it makes a full board read as empty. It prints both pointers; `--force` repoints
deliberately, and `kanbanr project use <name>` switches project within the same board. That's everything — there is **no server to run, no login,
no accounts**. Each change you make is a git commit authored by your identity. (libgit2 is linked
in — no external `git` needed.)

`init` also registers kanbanr's two **Claude Code hooks** in your global Claude Code settings
(`~/.claude/settings.json`): one shows the board when a Claude session starts, the other reminds
Claude to record its work. It's done once per machine and merged with your existing settings; the
hooks only act in folders kanbanr tracks. Skip it with `kanbanr init --no-hooks`, and manage it
later with `kanbanr hooks install | status | uninstall`.

### Setting up through Claude: a setup interview first

You can also skip the terminal and ask Claude to **"set up kanbanr for this project"**. It
doesn't start running commands. It switches to **plan mode** and interviews you:

1. **Board and identity**: where the board lives (the choices below), the project name, and the
   commit name and email, which default to your `git config`.
2. **Charter**: purpose, goals with measures, non-goals, stakeholders and constraints. Claude
   drafts these from your README and manifests, labels them as a draft, and leaves blank anything
   the repository doesn't answer so it can ask you.
3. **Process**: the workflow (the default kanban, TOGAF phases as the columns, or your own
   statuses), the git commit hooks (offered, defaulting to yes), and optionally a backup remote,
   the GitHub issue mirror and importing an existing tracker.

The plan lists every answer and the exact commands it will run. **Approving the plan (exiting plan
mode) is the go-ahead.** Claude then runs the whole setup: `init`, the workflow, `charter set`,
the Claude Code and git hooks, `claude sync`, and the optional steps. It saves the approved plan
as a board doc (`setup/<date>-setup.md`) and checks the result with `kanbanr doctor`. Only after
that does it get back to whatever you originally asked for. A folder that's already tracked skips
the interview.

### Where the board lives

The board is its own git repo, so it belongs **next to** your project, not inside it. A board
inside the project folder would be a repo nested in your project's repo: it has to be gitignored
and is easy to commit by accident.

`init` asks where to keep it:

```text
Where should kanbanr keep this project's board? (a separate git repo)
  1) /home/you/code/my-app.kanbanr  (new folder next to the project, recommended)
  2) /home/you/code/work.kanbanr    (existing kanbanr folder, shared with its other projects)
Choose a number or type a path [1]:
```

- The recommendation is a sibling of the project's **git repo root** named `<repo>.kanbanr`,
  even if you run `init` from a subfolder.
- Pick an existing kanbanr folder to share one board repo across several projects. Portfolio
  views, cross-project dependencies and the cross-project Gantt work within one data folder.
- Pass `--data-dir <folder>` to skip the question. Without a terminal (e.g. when Claude runs it)
  `init` uses the recommendation; with the skill, Claude asks you first and passes `--data-dir`.
- `init` warns if the folder you pick is inside a git repo.

The choice is recorded in the project's `.kanbanr` marker, relative to the marker:

```yaml
project: my-app
data_dir: ../my-app.kanbanr
```

Every `kanbanr` command run anywhere inside the project finds the marker (it walks up from the
current directory), so no env vars are needed. `kanbanr where` prints the board folder in use.
Commit the marker if everyone who clones the project uses the same layout; otherwise gitignore it.

The data dir resolves from `--data-dir` / `$KANBANR_DATA_DIR` / the marker's `data_dir` / legacy
`./data`. Existing `./data` boards keep working unchanged.

### Importing tasks you already track

If the project already tracks work, in a `TODO.md` or `ROADMAP.md`, in another AI tool's plan files
(Spec Kit, Kiro), or in GitHub issues, Claude offers to import it when you start using kanbanr (or
whenever you ask). It lists what it found and asks which sources to import; by default only open
and in-progress items come in.

- **Preview first.** Claude builds one bundle and shows you the dry run
  (`kanbanr batch --dry-run`), which writes nothing. After you confirm, the import is a single
  commit in the board repo, so it's easy to revert.
- **Nothing is lost if the old tracker goes away.** Each imported item records where it came from
  (`TODO.md:14` at commit `a1b2c3d`, or `owner/repo#123`) and keeps its original text in its spec
  under "Imported from". Whole files are copied into the board's docs under `imports/` before
  they're retired. So the item stays meaningful even if the file is later deleted or the git
  history rewritten.
- **Re-importing is safe.** Items already imported are skipped, matched by issue number or, for
  files, by title (so moving or renumbering a file doesn't import it twice).
- **You decide what happens to the old tracker**: leave it, replace the file with a pointer to
  kanbanr, delete it, or comment on / close GitHub issues with `gh`. Claude never does any of
  that without asking.

`kanbanr sources` (in the project folder) lists imported items and whether their files still
exist; `kanbanr sources --write` records the missing ones, and the monitor shows "source no longer
present" on those items.
