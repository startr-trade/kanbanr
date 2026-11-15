//! The tables (FTR-037 W7, R-4), derived from `raw/` alone: `decisions.tsv`, one row per decision
//! the board copy holds, and `metrics.md`, the run's numbers. Sorted, with no clock of its own, so a
//! second derive over the same `raw/` is byte for byte the first. The confusion and class columns
//! of SPK-002 need a person's judgement and are not derived here.

use super::guard::{RunDirs, SetupLog};
use super::truth::{TruthLine, Verdict};
use anyhow::{Context, Result};
use kanbanr_core::Store;
use std::collections::BTreeMap;
use std::path::Path;

/// One decision as the board copy records it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DecisionRow {
    /// When it was taken (RFC 3339, as the board has it).
    pub at: String,
    /// `approve`, `signoff`, `ratify`, `adr`, `promote`, `cut`, `gate-result`, `manual-test`.
    pub kind: String,
    /// The item, release or ADR.
    pub subject: String,
    /// The sign-off's name, the stage, the gate, the ADR's verdict.
    pub name: String,
    pub by: String,
    /// `monitor`, `terminal`, `conversation`, `delegated`, or `unknown`.
    pub origin: String,
    /// Who asked for it: `Claude (prepared <id>)` when a prepared record was confirmed for it, else
    /// `runbook`.
    pub asked_by: String,
    /// `monitor` when the monitor's log has the write, else the origin.
    pub where_taken: String,
    /// The truth check's verdict for the subject (`OK`, `MISMATCH`, `UNKNOWN`), or `-`.
    pub truth: String,
}

/// What a derive produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub decisions_tsv: String,
    pub metrics_md: String,
    pub rows: usize,
}

fn cell(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn read_or_empty(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The rows of a TSV with a header, as maps.
fn tsv_rows(text: &str) -> Vec<BTreeMap<String, String>> {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let keys: Vec<&str> = header.split('\t').collect();
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            keys.iter()
                .zip(l.split('\t').chain(std::iter::repeat("")))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .collect()
}

/// The decisions the board copy under `raw/board` holds for `project`, with the context the other
/// captures give each.
fn decisions(raw: &Path, project: &str) -> Result<Vec<DecisionRow>> {
    let board = raw.join("board");
    let mut rows = Vec::new();
    if !board.join("projects").join(project).exists() {
        return Ok(rows);
    }
    let store = Store::new(&board);
    let prepared = tsv_rows(&read_or_empty(&raw.join("prepared.tsv")));
    let monitor_log = read_or_empty(&raw.join("monitor.log"));
    let truth: Vec<TruthLine> = read_or_empty(&raw.join("truth.txt"))
        .lines()
        .filter_map(TruthLine::parse)
        .collect();
    let truth_of = |check: &str| -> String {
        truth
            .iter()
            .find(|l| l.check.ends_with(check))
            .map(|l| match l.verdict {
                Verdict::Ok => "OK",
                Verdict::Mismatch => "MISMATCH",
                Verdict::Unknown => "UNKNOWN",
            })
            .unwrap_or("-")
            .to_string()
    };
    let asked_by = |kind: &str, subject: &str| -> String {
        // A prepared record of the kind confirmed for this subject: Claude asked.
        prepared
            .iter()
            .find(|r| {
                r.get("kind").is_some_and(|k| {
                    let k = k.as_str();
                    match kind {
                        "signoff" => k == "signoff",
                        "ratify" => k == "ratification",
                        "promote" | "cut" => k == "promotion",
                        "gate-result" => k == "gate-result",
                        "manual-test" => k == "manual-test",
                        _ => false,
                    }
                }) && r
                    .get("id")
                    .is_some_and(|id| id.contains(subject) || !id.is_empty())
            })
            .map(|r| {
                format!(
                    "Claude (prepared {})",
                    r.get("id").cloned().unwrap_or_default()
                )
            })
            .unwrap_or_else(|| "runbook".to_string())
    };
    let where_taken = |origin: &str, route_part: &str| -> String {
        if monitor_log
            .lines()
            .any(|l| l.contains("/api/write/") && l.contains(route_part) && l.contains("-> 2"))
        {
            "monitor".to_string()
        } else if origin.is_empty() {
            "unknown".to_string()
        } else {
            origin.to_string()
        }
    };
    let origin_or_unknown = |o: &str| {
        if o.is_empty() {
            "unknown".to_string()
        } else {
            o.to_string()
        }
    };
    let p = store.load(project)?;
    for f in &p.features {
        let Some(def) = &f.definition else { continue };
        for a in &def.approvals {
            // The event says `approved`, `withdrawn` or `ratified`; the row names the decision.
            let kind = match a.verdict.as_str() {
                "approved" => "approve",
                "withdrawn" => "unapprove",
                "ratified" => "ratify",
                other => other,
            }
            .to_string();
            rows.push(DecisionRow {
                at: a.at.clone(),
                kind: kind.clone(),
                subject: f.code.clone(),
                name: String::new(),
                by: a.by.clone(),
                origin: origin_or_unknown(&a.origin),
                asked_by: asked_by(&kind, &f.code),
                where_taken: where_taken(&a.origin, &format!("/features/{}/{}", f.code, kind)),
                truth: "-".into(),
            });
        }
        for s in &def.signoffs {
            rows.push(DecisionRow {
                at: s.at.clone(),
                kind: "signoff".into(),
                subject: f.code.clone(),
                name: s.id.clone(),
                by: s.by.clone(),
                origin: origin_or_unknown(&s.origin),
                asked_by: asked_by("signoff", &f.code),
                where_taken: where_taken(
                    &s.origin,
                    &format!("/features/{}/signoff/{}", f.code, s.id),
                ),
                truth: "-".into(),
            });
        }
        for r in &def.requirements {
            for t in &r.tests {
                if t.kind.eq_ignore_ascii_case("manual") && !t.checked_rev.is_empty() {
                    // The manual result's record, when kept (DEF-040).
                    rows.push(DecisionRow {
                        at: t.checked_rev.clone(),
                        kind: "manual-test".into(),
                        subject: f.code.clone(),
                        name: format!("{} {}", r.id, t.name),
                        by: String::new(),
                        origin: "unknown".into(),
                        asked_by: asked_by("manual-test", &f.code),
                        where_taken: where_taken("", &format!("/features/{}/tests/", f.code)),
                        truth: "-".into(),
                    });
                }
            }
        }
    }
    if let Ok(results) = kanbanr_core::manual::results(&store, project, None) {
        // The kept results replace the placeholder rows above with who, when and where.
        rows.retain(|r| r.kind != "manual-test");
        for m in results {
            rows.push(DecisionRow {
                at: m.at.clone(),
                kind: "manual-test".into(),
                subject: m.code.clone(),
                name: format!("{} {} {}", m.requirement, m.test, m.state),
                by: m.by.clone(),
                origin: origin_or_unknown(&m.origin),
                asked_by: if m.prepared.is_empty() {
                    "runbook".into()
                } else {
                    format!("Claude (prepared {})", m.prepared)
                },
                where_taken: where_taken(&m.origin, &format!("/features/{}/tests/", m.code)),
                truth: "-".into(),
            });
        }
    }
    for adr in kanbanr_core::adr::list(&store, project).unwrap_or_default() {
        if matches!(adr.status.as_str(), "accepted" | "rejected") && !adr.decided.is_empty() {
            rows.push(DecisionRow {
                at: adr.decided.clone(),
                kind: "adr".into(),
                subject: adr.id.clone(),
                name: adr.status.clone(),
                by: adr.deciders.join(", "),
                origin: origin_or_unknown(&adr.decided_origin),
                asked_by: "runbook".into(),
                where_taken: where_taken(&adr.decided_origin, &format!("/adrs/{}/", adr.id)),
                truth: "-".into(),
            });
        }
    }
    for r in kanbanr_core::releases::load(&store, project).unwrap_or_default() {
        let last = r.stages.len();
        for (i, s) in r.stages.iter().enumerate() {
            let is_cut = i + 1 == last && !r.tag_commit.is_empty() && s.stage == r.stage;
            rows.push(DecisionRow {
                at: s.at.clone(),
                kind: if is_cut { "cut" } else { "promote" }.into(),
                subject: r.version.clone(),
                name: s.stage.clone(),
                by: s.by.clone(),
                origin: origin_or_unknown(&s.origin),
                asked_by: asked_by("promote", &r.version),
                where_taken: where_taken(&s.origin, &format!("/releases/{}/promote", r.version)),
                truth: truth_of(&format!("tag {}", r.version)),
            });
        }
    }
    for g in kanbanr_core::gates::results(&store, project, None).unwrap_or_default() {
        rows.push(DecisionRow {
            at: g.at.clone(),
            kind: "gate-result".into(),
            subject: g.commit.chars().take(7).collect(),
            name: format!("{} {}", g.name, if g.passed { "passed" } else { "failed" }),
            by: g.by.clone(),
            origin: origin_or_unknown(&g.origin),
            asked_by: asked_by("gate-result", &g.commit),
            where_taken: where_taken(&g.origin, "/gate-results"),
            truth: truth_of("ci "),
        });
    }
    rows.sort();
    Ok(rows)
}

fn decisions_tsv(rows: &[DecisionRow]) -> String {
    let mut out = String::from("at\tkind\tsubject\tname\tby\torigin\tasked_by\twhere\ttruth\n");
    for r in rows {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            cell(&r.at),
            cell(&r.kind),
            cell(&r.subject),
            cell(&r.name),
            cell(&r.by),
            cell(&r.origin),
            cell(&r.asked_by),
            cell(&r.where_taken),
            cell(&r.truth)
        ));
    }
    out
}

/// Seconds between two RFC 3339 times, when both parse.
fn seconds_between(a: &str, b: &str) -> Option<i64> {
    let f = time::format_description::well_known::Rfc3339;
    let a = time::OffsetDateTime::parse(a.trim(), &f).ok()?;
    let b = time::OffsetDateTime::parse(b.trim(), &f).ok()?;
    Some((b - a).whole_seconds())
}

fn metrics_md(raw: &Path, rows: &[DecisionRow], project: &str) -> String {
    let setup = read_or_empty(&raw.join("setup.log"));
    let setup_log = {
        let mut log = SetupLog::default();
        for line in setup.lines() {
            if let Some((k, v)) = line.split_once(' ') {
                log.push(k, v);
            }
        }
        log
    };
    let questions = tsv_rows(&read_or_empty(&raw.join("questions.tsv")));
    let prepared = tsv_rows(&read_or_empty(&raw.join("prepared.tsv")));
    let pushes = tsv_rows(&read_or_empty(&raw.join("pushes.tsv")));
    let truth: Vec<TruthLine> = read_or_empty(&raw.join("truth.txt"))
        .lines()
        .filter_map(TruthLine::parse)
        .collect();
    let findings = read_or_empty(&raw.join("findings.md"));
    let stream = read_or_empty(&raw.join("transcripts").join("stream.jsonl"));
    let turns = stream.lines().filter(|l| l.starts_with("# >>> ")).count();
    let mut cost = 0.0;
    let mut stops = 0;
    for l in stream.lines().filter(|l| !l.starts_with('#')) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(l)
            && v["type"] == "result"
        {
            if let Some(c) = v["total_cost_usd"].as_f64() {
                cost = c;
            }
            if v["is_error"] == true {
                stops += 1;
            }
        }
    }
    let mut per_item: BTreeMap<&str, usize> = BTreeMap::new();
    let mut per_kind: BTreeMap<&str, usize> = BTreeMap::new();
    let mut per_origin: BTreeMap<&str, usize> = BTreeMap::new();
    for r in rows {
        *per_item.entry(r.subject.as_str()).or_default() += 1;
        *per_kind.entry(r.kind.as_str()).or_default() += 1;
        *per_origin.entry(r.origin.as_str()).or_default() += 1;
    }
    let mut per_policy: BTreeMap<String, usize> = BTreeMap::new();
    for q in &questions {
        let policy = q
            .get("policy")
            .map(|p| p.split(" (").next().unwrap_or(p).to_string())
            .unwrap_or_default();
        *per_policy.entry(policy).or_default() += 1;
    }
    let by_source = |s: &str| {
        questions
            .iter()
            .filter(|q| q.get("source").map(String::as_str) == Some(s))
            .count()
    };
    let mismatches = truth
        .iter()
        .filter(|l| l.verdict == Verdict::Mismatch)
        .count();
    let unknown = truth
        .iter()
        .filter(|l| l.verdict == Verdict::Unknown)
        .count();
    let finding_count = findings
        .lines()
        .filter(|l| l.trim_start().starts_with("- "))
        .count();
    let edited = prepared
        .iter()
        .filter(|r| {
            r.get("edited")
                .is_some_and(|e| e != "none" && !e.is_empty())
        })
        .count();
    let wall = match (setup_log.get("started"), setup_log.get("ended")) {
        (Some(a), Some(b)) => match seconds_between(a, b) {
            Some(s) => format!("{}h {:02}m {:02}s", s / 3600, (s % 3600) / 60, s % 60),
            None => "unknown".into(),
        },
        _ => "not ended".into(),
    };
    let mut md = String::new();
    md.push_str(&format!("# Walkthrough metrics: {project}\n\n"));
    md.push_str("Derived from `raw/` alone; a second derive gives this file byte for byte.\n\n");
    md.push_str("## The run\n\n");
    md.push_str("| Measure | Value |\n|---|---|\n");
    md.push_str(&format!(
        "| kanbanr | {} |\n",
        setup_log.get("kanbanr").unwrap_or("unknown")
    ));
    md.push_str(&format!(
        "| Claude Code | {} |\n",
        setup_log.get("claude").unwrap_or("unknown")
    ));
    md.push_str(&format!(
        "| model | {} |\n",
        setup_log.get("model").unwrap_or("unknown")
    ));
    md.push_str(&format!("| user turns | {turns} |\n"));
    md.push_str(&format!(
        "| cost (USD, cumulative from the last result line) | {cost:.2} |\n"
    ));
    md.push_str(&format!("| session stops (error results) | {stops} |\n"));
    md.push_str(&format!(
        "| wall time (setup.log started to ended) | {wall} |\n"
    ));
    md.push_str(&format!("| pushes | {} |\n", pushes.len()));
    md.push_str(&format!("| findings | {finding_count} |\n"));
    md.push_str(&format!("| truth mismatches | {mismatches} |\n"));
    md.push_str(&format!("| truth unknown | {unknown} |\n\n"));
    md.push_str("## Decisions\n\n");
    md.push_str(&format!(
        "{} decision(s) on the board copy.\n\n",
        rows.len()
    ));
    md.push_str("| Item or release | Decisions |\n|---|---|\n");
    for (k, n) in &per_item {
        md.push_str(&format!("| {k} | {n} |\n"));
    }
    md.push_str("\n| Kind | Count |\n|---|---|\n");
    for (k, n) in &per_kind {
        md.push_str(&format!("| {k} | {n} |\n"));
    }
    md.push_str("\n| Taken where | Count |\n|---|---|\n");
    for (k, n) in &per_origin {
        md.push_str(&format!("| {k} | {n} |\n"));
    }
    md.push_str(&format!(
        "\nPrepared records confirmed: {} ({} with a field edited). A walkthrough types nothing: \
         the target for MT-1 is zero edited.\n\n",
        prepared.len(),
        edited
    ));
    md.push_str("## Questions\n\n");
    md.push_str(&format!(
        "{} question(s) asked: {} in text, {} through the dialog.\n\n",
        questions.len(),
        by_source("text"),
        by_source("dialog")
    ));
    md.push_str("| Answered by | Count |\n|---|---|\n");
    for (k, n) in &per_policy {
        md.push_str(&format!("| {k} | {n} |\n"));
    }
    md.push_str("\n## Truth\n\n");
    if truth.is_empty() {
        md.push_str("No truth lines.\n");
    } else {
        md.push_str("| Verdict | Check | Board | Truth |\n|---|---|---|---|\n");
        for l in &truth {
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                match l.verdict {
                    Verdict::Ok => "OK",
                    Verdict::Mismatch => "MISMATCH",
                    Verdict::Unknown => "UNKNOWN",
                },
                l.check.replace('|', "/"),
                l.board.replace('|', "/"),
                l.truth.replace('|', "/")
            ));
        }
    }
    md
}

/// The project the run was about: the one project folder under `raw/board/projects`, else the
/// run's `project` key in `setup.log`, else `shop`.
fn project_of(raw: &Path) -> String {
    if let Ok(entries) = std::fs::read_dir(raw.join("board").join("projects")) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        if let Some(first) = names.first() {
            return first.clone();
        }
    }
    read_or_empty(&raw.join("setup.log"))
        .lines()
        .find_map(|l| l.strip_prefix("project ").map(|p| p.trim().to_string()))
        .unwrap_or_else(|| "shop".into())
}

/// Derive the tables from `raw/`, without writing.
pub fn tables_of(raw: &Path) -> Result<Derived> {
    let project = project_of(raw);
    let rows = decisions(raw, &project)
        .with_context(|| format!("reading the board copy under {}", raw.display()))?;
    Ok(Derived {
        decisions_tsv: decisions_tsv(&rows),
        metrics_md: metrics_md(raw, &rows, &project),
        rows: rows.len(),
    })
}

/// Derive the tables for a run and write `results/decisions.tsv` and `results/metrics.md`.
pub fn tables(run: &RunDirs) -> Result<Derived> {
    let d = tables_of(&run.raw())?;
    std::fs::create_dir_all(run.results())?;
    kanbanr_core::atomic::write(run.results().join("decisions.tsv"), &d.decisions_tsv)?;
    kanbanr_core::atomic::write(run.results().join("metrics.md"), &d.metrics_md)?;
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("walkthrough-run")
    }

    /// FTR-037 R-4: two derives over the fixture's `raw/` are byte-identical, and match the tables
    /// kept beside it; the rows come from the board copy and the other captures alone.
    #[test]
    fn a_second_derive_over_the_fixture_is_byte_identical() {
        let fixture = fixture();
        // The fixture's board copy is read from under the temporary directory: a test opens no
        // board outside the fence (L-41), and the fixture itself stays as committed.
        let dir = std::env::temp_dir().join(format!("kanbanr-wt-derive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let run = RunDirs::new(&dir);
        run.make().unwrap();
        let _ = std::fs::remove_dir_all(run.raw());
        copy_dir(&fixture.join("raw"), &run.raw());
        let first = tables_of(&run.raw()).unwrap();
        let second = tables_of(&run.raw()).unwrap();
        assert_eq!(first, second);
        assert!(first.rows >= 5, "{}", first.decisions_tsv);
        // The tables kept beside the fixture are what the derive must keep producing. When the
        // fixture is new (no tables yet), this run writes them for the person to review and commit.
        let results = fixture.join("results");
        if !results.join("decisions.tsv").exists() {
            std::fs::create_dir_all(&results).unwrap();
            std::fs::write(results.join("decisions.tsv"), &first.decisions_tsv).unwrap();
            std::fs::write(results.join("metrics.md"), &first.metrics_md).unwrap();
        }
        let expected_tsv = std::fs::read_to_string(results.join("decisions.tsv")).unwrap();
        let expected_md = std::fs::read_to_string(results.join("metrics.md")).unwrap();
        assert_eq!(
            first.decisions_tsv, expected_tsv,
            "results/decisions.tsv differs from the derive; regenerate it with `kanbanr walkthrough derive` if the change is meant"
        );
        assert_eq!(
            first.metrics_md, expected_md,
            "results/metrics.md differs from the derive; regenerate it if the change is meant"
        );
        // What the rows say.
        assert!(
            first.decisions_tsv.contains("\tapprove\tFEAT-001\t"),
            "{}",
            first.decisions_tsv
        );
        assert!(
            first
                .decisions_tsv
                .contains("\tsignoff\tFEAT-001\tdesign-review\t")
        );
        assert!(first.decisions_tsv.contains("\tcut\tv0.1.0\ttag\t"));
        assert!(first.decisions_tsv.contains("\tgate-result\t"));
        assert!(
            first.decisions_tsv.contains("Claude (prepared "),
            "a confirmed record names Claude as the asker"
        );
        assert!(
            first.metrics_md.contains("| user turns | 3 |"),
            "{}",
            first.metrics_md
        );
        assert!(first.metrics_md.contains("| runbook | 2 |"));
        assert!(first.metrics_md.contains("| truth mismatches | 1 |"));
        // Writing into the run folder lands both files under results/.
        let d = tables(&run).unwrap();
        assert_eq!(d, first);
        assert!(run.results().join("decisions.tsv").exists());
        assert!(run.results().join("metrics.md").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let src = e.path();
            let dst = to.join(e.file_name());
            if src.is_dir() {
                copy_dir(&src, &dst);
            } else {
                std::fs::copy(&src, &dst).unwrap();
            }
        }
    }

    #[test]
    fn tsv_rows_and_times_are_read() {
        let rows = tsv_rows("a\tb\n1\t2\n3\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["b"], "2");
        assert_eq!(rows[1]["b"], "");
        assert_eq!(
            seconds_between("2026-11-15T10:00:00Z", "2026-11-15T11:30:05Z"),
            Some(5405)
        );
        assert_eq!(seconds_between("x", "y"), None);
    }
}
