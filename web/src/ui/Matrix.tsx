/**
 * A matrix (FTR-046, R-5): rows by columns, each cell a number that may link to its list. A real
 * table with a caption and scoped headers. A tone is drawn as the cell's background and, for warn
 * or error, a visible ⚠ plus the word for a screen reader, so it is never carried by colour alone.
 */
import { Link } from "react-router-dom";
import { TONE_WORD, type Tone } from "./tone.ts";

export interface MatrixCell {
  value: number;
  to?: string;
  tone?: Tone;
}

export function Matrix(p: {
  rows: string[];
  cols: string[];
  cell(r: string, c: string): MatrixCell;
  caption: string;
}): JSX.Element {
  return (
    <table className="ui-matrix">
      <caption>{p.caption}</caption>
      <thead>
        <tr>
          <td />
          {p.cols.map((c) => (
            <th key={c} scope="col">
              {c}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {p.rows.map((r) => (
          <tr key={r}>
            <th scope="row">{r}</th>
            {p.cols.map((c) => {
              const cell = p.cell(r, c);
              const tone = cell.tone ?? "neutral";
              const flagged = tone === "warn" || tone === "error";
              return (
                <td key={c} className={`ui-matrix-cell ui-matrix-${tone}`}>
                  {cell.to ? (
                    <Link to={cell.to} aria-label={`${r}, ${c}: ${cell.value}`}>
                      {cell.value}
                    </Link>
                  ) : (
                    cell.value
                  )}
                  {flagged && (
                    <>
                      {" "}
                      <span aria-hidden="true">⚠</span>
                      <span className="ui-sr-only">{TONE_WORD[tone]}</span>
                    </>
                  )}
                </td>
              );
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
