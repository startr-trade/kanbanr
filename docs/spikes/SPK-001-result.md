# SPK-001 result: a view at a position (main, a branch, uncommitted)

Written for [FTR-003](/p/kanbanr/feature/FTR-003) and the maintainer. Spike [SPK-001](/p/kanbanr/feature/SPK-001); chain decisions CD-2 and CD-3 of katalogr FEAT-043; rests on [ADR-0014](/p/kanbanr/docs/file/decisions/0014-code-intelligence-is-linked-by-a-contrac.md).

## Verdict

FTR-003 can build on this reading of git as it stands. The position (CD-2) and the view ID (CD-3) match katalogr's shared file and vector byte for byte; a file read at a position is exact or an error; the same view gives the same bytes and ID on repeat runs at main, at a branch and with uncommitted changes. The measured cost is under the proposed 200 ms budget with no stat cache. Two gaps to decide: submodules (below) and line endings (a decision already taken by CD-2, shown below).

## What was built

All in kanbanr-core, none reachable from the CLI, daemon or monitor:

- `position.rs`: `PositionRef` (`text`, `parse`, `at_commit`, `is_clean`, `short`), `PositionError`, `is_view_id`, `blob_id` (git2 `Oid::hash_object`, no write), `dirty_hash`, `WorkingState`, `read_position`, `read_at`, `tree_at`, `ViewQuery`, `VIEW_FORMAT`, `view_id`.
- `views.rs`: `facts_version`, `ViewAnswer` with a typed graph (katalogr's shape), `docs_view`, `answer_bytes`, `proof`, `ProofTable::render`, and one helper the Design did not name, `state_at` (a branch or commit as a clean position, or the live checkout), which the example and `proof` share.
- `coderepo::git_raw` (stdout as bytes, for `-z` listings).
- `examples/spike_view.rs` and `tools/spike/view-at-a-position.sh`: the proof; not shipped.
- `fixtures/contract/position-v1.json`: a byte-for-byte copy of katalogr's file from its `int/contract` branch.
- `sha2 = "0.10"` declared in kanbanr-core; it was already in the lockfile (0.10.9), so the lockfile gained only the dependency edge. `cargo deny check` from api/: advisories ok, bans ok, licenses ok, sources ok.

## The proof (R-5)

`tools/spike/view-at-a-position.sh`, run once on the final code, exit 0. It builds a scratch repository under the temporary directory (main with two docs; a branch with a committed edit; on that branch an uncommitted edit adding a Mermaid block, an untracked doc and a deleted doc) and a stand-in board repository. Each position is read twice; the table shows the second read repeated the bytes and the ID. The commit ids differ from run to run (commit times); within a run they repeat. The same scenario is the integration test `views::tests::the_proof_table_shows_three_positions_each_repeating`, which also shows an edit between the two reads makes the proof fail.

| Position | Written doc (docs/guide.md) | View ID | Bytes repeat | ID repeats |
|---|---|---|---|---|
| main | blob 1fb6c35ac9 | v1-dcc13aceef4a72d45dc5427a14062622 | yes | yes |
| branch feat/SPK-001-proof | blob ba10a21b60 (edited on the branch) | v1-ed8caa7a93163158c1550ea65ab61c07 | yes | yes |
| working tree (uncommitted) | absent (deleted), overview.md now has 1 diagram | v1-7c1a937c5bc403b55ba4ce15d7198e6a | yes | yes |

The three IDs are pairwise distinct. Full transcript:

```text
scratch repository: main, branch feat/SPK-001-proof (checked out) with an uncommitted edit to
docs/overview.md, an untracked docs/new.md and docs/guide.md deleted; written doc: docs/guide.md

== at main ==
position: 511e08ea3f0a501b11bd91397a5923b7b9c3a425
doc docs/guide.md (blob 1fb6c35ac90ba50bc0a23cbd7bdbfe3dfa1cdc7b):
# Guide

The guide.
view mermaid:
flowchart LR
%% kanbanr view: docs --max-nodes 60 --root docs
  ddc47fd6761f51d72["docs/"]
  n70bb5115f916ef90["guide.md"]
  nc04ba54099046c53["overview.md"]
  ddc47fd6761f51d72 --> n70bb5115f916ef90
  ddc47fd6761f51d72 --> nc04ba54099046c53
view bytes: {"schema":1,"view_format":1,"view_id":"v1-dcc13aceef4a72d45dc5427a14062622","exact":true,"positions":[{"repo":"spike","commit":"511e08ea3f0a501b11bd91397a5923b7b9c3a425","dirty":null}],"query":{"op":"docs","args":{"max-nodes":"60","root":"docs"}},"mermaid":"flowchart LR\n%% kanbanr view: docs --max-nodes 60 --root docs\n  ddc47fd6761f51d72[\"docs/\"]\n  n70bb5115f916ef90[\"guide.md\"]\n  nc04ba54099046c53[\"overview.md\"]\n  ddc47fd6761f51d72 --> n70bb5115f916ef90\n  ddc47fd6761f51d72 --> nc04ba54099046c53\n","graph":{"edges":[{"from":"ddc47fd6761f51d72","kind":"holds","to":"n70bb5115f916ef90"},{"from":"ddc47fd6761f51d72","kind":"holds","to":"nc04ba54099046c53"}],"nodes":[{"id":"ddc47fd6761f51d72","kind":"folder","label":"docs/","name":"docs","path":"docs"},{"id":"n70bb5115f916ef90","kind":"doc","label":"guide.md","name":"guide.md","path":"docs/guide.md","blob":"1fb6c35ac90ba50bc0a23cbd7bdbfe3dfa1cdc7b","bytes":20,"diagrams":0},{"id":"nc04ba54099046c53","kind":"doc","label":"overview.md","name":"overview.md","path":"docs/overview.md","blob":"3174043db02fc6d53136772adde10d9423232c57","bytes":26,"diagrams":0}],"shown":2,"title":"docs --max-nodes 60 --root docs","total":2}}
view id: v1-dcc13aceef4a72d45dc5427a14062622

== proof ==
at                   position                                           doc blob     view id                              bytes repeat id repeats
main                 511e08ea3f0a501b11bd91397a5923b7b9c3a425           1fb6c35ac9   v1-dcc13aceef4a72d45dc5427a14062622  yes          yes
positions distinct, every second read identical: yes
== at feat/SPK-001-proof ==
position: 782e3048fb8eb81cc77976cc6ee9eac6e8338cb2
doc docs/guide.md (blob ba10a21b609772832cd853c2f94497482380d04c):
# Guide

The guide, edited on the branch.
view mermaid:
flowchart LR
%% kanbanr view: docs --max-nodes 60 --root docs
  ddc47fd6761f51d72["docs/"]
  n70bb5115f916ef90["guide.md"]
  nc04ba54099046c53["overview.md"]
  ddc47fd6761f51d72 --> n70bb5115f916ef90
  ddc47fd6761f51d72 --> nc04ba54099046c53
view bytes: {"schema":1,"view_format":1,"view_id":"v1-ed8caa7a93163158c1550ea65ab61c07","exact":true,"positions":[{"repo":"spike","commit":"782e3048fb8eb81cc77976cc6ee9eac6e8338cb2","dirty":null}],"query":{"op":"docs","args":{"max-nodes":"60","root":"docs"}},"mermaid":"flowchart LR\n%% kanbanr view: docs --max-nodes 60 --root docs\n  ddc47fd6761f51d72[\"docs/\"]\n  n70bb5115f916ef90[\"guide.md\"]\n  nc04ba54099046c53[\"overview.md\"]\n  ddc47fd6761f51d72 --> n70bb5115f916ef90\n  ddc47fd6761f51d72 --> nc04ba54099046c53\n","graph":{"edges":[{"from":"ddc47fd6761f51d72","kind":"holds","to":"n70bb5115f916ef90"},{"from":"ddc47fd6761f51d72","kind":"holds","to":"nc04ba54099046c53"}],"nodes":[{"id":"ddc47fd6761f51d72","kind":"folder","label":"docs/","name":"docs","path":"docs"},{"id":"n70bb5115f916ef90","kind":"doc","label":"guide.md","name":"guide.md","path":"docs/guide.md","blob":"ba10a21b609772832cd853c2f94497482380d04c","bytes":42,"diagrams":0},{"id":"nc04ba54099046c53","kind":"doc","label":"overview.md","name":"overview.md","path":"docs/overview.md","blob":"3174043db02fc6d53136772adde10d9423232c57","bytes":26,"diagrams":0}],"shown":2,"title":"docs --max-nodes 60 --root docs","total":2}}
view id: v1-ed8caa7a93163158c1550ea65ab61c07

== proof ==
at                   position                                           doc blob     view id                              bytes repeat id repeats
feat/SPK-001-proof   782e3048fb8eb81cc77976cc6ee9eac6e8338cb2           ba10a21b60   v1-ed8caa7a93163158c1550ea65ab61c07  yes          yes
positions distinct, every second read identical: yes
== at working tree ==
position: 782e3048fb8eb81cc77976cc6ee9eac6e8338cb2+db50e3012b40f9617b1fcc91f5f63ecfdbe7dd53e9dedbd001aa04a6d385d7f2
doc docs/guide.md: absent at this position
view mermaid:
flowchart LR
%% kanbanr view: docs --max-nodes 60 --root docs
  ddc47fd6761f51d72["docs/"]
  n4fa5b8e21bcd0e02["new.md"]
  nc04ba54099046c53["overview.md (1 diagram)"]
  ddc47fd6761f51d72 --> n4fa5b8e21bcd0e02
  ddc47fd6761f51d72 --> nc04ba54099046c53
view bytes: {"schema":1,"view_format":1,"view_id":"v1-7c1a937c5bc403b55ba4ce15d7198e6a","exact":true,"positions":[{"repo":"spike","commit":"782e3048fb8eb81cc77976cc6ee9eac6e8338cb2","dirty":"db50e3012b40f9617b1fcc91f5f63ecfdbe7dd53e9dedbd001aa04a6d385d7f2"}],"query":{"op":"docs","args":{"max-nodes":"60","root":"docs"}},"mermaid":"flowchart LR\n%% kanbanr view: docs --max-nodes 60 --root docs\n  ddc47fd6761f51d72[\"docs/\"]\n  n4fa5b8e21bcd0e02[\"new.md\"]\n  nc04ba54099046c53[\"overview.md (1 diagram)\"]\n  ddc47fd6761f51d72 --> n4fa5b8e21bcd0e02\n  ddc47fd6761f51d72 --> nc04ba54099046c53\n","graph":{"edges":[{"from":"ddc47fd6761f51d72","kind":"holds","to":"n4fa5b8e21bcd0e02"},{"from":"ddc47fd6761f51d72","kind":"holds","to":"nc04ba54099046c53"}],"nodes":[{"id":"ddc47fd6761f51d72","kind":"folder","label":"docs/","name":"docs","path":"docs"},{"id":"n4fa5b8e21bcd0e02","kind":"doc","label":"new.md","name":"new.md","path":"docs/new.md","blob":"83a85c04940aaa1f4bc9dda3222f8dbdb72531e8","bytes":18,"diagrams":0},{"id":"nc04ba54099046c53","kind":"doc","label":"overview.md (1 diagram)","name":"overview.md","path":"docs/overview.md","blob":"4f209e20a7b6c59b2dc848bd1c769fa46c68d264","bytes":89,"diagrams":1}],"shown":2,"title":"docs --max-nodes 60 --root docs","total":2}}
view id: v1-7c1a937c5bc403b55ba4ce15d7198e6a

== proof ==
at                   position                                           doc blob     view id                              bytes repeat id repeats
working tree         782e3048fb8eb81cc77976..e9dedbd001aa04a6d385d7f2   absent       v1-7c1a937c5bc403b55ba4ce15d7198e6a  yes          yes
positions distinct, every second read identical: yes
######## the three together
== proof ==
at                   position                                           doc blob     view id                              bytes repeat id repeats
main                 511e08ea3f0a501b11bd91397a5923b7b9c3a425           1fb6c35ac9   v1-dcc13aceef4a72d45dc5427a14062622  yes          yes
feat/SPK-001-proof   782e3048fb8eb81cc77976cc6ee9eac6e8338cb2           ba10a21b60   v1-ed8caa7a93163158c1550ea65ab61c07  yes          yes
working tree         782e3048fb8eb81cc77976..e9dedbd001aa04a6d385d7f2   absent       v1-7c1a937c5bc403b55ba4ce15d7198e6a  yes          yes
positions distinct, every second read identical: yes
```

## The numbers (R-6)

`position::tests::read_position_10k_files_is_measured`: a generated repository of 10,000 tracked files (200 to 2,000 bytes, 100 folders) plus one, committed; three warm reads of a clean checkout, median printed.

| Build | Median | The three reads | Load average (1, 5, 15 min) |
|---|---|---|---|
| release (the budget's scenario) | **141.8 ms** | 124.7, 141.8, 142.5 ms | 6.60, 6.65, 7.79 |
| debug | 224.1 ms | 196.9, 224.1, 334.4 ms | 6.25, 7.04, 8.23 |

This repository (429 tracked files, release): 45.2 ms for one read; debug 158 ms.

Caveats: the machine was not idle (load about 6.6 on a 12-core machine, other builds queued behind the build lock), so the release figure is an upper estimate for an idle machine; katalogr measured 133.6 ms for the same uncached algorithm. The proposed budget of 200 ms warm at 10,000 files is met with 58 ms to spare (29 percent). The debug build misses it, so FTR-003's budget test must say it is a release-build figure (FTR-003 R-10 asserts budget x 2 under load above 4, which both builds meet). The cost is mostly fixed process and file work: four git processes plus one read and hash per tracked path; I did not profile further.

## Shared file outcome (R-1)

All 8 cases of `position-v1.json` (clean, edited, deleted, untracked, ignored, staged_only, edited_and_staged, line_endings) give exactly their expected text, cold and on a second read, built with real git as katalogr's test builds them (fixed author, `@0 +0000`, message by stdin, config after the commit, index entries by `update-index --cacheinfo`). The ID vector `v1-096ec1a42f3e9b74ae0f0348152caeb2`, the dirty-hash vectors (`83ebb47e...`, `c09b5a87...`) and the blob id of `changed
` (`5ea2ed41...`) match, and katalogr's twelve bad position texts are refused.

## Line endings

The `line_endings` case: HEAD holds `a
b
`, the worktree `a
b
`, with `core.autocrlf=true` and `* text=auto`. `git status` calls it clean because it applies filters; a position hashes the raw worktree bytes, so it is dirty (`24decf67...+3323e56b...`). That is CD-2 as written and is what we match. Consequence for FTR-003: on a Windows checkout with autocrlf, every text file reads as changed, so the position is dirty and a view is exact only at the live working-tree position, never at the HEAD commit it would otherwise equal. FTR-003 should say so in its docs and not try to normalise; that is a katalogr decision.

## Submodule gap

A gitlink (type `commit` in `ls-tree`) is skipped, as katalogr's code skips it, so an uncommitted change inside a submodule, or a submodule checked out at another commit, does not move the position. I confirmed it on a scratch repository with a submodule: editing a tracked file and adding an untracked file inside it left the position unchanged while `git status` shows `m vendor`. CD-2's text says "every tracked path", so the text and katalogr's code disagree; this should be reported to katalogr (the decided Q4 a). Committing a new submodule pointer does change the commit, so committed positions are exact. A nested repository that is untracked appears as a single `dir/` entry in `ls-files --others` and is ignored (it is not a file). Recommendation: take it as is for the docs view (nothing in it lives in a submodule), record the gap in FTR-003's docs, and let katalogr decide whether CD-2 should include the submodule's own position.

## Stat cache

Recommendation: do not build one in FTR-003. The uncached read is 142 ms at 10,000 files under load, inside the 200 ms proposal, and most repositories are far smaller (this one, 45 ms). A cache adds invalidation rules (new ignore rules, `info/exclude`, clock skew; katalogr needs tests for each) that are the riskiest part of exactness, for a saving only visible beyond roughly 15,000 files. Revisit if the maintainer's real repositories measure above the budget; the algorithm leaves room for one in `read_position` without changing its signature.

## Other findings

- `read_position` and `tree_at` assume `root` is the top of the work tree (`ls-files` and `ls-tree` print paths relative to the current directory). The CLI resolves the repository root first (`scm::repo_root()`), so FTR-003 passes that.
- Paths are bytes in git and lossy UTF-8 in `WorkingState`; a non-UTF-8 path round-trips through the hash correctly but `read_at` takes a `&str`, so such a path cannot be read at a position. Rare; acceptable for the docs view.
- A file moved between listing and hashing counts as deleted; `read_at` refuses an uncommitted file whose bytes no longer hash to the recorded id ("position moved"), so the view is exact or an error, never a mix. The retry and the `exact: false` answer are FTR-003's.
- A symbolic link is hashed as its target text (as git stores it); a mode-only change is not a difference.

## What FTR-003 should take as is, and what to change

Take as is: the whole of `position.rs` (every public name FTR-004 and FTR-016 assume is present with the Design's signature), `coderepo::git_raw`, `views::facts_version`, `ViewAnswer` and its graph, `docs_view`, `answer_bytes`, `proof`, `ProofTable::render` and `state_at`, the shared fixture and its test, and the position and view tests. Remove, as FTR-003's T8 says: the example and the script.

Change or decide: (1) the budget value: set it from the 141.8 ms release median, 200 ms is met; (2) the submodule gap: document it and tell katalogr; (3) the line-ending behaviour: document it; (4) no stat cache; (5) `views::tests::...` and `read_position_10k_files_is_measured` should assert on release only, or carry the x2-under-load rule.
