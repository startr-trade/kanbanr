# Designing CI and the release workflow

Design your pipeline before your first release, not during it. A release workflow can only be tested
by running it, a tag can't be moved once it's pushed, and shipped release notes can't be edited. So a
mistake in the pipeline costs a whole version number, and a first release often takes three or four
tries. A few checks beforehand avoid most of them.

This page applies to every process that has releases: [TOGAF](togaf-with-claude.md),
[Scrum](scrum-with-claude.md) and [Agile](agile-with-claude.md). The words below don't depend on
which remote server, CI service or package registry you use.

## Ask Claude to design it with you

> Design the CI and the release workflow for this project, before the first release. Propose
> separate stages I can rerun one at a time, with caching for our stack, on one platform to start.
> Tell me which checks our plan can't run. Use the release notes kanbanr wrote as the release
> description. Add a manual run that builds everything and publishes nothing.

Claude proposes the pipeline and brings you the choices. You approve it like any other design, and
it becomes an item on the board, so it has a definition, a test and an approval of its own.

## What to settle before the first tag

1. **Start small, in stages.** Keep the first pipeline small by covering one platform and a few
   checks, not by putting everything in one job. Then:
   - **Each stage is its own job**: lint (format and lint), test, build, then scan and publish in
     the release workflow. A failed stage can be rerun alone, and the jobs after it wait for it.
   - **Build once and pass it on.** A stage hands what it built to the next as an artifact. The
     publish stage publishes the files the build stage made and never rebuilds them.
   - **Cache by lock file.** The dependency and build caches are keyed on the lock files (and the
     toolchain file), and saved only from main. A pull request reads main's cache and never writes
     its own.
   - **Set a timeout on every job**, so a hung test fails within the hour instead of holding a
     runner.
   - **Cancel a run a newer push made pointless**, except a release run.
   - **Add more platforms, container images and scanners** after the first release, one at a time,
     each as its own item and its own stage.

   A pipeline that is green and small is worth more than a complete one that has never run.

   **Speed-ups by stack**, which Claude offers for the stacks the project uses:

   | Stack | Cache keyed on | Speed-ups |
   |---|---|---|
   | Rust | `Cargo.lock`, `rust-toolchain.toml` | a cache of the registry and `target/`; sccache for compiled crates; cargo-chef for an image's dependency layer; no incremental builds or debug info in CI; cargo-nextest to run tests side by side |
   | Python | the lock or requirements files | the pip cache (or the installer's own) |
   | Node | `package-lock.json` (or the lock file in use) | the npm cache, and `npm ci` |
   | Container images | the Dockerfile and the lock files | a layer cache for the image build, one per image |

   Which rows apply comes from the stack in the design, and from the project's baseline once
   [FTR-033](/p/kanbanr/feature/FTR-033) is built.
2. **Check what your plan allows.** Some checks only run on certain plans or settings. A code
   scanner may be unavailable for a private project, and a package registry may need a setting on
   your account before it accepts a push. Claude can't see your account settings, so it asks you,
   and you look. Leave out any check you can't run, and add it back when you can.
3. **Take the release description from kanbanr's notes.** Promoting a release to its first stage
   drafts the notes, and you and Claude rewrite them for readers. Tell Claude to make the pipeline
   publish those notes as the description. A pipeline that generates its own description from
   commits will ignore everything you wrote.
4. **Run the release workflow without a tag.** Ask for a manual trigger that runs every step
   up to publishing and publishes nothing. Run it on main before you cut the release, and again
   whenever the workflow changes. It finds the mistakes that checking the file by eye cannot,
   such as a step that downloads the wrong files.
5. **Run each check once per commit.** If both a push to main and a push of the tag start the same
   checks, the second run is wasted time. Decide which runs where.
6. **Run the product the way a person will.** Test the built result from another machine or
   address, not only from the one that built it. A container that calls its own machine by name
   passes every check on the machine that made it and fails everywhere else.
7. **Record CI before you cut.** kanbanr can see that main was pushed. It can't see whether CI
   passed. The Cut waits until a CI result is recorded for main's commit, on the Releases page.
   Claude reads the run and prepares the result with its link; you check it and click **Record ci
   passed**.
8. **Mark the checks that need a push.** A requirement that only CI can show (the job runs, a tag
   publishes the notes) gets `at: pushed` on its test, or `at: cut`. The item's gate doesn't wait for
   it, so Promote to main isn't held; Claude records the result once CI has run. Anything still not
   run at the cut waits for you to accept shipping it unverified, and the notes name it.

## Where it fits

| When | What |
|---|---|
| During setup or the first sprint | Ask Claude to design the pipeline, and approve it |
| Before **Promote to main** | Run the release workflow without a tag, and fix what it shows |
| After you push main | Wait for CI to go green, check the result Claude prepared and click **Record ci passed**, then **Check again** and **Cut** |
| After you push the tag | Wait for the release workflow, and check the release page has your notes |

If the pipeline needs a fix after the cut, that is a patch release. See
[Patch releases](../using/processes.md#patch-releases).
