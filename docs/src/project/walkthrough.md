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
   `kanbanr test` records any automated result. A `manual:` check is prepared, and all of them form
   one batch: the Review page offers **Confirm all N** and **Reject all N** beside the single
   buttons; one click records them all. A check that measured nothing goes nowhere, and a run that
   never made its board (`setup-incomplete`) puts nothing on yours.

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
- **`expect`** holds a `board` claim (`project: exists`, `release: {version, stage}`,
  `item: {title_contains}` or `{title_matches}`, `item: {item, status}`, `config: {release_prepare: configured}`,
  `sprint: active`), a `truth` claim (`tag`, `ci`, `merged`, `doctor: clean`), a `page` claim
  (`url`, `contains`, `absent`) or a `talk` claim (`asked`, `not_asked`, `said`: patterns over what
  Claude asked and said in the step).
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

A run writes nothing outside its folder, its scratch board, its own remotes and your board's
prepared records.
