/**
 * The Items list (FTR-048, R-9): every item as a row, filtered by kind, status, milestone, goal,
 * sprint, release and evidence. The filters live in the URL, so each number on the Overview links
 * to the list it counted (`/p/x/items?kind=defect&evidence=red`), and the API does the filtering.
 */
import { useId } from "react";
import { Link, useLocation, useParams } from "react-router-dom";
import { useQuery } from "../data/useQuery";
import { TAGS, urls } from "../data/urls";
import { LiveDot } from "../components/bits";
import { DataTable, Empty, ErrorState, LoadingState, useUrlState, type Column } from "../ui";
import { EVIDENCE, ITEM_FILTERS, itemsCsvName, itemsFilter, type ItemFilterState } from "../itemsQuery";
import { to } from "../overviewLinks";
import type { CompactIndex, ItemRow } from "../types";
import "./ItemsPage.css";

/** What the evidence filter's values say. */
const EVIDENCE_LABEL: Record<(typeof EVIDENCE)[number], string> = {
  red: "Red tests",
  planned: "Planned tests",
  stale: "Stale",
  unproven: "Unproven",
  proven: "Proven",
  untested: "Untested",
};

export default function ItemsPage() {
  const { project = "" } = useParams();
  const { search } = useLocation();
  const index = useQuery<CompactIndex>(project, urls.index(project), TAGS.index);
  const items = useQuery<ItemRow[]>(project, urls.items(project, itemsFilter(search)), TAGS.items);
  return <ItemsBody project={project} index={index.data} items={items} />;
}

type Option = { value: string; label: string };

/** `options` with `current` added when the URL names a value the index does not list. */
function withCurrent(options: Option[], current: string): Option[] {
  if (current === "" || options.some((o) => o.value === current)) return options;
  return [...options, { value: current, label: current }];
}

function unique(values: (string | undefined)[]): string[] {
  return [...new Set(values.filter((v): v is string => !!v))].sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
}

/** Each filter's options, from the compact index; "" (any) first. */
function filterOptions(ix: CompactIndex | null): Record<keyof ItemFilterState, Option[]> {
  const items = ix?.items ?? [];
  const kinds: Option[] = unique(items.map((i) => i.kind)).map((k) => ({ value: k, label: k }));
  if (items.some((i) => !i.kind)) kinds.push({ value: "none", label: "Untyped" });
  const statuses = ix?.config.statuses?.length ? ix.config.statuses : unique(items.map((i) => i.status));
  return {
    kind: kinds,
    status: statuses.map((s) => ({ value: s, label: s })),
    milestone: (ix?.milestones ?? []).map((m) => ({ value: m.code, label: m.name ? `${m.code} ${m.name}` : m.code })),
    goal: (ix?.goals ?? []).map((g) => ({ value: g.id, label: g.id })),
    sprint: (ix?.sprints ?? []).map((s) => ({ value: s.code, label: s.code })),
    release: (ix?.releases ?? []).map((r) => ({ value: r.version, label: r.version })),
    evidence: EVIDENCE.map((e) => ({ value: e, label: EVIDENCE_LABEL[e] })),
  };
}

const FILTER_LABEL: Record<keyof ItemFilterState, [string, string]> = {
  kind: ["Kind", "Any kind"],
  status: ["Status", "Any status"],
  milestone: ["Milestone", "Any milestone"],
  goal: ["Goal", "Any goal"],
  sprint: ["Sprint", "Any sprint"],
  release: ["Release", "Any release"],
  evidence: ["Evidence", "Any evidence"],
};

function columns(project: string): Column<ItemRow>[] {
  return [
    {
      key: "code",
      label: "Code",
      value: (r) => r.code,
      render: (r) => (
        <span className="items-nowrap">
          <Link to={to.item(project, r.code)}>{r.code}</Link>
        </span>
      ),
      sortable: true,
      narrow: "keep",
    },
    { key: "title", label: "Title", value: (r) => r.title, sortable: true, narrow: "keep" },
    { key: "kind", label: "Kind", value: (r) => r.kind ?? null, sortable: true, narrow: "hide" },
    { key: "status", label: "Status", value: (r) => r.status, sortable: true },
    { key: "milestone", label: "Milestone", value: (r) => r.milestone, sortable: true, narrow: "hide" },
    {
      key: "sprint",
      label: "Sprint",
      value: (r) => r.sprint ?? null,
      render: (r) => <span className="items-nowrap">{r.sprint ?? ""}</span>,
      sortable: true,
      narrow: "hide",
    },
    {
      key: "release",
      label: "Release",
      value: (r) => r.release ?? null,
      render: (r) => <span className="items-nowrap">{r.release ?? ""}</span>,
      sortable: true,
      narrow: "hide",
    },
    {
      key: "requirements",
      label: "Requirements proven",
      value: (r) => `${r.req_proven} of ${r.req_total}`,
      sortable: true,
      align: "end",
    },
    { key: "red", label: "Red tests", value: (r) => r.tests_red, sortable: true, align: "end" },
    { key: "stale", label: "Stale", value: (r) => (r.stale ? "stale" : ""), sortable: true, narrow: "hide" },
  ];
}

/** The page without its reads, so web::check_items_page_reads_its_filters_from_the_url can render it. */
export function ItemsBody(p: {
  project: string;
  index: CompactIndex | null;
  items: { data: ItemRow[] | null; loading: boolean; error: string | null; reload?(): void };
}): JSX.Element {
  const [state, setState] = useUrlState(ITEM_FILTERS);
  const { search } = useLocation();
  const base = useId();
  const options = filterOptions(p.index);
  const keys = Object.keys(ITEM_FILTERS) as (keyof ItemFilterState)[];

  return (
    <div className="items-page">
      <div className="page-head">
        <h1>Items</h1>
        <LiveDot />
      </div>
      <div className="page-head" role="group" aria-label="Filters">
        {keys.map((key) => {
          const id = `${base}-${key}`;
          const [label, any] = FILTER_LABEL[key];
          return (
            <label key={key} className="per-page" htmlFor={id}>
              {label}
              <select id={id} name={key} value={state[key]} onChange={(e) => setState({ [key]: e.target.value })}>
                <option value="">{any}</option>
                {withCurrent(options[key], state[key]).map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
          );
        })}
      </div>
      {p.items.loading && !p.items.data ? (
        <LoadingState />
      ) : p.items.error && !p.items.data ? (
        <ErrorState error={p.items.error} retry={p.items.reload} />
      ) : (
        <DataTable
          rows={p.items.data ?? []}
          columns={columns(p.project)}
          rowKey={(r) => r.code}
          csvName={itemsCsvName(search)}
          caption="Items"
          filterPlaceholder="code or title"
          empty={<Empty title="No items match these filters." />}
        />
      )}
    </div>
  );
}
