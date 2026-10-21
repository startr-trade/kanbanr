/**
 * What the monitor says about agent scopes (FEAT-214), as pure functions from what the daemon
 * answers to the text the pages draw. Kept apart from the pages so `npm run check:agent-scopes` can
 * assert them without a browser. The monitor only shows scopes: issuing, claiming and revoking are
 * the CLI's (`kanbanr scope …`).
 */
import type { Activity, AgentScope } from "./types";

/** "3h 20m", "45m", "under a minute". */
export function duration(ms: number): string {
  const minutes = Math.floor(Math.abs(ms) / 60000);
  if (minutes < 1) return "under a minute";
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h > 0 ? (m > 0 ? `${h}h ${m}m` : `${h}h`) : `${m}m`;
}

/** `2026-10-21 10:00 UTC`. */
export function when(at: string | null | undefined): string {
  if (!at) return "";
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return at;
  return `${d.toISOString().slice(0, 10)} ${d.toISOString().slice(11, 16)} UTC`;
}

/** One line of a scope history. */
export interface ScopeRow {
  id: string;
  item: string;
  role: string;
  state: AgentScope["state"];
  /** What the scope covers: its branch for a build agent, "reads only" for a design one. */
  covers: string;
  issued: string;
  /** Expiry, time left, or how it ended. */
  ends: string;
}

/** A project's or an item's scopes, newest first, each with how it stands. */
export function scopeRows(scopes: AgentScope[], item?: string, now: number = Date.now()): ScopeRow[] {
  return scopes
    .filter((s) => !item || s.item === item)
    .slice()
    .sort((a, b) => b.issued_at.localeCompare(a.issued_at))
    .map((s) => {
      const left = new Date(s.expires_at).getTime() - now;
      const ends =
        s.state === "revoked"
          ? `revoked by ${s.revoked_by ?? "the coordinator"} ${when(s.revoked_at)}${s.revoked_reason ? ` (${s.revoked_reason})` : ""}`
          : s.state === "expired"
            ? `expired ${when(s.expires_at)}`
            : `expires in ${duration(left)}`;
      return {
        id: s.id,
        item: s.item,
        role: s.role,
        state: s.state,
        covers: s.role === "build" ? (s.branch ?? "its branch") : "reads only",
        issued: `by ${s.issued_by} ${when(s.issued_at)}`,
        ends,
      };
    });
}

/** The banner over an item with an agent at work: who, where, since when, until when, and drift. */
export function scopeBanner(s: AgentScope, now: number = Date.now(), drift?: number): string {
  const where = s.role === "build" ? ` on ${s.branch ?? "its branch"}` : " (reads only)";
  const claimed = s.state === "issued" ? "not yet claimed" : `working in ${s.worktree ?? "its worktree"}`;
  const left = `expires in ${duration(new Date(s.expires_at).getTime() - now)}`;
  const ahead =
    drift && drift > 0 ? ` · ${drift} commit${drift === 1 ? "" : "s"} ahead of its tasks` : "";
  return `${s.role} agent ${s.id}${where} · ${claimed} · issued by ${s.issued_by} ${when(s.issued_at)} · ${left}${ahead}`;
}

/** Scopes still in force: issued or active. */
export function liveScopes(scopes: AgentScope[]): AgentScope[] {
  return scopes.filter((s) => s.state === "issued" || s.state === "active");
}

/** Who made a change: the person, and the agent they made it by, when one did. */
export function byAgentText(a: Pick<Activity, "actor" | "by_agent">): string {
  return a.by_agent ? `${a.actor}, by the agent for ${a.by_agent.item} (${a.by_agent.scope})` : a.actor;
}
