//! Test evidence and source provenance (FEAT-053, FEAT-042): what a test run proves, which recorded
//! results no longer have a test behind them, and which imported items lost their source.
//!
//! Core decides; the front end collects the facts (a run's output, whether a name is still in the
//! repository, whether a file exists) and applies the batch it is handed (FEAT-176). Each planner
//! returns the rows a front end shows and the batch operations that would correct the board, so
//! the report and the write can never disagree.

use crate::models::{Source, TestState};
use crate::store::Project;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Test name -> passed, parsed from a test runner's output. A fixed chain of readers, one per
/// runner (cargo, then marked lines such as Vitest's); the first reader that owns a line decides
/// it. Unknown formats yield nothing, which is the right failure: recording a guess would be
/// worse than recording nothing. A name reported more than once is green only if every line
/// passed (DEF-061).
pub fn parse_results(text: &str) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        let readings = if let Some(r) = read_cargo(line) {
            vec![r]
        } else if let Some(r) = read_pytest(line) {
            vec![r]
        } else if let Some(rs) = read_playwright(line) {
            rs
        } else if let Some(r) = read_marked(line) {
            vec![r]
        } else {
            continue;
        };
        for r in readings {
            match r.outcome {
                Outcome::Passed => record(&mut out, r.name, true),
                Outcome::Failed => record(&mut out, r.name, false),
                Outcome::Unproven => {}
            }
        }
    }
    out
}

/// What one line says, once a reader owns it. `Unproven`: the line is a result that proves
/// nothing (skipped, todo, ignored); owning it keeps a later reader from misreading it.
/// (Named `Outcome`, not `Verdict`: a verdict is a person's decision elsewhere in kanbanr.)
#[derive(Clone, Copy)]
enum Outcome {
    Passed,
    Failed,
    Unproven,
}

struct Reading {
    name: String,
    outcome: Outcome,
}

/// One name's result for the run: a name seen more than once is green only if every line passed.
fn record(out: &mut BTreeMap<String, bool>, name: String, passed: bool) {
    *out.entry(name).or_insert(true) &= passed;
}

/// cargo test: `test NAME ... ok|FAILED|ignored`. Boundary: the line starts with `test `; the
/// name ends at the last ` ... `.
fn read_cargo(line: &str) -> Option<Reading> {
    let rest = line.strip_prefix("test ")?;
    let (name, verdict) = rest.rsplit_once(" ... ")?;
    let outcome = match verdict.trim() {
        "ok" => Outcome::Passed,
        "FAILED" => Outcome::Failed,
        _ => Outcome::Unproven,
    };
    Some(Reading {
        name: name.trim().to_string(),
        outcome,
    })
}

/// pytest -v: `<node id> PASSED|FAILED|ERROR|SKIPPED|XFAIL|XPASS …`. Boundary: the first space
/// followed by one of those words and then the end of the line or a space; the node id is the text
/// before it, cut at ` <- ` (`-vv`), and must contain `::` and not start with `[` (xdist's
/// `[gw0] …` puts the word first and is not read). Everything after the word is ignored.
fn read_pytest(line: &str) -> Option<Reading> {
    const WORDS: [(&str, Outcome); 6] = [
        ("PASSED", Outcome::Passed),
        ("FAILED", Outcome::Failed),
        ("ERROR", Outcome::Failed),
        ("SKIPPED", Outcome::Unproven),
        ("XFAIL", Outcome::Unproven),
        ("XPASS", Outcome::Unproven),
    ];
    for (i, _) in line.match_indices(' ') {
        let rest = &line[i + 1..];
        for (word, outcome) in WORDS {
            let Some(after) = rest.strip_prefix(word) else {
                continue;
            };
            if !(after.is_empty() || after.starts_with(' ')) {
                continue;
            }
            let node = line[..i].trim();
            let node = node.split_once(" <- ").map_or(node, |(n, _)| n.trim());
            if !node.contains("::") || node.starts_with('[') {
                return None;
            }
            return Some(Reading {
                name: node.to_string(),
                outcome,
            });
        }
    }
    None
}

/// Playwright list reporter: `✓|✘|-  <n> [<project>] › <file>:<l>:<c> › <titles> (<duration>)`.
/// Boundary: a mark, two or more spaces, the digits of the index, one space, then a title of an
/// optional `[<project>] › `, a location ending `:<digits>:<digits>`, ` › ` and the titles. Two
/// readings: `<file> › <titles>` and `<titles>`. The `ok`/`x` marks are not read.
fn read_playwright(line: &str) -> Option<Vec<Reading>> {
    const SEP: &str = " \u{203a} ";
    let (mark_len, outcome) = if line.starts_with('\u{2713}') {
        ('\u{2713}'.len_utf8(), Outcome::Passed)
    } else if line.starts_with('\u{2718}') {
        ('\u{2718}'.len_utf8(), Outcome::Failed)
    } else if line.starts_with('-') {
        (1, Outcome::Unproven)
    } else {
        return None;
    };
    let rest = &line[mark_len..];
    let after_spaces = rest.trim_start_matches(' ');
    if rest.len() - after_spaces.len() < 2 {
        return None;
    }
    let digits = after_spaces.bytes().take_while(u8::is_ascii_digit).count();
    let title = after_spaces[digits..]
        .strip_prefix(' ')
        .filter(|_| digits > 0)?;
    // Optional `[<project>] › `.
    let title = match title.strip_prefix('[') {
        Some(t) => {
            let (_project, t) = t.split_once(']')?;
            t.strip_prefix(SEP)?
        }
        None => title,
    };
    let (location, titles) = title.split_once(SEP)?;
    let (file_line, col) = location.rsplit_once(':')?;
    let (file, row) = file_line.rsplit_once(':')?;
    if file.is_empty() || !all_digits(row) || !all_digits(col) {
        return None;
    }
    let titles = without_playwright_suffix(titles);
    if titles.is_empty() {
        return None;
    }
    let mk = |name: String| Reading { name, outcome };
    Some(vec![
        mk(format!("{file}{SEP}{titles}")),
        mk(titles.to_string()),
    ])
}

/// Whether `d` is a Playwright duration: `<N>ms`, `<N>.<d>s`, `<N>.<d>m` or `<N>.<d>h`.
fn is_playwright_duration(d: &str) -> bool {
    if let Some(n) = d.strip_suffix("ms") {
        return all_digits(n);
    }
    let Some(n) = d
        .strip_suffix('s')
        .or_else(|| d.strip_suffix('m'))
        .or_else(|| d.strip_suffix('h'))
    else {
        return false;
    };
    n.split_once('.')
        .is_some_and(|(a, b)| all_digits(a) && b.len() == 1 && all_digits(b))
}

/// A Playwright title without its trailing ` (<duration>)` and then ` (retry #<N>)`.
fn without_playwright_suffix(title: &str) -> &str {
    let mut s = title.trim();
    if let Some(h) = s.strip_suffix(')')
        && let Some(i) = h.rfind(" (")
        && is_playwright_duration(&h[i + 2..])
        && !h[..i].trim_end().is_empty()
    {
        s = h[..i].trim_end();
    }
    if let Some(h) = s.strip_suffix(')')
        && let Some(i) = h.rfind(" (retry #")
        && all_digits(&h[i + " (retry #".len()..])
        && !h[..i].trim_end().is_empty()
    {
        s = h[..i].trim_end();
    }
    s
}

/// A marked line: `✓|×|✗|↓|□|PASS|FAIL <name>`, Vitest's suffix removed. Boundary: the mark is
/// followed by exactly one ASCII space and then a non-space (so Playwright's `✓  1 ...` and
/// nextest's `PASS [` are not this reader's lines).
fn read_marked(line: &str) -> Option<Reading> {
    const MARKS: [(&str, Outcome); 7] = [
        ("\u{2713}", Outcome::Passed),
        ("\u{d7}", Outcome::Failed),
        ("\u{2717}", Outcome::Failed),
        ("\u{2193}", Outcome::Unproven),
        ("\u{25a1}", Outcome::Unproven),
        ("PASS", Outcome::Passed),
        ("FAIL", Outcome::Failed),
    ];
    for (mark, outcome) in MARKS {
        let Some(rest) = line.strip_prefix(mark).and_then(|r| r.strip_prefix(' ')) else {
            continue;
        };
        if rest.is_empty() || rest.starts_with(' ') {
            return None;
        }
        return Some(Reading {
            name: without_vitest_suffix(rest).to_string(),
            outcome,
        });
    }
    None
}

/// `s` without its last space-separated token when `token_ok` accepts it and something remains.
fn drop_last_token<'a>(s: &'a str, token_ok: impl Fn(&str) -> bool) -> Option<&'a str> {
    let (head, tail) = s.rsplit_once(' ')?;
    let head = head.trim_end();
    (!head.is_empty() && token_ok(tail)).then_some(head)
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `name` without Vitest's trailing suffix, from the end and in this order, each at most once and
/// only while something would remain: a `[note]`, ` <N> MB heap used`, ` (repeat x<N>)`,
/// ` (retry x<N>)`, then a token `<N>ms`.
fn without_vitest_suffix(name: &str) -> &str {
    let mut s = name.trim();
    // `[note]`: the last token begins with `[` and ends the line with `]`.
    if s.ends_with(']')
        && let Some(i) = s.rfind(" [")
        && !s[..i].trim_end().is_empty()
    {
        s = s[..i].trim_end();
    }
    if let Some(h) = s.strip_suffix(" MB heap used")
        && let Some(h) = drop_last_token(h, all_digits)
    {
        s = h;
    }
    for (open, close) in [(" (repeat x", ')'), (" (retry x", ')')] {
        if let Some(i) = s.rfind(open) {
            let tail = &s[i + open.len()..];
            if tail.strip_suffix(close).is_some_and(all_digits) && !s[..i].trim_end().is_empty() {
                s = s[..i].trim_end();
            }
        }
    }
    if let Some(h) = drop_last_token(s, |t| t.strip_suffix("ms").is_some_and(all_digits)) {
        s = h;
    }
    s.trim()
}

/// The `test.state` operations a run's `results` call for, stamped with the project revision the
/// run saw. Matching by test NAME rather than by "whatever item is in progress" is what makes this
/// trustworthy: it records what ran, not what someone meant to run. A result already recorded at
/// this revision is left alone, so a repeated run says nothing.
pub fn plan_capture(
    project: &Project,
    results: &BTreeMap<String, bool>,
    rev: Option<&str>,
) -> Vec<Value> {
    let mut ops = Vec::new();
    for f in &project.features {
        let Some(def) = &f.definition else { continue };
        for r in &def.requirements {
            for t in &r.tests {
                // A manual test is a person's check, recorded by them, never by a run (DEF-040).
                if t.kind.eq_ignore_ascii_case("manual") {
                    continue;
                }
                if let Some(passed) = results.get(t.name.as_str()) {
                    let state = if *passed { "green" } else { "red" };
                    if t.state == TestState::parse(state).unwrap_or_default()
                        && (!passed || t.checked_rev.as_str() == rev.unwrap_or(""))
                    {
                        continue; // already recorded at this revision; nothing to say
                    }
                    let mut op = json!({
                        "op": "test.state", "feature": f.code, "requirement": r.id,
                        "test": t.name, "state": state,
                    });
                    if let Some(rev) = rev {
                        op["checked_rev"] = json!(rev);
                    }
                    ops.push(op);
                }
            }
        }
    }
    ops
}

/// The batch body that records a run's results, or `None` when there is nothing to record.
pub fn capture_batch(ops: Vec<Value>) -> Option<Value> {
    if ops.is_empty() {
        return None;
    }
    let n = ops.len();
    Some(json!({ "operations": ops, "message": format!("record {n} test result(s) from a run") }))
}

/// One tracked test and whether its name is still in the repository.
#[derive(Debug, Clone, Serialize)]
pub struct TrackedTest {
    pub code: String,
    pub requirement: String,
    pub test: String,
    pub state: TestState,
    pub found: bool,
}

/// Every tracked test, and the operations that return a vanished one's recorded result to
/// `planned`.
#[derive(Debug, Clone, Default)]
pub struct StaleTests {
    pub rows: Vec<TrackedTest>,
    pub ops: Vec<Value>,
}

impl StaleTests {
    /// The batch body that applies [`StaleTests::ops`], or `None` when nothing is stale.
    pub fn batch(&self) -> Option<Value> {
        if self.ops.is_empty() {
            return None;
        }
        let n = self.ops.len();
        Some(json!({
            "operations": self.ops,
            "message": format!("tests: {n} result(s) whose test no longer exists are back to planned"),
        }))
    }
}

/// Check that every tracked test name still exists (FEAT-053). A test that has been renamed or
/// deleted leaves its last result behind, and that stale green is the most misleading thing a
/// board can hold: it reports a requirement as proven by something nobody can run. Such a result
/// goes back to `planned` rather than being deleted — the requirement still needs a test, it just
/// no longer has one. `repo_search` answers whether a test name is still in the repository.
pub fn plan_stale_tests(project: &Project, repo_search: &dyn Fn(&str) -> bool) -> StaleTests {
    let mut out = StaleTests::default();
    for f in &project.features {
        let Some(def) = &f.definition else { continue };
        for r in &def.requirements {
            for t in &r.tests {
                let name = t.name.trim();
                // A manual check is performed by a person and has no name in the repo to find;
                // treating its absence as rot would push people to stop recording manual gates.
                if name.is_empty() || t.kind.trim().eq_ignore_ascii_case("manual") {
                    continue;
                }
                let found = repo_search(name);
                out.rows.push(TrackedTest {
                    code: f.code.clone(),
                    requirement: r.id.clone(),
                    test: name.to_string(),
                    state: t.state,
                    found,
                });
                // Only recorded evidence is worth correcting: a `planned` test is expected to be
                // absent — that is what planned means.
                if !found && t.state != TestState::Planned {
                    out.ops.push(json!({
                        "op": "test.state", "feature": f.code, "requirement": r.id,
                        "test": name, "state": "planned", "checked_rev": "",
                    }));
                }
            }
        }
    }
    out
}

/// One imported item's provenance, and whether its file source is still there.
#[derive(Debug, Clone, Serialize)]
pub struct SourceRow {
    pub code: String,
    pub system: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub revision: Option<String>,
    pub url: Option<String>,
    pub imported_at: String,
    /// `None` for a source that cannot be checked locally (anything but a file).
    pub present: Option<bool>,
    pub missing_since: Option<String>,
}

/// Every imported item, and the edits that mark a lost source missing (or clear the mark).
#[derive(Debug, Clone, Default)]
pub struct SourcesPlan {
    pub rows: Vec<SourceRow>,
    pub edits: Vec<Value>,
}

impl SourcesPlan {
    /// The batch body that applies [`SourcesPlan::edits`], or `None` when every mark is current.
    pub fn batch(&self) -> Option<Value> {
        (!self.edits.is_empty()).then(
            || json!({"operations": self.edits, "message": "sources: update missing-source marks"}),
        )
    }
}

/// The file a file source names: `TODO.md:14` -> `TODO.md`.
pub fn source_file(src: &Source) -> &str {
    match src.reference.rsplit_once(':') {
        Some((file, line)) if line.chars().all(|c| c.is_ascii_digit()) => file,
        _ => src.reference.as_str(),
    }
}

/// Check imported items' file sources (FEAT-042). `exists` answers whether a path, relative to the
/// project folder, is there; `now` stamps a newly missing source.
pub fn plan_sources(project: &Project, exists: &dyn Fn(&str) -> bool, now: &str) -> SourcesPlan {
    let mut out = SourcesPlan::default();
    for f in &project.features {
        let Some(src) = &f.source else { continue };
        // Only file sources can be checked locally.
        let present = (src.system == "file").then(|| exists(source_file(src)));
        let mut updated = src.clone();
        match present {
            Some(false) if src.missing_since.is_none() => {
                updated.missing_since = Some(now.to_string())
            }
            Some(true) if src.missing_since.is_some() => updated.missing_since = None,
            _ => {}
        }
        if updated != *src {
            out.edits
                .push(json!({"op": "feature.edit", "code": f.code, "source": updated}));
        }
        out.rows.push(SourceRow {
            code: f.code.clone(),
            system: src.system.clone(),
            reference: src.reference.clone(),
            revision: src.revision.clone(),
            url: src.url.clone(),
            imported_at: src.imported_at.clone(),
            present,
            missing_since: updated.missing_since,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_is_read_from_the_formats_a_run_actually_prints() {
        let text = "\
running 3 tests
test hooks::tests::install_merges ... ok
test report::tests::cycle_time ... FAILED
test slow::case ... ignored
✓ cart retains items for 7 days
✗ cart empties on sign-out
PASS src/cart.test.ts
FAIL src/checkout.test.ts
some unrelated line ... ok";
        let results = parse_results(text);
        assert_eq!(results.get("hooks::tests::install_merges"), Some(&true));
        assert_eq!(results.get("report::tests::cycle_time"), Some(&false));
        assert_eq!(results.get("cart retains items for 7 days"), Some(&true));
        assert_eq!(results.get("cart empties on sign-out"), Some(&false));
        assert_eq!(results.get("src/cart.test.ts"), Some(&true));
        assert_eq!(results.get("src/checkout.test.ts"), Some(&false));
        // An ignored test proves nothing, and a line that merely ends in "ok" is not a result.
        assert_eq!(results.get("slow::case"), None);
        assert_eq!(results.len(), 6, "{results:?}");
        // Nothing recognizable: the hook stays quiet rather than guessing.
        assert!(parse_results("cargo build finished in 3.2s").is_empty());
    }

    /// Real lines from the Vitest 5.1.0 verbose run of DOC-001 (14 Nov 2026), leading spaces kept.
    const VITEST_5_1: &str = "\
 \u{2713} src/App.test.tsx > title shows the basket item count 41ms
 \u{2713} src/App.test.tsx > title updates when the basket changes 5ms
 \u{2713} src/components/BasketView.test.tsx > changes a line's quantity 67ms
 \u{2713} src/components/BasketView.test.tsx > removes a line 12ms
 \u{2713} src/components/BasketView.test.tsx > shows the total 9ms
 \u{2713} src/money.test.ts > formats paise as rupees 2ms
 \u{2713} src/money.test.ts > rounds half up 1ms
 \u{2713} src/money.test.ts > formats zero 3ms
 \u{2713} src/money.test.ts > rejects negatives 4ms
\u{2713} built in 163ms";

    #[test]
    fn capture_reads_vitest5_lines_with_durations() {
        let r = parse_results(VITEST_5_1);
        assert_eq!(
            r.get("src/App.test.tsx > title shows the basket item count"),
            Some(&true)
        );
        assert_eq!(
            r.get("src/money.test.ts > formats paise as rupees"),
            Some(&true)
        );
        assert_eq!(
            r.len(),
            10,
            "nine tests and Vite's harmless `built in`: {r:?}"
        );
        assert_eq!(r.get("built in"), Some(&true));
        let one = |t: &str| parse_results(t);
        let r = one(" \u{d7} src/App.test.tsx > title updates when the basket changes 5ms");
        assert_eq!(
            r.get("src/App.test.tsx > title updates when the basket changes"),
            Some(&false)
        );
        assert!(one(" \u{2193} src/x.test.tsx > later [skipped]").is_empty());
        assert!(one(" \u{25a1} src/x.test.tsx > todo").is_empty());
        let r = one(" \u{2713} src/a.test.ts > waits 5s 12ms");
        assert_eq!(r.get("src/a.test.ts > waits 5s"), Some(&true));
        let r = one(" \u{2713} src/a.test.ts > flaky 12ms (retry x1)");
        assert_eq!(r.get("src/a.test.ts > flaky"), Some(&true));
        let r = one(" \u{2713} src/a.test.ts > big 12ms 34 MB heap used");
        assert_eq!(r.get("src/a.test.ts > big"), Some(&true));
        assert_eq!(one("\u{2713} 41ms").get("41ms"), Some(&true));
        let r = one(" \u{2713} src/App.test.tsx (3 tests) 12ms");
        assert_eq!(r.get("src/App.test.tsx (3 tests)"), Some(&true));
        assert_eq!(one("\u{2713} cart retains items for 7 days").len(), 1);
        assert_eq!(
            one("PASS src/cart.test.ts").get("src/cart.test.ts"),
            Some(&true)
        );
        // Fail wins for a name seen twice in one run, in either order.
        let r = one("\u{2713} a > b 1ms\n\u{d7} a > b 2ms");
        assert_eq!(r.get("a > b"), Some(&false));
        let r = one("\u{d7} a > b 1ms\n\u{2713} a > b 2ms");
        assert_eq!(r.get("a > b"), Some(&false));
        // A long transcript of these lines parses to the same map.
        let big = format!("{VITEST_5_1}\n").repeat(1200);
        assert!(big.lines().count() >= 10_000);
        let t = std::time::Instant::now();
        let r = parse_results(&big);
        eprintln!("10k-line parse: {:?}", t.elapsed());
        assert_eq!(r, parse_results(VITEST_5_1));
    }

    /// Real lines from the pytest 9.2.0 `-v` run of DOC-001 (14 Nov 2026), fill spaces kept.
    const PYTEST_9_2: &str = "\
tests/test_basket_api.py::test_only_the_basket_cookie_is_set PASSED      [ 25%]
tests/test_ci_workflow.py::test_publishes_nothing FAILED                 [ 42%]
tests/test_repository.py::test_changelog_exists_with_unreleased_section PASSED [100%]";

    #[test]
    fn pytest_verbose_lines_are_read_as_node_ids() {
        let r = parse_results(PYTEST_9_2);
        assert_eq!(r.len(), 3, "{r:?}");
        assert_eq!(
            r.get("tests/test_basket_api.py::test_only_the_basket_cookie_is_set"),
            Some(&true)
        );
        assert_eq!(
            r.get("tests/test_ci_workflow.py::test_publishes_nothing"),
            Some(&false)
        );
        assert_eq!(
            r.get("tests/test_repository.py::test_changelog_exists_with_unreleased_section"),
            Some(&true)
        );
        let one = |t: &str| parse_results(t);
        assert_eq!(
            one("tests/x.py::test_e ERROR [ 50%]").get("tests/x.py::test_e"),
            Some(&false)
        );
        assert!(one("tests/x.py::test_s SKIPPED (no db) [ 50%]").is_empty());
        assert!(one("tests/x.py::test_xf XFAIL").is_empty());
        assert!(one("tests/x.py::test_xp XPASS").is_empty());
        let r = one("tests/x.py::test_p[a b] PASSED [ 3/12]");
        assert_eq!(r.get("tests/x.py::test_p[a b]"), Some(&true));
        let r = one("tests/x.py::test_v <- src/x.py PASSED");
        assert_eq!(r.get("tests/x.py::test_v"), Some(&true));
        assert!(one("[gw0] [ 25%] PASSED tests/x.py::test_d").is_empty());
        assert!(one("PASSED tests/x.py::test_r").is_empty());
        assert!(one("FAILED tests/x.py::test_r - assert 1 == 2").is_empty());
        assert!(one("the tests PASSED quickly").is_empty());
    }

    /// A real line from the Playwright 1.65.1 list-reporter run of DOC-001 (14 Nov 2026).
    const PLAYWRIGHT_1_65: &str = "  \u{2713}  1 [chromium] \u{203a} e2e/kept-basket.spec.ts:4:3 \u{203a} kept basket \u{203a} basket is kept after the browser is closed and reopened (701ms)";

    #[test]
    fn playwright_list_lines_are_read_as_file_and_titles() {
        let full = "e2e/kept-basket.spec.ts \u{203a} kept basket \u{203a} basket is kept after the browser is closed and reopened";
        let titles = "kept basket \u{203a} basket is kept after the browser is closed and reopened";
        let r = parse_results(PLAYWRIGHT_1_65);
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!(r.get(full), Some(&true));
        assert_eq!(r.get(titles), Some(&true));
        let one = |t: &str| parse_results(t);
        let r = one(
            "  \u{2718}  2 [chromium] \u{203a} e2e/basket.spec.ts:10:1 \u{203a} removes a line (800ms)",
        );
        assert_eq!(
            r.get("e2e/basket.spec.ts \u{203a} removes a line"),
            Some(&false)
        );
        assert_eq!(r.get("removes a line"), Some(&false));
        assert!(
            one("  -  3 [chromium] \u{203a} e2e/basket.spec.ts:20:1 \u{203a} later").is_empty()
        );
        // The same test failing in one project and passing in another stays red, in either order.
        let a = "  \u{2718}  2 [chromium] \u{203a} e2e/basket.spec.ts:10:1 \u{203a} removes a line (800ms)";
        let b = "  \u{2713}  4 [firefox] \u{203a} e2e/basket.spec.ts:10:1 \u{203a} removes a line (retry #1) (1.0s)";
        assert_eq!(
            one(&format!("{a}\n{b}")).get("removes a line"),
            Some(&false)
        );
        assert_eq!(
            one(&format!("{b}\n{a}")).get("removes a line"),
            Some(&false)
        );
        let r = one("  \u{2713}  5 e2e/basket.spec.ts:30:1 \u{203a} no project (1.2m)");
        assert_eq!(r.get("e2e/basket.spec.ts \u{203a} no project"), Some(&true));
        let r = one(
            "  \u{2713}  6 [chromium] \u{203a} e2e/basket.spec.ts:40:1 \u{203a} tagged @smoke (3ms)",
        );
        assert_eq!(
            r.get("e2e/basket.spec.ts \u{203a} tagged @smoke"),
            Some(&true)
        );
        assert!(one("\u{2713}  7 something else").is_empty());
    }

    fn project_with_tests(tests: &str) -> Project {
        let mut p = Project {
            id: "shop".into(),
            config: crate::config::ProjectConfig::default_for("shop"),
            milestones: vec![],
            features: vec![],
        };
        let def: crate::models::FeatureDefinition = crate::yaml::from_str(&format!(
            "requirements:\n  - id: R-1\n    text: x\n    tests:\n{tests}"
        ))
        .unwrap();
        let f: crate::models::FeatureItem = crate::yaml::from_str(
            "code: FEAT-001\ntitle: t\nstatus: Planned\nmilestone: MS-001\n\
             created_at: '2026-01-01T00:00:00Z'\nupdated_at: '2026-01-01T00:00:00Z'\n",
        )
        .unwrap();
        p.features.push(crate::models::FeatureItem {
            definition: Some(def),
            ..f
        });
        p
    }

    #[test]
    fn a_run_records_only_what_changed_at_this_revision() {
        let p = project_with_tests(
            "      - name: a::passes\n        state: green\n        checked_rev: abc\n\
             \x20     - name: b::fails\n        state: green\n        checked_rev: abc\n\
             \x20     - name: c::untracked_run\n",
        );
        let results = parse_results("test a::passes ... ok\ntest b::fails ... FAILED\n");
        let ops = plan_capture(&p, &results, Some("abc"));
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert_eq!(ops[0]["test"], "b::fails");
        assert_eq!(ops[0]["state"], "red");
        assert_eq!(ops[0]["checked_rev"], "abc");
        // A green at a new revision is worth stamping.
        let ops = plan_capture(&p, &results, Some("def"));
        assert_eq!(ops.len(), 2);
        assert!(capture_batch(vec![]).is_none());
        assert_eq!(
            capture_batch(ops).unwrap()["message"],
            "record 2 test result(s) from a run"
        );
    }

    /// DEF-040 R-2: a manual test's result is a person's check, so a run never records one, even
    /// if a runner printed a line with its name.
    #[test]
    fn capture_never_records_a_manual_test() {
        let p = project_with_tests(
            "      - name: 'manual: looks right'\n        kind: manual\n\
             \x20     - name: a::passes\n        kind: unit\n",
        );
        let results = parse_results("test manual: looks right ... ok\ntest a::passes ... ok\n");
        assert_eq!(results.get("manual: looks right"), Some(&true));
        let ops = plan_capture(&p, &results, Some("abc"));
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert_eq!(ops[0]["test"], "a::passes");
    }

    #[test]
    fn a_vanished_test_with_a_result_goes_back_to_planned() {
        let p = project_with_tests(
            "      - name: gone::green\n        state: green\n\
             \x20     - name: gone::planned\n\
             \x20     - name: still::here\n        state: red\n\
             \x20     - name: tried by hand\n        kind: manual\n        state: green\n",
        );
        let plan = plan_stale_tests(&p, &|name| name.starts_with("still"));
        assert_eq!(plan.rows.len(), 3, "a manual check is not searched for");
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(plan.ops[0]["test"], "gone::green");
        assert_eq!(plan.ops[0]["state"], "planned");
        assert!(
            plan.batch().unwrap()["message"]
                .as_str()
                .unwrap()
                .starts_with("tests: 1")
        );
    }

    #[test]
    fn a_lost_file_source_is_marked_and_a_found_one_cleared() {
        let mut p = project_with_tests("      - name: x\n");
        let source = |reference: &str, missing: Option<&str>| Source {
            system: "file".into(),
            reference: reference.into(),
            revision: None,
            url: None,
            imported_at: "2026-01-01T00:00:00Z".into(),
            key: String::new(),
            missing_since: missing.map(String::from),
            extra: Default::default(),
        };
        let mut second = p.features[0].clone();
        second.code = "FEAT-002".into();
        second.source = Some(source("BACKLOG.md", Some("2026-01-02T00:00:00Z")));
        p.features[0].source = Some(source("TODO.md:14", None));
        p.features.push(second);
        let plan = plan_sources(&p, &|path| path == "BACKLOG.md", "2026-02-01T00:00:00Z");
        assert_eq!(plan.rows[0].present, Some(false));
        assert_eq!(
            plan.rows[0].missing_since.as_deref(),
            Some("2026-02-01T00:00:00Z")
        );
        assert_eq!(plan.rows[1].missing_since, None);
        assert_eq!(plan.edits.len(), 2);
        assert!(plan.batch().is_some());
        assert_eq!(source_file(&source("TODO.md:14", None)), "TODO.md");
        assert_eq!(source_file(&source("a:b.md", None)), "a:b.md");
    }
}
