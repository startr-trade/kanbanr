import { delegatedHints, nextStageHint, type NextStage } from "../cardText";
import { to } from "../overviewLinks";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { type Connection, useConnection } from "../data/live";
import type { Feature, Task, TaskState } from "../types";
import { featureProgress } from "../types";
import type { Gap } from "../types";

export function Loading() {
  return <div className="muted pad" role="status" aria-live="polite">Loading…</div>;
}

export function ErrorBox({ error }: { error: string }) {
  return <div className="error pad" role="alert">{error}</div>;
}

/**
 * The live dot, with its state written beside it (DEF-082 R-9): "Live", or "Reconnecting…" with
 * the dot grey while the event stream is down, so the state is never carried by colour alone. The
 * text sits inside `role=status`, so a change is announced. Without `state` it reads the current
 * project's stream itself, so a page writes `<LiveDot />`.
 */
export function LiveDot({ state }: { state?: Connection } = {}) {
  const { project } = useParams();
  const own = useConnection(state ? null : project);
  const s = state ?? own;
  const live = s === "live";
  return (
    <span
      className={`livedot-status${live ? "" : " reconnecting"}`}
      role="status"
      title={live ? "Live — updates as the CLI changes data" : "Reconnecting to the live updates…"}
    >
      <span className="livedot" aria-hidden="true" />
      <span className="livedot-text">{live ? "Live" : "Reconnecting…"}</span>
    </span>
  );
}

type Theme = "light" | "dark";

/** Resolve the initial theme: saved choice → OS preference → dark. */
function initialTheme(): Theme {
  const saved = localStorage.getItem("kanbanr-theme");
  if (saved === "light" || saved === "dark") return saved;
  return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

/** A topbar button that toggles light/dark and persists the choice. */
export function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(initialTheme);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("kanbanr-theme", theme);
  }, [theme]);

  const next = theme === "dark" ? "light" : "dark";
  return (
    <button
      type="button"
      className="theme-toggle"
      onClick={() => setTheme(next)}
      aria-label={`Switch to ${next} theme`}
      title={`Switch to ${next} theme`}
    >
      {theme === "dark" ? "☀" : "☾"}
    </button>
  );
}

const TASK_LABEL: Record<TaskState, string> = {
  NotStarted: "Not started",
  InProgress: "In progress",
  Completed: "Completed",
};

export function TaskBadge({ state }: { state: TaskState }) {
  const cls = state === "Completed" ? "done" : state === "InProgress" ? "wip" : "todo";
  return <span className={`taskbadge ${cls}`}>{TASK_LABEL[state]}</span>;
}

export function progress(tasks: Task[]): { done: number; total: number } {
  return {
    done: tasks.filter((t) => t.state === "Completed").length,
    total: tasks.length,
  };
}

/**
 * What a card draws: the item's own fields and its task counts, no bodies (DEF-081). The compact
 * index's items are already this shape; a page holding a whole `Feature` passes `toCard(f)`.
 */
export type CardItem = Pick<
  Feature,
  "code" | "title" | "milestone" | "kind" | "priority" | "assignee" | "team" | "labels" | "depends_on"
> & { done: number; total: number };

/** A whole `Feature` as a card: its fields, and its tasks counted across its todo-lists. */
export function toCard(f: Feature): CardItem {
  const p = featureProgress(f);
  return {
    code: f.code,
    title: f.title,
    milestone: f.milestone,
    kind: f.kind,
    priority: f.priority,
    assignee: f.assignee,
    team: f.team,
    labels: f.labels,
    depends_on: f.depends_on,
    done: p.done,
    total: p.total,
  };
}

type BadgeFields = Pick<Feature, "kind" | "priority" | "assignee" | "team" | "labels">;

/** Small badges for a feature's kind / priority / owner / labels (shared by cards + the feature page). */
export function FeatureBadges({ feature }: { feature: BadgeFields }) {
  return (
    <>
      {feature.kind && <span className="badge kind">{feature.kind}</span>}
      {feature.priority && (
        <span className={`badge prio ${feature.priority.toLowerCase()}`}>{feature.priority}</span>
      )}
      {feature.assignee && <span className="badge assignee">@{feature.assignee}</span>}
      {feature.team && <span className="badge team">{feature.team}</span>}
      {(feature.labels ?? []).map((l) => (
        <span key={l} className="badge label">
          {l}
        </span>
      ))}
    </>
  );
}

/**
 * Whether `feature` is blocked by a same-project dependency that isn't yet Completed (FEAT-027).
 * Computed client-side from the already-loaded sibling items (the compact index's are enough: a code
 * and a status each). Cross-project deps (those carrying a `"proj:CODE"` form) are intentionally
 * ignored here — the board only has this project loaded.
 */
export function isBlockedBySiblings(
  feature: Pick<Feature, "depends_on">,
  siblings: { code: string; status: string }[],
): boolean {
  const deps = (feature.depends_on ?? []).filter((d) => !d.includes(":"));
  if (deps.length === 0) return false;
  return deps.some((code) => {
    const dep = siblings.find((s) => s.code === code);
    // An unresolved (missing) dep is treated as blocking; a present, non-Completed dep blocks.
    return !dep || dep.status !== "Completed";
  });
}

/** A feature card linking to its dedicated page. */
export function FeatureCard({
  project,
  feature,
  siblings,
  gaps = [],
  next = [],
}: {
  project: string;
  feature: CardItem;
  siblings?: { code: string; status: string }[];
  /** From the daemon's readiness engine; only live items have any. */
  gaps?: Gap[];
  /** The next stage and what moving there still lacks (FEAT-117, FEAT-205). */
  next?: NextStage[];
}) {
  const p = { done: feature.done, total: feature.total };
  const blocked = siblings ? isBlockedBySiblings(feature, siblings) : false;
  return (
    <article className="card">
      <div className="card-code">{feature.code}</div>
      <h3 className="card-title">
        <Link className="card-link" to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(feature.code)}`}>
          {feature.title}
        </Link>
      </h3>
      <div className="card-meta">
        {gaps.length > 0 && (
          <span className="chip warn" title={`Not yet stated: ${gaps.map((g) => g.label).join(", ")}`}>
            {gaps.length === 1 && gaps[0].check === "definition" ? "no definition" : `${gaps.length} gaps`}
          </span>
        )}
        {next.map((n) => {
          const hint = nextStageHint(n);
          return hint ? (
            <span key={n.status} className={`hint hint-${hint.kind}`} title={hint.title}>
              {hint.text}
            </span>
          ) : null;
        })}
        {next.flatMap((n) =>
          delegatedHints(n).map((h) => (
            <Link key={`${n.status}:${h.delegation}`} className="hint hint-delegated card-above" to={to.delegation(project, h.delegation)} title={h.title}>
              <span aria-hidden="true">⚙</span> {h.text}
            </Link>
          )),
        )}
        {blocked && <span className="chip blocked">blocked</span>}
        {feature.milestone && <span className="chip">{feature.milestone}</span>}
        {p.total > 0 && (
          <span className="chip tasks">
            {p.done}/{p.total} tasks
          </span>
        )}
        <FeatureBadges feature={feature} />
      </div>
    </article>
  );
}
