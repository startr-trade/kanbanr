//! Issue mirror (FEAT-043), the side that talks to GitHub: the `gh` implementation of core's
//! [`IssueTracker`], and the board's side of each operation. The operations themselves — what to
//! check, push and record — are `kanbanr_core::mirror`'s (FEAT-176); this file hands them the
//! tracker and a way to write the board through the CLI's write path.

use crate::backend::{Backend, Method};
use anyhow::anyhow;
use kanbanr_core::mirror::{self, GithubMirror, IssueDoc};
pub use kanbanr_core::mirror::{IssueTracker, PullReport, RemoteComment, RemoteIssue, SyncOutcome};
use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

/// GitHub through the `gh` CLI (`$KANBANR_GH` overrides the binary, e.g. for tests).
pub struct GhCli {
    bin: String,
}

impl GhCli {
    pub fn new() -> Self {
        let bin = std::env::var("KANBANR_GH")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "gh".to_string());
        GhCli { bin }
    }

    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
        let mut child = Command::new(&self.bin)
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    "gh (the GitHub CLI) is not installed: see https://cli.github.com".to_string()
                }
                _ => format!("could not run gh: {e}"),
            })?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(input.as_bytes())
                .map_err(|e| format!("could not write to gh: {e}"))?;
        }
        let out = child
            .wait_with_output()
            .map_err(|e| format!("gh failed: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("gh {}: {}", args.join(" "), err.trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn api(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let input = body.map(Value::to_string);
        let mut args = vec!["api", "--method", method, path];
        if input.is_some() {
            args.extend(["--input", "-"]);
        }
        let out = self.run(&args, input.as_deref())?;
        if out.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&out).map_err(|e| format!("unexpected output from gh api {path}: {e}"))
    }
}

/// The REST payload that sets an issue to `doc`. The issues API creates missing labels.
fn payload(doc: &IssueDoc) -> Value {
    let mut v = json!({
        "title": doc.title,
        "body": doc.body,
        "labels": doc.labels,
        "state": if doc.open { "open" } else { "closed" },
    });
    if let Some(reason) = &doc.state_reason {
        v["state_reason"] = json!(reason);
    }
    v
}

fn remote_issue(v: &Value) -> RemoteIssue {
    let open = v["state"].as_str() != Some("closed");
    let labels = v["labels"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l["name"].as_str().or_else(|| l.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    RemoteIssue {
        number: v["number"].as_u64().unwrap_or(0),
        url: v["html_url"].as_str().unwrap_or("").to_string(),
        updated_at: v["updated_at"].as_str().unwrap_or("").to_string(),
        doc: IssueDoc {
            title: v["title"].as_str().unwrap_or("").to_string(),
            body: v["body"].as_str().unwrap_or("").to_string(),
            open,
            state_reason: v["state_reason"].as_str().map(str::to_string),
            labels,
        }
        .normalized(),
    }
}

impl IssueTracker for GhCli {
    fn check(&self) -> Result<(), String> {
        self.run(&["auth", "status"], None)
            .map(|_| ())
            .map_err(|e| {
                if e.contains("not installed") {
                    e
                } else {
                    "gh is not logged in to GitHub: run `gh auth login`".to_string()
                }
            })
    }

    fn is_public(&self, repo: &str) -> Result<bool, String> {
        let v = self.api("GET", &format!("repos/{repo}"), None)?;
        Ok(match v["private"].as_bool() {
            Some(private) => !private,
            None => v["visibility"].as_str() == Some("public"),
        })
    }

    fn create(&self, repo: &str, doc: &IssueDoc) -> Result<(u64, String), String> {
        let body = json!({"title": doc.title, "body": doc.body, "labels": doc.labels});
        let v = self.api("POST", &format!("repos/{repo}/issues"), Some(&body))?;
        let number = v["number"]
            .as_u64()
            .ok_or_else(|| "gh did not return an issue number".to_string())?;
        let url = v["html_url"].as_str().unwrap_or("").to_string();
        if !doc.open {
            self.update(repo, number, doc)?;
        }
        Ok((number, url))
    }

    fn update(&self, repo: &str, number: u64, doc: &IssueDoc) -> Result<(), String> {
        self.api(
            "PATCH",
            &format!("repos/{repo}/issues/{number}"),
            Some(&payload(doc)),
        )
        .map(|_| ())
    }

    fn fetch(&self, repo: &str, number: u64) -> Result<RemoteIssue, String> {
        let v = self.api("GET", &format!("repos/{repo}/issues/{number}"), None)?;
        Ok(remote_issue(&v))
    }

    fn comments(
        &self,
        repo: &str,
        number: u64,
        since: Option<&str>,
    ) -> Result<Vec<RemoteComment>, String> {
        let mut path = format!("repos/{repo}/issues/{number}/comments?per_page=100");
        if let Some(s) = since {
            path.push_str(&format!("&since={s}"));
        }
        let v = self.api("GET", &path, None)?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .map(|c| RemoteComment {
                        author: c["user"]["login"].as_str().unwrap_or("").to_string(),
                        body: c["body"].as_str().unwrap_or("").to_string(),
                        created_at: c["created_at"].as_str().unwrap_or("").to_string(),
                        url: c["html_url"].as_str().unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// The project's GitHub mirror settings, when enabled. Only the tests below still ask: core's
/// operations read the config themselves.
#[cfg(test)]
pub fn enabled(client: &Backend, project: &str) -> anyhow::Result<Option<GithubMirror>> {
    Ok(mirror::load_config(client.store(), project)?.github)
}

#[cfg(test)]
fn load_project(client: &Backend, project: &str) -> anyhow::Result<kanbanr_core::Project> {
    Ok(client.store().load(project)?)
}

/// A batch write to `project` through the CLI's write path, for core's mirror operations.
fn recorder(client: &Backend, project: &str) -> impl Fn(Value) -> Result<(), String> {
    move |body| {
        client
            .write(
                Method::Post,
                &format!("/projects/{project}/batch"),
                Some(body),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Turn the mirror on (core checks the repo and refuses a public one unless `allow_public`).
pub fn enable(
    client: &Backend,
    tracker: &dyn IssueTracker,
    project: &str,
    repo: &str,
    allow_public: bool,
) -> anyhow::Result<()> {
    let config = mirror::enable(tracker, repo, allow_public).map_err(|e| anyhow!(e))?;
    client.write(
        Method::Put,
        &format!("/projects/{project}/mirror"),
        Some(serde_json::to_value(config)?),
    )?;
    Ok(())
}

pub fn disable(client: &Backend, project: &str) -> anyhow::Result<()> {
    client.write(
        Method::Put,
        &format!("/projects/{project}/mirror"),
        Some(json!({})),
    )?;
    Ok(())
}

/// The current plan, without touching the tracker.
pub fn status(
    client: &Backend,
    project: &str,
    all: bool,
) -> anyhow::Result<Option<(GithubMirror, Vec<mirror::MirrorAction>, usize)>> {
    Ok(mirror::status(client.store(), project, all)?.map(|s| (s.mirror, s.actions, s.linked)))
}

/// Push every feature whose issue is missing or out of date, and record the links in one commit.
/// The mirror's own write does not trigger another sync.
pub fn sync(
    client: &Backend,
    tracker: &dyn IssueTracker,
    project: &str,
    all: bool,
) -> anyhow::Result<SyncOutcome> {
    client.with_mirror_suppressed(|| {
        mirror::sync(
            client.store(),
            project,
            tracker,
            all,
            &recorder(client, project),
        )
        .map_err(|e| anyhow!(e))
    })
}

/// Link a feature to an existing issue. The next sync replaces that issue's content with
/// kanbanr's.
pub fn link(
    client: &Backend,
    tracker: &dyn IssueTracker,
    project: &str,
    code: &str,
    number: u64,
) -> anyhow::Result<String> {
    client.with_mirror_suppressed(|| {
        mirror::link(
            client.store(),
            project,
            tracker,
            code,
            number,
            &recorder(client, project),
        )
        .map_err(|e| anyhow!(e))
    })
}

/// What changed on the tracker since the last push.
pub fn pull(
    client: &Backend,
    tracker: &dyn IssueTracker,
    project: &str,
    code: &str,
) -> anyhow::Result<PullReport> {
    mirror::pull(client.store(), project, tracker, code).map_err(|e| anyhow!(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// An in-memory tracker that records calls.
    #[derive(Default)]
    struct FakeTracker {
        issues: RefCell<BTreeMap<u64, IssueDoc>>,
        calls: RefCell<Vec<String>>,
        public: bool,
        fail_updates: bool,
    }

    impl IssueTracker for FakeTracker {
        fn check(&self) -> Result<(), String> {
            Ok(())
        }
        fn is_public(&self, _repo: &str) -> Result<bool, String> {
            Ok(self.public)
        }
        fn create(&self, repo: &str, doc: &IssueDoc) -> Result<(u64, String), String> {
            let n = self.issues.borrow().len() as u64 + 1;
            self.issues.borrow_mut().insert(n, doc.clone());
            self.calls.borrow_mut().push(format!("create #{n}"));
            Ok((n, format!("https://github.com/{repo}/issues/{n}")))
        }
        fn update(&self, _repo: &str, number: u64, doc: &IssueDoc) -> Result<(), String> {
            if self.fail_updates {
                return Err("boom".into());
            }
            self.issues.borrow_mut().insert(number, doc.clone());
            self.calls.borrow_mut().push(format!("update #{number}"));
            Ok(())
        }
        fn fetch(&self, repo: &str, number: u64) -> Result<RemoteIssue, String> {
            let doc = self
                .issues
                .borrow()
                .get(&number)
                .cloned()
                .ok_or("no such issue")?;
            Ok(RemoteIssue {
                number,
                url: format!("https://github.com/{repo}/issues/{number}"),
                doc,
                updated_at: String::new(),
            })
        }
        fn comments(
            &self,
            _repo: &str,
            _number: u64,
            _since: Option<&str>,
        ) -> Result<Vec<RemoteComment>, String> {
            Ok(vec![RemoteComment {
                author: "alice".into(),
                body: "Looks good".into(),
                created_at: "2026-09-13T00:00:00Z".into(),
                url: String::new(),
            }])
        }
    }

    fn temp_backend() -> (Backend, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-mirror-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A board commits as someone (FEAT-128).
        kanbanr_core::git::ensure_repo_as(&dir, Some(("Tester", "t@example.com")));
        let b = Backend::with_policy(dir.clone(), crate::backend::PushPolicy::Off);
        b.disable_auto_mirror();
        b.write(Method::Post, "/projects", Some(json!({"name": "shop"})))
            .unwrap();
        b.write(
            Method::Post,
            "/projects/shop/batch",
            Some(json!({"operations": [{"op": "milestone.add", "name": "M", "code": "MS-001"}]})),
        )
        .unwrap();
        (b, dir)
    }

    fn add_feature(b: &Backend, title: &str) {
        b.write(
            Method::Post,
            "/projects/shop/batch",
            Some(json!({"operations": [{"op": "feature.add", "title": title, "milestone": "MS-001"}]})),
        )
        .unwrap();
    }

    fn feature(b: &Backend, code: &str) -> kanbanr_core::FeatureItem {
        load_project(b, "shop")
            .unwrap()
            .features
            .into_iter()
            .find(|f| f.code == code)
            .unwrap()
    }

    #[test]
    fn enable_refuses_public_repos_unless_allowed() {
        let (b, dir) = temp_backend();
        let public = FakeTracker {
            public: true,
            ..Default::default()
        };
        let err = enable(&b, &public, "shop", "acme/shop", false).unwrap_err();
        assert!(err.to_string().contains("is public"), "{err}");
        assert!(enabled(&b, "shop").unwrap().is_none());
        assert!(enable(&b, &public, "shop", "not-a-repo", true).is_err());
        enable(&b, &public, "shop", "acme/shop", true).unwrap();
        assert_eq!(enabled(&b, "shop").unwrap().unwrap().repo, "acme/shop");
        disable(&b, "shop").unwrap();
        assert!(enabled(&b, "shop").unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sync_creates_then_updates_only_changed_features_and_records_links() {
        let (b, dir) = temp_backend();
        let t = FakeTracker::default();
        add_feature(&b, "Old item"); // before enabling: not mirrored without --all
        std::thread::sleep(std::time::Duration::from_millis(5));
        enable(&b, &t, "shop", "acme/shop", false).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        add_feature(&b, "Cart page");

        let out = sync(&b, &t, "shop", false).unwrap();
        assert_eq!(out.created, vec![("FEAT-002".to_string(), 1)]);
        let link = feature(&b, "FEAT-002").issue.unwrap();
        assert_eq!((link.repo.as_str(), link.number), ("acme/shop", 1));
        assert!(link.synced_hash.is_some() && link.synced_at.is_some());

        // Nothing changed: no tracker calls.
        let calls = t.calls.borrow().len();
        let out = sync(&b, &t, "shop", false).unwrap();
        assert!(out.created.is_empty() && out.updated.is_empty());
        assert_eq!(t.calls.borrow().len(), calls);

        // Completing the feature closes the issue.
        b.write(
            Method::Post,
            "/projects/shop/features/FEAT-002/move",
            Some(json!({"to": "In Progress"})),
        )
        .unwrap();
        b.write(
            Method::Post,
            "/projects/shop/features/FEAT-002/move",
            Some(json!({"to": "Completed"})),
        )
        .unwrap();
        let out = sync(&b, &t, "shop", false).unwrap();
        assert_eq!(out.updated, vec![("FEAT-002".to_string(), 1)]);
        let pushed = t.issues.borrow().get(&1).cloned().unwrap();
        assert!(!pushed.open);
        assert_eq!(pushed.state_reason.as_deref(), Some("completed"));

        // --all backfills the older open item.
        let out = sync(&b, &t, "shop", true).unwrap();
        assert_eq!(out.created, vec![("FEAT-001".to_string(), 2)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_pushes_are_reported_and_retried_later() {
        let (b, dir) = temp_backend();
        let mut t = FakeTracker::default();
        enable(&b, &t, "shop", "acme/shop", false).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        add_feature(&b, "Cart page");
        sync(&b, &t, "shop", false).unwrap();
        b.write(
            Method::Post,
            "/projects/shop/batch",
            Some(json!({"operations": [{"op": "feature.edit", "code": "FEAT-001", "title": "Cart v2"}]})),
        )
        .unwrap();
        t.fail_updates = true;
        let out = sync(&b, &t, "shop", false).unwrap();
        assert_eq!(out.failed.len(), 1);
        t.fail_updates = false;
        let out = sync(&b, &t, "shop", false).unwrap();
        assert_eq!(out.updated, vec![("FEAT-001".to_string(), 1)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn link_and_pull_report_edits_on_both_sides_and_new_comments() {
        let (b, dir) = temp_backend();
        let t = FakeTracker::default();
        enable(&b, &t, "shop", "acme/shop", false).unwrap();
        add_feature(&b, "Imported from GitHub");
        // An existing issue #1 on the tracker.
        t.issues.borrow_mut().insert(
            1,
            IssueDoc {
                title: "Original".into(),
                body: "Original body".into(),
                open: true,
                state_reason: None,
                labels: vec![],
            },
        );
        let url = link(&b, &t, "shop", "FEAT-001", 1).unwrap();
        assert!(url.ends_with("/issues/1"));

        let report = pull(&b, &t, "shop", "FEAT-001").unwrap();
        assert!(report.edited_on_tracker, "never synced: differs");
        assert!(report.changed_in_kanbanr);
        assert_eq!(report.comments.len(), 1);

        sync(&b, &t, "shop", true).unwrap();
        let report = pull(&b, &t, "shop", "FEAT-001").unwrap();
        assert!(!report.edited_on_tracker && !report.changed_in_kanbanr);

        // Someone edits the title on GitHub.
        t.issues.borrow_mut().get_mut(&1).unwrap().title = "Edited on GitHub".into();
        let report = pull(&b, &t, "shop", "FEAT-001").unwrap();
        assert!(report.edited_on_tracker);
        assert!(!report.changed_in_kanbanr);
        assert_eq!(report.remote.doc.title, "Edited on GitHub");
        let _ = std::fs::remove_dir_all(dir);
    }
}
