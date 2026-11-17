# Walkthroughs that run themselves

A walkthrough follows one of the [process guides](../guides/index.md) from an empty folder to a cut
release, so that what the guide says is seen working on the installed kanbanr before a release is
accepted. Two such walkthroughs cost the maintainer three to four hours each; `kanbanr walkthrough
run` does the same without a person present, and leaves the person a short check of what only a
person can judge.

## What a run does

`kanbanr walkthrough run walkthroughs/togaf.md` reads a **runbook** (a board doc) and:

1. **Refuses to start** unless everything it will touch is inside a fence (`KANBANR_BOARD_FENCE`,
   or the runs folder), its remotes are its own, your board is outside the run, and the `kanbanr`
   the session will find on its `PATH` is the run's own binary (the same version line). It writes
   `setup.log` naming what is under test: the kanbanr binary and the one the session runs, the
   skill's checksum and where it came from (and a note when your installed skill differs), the
   Claude Code version, the model and the checksums of your rules file and its imports.
2. **Makes a scratch project** with its own two remotes (two bare repositories under the run folder,
   or two private repositories on the remote server with `--remotes <prefix>`, made with `gh`), a
   copy of your Claude Code configuration (rules, settings, credentials, and the skill this kanbanr
   binary carries, installed as `kanbanr skill install` writes it), a `bin/` folder holding a link
   to the run's binary, first on the session's `PATH` so the session and every hook use it, and a
   monitor with writes on over the scratch board, started once the first step has made it.
3. **Drives a Claude Code session** headlessly (`claude -p` over stream-json), saying what the guide
   says, step by step. Under `claude -p` a question arrives in Claude's final text (with
   "(recommended)" after the recommended option); the run answers it, as the next turn, from the
   first of: a runbook rule matching the question, the recommended option, a rule matching the
   whole message (only for a question with no options), the first option, the free-text pick. It
   logs every question, its options, the pick and which rule chose it in `questions.tsv`. A question
   never stops a run; one still asked after three answers, or twenty answers in one step, is the
   finding `question-unanswered`. Each step starts on an idle session: a turn that started a
   background agent or build waits for it to finish and the stream to go quiet, within the run's
   time, else the finding `agents-still-running`.
4. **Takes each decision as the scripted person**: it first checks that the button the guide names
   is offered on the Review or Releases page (reading what the page reads), then posts what the page
   would post, with nothing typed. With a browser on the machine, a `press` step clicks the button on
   the rendered page instead, and a page claim reads the rendered text. A title selects an item by
   exact title, then whole word, then substring; several equal fits are the finding
   `ambiguous-selector`, and nothing is acted on. A sign-off on an item whose approval lapsed is
   preceded by approving it again, as the guide says, and recorded as the finding `reapproved`. A
   missing offer, a board claim that fails, or a mismatch between the board and the remote is a
   **finding**, not a stop. Findings name the line of the transcript file they come from.
5. **Pushes only to its own remotes**, and refuses any other.
6. **Checks the truth**: a shipped release's tag on the remote and its CI run, the git doctor and the
   board doctor. A doctor's errors are a `MISMATCH`; its warnings are listed beside an `OK`, for
   information.
7. **Keeps everything**: the transcript, both repositories as bundles, a copy of the board, the
   monitor's log, screenshots, the remotes' refs and tags, CI results (with `gh`), the questions,
   the prepared records it confirmed, the truth checks and the findings, under `MANIFEST.sha256`;
   `kanbanr walkthrough verify <run>` re-checks it.
8. **Derives the tables** (`decisions.tsv`, `metrics.md`) from the captures alone, so
   `kanbanr walkthrough derive <run>` gives the same bytes again.
9. **Puts the run's checks on your board.** A check whose test is `walkthrough:` kind is something
   the run proves, so it is recorded green or red directly, at the commit of the run's binary, as
   `kanbanr test` records any automated result; a result that already says the same at the same
   commit is left as it is (`R-n <test>: already recorded at <rev>`). A `manual:` check is
   prepared, and all of them form one batch: the Review page offers **Confirm all N** and
   **Reject all N** beside the single buttons; one click records them all. Each prepared result
   shows its check id (**Check** `MT-6`) beside its result, note and commit, and the batch's
   summary names its checks (`(checks MT-1, MT-4)`), so the Review page matches the run log and
   Claude's report. A check that measured nothing goes nowhere, and a run that never made its
   board (`setup-incomplete`, and `setup_failed true` in `raw/setup.log`) puts nothing on yours.

**Every result names the commit the binary was built from; a dirty or unknown build is refused
unless `--rev` names the commit.** The run reads the commit from the binary under test itself, not
from the checkout it was started in, so a run from any folder records the same commit for the same
build. A build from a modified tree (`<hash>-dirty`) or one without git metadata (`unknown`) holds
no commit that proves its code, so the run refuses before any session starts and spends nothing.
`--rev <commit>` (7 to 40 hex digits) is your statement of what the build holds: it is taken as
given, never looked up in the checkout (the build may come from another clone or worktree), and
`raw/setup.log` records it as `evidence_rev <commit>` with `evidence_rev_from --rev` (or
`the build`), so the claim can be audited.

A run stops early only at its budget (`--budget-usd`), its wall time (`--max-minutes`) or when the
session ends; the stop is a finding, and the captures are made all the same.

## The runbook

A runbook is a board doc under `walkthroughs/` whose first fenced `yaml` block holds the steps:

```yaml
runbook: togaf
process: togaf
model: sonnet
budget_usd: 40
max_minutes: 180
checks:
  MT-1: {item: FTR-034, requirement: R-5, test: "manual: in a walkthrough, every sign-off, promotion and CI result is taken with one click and nothing typed"}
answers:
  - {match: "where .*board", pick: "In ../board. Its remote is {board_remote}."}
steps:
  - id: "1.1"
    say: "Start using kanbanr for this project. Use TOGAF, with sprints and releases."
    expect: {board: {project: exists}}
  - id: "4.2"
    decide: {approve: {item: newest}}
    offer: {page: review, button: Approve, subject: {item: newest}}
    check: MT-2
  - id: "5.8"
    push: {repo: code, refspec: main}
  - id: "5.9"
    say: "CI on main passed."
    decide: {confirm_prepared: {kind: gate-result}}
    offer: {page: releases, button: "Record ci passed"}
    expect: {board: {release: {version: v0.1.0, stage: main}}, truth: {ci: {release: v0.1.0, passed: true}}}
```

- **`say`** is what the person says to Claude; `{board_remote}`, `{code_remote}`, `{monitor_url}`
  and `{today}` are filled in by the run. **`answers`** are rules (`match` is a case-insensitive
  pattern over the question's text) on a step or shared; the current step's are tried first.
- **`decide`** is one of `approve`, `signoff`, `ratify`, `adr`, `promote`, `gate_result`,
  `confirm_prepared`, `grant_delegation`, `set_mode`, `record_manual`; an item is `newest`, a code
  `{title_contains: …}` or `{title_matches: …}` (a case-insensitive pattern). Every `decide` names the **`offer`** the guide promises (`page` and
  `button`), checked first. **`press`** clicks the button on the rendered page instead.
  `press` may be a list: the presses are made in turn on one page (the first names the page), for
  a button that asks first and then its confirmation or Cancel; a press the page does not offer
  stops the list.
- **`expect`** holds a `board` claim (`project: exists`, `release: {version, stage}`,
  `item: {title_contains}` or `{title_matches}`, `item: {item, status}`, `config: {release_prepare: configured}`,
  `sprint: active`), a `truth` claim (`tag`, `ci`, `merged`, `doctor: clean`), a `page` claim
  (`url`, `contains`, `absent`) or a `talk` claim (`asked`, `not_asked`, `said`: patterns over what
  Claude asked and said in the step). In a `talk` claim, `since: <step id>` widens `asked` and
  `not_asked` to every step from that one to this, so a question Claude asks while it first scopes
  the work still counts; `said` always reads the step's own last message. `offer: {page, button,
  subject}` with `absent: true` claims that the page does not offer that button for that item.
- **`only_if: {board: …}`** runs the step only when that board claim holds, such as
  `{release: {version: v0.1.0, stage: candidate}}` for a defect found on a candidate; otherwise the
  step is skipped with the finding `skipped`.
- **`check: MT-n`** names a check from `checks`: it passes when the step checked something (a
  decision, a press, a push or an expectation) and produced no finding. A check on a step that only
  says something, or that was skipped, is **not measured**: never passed, and not put on your
  board. A marker named on several steps is one check, passed only if each measured step passed.

`kanbanr walkthrough lint walkthroughs/togaf.md` says what a run would stumble on before one starts.

## What you do

Start a run, read `findings.md` and the tables, confirm or reject the batch on the Review page with
one click, and check what only a person can judge: whether the guide's words were confusing, and
whether what Claude wrote reads well. `--pause-at-decisions` leaves each click to you and changes
nothing else.

Two runbooks ship on this board: `walkthroughs/togaf.md` and `walkthroughs/agile.md`, for the two
guides. Other projects on a board bring their own.

## Release evidence: both guides

The two runbooks share MT-1 to MT-5; MT-6 to MT-8 exist only in agile and MT-9 only in TOGAF, and
each carries MT-10 (the guide reached its cut). A release's guide checks are therefore one batch
made from two runs of one build:

1. Build and install the candidate from a clean tree. Its `kanbanr --version` names a commit with
   no `-dirty`.
2. From the kanbanr checkout, run
   `kanbanr walkthrough run walkthroughs/togaf.md --no-batch --name togaf-<version>-<n> --remotes <prefix>`.
3. Then, never in parallel (one heavy job at a time), run
   `kanbanr walkthrough run walkthroughs/agile.md --no-batch --name agile-<version>-<n> --remotes <prefix>`.
   Watch the run logs and the machine's load as for any long job.
4. Run `kanbanr walkthrough prepare --batch <version>-guides <togaf run> <agile run>`.
5. Claude reports each check by its id from both runs' `checks.tsv` and findings, and you confirm
   the batch on the Review page. MT-10 is recorded by step 4 as automated evidence, green only if
   both runs reached their cut.

`walkthrough prepare` reads only the run folders, so the batch is derived and the same every time.
Each result's step is prefixed with its run's name (`togaf-v080-1:6.20a`), and a check measured in
both runs passes only if it passed in both. It refuses, naming the run folder, and prepares
nothing when:

- a run's setup did not complete (`setup_failed true` in `raw/setup.log`);
- a run's `raw/setup.log` names no `evidence_rev` (a run from before this was recorded): run it
  again;
- two runs name different commits: "two builds are not one release's evidence";
- a run folder's `raw/MANIFEST.sha256` does not verify;
- a check id is defined differently in the two runbooks (another item, requirement or test).

A run that stopped early is noted (what it did not reach is not measured), not refused. Running
`prepare` again with the same runs prepares nothing twice: each record already open in the batch
is reported as `already prepared`, and each automated result as `already recorded at <rev>`.

## Options

| Flag | Does |
|---|---|
| `--model` | The model for the session (default: the runbook's, `sonnet`) |
| `--budget-usd` | Stop the session at this spend (default: the runbook's) |
| `--max-minutes` | Stop the run after this long (default: the runbook's) |
| `--remotes <prefix>` | Two private repositories on the remote server through `gh`, instead of bare ones under the run; `kanbanr walkthrough remotes list` and `remotes clean` find and delete them |
| `--pause-at-decisions` | Wait at each decision for you to act on the monitor |
| `--screenshot-cmd` | The command that takes a page screenshot (default: a headless Chromium or Chrome on PATH) |
| `--runs <dir>` | Where runs are kept (default: `~/.cache/kanbanr-walkthroughs`, or `KANBANR_WALKTHROUGH_RUNS`) |
| `--name` | The run's name, and the batch's |
| `--no-browser` | Never open a browser: press and page steps are findings |
| `--rev <commit>` | The commit the build under test was made from, when the binary cannot say (a dirty or unknown build); every result the run puts on the board names it |
| `--no-batch` | Put nothing on your board; prepare the batch later with `kanbanr walkthrough prepare` |

A run writes nothing outside its folder, its scratch board, its own remotes and your board's
prepared records.
