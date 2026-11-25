//! The scripted person (FTR-037 W4, W4b, R-3, R-9): takes each step's decision the way the person
//! would, through the monitor's write routes on the run's fenced board (ADR-0016), after checking
//! that the offer the guide names is there. With a browser on the machine, a `press` step clicks
//! the button on the rendered page instead, and an `expect.page` claim reads the rendered text.
//!
//! The route table is the monitor's own (`web/src/api.ts`): a test holds the two together. Nothing
//! here reads or writes the board's files; every read is the GET the page makes, every write the
//! POST or PUT the page posts, with the same body.

use super::guard::RunDirs;
use super::runbook::{Decision, DelegationSel, ItemSel, Offer, PageClaim, Pick};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

/// Every write path of `web/src/api.ts`, as `(method, path)`: what the monitor's pages post to.
/// The person module posts only to these. `{…}` are path parameters.
pub const WRITE_ROUTES: &[(&str, &str)] = &[
    ("POST", "/projects/{p}/features/{code}/approve"),
    ("POST", "/projects/{p}/features/{code}/unapprove"),
    ("POST", "/projects/{p}/features/{code}/ratify"),
    ("POST", "/projects/{p}/features/{code}/signoff/{name}"),
    (
        "POST",
        "/projects/{p}/features/{code}/signoff/{name}/withdraw",
    ),
    (
        "PUT",
        "/projects/{p}/features/{code}/tests/{requirement}/{test}",
    ),
    ("POST", "/projects/{p}/adrs/{adr}/accept"),
    ("POST", "/projects/{p}/adrs/{adr}/reject"),
    ("POST", "/projects/{p}/releases"),
    ("DELETE", "/projects/{p}/releases/{version}"),
    ("POST", "/projects/{p}/releases/{version}/plan"),
    ("PUT", "/projects/{p}/releases/{version}/notes"),
    ("POST", "/projects/{p}/releases/{version}/promote"),
    ("POST", "/projects/{p}/releases/{version}/patch-from-tag"),
    ("POST", "/projects/{p}/gate-results"),
    ("POST", "/projects/{p}/prepared/{id}/confirm"),
    ("POST", "/projects/{p}/prepared/{id}/reject"),
    ("POST", "/projects/{p}/prepared/batch/{batch}/confirm"),
    ("POST", "/projects/{p}/prepared/batch/{batch}/reject"),
    ("PUT", "/projects/{p}/decisions/mode"),
    ("POST", "/projects/{p}/decisions/delegations/{id}/grant"),
    ("POST", "/projects/{p}/decisions/delegations/{id}/revoke"),
    ("POST", "/projects/{p}/decisions/reviews"),
    ("POST", "/projects/{p}/baseline/revisions/{id}/accept"),
    ("POST", "/projects/{p}/baseline/revisions/{id}/withdraw"),
    ("POST", "/projects/{p}/catalog/baseline/accept"),
    ("POST", "/projects/{p}/review/decide"),
    ("POST", "/projects/{p}/planning"),
];

/// What a decision did: the route, the answer, and whether anything was typed (never, here: the
/// scripted person posts what the page would with nothing filled in).
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub route: String,
    pub status: u16,
    pub body: String,
    /// The prepared record a confirmation answered.
    pub confirmed: Option<String>,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Whether the offer the guide names is on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Offered,
    /// Not offered, and what the page shows instead.
    Absent(String),
}

/// One item as the board lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemBrief {
    pub code: String,
    pub title: String,
    pub status: String,
    pub created_at: String,
}

/// The monitor a run starts over its scratch board: `kanbanr serve --bind 127.0.0.1:0
/// --allow-writes` in the run project, its port read from its start line, its output kept as
/// `raw/monitor.log`, and the page token read from the page it serves (DEF-024).
pub struct Monitor {
    /// The `kanbanr serve` this run started; none for a monitor reached by its address.
    child: Option<Child>,
    pub url: String,
    token: String,
    agent: ureq::Agent,
}

impl Monitor {
    /// Start the monitor. `kanbanr` is the program to run (the one under test); `ui_dir` a built
    /// monitor to serve when the binary carries none (tests). Waits up to 30 s for the start line.
    pub fn start(
        run: &RunDirs,
        kanbanr: &Path,
        fence: &Path,
        ui_dir: Option<&Path>,
        data_dir: Option<&Path>,
    ) -> Result<Monitor> {
        std::fs::create_dir_all(run.raw())?;
        let log_path = run.raw().join("monitor.log");
        let mut cmd = Command::new(kanbanr);
        cmd.args(["serve", "--bind", "127.0.0.1:0", "--allow-writes"]);
        if let Some(ui) = ui_dir {
            cmd.arg("--ui-dir").arg(ui);
        }
        // The run's board, named so a board the first step has yet to make is still the one served.
        if let Some(d) = data_dir {
            cmd.arg("--data-dir").arg(d);
        }
        cmd.current_dir(run.project());
        for (k, _) in std::env::vars_os() {
            let key = k.to_string_lossy();
            if (key.starts_with("KANBANR_") && !key.starts_with("KANBANR_WALKTHROUGH_"))
                || key == "CLAUDECODE"
            {
                cmd.env_remove(k);
            }
        }
        cmd.env(kanbanr_core::project::FENCE_VAR, fence);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .with_context(|| format!("could not start {} serve", kanbanr.display()))?;
        let (tx, rx) = channel::<String>();
        let log = std::sync::Arc::new(std::sync::Mutex::new(super::append_to(&log_path)?));
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let tx = tx.clone();
            let log = log.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(|l| l.ok()) {
                    if let Ok(mut f) = log.lock() {
                        let _ = writeln!(f, "{line}");
                    }
                    let _ = tx.send(line);
                }
            });
        }
        drop(tx);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut url = None;
        let mut said = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    if let Some(rest) = line.strip_prefix("kanbanr view daemon on ") {
                        url = Some(rest.split_whitespace().next().unwrap_or("").to_string());
                        break;
                    }
                    said.push(line);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if child.try_wait()?.is_some() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let Some(url) = url else {
            let _ = child.kill();
            bail!(
                "the monitor did not say where it listens within 30 s; it said: {}",
                said.join(" | ")
            );
        };
        let mut monitor = Monitor::connect(&url)?;
        monitor.child = Some(child);
        Ok(monitor)
    }

    /// Reach a monitor already running at `url` (`http://127.0.0.1:<port>`): reads the page token
    /// from the page it serves, which a write needs (DEF-024). Waits up to 10 s for the page.
    pub fn connect(url: &str) -> Result<Monitor> {
        let url = url.trim_end_matches('/').to_string();
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .build();
        let mut token = String::new();
        let page_deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < page_deadline {
            if let Ok(resp) = agent.get(&format!("{url}/")).call()
                && let Ok(html) = resp.into_string()
                && let Some(t) = page_token(&html)
            {
                token = t;
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(Monitor {
            child: None,
            url,
            token,
            agent,
        })
    }

    /// Whether the monitor served a page with its token: without one no write goes through.
    pub fn has_token(&self) -> bool {
        !self.token.is_empty()
    }

    /// The monitor's address for a page: `/p/<project>/review`.
    pub fn page_url(&self, project: &str, page: &str) -> String {
        format!("{}/p/{}/{}", self.url, urlencode(project), page)
    }

    /// GET `/api<path>` as JSON.
    pub fn get_json(&self, path: &str) -> Result<Value> {
        let resp = self
            .agent
            .get(&format!("{}/api{path}", self.url))
            .call()
            .map_err(|e| anyhow!("GET {path}: {e}"))?;
        resp.into_json::<Value>()
            .map_err(|e| anyhow!("GET {path}: not JSON: {e}"))
    }

    /// A write as the page posts it: `/api/write<path>` with the page token and a JSON body.
    pub fn write(&self, method: &str, path: &str, body: &Value) -> Result<Outcome> {
        if !WRITE_ROUTES
            .iter()
            .any(|(m, template)| *m == method && path_matches(template, path))
        {
            bail!("{method} {path} is not a route the monitor's pages post to");
        }
        let url = format!("{}/api/write{path}", self.url);
        let req = self
            .agent
            .request(method, &url)
            .set("content-type", "application/json")
            .set("x-kanbanr-token", &self.token);
        let (status, text) = match req.send_string(&body.to_string()) {
            Ok(resp) => {
                let status = resp.status();
                (status, resp.into_string().unwrap_or_default())
            }
            Err(ureq::Error::Status(code, resp)) => (code, resp.into_string().unwrap_or_default()),
            Err(e) => bail!("{method} {path}: {e}"),
        };
        Ok(Outcome {
            route: format!("{method} {path}"),
            status,
            body: text,
            confirmed: None,
        })
    }

    /// One item as the project route carries it: the monitor reads items through the project.
    pub fn feature(&self, project: &str, code: &str) -> Result<Value> {
        let v = self.get_json(&format!("/projects/{}", urlencode(project)))?;
        v["features"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| f["code"] == code)
            .cloned()
            .ok_or_else(|| anyhow!("{project} has no item {code}"))
    }

    /// The project's items, oldest first.
    pub fn items(&self, project: &str) -> Result<Vec<ItemBrief>> {
        let v = self.get_json(&format!("/projects/{}", urlencode(project)))?;
        let mut out: Vec<ItemBrief> = v["features"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| ItemBrief {
                code: f["code"].as_str().unwrap_or("").to_string(),
                title: f["title"].as_str().unwrap_or("").to_string(),
                status: f["status"].as_str().unwrap_or("").to_string(),
                created_at: f["created_at"].as_str().unwrap_or("").to_string(),
            })
            .collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.code.cmp(&b.code)));
        Ok(out)
    }

    /// The item a selector names, if any.
    pub fn resolve(&self, project: &str, sel: &ItemSel) -> Result<Option<ItemBrief>> {
        let items = self.items(project)?;
        match pick(&items, sel) {
            Pick::One(i) => Ok(Some(i)),
            Pick::None => Ok(None),
            Pick::Ambiguous(v) => Err(anyhow!(
                "{}",
                sel.ambiguity(&v.into_iter().map(|i| (i.code, i.title)).collect::<Vec<_>>())
            )),
        }
    }

    fn review(&self, project: &str) -> Result<Vec<Value>> {
        Ok(self
            .get_json(&format!("/projects/{}/review", urlencode(project)))?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    fn plan(&self, project: &str, release: &str) -> Result<Value> {
        self.get_json(&format!(
            "/projects/{}/releases/{}/plan",
            urlencode(project),
            urlencode(release)
        ))
    }

    fn prepared_open(&self, project: &str) -> Result<Vec<Value>> {
        Ok(self
            .get_json(&format!(
                "/projects/{}/prepared?state=open",
                urlencode(project)
            ))?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    fn decisions(&self, project: &str) -> Result<Value> {
        self.get_json(&format!("/projects/{}/decisions", urlencode(project)))
    }

    fn manual_waiting(&self, project: &str) -> Result<Vec<Value>> {
        Ok(self
            .get_json(&format!("/projects/{}/manual-tests", urlencode(project)))?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// The identity a verdict from this monitor is attributed to (`/api/meta`).
    pub fn approver(&self) -> Result<String> {
        let meta = self.get_json("/meta")?;
        meta["identity"]["name"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("the board has no commit identity: the monitor offers nothing"))
    }

    /// Is the button the guide names offered for the subject, as the page would show it? Reads the
    /// GET the Review or Releases page reads for that button. `decision` gives the release or kind
    /// an offer on the Releases page is about.
    pub fn offers(
        &self,
        project: &str,
        offer: &Offer,
        decision: Option<&Decision>,
    ) -> Result<Found> {
        let button = norm(&offer.button);
        let subject = match &offer.subject {
            Some(sel) => match self.resolve(project, sel)? {
                Some(item) => Some(item),
                None => {
                    return Ok(Found::Absent(format!("no item matches {}", sel.describe())));
                }
            },
            None => None,
        };
        let starts =
            |label: &str| norm(label).starts_with(&button) || button.starts_with(&norm(label));
        match offer.page.as_str() {
            "review" => {
                let entries = self.review(project)?;
                let entry = |code: &str| entries.iter().find(|e| e["code"] == code);
                // Approve / Ratify: an entry with the matching approval state.
                if starts("Approve") || starts("Ratify") {
                    let Some(item) = subject else {
                        return Ok(Found::Absent("Approve needs a subject".into()));
                    };
                    let Some(e) = entry(&item.code) else {
                        return Ok(Found::Absent(format!(
                            "{} is not on the Review page",
                            item.code
                        )));
                    };
                    let approval = e["approval"].as_str().unwrap_or("");
                    let shown = match approval {
                        "missing" | "lapsed" | "added" => "approve",
                        "unratified" => "ratify",
                        _ => "",
                    };
                    let wanted = if starts("Ratify") {
                        "ratify"
                    } else {
                        "approve"
                    };
                    return Ok(if shown == wanted {
                        Found::Offered
                    } else {
                        Found::Absent(format!(
                            "{} shows approval={approval}, signoffs_needed={}",
                            item.code, e["signoffs_needed"]
                        ))
                    });
                }
                if let Some(name) = offer.button.trim().strip_prefix("Sign off") {
                    let name = name.trim();
                    let Some(item) = subject else {
                        return Ok(Found::Absent("Sign off needs a subject".into()));
                    };
                    let Some(e) = entry(&item.code) else {
                        return Ok(Found::Absent(format!(
                            "{} is not on the Review page",
                            item.code
                        )));
                    };
                    let needed: Vec<&str> = e["signoffs_needed"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    if !needed.contains(&name) {
                        return Ok(Found::Absent(format!(
                            "{} waits for {:?}, not {name} (approval={})",
                            item.code, needed, e["approval"]
                        )));
                    }
                    // docs-folded is offered only when no text is left to check (DEF-039).
                    let left = e["to_check"].as_array().map(Vec::len).unwrap_or(0);
                    if name == "docs-folded" && left > 0 {
                        return Ok(Found::Absent(format!(
                            "{} has {left} text(s) left to check before docs-folded",
                            item.code
                        )));
                    }
                    return Ok(Found::Offered);
                }
                if starts("Grant") {
                    let d = self.decisions(project)?;
                    let proposed = d["delegations"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|g| g["state"] == "proposed")
                        .count();
                    return Ok(if proposed > 0 {
                        Found::Offered
                    } else {
                        Found::Absent("no delegation is proposed".into())
                    });
                }
                if starts("Change") {
                    return Ok(Found::Offered);
                }
                if starts("Keep") || starts("Undo") {
                    let d = self.decisions(project)?;
                    let n = d["to_review"].as_array().map(Vec::len).unwrap_or(0);
                    return Ok(if n > 0 {
                        Found::Offered
                    } else {
                        Found::Absent("no delegated decision waits for a review".into())
                    });
                }
                if starts("Accept") || starts("Reject") {
                    let adrs = self.get_json(&format!("/projects/{}/adrs", urlencode(project)))?;
                    let proposed = adrs
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|a| a["status"] == "proposed")
                        .count();
                    return Ok(if proposed > 0 {
                        Found::Offered
                    } else {
                        Found::Absent("no decision record is proposed".into())
                    });
                }
                if starts("Passed") || starts("Failed") || starts("Record") {
                    let waiting = self.manual_waiting(project)?;
                    let test_part = match decision {
                        Some(Decision::RecordManual { test, .. }) => test.clone(),
                        _ => String::new(),
                    };
                    let hit = waiting.iter().any(|w| {
                        subject
                            .as_ref()
                            .is_none_or(|i| w["code"] == i.code.as_str())
                            && w["test"]
                                .as_str()
                                .is_some_and(|t| norm(t).contains(&norm(&test_part)))
                    });
                    return Ok(if hit {
                        Found::Offered
                    } else {
                        Found::Absent(format!(
                            "no manual check waits for {}",
                            subject.map(|i| i.code).unwrap_or_else(|| "the item".into())
                        ))
                    });
                }
                Ok(Found::Absent(format!(
                    "the Review page has no button named {}",
                    offer.button
                )))
            }
            "releases" => {
                let release = match decision {
                    Some(Decision::Promote { release, .. })
                    | Some(Decision::GateResult { release, .. }) => Some(release.clone()),
                    Some(Decision::ConfirmPrepared {
                        release: Some(release),
                        ..
                    }) => Some(release.clone()),
                    _ => None,
                };
                if starts("Record") || starts("Passed") || starts("Failed") {
                    // A prepared result shows "Record <gate> <passed|failed>"; the form shows
                    // Passed / Failed while the plan waits for a result.
                    let open = self.prepared_open(project)?;
                    if let Some(r) = open.iter().find(|r| r["kind"] == "gate-result") {
                        let label = format!(
                            "Record {} {}",
                            r["subject"]["gate"].as_str().unwrap_or("ci"),
                            if r["fields"]["passed"] == true {
                                "passed"
                            } else {
                                "failed"
                            }
                        );
                        if starts(&label) || starts("Record") {
                            return Ok(Found::Offered);
                        }
                    }
                    if let Some(release) = &release {
                        let plan = self.plan(project, release)?;
                        if gate_commit(&plan).is_some() {
                            return Ok(Found::Offered);
                        }
                        return Ok(Found::Absent(format!(
                            "{release} waits for no CI result: ready={}, missing={}",
                            plan["ready"], plan["missing"]
                        )));
                    }
                    return Ok(Found::Absent(
                        "no CI result is prepared, and the step names no release".into(),
                    ));
                }
                if starts("Check again") {
                    return Ok(Found::Offered);
                }
                let Some(release) = release else {
                    return Ok(Found::Absent(
                        "the step's decision names no release for the Releases page".into(),
                    ));
                };
                let plan = self.plan(project, &release)?;
                Ok(release_offer(&plan, &release, &offer.button))
            }
            other => Ok(Found::Absent(format!("no page named {other}"))),
        }
    }

    /// Take the decision through the route the page posts to, with the page's body, as `by`. A
    /// confirmation of a prepared record also appends a row to `raw/prepared.tsv`.
    pub fn decide(
        &self,
        project: &str,
        decision: &Decision,
        by: &str,
        step: &str,
        run: Option<&RunDirs>,
    ) -> Result<Outcome> {
        let p = urlencode(project);
        let item_of = |sel: &ItemSel| -> Result<(ItemBrief, Option<Value>)> {
            let item = self
                .resolve(project, sel)?
                .ok_or_else(|| anyhow!("no item matches {}", sel.describe()))?;
            let entry = self
                .review(project)?
                .into_iter()
                .find(|e| e["code"] == item.code.as_str());
            Ok((item, entry))
        };
        let rev_of = |entry: &Option<Value>, key: &str| -> String {
            entry
                .as_ref()
                .and_then(|e| e[key].as_str())
                .unwrap_or("")
                .to_string()
        };
        match decision {
            Decision::Approve(sel) => {
                let (item, entry) = item_of(sel)?;
                self.write(
                    "POST",
                    &format!("/projects/{p}/features/{}/approve", urlencode(&item.code)),
                    &json!({"by": by, "rev": rev_of(&entry, "rev")}),
                )
            }
            Decision::Ratify(sel) => {
                let (item, entry) = item_of(sel)?;
                self.write(
                    "POST",
                    &format!("/projects/{p}/features/{}/ratify", urlencode(&item.code)),
                    &json!({"by": by, "rev": rev_of(&entry, "rev")}),
                )
            }
            Decision::Signoff { item: sel, name } => {
                let (item, entry) = item_of(sel)?;
                self.write(
                    "POST",
                    &format!(
                        "/projects/{p}/features/{}/signoff/{}",
                        urlencode(&item.code),
                        urlencode(name)
                    ),
                    &json!({
                        "by": by,
                        "rev": rev_of(&entry, "rev"),
                        "spec_rev": rev_of(&entry, "spec_rev"),
                    }),
                )
            }
            Decision::Adr { adr, verdict } => {
                let id = if adr.eq_ignore_ascii_case("newest") {
                    let adrs = self.get_json(&format!("/projects/{p}/adrs"))?;
                    adrs.as_array()
                        .into_iter()
                        .flatten()
                        .filter(|a| a["status"] == "proposed")
                        .filter_map(|a| a["id"].as_str())
                        .max()
                        .map(str::to_string)
                        .ok_or_else(|| anyhow!("no decision record is proposed"))?
                } else {
                    adr.clone()
                };
                self.write(
                    "POST",
                    &format!("/projects/{p}/adrs/{}/{verdict}", urlencode(&id)),
                    &json!({"by": by, "reason": ""}),
                )
            }
            Decision::Promote { release, to } => {
                let plan = self.plan(project, release)?;
                let mut body = json!({"by": by});
                if let Some(stage) = to {
                    body["stage"] = json!(stage);
                }
                if let Some(tip) = plan["release_tip"].as_str().filter(|t| !t.is_empty()) {
                    body["expect"] = json!(tip);
                }
                // A stage after the tag confirms what the person checked (FTR-026): the page's
                // note, here the step's own words.
                if plan["confirm"] == true {
                    body["reason"] = json!(format!("checked in walkthrough step {step}"));
                }
                if let Some(refs) = accept_untested(&plan) {
                    body["accept_untested"] = json!(refs);
                }
                self.write(
                    "POST",
                    &format!("/projects/{p}/releases/{}/promote", urlencode(release)),
                    &body,
                )
            }
            Decision::GateResult {
                release,
                gate,
                passed,
                link,
            } => {
                let plan = self.plan(project, release)?;
                let commit = gate_commit(&plan)
                    .ok_or_else(|| anyhow!("{release} waits for no CI result"))?;
                let mut body = json!({"name": gate, "commit": commit, "passed": passed, "by": by});
                if let Some(url) = link {
                    body["url"] = json!(url);
                }
                self.write("POST", &format!("/projects/{p}/gate-results"), &body)
            }
            Decision::ConfirmPrepared { kind, release } => {
                let open = self.prepared_open(project)?;
                // The commits a CI result for the release may be about: its tip and its tag's.
                let commits: Vec<String> = match release {
                    Some(r) if kind == "gate-result" => {
                        let plan = self.plan(project, r).unwrap_or(Value::Null);
                        [plan["release_tip"].as_str(), plan["tag"]["commit"].as_str()]
                            .into_iter()
                            .flatten()
                            .filter(|c| !c.is_empty())
                            .map(str::to_string)
                            .collect()
                    }
                    _ => Vec::new(),
                };
                let for_release = |r: &Value| match release {
                    None => true,
                    Some(v) if kind == "gate-result" => r["subject"]["commit"]
                        .as_str()
                        .is_some_and(|c| commits.iter().any(|x| x == c)),
                    Some(v) => r["subject"]["release"] == v.as_str(),
                };
                // Newest first, as the listing comes.
                let Some(record) = open
                    .iter()
                    .find(|r| r["kind"] == kind.as_str() && for_release(r))
                else {
                    bail!(
                        "no prepared {kind}{} waits for the person",
                        release
                            .as_ref()
                            .map(|r| format!(" for {r}"))
                            .unwrap_or_default()
                    );
                };
                let id = record["id"].as_str().unwrap_or("").to_string();
                let mut body = json!({"by": by});
                if kind == "promotion"
                    && let Some(release) = record["subject"]["release"].as_str()
                    && let Ok(plan) = self.plan(project, release)
                    && let Some(tip) = plan["release_tip"].as_str().filter(|t| !t.is_empty())
                {
                    body["expect"] = json!(tip);
                }
                let mut out = self.write(
                    "POST",
                    &format!("/projects/{p}/prepared/{}/confirm", urlencode(&id)),
                    &body,
                )?;
                if out.ok() {
                    out.confirmed = Some(id.clone());
                    if let Some(run) = run {
                        append_prepared_row(
                            run,
                            step,
                            &id,
                            kind,
                            record["by"].as_str().unwrap_or(""),
                            by,
                        )?;
                    }
                }
                Ok(out)
            }
            Decision::GrantDelegation(sel) => {
                let d = self.decisions(project)?;
                let delegations: Vec<&Value> = d["delegations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|g| match sel {
                        DelegationSel::Proposed => g["state"] == "proposed",
                        DelegationSel::Id(id) => g["id"] == id.as_str(),
                    })
                    .collect();
                if delegations.is_empty() {
                    bail!("no delegation to grant");
                }
                let mut last = None;
                for g in delegations {
                    let id = g["id"].as_str().unwrap_or("");
                    let out = self.write(
                        "POST",
                        &format!(
                            "/projects/{p}/decisions/delegations/{}/grant",
                            urlencode(id)
                        ),
                        &json!({"rev": g["rev"].as_str().unwrap_or(""), "by": by}),
                    )?;
                    if !out.ok() {
                        return Ok(out);
                    }
                    last = Some(out);
                }
                Ok(last.expect("at least one grant"))
            }
            Decision::SetMode(mode) => self.write(
                "PUT",
                &format!("/projects/{p}/decisions/mode"),
                &json!({"mode": mode, "by": by}),
            ),
            // The run expands a sweep into one Approve or Sign off per item the page offers it
            // for (`Running::sweep`), each through this same route.
            Decision::ApproveWaiting { .. } | Decision::SignoffWaiting { .. } => Err(anyhow!(
                "{} is taken item by item, as the page offers each",
                decision.kind()
            )),
            Decision::RecordManual {
                item: sel,
                requirement,
                test,
                passed,
            } => {
                let item = self
                    .resolve(project, sel)?
                    .ok_or_else(|| anyhow!("no item matches {}", sel.describe()))?;
                let waiting = self.manual_waiting(project)?;
                let row = waiting
                    .iter()
                    .find(|w| {
                        w["code"] == item.code.as_str()
                            && requirement
                                .as_ref()
                                .is_none_or(|r| w["requirement"] == r.as_str())
                            && w["test"]
                                .as_str()
                                .is_some_and(|t| norm(t).contains(&norm(test)))
                    })
                    .ok_or_else(|| {
                        anyhow!("no manual check like \"{test}\" waits on {}", item.code)
                    })?;
                let req = row["requirement"].as_str().unwrap_or("");
                let name = row["test"].as_str().unwrap_or("");
                self.write(
                    "PUT",
                    &format!(
                        "/projects/{p}/features/{}/tests/{}/{}",
                        urlencode(&item.code),
                        urlencode(req),
                        urlencode(name)
                    ),
                    &json!({"state": if *passed { "green" } else { "red" }, "by": by}),
                )
            }
        }
    }

    /// A sweep (`approve_waiting`, or `signoff_waiting` with `signoff` the sign-off's name): for
    /// each of `planned` (code and title) the page offers the decision for, take it through the
    /// page's own route, or with `pause` wait for the person to. Returns the items taken (`CODE
    /// "title"`) and the failures; an item the page does not offer it for is not waiting.
    #[allow(clippy::too_many_arguments)]
    pub fn sweep(
        &self,
        project: &str,
        planned: &[(String, String)],
        signoff: Option<&str>,
        offer: &Offer,
        step: &str,
        run: Option<&RunDirs>,
        pause: bool,
    ) -> (Vec<String>, Vec<String>) {
        let by = self.approver().unwrap_or_else(|_| "the person".into());
        let mut taken = Vec::new();
        let mut failed = Vec::new();
        for (code, title) in planned {
            let decision = match signoff {
                None => Decision::Approve(ItemSel::Code(code.clone())),
                Some(n) => Decision::Signoff {
                    item: ItemSel::Code(code.clone()),
                    name: n.to_string(),
                },
            };
            let item_offer = Offer {
                page: offer.page.clone(),
                button: offer.button.clone(),
                subject: Some(ItemSel::Code(code.clone())),
            };
            if !matches!(
                self.offers(project, &item_offer, Some(&decision)),
                Ok(Found::Offered)
            ) {
                continue;
            }
            if pause {
                println!(
                    "step {step}: waiting for you to press \"{}\" for {code} on the {} page",
                    offer.button, offer.page
                );
                match self.wait_for_change(
                    project,
                    &decision,
                    Duration::from_secs(5),
                    Duration::from_secs(60 * 60),
                ) {
                    Ok(true) => taken.push(format!("{code} \"{title}\"")),
                    Ok(false) => failed.push(format!("{code}: nothing changed within an hour")),
                    Err(e) => failed.push(format!("{code}: {e}")),
                }
                continue;
            }
            match self.decide(project, &decision, &by, step, run) {
                Ok(o) if o.ok() => {
                    eprintln!("step {step}: {} -> {}", o.route, o.status);
                    taken.push(format!("{code} \"{title}\""));
                }
                Ok(o) => failed.push(format!("{code}: {} answered {}", o.route, o.status)),
                Err(e) => failed.push(format!("{code}: {e}")),
            }
        }
        (taken, failed)
    }

    /// Before a sign-off: when the item shows **Approve** on the Review page because its approval
    /// lapsed or text was added since (`lapsed`, `added`), approve it again first, as the guide says
    /// ("Approve first, then sign off"). Returns the item's code when it approved; a never-approved
    /// item (`missing`) is left alone, since the runbook is to approve it.
    pub fn reapprove_if_lapsed(
        &self,
        project: &str,
        sel: &ItemSel,
        by: &str,
        step: &str,
    ) -> Result<Option<String>> {
        let Some(item) = self.resolve(project, sel)? else {
            return Ok(None);
        };
        let lapsed = self.review(project)?.into_iter().any(|e| {
            e["code"] == item.code.as_str()
                && matches!(e["approval"].as_str(), Some("lapsed" | "added"))
        });
        if !lapsed {
            return Ok(None);
        }
        let out = self.decide(
            project,
            &Decision::Approve(ItemSel::Code(item.code.clone())),
            by,
            step,
            None,
        )?;
        if !out.ok() {
            bail!(
                "approving {} again answered {}: {}",
                item.code,
                out.status,
                out.body
            );
        }
        Ok(Some(item.code))
    }

    /// What `--pause-at-decisions` polls: the record a decision changes, as the page reads it.
    pub fn fingerprint(&self, project: &str, decision: &Decision) -> Result<String> {
        let p = urlencode(project);
        let v = match decision {
            Decision::Approve(sel)
            | Decision::Ratify(sel)
            | Decision::Signoff { item: sel, .. }
            | Decision::RecordManual { item: sel, .. } => {
                let code = self
                    .resolve(project, sel)?
                    .map(|i| i.code)
                    .unwrap_or_default();
                let feature = self.feature(project, &code).unwrap_or(Value::Null);
                json!({"definition": feature["definition"], "status": feature["status"]})
            }
            Decision::Adr { .. } => self.get_json(&format!("/projects/{p}/adrs"))?,
            Decision::Promote { release, .. } | Decision::GateResult { release, .. } => {
                let releases = self.get_json(&format!("/projects/{p}/releases"))?;
                let plan = self.plan(project, release).unwrap_or(Value::Null);
                json!({"release": releases, "ready": plan["ready"], "to": plan["to"]})
            }
            Decision::ConfirmPrepared { .. } => json!(self.prepared_open(project)?),
            Decision::GrantDelegation(_) | Decision::SetMode(_) => self.decisions(project)?,
            // A sweep changes the items' definitions and sign-offs.
            Decision::ApproveWaiting { .. } | Decision::SignoffWaiting { .. } => {
                self.get_json(&format!("/projects/{p}"))?["features"].clone()
            }
        };
        Ok(v.to_string())
    }

    /// Wait until the record a decision changes has changed (the person acted on the monitor), or
    /// `timeout` passed. True when it changed.
    pub fn wait_for_change(
        &self,
        project: &str,
        decision: &Decision,
        poll: Duration,
        timeout: Duration,
    ) -> Result<bool> {
        let before = self.fingerprint(project, decision)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            std::thread::sleep(poll);
            if self.fingerprint(project, decision)? != before {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Take a screenshot of `url` into `file` with the configured command (`--screenshot-cmd`), or
    /// the first of chromium, google-chrome and chromium-browser on PATH, headless. `Ok(None)` and
    /// a note when there is none to run.
    pub fn screenshot(&self, url: &str, file: &Path, cmd: Option<&str>) -> Result<Option<PathBuf>> {
        let program = match cmd {
            Some(c) => Some(c.to_string()),
            None => ["chromium", "google-chrome", "chromium-browser"]
                .iter()
                .find(|c| which(c).is_some())
                .map(|c| c.to_string()),
        };
        let Some(program) = program else {
            return Ok(None);
        };
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let out = Command::new(&program)
            .args([
                "--headless=new",
                "--window-size=1400,1000",
                &format!("--screenshot={}", file.display()),
                url,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match out {
            Ok(s) if s.success() && file.exists() => Ok(Some(file.to_path_buf())),
            Ok(s) => Err(anyhow!("{program} exited with {s} taking the screenshot")),
            Err(e) => Err(anyhow!("could not run {program}: {e}")),
        }
    }

    /// Stop the monitor this run started (a monitor reached by its address is left running).
    pub fn stop(mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut()
            && child.try_wait().ok().flatten().is_none()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// `raw/prepared.tsv`: `time step id kind prepared_by confirmed_by edited`, one row per prepared
/// record the scripted person confirmed; `edited` is always `none` (nothing is ever typed).
pub fn append_prepared_row(
    run: &RunDirs,
    step: &str,
    id: &str,
    kind: &str,
    prepared_by: &str,
    confirmed_by: &str,
) -> Result<()> {
    std::fs::create_dir_all(run.raw())?;
    let path = run.raw().join("prepared.tsv");
    let new = !path.exists();
    let mut f = super::append_to(&path)?;
    if new {
        writeln!(f, "time\tstep\tid\tkind\tprepared_by\tconfirmed_by\tedited")?;
    }
    writeln!(
        f,
        "{}\t{step}\t{id}\t{kind}\t{prepared_by}\t{confirmed_by}\tnone",
        kanbanr_core::now_rfc3339()
    )?;
    Ok(())
}

/// Which of a release plan's buttons an offer names: the primary (`plan.button`), or the second
/// one a plan at a preparing stage whose preparation would change carries as `plan.again.button`
/// (DEF-057, "Promote to candidate again"). An exact label wins over a prefix, so "Promote to
/// candidate" never takes the `again` button and "Promote to candidate again" never takes the
/// primary. Each label is compared as served and with `release`'s own version token removed
/// (DEF-119): DEF-034's "Promote v0.1.0 to candidate" also reads as a guide names it, "Promote to
/// candidate". `Err` lists the labels the page offers instead.
pub fn release_button(
    plan: &Value,
    button: &str,
    release: &str,
) -> std::result::Result<(String, bool), Vec<String>> {
    let primary = plan["button"].as_str().unwrap_or("").to_string();
    let again = plan["again"]["button"]
        .as_str()
        .filter(|b| !b.is_empty())
        .map(str::to_string);
    let wanted = norm(button);
    let mut labels: Vec<(String, bool)> = Vec::new();
    if !primary.is_empty() {
        labels.push((primary, false));
    }
    if let Some(a) = again {
        labels.push((a, true));
    }
    let version = norm(release);
    // A label as served, then without the release's version token.
    let forms = |l: &str| -> [String; 2] {
        let served = norm(l);
        let unversioned = served
            .split(' ')
            .filter(|t| version.is_empty() || *t != version)
            .collect::<Vec<_>>()
            .join(" ");
        [served, unversioned]
    };
    if let Some((l, is_again)) = labels.iter().find(|(l, _)| forms(l).contains(&wanted)) {
        return Ok((l.clone(), *is_again));
    }
    if let Some((l, is_again)) = labels.iter().find(|(l, _)| {
        forms(l)
            .iter()
            .any(|f| !f.is_empty() && (f.starts_with(&wanted) || wanted.starts_with(f.as_str())))
    }) {
        return Ok((l.clone(), *is_again));
    }
    Err(labels.into_iter().map(|(l, _)| l).collect())
}

/// Is the button an offer names offered for `release`, as the Releases page would show it from
/// `plan` (`GET /projects/{p}/releases/{v}/plan`)? The primary button is offered when the plan is
/// ready with the repository and no tag result blocks it, or when the cut waits only for the
/// person's acceptance of untested requirements; the `again` button when its own `ready` is true.
pub fn release_offer(plan: &Value, release: &str, button: &str) -> Found {
    if plan["empty"] == true {
        return Found::Absent(format!("{release} holds nothing"));
    }
    let (label, is_again) = match release_button(plan, button, release) {
        Ok(found) => found,
        Err(labels) => {
            let offered: Vec<String> = labels.iter().map(|l| format!("\"{l}\"")).collect();
            return Found::Absent(format!(
                "the Releases page offers {} for {release}, not \"{button}\"",
                if offered.is_empty() {
                    "no button".to_string()
                } else {
                    offered.join(" and ")
                }
            ));
        }
    };
    if is_again {
        return if plan["again"]["ready"] == true {
            Found::Offered
        } else {
            let missing: Vec<String> = plan["again"]["missing"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            Found::Absent(format!(
                "\"{label}\" is greyed out for {release}: {}",
                missing.join("; ")
            ))
        };
    }
    let ready = plan["ready"] == true && plan["repo"] == true;
    let tag_blocked = plan["confirm"] == true
        && plan["tag"].is_object()
        && plan["tag"]["gate"].as_str().is_some_and(|g| !g.is_empty())
        && plan["tag"]["state"] != "passed";
    let accept_only = accept_untested(plan).is_some();
    if (ready && !tag_blocked) || accept_only {
        Found::Offered
    } else {
        let missing: Vec<String> = plan["conditions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["holds"] != true)
            .map(|c| {
                format!(
                    "{}: {}",
                    c["check"].as_str().unwrap_or(""),
                    c["detail"].as_str().unwrap_or("")
                )
            })
            .chain(
                plan["missing"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            )
            .collect();
        Found::Absent(format!(
            "\"{label}\" is greyed out for {release}: {}",
            missing.join("; ")
        ))
    }
}

/// The commit a release's page records a CI result for: the release tip while `gate_passed` is
/// unmet, else the tag's commit while its result is not passed.
fn gate_commit(plan: &Value) -> Option<String> {
    let unmet_gate = plan["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|c| c["check"] == "gate_passed" && c["holds"] != true);
    if unmet_gate && let Some(tip) = plan["release_tip"].as_str().filter(|t| !t.is_empty()) {
        return Some(tip.to_string());
    }
    if plan["tag"].is_object()
        && plan["tag"]["state"] != "passed"
        && plan["tag"]["gate"].as_str().is_some_and(|g| !g.is_empty())
    {
        return plan["tag"]["commit"].as_str().map(str::to_string);
    }
    None
}

/// The requirements the cut waits to see accepted untested (DEF-043), when that is all it waits for.
fn accept_untested(plan: &Value) -> Option<Vec<String>> {
    let unmet: Vec<&Value> = plan["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["holds"] != true)
        .collect();
    let untested = unmet.iter().find(|c| c["check"] == "untested_accepted")?;
    if !unmet.iter().all(|c| c["check"] == "untested_accepted") || plan["repo"] != true {
        return None;
    }
    let refs: Vec<String> = untested["refs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    (!refs.is_empty()).then_some(refs)
}

/// The item a selector picks from the board's items (oldest first).
/// The item a selector picks, when exactly one fits (the tests' view).
#[cfg(test)]
pub fn select(items: &[ItemBrief], sel: &ItemSel) -> Option<ItemBrief> {
    match pick(items, sel) {
        Pick::One(i) => Some(i),
        _ => None,
    }
}

/// The item a selector picks, or every candidate when several fit equally well (RC6).
pub fn pick(items: &[ItemBrief], sel: &ItemSel) -> Pick<ItemBrief> {
    match sel.pick(items, |i| &i.code, |i| &i.title) {
        Pick::One(i) => Pick::One(i.clone()),
        Pick::None => Pick::None,
        Pick::Ambiguous(v) => Pick::Ambiguous(v.into_iter().cloned().collect()),
    }
}

/// Lowercased, single-spaced.
fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// `s` as a path segment.
pub fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Does a concrete path fit a template with `{…}` segments?
pub fn path_matches(template: &str, path: &str) -> bool {
    let bare = path.split('?').next().unwrap_or(path);
    let t: Vec<&str> = template.split('/').filter(|s| !s.is_empty()).collect();
    let p: Vec<&str> = bare.split('/').filter(|s| !s.is_empty()).collect();
    t.len() == p.len() && t.iter().zip(&p).all(|(a, b)| a.starts_with('{') || a == b)
}

/// The page token the monitor put in its page (DEF-024).
pub fn page_token(html: &str) -> Option<String> {
    let at = html.find("name=\"kanbanr-token\"")?;
    let rest = &html[at..];
    let content = rest.find("content=\"")? + "content=\"".len();
    let rest = &rest[content..];
    let end = rest.find('"')?;
    let token = &rest[..end];
    (!token.is_empty()).then(|| token.to_string())
}

fn which(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

// ---- the browser (W4b) --------------------------------------------------------------------

/// Chromium or Chrome is not on this machine: every `press` and `expect.page` step is a finding
/// that says so, and the run goes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoBrowser(pub String);

impl std::fmt::Display for NoBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no browser: {}", self.0)
    }
}

impl std::error::Error for NoBrowser {}

/// Is this error the absence of a browser?
pub fn is_no_browser(e: &anyhow::Error) -> bool {
    e.downcast_ref::<NoBrowser>().is_some()
}

/// A headless Chromium or Chrome over the DevTools protocol (the `chromiumoxide` crate, async on a
/// tokio runtime of its own; this API stays synchronous for the run loop).
pub struct Browser {
    rt: tokio::runtime::Runtime,
    inner: chromiumoxide::Browser,
    handler: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Browser")
    }
}

/// One rendered page.
pub struct Page<'a> {
    rt: &'a tokio::runtime::Runtime,
    page: chromiumoxide::Page,
}

/// The browser to drive: `KANBANR_WALKTHROUGH_CHROME`, else `CHROME`, else the first of the usual
/// names on PATH (chromiumoxide's detection, Edge included).
pub fn find_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KANBANR_WALKTHROUGH_CHROME")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(p);
    }
    chromiumoxide::detection::default_executable(Default::default()).ok()
}

impl Browser {
    /// Open a headless browser, or `NoBrowser` when none is on the machine.
    pub fn open() -> Result<Browser> {
        Browser::open_at(find_browser())
    }

    /// Open the browser at `path`; `None` is the finding, not an error of any other kind.
    pub fn open_at(path: Option<PathBuf>) -> Result<Browser> {
        let Some(path) = path.filter(|p| p.is_file()) else {
            return Err(NoBrowser(
                "neither chromium, chromium-browser nor google-chrome is on PATH (set \
                 KANBANR_WALKTHROUGH_CHROME to name one)"
                    .into(),
            )
            .into());
        };
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let config = chromiumoxide::BrowserConfig::builder()
            .chrome_executable(&path)
            .new_headless_mode()
            .window_size(1400, 1000)
            .launch_timeout(Duration::from_secs(60))
            .request_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow!("browser options: {e}"))?;
        let (inner, mut handler) = rt
            .block_on(chromiumoxide::Browser::launch(config))
            .map_err(|e| NoBrowser(format!("{} would not start: {e}", path.display())))?;
        // The handler drives the DevTools connection for as long as the browser lives.
        let handler = rt.spawn(async move {
            use futures::StreamExt;
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });
        Ok(Browser { rt, inner, handler })
    }

    /// Open `url` in a new tab and wait for its text to settle (the monitor renders after load).
    pub fn page(&self, url: &str) -> Result<Page<'_>> {
        let page = self
            .rt
            .block_on(async {
                let page = self.inner.new_page(url).await?;
                page.wait_for_navigation().await?;
                Ok::<_, chromiumoxide::error::CdpError>(page)
            })
            .map_err(|e| anyhow!("open {url}: {e}"))?;
        let page = Page { rt: &self.rt, page };
        page.settle(Duration::from_secs(10))?;
        Ok(page)
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.rt.block_on(async {
            let _ = self.inner.close().await;
            let _ = self.inner.wait().await;
        });
        self.handler.abort();
    }
}

impl Page<'_> {
    /// Wait until two reads of the page's text, 400 ms apart, agree, or `max` has passed.
    fn settle(&self, max: Duration) -> Result<()> {
        let deadline = Instant::now() + max;
        let mut last = self.text()?;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(400));
            let now = self.text()?;
            if now == last && !now.trim().is_empty() {
                return Ok(());
            }
            last = now;
        }
        Ok(())
    }

    /// Run `js` as an expression (never as a function to call) and read its string value.
    fn eval_string(&self, js: &str) -> Result<String> {
        use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
        let params = EvaluateParams::builder()
            .expression(js)
            .return_by_value(true)
            .build()
            .map_err(|e| anyhow!("page script: {e}"))?;
        let out = self
            .rt
            .block_on(
                self.page
                    .evaluate(chromiumoxide::js::Evaluation::from(params)),
            )
            .map_err(|e| anyhow!("page script: {e}"))?;
        Ok(out
            .value()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    /// The rendered text, as a person reads it.
    pub fn text(&self) -> Result<String> {
        self.eval_string("document.body ? document.body.innerText : ''")
    }

    /// Click the button whose label starts with `button`, within the card that names `subject`
    /// when one is given. With no such button, a checkbox whose accessible name starts with
    /// `button` is clicked instead (FTR-063 T7: "Select all"); a button always wins over a
    /// checkbox. An error names what the page offered instead.
    pub fn press(&self, button: &str, subject: Option<&str>) -> Result<()> {
        let js = format!(
            r#"(function(label, subject) {{
  const norm = s => (s || '').replace(/\s+/g, ' ').trim().toLowerCase();
  const all = Array.from(document.querySelectorAll('button'));
  const boxName = c => {{
    if (c.getAttribute('aria-label')) return c.getAttribute('aria-label');
    const by = c.getAttribute('aria-labelledby');
    if (by) return by.split(/\s+/).map(id => (document.getElementById(id) || {{}}).innerText || '').join(' ');
    if (c.labels && c.labels.length) return Array.from(c.labels).map(l => l.innerText).join(' ');
    return c.title || c.innerText || '';
  }};
  const boxes = Array.from(document.querySelectorAll('input[type=checkbox], [role=checkbox]'));
  const inCardOrAll = list => subject ? list.find(inCard) : list[0];
  const named = all.filter(b => norm(b.innerText).startsWith(norm(label)));
  const inCard = b => {{
    let e = b.parentElement;
    while (e && e !== document.body) {{
      const cls = (e.className || '') + ' ' + e.tagName;
      if (/review-item|tile|release-action|manual-check|decisions-here|DETAILS|SECTION|LI/.test(cls)
          && (e.innerText || '').includes(subject)) return true;
      e = e.parentElement;
    }}
    return false;
  }};
  const chosen = inCardOrAll(named)
    || inCardOrAll(boxes.filter(c => norm(boxName(c)).startsWith(norm(label))));
  if (!chosen) return 'none: ' + all.map(b => norm(b.innerText))
    .concat(boxes.map(c => norm(boxName(c))).filter(Boolean).map(n => n + ' (checkbox)'))
    .filter(Boolean).join(' | ');
  if (chosen.disabled || chosen.getAttribute('aria-disabled') === 'true')
    return 'disabled: ' + (chosen.title || '');
  chosen.click();
  return 'clicked';
}})({label}, {subject})"#,
            label = Value::String(button.to_string()),
            subject = match subject {
                Some(s) => Value::String(s.to_string()).to_string(),
                None => "null".to_string(),
            }
        );
        let said = self.eval_string(&js)?;
        if said == "clicked" {
            Ok(())
        } else if let Some(rest) = said.strip_prefix("none: ") {
            Err(anyhow!(
                "no button named \"{button}\"{} on the page; it offers: {rest}",
                subject.map(|s| format!(" for {s}")).unwrap_or_default()
            ))
        } else {
            Err(anyhow!("the button \"{button}\" is {said}"))
        }
    }

    /// Make the presses in turn on this page (FTR-063): a button that asks first, then its
    /// confirmation or Cancel. The page settles (at most 5 s) before each later press. The first
    /// press that fails stops the list; with several, its error names its place (`press 2 of 2: `).
    pub fn press_in_turn(&self, presses: &[(&str, Option<&str>)]) -> Result<()> {
        let n = presses.len();
        for (i, (button, subject)) in presses.iter().enumerate() {
            if i > 0 {
                self.settle(Duration::from_secs(5))?;
            }
            if let Err(e) = self.press(button, *subject) {
                return Err(if n > 1 {
                    anyhow!("press {} of {n}: {e}", i + 1)
                } else {
                    e
                });
            }
        }
        Ok(())
    }

    /// Save the page as a PNG.
    pub fn screenshot(&self, file: &Path) -> Result<()> {
        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
        use chromiumoxide::page::ScreenshotParams;
        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(true)
            .build();
        let png = self
            .rt
            .block_on(self.page.screenshot(params))
            .map_err(|e| anyhow!("screenshot: {e}"))?;
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        kanbanr_core::atomic::write(file, png)?;
        Ok(())
    }
}

/// What a page claim found: each text it wanted and did not see, each it saw and should not have.
pub fn check_page_claim(text: &str, claim: &PageClaim) -> Vec<String> {
    let have = norm(text);
    let mut out = Vec::new();
    for want in &claim.contains {
        if !have.contains(&norm(want)) {
            out.push(format!("the page does not show \"{want}\""));
        }
    }
    for not in &claim.absent {
        if have.contains(&norm(not)) {
            out.push(format!("the page shows \"{not}\", which it should not"));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::walkthrough::guard::RunDirs;

    /// The small monitor page the tests serve (`tests/fixtures/walkthrough-ui`): it carries the
    /// token the daemon injects and renders the Review page's items with an Approve button each.
    fn ui_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("walkthrough-ui")
    }

    /// A free loopback port, picked and released; the daemon binds it a moment later.
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// A run under the temporary directory whose board has one project with one item, defined and
    /// ready to approve, served with writes on by the daemon in this process (the unit tests of a
    /// binary have no built `kanbanr` to spawn); the monitor is reached by its address.
    pub(crate) fn scratch_board(name: &str) -> (RunDirs, PathBuf, Monitor) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "kanbanr-wt-person-{}-{}-{name}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&base);
        let run = RunDirs::new(&base.join("runs").join(name));
        run.make().unwrap();
        let board = run.board();
        // The board is the run's own, inside the fence, before anything is written to it.
        assert!(project::normalize(&board).starts_with(project::normalize(&base)));
        kanbanr_core::project::assert_inside_fence(&board);
        let store = kanbanr_core::Store::new(&board);
        store
            .init_project(
                "shop",
                kanbanr_core::config::ProjectConfig::default_for("shop"),
            )
            .unwrap();
        kanbanr_core::git::ensure_repo_as(&board, Some(("Tester", "t@example.com")));
        store
            .add_milestone("shop", "M1", "", vec![], Some("MS-1".into()))
            .unwrap();
        let f = store
            .add_feature("shop", "Saved basket", "spec", "MS-1", None)
            .unwrap();
        let def: kanbanr_core::models::FeatureDefinition = kanbanr_core::yaml::from_str(
            "statement: A shopper's basket survives between visits\n\
             requirements:\n\
             \x20 - id: R-1\n\
             \x20   text: WHEN a shopper returns THE SYSTEM SHALL show the basket they left\n\
             \x20   tests:\n\
             \x20     - name: 'manual: the person judges whether the basket they left reads clearly'\n\
             \x20       kind: manual\n",
        )
        .unwrap();
        store
            .set_feature_definition("shop", &f.code, Some(def))
            .unwrap();
        assert_eq!(f.code, "FEAT-001");
        let port = free_port();
        let bind = format!("127.0.0.1:{port}");
        let data_dir = board.clone();
        let ui = ui_dir().display().to_string();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let _ = rt.block_on(kanbanr_server::run_with(
                data_dir,
                bind,
                Some(ui),
                true,
                None,
                false,
                None,
            ));
        });
        let url = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(20);
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(2))
            .build();
        while Instant::now() < deadline {
            if agent.get(&format!("{url}/api/meta")).call().is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let monitor = Monitor::connect(&url).unwrap();
        (run, base, monitor)
    }

    use kanbanr_core::project;

    fn approve_offer() -> Offer {
        Offer {
            page: "review".into(),
            button: "Approve".into(),
            subject: Some(ItemSel::TitleContains("basket".into())),
        }
    }

    /// FTR-037 R-3: a decision goes through the write route the page posts to, as the scripted
    /// person, with the page's body; the board records it as taken on the monitor.
    #[test]
    fn a_decision_goes_through_the_write_route_as_the_scripted_person() {
        let (run, base, monitor) = scratch_board("decide");
        assert!(
            monitor.has_token(),
            "the page token is read from the served page"
        );
        let by = monitor.approver().unwrap();
        assert_eq!(by, "Tester");
        assert_eq!(
            monitor.offers("shop", &approve_offer(), None).unwrap(),
            Found::Offered
        );
        let out = monitor
            .decide(
                "shop",
                &Decision::Approve(ItemSel::Newest),
                &by,
                "4.2",
                Some(&run),
            )
            .unwrap();
        assert!(out.ok(), "{out:?}");
        assert_eq!(out.route, "POST /projects/shop/features/FEAT-001/approve");
        let feature = monitor.feature("shop", "FEAT-001").unwrap();
        assert_eq!(feature["definition"]["approval"]["by"], "Tester");
        assert_eq!(feature["definition"]["approval"]["origin"], "monitor");
        // Once approved, the offer is gone and says what the page shows instead.
        match monitor.offers("shop", &approve_offer(), None).unwrap() {
            Found::Absent(why) => assert!(why.contains("not on the Review page"), "{why}"),
            Found::Offered => panic!("still offered"),
        }
        // A route the pages do not post to is never posted.
        let err = monitor
            .write("POST", "/projects/shop/features/FEAT-001/move", &json!({}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a route"), "{err}");
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Preview-7 triage RC3: `approve_waiting` approves, one by one through the page's own route,
    /// each planned item whose approval the Review page offers (the follow-ups Claude filed), and
    /// leaves alone one already approved; a second sweep finds nothing waiting.
    #[test]
    fn a_sweep_approves_every_waiting_item_the_page_offers() {
        let (run, base, monitor) = scratch_board("sweep");
        let store = kanbanr_core::Store::new(run.board());
        let f2 = store
            .add_feature("shop", "Chore: run the formatter", "spec", "MS-1", None)
            .unwrap();
        let def: kanbanr_core::models::FeatureDefinition = kanbanr_core::yaml::from_str(
            "statement: The code is formatted\n\
             requirements:\n\
             \x20 - id: R-1\n\
             \x20   text: THE SYSTEM SHALL pass the formatter check\n\
             \x20   tests:\n\
             \x20     - name: fmt_check\n\
             \x20       kind: unit\n",
        )
        .unwrap();
        store
            .set_feature_definition("shop", &f2.code, Some(def))
            .unwrap();
        let by = monitor.approver().unwrap();
        let first = monitor
            .decide(
                "shop",
                &Decision::Approve(ItemSel::Code("FEAT-001".into())),
                &by,
                "6.3",
                Some(&run),
            )
            .unwrap();
        assert!(first.ok(), "{first:?}");
        let planned = vec![
            ("FEAT-001".to_string(), "Saved basket".to_string()),
            (f2.code.clone(), f2.title.clone()),
        ];
        let offer = Offer {
            page: "review".into(),
            button: "Approve".into(),
            subject: None,
        };
        let (taken, failed) =
            monitor.sweep("shop", &planned, None, &offer, "6.3a", Some(&run), false);
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(taken, [format!("{} \"{}\"", f2.code, f2.title)]);
        let feature = monitor.feature("shop", &f2.code).unwrap();
        assert_eq!(feature["definition"]["approval"]["origin"], "monitor");
        let (again, _) = monitor.sweep("shop", &planned, None, &offer, "6.3a", Some(&run), false);
        assert!(again.is_empty(), "{again:?}");
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-037 R-3: an offer the guide names that the page does not show is reported as absent,
    /// with what the page shows instead, and nothing is posted; the run goes on.
    #[test]
    fn a_missing_offer_is_a_finding_and_the_run_goes_on() {
        let (_run, base, monitor) = scratch_board("offer");
        let signoff = Offer {
            page: "review".into(),
            button: "Sign off design-review".into(),
            subject: Some(ItemSel::Code("FEAT-001".into())),
        };
        let found = monitor.offers("shop", &signoff, None).unwrap();
        let Found::Absent(why) = found else {
            panic!("a sign-off is not offered before the approval")
        };
        assert!(why.contains("FEAT-001"), "{why}");
        let nobody = Offer {
            page: "review".into(),
            button: "Approve".into(),
            subject: Some(ItemSel::TitleContains("checkout".into())),
        };
        let Found::Absent(why) = monitor.offers("shop", &nobody, None).unwrap() else {
            panic!("no such item")
        };
        assert!(why.contains("no item matches"), "{why}");
        let promote = Offer {
            page: "releases".into(),
            button: "Promote to candidate".into(),
            subject: None,
        };
        let Found::Absent(why) = monitor
            .offers(
                "shop",
                &promote,
                Some(&Decision::Promote {
                    release: "v0.1.0".into(),
                    to: None,
                }),
            )
            .unwrap_or(Found::Absent("no plan".into()))
        else {
            panic!("no release exists")
        };
        assert!(!why.is_empty());
        // A plan with DEF-057's second button: the offer naming it is found, the primary still is,
        // and an absent offer names both.
        let plan = json!({
            "version": "v0.1.0", "button": "Promote to candidate", "ready": true, "repo": true,
            "empty": false, "confirm": false, "conditions": [], "missing": [],
            "again": {"stage": "candidate", "button": "Promote to candidate again", "does": [], "ready": true, "missing": []}
        });
        assert_eq!(
            release_offer(&plan, "v0.1.0", "Promote to candidate again"),
            Found::Offered
        );
        assert_eq!(
            release_offer(&plan, "v0.1.0", "Promote to candidate"),
            Found::Offered
        );
        let Found::Absent(why) = release_offer(&plan, "v0.1.0", "Cut v0.1.0") else {
            panic!("the cut is not offered")
        };
        assert!(
            why.contains("\"Promote to candidate\" and \"Promote to candidate again\""),
            "{why}"
        );
        // The board is untouched: the item is still unapproved.
        let feature = monitor.feature("shop", "FEAT-001").unwrap();
        assert!(
            feature["definition"]["approval"]["by"]
                .as_str()
                .unwrap_or("")
                .is_empty()
        );
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-037 R-9: with `--pause-at-decisions`, the run waits until the record the decision changes
    /// has changed on the board (the person acted on the monitor), then goes on; it does not wait
    /// forever when nothing changes.
    #[test]
    fn pause_at_decisions_waits_for_the_board_to_change() {
        let (_run, base, monitor) = scratch_board("pause");
        let decision = Decision::Approve(ItemSel::Code("FEAT-001".into()));
        assert!(
            !monitor
                .wait_for_change(
                    "shop",
                    &decision,
                    Duration::from_millis(100),
                    Duration::from_millis(600)
                )
                .unwrap(),
            "nothing changed, so the wait ends false at its timeout"
        );
        let by = monitor.approver().unwrap();
        let url = monitor.url.clone();
        let token_page = monitor.get_json("/meta").unwrap();
        assert_eq!(token_page["writes"], true);
        // The person acts after a moment, on the monitor.
        let acting = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            let agent = ureq::AgentBuilder::new().build();
            let html = agent
                .get(&format!("{url}/"))
                .call()
                .unwrap()
                .into_string()
                .unwrap();
            let token = page_token(&html).unwrap();
            agent
                .post(&format!(
                    "{url}/api/write/projects/shop/features/FEAT-001/approve"
                ))
                .set("content-type", "application/json")
                .set("x-kanbanr-token", &token)
                .send_string(&json!({"by": by}).to_string())
                .unwrap();
        });
        let started = Instant::now();
        let changed = monitor
            .wait_for_change(
                "shop",
                &decision,
                Duration::from_millis(100),
                Duration::from_secs(20),
            )
            .unwrap();
        acting.join().unwrap();
        assert!(changed, "the wait ends when the board changed");
        assert!(started.elapsed() < Duration::from_secs(15));
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-037 W4: every write path in `web/src/api.ts` has a row in the route table, so the
    /// scripted person can only ever post what a page posts, and a page route added without a row
    /// fails here.
    #[test]
    fn every_monitor_write_path_has_a_row() {
        let api = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/api.ts"),
        )
        .unwrap();
        let generic = |t: &str| -> String {
            t.split('/')
                .map(|s| if s.starts_with('{') { "{}" } else { s })
                .collect::<Vec<_>>()
                .join("/")
        };
        let rows: Vec<String> = WRITE_ROUTES.iter().map(|(_, t)| generic(t)).collect();
        let mut found = 0;
        let mut rest = api.as_str();
        while let Some(at) = rest.find("`/") {
            let after = &rest[at + 1..];
            let Some(end) = after.find('`') else { break };
            let template = &after[..end];
            rest = &after[end + 1..];
            if !template.starts_with("/write/projects") && !template.starts_with("/projects") {
                continue;
            }
            // Only templates passed to a write: `writePost(` or `postWrite(apiUrl(`/write…
            let before = &api[..api.len() - rest.len() - template.len() - 2];
            let tail = &before[before.len().saturating_sub(120)..];
            let is_write = template.starts_with("/write/") || tail.contains("writePost(");
            if !is_write {
                continue;
            }
            let path = template.trim_start_matches("/write");
            // `${encodeURIComponent(x)}` and any other `${…}` are path parameters; a literal
            // `${action}` segment stands for approve|unapprove|ratify.
            let mut generic_path = String::new();
            let mut s = path;
            while let Some(a) = s.find("${") {
                generic_path.push_str(&s[..a]);
                let Some(b) = s[a..].find('}') else { break };
                let inner = &s[a + 2..a + b];
                generic_path.push_str(match inner {
                    "action" => "{action}",
                    "verdict" => "{verdict}",
                    _ => "{}",
                });
                s = &s[a + b + 1..];
            }
            generic_path.push_str(s);
            let candidates: Vec<String> = if generic_path.contains("{action}") {
                ["approve", "unapprove", "ratify"]
                    .iter()
                    .map(|a| generic_path.replace("{action}", a))
                    .collect()
            } else if generic_path.contains("{verdict}") {
                ["accept", "reject"]
                    .iter()
                    .map(|a| generic_path.replace("{verdict}", a))
                    .collect()
            } else {
                vec![generic_path.clone()]
            };
            for c in candidates {
                let c = c.replace("/{verdict}", "/{}");
                assert!(
                    rows.contains(&c),
                    "api.ts posts to {c} (from {template}), which has no row in WRITE_ROUTES"
                );
                found += 1;
            }
        }
        assert!(
            found >= 18,
            "only {found} write paths were read from api.ts"
        );
        assert!(path_matches(
            "/projects/{p}/features/{code}/approve",
            "/projects/shop/features/FEAT-001/approve"
        ));
        assert!(!path_matches(
            "/projects/{p}/features/{code}/approve",
            "/projects/shop/features/FEAT-001/move"
        ));
    }

    /// FTR-037 R-3 (W4b): without a browser on the machine, opening one is the `NoBrowser`
    /// finding, which the run records for every press and page step and goes on; it is told from
    /// any other error.
    #[test]
    fn no_browser_makes_page_steps_findings() {
        let bogus =
            std::env::temp_dir().join(format!("kanbanr-wt-no-chrome-{}", std::process::id()));
        let err = Browser::open_at(Some(bogus)).unwrap_err();
        assert!(is_no_browser(&err), "{err}");
        assert!(err.to_string().starts_with("no browser:"), "{err}");
        let err = Browser::open_at(None).unwrap_err();
        assert!(is_no_browser(&err));
        assert!(!is_no_browser(&anyhow!("something else")));
        // A page claim with nothing to read against is the same finding; with text, each want.
        let claim = PageClaim {
            url: "/p/shop/review".into(),
            contains: vec!["Sign off release".into()],
            absent: vec!["Approve".into()],
        };
        let missing = check_page_claim("Saved basket  Approve FEAT-001", &claim);
        assert_eq!(missing.len(), 2, "{missing:?}");
        assert!(check_page_claim("Sign off release", &claim).is_empty());
    }

    /// FTR-037 R-3 (W4b): the rendered text of the monitor's page is read through the browser;
    /// without a browser on this machine the test exercises the `NoBrowser` path instead and says so.
    #[test]
    fn a_page_claim_reads_the_rendered_text() {
        let (_run, base, monitor) = scratch_board("page");
        match Browser::open() {
            Err(e) if is_no_browser(&e) => {
                eprintln!("no browser on this machine: the page claim is the finding {e}");
            }
            Err(e) => panic!("{e}"),
            Ok(browser) => {
                let page = browser.page(&monitor.page_url("shop", "review")).unwrap();
                let text = page.text().unwrap();
                assert!(text.contains("FEAT-001"), "{text}");
                assert!(text.contains("Saved basket"), "{text}");
                let claim = PageClaim {
                    url: "/p/shop/review".into(),
                    contains: vec!["Approve FEAT-001".into()],
                    absent: vec!["Sign off".into()],
                };
                assert!(check_page_claim(&text, &claim).is_empty(), "{text}");
                let shot = base.join("review.png");
                page.screenshot(&shot).unwrap();
                assert!(std::fs::metadata(&shot).unwrap().len() > 1000);
            }
        }
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-037 R-3 (W4b): a `press` step clicks the named button within the subject's card on the
    /// rendered page, and the board changes as the page's own click would change it; without a
    /// browser the test exercises the `NoBrowser` path and says so.
    #[test]
    fn a_press_takes_the_decision_through_the_page() {
        let (_run, base, monitor) = scratch_board("press");
        match Browser::open() {
            Err(e) if is_no_browser(&e) => {
                eprintln!("no browser on this machine: the press is the finding {e}");
            }
            Err(e) => panic!("{e}"),
            Ok(browser) => {
                let page = browser.page(&monitor.page_url("shop", "review")).unwrap();
                let err = page
                    .press("Sign off release", Some("FEAT-001"))
                    .unwrap_err();
                assert!(err.to_string().contains("no button named"), "{err}");
                page.press("Approve", Some("FEAT-001")).unwrap();
                let decision = Decision::Approve(ItemSel::Code("FEAT-001".into()));
                // The click posts asynchronously; the board changes within a moment.
                let deadline = Instant::now() + Duration::from_secs(20);
                let mut approved = false;
                while Instant::now() < deadline {
                    let feature = monitor.feature("shop", "FEAT-001").unwrap();
                    if feature["definition"]["approval"]["by"] == "Tester" {
                        approved = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                assert!(
                    approved,
                    "the press approved FEAT-001 as the monitor's identity"
                );
                let feature = monitor.feature("shop", "FEAT-001").unwrap();
                assert_eq!(feature["definition"]["approval"]["origin"], "monitor");
                assert!(!monitor.fingerprint("shop", &decision).unwrap().is_empty());
            }
        }
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Wait up to `max` for FEAT-001 on the scratch board to be approved as the monitor's identity.
    fn approved_within(monitor: &Monitor, max: Duration) -> bool {
        let deadline = Instant::now() + max;
        while Instant::now() < deadline {
            let feature = monitor.feature("shop", "FEAT-001").unwrap();
            if feature["definition"]["approval"]["by"] == "Tester" {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    /// FTR-063 R-3: on a page whose Approve asks first, two presses in turn on one page approve
    /// the item as the monitor's identity (that the first alone approves nothing is R-4's proof);
    /// without a browser the test exercises the `NoBrowser` path and says so.
    #[test]
    fn two_presses_on_one_page_confirm_what_the_first_asked() {
        let (_run, base, monitor) = scratch_board("ask-twice");
        match Browser::open() {
            Err(e) if is_no_browser(&e) => {
                eprintln!("no browser on this machine: the presses are the finding {e}");
            }
            Err(e) => panic!("{e}"),
            Ok(browser) => {
                let page = browser.page(&monitor.page_url("shop", "ask")).unwrap();
                page.press_in_turn(&[("Approve", Some("FEAT-001")), ("Approve", Some("FEAT-001"))])
                    .unwrap();
                assert!(
                    approved_within(&monitor, Duration::from_secs(20)),
                    "the second press approved FEAT-001 as the monitor's identity"
                );
                let feature = monitor.feature("shop", "FEAT-001").unwrap();
                assert_eq!(feature["definition"]["approval"]["origin"], "monitor");
            }
        }
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-063 R-4: Cancel after the asking press changes nothing: the item is still not
    /// approved, and the card again offers Approve without the question.
    #[test]
    fn cancel_after_the_first_press_changes_nothing() {
        let (_run, base, monitor) = scratch_board("ask-cancel");
        match Browser::open() {
            Err(e) if is_no_browser(&e) => {
                eprintln!("no browser on this machine: the presses are the finding {e}");
            }
            Err(e) => panic!("{e}"),
            Ok(browser) => {
                let page = browser.page(&monitor.page_url("shop", "ask")).unwrap();
                page.press_in_turn(&[("Approve", Some("FEAT-001")), ("Cancel", Some("FEAT-001"))])
                    .unwrap();
                assert!(
                    !approved_within(&monitor, Duration::from_secs(3)),
                    "Cancel left FEAT-001 unapproved"
                );
                let text = page.text().unwrap();
                assert!(text.contains("Approve FEAT-001"), "{text}");
                assert!(!text.contains("This records your approval"), "{text}");
            }
        }
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-063 R-5: a later press the page does not offer stops the list with its place and what
    /// the page offered, and the board is unchanged.
    #[test]
    fn a_later_press_the_page_does_not_offer_names_its_place() {
        let (_run, base, monitor) = scratch_board("ask-missing");
        match Browser::open() {
            Err(e) if is_no_browser(&e) => {
                eprintln!("no browser on this machine: the presses are the finding {e}");
            }
            Err(e) => panic!("{e}"),
            Ok(browser) => {
                let page = browser.page(&monitor.page_url("shop", "ask")).unwrap();
                let err = page
                    .press_in_turn(&[
                        ("Approve", Some("FEAT-001")),
                        ("Sign off release", Some("FEAT-001")),
                    ])
                    .unwrap_err()
                    .to_string();
                assert!(
                    err.starts_with("press 2 of 2: no button named \"Sign off release\""),
                    "{err}"
                );
                assert!(
                    err.contains("cancel"),
                    "the offered buttons are named: {err}"
                );
                assert!(
                    !approved_within(&monitor, Duration::from_secs(3)),
                    "the board is unchanged"
                );
            }
        }
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FTR-063 R-3 (T7, for FTR-038's group view): a press names a checkbox by its accessible
    /// name, its label's text here, and ticks it; the group's button then reads "Approve 1" for
    /// the one waiting item, and a card's Approve is still the card's button. Without a browser
    /// the test exercises the `NoBrowser` path and says so.
    #[test]
    fn a_press_ticks_a_checkbox_by_its_name() {
        let (_run, base, monitor) = scratch_board("tick");
        match Browser::open() {
            Err(e) if is_no_browser(&e) => {
                eprintln!("no browser on this machine: the press is the finding {e}");
            }
            Err(e) => panic!("{e}"),
            Ok(browser) => {
                let page = browser.page(&monitor.page_url("shop", "review")).unwrap();
                let ticked = "String(Array.from(document.querySelectorAll('input[type=checkbox]'))\
                              .some(c => c.checked))";
                assert_eq!(page.eval_string(ticked).unwrap(), "false");
                assert!(!page.text().unwrap().contains("Approve 1"));
                page.press("Select all", None).unwrap();
                assert_eq!(page.eval_string(ticked).unwrap(), "true");
                page.settle(Duration::from_secs(5)).unwrap();
                let text = page.text().unwrap();
                assert!(text.contains("Approve 1"), "{text}");
                // A press that matches a button takes the button: the card's Approve, not the box.
                page.press("Approve", Some("FEAT-001")).unwrap();
                assert!(
                    approved_within(&monitor, Duration::from_secs(20)),
                    "the card's Approve approved FEAT-001"
                );
            }
        }
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// T15 (DEF-057): a release plan's `again` button is an offer too; an exact label wins over a
    /// prefix either way; the `again` button follows its own readiness; a plan without `again`
    /// reads as before, naming the one button it has.
    #[test]
    fn a_release_plans_again_button_is_an_offer_too() {
        let with_again = |again_ready: bool| {
            json!({
                "version": "v0.1.0", "button": "Promote to candidate", "ready": false, "repo": true,
                "empty": false, "confirm": false,
                "conditions": [{"check": "items_finished", "holds": false, "detail": "FEAT-002 is at Design"}],
                "missing": [],
                "again": {"stage": "candidate", "button": "Promote to candidate again", "does": ["prepare again"],
                          "ready": again_ready, "missing": if again_ready { vec![] } else { vec!["the notes changed since"] }}
            })
        };
        assert_eq!(
            release_button(&with_again(true), "Promote to candidate again", "v0.1.0"),
            Ok(("Promote to candidate again".into(), true))
        );
        assert_eq!(
            release_button(&with_again(true), "Promote to candidate", "v0.1.0"),
            Ok(("Promote to candidate".into(), false))
        );
        assert_eq!(
            release_button(&with_again(true), "promote", "v0.1.0"),
            Ok(("Promote to candidate".into(), false)),
            "a prefix takes the primary first"
        );
        assert_eq!(
            release_offer(&with_again(true), "v0.1.0", "Promote to candidate again"),
            Found::Offered
        );
        let Found::Absent(why) =
            release_offer(&with_again(false), "v0.1.0", "Promote to candidate again")
        else {
            panic!("not ready")
        };
        assert!(
            why.contains("greyed out") && why.contains("the notes changed since"),
            "{why}"
        );
        let Found::Absent(why) = release_offer(&with_again(true), "v0.1.0", "Promote to candidate")
        else {
            panic!("the primary is not ready")
        };
        assert!(
            why.contains("items_finished: FEAT-002 is at Design"),
            "{why}"
        );
        let Found::Absent(why) = release_offer(&with_again(true), "v0.1.0", "Promote to main")
        else {
            panic!("not offered")
        };
        assert!(
            why.contains("offers \"Promote to candidate\" and \"Promote to candidate again\" for v0.1.0, not \"Promote to main\""),
            "{why}"
        );
        // Without `again`: one label named, as before.
        let plain = json!({"button": "Cut v0.1.0", "ready": true, "repo": true, "empty": false, "confirm": false, "conditions": [], "missing": []});
        assert_eq!(
            release_offer(&plain, "v0.1.0", "Cut v0.1.0"),
            Found::Offered
        );
        let Found::Absent(why) = release_offer(&plain, "v0.1.0", "Promote to main") else {
            panic!("not offered")
        };
        assert!(
            why.contains("offers \"Cut v0.1.0\" for v0.1.0, not \"Promote to main\""),
            "{why}"
        );
        assert!(matches!(
            release_offer(&json!({"empty": true}), "v0.1.0", "Cut v0.1.0"),
            Found::Absent(_)
        ));
    }

    /// DEF-119 (R-1, R-2): the plan serves versioned buttons (DEF-034); a runbook names them as a
    /// guide reads them, without the version, or exactly as served. The primary is ready and the
    /// `again` button greyed out, so which one an offer took shows in what it reports.
    #[test]
    fn a_runbook_names_a_release_button_without_the_version() {
        let plan = json!({
            "version": "v0.1.0", "button": "Promote v0.1.0 to candidate", "ready": true, "repo": true,
            "empty": false, "confirm": false, "conditions": [], "missing": [],
            "again": {"stage": "candidate", "button": "Promote v0.1.0 to candidate again",
                      "does": ["prepare again"], "ready": false, "missing": ["the notes changed since"]}
        });
        assert_eq!(
            release_offer(&plan, "v0.1.0", "Promote to candidate"),
            Found::Offered,
            "the guide's wording finds the primary"
        );
        let Found::Absent(why) = release_offer(&plan, "v0.1.0", "Promote to candidate again")
        else {
            panic!("the again button is greyed out")
        };
        assert!(
            why.contains("\"Promote v0.1.0 to candidate again\" is greyed out"),
            "the guide's wording finds the again button: {why}"
        );
        assert_eq!(
            release_offer(&plan, "v0.1.0", "Promote v0.1.0 to candidate"),
            Found::Offered,
            "the served label still finds the primary"
        );
        assert_eq!(
            release_button(&plan, "Promote to candidate", "v0.1.0"),
            Ok(("Promote v0.1.0 to candidate".into(), false))
        );
        assert_eq!(
            release_button(&plan, "Promote to candidate again", "v0.1.0"),
            Ok(("Promote v0.1.0 to candidate again".into(), true))
        );
        assert_eq!(
            release_button(&plan, "Promote v0.1.0 to candidate", "v0.1.0"),
            Ok(("Promote v0.1.0 to candidate".into(), false))
        );
    }

    /// RC6: with several items whose titles name "basket", a whole-word and an exact title win over a
    /// substring, and when several still fit equally the pick is ambiguous — named, not acted on.
    #[test]
    fn an_ambiguous_selector_is_a_finding_not_a_pick() {
        let item = |code: &str, title: &str| ItemBrief {
            code: code.into(),
            title: title.into(),
            status: "Vision".into(),
            created_at: format!("2026-01-0{}T00:00:00Z", &code[code.len() - 1..]),
        };
        let items = vec![
            item("FEAT-001", "A shopper finds their basket kept"),
            item(
                "FEAT-003",
                "Chore: a README that says where the basket page is",
            ),
            item("FEAT-004", "The basket's form address moves to /cart"),
            item("FEAT-005", "Baskets"),
        ];
        let sel = ItemSel::TitleContains("basket".into());
        let Pick::Ambiguous(v) = pick(&items, &sel) else {
            panic!("several whole-word matches")
        };
        assert_eq!(
            v.iter().map(|i| i.code.as_str()).collect::<Vec<_>>(),
            ["FEAT-001", "FEAT-003", "FEAT-004"]
        );
        let why = sel.ambiguity(
            &v.iter()
                .map(|i| (i.code.clone(), i.title.clone()))
                .collect::<Vec<_>>(),
        );
        assert!(
            why.starts_with("ambiguous: the item titled like \"basket\" matches FEAT-001"),
            "{why}"
        );
        assert!(why.contains("nothing was acted on"));
        assert!(select(&items, &sel).is_none(), "never a silent pick");
        // An exact title wins; a whole word beats a substring.
        assert_eq!(
            select(&items, &ItemSel::TitleContains("baskets".into()))
                .unwrap()
                .code,
            "FEAT-005"
        );
        assert_eq!(
            select(&items, &ItemSel::TitleContains("readme".into()))
                .unwrap()
                .code,
            "FEAT-003"
        );
        // A pattern that fits one is a pick; one that fits two is ambiguous.
        assert_eq!(
            select(&items, &ItemSel::TitleMatches("basket kept".into()))
                .unwrap()
                .code,
            "FEAT-001"
        );
        assert!(matches!(
            pick(&items, &ItemSel::TitleMatches("readme|cart".into())),
            Pick::Ambiguous(_)
        ));
    }

    /// RC1 (harness half): a sign-off on an item whose approval lapsed approves it again first, as
    /// the guide says, and says so; an item still approved is left alone.
    #[test]
    fn a_lapsed_approval_is_given_again_before_a_signoff() {
        let (run, base, monitor) = scratch_board("reapprove");
        let by = monitor.approver().unwrap();
        let sel = ItemSel::Code("FEAT-001".into());
        assert!(
            monitor
                .decide("shop", &Decision::Approve(sel.clone()), &by, "3.2", None)
                .unwrap()
                .ok()
        );
        assert_eq!(
            monitor
                .reapprove_if_lapsed("shop", &sel, &by, "3.5")
                .unwrap(),
            None
        );
        // The definition changes after the approval: it lapses, and the Review page offers Approve.
        let store = kanbanr_core::Store::new(run.board());
        let mut def = store
            .load("shop")
            .unwrap()
            .feature("FEAT-001")
            .unwrap()
            .definition
            .clone()
            .unwrap();
        def.statement = "A shopper's basket survives between visits, for a week".into();
        store
            .set_feature_definition("shop", "FEAT-001", Some(def))
            .unwrap();
        // Commit the direct write, as the CLI does: the board HEAD is part of the monitor's memo
        // stamp, so the Review page is computed afresh rather than served from memory (DEF-081).
        assert!(kanbanr_core::git::commit(
            &run.board(),
            "Tester",
            "t@example.com",
            "test: change FEAT-001's definition"
        ));
        let approve = Offer {
            page: "review".into(),
            button: "Approve".into(),
            subject: Some(sel.clone()),
        };
        assert_eq!(
            monitor.offers("shop", &approve, None).unwrap(),
            Found::Offered
        );
        assert_eq!(
            monitor
                .reapprove_if_lapsed("shop", &sel, &by, "3.5")
                .unwrap()
                .as_deref(),
            Some("FEAT-001")
        );
        assert!(matches!(
            monitor.offers("shop", &approve, None).unwrap(),
            Found::Absent(_)
        ));
        monitor.stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn selectors_pick_newest_code_and_title() {
        let items = vec![
            ItemBrief {
                code: "FEAT-001".into(),
                title: "Saved basket".into(),
                status: "Vision".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
            },
            ItemBrief {
                code: "FEAT-002".into(),
                title: "README chore".into(),
                status: "Vision".into(),
                created_at: "2026-01-02T00:00:00Z".into(),
            },
        ];
        assert_eq!(select(&items, &ItemSel::Newest).unwrap().code, "FEAT-002");
        assert_eq!(
            select(&items, &ItemSel::Code("feat-001".into()))
                .unwrap()
                .code,
            "FEAT-001"
        );
        assert_eq!(
            select(&items, &ItemSel::TitleContains("BASKET".into()))
                .unwrap()
                .code,
            "FEAT-001"
        );
        assert!(select(&items, &ItemSel::TitleContains("cart".into())).is_none());
        assert_eq!(
            page_token("<meta name=\"kanbanr-token\" content=\"abc\">"),
            Some("abc".into())
        );
        assert_eq!(page_token("<html></html>"), None);
        assert_eq!(urlencode("manual: a b"), "manual%3A%20a%20b");
    }
}
