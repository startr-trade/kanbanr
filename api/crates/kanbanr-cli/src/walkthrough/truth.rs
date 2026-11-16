//! What was really so (FTR-037 W6, R-3): the board's claims against the remote and CI, ported from
//! SPK-002's `truth.sh`. A release that reads shipped must have its tag on the remote and a passed
//! run for the tag's commit; integrated items must be merged where the process says (`kanbanr git
//! doctor`); the board's own doctor must report nothing. One line per check in `raw/truth.txt`:
//! `OK|MISMATCH|UNKNOWN <check> | board: … | truth: …`. A MISMATCH is a finding, never a stop;
//! UNKNOWN (no `gh`, no repository) is noted, never read as fine.
//!
//! The board claims a runbook's `expect.board` makes are judged here too, from the board's files.

use super::guard::{RemotesEnv, RunDirs};
use super::runbook::{BoardClaim, ItemSel, TruthClaim};
use anyhow::{Result, anyhow};
use kanbanr_core::Store;
use serde_json::Value;
use std::path::Path;

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Mismatch,
    /// Could not be looked up: no `gh`, no repository, an unreachable remote.
    Unknown,
}

/// One line of `truth.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruthLine {
    pub verdict: Verdict,
    pub check: String,
    pub board: String,
    pub truth: String,
}

impl std::fmt::Display for TruthLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let v = match self.verdict {
            Verdict::Ok => "OK",
            Verdict::Mismatch => "MISMATCH",
            Verdict::Unknown => "UNKNOWN",
        };
        write!(
            f,
            "{v} {} | board: {} | truth: {}",
            self.check, self.board, self.truth
        )
    }
}

impl TruthLine {
    pub fn parse(line: &str) -> Option<TruthLine> {
        let (head, rest) = line.split_once(" | board: ")?;
        let (board, truth) = rest.split_once(" | truth: ")?;
        let (v, check) = head.split_once(' ')?;
        let verdict = match v {
            "OK" => Verdict::Ok,
            "MISMATCH" => Verdict::Mismatch,
            "UNKNOWN" => Verdict::Unknown,
            _ => return None,
        };
        Some(TruthLine {
            verdict,
            check: check.to_string(),
            board: board.to_string(),
            truth: truth.to_string(),
        })
    }
}

/// One CI run, as `gh run list --json headSha,conclusion,status,headBranch,url` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiRun {
    pub sha: String,
    pub conclusion: String,
    pub status: String,
    pub branch: String,
    pub url: String,
}

/// The CI runs of the run's code remote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CiRuns {
    pub runs: Vec<CiRun>,
}

impl CiRuns {
    /// From `gh run list --json …` output.
    pub fn from_json(text: &str) -> Result<CiRuns> {
        let v: Value = serde_json::from_str(text)?;
        let runs = v
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| CiRun {
                sha: r["headSha"].as_str().unwrap_or("").to_string(),
                conclusion: r["conclusion"].as_str().unwrap_or("").to_string(),
                status: r["status"].as_str().unwrap_or("").to_string(),
                branch: r["headBranch"].as_str().unwrap_or("").to_string(),
                url: r["url"].as_str().unwrap_or("").to_string(),
            })
            .collect();
        Ok(CiRuns { runs })
    }

    /// Did CI pass for `sha`? `Some(true)` when a completed run succeeded; `Some(false)` when runs
    /// exist and none did; `None` when no run is recorded for it.
    pub fn passed_for(&self, sha: &str) -> Option<bool> {
        let for_sha: Vec<&CiRun> = self.runs.iter().filter(|r| r.sha == sha).collect();
        if for_sha.is_empty() {
            return None;
        }
        Some(
            for_sha
                .iter()
                .any(|r| r.status == "completed" && r.conclusion == "success"),
        )
    }
}

/// `gh run list --repo <remote> --json … --limit 50`, kept as `raw/ci-runs.json`. `None` when `gh`
/// is not there or fails: the truth is then unknown, and the setup log says so.
pub fn fetch_ci_runs(run: &RunDirs, remote: &str, gh: &Path) -> Option<CiRuns> {
    let out = std::process::Command::new(gh)
        .args([
            "run",
            "list",
            "--repo",
            remote,
            "--limit",
            "50",
            "--json",
            "headSha,conclusion,status,headBranch,url",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::create_dir_all(run.raw());
    let _ = kanbanr_core::atomic::write(run.raw().join("ci-runs.json"), &text);
    CiRuns::from_json(&text).ok()
}

/// Is `tag` on `remote`? `None` when the remote cannot be reached.
pub fn remote_has_tag(remote: &str, tag: &str) -> Option<bool> {
    let out = std::process::Command::new("git")
        .args(["ls-remote", "--tags", remote, &format!("refs/tags/{tag}")])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(!String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

/// Run a kanbanr doctor in the run project against the run's board: `Ok(issues)` as JSON, or the
/// reason it could not run.
fn doctor_json(
    kanbanr: &Path,
    run: &RunDirs,
    project: &str,
    args: &[&str],
) -> std::result::Result<Vec<Value>, String> {
    let mut cmd = std::process::Command::new(kanbanr);
    cmd.args(args)
        .args(["--json", "--project", project, "--data-dir"])
        .arg(run.board())
        .current_dir(run.project());
    for (k, _) in std::env::vars_os() {
        let key = k.to_string_lossy();
        if (key.starts_with("KANBANR_") && !key.starts_with("KANBANR_WALKTHROUGH_"))
            || key == "CLAUDECODE"
        {
            cmd.env_remove(k);
        }
    }
    if let Some(fence) = kanbanr_core::project::fence() {
        cmd.env(kanbanr_core::project::FENCE_VAR, fence);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Array(a)) => Ok(a),
        Ok(Value::Object(o)) => Ok(o
            .get("issues")
            .or_else(|| o.get("problems"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()),
        _ => {
            if out.status.success() {
                Ok(Vec::new())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        }
    }
}

/// The checks of a run: releases against the remote's tags and CI, items and sprints against the
/// branches (`kanbanr git doctor`), and the board's doctor. Written to `raw/truth.txt`.
pub fn check(
    run: &RunDirs,
    project: &str,
    remotes: &RemotesEnv,
    ci: Option<&CiRuns>,
    kanbanr: &Path,
) -> Result<Vec<TruthLine>> {
    let store = Store::new(run.board());
    let mut lines = Vec::new();
    let releases = kanbanr_core::releases::load(&store, project).unwrap_or_default();
    for r in &releases {
        let tagged = r.state == kanbanr_core::releases::ReleaseState::Shipped
            || !r.tag_commit.is_empty()
            || !r.tagged_at.is_empty();
        if !tagged {
            continue;
        }
        let board = format!(
            "{} {} at stage {}, tag commit {}",
            r.version,
            if r.state == kanbanr_core::releases::ReleaseState::Shipped {
                "shipped"
            } else {
                "tagged"
            },
            if r.stage.is_empty() { "-" } else { &r.stage },
            if r.tag_commit.is_empty() {
                "-"
            } else {
                &r.tag_commit[..r.tag_commit.len().min(7)]
            }
        );
        lines.push(match remote_has_tag(&remotes.code, &r.version) {
            Some(true) => TruthLine {
                verdict: Verdict::Ok,
                check: format!("tag {}", r.version),
                board: board.clone(),
                truth: format!("refs/tags/{} is on the code remote", r.version),
            },
            Some(false) => TruthLine {
                verdict: Verdict::Mismatch,
                check: format!("tag {}", r.version),
                board: board.clone(),
                truth: format!("refs/tags/{} is not on the code remote", r.version),
            },
            None => TruthLine {
                verdict: Verdict::Unknown,
                check: format!("tag {}", r.version),
                board: board.clone(),
                truth: "the code remote could not be reached".into(),
            },
        });
        lines.push(ci_line(&r.version, &r.tag_commit, &board, ci));
    }
    // Branches against the board.
    match doctor_json(kanbanr, run, project, &["git", "doctor"]) {
        Ok(issues) => {
            let errors: Vec<&Value> = issues.iter().filter(|i| i["severity"] == "error").collect();
            if errors.is_empty() {
                lines.push(TruthLine {
                    verdict: Verdict::Ok,
                    check: "git-doctor".into(),
                    board: format!("{} warning(s)", issues.len()),
                    truth: "every integrated item is merged where the process says".into(),
                });
            }
            for e in errors {
                lines.push(TruthLine {
                    verdict: Verdict::Mismatch,
                    check: format!("git-doctor {}", e["code"].as_str().unwrap_or("-")),
                    board: e["code"].as_str().unwrap_or("the board").to_string(),
                    truth: e["message"].as_str().unwrap_or("").to_string(),
                });
            }
        }
        Err(why) => lines.push(TruthLine {
            verdict: Verdict::Unknown,
            check: "git-doctor".into(),
            board: "-".into(),
            truth: format!("could not run: {why}"),
        }),
    }
    match doctor_json(kanbanr, run, project, &["doctor"]) {
        Ok(issues) => lines.push(doctor_line(&issues, "no error")),
        Err(why) => lines.push(TruthLine {
            verdict: Verdict::Unknown,
            check: "doctor".into(),
            board: "-".into(),
            truth: format!("could not run: {why}"),
        }),
    }
    std::fs::create_dir_all(run.raw())?;
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    kanbanr_core::atomic::write(run.raw().join("truth.txt"), text)?;
    Ok(lines)
}

/// The doctor's verdict as a truth line (RC5 of the fourth real run): its errors are a mismatch;
/// its warnings — advice such as `git tidy` notes, or a doc to fold in later — are listed as
/// information beside an OK, as the git-doctor line already treats them.
pub fn doctor_line(issues: &[Value], board: &str) -> TruthLine {
    let text = |i: &Value| {
        format!(
            "{}{}",
            i["code"]
                .as_str()
                .map(|c| format!("{c}: "))
                .unwrap_or_default(),
            i["message"]
                .as_str()
                .or_else(|| i["detail"].as_str())
                .unwrap_or("?")
        )
    };
    let errors: Vec<String> = issues
        .iter()
        .filter(|i| i["severity"].as_str().is_none_or(|s| s == "error"))
        .map(text)
        .collect();
    let warnings: Vec<String> = issues
        .iter()
        .filter(|i| i["severity"] == "warning")
        .map(text)
        .collect();
    let notes = if warnings.is_empty() {
        String::new()
    } else {
        format!(
            "; {} warning(s), for information: {}",
            warnings.len(),
            warnings.join("; ")
        )
    };
    if errors.is_empty() {
        TruthLine {
            verdict: Verdict::Ok,
            check: "doctor".into(),
            board: board.into(),
            truth: format!("no error{notes}"),
        }
    } else {
        TruthLine {
            verdict: Verdict::Mismatch,
            check: "doctor".into(),
            board: board.into(),
            truth: format!("{} error(s): {}{notes}", errors.len(), errors.join("; ")),
        }
    }
}

fn ci_line(version: &str, tag_commit: &str, board: &str, ci: Option<&CiRuns>) -> TruthLine {
    let check = format!("ci {version}");
    match ci {
        None => TruthLine {
            verdict: Verdict::Unknown,
            check,
            board: board.to_string(),
            truth: "unknown: no gh, so the remote's runs were not read".into(),
        },
        Some(runs) => match runs.passed_for(tag_commit) {
            Some(true) => TruthLine {
                verdict: Verdict::Ok,
                check,
                board: board.to_string(),
                truth: format!("a run passed for {}", short(tag_commit)),
            },
            Some(false) => TruthLine {
                verdict: Verdict::Mismatch,
                check,
                board: board.to_string(),
                truth: format!("no run passed for {}", short(tag_commit)),
            },
            None => TruthLine {
                verdict: Verdict::Mismatch,
                check,
                board: board.to_string(),
                truth: format!("no run is recorded for {}", short(tag_commit)),
            },
        },
    }
}

fn short(sha: &str) -> &str {
    if sha.is_empty() {
        "-"
    } else {
        &sha[..sha.len().min(7)]
    }
}

/// A runbook step's `expect.truth`, judged now.
pub fn claim(
    run: &RunDirs,
    project: &str,
    remotes: &RemotesEnv,
    ci: Option<&CiRuns>,
    kanbanr: &Path,
    claim: &TruthClaim,
) -> TruthLine {
    let store = Store::new(run.board());
    match claim {
        TruthClaim::Tag { release } => {
            let board = format!("{release} is tagged");
            match remote_has_tag(&remotes.code, release) {
                Some(true) => TruthLine {
                    verdict: Verdict::Ok,
                    check: format!("tag {release}"),
                    board,
                    truth: "the tag is on the code remote".into(),
                },
                Some(false) => TruthLine {
                    verdict: Verdict::Mismatch,
                    check: format!("tag {release}"),
                    board,
                    truth: "the tag is not on the code remote".into(),
                },
                None => TruthLine {
                    verdict: Verdict::Unknown,
                    check: format!("tag {release}"),
                    board,
                    truth: "the code remote could not be reached".into(),
                },
            }
        }
        TruthClaim::Ci { release, passed } => {
            let tag_commit = kanbanr_core::releases::load(&store, project)
                .unwrap_or_default()
                .into_iter()
                .find(|r| &r.version == release)
                .map(|r| r.tag_commit)
                .unwrap_or_default();
            let mut line = ci_line(
                release,
                &tag_commit,
                &format!("ci {}", if *passed { "passed" } else { "failed" }),
                ci,
            );
            if !*passed && line.verdict != Verdict::Unknown {
                line.verdict = match line.verdict {
                    Verdict::Ok => Verdict::Mismatch,
                    _ => Verdict::Ok,
                };
            }
            line
        }
        TruthClaim::Merged { item } => {
            let code = match item_code(&store, project, item) {
                Ok(Some(c)) => c,
                Ok(None) => item.describe(),
                Err(why) => {
                    return TruthLine {
                        verdict: Verdict::Mismatch,
                        check: format!("merged {}", item.describe()),
                        board: item.describe(),
                        truth: why,
                    };
                }
            };
            match doctor_json(kanbanr, run, project, &["git", "doctor"]) {
                Ok(issues) => {
                    let about: Vec<String> = issues
                        .iter()
                        .filter(|i| i["severity"] == "error" && i["code"] == code.as_str())
                        .map(|i| i["message"].as_str().unwrap_or("").to_string())
                        .collect();
                    TruthLine {
                        verdict: if about.is_empty() {
                            Verdict::Ok
                        } else {
                            Verdict::Mismatch
                        },
                        check: format!("merged {code}"),
                        board: format!("{code} is integrated"),
                        truth: if about.is_empty() {
                            "its branch is merged".into()
                        } else {
                            about.join("; ")
                        },
                    }
                }
                Err(why) => TruthLine {
                    verdict: Verdict::Unknown,
                    check: format!("merged {code}"),
                    board: format!("{code} is integrated"),
                    truth: format!("could not run git doctor: {why}"),
                },
            }
        }
        TruthClaim::DoctorClean => match doctor_json(kanbanr, run, project, &["doctor"]) {
            Ok(issues) => doctor_line(&issues, "clean"),
            Err(why) => TruthLine {
                verdict: Verdict::Unknown,
                check: "doctor".into(),
                board: "clean".into(),
                truth: format!("could not run: {why}"),
            },
        },
    }
}

/// The code of the item a selector names on the board: `Err` names the candidates when several
/// fit equally well (RC6).
fn item_code(
    store: &Store,
    project: &str,
    sel: &ItemSel,
) -> std::result::Result<Option<String>, String> {
    let Ok(p) = store.load(project) else {
        return Ok(None);
    };
    let mut items: Vec<_> = p.features.iter().collect();
    items.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.code.cmp(&b.code)));
    match sel.pick(&items, |f| &f.code, |f| &f.title) {
        super::runbook::Pick::One(f) => Ok(Some(f.code.clone())),
        super::runbook::Pick::None => Ok(None),
        super::runbook::Pick::Ambiguous(v) => Err(sel.ambiguity(
            &v.iter()
                .map(|f| (f.code.clone(), f.title.clone()))
                .collect::<Vec<_>>(),
        )),
    }
}

/// A runbook step's `expect.board`, judged from the board's files: `Ok(())` when it holds, else
/// what the board says instead.
pub fn board_claim(
    board: &Path,
    project: &str,
    claim: &BoardClaim,
) -> Result<std::result::Result<(), String>> {
    let store = Store::new(board);
    match claim {
        BoardClaim::ProjectExists => Ok(match store.load(project) {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("the board has no project {project}: {e}")),
        }),
        BoardClaim::Release { version, stage } => {
            let releases = kanbanr_core::releases::load(&store, project)?;
            Ok(match releases.iter().find(|r| &r.version == version) {
                None => Err(format!(
                    "no release {version} (the board has {})",
                    releases
                        .iter()
                        .map(|r| r.version.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Some(r) => {
                    let shipped = r.state == kanbanr_core::releases::ReleaseState::Shipped;
                    if r.stage == *stage || (stage == "shipped" && shipped) {
                        Ok(())
                    } else {
                        Err(format!(
                            "{version} is at stage {}{}",
                            if r.stage.is_empty() { "-" } else { &r.stage },
                            if shipped { ", shipped" } else { "" }
                        ))
                    }
                }
            })
        }
        BoardClaim::ItemTitleMatches(pat) => {
            let p = store.load(project)?;
            let sel = ItemSel::TitleMatches(pat.clone());
            Ok(if p.features.iter().any(|f| sel.title_fits(&f.title)) {
                Ok(())
            } else {
                Err(format!(
                    "no item's title matches /{pat}/ (items: {})",
                    p.features
                        .iter()
                        .map(|f| format!("{} {}", f.code, f.title))
                        .collect::<Vec<_>>()
                        .join("; ")
                ))
            })
        }
        BoardClaim::ItemTitleContains(t) => {
            let p = store.load(project)?;
            let t = t.to_ascii_lowercase();
            Ok(
                if p.features
                    .iter()
                    .any(|f| f.title.to_ascii_lowercase().contains(&t))
                {
                    Ok(())
                } else {
                    Err(format!(
                        "no item's title contains \"{t}\" (items: {})",
                        p.features
                            .iter()
                            .map(|f| format!("{} {}", f.code, f.title))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ))
                },
            )
        }
        BoardClaim::ItemStatus { item, status } => {
            let p = store.load(project)?;
            Ok(match item_code(&store, project, item) {
                Err(why) => Err(why),
                Ok(None) => Err(format!("no item matches {}", item.describe())),
                Ok(Some(code)) => {
                    let f = p.feature(&code).map_err(|e| anyhow!("{e}"))?;
                    if f.status.eq_ignore_ascii_case(status) {
                        Ok(())
                    } else {
                        Err(format!("{code} is at {}", f.status))
                    }
                }
            })
        }
        BoardClaim::ItemExists(item) => Ok(match item_code(&store, project, item) {
            Err(why) => Err(why),
            Ok(None) => Err(format!("no item matches {}", item.describe())),
            Ok(Some(_)) => Ok(()),
        }),
        BoardClaim::ReleasePrepareConfigured => {
            let p = store.load(project)?;
            Ok(if p.config.cadence.release_prepare.is_empty() {
                Err("release preparation is not configured (no notes path, changelog or version file)".into())
            } else {
                Ok(())
            })
        }
        BoardClaim::SprintActive => {
            let sprints = kanbanr_core::sprints::load(&store, project)?;
            Ok(match kanbanr_core::sprints::active(&sprints) {
                Some(_) => Ok(()),
                None => Err(format!(
                    "no sprint is running ({} sprint(s) on the board)",
                    sprints.len()
                )),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walkthrough::guard::tests::scratch_run;
    use kanbanr_core::releases::{Release, ReleaseState};

    const SHA: &str = "9f078bd0c1e2aaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    /// The unit tests of the binary have no built `kanbanr` to run the doctors with: they get the
    /// UNKNOWN lines a missing program gives, which is what the test about them checks. The
    /// integration tests run the real doctors.
    fn cli() -> std::path::PathBuf {
        std::path::PathBuf::from("kanbanr-that-is-not-run")
    }

    /// A board with the project and one release in the given state.
    fn board_with_release(
        run: &RunDirs,
        state: ReleaseState,
        stage: &str,
        tag_commit: &str,
    ) -> Store {
        let store = Store::new(run.board());
        if store.load("shop").is_err() {
            store
                .init_project(
                    "shop",
                    kanbanr_core::config::ProjectConfig::default_for("shop"),
                )
                .unwrap();
        }
        let release = Release {
            version: "v0.1.0".into(),
            state,
            stage: stage.into(),
            tag_commit: tag_commit.into(),
            ..Default::default()
        };
        std::fs::write(
            store.project_dir("shop").join("releases.yaml"),
            kanbanr_core::yaml::to_string(&vec![release]).unwrap(),
        )
        .unwrap();
        store
    }

    /// FTR-037 R-3: a release the board reads as shipped, with no tag on the remote and no passed
    /// run for its commit, is two MISMATCH lines; with the tag pushed and a passed run, two OK
    /// lines; without `gh` the CI line is UNKNOWN, never OK. The lines round-trip through
    /// `truth.txt`.
    #[test]
    fn a_shipped_claim_without_a_tag_or_a_passed_run_is_a_mismatch() {
        let (run, base) = scratch_run("truth");
        let remotes = RemotesEnv::read(&run).unwrap();
        board_with_release(&run, ReleaseState::Shipped, "tag", SHA);
        let no_runs = CiRuns::from_json("[]").unwrap();
        let lines = check(&run, "shop", &remotes, Some(&no_runs), &cli()).unwrap();
        let tag = lines.iter().find(|l| l.check == "tag v0.1.0").unwrap();
        assert_eq!(tag.verdict, Verdict::Mismatch, "{tag}");
        assert!(tag.board.contains("shipped"), "{tag}");
        assert!(tag.truth.contains("not on the code remote"), "{tag}");
        let ci = lines.iter().find(|l| l.check == "ci v0.1.0").unwrap();
        assert_eq!(ci.verdict, Verdict::Mismatch, "{ci}");
        assert!(ci.truth.contains("no run is recorded"), "{ci}");
        let written = std::fs::read_to_string(run.raw().join("truth.txt")).unwrap();
        assert!(
            written
                .lines()
                .any(|l| l.starts_with("MISMATCH tag v0.1.0 | board: ")),
            "{written}"
        );
        let parsed: Vec<TruthLine> = written.lines().filter_map(TruthLine::parse).collect();
        assert_eq!(parsed.len(), written.lines().count());
        assert_eq!(&parsed[0], tag);
        // A failed run is a mismatch too; a passed one and the tag pushed make both OK.
        let failed = CiRuns::from_json(&format!(
            "[{{\"headSha\":\"{SHA}\",\"conclusion\":\"failure\",\"status\":\"completed\",\"headBranch\":\"main\",\"url\":\"\"}}]"
        ))
        .unwrap();
        assert_eq!(failed.passed_for(SHA), Some(false));
        let project = run.project();
        let git = |args: &[&str]| kanbanr_core::coderepo::git(&project, args).unwrap();
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(project.join("a.txt"), "a\n").unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "a",
        ]);
        git(&["tag", "v0.1.0"]);
        git(&["remote", "add", "origin", &remotes.code]);
        crate::walkthrough::guard::git_push(&project, "origin", "main").unwrap();
        crate::walkthrough::guard::git_push(&project, "origin", "v0.1.0").unwrap();
        let sha = git(&["rev-parse", "HEAD"]);
        board_with_release(&run, ReleaseState::Shipped, "tag", &sha);
        let passed = CiRuns::from_json(&format!(
            "[{{\"headSha\":\"{sha}\",\"conclusion\":\"success\",\"status\":\"completed\",\"headBranch\":\"main\",\"url\":\"https://example.invalid/run/1\"}}]"
        ))
        .unwrap();
        let lines = check(&run, "shop", &remotes, Some(&passed), &cli()).unwrap();
        let tag = lines.iter().find(|l| l.check == "tag v0.1.0").unwrap();
        assert_eq!(tag.verdict, Verdict::Ok, "{tag}");
        let ci = lines.iter().find(|l| l.check == "ci v0.1.0").unwrap();
        assert_eq!(ci.verdict, Verdict::Ok, "{ci}");
        // No gh: unknown, and said so.
        let lines = check(&run, "shop", &remotes, None, &cli()).unwrap();
        let ci = lines.iter().find(|l| l.check == "ci v0.1.0").unwrap();
        assert_eq!(ci.verdict, Verdict::Unknown, "{ci}");
        assert!(ci.truth.contains("no gh"), "{ci}");
        // The doctors ran: one line each.
        assert!(lines.iter().any(|l| l.check == "doctor"), "{lines:?}");
        assert!(lines.iter().any(|l| l.check == "git-doctor"), "{lines:?}");
        // The step claims.
        let t = claim(
            &run,
            "shop",
            &remotes,
            None,
            &cli(),
            &TruthClaim::Tag {
                release: "v0.1.0".into(),
            },
        );
        assert_eq!(t.verdict, Verdict::Ok, "{t}");
        let t = claim(
            &run,
            "shop",
            &remotes,
            None,
            &cli(),
            &TruthClaim::Tag {
                release: "v9.9.9".into(),
            },
        );
        assert_eq!(t.verdict, Verdict::Mismatch, "{t}");
        let t = claim(
            &run,
            "shop",
            &remotes,
            Some(&passed),
            &cli(),
            &TruthClaim::Ci {
                release: "v0.1.0".into(),
                passed: true,
            },
        );
        assert_eq!(t.verdict, Verdict::Ok, "{t}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// RC5: the doctor's warnings are information, its errors a mismatch.
    #[test]
    fn doctor_warnings_are_information_and_errors_a_mismatch() {
        let warn = serde_json::json!({"severity": "warning", "code": "FEAT-002", "message": "fold the design into ci-pipeline.md"});
        let err = serde_json::json!({"severity": "error", "code": "FEAT-003", "message": "depends on a missing item"});
        let ok = doctor_line(std::slice::from_ref(&warn), "clean");
        assert_eq!(ok.verdict, Verdict::Ok, "{ok}");
        assert!(
            ok.truth
                .contains("1 warning(s), for information: FEAT-002: fold the design"),
            "{ok}"
        );
        let bad = doctor_line(&[warn, err], "clean");
        assert_eq!(bad.verdict, Verdict::Mismatch, "{bad}");
        assert!(
            bad.truth.starts_with("1 error(s): FEAT-003: depends"),
            "{bad}"
        );
        assert_eq!(doctor_line(&[], "clean").truth, "no error");
    }

    /// The board claims a runbook makes are read from the board's files, each saying what the
    /// board holds instead when it does not hold.
    #[test]
    fn board_claims_are_read_from_the_board_files() {
        let (run, base) = scratch_run("claims");
        let store = board_with_release(&run, ReleaseState::Planned, "candidate", "");
        let board = run.board();
        assert_eq!(
            board_claim(&board, "shop", &BoardClaim::ProjectExists).unwrap(),
            Ok(())
        );
        assert!(
            board_claim(&board, "other", &BoardClaim::ProjectExists)
                .unwrap()
                .is_err()
        );
        assert_eq!(
            board_claim(
                &board,
                "shop",
                &BoardClaim::Release {
                    version: "v0.1.0".into(),
                    stage: "candidate".into()
                }
            )
            .unwrap(),
            Ok(())
        );
        let why = board_claim(
            &board,
            "shop",
            &BoardClaim::Release {
                version: "v0.1.0".into(),
                stage: "main".into(),
            },
        )
        .unwrap()
        .unwrap_err();
        assert!(why.contains("at stage candidate"), "{why}");
        let why = board_claim(
            &board,
            "shop",
            &BoardClaim::ItemTitleContains("basket".into()),
        )
        .unwrap()
        .unwrap_err();
        assert!(why.contains("no item's title"), "{why}");
        store
            .add_milestone("shop", "M", "", vec![], Some("M".into()))
            .unwrap();
        let f = store
            .add_feature("shop", "Saved basket", "spec", "M", None)
            .unwrap();
        assert_eq!(
            board_claim(
                &board,
                "shop",
                &BoardClaim::ItemTitleContains("BASKET".into())
            )
            .unwrap(),
            Ok(())
        );
        // T16: a title pattern, case-insensitive.
        assert_eq!(
            board_claim(
                &board,
                "shop",
                &BoardClaim::ItemTitleMatches("^saved b".into())
            )
            .unwrap(),
            Ok(())
        );
        assert!(
            board_claim(
                &board,
                "shop",
                &BoardClaim::ItemTitleMatches("^basket".into())
            )
            .unwrap()
            .is_err()
        );
        assert_eq!(
            board_claim(
                &board,
                "shop",
                &BoardClaim::ItemStatus {
                    item: ItemSel::Code(f.code.clone()),
                    status: f.status.clone()
                }
            )
            .unwrap(),
            Ok(())
        );
        let why = board_claim(
            &board,
            "shop",
            &BoardClaim::ItemStatus {
                item: ItemSel::TitleContains("basket".into()),
                status: "Migration".into(),
            },
        )
        .unwrap()
        .unwrap_err();
        assert!(why.contains("is at"), "{why}");
        let why = board_claim(&board, "shop", &BoardClaim::ReleasePrepareConfigured)
            .unwrap()
            .unwrap_err();
        assert!(why.contains("not configured"), "{why}");
        let why = board_claim(&board, "shop", &BoardClaim::SprintActive)
            .unwrap()
            .unwrap_err();
        assert!(why.contains("no sprint is running"), "{why}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
