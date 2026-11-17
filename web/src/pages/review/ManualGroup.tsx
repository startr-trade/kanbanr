/**
 * The Review page's Manual group, grouped by item (FTR-057). Inside FTR-038's group panel, whose
 * header keeps the selection, the filter, the sort and the one ask-first action:
 *
 * - a section per release being shipped ("Holding v0.8.0 · 2 items, 3 checks"), in the order the
 *   server names them (newest first); a holding release with nothing due draws no section;
 * - each item is FTR-046's `AccordionRow` (an `h3`, so DEF-082's card heading) reading "<code>
 *   <title> · <status> · <release> · N checks", with a "holds Promote v0.8.0 to main" pill when one
 *   of its checks holds that promotion; its checks beneath it, each a `ReviewRow` with the row's
 *   checkbox and its evidence, the check that holds the promotion carrying the same pill;
 * - an item with two or more prepared results offers "Confirm N prepared for <code>" on its
 *   heading: the first press selects exactly those rows and asks, the second records them as the
 *   group's one decision (FTR-038, A-P1: every group action asks first);
 * - "Other items (N)" after the holding sections, collapsed while one is drawn above it, and
 *   "Not due yet (N)" (DEF-076), collapsed, last.
 *
 * Which release holds and which check holds its promotion is derived by core (`manual::holding`,
 * `ManualWaiting.holds`, ADR-0002); this only draws it.
 */
import { useMemo, useState, type ReactNode } from "react";
import { AskFirst } from "../../components/AskFirst";
import { askText, type Selection } from "../../reviewGroups";
import { itemPrepared, joinManual, manualGroups, manualRowKey, manualRows, type ItemGroup, type ManualRow } from "../../manualTests";
import { AccordionRow, Section, SeverityPill } from "../../ui";
import { useQuery } from "../../data/useQuery";
import { TAGS, urls } from "../../data/urls";
import type { GroupSlice, ManualWaiting, PreparedView, ReviewRow as Row } from "../../types";
import { ReviewRow } from "./ReviewRow";

/** "1 check", "3 checks". */
const many = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

export interface ManualGroupProps {
  project: string;
  slice: GroupSlice;
  /** The slice's rows the header's filter and sort leave, in that order. */
  rows: Row[];
  /** The waiting checks with their prepared records (`manualRows`); null while being read. */
  manual: ManualRow[] | null;
  writable: boolean;
  sel: Selection;
  /** Tick or untick one row, as the group's own checkbox does. */
  onSelect(key: string, on: boolean): void;
  /** Make `sel` the group's selection (an item's heading action, first press). */
  onSelectItem(sel: Selection): void;
  /** Record `sel` as the group's decision (an item's heading action, second press). */
  onConfirmItem(sel: Selection): void;
  /** Why a row cannot be selected; "" when it can. */
  reasonFor(row: Row): string;
  openRows: ReadonlySet<string>;
  onOpen(key: string, open: boolean): void;
  inFlight: boolean;
  body(row: Row): ReactNode;
}

/** The Manual group, its waiting checks read live beside the slice. */
export function ManualGroupLive(p: Omit<ManualGroupProps, "manual">): JSX.Element {
  // Both reads go through the shared cache (DEF-106): the page's own prepared read is the same URL.
  const waiting = useQuery<ManualWaiting[]>(p.project, urls.manualTests(p.project), TAGS.manualTests);
  const prepared = useQuery<PreparedView[]>(p.project, urls.prepared(p.project), TAGS.prepared);
  const manual = useMemo(
    () => (waiting.data && prepared.data ? manualRows(waiting.data, prepared.data) : null),
    [waiting.data, prepared.data],
  );
  return <ManualGroup {...p} manual={manual} />;
}

export function ManualGroup(p: ManualGroupProps): JSX.Element {
  const sections = manualGroups(joinManual(p.rows, p.manual), p.slice.holding ?? []);
  const holding = sections.holding.filter((h) => h.items.length > 0);
  const byKey = new Map(p.rows.map((r) => [r.key, r]));
  // Items are open unless the person closed one; "Other items" is open until it follows a
  // holding section, then collapsed unless the person opened it.
  const [closed, setClosed] = useState<Set<string>>(() => new Set());
  const [otherOpen, setOtherOpen] = useState<boolean | null>(null);
  const toggleItem = (code: string, open: boolean) =>
    setClosed((c) => {
      const next = new Set(c);
      if (open) next.delete(code);
      else next.add(code);
      return next;
    });

  const row = (m: ManualRow, level: 3 | 4) => {
    const r = byKey.get(manualRowKey(m.row));
    if (!r) return null;
    const why = p.reasonFor(r);
    return (
      <ReviewRow
        key={r.key}
        project={p.project}
        row={r}
        level={level}
        badge={m.row.holds ? <HoldsPill holds={m.row.holds} /> : undefined}
        open={p.openRows.has(r.key)}
        onToggle={(o) => p.onOpen(r.key, o)}
        select={p.writable ? { checked: p.sel.keys.has(r.key), disabled: why || undefined, onChange: (v) => p.onSelect(r.key, v) } : undefined}
      >
        {() => p.body(r)}
      </ReviewRow>
    );
  };

  const item = (g: ItemGroup) => {
    const holds = g.rows.find((m) => m.row.holds)?.row.holds ?? "";
    const pick = itemPrepared(p.slice, g.code);
    const heading = [`${g.code} ${g.title}`, g.status, g.release, many(g.rows.length, "check")].filter((s) => s !== "").join(" · ");
    return (
      <div className="manual-item-wrap" key={g.code}>
        <AccordionRow
          id={`manual-item-${g.code}`}
          level={3}
          open={!closed.has(g.code)}
          onToggle={(o) => toggleItem(g.code, o)}
          title={<span className="manual-item-title">{heading}</span>}
          meta={holds ? <HoldsPill holds={holds} /> : undefined}
          actions={
            p.writable && pick.keys.size >= 2 ? (
              // The first press makes the selection and asks; the second records it.
              <span onClickCapture={() => p.onSelectItem(pick)}>
                <AskFirst
                  label={`Confirm ${pick.keys.size} prepared for ${g.code}`}
                  ask={askText(p.slice, pick, p.slice.rows)}
                  className="btn btn-primary"
                  disabled={p.inFlight}
                  disabledWhy="Recording…"
                  onConfirm={() => p.onConfirmItem(pick)}
                />
              </span>
            ) : undefined
          }
        >
          {() => <div className="manual-item-rows">{g.rows.map((m) => row(m, 4))}</div>}
        </AccordionRow>
      </div>
    );
  };

  const otherShown = otherOpen ?? holding.length === 0;
  return (
    <div className="manual-groups">
      {holding.map((h) => (
        <Section
          key={h.version}
          id={`manual-holding-${h.version}`}
          title={`Holding ${h.version} · ${many(h.items.length, "item")}, ${many(
            h.items.reduce((n, g) => n + g.rows.length, 0),
            "check",
          )}`}
        >
          <div className="manual-section-items">{h.items.map(item)}</div>
        </Section>
      ))}
      {sections.other.length > 0 ? (
        <div className="manual-section">
          <AccordionRow
            id="manual-other"
            level={2}
            open={otherShown}
            onToggle={setOtherOpen}
            title={`Other items (${sections.other.length})`}
          >
            {() => <div className="manual-section-items">{sections.other.map(item)}</div>}
          </AccordionRow>
        </div>
      ) : null}
      {sections.notDue.length > 0 ? (
        <div className="manual-section">
          <AccordionRow id="manual-not-due" level={2} title={`Not due yet (${sections.notDue.length})`}>
            {() => <div className="manual-section-items">{sections.notDue.map((m) => row(m, 3))}</div>}
          </AccordionRow>
        </div>
      ) : null}
    </div>
  );
}

/** The pill on a check that holds a promotion, and on its item: "holds Promote v0.8.0 to main". */
function HoldsPill({ holds }: { holds: string }): JSX.Element {
  return <SeverityPill tone="warn">holds {holds}</SeverityPill>;
}
