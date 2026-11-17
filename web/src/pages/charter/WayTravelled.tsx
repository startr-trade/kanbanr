/**
 * The way travelled (FTR-076): the shipped releases in the order they shipped and the next one
 * ahead, with the goals each release moved in a matrix whose cells link to their items. The matrix
 * is the line's table view.
 */
import { Empty, Matrix, StageStepper } from "../../ui";
import { matrixCell, routeSteps, todayLine } from "../../charterModel";
import type { CharterProgress } from "../../types";

export function WayTravelled(p: { progress: CharterProgress; goalIds: string[] }): JSX.Element {
  const route = p.progress.route ?? [];
  if (!p.progress.releases_on) {
    return <Empty title="This project does not ship releases, so there is no line of releases to draw. Finished work shows under Recent wins." />;
  }
  const shipped = route.some((s) => s.state === "shipped");
  return (
    <>
      {!shipped && <p className="muted">Nothing has shipped yet. The first release will start the line.</p>}
      {route.length > 0 && <StageStepper stages={routeSteps(p.progress)} label="Releases travelled" />}
      {shipped && <p className="muted small charter-today">{todayLine(p.progress)}</p>}
      {route.length > 0 && p.goalIds.length > 0 && (
        <Matrix
          rows={p.goalIds}
          cols={route.map((s) => s.version)}
          cell={(g, v) => matrixCell(p.progress, g, v)}
          caption="Items finished per goal in each release"
        />
      )}
    </>
  );
}
