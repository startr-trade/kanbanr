/**
 * One release on the Releases page (FTR-041): an `AccordionRow` whose header is two lines (R-8).
 * The first holds its chips and its stage stepper, dots joined by a line with the date and who under
 * each stage reached; the second its next step, the button with its one line beside it.
 *
 * A release in the Current tab, or one that is open, reads its plan for the button (R-9); a closed
 * Planned release shows its next step as text and reads nothing. The body (`ReleaseBody`) is drawn
 * only while the row is open, inside the row's `children`, so a closed release reads no table.
 *
 * The acceptance of what the cut ships untested is ticked in the body and enables the header's
 * button, so the row holds it and passes it to both halves of the button.
 */
import { useState } from "react";
import { AccordionRow, StageStepper } from "../../ui";
import { headerChips, stepperSteps } from "../../releaseTabs";
import type { Release, Scope } from "../../types";
import { ReleaseButton } from "./ReleaseActions";
import { ReleaseBody, type ReleaseCtx } from "./ReleaseBody";

export function ReleaseRow({
  ctx,
  release: r,
  scope,
  next,
  open,
  onToggle,
}: {
  ctx: ReleaseCtx;
  release: Release;
  /** The tab it is listed in. */
  scope: Scope;
  /** The stage its button speaks for; null once it has none (shipped or superseded). */
  next: string | null;
  open: boolean;
  onToggle(open: boolean): void;
}): JSX.Element {
  // Ship these unverified (DEF-043), shared by the header's button and the body's checkbox.
  const [accepted, setAccepted] = useState(false);
  // The title already names the version and the name.
  const chips = headerChips(r, ctx.stages).filter((c) => c.key !== "version" && c.key !== "name");
  const readsPlan = next != null && (scope === "current" || open);
  return (
    <div className="release-head">
      <AccordionRow
        id={r.version}
        level={2}
        open={open}
        onToggle={onToggle}
        title={
          <>
            {r.version}
            {r.name ? ` · ${r.name}` : ""}
          </>
        }
        meta={
          <>
            <span className="release-chips">
              {chips.map((c) => (
                <span key={c.key} className={`ui-pill ui-pill-${c.tone}`}>
                  {c.text}
                </span>
              ))}
            </span>
            <StageStepper stages={stepperSteps(r, ctx.stages)} label={`${r.version}'s stages`} />
          </>
        }
        actions={
          next == null ? undefined : readsPlan ? (
            <ReleaseButton
              project={ctx.project}
              release={r}
              writable={ctx.writable}
              gates={ctx.gates}
              who={ctx.who}
              busy={ctx.busy}
              onPromote={ctx.onPromote}
              outcome={ctx.outcome[r.version]}
              prepared={ctx.prepared}
              onShown={ctx.onShown}
              variant="header"
              accepted={accepted}
              onAccepted={setAccepted}
            />
          ) : (
            <span className="ui-pill ui-pill-neutral release-next">
              next: Promote {r.version} to {next}
            </span>
          )
        }
      >
        {() => <ReleaseBody ctx={ctx} release={r} next={next} accepted={accepted} onAccepted={setAccepted} />}
      </AccordionRow>
    </div>
  );
}
