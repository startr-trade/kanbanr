import { Link } from "react-router-dom";
import { stageGateLines } from "../../baseline";
import type { ProjectConfig } from "../../types";

/**
 * The gates the project's engineering baseline requires at its release stages (FTR-033), one line
 * per stage gate: "main: requires `ci` (baseline B-001)". Nothing when the baseline requires none.
 */
export function BaselineStageGates({ project, cadence }: { project: string; cadence: ProjectConfig["cadence"] }) {
  const lines = stageGateLines(cadence);
  if (!lines.length) return null;
  return (
    <div className="small">
      <p>
        <strong>Release stages the baseline requires gates at</strong>{" "}
        <Link to={`/p/${encodeURIComponent(project)}/baseline`}>Baseline</Link>
      </p>
      <ul className="dep-list">
        {lines.map((l) => (
          <li key={`${l.stage} ${l.text}`}>
            stage <strong>{l.stage}</strong> {l.text}
          </li>
        ))}
      </ul>
    </div>
  );
}
