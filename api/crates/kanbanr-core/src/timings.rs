//! Test durations (FTR-058): what a captured run timed, kept as one file per run in the project's
//! `timings/` folder, `<YYYYMMDDTHHMMSSZ>-<sha7>.yaml` (`-norev` when the run had no revision),
//! written once and never rewritten, as gate results are (`gates.rs`).
//!
//! A run file holds only tracked tests. A test's last duration and the slowest and slowing report
//! are derived from the kept files on read and never stored (ADR-0002, ADR-0005). Durations stay
//! off `TestRef`, so they never touch an approval's hash (ADR-0015).

use crate::error::{CoreError, Result};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// The project folder that holds the kept runs.
pub const FOLDER: &str = "timings";

/// One timed tracked test in a run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TimingRow {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub requirement: String,
    #[serde(default)]
    pub test: String,
    #[serde(default)]
    pub passed: bool,
    /// Whole milliseconds.
    #[serde(default)]
    pub ms: u64,
    /// Keys this version does not know — written by a newer kanbanr — kept (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

/// One captured run: when its batch applied, the project revision it ran at, and its timed rows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TimingRun {
    /// RFC 3339, UTC.
    #[serde(default)]
    pub at: String,
    /// The project revision of the run, or empty.
    #[serde(default)]
    pub rev: String,
    #[serde(default)]
    pub tests: Vec<TimingRow>,
    /// Keys this version does not know — written by a newer kanbanr — kept (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

fn dir(store: &Store, id: &str) -> PathBuf {
    store.project_dir(id).join(FOLDER)
}

/// Keep a run as a new file beside the earlier ones: `<stamp>-<sha7>.yaml`, `-norev` for a run
/// with no revision, and `-2`, `-3` … when two runs fall in one second. Written atomically.
pub fn write(store: &Store, id: &str, run: &TimingRun) -> Result<PathBuf> {
    let folder = dir(store, id);
    let stamp: String = run
        .at
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(14)
        .collect();
    if stamp.len() != 14 {
        return Err(CoreError::Unsupported(format!(
            "'{}' is not a time",
            run.at
        )));
    }
    let rev = run.rev.trim();
    let sha7: String = if rev.is_empty() {
        "norev".to_string()
    } else {
        rev.chars().take(7).collect()
    };
    let base = format!("{}T{}Z-{sha7}", &stamp[..8], &stamp[8..]);
    std::fs::create_dir_all(&folder)?;
    let mut path = folder.join(format!("{base}.yaml"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{base}-{n}.yaml"));
        n += 1;
    }
    crate::atomic::write(&path, crate::yaml::to_string(run)?)?;
    Ok(path)
}

/// Every kept run, oldest first by file name. A file that cannot be read is skipped.
pub fn runs(store: &Store, id: &str) -> Result<Vec<TimingRun>> {
    let Ok(entries) = std::fs::read_dir(dir(store, id)) else {
        return Ok(Vec::new());
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml") && p.is_file())
        .collect();
    paths.sort_by_cached_key(|p| order(p));
    Ok(paths
        .iter()
        .filter_map(|p| {
            std::fs::read_to_string(p)
                .ok()
                .and_then(|s| crate::yaml::from_str::<TimingRun>(&s).ok())
        })
        .collect())
}

/// The order of a run file's name: its stamp, then the clash number (`-2` after the first), so
/// two runs in one second read in the order they were written.
fn order(path: &std::path::Path) -> (String, u32, String) {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut parts = stem.splitn(3, '-');
    let stamp = parts.next().unwrap_or_default().to_string();
    let _rev = parts.next();
    let n = parts.next().and_then(|n| n.parse().ok()).unwrap_or(1);
    (stamp, n, stem)
}

/// A test's key: `(code, requirement, test)`.
pub type TestKey = (String, String, String);

/// Per test, the newest kept row's `ms` and its run's `at`. `runs` is oldest first.
pub fn last(runs: &[TimingRun]) -> BTreeMap<TestKey, (u64, String)> {
    // Borrowed keys while scanning: one owned key per test, not per row.
    let mut seen: HashMap<(&str, &str, &str), (u64, &str)> = HashMap::new();
    for run in runs {
        for row in &run.tests {
            seen.insert(
                (&row.code, &row.requirement, &row.test),
                (row.ms, run.at.as_str()),
            );
        }
    }
    seen.into_iter()
        .map(|((c, r, t), (ms, at))| ((c.into(), r.into(), t.into()), (ms, at.into())))
        .collect()
}

/// [`last`] for one item's tests only: the design's read for one item (R-11). It reads the runs
/// newest first and keeps the first row it meets per test, comparing only the code of the rows of
/// other items.
pub fn last_for(runs: &[TimingRun], code: &str) -> BTreeMap<TestKey, (u64, String)> {
    let mut out: BTreeMap<TestKey, (u64, String)> = BTreeMap::new();
    for run in runs.iter().rev() {
        for row in run.tests.iter().filter(|r| r.code == code) {
            out.entry((row.code.clone(), row.requirement.clone(), row.test.clone()))
                .or_insert_with(|| (row.ms, run.at.clone()));
        }
    }
    out
}

/// The release the report compares with: the newest shipped one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Baseline {
    pub version: String,
    /// RFC 3339, UTC.
    pub shipped_at: String,
}

/// A test among the slowest, by its last duration.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SlowRow {
    pub code: String,
    pub requirement: String,
    pub test: String,
    pub ms: u64,
    /// When the run that timed it applied.
    pub at: String,
}

/// A test whose time grew since the baseline.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SlowingRow {
    pub code: String,
    pub requirement: String,
    pub test: String,
    /// The newest duration at or before the baseline.
    pub before_ms: u64,
    /// The last duration.
    pub now_ms: u64,
    pub grew_ms: u64,
}

/// The slowest tests and those that slowed most since the last shipped release, derived from the
/// kept runs only.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TimingReport {
    /// How many kept runs it was derived from.
    pub runs: usize,
    pub baseline: Option<Baseline>,
    pub slowest: Vec<SlowRow>,
    pub slowing: Vec<SlowingRow>,
}

/// The report over `runs` (oldest first): the `top` slowest by last duration, ties broken by name,
/// and the `top` that grew most since `baseline`. With no baseline, nothing is slowing.
pub fn report(runs: &[TimingRun], baseline: Option<Baseline>, top: usize) -> TimingReport {
    /// What one pass over the runs learns about a test.
    struct Seen<'a> {
        ms: u64,
        at: &'a str,
        /// The newest duration at or before the baseline.
        before: Option<u64>,
        /// Whether a run after the baseline timed it.
        after: bool,
    }
    let mut seen: HashMap<(&str, &str, &str), Seen> = HashMap::new();
    for run in runs {
        let is_after = baseline.as_ref().is_some_and(|b| {
            crate::cmp_instants(&run.at, &b.shipped_at) == std::cmp::Ordering::Greater
        });
        for row in &run.tests {
            let s = seen
                .entry((&row.code, &row.requirement, &row.test))
                .or_insert(Seen {
                    ms: row.ms,
                    at: &run.at,
                    before: None,
                    after: false,
                });
            s.ms = row.ms;
            s.at = &run.at;
            if is_after {
                s.after = true;
            } else {
                s.before = Some(row.ms);
            }
        }
    }
    let by_name = |a: (&str, &str, &str), b: (&str, &str, &str)| {
        a.2.cmp(b.2)
            .then_with(|| a.0.cmp(b.0))
            .then_with(|| a.1.cmp(b.1))
    };

    let mut slowest: Vec<(&(&str, &str, &str), &Seen)> = seen.iter().collect();
    slowest.sort_by(|(ka, a), (kb, b)| b.ms.cmp(&a.ms).then_with(|| by_name(**ka, **kb)));
    let slowest = slowest
        .into_iter()
        .take(top)
        .map(|((code, requirement, test), s)| SlowRow {
            code: code.to_string(),
            requirement: requirement.to_string(),
            test: test.to_string(),
            ms: s.ms,
            at: s.at.to_string(),
        })
        .collect();

    let mut slowing: Vec<(&(&str, &str, &str), u64, u64)> = if baseline.is_some() {
        seen.iter()
            .filter_map(|(k, s)| match s.before {
                Some(before) if s.after && s.ms > before => Some((k, before, s.ms)),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    slowing.sort_by(|(ka, ba, na), (kb, bb, nb)| {
        (nb - bb).cmp(&(na - ba)).then_with(|| by_name(**ka, **kb))
    });
    let slowing = slowing
        .into_iter()
        .take(top)
        .map(
            |((code, requirement, test), before_ms, now_ms)| SlowingRow {
                code: code.to_string(),
                requirement: requirement.to_string(),
                test: test.to_string(),
                before_ms,
                now_ms,
                grew_ms: now_ms - before_ms,
            },
        )
        .collect();
    TimingReport {
        runs: runs.len(),
        baseline,
        slowest,
        slowing,
    }
}

/// The shipped release with the newest `shipped_at`, or `None` when the project does not work in
/// releases or none has shipped.
pub fn baseline(store: &Store, id: &str) -> Result<Option<Baseline>> {
    if !store.read_project_config(id)?.cadence.releases {
        return Ok(None);
    }
    let releases = crate::releases::load(store, id)?;
    Ok(releases
        .iter()
        .filter(|r| r.state == crate::releases::ReleaseState::Shipped && !r.shipped_at.is_empty())
        .max_by(|a, b| crate::cmp_instants(&a.shipped_at, &b.shipped_at))
        .map(|r| Baseline {
            version: r.version.clone(),
            shipped_at: r.shipped_at.clone(),
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectConfig;

    /// A board of its own under the system temp folder (L-41), with releases on.
    fn scratch(name: &str) -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-timings-{}-{name}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(dir.starts_with(std::env::temp_dir()));
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        (store, dir)
    }

    fn row(code: &str, req: &str, test: &str, ms: u64) -> TimingRow {
        TimingRow {
            code: code.into(),
            requirement: req.into(),
            test: test.into(),
            passed: true,
            ms,
            ..TimingRow::default()
        }
    }

    fn run(at: &str, rows: Vec<TimingRow>) -> TimingRun {
        TimingRun {
            at: at.into(),
            rev: "0123456789abcdef0123456789abcdef01234567".into(),
            tests: rows,
            ..TimingRun::default()
        }
    }

    /// FTR-058 R-4: a run file is named by its time and revision, written once, read back, and a
    /// second run in the same second gets `-2`.
    #[test]
    fn a_run_file_is_written_once_and_read_back() {
        let (store, dir) = scratch("write");
        let first = run(
            "2026-11-16T17:20:31.123456789Z",
            vec![row("FTR-058", "R-1", "evidence::tests::a", 4)],
        );
        let p1 = write(&store, "demo", &first).unwrap();
        assert_eq!(
            p1.file_name().unwrap().to_string_lossy(),
            "20261116T172031Z-0123456.yaml"
        );
        assert_eq!(
            p1.parent().unwrap().canonicalize().unwrap(),
            store
                .project_dir("demo")
                .join(FOLDER)
                .canonicalize()
                .unwrap()
        );
        let text = std::fs::read_to_string(&p1).unwrap();
        assert!(
            text.contains("at: 2026-11-16T17:20:31.123456789Z"),
            "{text}"
        );
        assert!(text.contains("ms: 4"), "{text}");

        let second = run(
            "2026-11-16T17:20:31.900Z",
            vec![row("FTR-058", "R-1", "evidence::tests::a", 7)],
        );
        let p2 = write(&store, "demo", &second).unwrap();
        assert_eq!(
            p2.file_name().unwrap().to_string_lossy(),
            "20261116T172031Z-0123456-2.yaml"
        );
        // The first file is not rewritten.
        assert_eq!(std::fs::read_to_string(&p1).unwrap(), text);

        let norev = TimingRun {
            rev: String::new(),
            ..run("2026-11-16T17:21:00Z", vec![])
        };
        let p3 = write(&store, "demo", &norev).unwrap();
        assert_eq!(
            p3.file_name().unwrap().to_string_lossy(),
            "20261116T172100Z-norev.yaml"
        );

        // An unreadable file is skipped, not fatal.
        std::fs::write(
            p1.with_file_name("20261116T172040Z-broken.yaml"),
            "tests: [",
        )
        .unwrap();
        let back = runs(&store, "demo").unwrap();
        assert_eq!(back, vec![first, second, norev]);
        assert!(write(&store, "demo", &run("not a time", vec![])).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-058 R-8: a test's last duration is the one its newest kept run timed.
    #[test]
    fn the_last_duration_is_the_newest_kept_one() {
        let runs = vec![
            run(
                "2026-11-01T10:00:00Z",
                vec![row("A-1", "R-1", "a", 10), row("A-1", "R-1", "b", 20)],
            ),
            run("2026-11-02T10:00:00Z", vec![row("A-1", "R-1", "a", 30)]),
            run("2026-11-03T10:00:00Z", vec![row("A-1", "R-1", "a", 5)]),
        ];
        let l = last(&runs);
        let k = |t: &str| ("A-1".to_string(), "R-1".to_string(), t.to_string());
        assert_eq!(
            l.get(&k("a")),
            Some(&(5, "2026-11-03T10:00:00Z".to_string()))
        );
        assert_eq!(
            l.get(&k("b")),
            Some(&(20, "2026-11-01T10:00:00Z".to_string()))
        );
        assert_eq!(l.len(), 2);
        assert!(last(&[]).is_empty());
        assert_eq!(last_for(&runs, "A-1"), l);
        assert!(last_for(&runs, "B-2").is_empty());
    }

    /// FTR-058 R-9: the slowest are ranked by last duration, ties by name; "slowing" compares the
    /// newest duration at or before the baseline with the last one, and a test that got faster or
    /// was not timed on both sides is not in it.
    #[test]
    fn the_report_ranks_slowest_and_slowing_from_kept_runs_only() {
        let base = Baseline {
            version: "v0.7.0".into(),
            shipped_at: "2026-11-10T00:00:00Z".into(),
        };
        let runs = vec![
            run(
                "2026-11-05T00:00:00Z",
                vec![
                    row("A-1", "R-1", "grows", 100),
                    row("A-1", "R-1", "faster", 500),
                    row("A-1", "R-2", "grows_a_little", 50),
                ],
            ),
            run(
                "2026-11-09T00:00:00Z",
                vec![
                    row("A-1", "R-1", "grows", 120),
                    row("B-2", "R-1", "only_before", 9),
                ],
            ),
            run(
                "2026-11-12T00:00:00Z",
                vec![
                    row("A-1", "R-1", "grows", 400),
                    row("A-1", "R-1", "faster", 200),
                    row("A-1", "R-2", "grows_a_little", 51),
                    row("B-2", "R-1", "only_after", 400),
                ],
            ),
        ];
        let r = report(&runs, Some(base.clone()), 10);
        assert_eq!(r.runs, 3);
        assert_eq!(r.baseline, Some(base.clone()));
        let names: Vec<(&str, u64)> = r.slowest.iter().map(|s| (s.test.as_str(), s.ms)).collect();
        // 400 twice: ties broken by name.
        assert_eq!(
            names,
            vec![
                ("grows", 400),
                ("only_after", 400),
                ("faster", 200),
                ("grows_a_little", 51),
                ("only_before", 9)
            ]
        );
        assert_eq!(r.slowest[0].at, "2026-11-12T00:00:00Z");
        assert_eq!(
            r.slowing,
            vec![
                SlowingRow {
                    code: "A-1".into(),
                    requirement: "R-1".into(),
                    test: "grows".into(),
                    before_ms: 120,
                    now_ms: 400,
                    grew_ms: 280,
                },
                SlowingRow {
                    code: "A-1".into(),
                    requirement: "R-2".into(),
                    test: "grows_a_little".into(),
                    before_ms: 50,
                    now_ms: 51,
                    grew_ms: 1,
                },
            ]
        );
        let top1 = report(&runs, Some(base), 1);
        assert_eq!((top1.slowest.len(), top1.slowing.len()), (1, 1));
        assert_eq!(top1.slowing[0].test, "grows");
    }

    /// FTR-058 R-9: with no shipped release there is no baseline, and nothing is slowing.
    #[test]
    fn with_no_shipped_release_the_report_says_there_is_no_baseline() {
        let (store, dir) = scratch("baseline");
        // Releases off: no baseline whatever releases.yaml holds.
        let shipped = "- version: v0.9.0\n  state: shipped\n  shipped_at: 2026-10-01T00:00:00Z\n\
                       - version: v0.9.1\n  state: shipped\n  shipped_at: 2026-10-05T00:00:00Z\n\
                       - version: v1.0.0\n  state: planned\n";
        std::fs::write(store.project_dir("demo").join("releases.yaml"), shipped).unwrap();
        assert_eq!(baseline(&store, "demo").unwrap(), None);
        crate::dispatch::dispatch(
            &store,
            "PUT",
            "/projects/demo/config/cadence",
            Some(&serde_json::json!({"releases": true})),
        )
        .unwrap();
        assert_eq!(
            baseline(&store, "demo").unwrap(),
            Some(Baseline {
                version: "v0.9.1".into(),
                shipped_at: "2026-10-05T00:00:00Z".into()
            })
        );
        // Releases on, none shipped.
        std::fs::write(
            store.project_dir("demo").join("releases.yaml"),
            "- version: v1.0.0\n  state: planned\n",
        )
        .unwrap();
        assert_eq!(baseline(&store, "demo").unwrap(), None);

        let runs = vec![
            run("2026-11-01T00:00:00Z", vec![row("A-1", "R-1", "a", 1)]),
            run("2026-11-02T00:00:00Z", vec![row("A-1", "R-1", "a", 9)]),
        ];
        let r = report(&runs, None, 10);
        assert_eq!(r.baseline, None);
        assert!(r.slowing.is_empty());
        assert_eq!(r.slowest.len(), 1);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["baseline"], serde_json::Value::Null);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-058 R-11: on 500 kept runs of 1,300 rows, `last` for one item (`last_for`) stays under
    /// 150 ms and `report` under 1 s. The fixture is generated in memory: what is measured is the
    /// two derivations, not the YAML parse of 65 MB of run files (printed for reference from one
    /// file).
    #[test]
    fn reads_stay_within_budget_on_500_runs() {
        // 100 items of 13 tests each.
        let kept: Vec<TimingRun> = (0..500u64)
            .map(|r| {
                let rows = (0..100u64)
                    .flat_map(|i| {
                        (0..13u64).map(move |t| TimingRow {
                            code: format!("FTR-{i:03}"),
                            requirement: format!("R-{t}"),
                            test: format!("module::tests::test_{t}"),
                            passed: true,
                            ms: (r * 7 + i * 13 + t) % 5000,
                            ..TimingRow::default()
                        })
                    })
                    .collect();
                run(&format!("2026-11-01T{:02}:{:02}:00Z", r / 60, r % 60), rows)
            })
            .collect();
        let base = Some(Baseline {
            version: "v0.7.0".into(),
            shipped_at: "2026-11-01T04:00:00Z".into(),
        });

        // One run file's parse, for reference: the read cost scales with the runs kept.
        let (store, dir) = scratch("budget");
        write(&store, "demo", &kept[0]).unwrap();
        let t = std::time::Instant::now();
        assert_eq!(runs(&store, "demo").unwrap()[0].tests.len(), 1300);
        let read_one = t.elapsed();
        let _ = std::fs::remove_dir_all(dir);

        let t = std::time::Instant::now();
        let one = last_for(&kept, "FTR-042");
        let last_one = t.elapsed();
        assert_eq!(one.len(), 13);
        let all = last(&kept);
        assert!(one.iter().all(|(k, v)| all.get(k) == Some(v)));

        let t = std::time::Instant::now();
        let r = report(&kept, base, 10);
        let rep = t.elapsed();
        assert_eq!((r.runs, r.slowest.len()), (500, 10));

        eprintln!(
            "timings budget: read one run file {read_one:?}, last for one item {last_one:?}, \
             report {rep:?}"
        );
        assert!(
            last_one.as_millis() < 150,
            "last for one item took {last_one:?}"
        );
        assert!(rep.as_millis() < 1000, "report took {rep:?}");
    }
}
