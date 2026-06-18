import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import type { Feature, Task, TaskState } from "../types";
import { featureProgress } from "../types";

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

/** Small badges for a feature's kind / priority / labels (shared by cards + the feature page). */
export function FeatureBadges({ feature }: { feature: Feature }) {
  return (
    <>
      {feature.kind && <span className="badge kind">{feature.kind}</span>}
      {feature.priority && (
        <span className={`badge prio ${feature.priority.toLowerCase()}`}>{feature.priority}</span>
      )}
      {(feature.labels ?? []).map((l) => (
        <span key={l} className="badge label">
          {l}
        </span>
      ))}
    </>
  );
}

/** A feature card linking to its dedicated page. */
export function FeatureCard({ project, feature }: { project: string; feature: Feature }) {
  const p = featureProgress(feature);
  return (
    <Link className="card" to={`/p/${encodeURIComponent(project)}/feature/${encodeURIComponent(feature.code)}`}>
      <div className="card-code">{feature.code}</div>
      <div className="card-title">{feature.title}</div>
      <div className="card-meta">
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
