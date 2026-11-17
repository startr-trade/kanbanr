/**
 * The Flow panel (FTR-061), first on the Plan page's Current tab: the scope's remaining work in
 * agent-hours over its task graph. What runs, what is ready, what each blocked task waits on, and
 * how long the work takes with 1, 4 or 8 lanes, bounded by the build slots, the critical path as
 * the floor. A forecast is hours from now, never a date; the only dates are the ones a person set
 * (a running sprint's last day, a release's target), shown as chips labelled so.
 *
 * Its critical path is over tasks; the timeline below marks the item-level one (FTR-043).
 */
import { Link } from "react-router-dom";
import { useQuery } from "../../data/useQuery";
import { TAGS, urls } from "../../data/urls";
import { Empty, ErrorState, LoadingState, Section } from "../../ui";
import { dateChips, forecastRows, hoursText, statsLine, taskTextParts, waitText } from "../../planFlow";
import type { FlowPlan, FlowRunning } from "../../types";

/** Reads the running sprint's flow plan (else the open work's); a failed read shows here only. */
export function FlowPanel(p: { project: string }): JSX.Element {
  const { project } = p;
  const flow = useQuery<FlowPlan>(project, urls.planFlow(project), TAGS.planFlow);
  if (!flow.data) {
    return (
      <div className="plan-flow">
        {flow.error ? <ErrorState error={flow.error} retry={flow.reload} /> : <LoadingState label="Reading the flow plan…" />}
      </div>
    );
  }
  return <FlowPanelBody project={project} plan={flow.data} />;
}

function runningTime(r: FlowRunning): string {
  return r.estimate_h != null ? `${hoursText(r.elapsed_h)} of ${hoursText(r.estimate_h)}` : `${hoursText(r.elapsed_h)} so far`;
}

/** A task's text with its `code` and links drawn as elements; no HTML is built from the string. */
function TaskText(p: { text: string }): JSX.Element {
  return (
    <span style={{ overflowWrap: "anywhere" }}>
      {taskTextParts(p.text).map((part, i) => {
        if (part.kind === "code") return <code key={i} style={{ overflowWrap: "anywhere", whiteSpace: "normal" }}>
            {part.text}
          </code>;
        if (part.kind === "link" && part.href?.startsWith("/p/"))
          return (
            <Link key={i} to={part.href}>
              {part.text}
            </Link>
          );
        if (part.kind === "link")
          return (
            <a key={i} href={part.href} rel="noopener noreferrer">
              {part.text}
            </a>
          );
        return <span key={i}>{part.text}</span>;
      })}
    </span>
  );
}

/** Notes that list more than this many ids fold them behind a summary. */
const FOLD_OVER = 5;

/** One note naming ids: a plain line, or a closed `<details>` when the list is long. */
function IdNote(p: { lead: string; ids: string[]; fold: string }): JSX.Element {
  if (p.ids.length <= FOLD_OVER) return <li>{`${p.lead}: ${p.ids.join(", ")}`}</li>;
  return (
    <li>
      <details>
        <summary>{p.fold}</summary>
        <p className="plan-flow-ids">{p.ids.join(", ")}</p>
      </details>
    </li>
  );
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** The panel once its plan is read. Exported so the checks render it without a daemon. */
export function FlowPanelBody(p: { project: string; plan: FlowPlan }): JSX.Element {
  const { plan } = p;
  const chips = dateChips(plan);
  const dates =
    chips.length > 0 ? (
      <ul className="plan-flow-dates" aria-label="Dates a person set">
        {chips.map((c) => (
          <li key={c} className="ui-pill ui-pill-neutral">
            {c}
          </li>
        ))}
      </ul>
    ) : null;
  const nothing = plan.total_h === 0 && plan.running.length === 0 && plan.ready.length === 0 && plan.blocked.length === 0;
  if (nothing) {
    return (
      <div className="plan-flow">
        <Section title="Flow">
          <Empty title="No tasks to plan." ask="Ask Claude to break the sprint's items into tasks." />
          {dates}
        </Section>
      </div>
    );
  }

  const notes: JSX.Element[] = [];
  if (plan.unestimated.length > 0) {
    const lead = `${plural(plan.unestimated.length, "task has", "tasks have")} no estimate (counted as 1 h)`;
    notes.push(<IdNote key="unestimated" lead={lead} ids={plan.unestimated} fold={`${lead} — show`} />);
  }
  if (plan.untasked_items.length > 0) {
    const lead = `${plural(plan.untasked_items.length, "item has", "items have")} no tasks`;
    notes.push(<IdNote key="untasked" lead={lead} ids={plan.untasked_items} fold={`${lead} — show`} />);
  }
  if (plan.outside.length > 0) {
    const lead = `${plural(plan.outside.length, "open task", "open tasks")} outside the scope that its tasks wait on`;
    notes.push(<IdNote key="outside" lead={lead} ids={plan.outside} fold={`${lead} — show`} />);
  }
  notes.push(<li key="share">{`build share ${plan.build_share} (${plan.build_share_measured ? "measured" : "assumed"})`}</li>);

  return (
    <div className="plan-flow">
      <Section title="Flow">
        <p className="plan-flow-stats">{statsLine(plan)}</p>
        <p className="muted small">
          In agent-hours, for {plan.scope}. The critical path here runs over tasks, not items: the timeline below marks the items' own.
        </p>
        {dates}

        <div className="plan-flow-lists">
          <section className="plan-flow-running" aria-labelledby="plan-flow-running">
            <h3 id="plan-flow-running">Running · {plan.running.length}</h3>
            {plan.running.length === 0 ? (
              <p className="muted small">Nothing runs now.</p>
            ) : (
              <ul>
                {plan.running.map((r) => (
                  <li key={r.id}>
                    <code>{r.id}</code> <TaskText text={r.text} />
                    {r.scope ? <span className="muted small"> {r.scope}</span> : null} <span>{runningTime(r)}</span>
                    {r.over ? (
                      <>
                        {" "}
                        <span className="ui-pill ui-pill-warn">over</span>
                      </>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="plan-flow-ready" aria-labelledby="plan-flow-ready">
            <h3 id="plan-flow-ready">Ready · {plan.ready.length}</h3>
            {plan.ready.length === 0 ? (
              <p className="muted small">Nothing is free to start.</p>
            ) : (
              <ul>
                {plan.ready.map((r) => (
                  <li key={r.id}>
                    <code>{r.id}</code> <TaskText text={r.text} />
                    {r.critical ? (
                      <>
                        {" "}
                        <span className="ui-pill ui-pill-warn">critical path</span>
                      </>
                    ) : null}{" "}
                    <span className="muted small">chain {hoursText(r.chain_h)}</span>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="plan-flow-blocked" aria-labelledby="plan-flow-blocked">
            <h3 id="plan-flow-blocked">Blocked · {plan.blocked.length}</h3>
            {plan.blocked.length === 0 ? (
              <p className="muted small">Nothing is blocked.</p>
            ) : (
              <ul>
                {plan.blocked.map((b) => (
                  <li key={b.id}>
                    <code>{b.id}</code>
                    <ul>
                      {b.waits.map((w, i) => (
                        <li key={i}>{waitText(b.id, w)}</li>
                      ))}
                    </ul>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>

        <div className="plan-flow-forecast">
          <table>
            <caption>Forecast, in hours from now</caption>
            <thead>
              <tr>
                <th scope="col">Lanes, build slots and finish</th>
              </tr>
            </thead>
            <tbody>
              {forecastRows(plan).map((r) => (
                <tr key={`${r.forecast.lanes}-${r.forecast.slots}`}>
                  <td>{r.text}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted small">critical path {hoursText(plan.critical_h)}: the least it can take however many lanes run</p>
        </div>

        <ul className="plan-flow-notes muted small">
          {notes}
        </ul>
      </Section>
    </div>
  );
}
