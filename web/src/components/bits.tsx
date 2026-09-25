import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import type { Feature, Task, TaskState } from "../types";
import { definitionGaps, featureProgress } from "../types";

export function Loading() {
  return <div className="muted pad" role="status" aria-live="polite">Loading…</div>;
}

export function ErrorBox({ error }: { error: string }) {
  return <div className="error pad" role="alert">{error}</div>;
}

export function LiveDot() {
  return (
    <span
      className="livedot"
      role="status"
      aria-label="Live — updates as the CLI changes data"
      title="Live — updates as the CLI changes data"
    />
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

/** Small badges for a feature's kind / priority / owner / labels (shared by cards + the feature page). */
export function FeatureBadges({ feature }: { feature: Feature }) {
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
 * Computed client-side from the already-loaded sibling features. Cross-project deps (those carrying
 * a `"proj:CODE"` form) are intentionally ignored here — the board only has this project loaded.
 */
export function isBlockedBySiblings(feature: Feature, siblings: Feature[]): boolean {
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
}: {
  project: string;
  feature: Feature;
  siblings?: Feature[];
}) {
  const p = featureProgress(feature);
  const blocked = siblings ? isBlockedBySiblings(feature, siblings) : false;
  // Only live work is worth nagging about; finished items are history (the rule doctor uses).
  const gaps = feature.status === "Completed" ? [] : definitionGaps(feature);
  return (
    <Link className="card" to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(feature.code)}`}>
      <div className="card-code">{feature.code}</div>
      <div className="card-title">{feature.title}</div>
      <div className="card-meta">
        {gaps.length > 0 && (
          <span className="chip warn" title={`Not yet stated: ${gaps.join(", ")}`}>
            {gaps.length === 1 && gaps[0] === "no definition" ? "no definition" : `${gaps.length} gaps`}
          </span>
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
    </Link>
  );
}
