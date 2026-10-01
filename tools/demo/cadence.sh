#!/usr/bin/env bash
# Build a throwaway Scrum project with a running sprint, a shipped release and a planned one.
#
#   tools/demo/cadence.sh [DATA_DIR]      # default: a temp dir, printed at the end
#
# Why this exists: kanbanr's own board does not use sprints or releases, so nothing in this
# repository shows the sprint header, its burndown, the Releases page or a Gantt with sprints. This
# builds a small, honest example with the `scrum` preset — every item defined, approved and moved
# through its gates by the CLI, exactly as a user would — and is the fixture those screenshots are
# taken against (FEAT-133).
#
# One thing a user would not do: the arrival dates. A burndown is read off the dates items reached
# Done, and a script runs in a second, so after the moves the demo spreads those dates over the
# sprint's first days. It touches only the demo's own folder, never your board.
set -euo pipefail

KANBANR="${KANBANR:-kanbanr}"
DATA_DIR="${1:-$(mktemp -d "${TMPDIR:-/tmp}/kanbanr-demo.XXXXXX")/board}"
mkdir -p "$DATA_DIR"
export KANBANR_DATA_DIR="$DATA_DIR"
P=shop
k() { "$KANBANR" --data-dir "$DATA_DIR" --project "$P" "$@"; }
day() { date -u -d "$1 days" +%F; }   # GNU date: day -6 is six days ago

echo "building a Scrum demo in $DATA_DIR"
"$KANBANR" --data-dir "$DATA_DIR" identity --name "Demo Maintainer" --email demo@example.com >/dev/null
"$KANBANR" --data-dir "$DATA_DIR" project init "$P" --workflow scrum \
    --description "demo: a small web shop, run in two-week sprints" >/dev/null
k config cadence --sprints on --releases on --unit points --sprint-length 10 >/dev/null
k milestone add --name "Checkout" --code MS-001 >/dev/null

charter=$(mktemp)
cat > "$charter" <<'YAML'
purpose: Let a customer buy what is in their basket without help.
goals:
  - id: G-1
    statement: A customer can pay for a basket on their own
  - id: G-2
    statement: Every order can be traced to what was promised
YAML
k charter set --file "$charter" >/dev/null

# title | points | requirement
items=(
  "Price a basket|3|WHEN a basket changes, THE SYSTEM SHALL show its total including tax."
  "Apply a discount code|2|WHEN a valid code is entered, THE SYSTEM SHALL reduce the total by its amount."
  "Take a card payment|8|WHEN the customer confirms, THE SYSTEM SHALL charge the card for the total."
  "Email a receipt|3|WHEN a payment succeeds, THE SYSTEM SHALL email the customer a receipt."
  "Refund a payment|5|WHEN a refund is approved, THE SYSTEM SHALL return the amount to the card."
  "Show order history|5|WHEN a customer signs in, THE SYSTEM SHALL list their past orders."
)
n=0
for line in "${items[@]}"; do
    IFS='|' read -r title points req <<<"$line"
    n=$((n + 1)); code=$(printf 'FEAT-%03d' "$n")
    k feature add --title "$title" --milestone MS-001 --spec "# $title" --points "$points" >/dev/null
    test_name="checkout::tests::$(echo "$title" | tr 'A-Z ' 'a-z_')"
    def=$(mktemp)
    cat > "$def" <<YAML
statement: "$title, for a customer at checkout, so that they can finish buying on their own"
goals: [G-1]
zachman:
  what: "$title"
  how: "A checkout step in the web shop"
  where: "The shop's checkout service"
  when: "During checkout"
  who: "A customer"
  why: "Checkout cannot finish without it"
requirements:
  - kind: functional
    text: "$req"
    tests:
      - name: "$test_name"
        kind: unit
        state: planned
YAML
    k feature define "$code" --file "$def" >/dev/null
    k approve "$code" >/dev/null
    k move "$code" Ready >/dev/null
done

# Sprint 1: started six days ago, ten days long, everything in it.
k sprint add --start "$(day -6)" --length 10d --goal "Customers can pay, and get a receipt" --capacity 30 >/dev/null
k sprint plan SP-001 FEAT-001 FEAT-002 FEAT-003 FEAT-004 FEAT-005 FEAT-006 >/dev/null
k sprint start SP-001 >/dev/null

# Releases: v0.1.0 ships the first two, v0.2.0 is planned for the rest.
k release add v0.1.0 --target "$(day 0)" --name "Basket" >/dev/null
k release add v0.2.0 --target "$(day 14)" --name "Payments" >/dev/null
k release plan v0.1.0 FEAT-001 FEAT-002 >/dev/null
k release plan v0.2.0 FEAT-003 FEAT-004 FEAT-005 FEAT-006 >/dev/null

walk() { # walk <code> <stages...>: move through each stage, turning the test green before Done
    local code=$1; shift
    for stage in "$@"; do
        if [ "$stage" = Done ]; then
            local t
            t=$(k feature show "$code" | grep -oE 'checkout::tests::[a-z_]+' | head -1)
            k test "$code" R-1 "$t" green >/dev/null
        fi
        k move "$code" "$stage" >/dev/null
    done
}
walk FEAT-001 "In Progress" Review Testing Done
walk FEAT-002 "In Progress" Review Testing Done
walk FEAT-004 "In Progress" Review Testing Done
walk FEAT-003 "In Progress" Review Testing
walk FEAT-005 "In Progress"
k release cut v0.1.0 >/dev/null

# Spread the arrivals over the sprint, so the burndown has the shape a real one would.
python3 - "$DATA_DIR/projects/$P/features" "$(day -5)" "$(day -3)" "$(day -1)" <<'PY'
import pathlib, sys, yaml
root, d1, d2, d3 = pathlib.Path(sys.argv[1]), *sys.argv[2:]
when = {"FEAT-001": d1, "FEAT-002": d2, "FEAT-004": d3}
for path in root.rglob("FEAT-*.yaml"):
    meta = yaml.safe_load(path.read_text())
    day = when.get(meta.get("code"))
    if not day:
        continue
    for t in meta.get("history") or []:
        if t.get("to") in ("Done", "Released"):
            t["at"] = f"{day}T15:00:00Z"
    path.write_text(yaml.safe_dump(meta, sort_keys=False, allow_unicode=True))
PY

echo
k sprint show SP-001
echo
echo "  kanbanr serve --data-dir $DATA_DIR --bind 127.0.0.1:8082   # then open /p/$P"
