/**
 * What a release's one button says and does (FEAT-212), decided from the daemon's plan: the label
 * names the next step ("Promote v0.2.0 to main", "Cut v0.2.1"), the lines beside it say what the step
 * will do, the cut shows its preview inline, and until the plan is ready the button is disabled
 * and the page says what is missing and how to fix it. A pure function, so the check script can
 * hold it to each case.
 */
import type { ReleasePlan } from "./types";

export interface ReleaseAction {
  /** Whether a button is offered at all: never on a read-only monitor or for an empty release. */
  button: boolean;
  label: string;
  /**
   * The question a direct press asks before it acts (DEF-034): the label with a question mark,
   * present exactly when `button` is. The first press shows it beside the same button and Cancel.
   */
  ask?: string;
  /** The button can be pressed: writable, the daemon has the repository, and the plan is ready. */
  enabled: boolean;
  /**
   * Why the button is disabled, shown beside it and as its title (DEF-071): "" when it is enabled
   * or there is no button; else the first missing thing (with its fix), the tag's sentence, the
   * untested acceptance, or a pointer to the list below.
   */
  disabledWhy: string;
  /** What the step will do, one sentence each. */
  does: string[];
  /** The cut only: the commit it tags, what ships, what carries over. */
  preview?: { tag: string; ships: string[]; carries: { code: string; why: string }[] };
  /** What is missing before it can go, and how to fix each. */
  missing: { text: string; fix: string }[];
  /** The CLI command that does the same thing. */
  command: string;
  /** An unmet `gate_passed`: the commit a result is needed for, and the command that records it. */
  gate?: { commit: string; record: string };
  /** The merge that sent the release back to its first stage, and the steps from there (DEF-031). */
  returned?: { item: string; by: string; at: string; from: string; steps: string[] };
  /** A line shown instead of a button, when there is none to offer. */
  note?: string;
  /** A stage after the tag: the button confirms what the user checked, with an optional note (FTR-026). */
  confirm: boolean;
  /** A failed result for the tag: look again after a rerun, or start a patch release (FTR-026). */
  secondary: ("runAgain" | "startPatch")[];
  /**
   * The cut waits only for the person to accept shipping these requirements unverified, as
   * CODE/R-n (DEF-043). The button stays disabled until the page has the acceptance.
   */
  accept?: string[];
  /** What the cut said of them, beside the list. */
  acceptDetail?: string;
  /**
   * Entering the stage the release is at again would redo something (DEF-057): the second,
   * secondary button, present exactly when the plan carries `again`.
   */
  again?: {
    stage: string;
    label: string;
    /** The question its first press asks (DEF-034): its own label with a question mark. */
    ask: string;
    enabled: boolean;
    /** Why it is disabled, by the same rule as the primary button's (DEF-071). */
    disabledWhy: string;
    does: string[];
    missing: { text: string; fix: string }[];
    command: string;
  };
}

/** The last resort for a disabled button whose plan names nothing more specific (DEF-071). */
export const NOT_READY = "not ready: see what is missing below";

/** The sentence beside a cut that waits only for the untested acceptance (DEF-071). */
export const TICK_UNTESTED = "Tick 'Ship these unverified' below to cut";

/**
 * Why a release button is disabled (DEF-071), in this order: the untested acceptance when the cut
 * waits only for it, the first missing thing (with its fix after " — "), the tag's sentence, else a
 * pointer to the list below.
 * "" when the button is enabled or there is none.
 */
function whyDisabled(
  button: boolean,
  enabled: boolean,
  missing: { text: string; fix: string }[],
  tagSentence?: string,
  untestedOnly = false,
): string {
  if (!button || enabled) return "";
  // A cut waiting only for the person's acceptance says how to give it, not the fix text
  // (DEF-071 T4, 17 Nov).
  if (untestedOnly) return TICK_UNTESTED;
  const first = missing[0];
  if (first) return first.fix ? `${first.text} — ${first.fix}` : first.text;
  if (tagSentence) return tagSentence;
  return NOT_READY;
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
      disabledWhy: "",
      does: [],
      missing: [{ text, fix: "" }],
      command: `kanbanr release plan ${plan.version} <CODE>…`,
      note: text,
      confirm: false,
      secondary: [],
    };
  }
  const does = plan.steps.filter((s) => !s.noop && !s.blocked).map((s) => s.does);
  // A promotion that asks for nothing and does nothing only records the pass (FTR-020 R-5). The
  // button stays: a `note` would replace it, and an older board still promotes through its stages.
  if (!plan.cut && !plan.confirm && plan.steps.length === 0 && plan.conditions.length === 0) {
    does.push(`Nothing happens when you promote to ${plan.to}: it only records that the release passed it.`);
  }
  let missing: { text: string; fix: string }[] = [];
  if (!plan.repo && plan.steps.length + plan.conditions.length > 0) {
    // Everything that reads the repository would say the same thing: say it once, with the way out.
    missing.push({ text: NO_REPO, fix: `run \`${command}\` from the project folder` });
    // A gate result needs the release tip, which the plan carries whether or not the repository
    // could be read; without a tip there is nothing to record against.
    // items_merged, merged_items_ship and stage_passed read the board and the release's history,
    // not the repository.
    const wanted = (c: { check: string }) =>
      c.check === "items_finished" ||
      c.check === "merged_items_ship" ||
      c.check === "items_merged" ||
      c.check === "stage_passed" ||
      (c.check === "gate_passed" && plan.release_tip);
    for (const c of plan.conditions.filter((c) => !c.holds && wanted(c))) {
      missing.push({ text: `${c.check}${c.implied ? " (implied)" : ""}: ${c.detail}`, fix: c.fix ?? "" });
    }
  } else {
    for (const c of plan.conditions.filter((c) => !c.holds)) {
      missing.push({ text: `${c.check}${c.implied ? " (implied)" : ""}: ${c.detail}`, fix: c.fix ?? "" });
    }
    for (const s of plan.steps.filter((s) => s.blocked)) {
      missing.push({ text: s.blocked ?? "", fix: "" });
    }
    // Anything else the plan names (an "already at", a reason a move back needs).
    for (const m of plan.missing) {
      if (!missing.some((x) => x.text === m || x.fix === m)) missing.push({ text: m, fix: "" });
    }
  }
  // A confirmation after the tag waits for a passed result for the tag; when the plan names no
  // condition for it (a project that has no commit gate to judge by judges nothing), the result
  // is still the thing to wait for if one is being judged (FTR-026).
  const judged = plan.confirm && plan.tag != null && plan.tag.gate !== "";
  const tagBlocked = judged && plan.tag?.state !== "passed";
  if (plan.ready && plan.repo) missing = [];
  const tagSentence = tagBlocked
    ? plan.tag?.state === "failed"
      ? `the tag's ${plan.tag.gate} result is failed`
      : "no result recorded for the tag since it was made"
    : undefined;
  if (tagBlocked && !missing.some((m) => m.text.startsWith("tag_passed"))) {
    missing.push({ text: tagSentence ?? "", fix: "run your CI on the tag, then record its verdict" });
  }
  const preview = plan.ships
    ? {
        tag: plan.tag_commit
          ? `${plan.tag_commit.sha.slice(0, 7)}${plan.tag_commit.sha ? " " : ""}${plan.tag_commit.subject}`.trim()
          : "",
        ships: (plan.preview ?? []).filter((p) => p.ships).map((p) => (p.to ? `${p.code} (→ ${p.to})` : p.code)),
        carries: (plan.preview ?? []).filter((p) => !p.ships).map((p) => ({ code: p.code, why: p.why ?? "" })),
      }
    : undefined;
  const unmetGate = plan.conditions.find((c) => !c.holds && c.check === "gate_passed");
  const gate =
    unmetGate && plan.release_tip && !plan.ready
      ? { commit: plan.release_tip, record: `kanbanr gate record <gate> --passed --commit ${plan.release_tip}` }
      : undefined;
  // The cut asks only for the person's acceptance of what ships untested (DEF-043): offered when
  // that is the one unmet condition and nothing else stands in the way.
  const unmet = plan.conditions.filter((c) => !c.holds);
  const untested = unmet.find((c) => c.check === "untested_accepted");
  const acceptOnly =
    untested != null &&
    unmet.every((c) => c.check === "untested_accepted") &&
    plan.repo &&
    !plan.steps.some((s) => s.blocked) &&
    (untested.refs ?? []).length > 0;
  const offer = plan.again;
  const enabled = writable && plan.ready && plan.repo && !tagBlocked;
  const againEnabled = offer ? writable && offer.ready && plan.repo : false;
  const againMissing = offer && !offer.ready ? offer.missing.map((text) => ({ text, fix: "" })) : [];
  return {
    ...(offer
      ? {
          again: {
            stage: offer.stage,
            label: offer.button,
            ask: `${offer.button}?`,
            enabled: againEnabled,
            disabledWhy: whyDisabled(writable, againEnabled, againMissing),
            does: offer.does,
            missing: againMissing,
            command: `kanbanr release promote ${plan.version} ${offer.stage}`,
          },
        }
      : {}),
    ...(acceptOnly ? { accept: untested.refs ?? [], acceptDetail: untested.detail } : {}),
    gate,
    returned: plan.returned,
    button: writable,
    label: plan.button,
    ...(writable ? { ask: `${plan.button}?` } : {}),
    enabled,
    disabledWhy: whyDisabled(writable, enabled, missing, tagSentence, acceptOnly),
    does,
    preview,
    missing,
    command,
    confirm: plan.confirm,
    secondary: plan.confirm && plan.tag?.state === "failed" ? ["runAgain", "startPatch"] : [],
  };
}
