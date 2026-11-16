# Runbook: the agile guide, run headlessly

This runbook drives the guide *Agile, sprints and releases with Claude* (`docs/guide/guides/agile-with-claude.md`) from an empty folder to a cut release, with no person present. `kanbanr walkthrough run walkthroughs/agile.md` reads the fenced `yaml` block below; the prose is for the person who reviews it.

How it is used:

- **What is said** is the guide's own words. Claude's questions are answered from the `answers` rules (the current step's first, then the shared table), else the recommended option, else the first; every question and its answer is logged in `questions.tsv`, and a run never stops for one.
- **Every decision is the scripted person's click** on the monitor's write routes, on the run's own fenced board: the offer the guide names is checked first, and an offer that is missing is a finding, not a stop.
- **Pushes** go only to the run's own remotes (`remotes.env`), never anywhere else.
- **The eight v0.6.0 checks** appear as `MT-n` markers under `checks`; this run records all eight. A check passes when its step produced no finding: the offer was there, the decision went through with nothing typed, and what the step expected held, including what was asked (MT-6), what was not asked (MT-7) and what was said (MT-8).
- The delegations setup proposes are **granted** (MT-2) but the mode is left on `monitor`, as the guide says a granted delegation does nothing until the mode changes, so every docs-folded sign-off below is the person's click (MT-3).
- `{board_remote}`, `{code_remote}`, `{monitor_url}` and `{today}` are filled in by the run from `remotes.env` and the monitor it started. No remote, account or server name is written here.

```yaml
runbook: agile
process: agile
model: sonnet
budget_usd: 40
max_minutes: 180
checks:
  MT-1: {item: FTR-034, requirement: R-5, test: "manual: in a walkthrough, every sign-off, promotion and CI result is taken with one click and nothing typed"}
  MT-2: {item: FTR-028, requirement: R-5, test: "manual: a walkthrough's setup proposes delegations for its process and the person accepts them"}
  MT-3: {item: DEF-039, requirement: R-2, test: "manual: on a walkthrough board, an item with drifted docs reaches docs-folded with nothing for the person to fill in"}
  MT-4: {item: DEF-040, requirement: R-2, test: "manual: on a walkthrough board, a manual test is recorded from the Review page with no terminal command"}
  MT-5: {item: DEF-045, requirement: R-1, test: "manual: in a walkthrough, a release fix reaches the line with no command run by the person"}
  MT-6: {item: DOC-008, requirement: R-1, test: "manual: the agile walkthrough's first screen request gets a style question"}
  MT-7: {item: DOC-008, requirement: R-2, test: "manual: the agile walkthrough reaches its cut with no question about push order or the Record result form"}
  MT-8: {item: DOC-008, requirement: R-3, test: "manual: the agile walkthrough's proposed pipeline has separate rerunnable stages and stack caching"}
answers:
  - {match: "confirm (this|the) (draft|charter|plan)|what would you change", pick: "I confirm it as drafted."}
  - {match: "what would you like to (build|scope)|what (do you want|should (i|we)) (to )?(build|built|scope)|(build|scope) first|\\bfirst item\\b", pick: "Nothing yet: finish the setup, and I'll tell you what to build in a later step."}
  - {match: "^where .*\\bboard\\b|\\bboard\\b.*\\b(live|location|folder|path)\\b|data.?dir", pick: "In ../board, next to this project. Its remote is {board_remote}."}
  - {match: "\\bcode remote\\b|remote .*\\b(code|repository)\\b|^where .*\\bpush", pick: "The code remote is {code_remote}."}
  - {match: "^which process|\\bprocess\\b.*\\b(use|want|follow)\\b", pick: "Agile, with sprints and releases."}
  - {match: "\\bsprints? and releases?\\b|\\breleases? and sprints?\\b|\\bgit model\\b", pick: "Yes: sprints and releases on, and the git model on."}
  - {match: "\\bpurpose\\b|^(question \\d+: )?what is (this|the) (project|shop) for|\\bproject.s goals?\\b", pick: "A small shop where a shopper's basket survives between visits. One goal: a shopper who comes back finds their basket as they left it."}
  - {match: "\\btechnolog|\\blanguage\\b|\\b(tech )?stack\\b|\\bframework\\b|which database|built with|test runner|which runtime", pick: "Python 3 standard library and SQLite only, one HTML page served by the same program, tested with unittest. Yes, it has a screen."}
  - {match: "\\bstyl(e|ed|es|ing)\\b|look and feel|how should .* look|\\bfonts?\\b|visual direction|\\bcolou?rs?\\b", pick: "Hand-written CSS. Plain and simple: one column, system fonts, no images."}
  - {match: "\\bscreens?\\b|user interface|\\bui\\b", pick: "It has a screen."}
  - {match: "(does|will) (the|this) project (have|use) ci|\\bci\\b.*\\b(have|run|product)\\b|continuous integration", pick: "CI runs on the remote server. Keep it minimal, as the delivery-pipeline guide says: one job, one platform. Accept the proposal as it stands."}
  - {match: "delegat", pick: "Propose them; I'll grant them on the Review page."}
  - {match: "\\bskills? folder\\b|\\.claude/skills", pick: "Leave .claude/skills untracked."}
  - {match: "skill install|self-update|replace kanbanr|upgrade|newer version", pick: "No."}
  - {match: "charter", pick: "Yes, agree the charter now: the purpose above, and one goal, a shopper who comes back finds their basket as they left it."}
  - {match: "\\bsprint.s goal\\b|\\bstart (the |a )?(first )?sprint\\b|\\bsprint\\b.*\\b(length|capacity|start|dates?)\\b", pick: "Goal: Saved basket, first release. Two weeks, starting today ({today})."}
  - {match: "\\bwhich version\\b|\\bversion\\b.*\\b(first )?release\\b|\\brelease.s (version|target|date)\\b", pick: "v0.1.0, two weeks from today."}
  - {match: "\\b(start|open) (the |a )?monitor\\b|\\bwhich port\\b", pick: "The monitor is already running at {monitor_url} with writes on. Do not start another."}
  - {match: "^(shall|should|can|may) i (approve|sign|ratify|promote|cut|record|confirm)\\b", pick: "I'll decide on the monitor."}
  - {match: "^(shall|should|can|may) i\\b|^do you want (me )?to\\b|^go ahead\\?|\\bok to (proceed|go ahead)\\b", pick: "Yes, go ahead."}
steps:
  # 1 Setting it up
  - id: "1.1"
    say: "Start using kanbanr for this project. Use the agile process."
    expect: {board: {project: exists}}
  - id: "1.2"
    say: "Agree a charter with me first, then set up release stages so that main has to be pushed before anything is tagged. Use only candidate, main and production."
  - id: "1.3"
    decide: {grant_delegation: {delegation: proposed}}
    offer: {page: review, button: Grant}
    check: MT-2
  - id: "1.4"
    say: "The monitor is open at {monitor_url}, with writes on. I'll take every decision there."
  # 2 Planning a sprint
  - id: "2.1"
    say: "Start a two-week sprint for the saved basket. Put it in, for release 0.1.0."
    expect: {board: {sprint: active}}
    names: {basket: {new_item: {title_matches: "\\bbasket\\b"}}}
    answers:
    - {match: "what would you like to (build|scope)|what (do you want|should (i|we)) (to )?(build|built|scope)|(build|scope) first|\\bfirst item\\b", pick: "A shopper opens their basket on our shop's web page, leaves, and comes back the next day to find what they chose still there."}
  - id: "2.2"
    say: "A shopper opens their basket on our shop's web page, leaves, and comes back the next day to find what they chose still there. Take it to Design."
    expect: {board: {item: {named: basket}}}
  - id: "2.3"
    say: "Add a manual test to the saved basket: open the basket page, add an item, close the browser, reopen the page, and the item is still there."
  # 3 One item, stage by stage
  - id: "3.1"
    decide: {approve: {item: {named: basket}}}
    offer: {page: review, button: Approve, subject: {item: {named: basket}}}
  - id: "3.2"
    say: "Build the basket page."
    expect: {talk: {asked: "style|styling|look|feel|screen"}}
    check: MT-6
  - id: "3.3"
    say: "Move the saved basket on to Develop: write the design and its task list, and name a test for each requirement."
  - id: "3.4"
    decide: {signoff: {item: {named: basket}, name: design-review}}
    offer: {page: review, button: "Sign off design-review", subject: {item: {named: basket}}}
  - id: "3.5"
    say: "Have a sub-agent build these tasks."
  - id: "3.6"
    say: "Move the saved basket on to Test, then to Review. Tell me what waits for me on the Review page."
  # The manual check comes before docs-folded: Review holds the sign-off until the item's tests,
  # its manual one included, are green (FEAT-182).
  - id: "3.6a"
    decide: {record_manual: {item: {named: basket}, test: "basket", passed: true}}
    offer: {page: review, button: Passed, subject: {item: {named: basket}}}
    check: MT-4
  - id: "3.7"
    decide: {signoff: {item: {named: basket}, name: docs-folded}}
    offer: {page: review, button: "Sign off docs-folded", subject: {item: {named: basket}}}
  - id: "3.8"
    say: "Move the saved basket on to Review, if it has not moved already."
    expect: {board: {item: {item: {named: basket}, status: Review}}}
  # 4 A chore (the manual check is 3.6a)
  - id: "4.1"
    say: "Add a README that says how to run the shop, and name the address of the basket page in it: it's a chore. Take it to its last stage and tell me what waits for me on the Review page."
    names: {readme: new_item}
  - id: "4.2"
    decide: {approve: {item: {named: readme}}}
    offer: {page: review, button: Approve, subject: {item: {named: readme}}}
  - id: "4.3"
    say: "Move the README chore on as far as it goes. Tell me what waits for me on the Review page."
  # 5 A second item, with drifted docs
  - id: "5.1"
    say: "Add an item to rename the basket page address from /basket to /cart. Take it to Design and tell me what waits for me on the Review page."
    names: {rename: new_item}
  - id: "5.2"
    decide: {approve: {item: {named: rename}}}
    offer: {page: review, button: Approve, subject: {item: {named: rename}}}
  - id: "5.3"
    say: "Move the rename on to Develop: write its design and task list."
  - id: "5.4"
    decide: {signoff: {item: {named: rename}, name: design-review}}
    offer: {page: review, button: "Sign off design-review", subject: {item: {named: rename}}}
  - id: "5.5"
    say: "Build the rename yourself, then move it on to Test and Review. Check every doc that names the old address. Tell me what waits for me on the Review page."
  - id: "5.6"
    decide: {signoff: {item: {named: rename}, name: docs-folded}}
    offer: {page: review, button: "Sign off docs-folded", subject: {item: {named: rename}}}
    check: MT-3
  - id: "5.7"
    say: "Move the rename on to Review, if it has not moved already."
  # 6 Releasing
  - id: "6.1"
    say: "Design the CI and the release workflow for this project, before the first release. Propose separate stages I can rerun one at a time, with caching for our stack, on one platform to start. Tell me which checks our plan can't run. Use the release notes kanbanr wrote as the release description. Add a manual run that builds everything and publishes nothing."
    expect: {talk: {said: "stage[s]?[^.]*(rerun|re-run|one at a time)|cach"}}
    check: MT-8
    answers:
    - {match: "can (your|the|our) (plan|settings|runners?|account)|which of these can|what .*plan (can|can't|cannot) run", pick: "Keep the separate stages, each a job I can rerun on its own, with caching for our stack; run every stage our plan allows."}
  - id: "6.2"
    say: "Build the pipeline as an item for the first release and take it as far as it goes; tell me what waits for me on the Review page."
  - id: "6.3"
    say: "Set up the release files for v0.1.0: the notes path, the changelog and the version files."
    expect: {board: {config: {release_prepare: configured}}}
  # The CHANGELOG chore and the pipeline item the guide has Claude file: approved and built.
  - id: "6.3a"
    decide: {approve: {item: {title_contains: CHANGELOG}}}
    offer: {page: review, button: Approve, subject: {item: {title_contains: CHANGELOG}}}
  - id: "6.3b"
    decide: {approve: {item: {title_matches: "pipeline|\\bci\\b|workflow"}}}
    offer: {page: review, button: Approve, subject: {item: {title_matches: "pipeline|\\bci\\b|workflow"}}}
  - id: "6.3c"
    say: "Build the CHANGELOG chore and the pipeline item yourself, and take both as far as they go. Tell me what waits for me on the Review page."
  - id: "6.3d"
    decide: {signoff: {item: {title_contains: CHANGELOG}, name: docs-folded}}
    offer: {page: review, button: "Sign off docs-folded", subject: {item: {title_contains: CHANGELOG}}}
  - id: "6.3e"
    decide: {signoff: {item: {title_matches: "pipeline|\\bci\\b|workflow"}, name: docs-folded}}
    offer: {page: review, button: "Sign off docs-folded", subject: {item: {title_matches: "pipeline|\\bci\\b|workflow"}}}
  - id: "6.4"
    say: "Take every item in v0.1.0 as far as it goes, and tell me what waits for me on the Review page."
  # Follow-up items Claude filed into the release on its own recommendation (a docs correction, a
  # formatter chore): the person approves and signs what the Review page offers for them.
  - id: "6.4a"
    decide: {approve_waiting: {release: v0.1.0}}
    offer: {page: review, button: Approve}
  - id: "6.4b"
    say: "Take every item in v0.1.0 as far as it goes again, and tell me what waits for me on the Review page."
  - id: "6.4c"
    decide: {signoff_waiting: {release: v0.1.0, name: design-review}}
    offer: {page: review, button: "Sign off design-review"}
  - id: "6.4d"
    say: "Build what is left in v0.1.0 and take it as far as it goes. Tell me what waits for me on the Review page."
  - id: "6.4e"
    decide: {signoff_waiting: {release: v0.1.0, name: docs-folded}}
    offer: {page: review, button: "Sign off docs-folded"}
  - id: "6.4f"
    say: "Move every item in v0.1.0 on to Review, if it has not moved already."
  - id: "6.5"
    decide: {promote: {release: v0.1.0, to: candidate}}
    offer: {page: releases, button: "Promote v0.1.0 to candidate"}
    expect: {board: {release: {version: v0.1.0, stage: candidate}}}
  - id: "6.6"
    say: "Write the release notes and the changelog for readers."
  - id: "6.7"
    say: "Run the full checks over the release."
  - id: "6.8"
    say: "While the release is a candidate we found that the basket page's title does not name the shop: it should read \"Your basket - the corner shop\". Add it as a defect with that in its title, and fix it in the release. Take the defect as far as it goes and tell me what waits for me on the Review page."
    expect: {board: {item: {named: defect}}}
    check: MT-5
    only_if: {board: {release: {version: v0.1.0, stage: candidate}}}
    answers:
    - {match: "how should i (record|file|track)|record (it|this|the (change|fix))|\\bamend\\b|new item or|defect or", pick: "File it as a defect, with \"corner shop\" in its title."}
    names: {defect: new_item}
  - id: "6.9"
    decide: {approve: {item: {named: defect}}}
    offer: {page: review, button: Approve, subject: {item: {named: defect}}}
    only_if: {board: {release: {version: v0.1.0, stage: candidate}}}
  - id: "6.10"
    say: "Move the defect on as far as it goes. Tell me what waits for me on the Review page."
    only_if: {board: {release: {version: v0.1.0, stage: candidate}}}
  - id: "6.11"
    decide: {signoff: {item: {named: defect}, name: docs-folded}}
    offer: {page: review, button: "Sign off docs-folded", subject: {item: {named: defect}}}
    only_if: {board: {release: {version: v0.1.0, stage: candidate}}}
  - id: "6.12"
    say: "Move the defect on to Review, if it has not moved already. Prepare the release again if the release files changed, then tell me whether Promote to main is offered."
    only_if: {board: {release: {version: v0.1.0, stage: candidate}}}
  - id: "6.13"
    decide: {promote: {release: v0.1.0, to: main}}
    offer: {page: releases, button: "Promote v0.1.0 to main"}
    expect: {board: {release: {version: v0.1.0, stage: main}}}
  - id: "6.14"
    push: {repo: code, refspec: main}
  - id: "6.15"
    say: "I pushed main for v0.1.0. CI on main passed for that commit. Prepare the result for me to confirm on the Releases page."
    decide: {confirm_prepared: {kind: gate-result, release: v0.1.0}}
    offer: {page: releases, button: "Record ci passed"}
    expect: {talk: {not_asked: "push order|order .*push|record result form|fill in"}}
  - id: "6.16"
    decide: {promote: {release: v0.1.0, to: production}}
    offer: {page: releases, button: "Cut v0.1.0"}
    expect: {board: {release: {version: v0.1.0, stage: production}}}
    check: MT-7
  - id: "6.17"
    push: {repo: code, refspec: v0.1.0}
    expect: {truth: {tag: {release: v0.1.0}}}
    check: MT-1
  - id: "6.18"
    say: "I pushed the tag. Bring the release forward into the sprint and tell me what you did."
  # 7 Closing the sprint
  - id: "7.1"
    say: "Close the sprint and do a retro."
  - id: "7.2"
    say: "Check for leftovers."
    expect: {truth: {doctor: clean}}
  - id: "7.3"
    push: {repo: board, refspec: "{board_branch}"}
```



