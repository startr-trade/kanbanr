/**
 * What the project will not do (FTR-076): non-goals, stakeholders and constraints as compact tables,
 * one row an entry, nothing dropped. An entry longer than `FOLD_ENTRY` shows its first line, with
 * the whole text one click away in a `<details>`.
 */
import type { ReactNode } from "react";
import { FOLD_ENTRY, fold } from "../../charterModel";
import type { Stakeholder } from "../../types";

function Entry(p: { text: string }): JSX.Element {
  const { head, rest } = fold(p.text, FOLD_ENTRY);
  return (
    <>
      {head}
      {rest && (
        <details className="charter-more">
          <summary>Whole entry</summary>
          <p>{rest}</p>
        </details>
      )}
    </>
  );
}

function TextTable(p: { caption: string; items: string[] }): JSX.Element | null {
  if (p.items.length === 0) return null;
  return (
    <table className="ui-table charter-list">
      <caption>{p.caption}</caption>
      <thead>
        <tr>
          <th scope="col" className="ui-end">
            #
          </th>
          <th scope="col">{p.caption === "Constraints" ? "Constraint" : "Non-goal"}</th>
        </tr>
      </thead>
      <tbody>
        {p.items.map((t, i) => (
          <tr key={i}>
            <td className="ui-end">{i + 1}</td>
            <td>
              <Entry text={t} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function CharterLists(p: { nonGoals: string[]; stakeholders: Stakeholder[]; constraints: string[] }): JSX.Element | null {
  if (!p.nonGoals.length && !p.stakeholders.length && !p.constraints.length) return null;
  const who = (s: Stakeholder): ReactNode => (
    <>
      <strong>{s.name}</strong>
      {s.role ? <span className="muted"> · {s.role}</span> : null}
    </>
  );
  return (
    <>
      <TextTable caption="Non-goals" items={p.nonGoals} />
      {p.stakeholders.length > 0 && (
        <table className="ui-table charter-list">
          <caption>Stakeholders</caption>
          <thead>
            <tr>
              <th scope="col">Who</th>
              <th scope="col">Interest</th>
            </tr>
          </thead>
          <tbody>
            {p.stakeholders.map((s, i) => (
              <tr key={i}>
                <td>{who(s)}</td>
                <td>{s.interest ? <Entry text={s.interest} /> : null}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <TextTable caption="Constraints" items={p.constraints} />
    </>
  );
}
