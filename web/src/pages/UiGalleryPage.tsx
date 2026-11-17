/// <reference types="vite/client" />
/**
 * The UI kit's gallery at /ui (FTR-046, R-8): every component the kit exports, drawn from
 * `ui/fixtures.ts`, in the current theme beside the other one (a nested `data-theme` wrapper), with
 * the type scale, the spacing scale and every colour token with its value and its contrast on
 * `--surface`. Unlinked, in every build, so the person can judge the kit on an installed monitor
 * and a walkthrough can screenshot it.
 */
import { useEffect, useState } from "react";
import tokensCss from "../ui/tokens.css?raw";
import {
  AccordionRow,
  CopyText,
  DataTable,
  Empty,
  ErrorState,
  GrowthChart,
  LoadingState,
  Matrix,
  ProgressBar,
  Section,
  SeverityPill,
  StageStepper,
  StatTile,
  Tabs,
  Timeline,
  parseTokens,
  ratio,
  stepsFromStages,
  useOpenRows,
} from "../ui/index.ts";
import {
  ASK,
  ITEM_COLUMNS,
  ITEM_ROWS,
  MATRIX_COLS,
  MATRIX_ROWS,
  ROWS,
  SEGMENTS,
  SEGMENTS_TOTAL,
  STAGES,
  STAGE_CURRENT,
  STAGE_HISTORY,
  TABS,
  TAB_TEXT,
  TILES,
  TIMELINE,
  TONES,
  matrixCell,
} from "../ui/fixtures.ts";

type Theme = "dark" | "light";

const THEMES = parseTokens(tokensCss);
const TYPE_STEPS = [1, 2, 3, 4, 5].map((n) => `--fs-${n}`);
const SPACE_STEPS = [1, 2, 3, 4, 5, 6].map((n) => `--sp-${n}`);

/** The page's theme, as the toggle sets it on <html>; dark when unset. */
function useCurrentTheme(): Theme {
  const read = (): Theme =>
    typeof document !== "undefined" && document.documentElement.dataset.theme === "light" ? "light" : "dark";
  const [theme, setTheme] = useState<Theme>(read);
  useEffect(() => {
    const obs = new MutationObserver(() => setTheme(read()));
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => obs.disconnect();
  }, []);
  return theme;
}

/** Every colour token of a theme with its value and its contrast on --surface. */
function Colours(p: { theme: Theme }) {
  const t = THEMES[p.theme];
  const surface = t["--surface"];
  const colours = Object.entries(t).filter(([, v]) => /^#[0-9a-f]{6}$/.test(v));
  return (
    <table className="ui-gallery-scale">
      <caption className="ui-sr-only">Colour tokens, {p.theme} theme</caption>
      <thead>
        <tr>
          <th scope="col">Token</th>
          <th scope="col">Value</th>
          <th scope="col">On --surface</th>
        </tr>
      </thead>
      <tbody>
        {colours.map(([name, hex]) => (
          <tr key={name}>
            <th scope="row">
              <span className="ui-gallery-swatch" style={{ background: `var(${name})` }} aria-hidden="true" /> <code>{name}</code>
            </th>
            <td>
              <code>{hex}</code>
            </td>
            <td>{surface ? `${ratio(hex, surface).toFixed(2)}:1` : "—"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** One theme's column: every component, then the scales. */
function Kit(p: { theme: Theme }) {
  const k = p.theme;
  const [open, setOpen] = useOpenRows(`${k}open`);
  const [selected, setSelected] = useState(new Set<string>());
  const steps = stepsFromStages(STAGES, STAGE_CURRENT, STAGE_HISTORY, false);
  return (
    <>
      <h2>{k === "dark" ? "Dark theme" : "Light theme"}</h2>

      <Section title="Tabs" level={3} id={`${k}-tabs`}>
        <Tabs tabs={TABS} param={`${k}tab`} label={`Review groups (${k})`}>
          {(key) => <p>{TAB_TEXT[key]}</p>}
        </Tabs>
      </Section>

      <Section title="Accordion rows" level={3} id={`${k}-rows`}>
        {ROWS.map((r) => (
          <AccordionRow
            key={r.id}
            id={`${k}-${r.id}`}
            level={4}
            title={r.title}
            open={open.has(`${k}-${r.id}`)}
            onToggle={(v) => setOpen(`${k}-${r.id}`, v)}
            meta={r.pills.map((pill) => (
              <SeverityPill key={pill.text} tone={pill.tone}>
                {pill.text}
              </SeverityPill>
            ))}
            select={{
              checked: selected.has(r.id),
              disabled: r.selectDisabled,
              onChange: (v) => {
                const next = new Set(selected);
                if (v) next.add(r.id);
                else next.delete(r.id);
                setSelected(next);
              },
            }}
          >
            {() => <p>{r.body}</p>}
          </AccordionRow>
        ))}
      </Section>

      <Section title="Table" level={3} id={`${k}-table`}>
        <DataTable
          rows={ITEM_ROWS}
          columns={ITEM_COLUMNS}
          rowKey={(r) => r.code}
          stateKey={`${k}t`}
          csvName={`items-${k}`}
          caption="Items in the sprint"
          filterPlaceholder="code, title or stage"
          empty={<Empty title="No items match" />}
        />
      </Section>

      <Section title="Stat tiles" level={3} id={`${k}-tiles`}>
        <div className="ui-gallery-row">
          {TILES.map((t) => (
            <StatTile key={t.label} label={t.label} value={t.value} tone={t.tone} to={t.to} sub={t.sub} spark={t.spark} />
          ))}
        </div>
      </Section>

      <Section title="Progress bar" level={3} id={`${k}-progress`}>
        <ProgressBar segments={SEGMENTS} total={SEGMENTS_TOTAL} label="Sprint items" />
      </Section>

      <Section title="Severity pills" level={3} id={`${k}-pills`}>
        <div className="ui-gallery-row">
          {TONES.map((tone) => (
            <SeverityPill key={tone} tone={tone}>
              {tone === "neutral" ? "draft" : tone === "ok" ? "passed" : `DEF-066 ${tone === "info" ? "noted" : "blocked"}`}
            </SeverityPill>
          ))}
        </div>
      </Section>

      <Section title="Matrix" level={3} id={`${k}-matrix`}>
        <Matrix rows={MATRIX_ROWS} cols={MATRIX_COLS} cell={matrixCell} caption="Items by stage and kind" />
      </Section>

      <Section title="Timeline" level={3} id={`${k}-timeline`}>
        <Timeline from={TIMELINE.from} to={TIMELINE.to} today={TIMELINE.today} lanes={TIMELINE.lanes} caption="Sprints and releases" />
      </Section>

      <Section title="Growth chart" level={3} id={`${k}-growth`}>
        <GrowthChart
          points={[
            { label: "v0.5.0", value: 12, total: 20 },
            { label: "v0.6.0", value: 26, total: 34 },
            { label: "v0.7.0", value: 41, total: 52 },
          ]}
          max={60}
          unit="requirements proven"
          caption="Requirements proven by each release, as tested today"
          tableCaption="Requirements proven by release"
          stateKey={`${k}gc`}
        />
      </Section>

      <Section title="Stage stepper" level={3} id={`${k}-stepper`}>
        <StageStepper stages={steps} label="Release v0.8.0's stages" />
      </Section>

      <Section title="States" level={3} id={`${k}-states`}>
        <LoadingState />
        <ErrorState error="The board could not be read: connection refused" retry={() => undefined} />
        <Empty title="Nothing is ready to pick up" ask={ASK}>
          <p>Every item in the sprint is waiting on another.</p>
        </Empty>
      </Section>

      <Section title="Copy" level={3} id={`${k}-copy`}>
        <CopyText text={ASK} label="Copy the sentence" />
      </Section>

      <Section title="Type scale" level={3} id={`${k}-type`}>
        <table className="ui-gallery-scale">
          <caption className="ui-sr-only">Type scale</caption>
          <tbody>
            {TYPE_STEPS.map((name) => (
              <tr key={name}>
                <th scope="row">
                  <code>{name}</code>
                </th>
                <td>
                  <code>{THEMES[k][name]}</code>
                </td>
                <td style={{ fontSize: `var(${name})` }}>The quick brown fox</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Section>

      <Section title="Spacing" level={3} id={`${k}-space`}>
        <table className="ui-gallery-scale">
          <caption className="ui-sr-only">Spacing scale</caption>
          <tbody>
            {SPACE_STEPS.map((name) => (
              <tr key={name}>
                <th scope="row">
                  <code>{name}</code>
                </th>
                <td>
                  <code>{THEMES[k][name]}</code>
                </td>
                <td>
                  <span className="ui-gallery-space" style={{ width: `var(${name})` }} aria-hidden="true" />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Section>

      <Section title="Colours" level={3} id={`${k}-colours`}>
        <Colours theme={k} />
      </Section>
    </>
  );
}

export default function UiGalleryPage(): JSX.Element {
  const current = useCurrentTheme();
  const other: Theme = current === "dark" ? "light" : "dark";
  return (
    <div className="ui-gallery">
      <h1>UI kit</h1>
      <p>
        Every shared component, drawn from sample data, in the current theme and the other one. Pages are built from these
        (FTR-046).
      </p>
      <div className="ui-gallery-themes">
        <div className="ui-gallery-theme">
          <Kit theme={current} />
        </div>
        <div className="ui-gallery-theme" data-theme={other}>
          <Kit theme={other} />
        </div>
      </div>
    </div>
  );
}
