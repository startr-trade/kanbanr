/**
 * The monitor's UI kit (FTR-046): import every component, hook and model from here.
 *
 * Each component is a thin `.tsx` over a pure `.ts` model that node checks import directly. The
 * rules every page keeps: tokens only (`tokens.css`), one `<h1>` per page, every card or row header
 * a heading, and tone never carried by colour alone. `/ui` shows every component exported here
 * (`npm run check:ui-gallery` holds the gallery to this list).
 */

// Components.
export { Tabs } from "./Tabs.tsx";
export { AccordionRow } from "./AccordionRow.tsx";
export { DataTable } from "./DataTable.tsx";
export { StatTile, sparkPoints } from "./StatTile.tsx";
export { ProgressBar, progressLabel } from "./ProgressBar.tsx";
export { SeverityPill, saysTone, textOf } from "./SeverityPill.tsx";
export { Matrix, type MatrixCell } from "./Matrix.tsx";
export { Timeline } from "./Timeline.tsx";
export { GrowthChart } from "./GrowthChart.tsx";
export { StageStepper, STEP_WORD } from "./StageStepper.tsx";
export { Empty, ErrorState, LoadingState } from "./States.tsx";
export { Section } from "./Section.tsx";
export { CopyText } from "./CopyText.tsx";

// Hooks and the URL state they keep.
export { parseOpen, parseState, useOpenRows, useUrlState, writeOpen, writeState } from "./urlState.ts";

// Models.
export { TONE_WORD, type Tone } from "./tone.ts";
export { pickTab, visibleTabs, type TabDef } from "./tabs.ts";
export { filterRows, sortRows, toCsv, type Column } from "./table.ts";
export { percentages, type Segment } from "./progress.ts";
export { growthPath, pointText, type GrowthPoint } from "./growth.ts";
export { bandText, clampWindow, dayOf, isoDay, markText, percentDone, scale, type Band, type Lane, type Mark } from "./timeline.ts";
export { stepsFromStages, type History, type StageInput, type Step } from "./stepper.ts";
export { failures, luminance, mix, PAIRS, parseTokens, ratio, type Pair } from "./contrast.ts";
