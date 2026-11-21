import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { guardInFlight } from "../inFlight";
import { actionLabel, nearestLabel, nearestRequest, requestText, targetRequest } from "../planning";
import type { PlanningDecision, PlanningRequest, PlanningView } from "../types";
import { AskFirst } from "./AskFirst";

/** The chip a recorded decision shows until Claude acts on it. */
const PENDING = "pending: the agent carries it out at its next turn";

/**
 * Record `request` once per press (DEF-090): a press while the call is in flight is dropped. The
 * outcome, a decision or the refusal's words, replaces whatever was shown before.
 */
function useRecord(project: string, who: string) {
  const [made, setMade] = useState<PlanningDecision | null>(null);
  const [error, setError] = useState<string | null>(null);
  const guard = useRef<ReturnType<typeof guardInFlight<[PlanningRequest]>> | null>(null);
  if (!guard.current) {
    guard.current = guardInFlight(async (r: PlanningRequest) => {
      setError(null);
      try {
        setMade(await api.recordPlanning(project, { ...r, by: who }));
      } catch (e) {
        setMade(null);
        setError(e instanceof Error ? e.message : String(e));
      }
    });
  }
  const run = (r: PlanningRequest) => guard.current!.run(r);
  const reset = () => {
    setMade(null);
    setError(null);
  };
  return { made, error, run, reset };
}

function Outcome({ made, error }: { made: PlanningDecision | null; error: string | null }) {
  if (error) {
    return (
      <span className="chip warn" role="alert">
        {error}
      </span>
    );
  }
  if (made) {
    return (
      <span className="chip" title={made.id}>
        {PENDING}
      </span>
    );
  }
  return null;
}

/**
 * The person's planning decision for the selected items (FTR-049): a select of `targets` (keys
 * `sprint:SP-004` or `release:v0.9.0`) and a button named for the chosen one ("Plan into SP-004",
 * "Move to v0.9.0"). The first press asks (AskFirst, DEF-034); the second records the decision as
 * the person's, which changes no plan until Claude carries it out. Hidden on a read-only monitor.
 */
export function PlanningAction(p: {
  project: string;
  items: string[];
  targets: { key: string; label: string }[];
  writable: boolean;
  /** The person the decision is recorded for: the board's identity. */
  who: string;
  /** Open with the question showing, for a request a drop on the Plan page drafted (FTR-049, R-7). */
  asking?: boolean;
}): JSX.Element | null {
  const [key, setKey] = useState(p.targets[0]?.key ?? "");
  const { made, error, run, reset } = useRecord(p.project, p.who);
  // A target that is no longer offered falls back to the first one.
  const chosen = p.targets.some((t) => t.key === key) ? key : (p.targets[0]?.key ?? "");
  const itemsKey = p.items.join(" ");
  useEffect(reset, [itemsKey, chosen]);
  const request = useMemo(() => targetRequest(chosen, p.items), [chosen, itemsKey]);
  if (!p.writable || p.targets.length === 0) return null;
  const target = p.targets.find((t) => t.key === chosen)!;
  const shape = targetRequest(chosen, ["?"]);
  const label = shape ? actionLabel(shape.action, shape.target) : target.label;
  return (
    <span className="planning-action">
      <select aria-label="Plan into" value={chosen} onChange={(e) => setKey(e.target.value)}>
        {p.targets.map((t) => (
          <option key={t.key} value={t.key}>
            {t.label}
          </option>
        ))}
      </select>{" "}
      <AskFirst
        label={label}
        ask={request ? requestText(request.action, request.target, request.items) : ""}
        disabled={!request}
        disabledWhy="select one or more items first"
        asking={p.asking}
        onConfirm={() => (request ? run(request) : undefined)}
      />{" "}
      <Outcome made={made} error={error} />
    </span>
  );
}

/**
 * A declined planning decision on an item's row (FTR-049, R-5): "declined: <reason>", and, when
 * Claude named a nearest valid move, one button that asks for it for the same items. The reason
 * shows on a read-only monitor too; the button does not.
 */
export function DeclinedPlanning(p: { project: string; view: PlanningView; writable: boolean; who: string }): JSX.Element | null {
  const { made, error, run } = useRecord(p.project, p.who);
  if (p.view.state !== "declined") return null;
  const request = nearestRequest(p.view);
  const offer = nearestLabel(p.view);
  return (
    <span className="planning-declined">
      <span className="chip warn" title={p.view.decision.id}>
        declined: {p.view.outcome?.reason ?? ""}
      </span>
      {p.writable && request && offer && !made ? (
        <>
          {" "}
          <button className="btn" onClick={() => void run(request)}>
            {offer}
          </button>
        </>
      ) : null}{" "}
      <Outcome made={made} error={error} />
    </span>
  );
}
