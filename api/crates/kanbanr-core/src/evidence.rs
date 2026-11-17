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

/// A test's result in one run: whether it passed and, when the runner printed one, how long it
/// took in whole milliseconds (FTR-058).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timed {
    pub passed: bool,
    pub ms: Option<u64>,
}

/// Test name -> passed, parsed from a test runner's output: [`parse_timed_results`] without the
/// durations, so every caller that needs only results is unchanged.
pub fn parse_results(text: &str) -> BTreeMap<String, bool> {
    parse_timed_results(text)
        .into_iter()
        .map(|(name, t)| (name, t.passed))
        .collect()
}

/// Test name -> result and duration, parsed from a test runner's output. A fixed chain of readers,
/// one per runner (unittest, cargo-nextest, cargo, pytest, Playwright, then marked lines such as
/// Vitest's); the first reader that owns a line decides it. Unknown formats yield nothing, which is
/// the right failure: recording a guess would be worse than recording nothing. A name reported more
/// than once is green only if every line passed (DEF-061), and keeps the last duration seen. A
/// unittest head line (a docstring follows) is completed only by the line straight after it
/// (DEF-068). cargo's `Running tests/<file>.rs (…)` line names the integration test file its
/// results belong to, until `Running unittests …` or `Doc-tests …` (MNT-007 R-4).
pub fn parse_timed_results(text: &str) -> BTreeMap<String, Timed> {
    let mut out = BTreeMap::new();
    let mut head: Option<String> = None;
    let mut prefix: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = head.take()
            && let Some((_, verdict)) = line.rsplit_once(" ... ")
            && let Some(outcome) = unittest_outcome(verdict.trim())
        {
            record_outcome(
                &mut out,
                Reading {
                    name,
                    outcome,
                    ms: None,
                },
            );
            continue;
        }
        if let Some(target) = cargo_target(line) {
            prefix = target;
            continue;
        }
        let readings = match read_unittest(line) {
            Some(Unittest::Result(r)) => vec![r],
            Some(Unittest::Head(name)) => {
                head = Some(name);
                continue;
            }
            Some(Unittest::Silent) => continue,
            None => {
                if let Some(r) = read_nextest(line) {
                    vec![r]
                } else if let Some(mut r) = read_cargo(line) {
                    if let Some(file) = &prefix
                        && !r.name.contains("::")
                    {
                        r.name = format!("{file}::{}", r.name);
                    }
                    vec![r]
                } else if let Some(r) = read_pytest(line) {
                    vec![r]
                } else if let Some(rs) = read_playwright(line) {
                    rs
                } else if let Some(r) = read_marked(line) {
                    vec![r]
                } else {
                    continue;
                }
            }
        };
        for r in readings {
            record_outcome(&mut out, r);
        }
    }
    out
}

/// Record one reading: `Passed` green, `Failed` red, `Unproven` nothing.
fn record_outcome(out: &mut BTreeMap<String, Timed>, r: Reading) {
    let passed = match r.outcome {
        Outcome::Passed => true,
        Outcome::Failed => false,
        Outcome::Unproven => return,
    };
    let t = out.entry(r.name).or_insert(Timed {
        passed: true,
        ms: None,
    });
    t.passed &= passed;
    if r.ms.is_some() {
        t.ms = r.ms;
    }
}

/// What a cargo `Running` or `Doc-tests` line says about the lines after it: `Some(Some(file))`
/// for `Running tests/<file>.rs (<path>)`, `Some(None)` for `Running unittests …` and
/// `Doc-tests …`, and `None` for any other line.
fn cargo_target(line: &str) -> Option<Option<String>> {
    if line.starts_with("Doc-tests ") {
        return Some(None);
    }
    let rest = line.strip_prefix("Running ")?;
    if rest.starts_with("unittests ") {
        return Some(None);
    }
    let (file, path) = rest.strip_prefix("tests/")?.split_once(".rs (")?;
    (!file.is_empty() && path.ends_with(')')).then(|| Some(file.to_string()))
}

/// The runner forms capture reads, as the hook's notice and the docs name them.
pub const READABLE_RUNNERS: &str = "cargo test, cargo nextest, pytest -v, python -m unittest -v, Vitest with \
    --reporter=verbose and Playwright with --reporter=list";

/// The test runner one simple command (its words; leading wrappers and assignments are set aside
/// here) invokes, as a short label such as `python3 -m unittest`; None when it runs no tests. Whole
/// words only, never substrings (L-23); the first word is read without its directory.
pub fn test_runner(words: &[String]) -> Option<String> {
    fn base(w: &str) -> &str {
        w.rsplit(['/', '\\']).next().unwrap_or(w)
    }
    // Past every known wrapper first (`rtk`, `rtk proxy`, `env`, `sudo`, ...; FTR-059 R-10).
    let words = &words[crate::shellwrap::command_start(words).0..];
    let mut words: Vec<&str> = words.iter().map(String::as_str).collect();
    if let Some(first) = words.first_mut() {
        *first = base(first);
    }
    // Wrappers that run the command after them.
    let drop = match words.as_slice() {
        ["uv" | "poetry" | "pipenv" | "pdm" | "hatch", "run", ..] | ["bundle", "exec", ..] => 2,
        ["npx" | "pnpx" | "bunx", ..] => 1,
        _ => 0,
    };
    let mut words = words.split_off(drop.min(words.len()));
    if let Some(first) = words.first_mut() {
        *first = base(first);
    }
    let w0 = *words.first()?;
    let w1 = words.get(1).copied();
    let has = |opts: &[&str]| words[1..].iter().find(|w| opts.contains(w)).copied();
    let label = match (w0, w1) {
        ("cargo", Some(sub @ ("test" | "nextest"))) => format!("cargo {sub}"),
        ("pytest" | "py.test" | "vitest" | "jest" | "mocha", _) => w0.to_string(),
        ("rspec" | "tox" | "nox" | "ctest", _) => w0.to_string(),
        ("playwright" | "go" | "dotnet", Some("test")) => format!("{w0} test"),
        ("make", Some(t @ ("test" | "check"))) => format!("make {t}"),
        ("npm" | "pnpm" | "yarn" | "bun", Some("test")) => format!("{w0} test"),
        ("npm" | "pnpm" | "yarn" | "bun", Some("run")) => {
            let script = words.get(2).filter(|w| w.starts_with("test"))?;
            format!("{w0} run {script}")
        }
        ("mvn" | "mvnw", _) => format!("{w0} {}", has(&["test", "verify"])?),
        ("gradle" | "gradlew", _) => format!("{w0} {}", has(&["test", "check"])?),
        (py, _) if py.starts_with("python") => {
            let m = words.iter().position(|w| *w == "-m")?;
            let module = words
                .get(m + 1)
                .filter(|w| matches!(**w, "pytest" | "unittest"))?;
            format!("{py} -m {module}")
        }
        _ => return None,
    };
    Some(label)
}

/// What the capture hook tells Claude about a run of `commands` that yielded `results_read`
/// results; None when no command runs tests. With `condensed_by` None it is DEF-068's notice,
/// given only when nothing was read. With `condensed_by` Some(wrapper), a wrapper that rewrites
/// its command's output (FTR-059 R-10), the notice is always given, because green results may
/// have been cut silently: DEF-068's text followed by the wrapper sentence when nothing was read,
/// else the wrapper sentence alone, saying how many were read.
pub fn unread_run_notice(
    commands: &[Vec<String>],
    results_read: usize,
    condensed_by: Option<&str>,
) -> Option<String> {
    if results_read > 0 && condensed_by.is_none() {
        return None;
    }
    let label = commands.iter().find_map(|c| test_runner(c))?;
    let unread = format!(
        "kanbanr capture read no test result from this run (`{label}`), so the board recorded \
         nothing. It reads {READABLE_RUNNERS}. Run the tests in one of those forms and name each \
         tracked test as that runner prints it; never record a result by hand."
    );
    let Some(w) = condensed_by else {
        return Some(unread);
    };
    let condensed = format!(
        "The command ran through `{w}`, which condenses its output, so kanbanr read only \
         {results_read} result(s) from it and the rest may have been cut: run the tests bare, or \
         as `{w} proxy <command>`, for kanbanr to read them all."
    );
    Some(if results_read == 0 {
        format!("{unread} {condensed}")
    } else {
        condensed
    })
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
    /// The duration the line printed, in whole milliseconds; `None` when it printed none.
    ms: Option<u64>,
}

/// What a unittest line is, once read_unittest owns it.
enum Unittest {
    /// A result line: the name and its outcome.
    Result(Reading),
    /// `<method> (<dotted>)` and nothing else: a docstring's first line follows, the verdict after it.
    Head(String),
    /// Owned, says nothing: a subtest header ending ` ...`, or a verdict that is not one of unittest's.
    Silent,
}

/// Whether `s` is a Python identifier as unittest prints one: non-empty, every char `_` or
/// alphanumeric, the first not an ASCII digit.
fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars().all(|c| c == '_' || c.is_alphanumeric())
}

/// unittest -v: `<method> (<module>.<Class>[.<method>])[ (<params>)] ... <verdict>` (DEF-068).
/// Boundary: the method is the text before the first ` (` and must be an identifier (so cargo's
/// `test a::b ... ok` and a doctest's `test src/lib.rs - f (line 3) ... ok` are not this
/// reader's); the dotted name runs to the first `)` and is two or more identifiers joined by `.`.
/// After it: nothing (a head; the docstring and verdict follow), ` ...` (a subtest header), or an
/// optional ` (<params>)` then ` ... <verdict>`. The name is `<module>.<Class>.<method>`, from
/// either 3.11's form or 3.10's `<method> (<module>.<Class>)`.
fn read_unittest(line: &str) -> Option<Unittest> {
    let (method, rest) = line.split_once(" (")?;
    if !is_identifier(method) {
        return None;
    }
    let (dotted, rest) = rest.split_once(')')?;
    let segments: Vec<&str> = dotted.split('.').collect();
    if segments.len() < 2 || !segments.iter().all(|s| is_identifier(s)) {
        return None;
    }
    let name = if dotted.ends_with(&format!(".{method}")) {
        dotted.to_string()
    } else {
        format!("{dotted}.{method}")
    };
    if rest.is_empty() {
        return Some(Unittest::Head(name));
    }
    if rest == " ..." {
        return Some(Unittest::Silent);
    }
    let (params, verdict) = rest.rsplit_once(" ... ")?;
    if !(params.is_empty() || (params.starts_with(" (") && params.ends_with(')'))) {
        return None;
    }
    Some(match unittest_outcome(verdict.trim()) {
        Some(outcome) => Unittest::Result(Reading {
            name,
            outcome,
            ms: None,
        }),
        None => Unittest::Silent,
    })
}

/// unittest's verdict words: ok → Passed; FAIL, ERROR, unexpected success → Failed;
/// skipped[ '<why>'], expected failure → Unproven; anything else → None.
fn unittest_outcome(verdict: &str) -> Option<Outcome> {
    match verdict {
        "ok" => Some(Outcome::Passed),
        "FAIL" | "ERROR" | "unexpected success" => Some(Outcome::Failed),
        "skipped" | "expected failure" => Some(Outcome::Unproven),
        v if v.starts_with("skipped '") => Some(Outcome::Unproven),
        _ => None,
    }
}

/// cargo-nextest: `<status> [<duration>] [(<i>/<n>)|(───)] <binary-id> <test>` (FTR-058).
/// Boundary: the text before the first `[` is one of nextest's status words, the text up to the
/// first `]` a duration `<s>.<ddd>s` padded with spaces (or `>`-prefixed on a SLOW line), then one
/// space, an optional parenthesised counter, the binary id and the test's name. PASS, LEAK and
/// FLAKY pass; FAIL, LEAK-FAIL, TIMEOUT, ABORT and any SIG<NAME> fail; SLOW, TRY, RETRY, START,
/// TERMINATING and SETUP are owned and prove nothing, so no later reader takes them.
fn read_nextest(line: &str) -> Option<Reading> {
    let (status, rest) = line.split_once('[')?;
    let (duration, rest) = rest.split_once(']')?;
    let rest = rest.strip_prefix(' ')?;
    let words: Vec<&str> = status.split_whitespace().collect();
    let mut outcome = nextest_status(&words)?;
    let duration = duration.trim();
    let ms = match duration.strip_prefix('>') {
        Some(d) => {
            nextest_ms(d.trim())?;
            outcome = Outcome::Unproven;
            None
        }
        None => Some(nextest_ms(duration)?),
    };
    let rest = rest.trim_start();
    let rest = match rest.strip_prefix('(') {
        Some(r) => r.split_once(')')?.1.trim_start(),
        None => rest,
    };
    let (binary_id, test) = rest.split_once(' ')?;
    let test = test.trim();
    if binary_id.is_empty() || test.is_empty() {
        return None;
    }
    let ms = if matches!(outcome, Outcome::Unproven) {
        None
    } else {
        ms
    };
    Some(Reading {
        name: board_name(binary_id, test),
        outcome,
        ms,
    })
}

/// What nextest's status words say, or `None` when they are not nextest's.
fn nextest_status(words: &[&str]) -> Option<Outcome> {
    let k_of_n = |s: &str| {
        s.split_once('/')
            .is_some_and(|(k, n)| all_digits(k) && all_digits(n))
    };
    let signal = |s: &str| {
        s.strip_prefix("SIG")
            .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_uppercase()))
    };
    Some(match words {
        ["PASS" | "LEAK"] => Outcome::Passed,
        ["FLAKY", kn] if k_of_n(kn) => Outcome::Passed,
        ["FAIL" | "LEAK-FAIL" | "TIMEOUT" | "ABORT"] => Outcome::Failed,
        [s] if signal(s) => Outcome::Failed,
        ["SLOW" | "RETRY" | "START" | "TERMINATING" | "SETUP"] => Outcome::Unproven,
        ["TRY", k, "PASS" | "FAIL"] if all_digits(k) => Outcome::Unproven,
        _ => return None,
    })
}

/// nextest's `<s>.<ddd>s` in whole milliseconds, exact.
fn nextest_ms(d: &str) -> Option<u64> {
    let (secs, frac) = d.strip_suffix('s')?.split_once('.')?;
    if !all_digits(secs) || frac.len() != 3 || !all_digits(frac) {
        return None;
    }
    secs.parse::<u64>()
        .ok()?
        .checked_mul(1000)?
        .checked_add(frac.parse().ok()?)
}

/// The board's name for a test nextest printed as `<binary_id> <test>`: an integration test
/// binary's id `<crate>::<file>` (a part after `::` with no `/`) gives `<file>::<test>`, as cargo
/// test's names are read (MNT-007 R-4); a library's (`kanbanr-core`) or a binary's
/// (`kanbanr-cli::bin/kanbanr`) id gives `test` as printed.
pub fn board_name(binary_id: &str, test: &str) -> String {
    match binary_id.split_once("::") {
        Some((_, part)) if !part.is_empty() && !part.contains('/') => format!("{part}::{test}"),
        _ => test.to_string(),
    }
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
        ms: None,
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
                ms: None,
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
    let (titles, ms) = without_playwright_suffix(titles);
    if titles.is_empty() {
        return None;
    }
    let mk = |name: String| Reading { name, outcome, ms };
    Some(vec![
        mk(format!("{file}{SEP}{titles}")),
        mk(titles.to_string()),
    ])
}

/// A Playwright duration in whole milliseconds: `<N>ms` as N; `<N>.<d>s`, `<N>.<d>m` and
/// `<N>.<d>h` as N units and d tenths of one. `None` when `d` is not one.
fn playwright_ms(d: &str) -> Option<u64> {
    if let Some(n) = d.strip_suffix("ms") {
        return if all_digits(n) { n.parse().ok() } else { None };
    }
    let (n, unit) = if let Some(n) = d.strip_suffix('s') {
        (n, 1000)
    } else if let Some(n) = d.strip_suffix('m') {
        (n, 60_000)
    } else {
        (d.strip_suffix('h')?, 3_600_000)
    };
    let (whole, tenth) = n.split_once('.')?;
    if !all_digits(whole) || tenth.len() != 1 || !all_digits(tenth) {
        return None;
    }
    let tenth: u64 = tenth.parse().ok()?;
    whole
        .parse::<u64>()
        .ok()?
        .checked_mul(unit)?
        .checked_add(tenth * unit / 10)
}

/// A Playwright title without its trailing ` (<duration>)` and then ` (retry #<N>)`, and the
/// duration it dropped.
fn without_playwright_suffix(title: &str) -> (&str, Option<u64>) {
    let mut s = title.trim();
    let mut ms = None;
    if let Some(h) = s.strip_suffix(')')
        && let Some(i) = h.rfind(" (")
        && let Some(d) = playwright_ms(&h[i + 2..])
        && !h[..i].trim_end().is_empty()
    {
        s = h[..i].trim_end();
        ms = Some(d);
    }
    if let Some(h) = s.strip_suffix(')')
        && let Some(i) = h.rfind(" (retry #")
        && all_digits(&h[i + " (retry #".len()..])
        && !h[..i].trim_end().is_empty()
    {
        s = h[..i].trim_end();
    }
    (s, ms)
}

/// A marked line: `✓|×|✗|↓|□|PASS|FAIL <name>`, Vitest's suffix removed. Boundary: the mark is
/// followed by exactly one ASCII space and then a non-space (so Playwright's `✓  1 ...` and
/// nextest's `PASS [` are not this reader's lines). The name keeps Vitest's `<N>ms` as the
/// reading's duration.
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
        if rest.is_empty()
            || rest.starts_with(' ')
            || (matches!(mark, "PASS" | "FAIL") && rest.starts_with('['))
        {
            return None;
        }
        let (name, ms) = without_vitest_suffix(rest);
        return Some(Reading {
            name: name.to_string(),
            outcome,
            ms,
        });
    }
    None
}

/// `s` without its last space-separated token when `token_ok` accepts it and something remains.
fn drop_last_token(s: &str, token_ok: impl Fn(&str) -> bool) -> Option<&str> {
    let (head, tail) = s.rsplit_once(' ')?;
    let head = head.trim_end();
    (!head.is_empty() && token_ok(tail)).then_some(head)
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `name` without Vitest's trailing suffix, from the end and in this order, each at most once and
/// only while something would remain: a `[note]`, ` <N> MB heap used`, ` (repeat x<N>)`,
/// ` (retry x<N>)`, then a token `<N>ms`, which is returned as the duration.
fn without_vitest_suffix(name: &str) -> (&str, Option<u64>) {
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
    let mut ms = None;
    if let Some(h) = drop_last_token(s, |t| t.strip_suffix("ms").is_some_and(all_digits)) {
        ms = s[h.len()..]
            .trim()
            .strip_suffix("ms")
            .and_then(|n| n.parse().ok());
        s = h;
    }
    (s.trim(), ms)
}

/// The `test.state` operations a run's `results` call for, stamped with the project revision the
/// run saw. Matching by test NAME rather than by "whatever item is in progress" is what makes this
/// trustworthy: it records what ran, not what someone meant to run. A result already recorded at
/// this revision is left alone, so a repeated run says nothing, unless the run timed it: then the
/// op carries the item's state unchanged and the duration, which the store keeps in the run's
/// `timings/` file without rewriting the item (FTR-058).
pub fn plan_capture(
    project: &Project,
    results: &BTreeMap<String, Timed>,
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
                if let Some(timed) = results.get(t.name.as_str()) {
                    let passed = timed.passed;
                    let state = if passed { "green" } else { "red" };
                    if t.state == TestState::parse(state).unwrap_or_default()
                        && (!passed || t.checked_rev.as_str() == rev.unwrap_or(""))
                        && timed.ms.is_none()
                    {
                        continue; // already recorded at this revision; nothing to say
                    }
                    let mut op = json!({
                        "op": "test.state", "feature": f.code, "requirement": r.id,
                        "test": t.name, "state": state,
                    });
                    if let Some(ms) = timed.ms {
                        op["ms"] = json!(ms);
                    }
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

/// The test kinds whose names are not in the repository: `manual`, a person's check, and
/// `walkthrough`, a run's (FTR-037). Neither is searched for or set back to planned when absent.
pub fn is_unsearchable(kind: &str) -> bool {
    let k = kind.trim();
    k.eq_ignore_ascii_case("manual") || k.eq_ignore_ascii_case("walkthrough")
}

/// The hint `kanbanr check` gives for a `walkthrough` test not yet green: a run proves it, never a
/// person by hand (FTR-037 R-6).
pub const WALKTHROUGH_HINT: &str =
    "proved by a walkthrough run: kanbanr walkthrough run <runbook doc>";

/// What a tracked test's name says about where its definition sits (MNT-001 R-1). The kind
/// decides how the test is looked for: a Rust test by `fn <name>` under its module path, a Python
/// test by `def <name>` in its file, a monitor check by the name its script prints, a path by the
/// file, and anything else only by its exact text. Never by substring (L-23).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub enum TestKind {
    /// `a::b::name`: `modules` is every segment but the last. Empty for an integration test named
    /// bare, as boards named them before the capture hook recorded `file::name` (MNT-007).
    Rust { modules: Vec<String>, name: String },
    /// `test_x.Class.name` (`file` is `test_x.py`), or `Class.name` with an empty `file`, which
    /// is searched in every `scripts/tests/*.py`.
    Python {
        file: String,
        class: String,
        name: String,
    },
    /// A monitor check, `web::…` or `web check: …`, found by the exact name its script prints.
    Web { name: String },
    /// A path to a test file, found by the file itself.
    File { path: String },
    /// Any other text, a description with spaces included: found only when a script prints that
    /// exact text. Never interpolated into a pattern.
    Literal { text: String },
    /// A person's check: never looked for in the code.
    Manual,
    /// A walkthrough run's check (FTR-037): never looked for in the code.
    Walkthrough,
}

/// The extensions a `File` test name may end in.
const SOURCE_EXTENSIONS: &[&str] = &["rs", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs", "sh"];

/// `[A-Za-z0-9_]+`: one `\w+` run as a regex would read it, ASCII only.
fn is_word(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Reads a tracked test's name (and its recorded `kind`) as the kind of test it names, so it can
/// be looked for where such a test is defined (MNT-001 R-1). In order: `manual`/`walkthrough`
/// (by kind or by prefix); a monitor check (`web::`, `web check:`); a name with a character
/// outside `[A-Za-z0-9_:./ -]` is a `Literal`, so no such name is ever put in a pattern; a path
/// (a `/`, no space, a source extension); `a::b` (every segment a word) is Rust;
/// `test_x.Class.name` and `Class.name` are Python; a bare lower-case word is a Rust integration
/// test; anything else is a `Literal`.
pub fn classify(name: &str, kind: &str) -> TestKind {
    let kind = kind.trim();
    if kind.eq_ignore_ascii_case("manual") || name.starts_with("manual:") {
        return TestKind::Manual;
    }
    if kind.eq_ignore_ascii_case("walkthrough") || name.starts_with("walkthrough:") {
        return TestKind::Walkthrough;
    }
    if name.starts_with("web::") || name.starts_with("web check:") {
        return TestKind::Web {
            name: name.to_string(),
        };
    }
    let literal = || TestKind::Literal {
        text: name.to_string(),
    };
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_:./ -".contains(c))
    {
        return literal();
    }
    if name.contains('/')
        && !name.contains(' ')
        && name
            .rsplit_once('.')
            .is_some_and(|(_, ext)| SOURCE_EXTENSIONS.contains(&ext))
    {
        return TestKind::File {
            path: name.to_string(),
        };
    }
    if name.contains("::") {
        let segments: Vec<&str> = name.split("::").collect();
        if !segments.iter().all(|s| is_word(s)) {
            return literal();
        }
        let (last, modules) = segments.split_last().expect("split yields one segment");
        return TestKind::Rust {
            modules: modules.iter().map(|s| s.to_string()).collect(),
            name: last.to_string(),
        };
    }
    let dotted: Vec<&str> = name.split('.').collect();
    if dotted.iter().all(|s| is_word(s)) {
        // `^test_\w+(\.\w+){2}$`
        if let [file, class, test] = dotted[..]
            && file.len() > "test_".len()
            && file.starts_with("test_")
        {
            return TestKind::Python {
                file: format!("{file}.py"),
                class: class.to_string(),
                name: test.to_string(),
            };
        }
        // `^[A-Z]\w*\.\w+$`
        if let [class, test] = dotted[..]
            && class.starts_with(|c: char| c.is_ascii_uppercase())
        {
            return TestKind::Python {
                file: String::new(),
                class: class.to_string(),
                name: test.to_string(),
            };
        }
    }
    // `^[a-z_][a-z0-9_]*$`
    if name.starts_with(|c: char| c == '_' || c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c == '_' || c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        return TestKind::Rust {
            modules: Vec::new(),
            name: name.to_string(),
        };
    }
    literal()
}

/// The path suffixes a test of this kind may be defined in (MNT-001 R-1): a definition found
/// anywhere else is "defined under another path". `a::b::name` → `a/b.rs`, `a/b/mod.rs`; a
/// trailing `tests` module is the file's own (`a::tests::name` → `a.rs`, `a/mod.rs`); a bare
/// name → `tests/` (any integration file); Python → its file under `scripts/tests/` or anywhere
/// (`Class.name` → `scripts/tests/`); a monitor check → `web/scripts/`; a path → itself. A
/// `Literal`, manual or walkthrough test names no place.
pub fn expected_paths(kind: &TestKind) -> Vec<String> {
    match kind {
        TestKind::Rust { modules, .. } => {
            let mut modules: &[String] = modules;
            if modules.len() > 1 && modules.last().is_some_and(|m| m == "tests") {
                modules = &modules[..modules.len() - 1];
            }
            if modules.is_empty() {
                return vec!["tests/".to_string()];
            }
            let base = modules.join("/");
            vec![format!("{base}.rs"), format!("{base}/mod.rs")]
        }
        TestKind::Python { file, .. } if file.is_empty() => vec!["scripts/tests/".to_string()],
        TestKind::Python { file, .. } => vec![format!("scripts/tests/{file}"), file.clone()],
        TestKind::Web { .. } => vec!["web/scripts/".to_string()],
        TestKind::File { path } => vec![path.clone()],
        TestKind::Literal { .. } | TestKind::Manual | TestKind::Walkthrough => Vec::new(),
    }
}

/// Where a test's definition was found: a repository-relative path and its 1-based line.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Location {
    pub path: String,
    pub line: u32,
}

/// What the repository says about a recorded revision: whether it has the commit, whether the
/// commit is an ancestor of HEAD, and how many commits behind HEAD it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RevFacts {
    pub known: bool,
    pub on_head_line: bool,
    pub behind: Option<u64>,
}

/// Finds test definitions and revision facts for the reconciliation planner (MNT-001). Core
/// plans over this trait; `GitLocator` answers it from the repository, a test from a fixture.
pub trait Locator {
    /// Where a test of this kind is defined at `rev`; `None` is the working tree. Empty when it
    /// is not defined there.
    fn find(&self, kind: &TestKind, rev: Option<&str>) -> Vec<Location>;
    /// What the repository knows about `rev`.
    fn revision(&self, rev: &str) -> RevFacts;
    /// The revision `find(_, None)` answers for.
    fn head(&self) -> String;
    /// Told every kind the planner will ask about at `rev` before it asks, so a locator can
    /// answer them in one pass. A no-op by default.
    fn prime(&self, _kinds: &[TestKind], _rev: Option<&str>) {}
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
                // treating its absence as rot would push people to stop recording manual gates. A
                // walkthrough test is proved by a run, which records it as any automated test, but
                // its name is a sentence, not a symbol in the repository (FTR-037).
                if name.is_empty() || is_unsearchable(&t.kind) {
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

/// The largest JUnit report [`parse_junit`] reads.
pub const JUNIT_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Test name -> result and duration, read from a cargo-nextest JUnit report (FTR-058 R-7). A
/// reader bounded to nextest's form, not a general XML parser: it reads every `<testsuite>` and,
/// inside it, every `<testcase name classname time>`. A testcase is red when a `<failure` or
/// `<error` child opens before its `</testcase>`, left out when a `<skipped` child is there, and
/// green otherwise; `ms` is `time` × 1000, rounded. Comments, CDATA and processing instructions
/// are stepped over. Only the five predefined entities and character references are decoded; a
/// report holding `<!DOCTYPE` or `<!ENTITY` is refused, so nothing is ever expanded (R-12).
pub fn parse_junit(xml: &str) -> Result<BTreeMap<String, Timed>, String> {
    if xml.len() > JUNIT_MAX_BYTES {
        return Err(format!(
            "a JUnit report over {} MiB is not read",
            JUNIT_MAX_BYTES / (1024 * 1024)
        ));
    }
    let upper = xml.to_ascii_uppercase();
    if upper.contains("<!DOCTYPE") || upper.contains("<!ENTITY") {
        return Err("a JUnit report with a DOCTYPE or entity declaration is not read".into());
    }
    struct Case {
        name: String,
        ms: Option<u64>,
        failed: bool,
        skipped: bool,
    }
    fn finish(out: &mut BTreeMap<String, Timed>, case: Case) {
        let outcome = if case.failed {
            Outcome::Failed
        } else if case.skipped {
            Outcome::Unproven
        } else {
            Outcome::Passed
        };
        record_outcome(
            out,
            Reading {
                name: case.name,
                outcome,
                ms: case.ms,
            },
        );
    }
    let mut out = BTreeMap::new();
    let mut suite: Option<String> = None;
    let mut case: Option<Case> = None;
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        rest = &rest[open..];
        // Markup whose content is never read.
        let skip = [("<!--", "-->"), ("<![CDATA[", "]]>"), ("<?", "?>")]
            .into_iter()
            .find(|(start, _)| rest.starts_with(start));
        if let Some((start, end)) = skip {
            let close = rest[start.len()..]
                .find(end)
                .ok_or_else(|| format!("the JUnit report ends inside a '{start}'"))?;
            rest = &rest[start.len() + close + end.len()..];
            continue;
        }
        let (tag, after) = junit_tag(rest)?;
        rest = after;
        match (tag.closing, tag.name) {
            (false, "testsuite") if !tag.empty => suite = tag.attr("name")?,
            (true, "testsuite") => suite = None,
            (false, "testcase") => {
                if let Some(c) = case.take() {
                    return Err(format!("testcase '{}' is not closed", c.name));
                }
                let name = tag
                    .attr("name")?
                    .ok_or("a testcase has no name attribute")?;
                let class = match tag.attr("classname")? {
                    Some(c) => c,
                    None => suite.clone().unwrap_or_default(),
                };
                let ms = match tag.attr("time")? {
                    Some(t) => Some(junit_ms(&t).ok_or_else(|| {
                        format!("testcase '{name}' has time=\"{t}\", which is not a duration")
                    })?),
                    None => None,
                };
                let c = Case {
                    name: board_name(&class, &name),
                    ms,
                    failed: false,
                    skipped: false,
                };
                if tag.empty {
                    finish(&mut out, c);
                } else {
                    case = Some(c);
                }
            }
            (true, "testcase") => {
                let c = case.take().ok_or("a </testcase> closes no testcase")?;
                finish(&mut out, c);
            }
            (false, "failure" | "error") => {
                if let Some(c) = case.as_mut() {
                    c.failed = true;
                }
            }
            (false, "skipped") => {
                if let Some(c) = case.as_mut() {
                    c.skipped = true;
                }
            }
            _ => {}
        }
    }
    if let Some(c) = case {
        return Err(format!("testcase '{}' is not closed", c.name));
    }
    Ok(out)
}

/// One tag of a JUnit report: its name, whether it closes (`</x>`) or is empty (`<x/>`), and its
/// raw attribute text.
struct JunitTag<'a> {
    name: &'a str,
    closing: bool,
    empty: bool,
    attrs: &'a str,
}

impl JunitTag<'_> {
    /// The decoded value of attribute `key`, if the tag has it.
    fn attr(&self, key: &str) -> Result<Option<String>, String> {
        let mut s = self.attrs;
        loop {
            s = s.trim_start();
            if s.is_empty() {
                return Ok(None);
            }
            let eq = s
                .find('=')
                .ok_or_else(|| format!("an attribute of <{}> has no value", self.name))?;
            let k = s[..eq].trim();
            let v = s[eq + 1..].trim_start();
            let quote = v
                .chars()
                .next()
                .filter(|c| *c == '"' || *c == '\'')
                .ok_or_else(|| format!("attribute '{k}' of <{}> is not quoted", self.name))?;
            let end = v[1..]
                .find(quote)
                .ok_or_else(|| format!("attribute '{k}' of <{}> is not closed", self.name))?;
            if k == key {
                return junit_decode(&v[1..1 + end]).map(Some);
            }
            s = &v[1 + end + 1..];
        }
    }
}

/// Read the tag `text` starts with (at its `<`), returning it and the text after its `>`. A `>`
/// inside a quoted attribute value does not end the tag.
fn junit_tag(text: &str) -> Result<(JunitTag<'_>, &str), String> {
    let mut quote: Option<char> = None;
    let mut end = None;
    for (i, c) in text.char_indices().skip(1) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => {
                end = Some(i);
                break;
            }
            (None, '<') => return Err("the JUnit report has a '<' inside a tag".into()),
            _ => {}
        }
    }
    let end = end.ok_or("the JUnit report ends inside a tag")?;
    let mut inner = &text[1..end];
    let closing = inner.starts_with('/');
    if closing {
        inner = &inner[1..];
    }
    let empty = inner.ends_with('/');
    if empty {
        inner = &inner[..inner.len() - 1];
    }
    let split = inner
        .find(|c: char| c.is_whitespace())
        .unwrap_or(inner.len());
    let tag = JunitTag {
        name: &inner[..split],
        closing,
        empty,
        attrs: &inner[split..],
    };
    Ok((tag, &text[end + 1..]))
}

/// An attribute value with `&amp; &lt; &gt; &quot; &apos;` and `&#N;`/`&#xH;` decoded. Any other
/// entity is refused: no entity is ever looked up or expanded.
fn junit_decode(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let semi = rest[amp..]
            .find(';')
            .filter(|n| *n <= 12)
            .ok_or_else(|| format!("'{raw}' has an '&' that starts no entity"))?;
        let entity = &rest[amp + 1..amp + semi];
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                {
                    u32::from_str_radix(hex, 16).ok()
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse().ok()
                } else {
                    None
                };
                code.and_then(char::from_u32).ok_or_else(|| {
                    format!("'&{entity};' is not an entity a JUnit report is read with")
                })?
            }
        };
        out.push(c);
        rest = &rest[amp + semi + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// A JUnit `time` in seconds as whole milliseconds, rounded.
fn junit_ms(time: &str) -> Option<u64> {
    let secs: f64 = time.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| (secs * 1000.0).round() as u64)
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

    /// The real `python3 -m unittest -v` run of the DEF-068 fixture class (Python 3.14.6, 15 Nov
    /// 2026, stderr), verbatim: the header ending `... ` and the subtest line's two spaces kept.
    const UNITTEST_3_14: &str = "\
test_add (tests.test_basket.BasketTests.test_add) ... ok
test_doc (tests.test_basket.BasketTests.test_doc)
A basket keeps its items. ... ok
test_error (tests.test_basket.BasketTests.test_error) ... ERROR
test_fail (tests.test_basket.BasketTests.test_fail) ... FAIL
test_prints (tests.test_basket.BasketTests.test_prints) ... ok
test_skip (tests.test_basket.BasketTests.test_skip) ... skipped 'not yet'
test_sub (tests.test_basket.BasketTests.test_sub) ...\x20
\x20 test_sub (tests.test_basket.BasketTests.test_sub) (i=1) ... FAIL
test_xfail (tests.test_basket.BasketTests.test_xfail) ... expected failure
test_xpass (tests.test_basket.BasketTests.test_xpass) ... unexpected success

======================================================================
ERROR: test_error (tests.test_basket.BasketTests.test_error)
FAIL: test_sub (tests.test_basket.BasketTests.test_sub) (i=1)
UNEXPECTED SUCCESS: test_xpass (tests.test_basket.BasketTests.test_xpass)
Ran 9 tests in 0.002s
FAILED (failures=2, errors=1, skipped=1, expected failures=1, unexpected successes=1)";

    #[test]
    fn unittest_verbose_lines_are_read_as_dotted_names() {
        assert!(UNITTEST_3_14.contains("test_sub) ... \n  test_sub"));
        let r = parse_results(UNITTEST_3_14);
        let at = |m: &str| {
            r.get(&format!("tests.test_basket.BasketTests.{m}"))
                .copied()
        };
        for green in ["test_add", "test_doc", "test_prints"] {
            assert_eq!(at(green), Some(true), "{green}: {r:?}");
        }
        for red in ["test_error", "test_fail", "test_xpass", "test_sub"] {
            assert_eq!(at(red), Some(false), "{red}: {r:?}");
        }
        assert_eq!(at("test_skip"), None);
        assert_eq!(at("test_xfail"), None);
        assert_eq!(r.len(), 7, "the summary lines add nothing: {r:?}");
        let one = |t: &str| parse_results(t);
        // Python 3.10 and earlier print the class only.
        let r = one("test_add (tests.test_basket.BasketTests) ... ok");
        assert_eq!(r.get("tests.test_basket.BasketTests.test_add"), Some(&true));
        assert_eq!(r.len(), 1);
        // A method named `test` is unittest's, not a cargo name.
        let r = one("test (m.C.test) ... ok");
        assert_eq!(r.get("m.C.test"), Some(&true));
        assert_eq!(r.len(), 1, "{r:?}");
        // Output written between the header and the verdict: nothing, and the stray verdict neither.
        assert!(one("test_p (m.C.test_p) ... hello\nok").is_empty());
        // A head followed by a non-verdict line is dropped; the cargo line after reads as usual.
        let r = one("test_doc (m.C.test_doc)\nnot a verdict\ntest cart::x ... ok");
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(r.get("cart::x"), Some(&true));
        // The documented cost of the two-line form.
        let r = one("print (a.b)\nx ... ok");
        assert_eq!(r.get("a.b.print"), Some(&true));
        assert_eq!(r.len(), 1);
        // cargo's lines, a doctest's included, are read as today.
        assert_eq!(one("test cart::x ... ok").get("cart::x"), Some(&true));
        let r = one("test src/lib.rs - f (line 3) ... ok");
        assert_eq!(r.get("src/lib.rs - f (line 3)"), Some(&true));
        assert_eq!(r.len(), 1);
        // A long transcript of these lines parses to the same map.
        let big = format!("{UNITTEST_3_14}\n").repeat(560);
        assert!(big.lines().count() >= 10_000);
        let t = std::time::Instant::now();
        let r = parse_results(&big);
        eprintln!("10k-line unittest parse: {:?}", t.elapsed());
        assert_eq!(r, parse_results(UNITTEST_3_14));
    }

    /// cargo-nextest 0.9.151's lines from the 16 Nov 2026 scratch-crate probe, verbatim (FTR-058).
    const NEXTEST_0_9_151: &str = "\
PASS [   0.006s] (2/4) nxprobe::cli_flow integration_ok
FAIL [   0.004s] (1/4) nxprobe tests::fails
SLOW [>  1.000s] (───) nxprobe tests::slowish";

    /// The probe's lines among the other forms nextest prints, indented as nextest indents them: a
    /// kanbanr integration test, unit test and binary test, then FLAKY, LEAK, LEAK-FAIL, TIMEOUT,
    /// SIGSEGV, TRY and SLOW lines, the captured libtest line of the failure and the summary that
    /// repeats its FAIL line.
    fn nextest_transcript() -> String {
        format!(
            "{NEXTEST_0_9_151}
    Starting 14 tests across 4 binaries
        PASS [   0.012s] (3/14) kanbanr-cli::local_mode capture_records_x
        PASS [   2.504s] (4/14) kanbanr-core evidence::tests::y
        PASS [   0.020s] (5/14) kanbanr-cli::bin/kanbanr tests::z
       FLAKY 2/3 [   0.100s] (6/14) kanbanr-core flaky::a
        LEAK [   0.030s] (7/14) kanbanr-core leak::a
   LEAK-FAIL [   0.031s] (8/14) kanbanr-core leakfail::a
     TIMEOUT [  60.000s] (9/14) kanbanr-core timeout::a
     SIGSEGV [   0.001s] (10/14) kanbanr-core seg::a
  TRY 2 PASS [   0.050s] (11/14) kanbanr-core retried::a
  TRY 1 FAIL [   0.040s] (12/14) kanbanr-core retried::b
        SLOW [>  1.000s] (───) kanbanr-core slow::a
        PASS [   1.234s] (13/14) nxprobe tests::slowish
        PASS [   0.007s] nxprobe tests::uncounted
--- STDOUT:              nxprobe tests::fails ---
test tests::fails ... FAILED
------------
     Summary [   3.000s] 14 tests run: 10 passed, 4 failed, 0 skipped
        FAIL [   0.004s] (1/4) nxprobe tests::fails"
        )
    }

    /// FTR-058 R-1 (MNT-007's old R-4 name): nextest's lines are read, and each name is the
    /// board's: an integration test binary's file joins the test's name, a library's or a
    /// `bin/` binary's test keeps the name nextest prints.
    #[test]
    fn nextest_output_is_read_as_the_boards_test_names() {
        let r = parse_results(&nextest_transcript());
        for green in [
            "cli_flow::integration_ok",
            "local_mode::capture_records_x",
            "evidence::tests::y",
            "tests::z",
            "flaky::a",
            "leak::a",
            "tests::slowish",
            "tests::uncounted",
        ] {
            assert_eq!(r.get(green), Some(&true), "{green}: {r:?}");
        }
        for red in ["tests::fails", "leakfail::a", "timeout::a", "seg::a"] {
            assert_eq!(r.get(red), Some(&false), "{red}: {r:?}");
        }
        // SLOW, TRY and summary lines add nothing.
        for none in ["retried::a", "retried::b", "slow::a"] {
            assert_eq!(r.get(none), None, "{none}: {r:?}");
        }
        assert_eq!(r.len(), 12, "{r:?}");
        assert!(
            r.keys()
                .all(|k| !k.starts_with('[') && !k.contains("Summary")),
            "no nextest line is read as a marked one: {r:?}"
        );
        // The probe alone: the SLOW line of a test with no later result proves nothing.
        let r = parse_results(NEXTEST_0_9_151);
        assert_eq!(r.get("cli_flow::integration_ok"), Some(&true));
        assert_eq!(r.get("tests::fails"), Some(&false));
        assert_eq!(r.len(), 2, "{r:?}");
        // The summary's repeated FAIL keeps a test red, whatever came before.
        let r = parse_results(
            "PASS [   0.004s] (1/1) nxprobe tests::fails\nFAIL [   0.004s] (1/4) nxprobe tests::fails",
        );
        assert_eq!(r.get("tests::fails"), Some(&false));
        // read_marked no longer owns a `PASS [` line.
        assert!(read_marked("PASS [   0.006s] (2/4) nxprobe tests::x").is_none());
        assert!(read_marked("FAIL [   0.006s] (2/4) nxprobe tests::x").is_none());
        assert!(read_marked("PASS src/cart.test.ts").is_some());
        // Not nextest lines: an unknown status word, a malformed duration.
        assert!(parse_results("NOPE [   0.006s] (2/4) nxprobe tests::x").is_empty());
        assert!(read_nextest("PASS [ abc ] (2/4) nxprobe tests::x").is_none());
        assert!(read_nextest("PASS [   0.006s] (2/4) nxprobe").is_none());
        assert_eq!(
            board_name("kanbanr-cli::local_mode", "capture_x"),
            "local_mode::capture_x"
        );
        assert_eq!(
            board_name("kanbanr-core", "evidence::tests::y"),
            "evidence::tests::y"
        );
        assert_eq!(
            board_name("kanbanr-cli::bin/kanbanr", "tests::z"),
            "tests::z"
        );
    }

    /// FTR-058 R-1: nextest's durations are kept in whole milliseconds; a SLOW or TRY line keeps
    /// none, and a name read twice keeps the last duration seen.
    #[test]
    fn nextest_lines_keep_their_durations() {
        let r = parse_timed_results(&nextest_transcript());
        let ms = |n: &str| r.get(n).and_then(|t| t.ms);
        assert_eq!(ms("cli_flow::integration_ok"), Some(6));
        assert_eq!(ms("evidence::tests::y"), Some(2504));
        assert_eq!(ms("local_mode::capture_records_x"), Some(12));
        assert_eq!(ms("tests::z"), Some(20));
        assert_eq!(ms("flaky::a"), Some(100));
        assert_eq!(ms("leak::a"), Some(30));
        assert_eq!(ms("leakfail::a"), Some(31));
        assert_eq!(ms("timeout::a"), Some(60_000));
        assert_eq!(ms("seg::a"), Some(1));
        assert_eq!(ms("tests::uncounted"), Some(7));
        // SLOW then PASS: the PASS line's time.
        assert_eq!(ms("tests::slowish"), Some(1234));
        // FAIL, the captured libtest line (no time) and the summary's FAIL: still 4 ms, still red.
        assert_eq!(
            r.get("tests::fails"),
            Some(&Timed {
                passed: false,
                ms: Some(4)
            })
        );
        assert_eq!(r.get("slow::a"), None);
        assert_eq!(r.get("retried::a"), None);
    }

    /// FTR-058 R-2: Vitest 5's and Playwright's durations are kept; a line with none keeps none.
    #[test]
    fn vitest_and_playwright_lines_keep_their_durations() {
        let r = parse_timed_results(VITEST_5_1);
        let ms = |r: &BTreeMap<String, Timed>, n: &str| r.get(n).and_then(|t| t.ms);
        for (name, want) in [
            ("src/App.test.tsx > title shows the basket item count", 41),
            (
                "src/App.test.tsx > title updates when the basket changes",
                5,
            ),
            (
                "src/components/BasketView.test.tsx > changes a line's quantity",
                67,
            ),
            ("src/components/BasketView.test.tsx > removes a line", 12),
            ("src/components/BasketView.test.tsx > shows the total", 9),
            ("src/money.test.ts > formats paise as rupees", 2),
            ("src/money.test.ts > rounds half up", 1),
            ("src/money.test.ts > formats zero", 3),
            ("src/money.test.ts > rejects negatives", 4),
            ("built in", 163),
        ] {
            assert_eq!(ms(&r, name), Some(want), "{name}: {r:?}");
        }
        let one = |t: &str| parse_timed_results(t);
        let r = one(" \u{2713} src/a.test.ts > flaky 12ms (retry x1)");
        assert_eq!(ms(&r, "src/a.test.ts > flaky"), Some(12));
        let r = one(" \u{2713} src/a.test.ts > big 13ms 34 MB heap used");
        assert_eq!(ms(&r, "src/a.test.ts > big"), Some(13));
        let r = one("\u{2713} cart retains items for 7 days");
        assert_eq!(
            r.get("cart retains items for 7 days"),
            Some(&Timed {
                passed: true,
                ms: None
            })
        );
        let r = one(PLAYWRIGHT_1_65);
        assert_eq!(r.len(), 2);
        assert!(r.values().all(|t| t.ms == Some(701)), "{r:?}");
        let r = one(
            "  \u{2713}  4 [firefox] \u{203a} e2e/basket.spec.ts:10:1 \u{203a} removes a line (retry #1) (1.0s)",
        );
        assert_eq!(ms(&r, "removes a line"), Some(1000));
        let r = one("  \u{2713}  5 e2e/basket.spec.ts:30:1 \u{203a} no project (1.2m)");
        assert_eq!(ms(&r, "no project"), Some(72_000));
        let r = one("  \u{2713}  6 e2e/basket.spec.ts:30:1 \u{203a} long (1.5h)");
        assert_eq!(ms(&r, "long"), Some(5_400_000));
        let r = one("  \u{2713}  7 e2e/basket.spec.ts:30:1 \u{203a} quick (2.3s)");
        assert_eq!(ms(&r, "quick"), Some(2300));
        let r = one("  \u{2713}  8 e2e/basket.spec.ts:30:1 \u{203a} no time");
        assert_eq!(
            r.get("no time"),
            Some(&Timed {
                passed: true,
                ms: None
            })
        );
    }

    /// FTR-058 R-3 (MNT-007's old R-4 name): cargo test's integration test results are named as
    /// the board names them, `<file>::<test>`, from the `Running tests/<file>.rs` line until the
    /// next target; a unit test, a nested module test and a doctest keep the name cargo prints.
    #[test]
    fn cargo_test_integration_output_is_read_as_the_boards_test_names() {
        let text = "\
     Running unittests src/lib.rs (target/debug/deps/kanbanr_core-1a2b3c)
test evidence::tests::y ... ok
     Running tests/local_mode.rs (target/debug/deps/local_mode-4d5e6f)
test capture_x ... ok
test capture_fails ... FAILED
test nested::case ... ok
     Running unittests src/main.rs (target/debug/deps/kanbanr-7a8b9c)
test plain ... ok
     Running tests/view_daemon.rs (target/debug/deps/view_daemon-0d1e2f)
test routes_answer ... ok
   Doc-tests kanbanr_core
test src/lib.rs - f (line 3) ... ok";
        let r = parse_results(text);
        assert_eq!(r.get("evidence::tests::y"), Some(&true), "{r:?}");
        assert_eq!(r.get("local_mode::capture_x"), Some(&true), "{r:?}");
        assert_eq!(r.get("local_mode::capture_fails"), Some(&false), "{r:?}");
        assert_eq!(r.get("nested::case"), Some(&true), "{r:?}");
        assert_eq!(r.get("plain"), Some(&true), "{r:?}");
        assert_eq!(r.get("view_daemon::routes_answer"), Some(&true), "{r:?}");
        assert_eq!(r.get("src/lib.rs - f (line 3)"), Some(&true), "{r:?}");
        assert_eq!(r.len(), 7, "the Running lines add nothing: {r:?}");
    }

    /// FTR-058 R-5: a run whose runner prints no duration records its results and no timing.
    #[test]
    fn a_run_without_durations_records_results_and_no_timing() {
        let text = "\
running 3 tests
test hooks::tests::install_merges ... ok
test report::tests::cycle_time ... FAILED
test slow::case ... ignored
tests/x.py::test_p PASSED [ 50%]
test_add (tests.test_basket.BasketTests.test_add) ... ok";
        let r = parse_timed_results(text);
        assert_eq!(r.len(), 4, "{r:?}");
        assert!(r.values().all(|t| t.ms.is_none()), "{r:?}");
        assert!(r["hooks::tests::install_merges"].passed);
        assert!(!r["report::tests::cycle_time"].passed);
        assert!(r["tests/x.py::test_p"].passed);
        assert!(r["tests.test_basket.BasketTests.test_add"].passed);
        assert_eq!(parse_results(text).len(), 4);
    }

    #[test]
    fn a_test_run_capture_cannot_read_is_named() {
        let words = |c: &str| -> Vec<String> { c.split(' ').map(String::from).collect() };
        let label = |c: &str| test_runner(&words(c));
        for (cmd, want) in [
            ("python3 -m unittest", "python3 -m unittest"),
            ("/usr/bin/python3.12 -m pytest -q", "python3.12 -m pytest"),
            ("uv run pytest", "pytest"),
            ("npx vitest run", "vitest"),
            ("npm run test:unit", "npm run test:unit"),
            ("cargo nextest run", "cargo nextest"),
            ("go test ./...", "go test"),
            ("./gradlew check", "gradlew check"),
            ("make test", "make test"),
        ] {
            assert_eq!(label(cmd).as_deref(), Some(want), "{cmd}");
        }
        // A quoted message is one word, as the shell splits it.
        let commit: Vec<String> = ["git", "commit", "-m", "run pytest"]
            .map(String::from)
            .to_vec();
        assert_eq!(test_runner(&commit), None);
        for cmd in [
            "python3 script.py",
            "cargo build",
            "npm install",
            "echo go test",
        ] {
            assert_eq!(label(cmd), None, "{cmd}");
        }
        assert_eq!(
            unread_run_notice(&[words("python3 -m unittest")], 1, None),
            None
        );
        assert_eq!(unread_run_notice(&[words("ls")], 0, None), None);
        let notice = unread_run_notice(&[words("cd x"), words("python3 -m unittest")], 0, None)
            .expect("a test run that recorded nothing is named");
        assert!(notice.contains("`python3 -m unittest`"), "{notice}");
        assert!(notice.contains("python -m unittest -v"), "{notice}");
        assert!(notice.contains(READABLE_RUNNERS), "{notice}");
    }

    #[test]
    fn test_runner_sees_through_known_wrappers() {
        let words = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        for cmd in [
            "rtk cargo test",
            "rtk proxy cargo test",
            "env RUST_LOG=x cargo test",
        ] {
            assert_eq!(
                test_runner(&words(cmd)).as_deref(),
                Some("cargo test"),
                "{cmd}"
            );
        }
    }

    #[test]
    fn a_condensed_test_run_names_the_wrapper_that_condensed_it() {
        let words = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        let run = [words("cargo test")];
        let unread = unread_run_notice(&run, 0, None).expect("DEF-068's notice");
        assert!(unread.contains("read no test result"), "{unread}");

        let both = unread_run_notice(&run, 0, Some("rtk")).expect("nothing read through rtk");
        assert!(both.starts_with(&unread), "{both}");
        assert!(both.contains("ran through `rtk`"), "{both}");
        assert!(both.contains("`rtk proxy <command>`"), "{both}");
        assert!(both.contains("only 0 result(s)"), "{both}");

        let some = unread_run_notice(&run, 3, Some("rtk")).expect("some read through rtk");
        assert!(!some.contains("read no test result"), "{some}");
        assert!(some.contains("ran through `rtk`"), "{some}");
        assert!(some.contains("only 3 result(s)"), "{some}");
        assert!(some.contains("`rtk proxy <command>`"), "{some}");

        assert_eq!(unread_run_notice(&run, 3, None), None);
        assert_eq!(unread_run_notice(&[words("ls")], 0, Some("rtk")), None);
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
        let results = parse_timed_results("test a::passes ... ok\ntest b::fails ... FAILED\n");
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
        let results =
            parse_timed_results("test manual: looks right ... ok\ntest a::passes ... ok\n");
        assert_eq!(
            results.get("manual: looks right").map(|t| t.passed),
            Some(true)
        );
        let ops = plan_capture(&p, &results, Some("abc"));
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert_eq!(ops[0]["test"], "a::passes");
    }

    /// FTR-037 R-6: a `walkthrough` test is automated evidence: a run's results record it as any
    /// unit test's (unlike a manual one), it is never searched for in the repository nor set back to
    /// planned for not being there, and it is not a manual check for the person.
    #[test]
    fn a_walkthrough_test_is_automated_evidence_not_a_manual_check() {
        let p = project_with_tests(
            "      - name: 'walkthrough: the togaf runbook completes with zero stops'\n\
             \x20       kind: walkthrough\n        state: green\n        checked_rev: abc\n\
             \x20     - name: 'manual: looks right'\n        kind: manual\n",
        );
        // A run that records it: captured, where a manual result never is.
        let results = parse_timed_results(
            "✓ walkthrough: the togaf runbook completes with zero stops\n✓ manual: looks right\n",
        );
        let ops = plan_capture(&p, &results, Some("def"));
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert_eq!(
            ops[0]["test"],
            "walkthrough: the togaf runbook completes with zero stops"
        );
        // Not in the repository, and not stale for it.
        let plan = plan_stale_tests(&p, &|_| false);
        assert!(plan.rows.is_empty(), "{:?}", plan.rows);
        assert!(plan.ops.is_empty());
        assert!(is_unsearchable("walkthrough") && is_unsearchable("manual"));
        assert!(!is_unsearchable("unit"));
        // Not a person's check: the open manual tests name only the manual one.
        let open = crate::readiness::open_manual_tests(&p.features[0]);
        assert_eq!(open, ["R-1: manual: looks right"]);
        assert!(WALKTHROUGH_HINT.contains("kanbanr walkthrough run"));
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

    /// cargo-nextest 0.9.151's JUnit report for the probe crate (16 Nov 2026): two suites, a
    /// failure whose message holds `&apos;`, captured output, and an empty testcase.
    const NEXTEST_JUNIT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="4" failures="1" errors="0" uuid="5b0b6a8e-1f63-4f4e-9d0e-7f1c1c7f9a10" timestamp="2026-11-16T17:20:31.123+00:00" time="1.012">
    <testsuite name="nxprobe" tests="3" disabled="0" errors="0" failures="1">
        <testcase name="tests::fails" classname="nxprobe" timestamp="2026-11-16T17:20:31.125+00:00" time="0.004">
            <failure message="thread &apos;tests::fails&apos; panicked at src/lib.rs:9:9" type="test failure with exit code 101">thread &apos;tests::fails&apos; panicked at src/lib.rs:9:9:
assertion `left == right` failed</failure>
            <system-out>
running 1 test
test tests::fails ... FAILED
</system-out>
            <system-err><![CDATA[<testcase name="not_a_test" classname="nxprobe"/> & stray text]]></system-err>
        </testcase>
        <testcase name="tests::passes" classname="nxprobe" timestamp="2026-11-16T17:20:31.125+00:00" time="0.002">
        </testcase>
        <!-- a comment holding <testcase name="nor_this"/> -->
        <testcase name="tests::slowish" classname="nxprobe" timestamp="2026-11-16T17:20:31.126+00:00" time="1.204"/>
    </testsuite>
    <testsuite name="nxprobe::cli_flow" tests="1" disabled="0" errors="0" failures="0">
        <testcase name="integration_ok" classname="nxprobe::cli_flow" timestamp="2026-11-16T17:20:31.127+00:00" time="0.006">
        </testcase>
    </testsuite>
</testsuites>
"#;

    /// FTR-058 R-7: a nextest JUnit report gives the board's names, each test's result and its
    /// duration; a skipped test is left out and an errored one is red.
    #[test]
    fn a_nextest_junit_report_is_read_with_names_results_and_durations() {
        let r = parse_junit(NEXTEST_JUNIT).unwrap();
        let t = |passed, ms| Timed {
            passed,
            ms: Some(ms),
        };
        assert_eq!(r.get("tests::fails"), Some(&t(false, 4)));
        assert_eq!(r.get("tests::passes"), Some(&t(true, 2)));
        assert_eq!(r.get("tests::slowish"), Some(&t(true, 1204)));
        // An integration binary's id names its file, as `board_name` says.
        assert_eq!(r.get("cli_flow::integration_ok"), Some(&t(true, 6)));
        // Nothing inside CDATA or a comment is read as a testcase.
        assert_eq!(r.len(), 4, "{r:?}");

        let skipped = r#"<testsuites><testsuite name="kanbanr-core">
<testcase name="evidence::tests::a &amp; b" classname="kanbanr-core" time="0.010"><skipped/></testcase>
<testcase name="evidence::tests::errs" classname="kanbanr-core" time="0.5"><error message="&lt;crash&gt;"/></testcase>
<testcase name="evidence::tests::untimed" classname="kanbanr-core"/>
<testcase name='local&#95;mode&#x3a;&#x3A;quoted' classname='kanbanr-cli::bin/kanbanr' time='0.0004'></testcase>
</testsuite></testsuites>"#;
        let r = parse_junit(skipped).unwrap();
        assert_eq!(r.get("evidence::tests::a & b"), None);
        assert_eq!(r.get("evidence::tests::errs"), Some(&t(false, 500)));
        assert_eq!(
            r.get("evidence::tests::untimed"),
            Some(&Timed {
                passed: true,
                ms: None
            })
        );
        // A binary's id (`…::bin/…`) leaves the name as printed.
        assert_eq!(r.get("local_mode::quoted"), Some(&t(true, 0)));
        assert_eq!(r.len(), 3, "{r:?}");

        // A report that is not one is an error, never a guess.
        assert!(parse_junit("<testsuites><testsuite name=\"x\"><testcase name=\"a\"").is_err());
        assert!(parse_junit("<testcase name=\"a\" time=\"soon\"/>").is_err());
        assert!(parse_junit("<testcase name=\"a&bogus;\"/>").is_err());
        assert!(parse_junit("<testsuite name=\"x\"><testcase name=\"a\"></testsuite>").is_err());
        assert!(parse_junit("").unwrap().is_empty());
    }

    /// FTR-058 R-12: a report with a DOCTYPE or an entity declaration is refused, nothing is
    /// expanded, an oversized one is refused, and no XML crate is in the lockfile.
    #[test]
    fn a_junit_report_with_a_doctype_or_entity_is_refused() {
        let want = "a JUnit report with a DOCTYPE or entity declaration is not read";
        let doctype = "<!DOCTYPE testsuites>\n<testsuites><testsuite name=\"x\">\
                       <testcase name=\"a\" classname=\"x\" time=\"0.1\"/></testsuite></testsuites>";
        assert_eq!(parse_junit(doctype), Err(want.to_string()));
        let entity = "<?xml version=\"1.0\"?>\n<testsuites>\n\
                      <!ENTITY x SYSTEM \"file:///etc/passwd\">\n\
                      <testsuite name=\"x\"><testcase name=\"&x;\" classname=\"x\" time=\"0.1\"/>\
                      </testsuite></testsuites>";
        assert_eq!(parse_junit(entity), Err(want.to_string()));
        assert!(parse_junit("<!doctype x>").is_err());

        let big = " ".repeat(JUNIT_MAX_BYTES + 1);
        assert!(parse_junit(&big).unwrap_err().contains("64 MiB"));

        let lock = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"),
        )
        .unwrap();
        let xml: Vec<&str> = lock
            .lines()
            .filter_map(|l| l.strip_prefix("name = \""))
            .filter_map(|n| n.strip_suffix('"'))
            .filter(|n| n.contains("xml"))
            .collect();
        assert!(xml.is_empty(), "the lockfile holds XML crates: {xml:?}");
    }

    /// A test locator over an in-memory tree, matching as `GitLocator`'s patterns do: a Rust or
    /// Python test by `(^|[^A-Za-z0-9_])(fn|def)[[:space:]]+NAME[^A-Za-z0-9_]` on one line, a
    /// monitor check or a `Literal` by its exact text in a script that prints it (under
    /// `scripts/` or `web/scripts/`), a path by the file.
    struct TreeLocator {
        files: Vec<(&'static str, &'static str)>,
    }

    fn defines(line: &str, name: &str) -> bool {
        let word = |c: char| c == '_' || c.is_ascii_alphanumeric();
        line.match_indices(name).any(|(at, _)| {
            let after = line[at + name.len()..].chars().next();
            if !after.is_some_and(|c| !word(c)) {
                return false;
            }
            let before = &line[..at];
            let keyword = before.trim_end_matches(char::is_whitespace);
            if keyword.len() == before.len() {
                return false;
            }
            ["fn", "def"].iter().any(|k| {
                keyword
                    .strip_suffix(k)
                    .is_some_and(|rest| !rest.ends_with(word))
            })
        })
    }

    impl Locator for TreeLocator {
        fn find(&self, kind: &TestKind, _rev: Option<&str>) -> Vec<Location> {
            let mut out = Vec::new();
            for (path, text) in &self.files {
                for (i, line) in text.lines().enumerate() {
                    let hit = match kind {
                        TestKind::Rust { name, .. } => defines(line, name),
                        TestKind::Python { file, name, .. } => {
                            (!file.is_empty() || path.starts_with("scripts/tests/"))
                                && defines(line, name)
                        }
                        TestKind::Web { name } | TestKind::Literal { text: name } => {
                            (path.starts_with("scripts/") || path.starts_with("web/scripts/"))
                                && line.contains(name.as_str())
                        }
                        TestKind::File { path: p } => i == 0 && p == path,
                        TestKind::Manual | TestKind::Walkthrough => false,
                    };
                    if hit {
                        out.push(Location {
                            path: path.to_string(),
                            line: i as u32 + 1,
                        });
                    }
                }
            }
            out
        }
        fn revision(&self, _rev: &str) -> RevFacts {
            RevFacts::default()
        }
        fn head(&self) -> String {
            "head".into()
        }
    }

    #[test]
    fn a_test_name_is_located_by_its_kind_and_never_by_substring() {
        let rust = |modules: &[&str], name: &str| TestKind::Rust {
            modules: modules.iter().map(|m| m.to_string()).collect(),
            name: name.into(),
        };
        // One name of each kind.
        assert_eq!(
            classify("releases::tests::x", "unit"),
            rust(&["releases", "tests"], "x")
        );
        assert_eq!(
            classify("local_mode::b", "integration"),
            rust(&["local_mode"], "b")
        );
        assert_eq!(
            classify("tests_locates", "integration"),
            rust(&[], "tests_locates")
        );
        assert_eq!(
            classify("test_x.C.test_d", "unit"),
            TestKind::Python {
                file: "test_x.py".into(),
                class: "C".into(),
                name: "test_d".into()
            }
        );
        assert_eq!(
            classify("C.test_d", ""),
            TestKind::Python {
                file: String::new(),
                class: "C".into(),
                name: "test_d".into()
            }
        );
        assert_eq!(
            classify("web::y", ""),
            TestKind::Web {
                name: "web::y".into()
            }
        );
        assert_eq!(
            classify("web check: the board loads", ""),
            TestKind::Web {
                name: "web check: the board loads".into()
            }
        );
        assert_eq!(
            classify("src/z.test.ts", "unit"),
            TestKind::File {
                path: "src/z.test.ts".into()
            }
        );
        assert_eq!(
            classify("cargo deny check", "unit"),
            TestKind::Literal {
                text: "cargo deny check".into()
            }
        );
        assert_eq!(
            classify("manual: the person reads it", ""),
            TestKind::Manual
        );
        assert_eq!(classify("reads clearly", "manual"), TestKind::Manual);
        assert_eq!(classify("walkthrough: a run", ""), TestKind::Walkthrough);
        assert_eq!(classify("a run", "walkthrough"), TestKind::Walkthrough);
        // A character outside the safe set is never put in a pattern.
        assert!(matches!(
            classify("a::b(.*)", "unit"),
            TestKind::Literal { .. }
        ));
        assert!(matches!(
            classify("a::b c", "unit"),
            TestKind::Literal { .. }
        ));
        assert!(matches!(
            classify("Docs build", ""),
            TestKind::Literal { .. }
        ));

        // Where each kind may be defined.
        assert_eq!(
            expected_paths(&rust(&["releases", "tests"], "x")),
            ["releases.rs", "releases/mod.rs"]
        );
        assert_eq!(
            expected_paths(&rust(&["a", "b"], "x")),
            ["a/b.rs", "a/b/mod.rs"]
        );
        assert_eq!(
            expected_paths(&rust(&["local_mode"], "b")),
            ["local_mode.rs", "local_mode/mod.rs"]
        );
        assert_eq!(expected_paths(&rust(&[], "b")), ["tests/"]);
        assert_eq!(
            expected_paths(&classify("test_x.C.test_d", "")),
            ["scripts/tests/test_x.py", "test_x.py"]
        );
        assert_eq!(
            expected_paths(&classify("C.test_d", "")),
            ["scripts/tests/"]
        );
        assert_eq!(expected_paths(&classify("web::y", "")), ["web/scripts/"]);
        assert_eq!(
            expected_paths(&classify("src/z.test.ts", "")),
            ["src/z.test.ts"]
        );
        assert!(expected_paths(&classify("cargo deny check", "")).is_empty());
        assert!(expected_paths(&TestKind::Manual).is_empty());

        let tree = TreeLocator {
            files: vec![
                ("Makefile", "deny:\n\tcargo deny check\n"),
                (
                    "src/lib.rs",
                    "// a_renamed_test_old_name was renamed to a_renamed_test_new_name\n\
                     fn a_renamed_test_new_name() {}\n\
                     fn a_renamed_test_old_name_too() {}\n\
                     fn xa_renamed_test_old_name() {}\n",
                ),
                (
                    "src/other.rs",
                    "mod tests {\n    #[test]\n    fn x() {}\n}\n",
                ),
                (
                    "scripts/tests/test_x.py",
                    "class C:\n    def test_d(self):\n        pass\n",
                ),
                ("web/scripts/check-y.mjs", "console.log('ok web::y')\n"),
                ("src/z.test.ts", "test('z', () => {})\n"),
            ],
        };
        let at = |name: &str| tree.find(&classify(name, "unit"), None);

        // A command in a Makefile is not a definition, though its text is there.
        assert!(tree.files[0].1.contains("cargo deny check"));
        assert!(at("cargo deny check").is_empty());
        // The old name in a comment, or inside a longer name, is not the test.
        assert!(at("lib::tests::a_renamed_test_old_name").is_empty());
        assert_eq!(
            at("lib::tests::a_renamed_test_new_name"),
            [Location {
                path: "src/lib.rs".into(),
                line: 2
            }]
        );
        // Found, but under another path than the name gives: elsewhere.
        let kind = classify("releases::tests::x", "unit");
        let found = tree.find(&kind, None);
        assert_eq!(
            found,
            [Location {
                path: "src/other.rs".into(),
                line: 3
            }]
        );
        let path_ok = found.iter().any(|l| {
            expected_paths(&kind)
                .iter()
                .any(|p| l.path.ends_with(p.as_str()) || l.path.contains(p.as_str()))
        });
        assert!(
            !path_ok,
            "src/other.rs is not where releases::tests::x belongs"
        );
        // Python, a monitor check and a path are found where they belong.
        assert_eq!(at("test_x.C.test_d")[0].path, "scripts/tests/test_x.py");
        assert_eq!(at("C.test_d")[0].path, "scripts/tests/test_x.py");
        assert_eq!(at("web::y")[0].path, "web/scripts/check-y.mjs");
        assert_eq!(at("src/z.test.ts")[0].path, "src/z.test.ts");
        assert!(at("src/missing.test.ts").is_empty());
        // The default prime is a no-op.
        tree.prime(&[kind], None);
        assert_eq!(tree.head(), "head");
    }
}
