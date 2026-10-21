/**
 * What the Workflow page says about the cadence, the git model and the guard rails (FEAT-213), as
 * pure functions from what the daemon answers to the rows the page draws. Kept apart from the page
 * so `npm run check:workflow` can assert them without a browser.
 *
 * Every rule here is the daemon's: these only put its answers into words, each with the CLI
 * command that changes it. The page is read-only, like the rest of the monitor.
 */
import type { GitDoctorIssue, GitModelView, Guards, ProjectConfig, Release, Sprint } from "./types";

/** How a line reads: on, off, worth a look, or not known here. */
export type Tone = "on" | "off" | "warn" | "unknown";

/** One line of a section: a label, its state, and the command that changes it. */
export interface Row {
  label: string;
  value: string;
  tone?: Tone;
  /** The CLI command that changes it, shown as code. */
  command?: string;
  /** A page in the monitor that shows more. */
  to?: string;
}

export interface CadenceSection {
  sprints: {
    rows: Row[];
    active: { code: string; name: string; dates: string; to: string } | null;
  } | null;
  releases: {
    rows: Row[];
    /** In order; the last is the cut. */
    stages: { name: string; cut: boolean }[];
    /** Planned or unshipped releases, with the stage each is at. */
    open: { version: string; stage: string; to: string }[];
  } | null;
}

const RELEASE_RHYTHM: Record<string, string> = {
  per_sprint: "a release each sprint",
  every_n: "a release every few sprints",
  on_demand: "a release when you decide",
};

const DEFAULT_STAGES = ["candidate", "test", "staging", "production"];

/** The Cadence section, or null when the project has neither sprints nor releases. */
export function cadenceSection(
  project: string,
  config: ProjectConfig,
  sprints: Sprint[],
  releases: Release[],
): CadenceSection | null {
  const cadence = config.cadence ?? {};
  if (!cadence.sprints && !cadence.releases) return null;
  const base = `/p/${encodeURIComponent(project)}`;
  let sprintPart: CadenceSection["sprints"] = null;
  if (cadence.sprints) {
    const running = sprints.find((s) => s.state === "active");
    sprintPart = {
      rows: [
        {
          label: "Sprint length",
          value: cadence.sprint_length_days ? `${cadence.sprint_length_days} days` : "not set",
          command: "kanbanr config cadence --sprint-length <days>",
        },
      ],
      active: running
        ? {
            code: running.code,
            name: running.name ?? running.goal ?? "",
            dates: `${running.start} to ${running.end}`,
            // The board opens on the running sprint.
            to: base,
          }
        : null,
    };
  }
  let releasePart: CadenceSection["releases"] = null;
  if (cadence.releases) {
    const names = cadence.release_stages && cadence.release_stages.length > 0 ? cadence.release_stages : DEFAULT_STAGES;
    releasePart = {
      rows: [
        {
          label: "How often",
          value: cadence.release ? RELEASE_RHYTHM[cadence.release] ?? cadence.release : "not set",
          command: "kanbanr config cadence --release <per_sprint|every_n|on_demand>",
        },
        {
          label: "Stages",
          value: names.join(" → "),
          command: "kanbanr config cadence --release-stages <a,b,c>",
        },
      ],
      stages: names.map((name, i) => ({ name, cut: i === names.length - 1 })),
      open: releases
        .filter((r) => r.state !== "shipped")
        .map((r) => ({ version: r.version, stage: r.stage ?? "not started", to: `${base}/releases` })),
    };
  }
  return { sprints: sprintPart, releases: releasePart };
}

const TOPOLOGY: Record<string, string> = {
  trunk: "trunk: items merge straight into main",
  sprints: "sprints: items merge into the running sprint's branch",
  releases: "releases: items merge into an integration branch, released from release branches",
  sprints_and_releases: "sprints and releases: items merge into the sprint's branch, released from release branches",
};

const PRODUCTION: Record<string, string> = {
  integrate_merge: "when an item is integrated",
  sprint_close: "when a sprint closes",
  release_cut: "when a release is cut",
};

/** The Git model section's lines, in reading order. */
export function gitModelRows(view: GitModelView): Row[] {
  const m = view.model;
  const rows: Row[] = [
    { label: "Topology", value: TOPOLOGY[view.topology] ?? view.topology, command: "kanbanr config cadence --sprints <on|off> --releases <on|off>" },
    { label: "Main", value: m.main, command: "kanbanr config cadence --main-branch <name>" },
  ];
  if (m.integration_branch) {
    rows.push({ label: "Integration branch", value: m.integration_branch, command: "kanbanr config cadence --integration-branch <name>" });
  }
  rows.push(
    { label: "Line of integration", value: view.line },
    { label: "Item branches from", value: view.branch_from },
    { label: "Production moves", value: PRODUCTION[view.production_at] ?? view.production_at },
    { label: "Item branch names", value: m.item_patterns.join(", ") },
    { label: "Protected (merges only)", value: m.protected.join(", ") },
    {
      label: "kanbanr merges",
      value: view.automated ? view.kanbanr_merges.join("; ") : "nothing: with automation off the model is a description",
    },
    {
      label: "Automation",
      value: view.automated ? "on: kanbanr branches and merges by this model" : "off",
      tone: view.automated ? "on" : "off",
      command: `kanbanr config cadence --git-model ${view.automated ? "off" : "on"}`,
    },
    { label: "Pushing", value: "yours: kanbanr never pushes code", command: "git push" },
  );
  return rows;
}

/** What `git doctor` found, as a warning: the count and the first few. Null when it found nothing. */
export function doctorWarning(issues: GitDoctorIssue[], first = 3): { count: number; first: string[] } | null {
  if (issues.length === 0) return null;
  return {
    count: issues.length,
    first: issues.slice(0, first).map((i) => (i.code ? `${i.code}: ${i.message}` : i.message)),
  };
}

/** One guard rail: what it is, its state, and the command that changes it. */
export interface GuardRow extends Row {
  group: "Method" | "Commits" | "Diagrams" | "Coverage" | "Pushing" | "Agent scopes";
  command: string;
  tone: Tone;
}

/** The Guard rails section: every rail, grouped, each with its state and its command. */
export function guardRows(g: Guards): GuardRow[] {
  const rows: GuardRow[] = [];
  for (const p of g.method) {
    rows.push({
      group: "Method",
      label: p.part,
      value: p.on ? (p.status ? `on, at ${p.status}` : "on") : "off",
      tone: p.on ? "on" : "off",
      command: `kanbanr config method ${p.part} --${p.on ? "off" : "on"}`,
    });
  }

  const c = g.commits;
  const hooks = c.hooks_installed;
  const hooksCommand = hooks ? "kanbanr git uninstall-hooks" : "kanbanr git install-hooks";
  rows.push(
    {
      group: "Commits",
      label: "only on an item branch",
      value: c.item_branch ? `on: ${c.branch_pattern}` : "off",
      tone: c.item_branch ? "on" : "off",
      command: "kanbanr start <CODE>",
    },
    {
      group: "Commits",
      label: `${c.trailer}: trailer naming the item`,
      value: `on: ${c.trailer}: kanbanr:<CODE>, or ${c.escape} on the record`,
      tone: "on",
      command: 'kanbanr commit -m "…"',
    },
    {
      group: "Commits",
      label: "never on a protected branch",
      value: c.protected.join(", "),
      tone: "on",
      command: "kanbanr config cadence --git-model <on|off>",
    },
    {
      group: "Commits",
      label: "hooks in this repository",
      value: hooks === null ? "not known here: the monitor does not read the repository's hooks" : hooks ? "installed" : "not installed",
      tone: hooks === null ? "unknown" : hooks ? "on" : "warn",
      command: hooks === null ? "kanbanr git status" : hooksCommand,
    },
  );

  rows.push(
    {
      group: "Diagrams",
      label: "lint on write",
      value: g.diagrams.lint_on_write ? "on: findings reported with the write" : "off",
      tone: g.diagrams.lint_on_write ? "on" : "off",
      command: "kanbanr config diagrams",
    },
    {
      group: "Diagrams",
      label: "strict",
      value: g.diagrams.strict ? "on: a write with a finding is refused" : "off: findings are warnings",
      tone: g.diagrams.strict ? "on" : "off",
      command: `kanbanr config diagrams --strict ${g.diagrams.strict ? "off" : "on"}`,
    },
  );

  if (g.coverage.length === 0) {
    rows.push({
      group: "Coverage",
      label: "rules",
      value: "none",
      tone: "off",
      command: "kanbanr config coverage --from-file <rules.yaml>",
    });
  } else {
    for (const r of g.coverage) {
      rows.push({
        group: "Coverage",
        label: r.id,
        value: r.problem
          ? `invalid: ${r.problem}`
          : r.uncovered === 0
            ? `all ${r.subjects} covered`
            : `${r.uncovered} of ${r.subjects} uncovered`,
        tone: r.problem || r.uncovered > 0 ? "warn" : "on",
        command: "kanbanr config coverage --from-file <rules.yaml>",
      });
    }
  }

  const source = { environment: "from KANBANR_PUSH", board: "this board's setting", default: "the default" }[g.push_policy.source];
  rows.push(
    {
      group: "Pushing",
      label: "the board",
      value: `${g.push_policy.policy} (${source})`,
      tone: g.push_policy.policy === "off" ? "off" : "on",
      command: "kanbanr remote push-policy <auto|off|debounce:N>",
    },
    {
      group: "Pushing",
      label: "the code",
      value: `never by kanbanr: pushing is ${g.code_pushes === "you" ? "yours" : g.code_pushes}`,
      tone: "on",
      command: "git push",
    },
  );

  // What sub-agents may do (FEAT-214): the roles, every scope in force, and the pre-push hook.
  const a = g.agents;
  if (!a) {
    rows.push({
      group: "Agent scopes",
      label: "scopes",
      value: "not known here: this daemon does not report them",
      tone: "unknown",
      command: "kanbanr scope list",
    });
    return rows;
  }
  for (const r of a.roles) {
    rows.push({
      group: "Agent scopes",
      label: r.role,
      value: r.operations.length === 0 ? "reads only" : r.operations.join(", "),
      tone: "on",
      command: `kanbanr scope issue ${r.role} <CODE>`,
    });
  }
  for (const s of a.scopes) {
    rows.push({
      group: "Agent scopes",
      label: s.id,
      value: `${s.role}, ${s.item}, ${s.state}${s.branch ? ` on ${s.branch}` : ""}, until ${s.expires_at.slice(0, 16).replace("T", " ")} UTC`,
      tone: "on",
      command: `kanbanr scope revoke ${s.id} --reason done`,
    });
  }
  const hook = a.push_hook_installed;
  rows.push({
    group: "Agent scopes",
    label: "pre-push hook",
    value:
      hook === null
        ? "not known here: the monitor does not read the repository's hooks"
        : hook
          ? "installed: nothing is pushed from an agent's worktree"
          : "not installed: no scope is issued until it is",
    tone: hook === null ? "unknown" : hook ? "on" : "warn",
    command: hook === null ? "kanbanr git status" : "kanbanr git install-hooks",
  });
  return rows;
}
