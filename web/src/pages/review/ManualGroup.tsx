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
 * - the group starts with one line on what a manual check is (DEF-113);
 * - after the holding sections, three collapsed sections by where the item stands: "Shipped without
 *   a judgement" (its release has shipped; the person may retire these, several at once with one
 *   reason), "Later releases" and "Not planned"; an empty one is not drawn, and "Later releases"
 *   opens when nothing is holding. A project with no release lists its items with no section.
 *   "Not due yet (N)" (DEF-076), collapsed, is last.
 *
 * Which release holds and which check holds its promotion is derived by core (`manual::holding`,
 * `ManualWaiting.holds`, ADR-0002); this only draws it.
 */
import { useMemo, useState, type ReactNode } from "react";
import { AskFirst } from "../../components/AskFirst";
import { askText, type Selection } from "../../reviewGroups";
import {
  MANUAL_PURPOSE,
  itemPrepared,
  joinManual,
  manualGroups,
  manualRowKey,
  manualRows,
  retirable,
  type ItemGroup,
  type ManualRow,
} from "../../manualTests";
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
  /** Retire `sel`, the shipped section's own selection, with one reason (DEF-113). */
  onRetire(sel: Selection, reason: string): void;
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
  const sections = manualGroups(joinManual(p.rows, p.manual), p.slice.holding ?? [], p.slice.shipped ?? []);
  const holding = sections.holding.filter((h) => h.items.length > 0);
  const byKey = new Map(p.rows.map((r) => [r.key, r]));
  // Items are open unless the person closed one; the sections after the holding ones are collapsed
  // unless the person opened one (DEF-113).
  const [closed, setClosed] = useState<Set<string>>(() => new Set());
  const [opened, setOpened] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  // The shipped section's own selection: retiring is not the group's Confirm.
  const [retiring, setRetiring] = useState<ReadonlySet<string>>(() => new Set());
  const pick = (key: string, on: boolean) =>
    setRetiring((c) => {
      const next = new Set(c);
      if (on) next.add(key);
      else next.delete(key);
      return next;
    });
  const toggleItem = (code: string, open: boolean) =>
    setClosed((c) => {
      const next = new Set(c);
      if (open) next.delete(code);
      else next.add(code);
      return next;
    });

  const row = (m: ManualRow, level: 3 | 4, local = false) => {
    const r = byKey.get(manualRowKey(m.row));
    if (!r) return null;
    const why = p.reasonFor(r);
    // In the shipped section a check with no prepared result is ticked into the section's own
    // selection, to be retired, even though the group's Confirm refuses it (`why`); one with a
    // prepared result keeps the group's checkbox.
    const own = local && retirable(m);
    const select = own
      ? { checked: retiring.has(r.key), onChange: (v: boolean) => pick(r.key, v) }
      : { checked: p.sel.keys.has(r.key), disabled: why || undefined, onChange: (v: boolean) => p.onSelect(r.key, v) };
    return (
      <ReviewRow
        key={r.key}
        project={p.project}
        row={r}
        level={level}
        badge={m.row.holds ? <HoldsPill holds={m.row.holds} /> : undefined}
        open={p.openRows.has(r.key)}
        onToggle={(o) => p.onOpen(r.key, o)}
        select={p.writable ? select : undefined}
      >
        {() => p.body(r)}
      </ReviewRow>
    );
  };

  const item = (g: ItemGroup, local = false) => {
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
          {() => <div className="manual-item-rows">{g.rows.map((m) => row(m, 4, local))}</div>}
        </AccordionRow>
      </div>
    );
  };

  const counts = (items: ItemGroup[]) =>
    `${many(items.length, "item")}, ${many(
      items.reduce((n, g) => n + g.rows.length, 0),
      "check",
    )}`;
  // A project with no release at all has none to name: its items are listed as they are.
  const bare = holding.length === 0 && sections.shipped.length === 0 && sections.later.length === 0;
  const section = (id: string, title: string, items: ItemGroup[], open: boolean, actions?: ReactNode, local = false) =>
    items.length > 0 ? (
      <div className="manual-section" key={id}>
        <AccordionRow
          id={id}
          level={2}
          open={opened.get(id) ?? open}
          onToggle={(o) => setOpened((m) => new Map(m).set(id, o))}
          title={`${title} · ${counts(items)}`}
          actions={actions}
        >
          {() => <div className="manual-section-items">{items.map((g) => item(g, local))}</div>}
        </AccordionRow>
      </div>
    ) : null;
  const shippedKeys = sections.shipped.flatMap((g) => g.rows.filter(retirable).map((m) => manualRowKey(m.row)));
  const retireSel: Selection = { group: p.slice.key, keys: new Set([...retiring].filter((k) => shippedKeys.includes(k))) };
  return (
    <div className="manual-groups">
      <p className="small muted manual-purpose">{MANUAL_PURPOSE}</p>
      {holding.map((h) => (
        <Section key={h.version} id={`manual-holding-${h.version}`} title={`Holding ${h.version} · ${counts(h.items)}`}>
          <div className="manual-section-items">{h.items.map((g) => item(g))}</div>
        </Section>
      ))}
      {bare ? (
        sections.unplanned.length > 0 ? (
          <div className="manual-section-items">{sections.unplanned.map((g) => item(g))}</div>
        ) : null
      ) : (
        <>
          {section(
            "manual-shipped",
            "Shipped without a judgement",
            sections.shipped,
            false,
            p.writable && shippedKeys.length > 0 ? (
              <span className="manual-retire">
                <button className="btn" onClick={() => setRetiring(new Set(shippedKeys))}>
                  Select all ({shippedKeys.length})
                </button>{" "}
                <RetireAction
                  slice={p.slice}
                  rows={p.slice.rows}
                  keys={retireSel.keys}
                  inFlight={p.inFlight}
                  onRetire={(reason) => {
                    p.onRetire(retireSel, reason);
                    setRetiring(new Set());
                  }}
                />
              </span>
            ) : undefined,
            true,
          )}
          {section("manual-later", "Later releases", sections.later, holding.length === 0)}
          {section("manual-unplanned", "Not planned", sections.unplanned, false)}
        </>
      )}
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

/**
 * "Retire N" (DEF-113): the first press asks, naming the checks, and shows a required reason; the
 * second press, enabled once the reason is not blank, retires them all with that one reason.
 */
function RetireAction(p: {
  slice: GroupSlice;
  rows: Row[];
  keys: ReadonlySet<string>;
  inFlight: boolean;
  onRetire(reason: string): void;
}): JSX.Element {
  const [asking, setAsking] = useState(false);
  const [reason, setReason] = useState("");
  const n = p.keys.size;
  const label = `Retire ${n}`;
  const sel: Selection = { group: p.slice.key, keys: new Set(p.keys) };
  // Changing the selection under an open question drops it, so a press never confirms another list.
  const [asked, setAsked] = useState("");
  const now = [...p.keys].join("\n");
  if (asking && asked !== now) {
    setAsking(false);
  }
  if (!asking) {
    return (
      <button
        className={n > 0 ? "btn btn-primary" : "btn"}
        disabled={n === 0 || p.inFlight}
        title={n === 0 ? "Tick the checks to retire" : p.inFlight ? "Recording…" : undefined}
        onClick={() => {
          setAsked(now);
          setAsking(true);
        }}
      >
        {label}
      </button>
    );
  }
  const question = `${askText({ ...p.slice, action: "Retire" }, sel, p.rows)} They stay not proven; the reason is kept with each.`;
  return (
    <span className="ask-first retire-ask" role="group" aria-label={question}>
      <span className="ask-first-question">{question}</span>{" "}
      <input value={reason} onChange={(e) => setReason(e.target.value)} maxLength={4000} aria-label="Reason" placeholder="Why" />{" "}
      <button
        className="btn btn-primary"
        disabled={reason.trim() === "" || p.inFlight}
        title={reason.trim() === "" ? "A retirement keeps its reason: give one" : undefined}
        onClick={() => {
          const r = reason.trim();
          setAsking(false);
          setReason("");
          p.onRetire(r);
        }}
      >
        {label}
      </button>{" "}
      <button className="btn" onClick={() => setAsking(false)}>
        Cancel
      </button>
    </span>
  );
}

/** The pill on a check that holds a promotion, and on its item: "holds Promote v0.8.0 to main". */
function HoldsPill({ holds }: { holds: string }): JSX.Element {
  return <SeverityPill tone="warn">holds {holds}</SeverityPill>;
}
