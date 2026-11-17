#!/usr/bin/env node
//
// DEF-081 R-8, R-9: the monitor's shared data cache, checked without React or a browser.
//   web::check_cache_rereads_only_matching_slices — a change marks stale only the entries of its
//     project whose tags it names;
//   web::check_cache_shares_one_request_per_url — two reads of one URL in flight run one request;
//   web::check_query_keeps_loading_apart_from_empty — `loading` only while nothing has arrived,
//     `refreshing` while stale data is re-read, so an empty answer is never drawn before data.
//
// Usage: npm run check:data-cache
import { DataCache, queryState, tagsMatch } from "../src/data/cache.ts";

let failed = false;
function report(name, problems) {
  if (problems.length) {
    failed = true;
    for (const p of problems) console.error(`${name} — ${p}`);
  } else {
    console.log(`${name} — ok`);
  }
}

// R-8: only matching slices are re-read.
{
  const problems = [];
  const c = new DataCache();
  const rel = "/api/projects/x/releases";
  const items = "/api/projects/x/features?status=Planned";
  const other = "/api/projects/y/features";
  await c.fetch(rel, ["releases"], async () => ["v1"]);
  await c.fetch(items, ["items"], async () => ["A"]);
  await c.fetch(other, ["items"], async () => ["B"]);
  let heard = 0;
  c.subscribe(items, () => heard++);
  const hit = c.invalidate("x", ["item:X", "items"]);
  if (JSON.stringify(hit) !== JSON.stringify([items])) problems.push(`invalidate returned ${JSON.stringify(hit)}, expected only ${items}`);
  if (c.read(rel).stale) problems.push("the releases entry went stale for an items change");
  if (!c.read(items).stale) problems.push("the items entry did not go stale");
  if (c.read(other).stale) problems.push("another project's entry went stale");
  if (heard !== 1) problems.push(`the items entry's subscriber was told ${heard} time(s), expected 1`);
  if (c.invalidate("x", []).length !== 0) problems.push("an empty change marked entries stale");
  c.invalidate("x", ["*"]);
  if (!c.read(rel).stale) problems.push('"*" did not mark the releases entry stale');
  if (!tagsMatch(["*"], ["releases"]) || tagsMatch(["items"], ["releases"])) problems.push("tagsMatch is wrong for * or disjoint tags");
  // A change during a read leaves the entry stale when the read finishes.
  let release;
  const pending = c.fetch(rel, ["releases"], () => new Promise((r) => (release = r)));
  c.invalidate("x", ["releases"]);
  release(["v2"]);
  await pending;
  if (!c.read(rel).stale) problems.push("a read that started before a change came back fresh");
  report("web::check_cache_rereads_only_matching_slices", problems);
}

// R-8: one request per URL while in flight.
{
  const problems = [];
  const c = new DataCache();
  let runs = 0;
  let release;
  const load = () => {
    runs++;
    return new Promise((r) => (release = r));
  };
  const a = c.fetch("/api/projects/x/index", ["items"], load);
  const b = c.fetch("/api/projects/x/index", ["items"], load);
  release({ items: [] });
  const [ra, rb] = await Promise.all([a, b]);
  if (runs !== 1) problems.push(`the loader ran ${runs} time(s) for two reads in flight, expected 1`);
  if (ra !== rb) problems.push("the two callers got different answers");
  const third = c.fetch("/api/projects/x/index", ["items"], load);
  release({ items: [] });
  await third;
  if (runs !== 2) problems.push(`a read after the first finished ran the loader ${runs} time(s) in all, expected 2`);
  report("web::check_cache_shares_one_request_per_url", problems);
}

// R-9: loading is kept apart from empty.
{
  const problems = [];
  const c = new DataCache();
  const url = "/api/projects/x/features?status=Done";
  const want = (label, got, exp) => {
    for (const [k, v] of Object.entries(exp)) {
      if (JSON.stringify(got[k]) !== JSON.stringify(v)) problems.push(`${label}: ${k} is ${JSON.stringify(got[k])}, expected ${JSON.stringify(v)}`);
    }
  };
  want("never read", queryState(c.read(url)), { data: null, loading: true, refreshing: false });
  let release;
  const first = c.fetch(url, ["items"], () => new Promise((r) => (release = r)));
  want("first read in flight", queryState(c.read(url)), { data: null, loading: true, refreshing: false });
  release([]);
  await first;
  want("empty answer arrived", queryState(c.read(url)), { data: [], error: null, loading: false, refreshing: false });
  c.invalidate("x", ["items"]);
  want("stale", queryState(c.read(url)), { data: [], loading: false, refreshing: true });
  const again = c.fetch(url, ["items"], () => new Promise((r) => (release = r)));
  want("re-read in flight", queryState(c.read(url)), { data: [], loading: false, refreshing: true });
  release([{ code: "A" }]);
  await again;
  want("re-read arrived", queryState(c.read(url)), { data: [{ code: "A" }], loading: false, refreshing: false });
  const bad = "/api/projects/x/config";
  await c.fetch(bad, ["config"], async () => { throw new Error("404 Not Found"); }).catch(() => {});
  want("failed first read", queryState(c.read(bad)), { data: null, error: "404 Not Found", loading: false });
  report("web::check_query_keeps_loading_apart_from_empty", problems);
}

// DEF-106 R-2: a read records when it started, so a page can say how long it has been working.
{
  const problems = [];
  const c = new DataCache();
  const url = "/api/projects/x/review/summary";
  if (queryState(c.read(url)).since !== null) problems.push("an unread URL has a start time");
  let release;
  const before = Date.now();
  const first = c.fetch(url, ["*"], () => new Promise((r) => (release = r)));
  const during = queryState(c.read(url));
  if (typeof during.since !== "number" || during.since < before) problems.push(`first read in flight: since is ${during.since}`);
  if (!during.loading) problems.push("first read in flight is not loading");
  release([]);
  await first;
  if (queryState(c.read(url)).since !== null) problems.push("since is kept after the read ended");
  c.invalidate("x", ["*"]);
  if (queryState(c.read(url)).since !== null) problems.push("a stale entry that is not being read has a start time");
  const again = c.fetch(url, ["*"], () => new Promise((r) => (release = r)));
  const refreshing = queryState(c.read(url));
  if (typeof refreshing.since !== "number" || !refreshing.refreshing) problems.push(`re-read in flight: since is ${refreshing.since}`);
  release([]);
  await again;
  const failing = "/api/projects/x/config";
  await c.fetch(failing, ["config"], async () => { throw new Error("boom"); }).catch(() => {});
  if (queryState(c.read(failing)).since !== null) problems.push("since is kept after a failed read");
  report("web::check_data_cache_records_when_a_read_started", problems);
}

// DEF-106 R-3: a URL already being read is never read a second time, however it is asked meanwhile.
{
  const problems = [];
  const c = new DataCache();
  const url = "/api/projects/x/prepared?state=open";
  let runs = 0;
  let release;
  const load = () => {
    runs++;
    return new Promise((r) => (release = r));
  };
  const a = c.fetch(url, ["*"], load);
  const startedAt = c.read(url).startedAt;
  const b = c.fetch(url, ["*"], load);
  c.invalidate("x", ["*"]);
  const d = c.fetch(url, ["*"], load);
  if (runs !== 1) problems.push(`${runs} reads ran for one URL in flight`);
  if (c.read(url).startedAt !== startedAt) problems.push("a second ask moved the start time");
  release([]);
  await Promise.all([a, b, d]);
  if (c.read(url).stale === false) problems.push("a change during the read left the entry fresh");
  report("web::check_data_cache_never_starts_a_second_read_of_a_url_in_flight", problems);
}

if (failed) process.exit(1);
