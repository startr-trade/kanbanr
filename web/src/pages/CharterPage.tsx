import { useParams } from "react-router-dom";
import { api } from "../api";
import { useAsync, useLiveTick } from "../live";
import { ErrorBox, Loading, LiveDot } from "../components/bits";
import Markdown from "../components/Markdown";
import type { Charter, Lesson } from "../types";

/**
 * The project's charter (FEAT-046): why it exists, what it commits to, for whom, and what it
 * deliberately won't do. This is the one question the board could not answer before — every other
 * page shows *what* is being built.
 *
 * Goals carry ids that work items link, so each goal shows how much work actually serves it; a
 * goal with nothing behind it is a stated intention nobody is delivering, which is worth seeing.
 */
export default function CharterPage() {
  const { project = "" } = useParams();
  const tick = useLiveTick(api.projectEvents(project));
  const charter = useAsync(() => api.getCharter(project), [project, tick]);
  const board = useAsync(() => api.getProject(project), [project, tick]);
  const lessons = useAsync(() => api.getLessons(project), [project, tick]);

  if (charter.loading && !charter.data) return <Loading />;
  if (charter.error) return <ErrorBox error={charter.error} />;
  if (!charter.data) return null;
  const c: Charter = charter.data;

  // How many items serve each goal. Until items carry definitions this is zero everywhere, which
  // reads correctly: nothing links yet.
  const served = (goalId: string) =>
    (board.data?.features ?? []).filter((f) => (f.definition?.goals ?? []).includes(goalId)).length;

  const empty =
    !c.purpose?.trim() && !(c.goals ?? []).length && !(c.non_goals ?? []).length && !(c.stakeholders ?? []).length;

  return (
    <div className="charter-page">
      <div className="page-head">
        <h1>Charter</h1>
        <LiveDot />
      </div>

      {empty ? (
        <p className="muted">
          No charter yet. Write one with <code>kanbanr charter set --file charter.yaml</code> — it is what
          work items link their goals to.
        </p>
      ) : null}

      <section className="section">
        <h2>Purpose</h2>
        {c.purpose?.trim() ? (
          <Markdown source={c.purpose} />
        ) : (
          <span className="chip warn">[MISSING: purpose]</span>
        )}
        {c.vision?.trim() ? <p className="muted">{c.vision}</p> : null}
      </section>

      <section className="section">
        <h2>
          Goals <span className="muted small">(work items link these by id)</span>
        </h2>
        {(c.goals ?? []).length === 0 ? (
          <p className="muted">No goals yet — items have nothing to link to.</p>
        ) : (
          <div className="tiles">
            {c.goals.map((g) => {
              const n = served(g.id);
              return (
                <div className="tile" key={g.id} id={g.id}>
                  <div className="tile-title">
                    <code className="taskkey">{g.id}</code> {g.statement}
                  </div>
                  {g.measure ? <div className="tile-desc">Measure: {g.measure}</div> : null}
                  <div className="tile-states">
                    {n > 0 ? (
                      <span className="chip tasks">
                        {n} item{n === 1 ? "" : "s"}
                      </span>
                    ) : (
                      <span className="chip warn">no items serve this</span>
                    )}
                    {g.horizon ? <span className="chip">{g.horizon}</span> : null}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>

      <Bullets title="Non-goals" hint="deliberately out of scope" items={c.non_goals} />

      {(c.stakeholders ?? []).length > 0 && (
        <section className="section">
          <h2>Stakeholders</h2>
          <ul className="dep-list">
            {(c.stakeholders ?? []).map((s, i) => (
              <li key={i}>
                <strong>{s.name}</strong>
                {s.role ? <span className="muted small"> · {s.role}</span> : null}
                {s.interest ? <div className="muted small">{s.interest}</div> : null}
              </li>
            ))}
          </ul>
        </section>
      )}

      <Bullets title="Constraints" items={c.constraints} />

      {(lessons.data ?? []).length > 0 && (
        <section className="section">
          <h2>
            Lessons <span className="muted small">(confidence decays unless reaffirmed)</span>
          </h2>
          <div className="tiles">
            {(lessons.data ?? []).map((l) => (
              <div className="tile" key={l.id}>
                <div className="tile-title">
                  <code className="taskkey">{l.id}</code> {l.lesson}
                </div>
                {l.evidence ? <div className="tile-desc">{l.evidence}</div> : null}
                <div className="tile-states">
                  <span className={`chip ${l.confidence >= 0.6 ? "tasks" : "warn"}`}>
                    {Math.round(confidenceNow(l) * 100)}% believed
                  </span>
                  <span className="chip">{l.kind}</span>
                  {l.from_item ? <span className="chip">from {l.from_item}</span> : null}
                  {(l.tags ?? []).map((t) => (
                    <span className="chip" key={t}>
                      {t}
                    </span>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </section>
      )}

      {c.adopted_at ? (
        <p className="muted small">
          Adopted {c.adopted_at.slice(0, 10)} — items created before this predate the method.
        </p>
      ) : null}
    </div>
  );
}

/**
 * The same decay the CLI reports: confidence halves every 90 days without reaffirmation. Computed
 * here rather than stored, so a page left open does not show a number that has quietly expired.
 */
function confidenceNow(l: Lesson): number {
  const from = Date.parse(l.last_affirmed || l.at);
  if (Number.isNaN(from)) return l.confidence;
  const days = Math.max(0, (Date.now() - from) / 86_400_000);
  return Math.min(1, Math.max(0, l.confidence * Math.pow(0.5, days / 90)));
}

function Bullets({ title, items, hint }: { title: string; items?: string[]; hint?: string }) {
  if (!items || items.length === 0) return null;
  return (
    <section className="section">
      <h2>
        {title} {hint ? <span className="muted small">({hint})</span> : null}
      </h2>
      <ul className="dep-list">
        {items.map((item, i) => (
          <li key={i}>{item}</li>
        ))}
      </ul>
    </section>
  );
}
