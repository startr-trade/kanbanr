/**
 * Decisions here (FTR-028), above the Review page's groups (FTR-038): where this project's
 * decisions are taken, with the mode to change it, and the delegations the person granted, each
 * with what it covers and its Revoke. A proposed delegation and a decision Claude took under one
 * wait in their own groups (`delegation` and `delegated`), each row's body a `DelegationEvidence`.
 */
import { useState } from "react";
import { api } from "../../api";
import { verdictNoun } from "../../needsYouState";
import type { DecisionsView, Waiting } from "../../types";

/** The verdict a waiting line asks ("sign off design-review" is `signoff:design-review`), or null. */
function verdictOf(w: Waiting): string | null {
  if (w.what.startsWith("sign off ")) return `signoff:${w.what.slice("sign off ".length)}`;
  if (w.what.startsWith("approve")) return "approve";
  if (w.what.startsWith("ratify")) return "ratify";
  return null;
}

/** "Holds now: 4 design reviews, 2 approvals" for one delegation, from what waits for Claude under it. */
function holdsNow(waiting: Waiting[], id: string): string {
  const counts = new Map<string, number>();
  for (const w of waiting) {
    if (w.place !== "delegated" || w.delegation !== id) continue;
    const v = verdictOf(w);
    if (v) counts.set(v, (counts.get(v) ?? 0) + 1);
  }
  if (counts.size === 0) return "Holds nothing now.";
  return `Holds now: ${[...counts].map(([v, n]) => `${n} ${verdictNoun(v, n)}`).join(", ")}`;
}

/** Where a waiting decision is taken, in words. */
function placeWords(w: Waiting): string {
  if (w.place === "delegated") return `for the agent, under ${w.delegation ?? "a delegation"}`;
  if (w.place === "conversation") return "in the conversation";
  return "here, on the monitor";
}

/**
 * Decisions here (FTR-028): where this project's decisions are taken, and the delegations the
 * person granted. What Claude proposed, and what it decided under a delegation, wait in their
 * groups.
 */
export function DecisionsHere({
  project,
  view,
  error,
  writable,
  who,
}: {
  project: string;
  view: DecisionsView | null;
  error: unknown;
  writable: boolean;
  who: string | null;
}) {
  const [mode, setMode] = useState<string>("");
  const [said, setSaid] = useState<Record<string, string>>({});
  const [waiting, setWaiting] = useState<Waiting[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [reasons, setReasons] = useState<Record<string, string>>({});
  if (error) {
    return (
      <section className="decisions-here">
        <h2>Decisions here</h2>
        <p className="error">Where decisions are taken could not be read: {String(error)}</p>
      </section>
    );
  }
  if (!view) return null;
  const run = async (key: string, f: () => Promise<unknown>, ok: string) => {
    setBusy(key);
    try {
      await f();
      setSaid((d) => ({ ...d, [key]: ok }));
    } catch (e) {
      setSaid((d) => ({ ...d, [key]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
    }
  };
  const by = who ?? "";
  const chosen = mode || view.mode;
  const proposed = view.delegations.filter((g) => g.state === "proposed");
  const granted = view.delegations.filter((g) => g.state === "granted");
  const reason = (k: string) => reasons[k] ?? "";
  const setReason = (k: string, v: string) => setReasons((r) => ({ ...r, [k]: v }));
  return (
    <section className="decisions-here">
      <h2>Decisions here</h2>
      <p className="muted">
        Decisions are taken in <strong>{view.mode}</strong> mode.{" "}
        {view.mode === "delegated"
          ? "You decide here; the agent may take a verdict a delegation you granted covers, and it comes back in the Delegated decisions group for you to keep or undo."
          : view.mode === "conversation"
            ? "Decisions are taken in the conversation in this project."
            : "Every verdict is yours, taken here."}
      </p>
      {writable ? (
        <p>
          <label>
            Mode{" "}
            <select value={chosen} onChange={(e) => setMode(e.target.value)} aria-label="Decision mode">
              <option value="monitor">monitor</option>
              <option value="delegated">delegated</option>
              <option value="conversation">conversation</option>
            </select>
          </label>{" "}
          <button
            disabled={busy === "mode" || chosen === view.mode}
            onClick={() =>
              run(
                "mode",
                async () => {
                  const answer = await api.setDecisionMode(project, chosen, by);
                  setWaiting(answer.waiting);
                },
                `now ${chosen}`,
              )
            }
          >
            Change
          </button>{" "}
          {said.mode ? <span className="small">{said.mode}</span> : null}
        </p>
      ) : (
        <p className="muted small">
          Change it with <code>kanbanr decisions mode &lt;monitor|delegated|conversation&gt;</code> in your own terminal.
        </p>
      )}
      {waiting && waiting.length > 0 ? (
        <ul className="small">
          {waiting.map((w, i) => (
            <li key={`${w.code}-${i}`}>
              <strong>{w.code}</strong> {w.what}: {placeWords(w)}
            </li>
          ))}
        </ul>
      ) : null}

      <h3>Delegations</h3>
      {granted.length === 0 ? (
        <p className="muted small">
          None in force.
          {proposed.length > 0 ? ` ${proposed.length} proposed: see the Delegations group.` : ""}
        </p>
      ) : null}
      {granted.map((g) => (
        <div key={g.id} id={g.id} className="small">
          <p>
            <strong>{g.id}</strong> granted: {g.verdicts.join(", ")}
            {g.kinds?.length ? ` · ${g.kinds.join(", ")}` : ""}
            {g.items?.length ? ` · ${g.items.join(", ")}` : ""}
            {g.milestone ? ` · ${g.milestone}` : ""}
            {g.all ? " · every item" : ""}
            {g.steps?.length ? ` · at ${g.steps.join(", ")}` : ""} · until {g.until.slice(0, 10)}
            {g.not_granted_through_kanbanr ? (
              <>
                {" "}
                · <strong>not granted through kanbanr</strong>: nothing is taken under it
              </>
            ) : null}
            {g.grant_unverifiable ? (
              <>
                {" "}
                · <strong>can't be checked</strong>: {g.grant_unverifiable}; nothing is taken under it until it can
              </>
            ) : null}
            {view.mode === "delegated" && g.permission_rules_checked ? (
              g.permission_rules_missing?.length ? (
                <>
                  {" "}
                  · auto mode: add <code>{g.permission_rules_missing.join(", ")}</code> to{" "}
                  <code>.claude/settings.local.json</code> (ask your agent, or run{" "}
                  <code>kanbanr claude permissions --write</code> in your own terminal)
                </>
              ) : (
                " · auto mode: allowed"
              )
            ) : null}
          </p>
          <p className="muted">{holdsNow(view.waiting, g.id)}</p>
          {said[g.id] ? (
            <p>{said[g.id]}</p>
          ) : writable ? (
            <p>
              <input
                aria-label={`Why revoke ${g.id}`}
                placeholder="why"
                value={reason(g.id)}
                onChange={(e) => setReason(g.id, e.target.value)}
              />{" "}
              <button
                disabled={busy === g.id || !reason(g.id).trim()}
                onClick={() => run(g.id, () => api.revokeDelegation(project, g.id, by, reason(g.id)), "revoked")}
              >
                Revoke
              </button>
            </p>
          ) : (
            <p className="muted">
              Revoke with <code>kanbanr decisions revoke {g.id} --reason "…"</code>.
            </p>
          )}
        </div>
      ))}
    </section>
  );
}
