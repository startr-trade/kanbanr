/**
 * One item that needs attention, as a card that tells its story in one look (FTR-078): the code and
 * what its badges mean, the title (the only link, so `<details>` inside is valid), the tasks bar,
 * the next stage's checks in words and as a stepper, the milestone's name and at most one curious
 * fact. Read-only: nothing here moves the item.
 */
import { Link } from "react-router-dom";
import { badgeWords, cardFact, checksLeftSegments, CHECK_WORDS, milestoneFullName, milestoneName, stepperSteps } from "../../boardModel.ts";
import type { BoardItem } from "../../boardModel.ts";
import { nextStageHint } from "../../cardText.ts";
import { to } from "../../overviewLinks.ts";
import type { CompactIndex } from "../../types.ts";
import { ProgressBar } from "../../ui/ProgressBar.tsx";
import { StageStepper } from "../../ui/StageStepper.tsx";

export function BoardCard(p: { project: string; b: BoardItem; index: CompactIndex; usual: number | null }): JSX.Element {
  const { b } = p;
  const i = b.item;
  const titleId = `card-${i.code}`;
  const fact = cardFact(b, p.usual);
  const steps = b.next ? stepperSteps(b.next) : [];
  const leftParts = b.next ? checksLeftSegments(b.next) : null;
  const left = leftParts ? leftParts.map((s) => s.text).join("") : null;
  const hint = b.next ? nextStageHint(b.next) : null;
  return (
    <article className="board-card" aria-labelledby={titleId}>
      <div className="board-card-top">
        <span className="board-code">{i.code}</span>
        {badgeWords(i).map((w) => (
          <span key={w.key} className="board-badge" title={w.title}>
            {w.text}
          </span>
        ))}
      </div>
      <h4 className="board-card-title" id={titleId}>
        <Link to={`/p/${encodeURIComponent(p.project)}/feature/${encodeURIComponent(i.code)}`}>{i.title}</Link>
      </h4>
      {i.total > 0 ? (
        <ProgressBar
          label="Tasks"
          total={i.total}
          segments={[
            { key: "done", label: "Done", value: i.done, tone: "ok" },
            { key: "open", label: "Open", value: i.total - i.done, tone: "neutral" },
          ]}
        />
      ) : (
        <p className="board-muted">No tasks yet</p>
      )}
      {b.next && steps.length > 0 && (
        <div className="board-next">
          <p className="board-next-head" title={left ?? undefined}>
            {`Next: ${b.next.status} · ${left === null ? "nothing left but the move" : `${b.next.missing} ${b.next.missing === 1 ? "check" : "checks"} left`}`}
          </p>
          <StageStepper stages={steps} label={`Checks for ${b.next.status}`} words={CHECK_WORDS} />
          {left && (
            <details className="board-left">
              <summary>What is left</summary>
              <p>
                {leftParts?.map((s, k) =>
                  s.delegation ? (
                    <Link key={k} to={to.delegation(p.project, s.delegation)}>
                      {s.text}
                    </Link>
                  ) : (
                    s.text
                  ),
                )}
              </p>
              {b.next.signoffs.length > 0 && <p>Sign-offs: {b.next.signoffs.join(", ")}</p>}
            </details>
          )}
        </div>
      )}
      {b.next && steps.length === 0 && hint && (
        <p className={`hint hint-${hint.kind}`} title={hint.title}>
          {hint.text}
        </p>
      )}
      {i.milestone && (
        <p className="board-milestone" title={milestoneFullName(p.index, i.milestone)}>
          Milestone: {milestoneName(p.index, i.milestone)}
        </p>
      )}
      {fact && (
        <p className="board-fact">
          <span aria-hidden="true">{fact.icon}</span> {fact.text}
        </p>
      )}
    </article>
  );
}
