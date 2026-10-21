/**
 * What a release's one button says and does (FEAT-212), decided from the daemon's plan: the label
 * names the next step ("Promote to main", "Cut v0.2.1"), the lines beside it say what the step
 * will do, the cut shows its preview inline, and until the plan is ready the button is disabled
 * and the page says what is missing and how to fix it. A pure function, so the check script can
 * hold it to each case.
 */
import type { ReleasePlan } from "./types";

export interface ReleaseAction {
  /** Whether a button is offered at all: never on a read-only monitor or for an empty release. */
  button: boolean;
  label: string;
  /** The button can be pressed: writable, the daemon has the repository, and the plan is ready. */
  enabled: boolean;
  /** What the step will do, one sentence each. */
  does: string[];
  /** The cut only: the commit it tags, what ships, what carries over. */
  preview?: { tag: string; ships: string[]; carries: { code: string; why: string }[] };
  /** What is missing before it can go, and how to fix each. */
  missing: { text: string; fix: string }[];
  /** The CLI command that does the same thing. */
  command: string;
  /** A line shown instead of a button, when there is none to offer. */
  note?: string;
}

/** When the daemon was not started in the project folder, it cannot read the code repository. */
export const NO_REPO = "the monitor was not started in the project folder";

export function releaseAction(plan: ReleasePlan, writable: boolean): ReleaseAction {
  const command = `kanbanr release promote ${plan.version}${plan.to ? ` ${plan.to}` : ""}`;
  // A release with nothing planned into it is never promoted (R-17): no button, only the sentence.
  if (plan.empty) {
    const text = plan.missing[0] ?? `${plan.version} has no items`;
    return {
      button: false,
      label: "",
      enabled: false,
      does: [],
      missing: [{ text, fix: "" }],
      command: `kanbanr release plan ${plan.version} <CODE>…`,
      note: text,
    };
  }
  const does = plan.steps.filter((s) => !s.noop && !s.blocked).map((s) => s.does);
  let missing: { text: string; fix: string }[] = [];
  if (!plan.repo && plan.steps.length + plan.conditions.length > 0) {
    // Everything that reads the repository would say the same thing: say it once, with the way out.
    missing.push({ text: NO_REPO, fix: `run \`${command}\` from the project folder` });
    for (const c of plan.conditions.filter((c) => !c.holds && c.check === "items_finished")) {
      missing.push({ text: `${c.check}: ${c.detail}`, fix: c.fix ?? "" });
    }
  } else {
    for (const c of plan.conditions.filter((c) => !c.holds)) {
      missing.push({ text: `${c.check}: ${c.detail}`, fix: c.fix ?? "" });
    }
    for (const s of plan.steps.filter((s) => s.blocked)) {
      missing.push({ text: s.blocked ?? "", fix: "" });
    }
    // Anything else the plan names (an "already at", a reason a move back needs).
    for (const m of plan.missing) {
      if (!missing.some((x) => x.text === m || x.fix === m)) missing.push({ text: m, fix: "" });
    }
  }
  if (plan.ready && plan.repo) missing = [];
  const preview = plan.cut
    ? {
        tag: plan.tag_commit
          ? `${plan.tag_commit.sha.slice(0, 7)}${plan.tag_commit.sha ? " " : ""}${plan.tag_commit.subject}`.trim()
          : "",
        ships: (plan.preview ?? []).filter((p) => p.ships).map((p) => (p.to ? `${p.code} (→ ${p.to})` : p.code)),
        carries: (plan.preview ?? []).filter((p) => !p.ships).map((p) => ({ code: p.code, why: p.why ?? "" })),
      }
    : undefined;
  return {
    button: writable,
    label: plan.button,
    enabled: writable && plan.ready && plan.repo,
    does,
    preview,
    missing,
    command,
  };
}
