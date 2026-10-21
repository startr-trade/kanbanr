# Upgrading a board to schema 5

*For 0.2.0.*

kanbanr reads and writes board files with a new YAML library, and approvals, sign-offs and process
records are now pinned to their content in a way that does not depend on how a file is written. A
board written by 0.1.7 or earlier has to be migrated to this format once. Until then, every
command stops and tells you to run `kanbanr migrate`.

## What to do

1. Update kanbanr with `kanbanr self-update`, or rerun the installer on Windows.
2. Stop anything long-running on the board, such as `kanbanr serve`, so nothing writes while it
   migrates.
3. Check the board's approval state, so you can compare afterwards:

   ```bash
   kanbanr review --pending --json > before-review.json
   kanbanr check --json > before-check.json
   ```

4. Migrate:

   ```bash
   kanbanr migrate
   ```

   It prints what it did and makes one commit on the board, "migrate board to schema 5: canonical
   revisions, files rewritten". The board's push policy applies as it does to any write.
5. Run the two commands from step 3 again and compare the results. They report the same states.
   The `rev` fields hold new values, because the revisions are computed a new way.

Everyone who uses the board needs the new kanbanr. An older kanbanr refuses a migrated board and
names the version it needs. It does not misread the board.

## What the migration does

- **It re-pins every revision.** Approvals, sign-offs and process records each store a revision of
  the content they were given for. The migration reads the board's git history to find the
  content each one was given for, and records that content's new revision. So an approval that was
  current stays current, and one that had lapsed stays lapsed.
- **It rewrites every board file in the new format.** The content stays the same, including fields
  this version does not know. Mostly the quoting changes: double quotes where single quotes were
  used before. Files under a project's `docs/` folder are not touched. Each file is checked to
  read back as it was before anything is written.
- **It stamps every project with `schema_version: 5`.**

## When history cannot vouch for a revision

The migration finds each verdict in the commit that recorded it. That commit can be missing: the
board was never a git repository, it is a shallow clone, or its history was rewritten. Then the
migration cannot show which content the verdict was given for. It does not trust it:

- the revision becomes `unverified`, so the approval or sign-off reads as lapsed;
- `kanbanr migrate` says how many there are and lists the ones that matter now;
- `kanbanr doctor` names each one, for example "approval of FEAT-012 could not be verified during
  the schema-5 migration; approve again".

Give those approvals and sign-offs again. For a process record, apply the process again with
`kanbanr process update`.

## Going back

`git revert` of the migration commit restores the old files, but the new kanbanr will refuse them
until they are migrated again. To go back to an earlier kanbanr, revert the commit and reinstall
that version.
