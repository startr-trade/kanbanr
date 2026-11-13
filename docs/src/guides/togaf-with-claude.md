# TOGAF, sprints and releases with Claude

This guide takes a project from setup to a shipped release using TOGAF phases, sprints and releases.
You don't need to learn any commands:

- **You talk to Claude** for everything except decisions and pushes. Use your own words. The ones
  here are only examples.
- **You decide on the monitor:** approvals and sign-offs on the **Review** page, promotions and the
  cut on the **Releases** page.
  Claude tells you what is waiting there and links to it. It never records a decision, and it
  doesn't ask for one in a plan or a question. What Claude already knows, such as a CI run's
  result or what a sign-off covered, arrives filled in beside the button: you check it and click.
- **You push from your terminal.** Nothing reaches your remote server until you do.

Prefer to run the commands yourself? See [TOGAF from the command line](togaf-cli.md).

## Three clocks

Three things move at their own pace, and most confusion comes from mixing them up:

| | What it answers | Moves when |
|---|---|---|
| **TOGAF phase** | How far has this item got? | Its gates are met, one item at a time |
| **Sprint** | What are we working on these two weeks? | You ask Claude to start or close it |
| **Release** | What has reached users? | You promote it on the Releases page |

An item can be finished in its sprint and still not released. Merging is not shipping: **an item is
released only when a release that holds it is cut.**

```mermaid
flowchart LR
  A["System Design"] -->|"build starts"| B["item branch"]
  B -->|"move to Migration"| C["sprint branch"]
  C -->|"Promote to candidate"| D["release branch"]
  D -->|"Promote to main"| E["main"]
  E -->|"Cut"| F["tag"]
```

## Setting it up

> Start using kanbanr for this project. Use TOGAF, with sprints and releases.

Claude asks where the board should live and what the project is for. It then writes a plan showing
the process, the branches it will use and where each one merges. **Accepting the plan applies it.**

Ask for the things that make the rest of this guide work:

> Agree a charter with me first, then set up release stages so that main has to be pushed before
> anything is tagged.

**Do the charter first.** Gates aren't enforced until a project has one, and items created before it
are exempt. Without one, a move that a gate would have refused still goes through, with a warning
saying what was missing. Claude asks for the project's purpose and goals. Every item then links to a goal.

**Releases can have stages after the tag.** The tag only says which commit the release is. Stages
after it, such as staging or production, are for you to confirm once you have seen the release
work there. The release ships, and its items reach their last status, when you confirm the last
one. If CI fails on the tag, record the failed result and choose **Start a patch release**: Claude
files the defect with the run's link and your notes, and the failed release is marked superseded.
Claude tells you where the release is and never confirms a stage for you.

**You record CI's verdict.** kanbanr can see that main was pushed, but it never asks a server whether
CI passed. You look at CI yourself, then record what it said on the **Releases** page (**Record
result**, with a link to the run if you like). A release stage can require that result before the
cut. Claude tells you when it is missing and never records it for you.

**The stages are candidate, main and tag.** A project that turns releases on gets these three, each
doing something: candidate prepares the release, main moves main once the items are finished, and tag
cuts the release once main is pushed. If you have CI, say so and the tag waits for your recorded
result. Ask for other stages only if you have a pipeline behind them. Releases on a board made before
this keep the four they had, `test` and `staging` among them. Those two do nothing, and Claude says so
when you configure them.
Those two record that a release passed a point and do nothing else: nothing builds or deploys when
you promote to them. Add them when you have a pipeline and environments behind them.

> Open the monitor, with writes on.

Keep it open. Claude gives you the address, and every decision below happens there.

**Sprints and Migration.** Migration counts as done for a sprint, and Operations is when the release
ships the item. So closing a sprint before its release is cut carries over only work that has not
merged.

## Planning a sprint

Setup does not create a sprint or a release. Ask for them when you are ready, and Claude asks the
sprint's goal and dates, or the release's version and target, then. A planned sprint can be changed
until it starts, and an empty sprint or release made by mistake can be removed.

> Start the first sprint. A shopper opens their basket on our shop's web page,
> leaves, and comes back the next day to find what they chose still there. Put that in the sprint, for the first release.

Claude starts the sprint and creates its branch, creates the item at Vision, and plans it into the
sprint and the release. Your checkout doesn't move.

**Keep 1.0 for something people can use.** Name your first release v0.1.0 unless it really is ready
for them.

A sprint and a release don't have to line up. One release can take items from several sprints.

## One item, phase by phase

Ask for the work in your own words, and Claude asks only what it can't work out. Each phase has a
gate. When something's missing, Claude either drafts it or brings it to you.

> Take the saved basket to Business Arch.

**Claude asks whether it has a user interface.** When you choose the technology (or, if you don't,
when the first item is designed), Claude asks whether the product has screens a person uses, or is
a service, library or command-line tool. If it has screens, they become items of their own,
designed and approved like the rest. If it has none, that goes into the design, so a missing screen
is a decision and not an oversight.

| Phase | What Claude does | What you do on the Review page |
|---|---|---|
| **Vision** | Writes what it is and why | Nothing yet. The page may offer **Approve**, but it isn't needed here |
| **Business Arch** | Drafts who, what, why and the requirements | **Approve** |
| **System Design** | Proposes how and where, and checks that against the requirements | Nothing: the approval stands while the definition grows |
| **Implementation** | Writes the design and its task list, and names a test for each requirement | **Sign off design-review**, which also reviews what was added |
| **Migration** | Builds on the item's branch, runs the tests and updates the docs | **Sign off docs-folded**, which appears once the docs are built; until then the Review page does not offer it. Claude first checks any doc or item that names something the item changed and has not changed since, confirming it is still right or filing a correction; the Review page shows what is left and what Claude checked, and you fill in nothing |
| **Operations** | Nothing: the cut moves it here | **Sign off release** |

**Approve first, then sign off.** An item with both waiting shows **Approve**, then **Sign off** once
you've approved it, so check back after approving.

**Each decision notes where you made it.** The monitor, your terminal, and your answer to Claude
are told apart, and the item shows which one next to each approval and sign-off. If you decide
the same thing twice, say a click and then an answer to Claude, the second one records nothing and
Claude tells you who decided first and when.

**You approve once, and review what's added.** Each approval covers the text you saw. Adding answers
keeps it, and the Review page shows what was added when the next decision comes, usually the design
review. Changing or removing what you approved lapses it, and the item returns to the Review page.

**You decide on the Review page.** Claude tells you what's waiting there and links to it, and never records a decision for you. kanbanr refuses if it tries. If the monitor isn't running, Claude offers to start it.

**A design can bring decisions with it.** Claude drafts each one as a decision record, and you use
**Accept** or **Reject** on the Review page.

**A changed spec comes back to you.** Your **design-review** covers the item's spec as you saw it. If
the spec changes afterwards, even outside the Design, the item returns to the Review page showing
only what changed, and you sign again.

**Operations means shipped.** **Sign off release** records your sign-off and leaves the item where it
is. The item moves to Operations when the release it is planned into is cut. The **Releases** page
tells you what has shipped.

**Just ask to move it on:**

> Move the saved basket on to System Design.

If something's missing, Claude tells you what and either fixes it or asks you. Nothing moves past a
gate unless you say why, and that reason stays on the item.

**Moving to Migration is the merge.** kanbanr merges the item's branch into the sprint's branch. It
isn't released yet.

### Chores

A chore doesn't walk every phase. It goes Vision → Implementation → Migration → Operations, skipping Business Arch and System Design, because their
questions don't fit small maintenance work. Tell Claude it's a chore when you ask for it.

> Pin the CI image: it's a chore.

Before its first move Claude asks once for what every item records: a one-sentence statement, the goal
it serves, and a requirement in the EARS form that says how it's verified. You still approve it before the build starts, and you still give the **release** sign-off at the end. A chore has no Design section and no **design-review**, because there is no design to review. If
you change an item's kind to a chore while it's at a phase the chore path leaves out, it's off
the path: it moves by the ordinary moves, or back to Vision.

## Handing work to sub-agents

> Have a sub-agent build these tasks.

Claude gives each agent a scope, which kanbanr enforces, so you don't have to rely on the agent's
brief. An agent can record its own tasks and test results and commit on its own branch. It can't
move items, approve anything or push.

You'll see its progress on the item's page as it works, and its scope on the **Workflow** page. When
it finishes, Claude reviews the branch and brings you any decisions.

Claude stays in the project folder while an agent works, and looks inside the agent's worktree
without moving into it (`git -C`), so a scope never gets in your way or Claude's. Revoking an agent's
scope stops that agent on its next tool call and never restricts you.

## Releasing

**Before your first release, design the pipeline.** A release workflow can only be tested by running
it, a tag can't be moved once it's pushed, and shipped notes can't be edited, so a mistake in the
pipeline costs a whole version number. Settle the checks, the release description and a run without a tag first:
see [Designing CI and the release workflow](delivery-pipeline.md).

Start once every item in the release has reached Migration. **Promote to candidate** is offered once
every item in the release has reached Migration, and the Releases page lists any that has not.
When you plan an item into a release while a sprint is running, ask Claude to plan it into the
sprint too; Claude does that unless you say otherwise, and `kanbanr doctor` names any item planned
into a release but into no sprint.

Follow this order and you won't have to go back:

| Step | Who | Do this |
|---|---|---|
| 1 | You | **Releases** page: **Promote to candidate**. This makes the release branch and drafts the version, changelog and notes |
| 2 | Claude | Say "Write the release notes for readers", because the draft only lists requirements |
| 3 | You | Open **Release notes** on the release, then **Edit notes** and **Save** for any changes |
| 4 | Claude | Say "Run the full checks over the release" |
| 5 | You | **Review** page: **Sign off release** on each item, if you haven't already |
| 6 | You | **Releases** page: **Promote to main**. This moves main to the release, without tagging |
| 7 | You | Terminal: `git push origin main`, then wait for CI to go green |
| 8 | You | **Releases** page: **Check again**, then, with CI green, **Cut v0.1.0**. This tags main. If your stages continue after the tag, the page shows the result you recorded for it; when it is green, **Confirm** each later stage with a note of what you checked. Confirming the last one moves any item not already there to Operations |
| 9 | You | Terminal: `git push origin v0.1.0`, then wait for the release workflow |
| 10 | Claude | Brings the release forward into the sprint and tells you; you push the sprint branch when you push |

**The release date is today's.** Moving main dates the notes and changelog with the day it happens.
If the Releases page warns that they carry an earlier date, say "Date the release today", and push
main again if Claude says to.

**Each button says what it will do**, and stays greyed out with the reason until it's ready. If it
says an item would carry over, that item usually needs its **Sign off release**. A promotion to the
cut or to the last stage is held, not carried, for an item that is already merged: the page names
the item and the **Sign off release** it lacks, and nothing moves until you give it or plan the item
into another release. Only work that isn't done or isn't merged is carried to the next release.

**Freeze the candidate.** Once a release is a candidate, tell Claude to send anything new to the next
release, and change only the notes. Reopening a candidate for each thing you find is how a release
never closes.

**Promoting moves main, not your checkout.** You may still be on the sprint's branch, and your remote
server won't change until you push.

**If you save notes after promoting to main,** main moves with them. The page tells you to push main
again and wait for CI before the cut.

**The date follows the release.** Promoting to main dates the notes and changelog today. If main moved
earlier and the cut comes later, the page warns you: ask Claude to re-date them before you cut,
because shipped notes can't be edited.

**A change merged into a promoted release sends it back.** When a fix is merged into a release that
has gone past candidate and isn't cut, the release goes back to candidate, and Claude tells you in
one message which item did it. The Releases page then shows a panel with the item and the steps from
there, and you do them in order:

1. Say "Write the release notes again"; the page lists the items the notes don't mention yet.
2. Say "Run the full checks over the release". A recorded pass for an earlier commit still counts if
   only documentation changed since, and the page names that commit.
3. **Releases** page: **Promote to main**.
4. Terminal: `git push origin main`, then wait for CI to go green.
5. **Releases** page: **Check again**, then **Cut**. The cut stays greyed out until main is at the
   release and pushed.

Claude reports where the release is and confirms no stage; the buttons are yours. Saving notes alone
doesn't send the release back. A release that is already tagged and waiting for its later stages
takes no change: the merge is refused, and the change needs a patch release (**Start a patch
release** on the page, or ask Claude).

**A fix made during the release reaches the sprint the same way:** Claude brings it forward.

## Closing the sprint

> Close the sprint and do a retro.

Claude closes it, shows what carried over, and writes the retro with you. This changes only the
board: no branch moves and nothing is tagged. Start the next sprint before starting new items,
because new items branch from the running sprint.

## Who does what

| | You | Claude |
|---|---|---|
| **Definitions** | Answer what only you know, then **Approve** | Draft them, ask, and keep the spec current |
| **Design** | **Sign off design-review**, and **Accept** or **Reject** each decision | Write the design, its decisions and its task list |
| **Code and tests** | Review what you want to | Build, or give the work to sub-agents, and record results |
| **Merging an item** | **Sign off docs-folded** | Update the docs and move the item |
| **Sprints** | Say what goes in and when it closes | Create, plan and close them |
| **Releases** | Each **Promote**, each **Sign off release**, and the **Cut** | Write the notes, run the checks, bring it forward |
| **Remote server** | **Every push** of code and tags, and the board | Nothing |

## When things go wrong

| You see | It means | Do this |
|---|---|---|
| Moves go through that shouldn't | No charter, or the item is older than it | Ask Claude to agree a charter before creating items |
| An item is back on the Review page | Its definition changed after you approved it | Read the change, then **Approve** again |
| An item says it's waiting for a sign-off, but the Review page has nothing | A sign-off appears once the stage's other requirements are met | Ask Claude what's left. It's often the docs |
| An item goes to the backlog when a sprint closes | It's merged but not done for that sprint | Ask Claude to plan it into the next sprint, or count Migration as done |
| A promotion says an item would carry over | It can't reach Operations yet | Do what the page says, usually **Sign off release**; the cut then moves it |
| An item stays in Migration after the cut | The release shipped but the item never got what it lacked | Ask Claude to run `kanbanr doctor`, which names it; give the sign-off it lacks, or plan it into a coming release |
| **Cut** stays greyed out after you pushed | The page hasn't read the push yet | **Check again** |
| The release is behind main, or only a greyed-out **Cut** is offered | A merge sent the release back to candidate | Read the panel on the Releases page and do its steps in order |
| Your remote server rejects your first push, naming a permission | Your sign-in can't change some files yet, often your CI's own files | Ask Claude what the message means, then grant it and push again |
| A defect in a shipped release | It needs its own fix | Ask Claude for a patch release, and see [Patch releases](../using/processes.md#patch-releases) |

## What Claude runs

You never need these, but here they are if you're curious, or if you want to check what Claude did.
[TOGAF from the command line](togaf-cli.md) covers them step by step.

| You asked for | Claude runs |
|---|---|
| Set up the process | `kanbanr config workflow --preset togaf` |
| Sprints, releases and branches | `kanbanr config cadence --sprints on --releases on --git-model on` |
| Release stages | `kanbanr config cadence --release-stages`, `kanbanr config release-stage` |
| A sprint | `kanbanr sprint add`, `kanbanr sprint start`, `kanbanr sprint plan`, `kanbanr sprint unplan` |
| A release | `kanbanr release add`, `kanbanr release plan` |
| Moving an item | `kanbanr move` |
| Starting the build | `kanbanr start` |
| A sub-agent | `kanbanr scope issue` |
| Release notes | `kanbanr release notes` |
| Bringing a release forward | `kanbanr git forward` |
| Closing a sprint | `kanbanr sprint close`, `kanbanr retro` |

To take an item out of the sprint, ask Claude to take it out; it goes back to no sprint.
