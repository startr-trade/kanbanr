#!/usr/bin/env bash
# Builds the FEAT-187 v0.1.7 fixture board with the v0.1.7 binary, under /tmp/feat187. The board was then
# copied here with its .git stored as board/dot-git (a nested .git cannot be committed): HEAD, config,
# index, refs/heads/master and the repacked objects. Tests copy board/ to a temp dir and rename dot-git.
set -euo pipefail
K=/tmp/feat187/kanbanr-017
B=/tmp/feat187/fixture/board
DEFS=/tmp/feat187/defs
rm -rf /tmp/feat187/fixture "$DEFS"
mkdir -p "$B" "$DEFS" /tmp/feat187/work
cd /tmp/feat187/work
export KANBANR_PUSH=off
export KANBANR_PROCESSES_DIR=/tmp/feat187/personal-processes
unset KANBANR_SERVER_URL KANBANR_PROJECT KANBANR_DATA_DIR
k() { "$K" --data-dir "$B" "$@"; }
d() { k --project demo "$@"; }

k identity --name "Fixture Maker" --email fixture@example.com
k --project demo project init demo --description "FEAT-187 v0.1.7 fixture"
d milestone add --name "Fixture" --code MS-001 >/dev/null || d milestone add --name "Fixture" >/dev/null

def() { # def <file> <statement>
  cat > "$DEFS/$1" <<EOF
statement: "$2"
goals: []
zachman:
  what: The thing
  how: 'By doing it: carefully'
  where: crates/x
  when: Now
  who: Everyone
  why: Because
requirements:
  - id: R-1
    kind: functional
    text: THE SYSTEM SHALL do the thing.
    tests:
      - name: does_the_thing
        kind: unit
  - id: R-2
    kind: functional
    text: |-
      WHEN asked, THE SYSTEM SHALL answer
      on two lines.
    tests:
      - name: answers_when_asked
        kind: integration
EOF
}
add() { d feature add --title "$1" --milestone MS-001 --code "$2" >/dev/null; }
# 0.1.7's `feature define` replaces the sign-offs with the ones the file gives (none, by default):
# carry the item's sign-offs into the new definition, as an edit of the exported block would.
carry_signoffs() { # carry_signoffs <code> <def file>
  local f
  f=$(ls "$B"/projects/demo/features/*/"$1".yaml)
  awk '/^  signoffs:/{on=1; print "signoffs:"; next} on && /^  (- |  )/{print substr($0, 3); next} on{on=0}' "$f" >> "$DEFS/$2"
}

# 1 current
add "Approved and current" FEAT-001
def 1.yaml "Approved and current"; d feature define FEAT-001 --file "$DEFS/1.yaml" >/dev/null
d approve FEAT-001 >/dev/null
# 2 lapsed
add "Approved then edited" FEAT-002
def 2.yaml "Approved then edited"; d feature define FEAT-002 --file "$DEFS/2.yaml" >/dev/null
d approve FEAT-002 >/dev/null
def 2b.yaml "Approved then edited, and the edit lapsed it"; d feature define FEAT-002 --file "$DEFS/2b.yaml" >/dev/null
# 3 withdrawn
add "Approved then withdrawn" FEAT-003
def 3.yaml "Approved then withdrawn"; d feature define FEAT-003 --file "$DEFS/3.yaml" >/dev/null
d approve FEAT-003 >/dev/null
d unapprove FEAT-003 --reason "scope is wrong" >/dev/null
# 4 ratified
add "Started unapproved then ratified" FEAT-004
def 4.yaml "Started unapproved then ratified"; d feature define FEAT-004 --file "$DEFS/4.yaml" >/dev/null
d start FEAT-004 --no-branch --override "shipping under a deadline" >/dev/null
d ratify FEAT-004 --reason "agreed afterwards" >/dev/null
# 5 ratified then lapsed
add "Ratified then edited" FEAT-005
def 5.yaml "Ratified then edited"; d feature define FEAT-005 --file "$DEFS/5.yaml" >/dev/null
d start FEAT-005 --no-branch --override "hotfix" >/dev/null
d ratify FEAT-005 >/dev/null
def 5b.yaml "Ratified then edited, which lapsed it"; d feature define FEAT-005 --file "$DEFS/5b.yaml" >/dev/null
# 6 approved, edited, approved again
add "Approved again after an edit" FEAT-006
def 6.yaml "Approved again after an edit"; d feature define FEAT-006 --file "$DEFS/6.yaml" >/dev/null
d approve FEAT-006 >/dev/null
def 6b.yaml "Approved again after an edit, second wording"; d feature define FEAT-006 --file "$DEFS/6b.yaml" >/dev/null
d approve FEAT-006 >/dev/null
# 7 sign-off current
add "Signed off and current" FEAT-007
def 7.yaml "Signed off and current"; d feature define FEAT-007 --file "$DEFS/7.yaml" >/dev/null
d approve FEAT-007 >/dev/null
d signoff FEAT-007 design-review --note "held on Monday" >/dev/null
# 8 sign-off lapsed, approval current
add "Sign-off lapsed by an edit" FEAT-008
def 8.yaml "Sign-off lapsed by an edit"; d feature define FEAT-008 --file "$DEFS/8.yaml" >/dev/null
d signoff FEAT-008 design-review >/dev/null
def 8b.yaml "Sign-off lapsed by an edit, then approved"; carry_signoffs FEAT-008 8b.yaml; d feature define FEAT-008 --file "$DEFS/8b.yaml" >/dev/null
d approve FEAT-008 >/dev/null
# 9 defined, never approved
add "Defined, never approved" FEAT-009
def 9.yaml "Defined, never approved"; d feature define FEAT-009 --file "$DEFS/9.yaml" >/dev/null
# 10 tests go green after approval: approval stays current
add "Tests green after approval" FEAT-010
def 10.yaml "Tests green after approval"; d feature define FEAT-010 --file "$DEFS/10.yaml" >/dev/null
d approve FEAT-010 >/dev/null
d test FEAT-010 R-1 does_the_thing green --rev 0123456789abcdef0123456789abcdef01234567 >/dev/null
d test FEAT-010 R-2 answers_when_asked red --rev 0123456789abcdef0123456789abcdef01234567 >/dev/null
# 11 approved, withdrawn, approved again
add "Withdrawn then approved again" FEAT-011
def 11.yaml "Withdrawn then approved again"; d feature define FEAT-011 --file "$DEFS/11.yaml" >/dev/null
d approve FEAT-011 >/dev/null
d unapprove FEAT-011 --reason "not yet" >/dev/null
d approve FEAT-011 >/dev/null
# 12 no definition at all
add "No definition" FEAT-012
# 13 approved, then started: the file moves status folder
add "Approved then started" FEAT-013
def 13.yaml "Approved then started"; d feature define FEAT-013 --file "$DEFS/13.yaml" >/dev/null
d approve FEAT-013 >/dev/null
d start FEAT-013 --no-branch >/dev/null
d todo add FEAT-013 --title "Work" >/dev/null 2>&1 || true
# 14 two sign-offs, one lapsed by a later one? both current, then an edit lapses both; re-signed one
add "Two sign-offs" FEAT-014
def 14.yaml "Two sign-offs"; d feature define FEAT-014 --file "$DEFS/14.yaml" >/dev/null
d signoff FEAT-014 design-review >/dev/null
d signoff FEAT-014 release >/dev/null
def 14b.yaml "Two sign-offs, after an edit"; carry_signoffs FEAT-014 14b.yaml; d feature define FEAT-014 --file "$DEFS/14b.yaml" >/dev/null
d signoff FEAT-014 release >/dev/null
d approve FEAT-014 >/dev/null

# A key a newer kanbanr wrote, kept by this one (FEAT-151): hand-edited, committed by the next write.
F=$(ls "$B"/projects/demo/features/*/FEAT-012.yaml)
printf 'x_future:\n  note: kept by every reader\n  list:\n  - 1\n  - two\n' >> "$F"
d feature edit FEAT-012 --priority high >/dev/null

# Process provenance: a saved process applied to two projects, one of which is edited afterwards.
d process save team-flow --description "The fixture's process" >/dev/null
k --project flow project init flow --description "uses team-flow" >/dev/null
k --project flow config workflow --preset team-flow >/dev/null
k --project drifted project init drifted --description "uses team-flow, then edited" >/dev/null
k --project drifted config workflow --preset team-flow >/dev/null
k --project drifted config default-state Deferred >/dev/null

# A project whose saved process has moved on since it was applied.
d process save other-flow --description "A process that moves on" >/dev/null
k --project behind project init behind --description "uses other-flow v1" >/dev/null
k --project behind config workflow --preset other-flow >/dev/null
k --project drifted process save other-flow >/dev/null

echo done
