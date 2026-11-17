//! Shared data-route dispatcher: maps an HTTP-shaped `(method, path, body)` onto `Store` calls and
//! returns the response BODY exactly as the server would (JSON for structured results, raw text for
//! export and doc reads). This is the single source of truth for the `/projects…` data routes,
//! used by the CLI's local (serverless) mode — and available for the server to delegate to.
//!
//! It deliberately does NOT cover auth/users/remotes — those are server/identity concerns handled
//! by their respective layers.

use crate::config::ProjectConfig;
use crate::error::{CoreError, Result};
use crate::graph::DependencyView;
use crate::models::{TaskState, TestState};
use crate::{Store, export};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

// ---- small helpers -------------------------------------------------------------------------

fn ser<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| CoreError::Unsupported(format!("serialize: {e}")))
}

/// A write's response with the diagram lint's findings added as `diagram_warnings` (FEAT-200),
/// only when there are any; the body is returned as it was otherwise.
fn with_diagram_warnings(
    body: String,
    warnings: &[crate::diagrams::DiagramFinding],
) -> Result<String> {
    if warnings.is_empty() {
        return Ok(body);
    }
    let mut v: Value = serde_json::from_str(&body)
        .map_err(|e| CoreError::Unsupported(format!("serialize: {e}")))?;
    v["diagram_warnings"] = json!(warnings);
    ser(&v)
}

/// The earlier verdict a repeat approval or ratification of `code` collides with, if any (DEF-028).
fn repeat_approval(
    store: &Store,
    p: &str,
    code: &str,
    verdict: &str,
) -> Result<Option<crate::origin::Earlier>> {
    let project = store.load_meta(p)?;
    Ok(project
        .feature(code)?
        .definition
        .as_ref()
        .and_then(|d| crate::origin::duplicate_approval(d, verdict)))
}

/// The earlier ratification a repeat of `code`'s collides with, if any (DEF-028), unless an
/// override Claude made waits for one (DEF-064).
fn repeat_ratification(
    store: &Store,
    p: &str,
    code: &str,
) -> Result<Option<crate::origin::Earlier>> {
    let project = store.load_meta(p)?;
    Ok(crate::origin::duplicate_ratification(
        project.feature(code)?,
    ))
}

/// The answer to a verdict that adds nothing to the record: the item as it stands, with
/// `already_recorded` saying who recorded it first, when and from where. Nothing was written.
fn ser_already_item(
    item: &crate::models::FeatureItem,
    earlier: &crate::origin::Earlier,
) -> Result<String> {
    let mut out = serde_json::to_value(item).map_err(|e| CoreError::Unsupported(e.to_string()))?;
    out["already_recorded"] = earlier.to_json();
    ser(&out)
}

fn ser_already(
    store: &Store,
    p: &str,
    code: &str,
    earlier: &crate::origin::Earlier,
) -> Result<String> {
    let item = store.load_meta(p)?.feature(code)?.clone();
    ser_already_item(&item, earlier)
}

/// Who is recording this verdict — required, never defaulted (FEAT-077).
///
/// This used to fall back to the string "unknown", and the monitor sent "reviewed in the monitor":
/// a place, not a person. Twenty-eight approvals on this project's own board carry it. An approval
/// is the one human step the whole gate exists to obtain, so a record that cannot say who gave it
/// is worse than no record — it reads as accountability while carrying none. Refusing here means no
/// caller can produce one by omission; the CLI resolves `--by` from the commit identity and the
/// monitor from `/api/meta`, and a caller with neither is told to set one rather than quietly
/// attributed to nobody.
/// Who took a decisions action, when and from where (FTR-028): the origin the front end stamped;
/// from the conversation it is Claude, else the `by` the front end sent.
fn decision_stamp(b: &Value) -> Result<crate::decisions::Stamp> {
    let origin = crate::origin::from_body(b);
    let by = if origin == "conversation" {
        "Claude".to_string()
    } else {
        approver(b)?
    };
    Ok(crate::decisions::Stamp {
        by,
        at: crate::now_rfc3339(),
        origin,
        ..Default::default()
    })
}

fn approver(b: &Value) -> Result<String> {
    match str_field(b, "by") {
        Some(who) if !who.trim().is_empty() => Ok(who),
        _ => Err(CoreError::Unsupported(
            "an approval must name who gave it: pass `by` (the CLI defaults it to the data \
             folder's commit identity — set one with `kanbanr identity --name … --email …`)"
                .into(),
        )),
    }
}

/// A verdict covers what the person was shown (FEAT-159). When the request names the revision of
/// the definition it was given on, it is refused if the definition has moved on since — so a brief
/// shown in one moment and a yes recorded in the next can never cover different text.
fn shown_rev(store: &Store, p: &str, code: &str, b: &Value) -> Result<()> {
    let Some(shown) = str_field(b, "rev").filter(|r| !r.trim().is_empty()) else {
        return Ok(());
    };
    let project = store.load_meta(p)?;
    let now = project
        .feature(code)?
        .definition
        .as_ref()
        .map(|d| d.content_rev())
        .unwrap_or_default();
    if now != shown.trim() {
        return Err(CoreError::Unsupported(format!(
            "{code}'s definition changed after it was shown (shown {shown}, now {now}): show the \
             brief again and ask again — nothing was recorded"
        )));
    }
    Ok(())
}

/// A sign-off that pins the spec covers the text the person was shown (FTR-023). When the request
/// names the hash of the spec it was given on, it is refused if the spec has moved on since.
fn shown_spec_rev(store: &Store, p: &str, code: &str, name: &str, b: &Value) -> Result<()> {
    let Some(shown) = str_field(b, "spec_rev").filter(|r| !r.trim().is_empty()) else {
        return Ok(());
    };
    let project = store.load(p)?;
    if !project.config.signoff_pins_spec(name) {
        return Ok(());
    }
    let now = crate::readiness::spec_pin(&project.feature(code)?.specification).1;
    if now != shown.trim() {
        return Err(CoreError::Unsupported(format!(
            "{code}'s spec changed after it was shown (shown {shown}, now {now}): show the brief \
             again and ask again; nothing was recorded"
        )));
    }
    Ok(())
}

/// Where a sign-off was given (FTR-004): with the code repository, the position text the item
/// resolves to (`picks::resolve` of `Pick::Item`) and the blob id there of each repository doc its
/// spec lists; for every sign-off name. Without the repository, or when the position cannot be
/// resolved or read, both are empty: the record steps aside, it never blocks the sign-off
/// (ADR-0004).
fn signoff_location(
    store: &Store,
    p: &str,
    code: &str,
    repo: Option<&std::path::Path>,
) -> (String, std::collections::BTreeMap<String, String>) {
    let none = (String::new(), std::collections::BTreeMap::new());
    let Some(repo) = repo else {
        return none;
    };
    let pick = crate::picks::Pick::Item(code.to_string());
    let Ok(at) = crate::picks::resolve(store, p, repo, &pick) else {
        return none;
    };
    let Ok(project) = store.load_meta(p) else {
        return none;
    };
    let Ok(mut feature) = project.feature(code).cloned() else {
        return none;
    };
    let Ok(spec) = store.feature_spec(p, code) else {
        return none;
    };
    feature.specification = spec;
    let docs_read =
        crate::docsbuilt::docs_read_at(&project, &feature, repo, &at).unwrap_or_default();
    (at.state.position.text(), docs_read)
}

/// What was settled when `docs-folded` is given (FTR-013, DEF-039). With a code repository the
/// flags are recomputed. Each flag is covered by Claude's newest open settlement that still
/// `settles` it, else by an entry in the request's `checked` (a person's terminal may send one);
/// the sign-off is refused while any is neither, and entries for texts no longer flagged are
/// dropped. Without a repository, `checked` is kept as sent. Returns what to record and the ids
/// of the settlements used, which the sign-off then answers. Any other sign-off records nothing.
fn signoff_checked(
    store: &Store,
    p: &str,
    code: &str,
    name: &str,
    body: &Value,
    repo: Option<&std::path::Path>,
    (signer, origin): (&str, &str),
) -> Result<(Vec<crate::models::Checked>, Vec<String>)> {
    use crate::models::{Checked, Outcome};
    if name != crate::method::DOCS_FOLDED {
        return Ok((Vec::new(), Vec::new()));
    }
    let sent: Vec<Checked> = match body.get("checked") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| CoreError::Unsupported(format!("checked: {e}")))?,
    };
    let Some(repo) = repo else {
        return Ok((sent, Vec::new()));
    };
    let flags = crate::docdrift::flags_for_item(store, p, code, repo)?;
    let settlements = if flags.is_empty() {
        Vec::new()
    } else {
        crate::prepared::settlements(store, p, code)?
    };
    let now = crate::now_rfc3339();
    let mut kept = Vec::new();
    let mut used = Vec::new();
    let mut missing = Vec::new();
    for flag in &flags {
        let key = flag.target.key();
        if let Some(r) = settlements
            .iter()
            .find(|r| crate::prepared::settles(r, flag))
        {
            kept.push(crate::prepared::checked_of(r, flag));
            used.push(r.id.clone());
            continue;
        }
        match sent.iter().find(|c| c.target == key) {
            Some(c) if c.outcome == Outcome::Corrected && c.correction.trim().is_empty() => {
                missing.push(format!("{key} (a correction needs its code)"));
            }
            Some(c) => kept.push(Checked {
                by: signer.to_string(),
                origin: origin.to_string(),
                at: now.clone(),
                text_changed_at: flag.target_changed_at.clone(),
                edits: flag.edits.iter().map(|e| e.sha.clone()).collect(),
                ..c.clone()
            }),
            None => missing.push(key),
        }
    }
    if !missing.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "docs-folded waits on {} text(s) Claude has not settled: {}. Claude settles each with \
             `kanbanr prepare text {code} <TARGET>`",
            missing.len(),
            missing.join(", ")
        )));
    }
    Ok((kept, used))
}

/// Is this a manual test of the item's definition? A test that is not there is not, and the
/// store's own "has no test" message follows.
pub(crate) fn is_manual_test(
    store: &Store,
    p: &str,
    code: &str,
    requirement: &str,
    test: &str,
) -> bool {
    store
        .load_meta(p)
        .ok()
        .and_then(|pr| {
            pr.feature(code).ok().and_then(|f| {
                f.definition.as_ref().map(|d| {
                    d.requirements.iter().any(|r| {
                        r.id == requirement
                            && r.tests
                                .iter()
                                .any(|t| t.name == test && t.kind.eq_ignore_ascii_case("manual"))
                    })
                })
            })
        })
        .unwrap_or(false)
}

/// A build-settings change (FTR-062) applied to `current`: each key the body names is set, or
/// cleared by `null` or `""`; an unknown key, a wrong type or a value out of its set is refused and
/// nothing changes.
fn apply_build_change(
    current: crate::config::BuildConfig,
    body: &Value,
) -> Result<crate::config::BuildConfig> {
    let Some(map) = body.as_object() else {
        return Err(CoreError::Unsupported(
            "send the build settings as an object, e.g. {\"slots\": 2}".into(),
        ));
    };
    let mut c = current;
    let bad =
        |k: &str, want: &str| CoreError::Unsupported(format!("build setting `{k}` takes {want}"));
    for (k, v) in map {
        let clear = v.is_null() || v.as_str() == Some("");
        let count = |want: &str| -> Result<Option<u32>> {
            if clear {
                return Ok(None);
            }
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n >= 1)
                .map(Some)
                .ok_or_else(|| bad(k, want))
        };
        let choice = |set: &[&str]| -> Result<Option<String>> {
            if clear {
                return Ok(None);
            }
            v.as_str()
                .filter(|s| set.contains(s))
                .map(|s| Some(s.to_string()))
                .ok_or_else(|| bad(k, &set.join(" or ")))
        };
        match k.as_str() {
            "slots" => c.slots = count("a whole number of at least 1")?,
            "jobs_per_slot" => c.jobs_per_slot = count("a whole number of at least 1")?,
            "cache_gib" => c.cache_gib = count("a whole number of at least 1")?,
            "queue_batch" => c.queue_batch = count("a whole number of at least 1")?,
            "reserve_cpus" | "reserve_mem_gib" => {
                let n = if clear {
                    None
                } else {
                    Some(
                        v.as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or_else(|| bad(k, "a whole number"))?,
                    )
                };
                if k == "reserve_cpus" {
                    c.reserve_cpus = n;
                } else {
                    c.reserve_mem_gib = n;
                }
            }
            "max_load" => {
                c.max_load = if clear {
                    None
                } else {
                    Some(
                        v.as_f64()
                            .filter(|f| f.is_finite() && *f > 0.0)
                            .ok_or_else(|| bad(k, "a number above 0"))?,
                    )
                }
            }
            "sccache_port" => {
                c.sccache_port = if clear {
                    None
                } else {
                    Some(
                        v.as_u64()
                            .and_then(|n| u16::try_from(n).ok())
                            .filter(|n| *n >= 1)
                            .ok_or_else(|| bad(k, "a port number"))?,
                    )
                }
            }
            "sccache" => c.sccache = choice(&["auto", "off"])?,
            "linker" => c.linker = choice(&["default", "mold"])?,
            "web_cache" => {
                c.web_cache = if clear {
                    None
                } else {
                    Some(v.as_bool().ok_or_else(|| bad(k, "true or false"))?)
                }
            }
            "line_check" => {
                c.line_check = if clear {
                    None
                } else {
                    let argv: Vec<String> = v
                        .as_array()
                        .and_then(|a| a.iter().map(|x| x.as_str().map(String::from)).collect())
                        .ok_or_else(|| bad(k, "a command as a list of words"))?;
                    if argv.is_empty() { None } else { Some(argv) }
                }
            }
            other => {
                return Err(CoreError::Unsupported(format!(
                    "unknown build setting `{other}`"
                )));
            }
        }
    }
    Ok(c)
}

fn str_field(body: &Value, k: &str) -> Option<String> {
    body.get(k).and_then(|v| v.as_str()).map(String::from)
}
/// The planning decision a plan route carries out (FTR-049): the body's `decision`, trimmed, when
/// it names one.
fn decision_of(body: &Value) -> Option<String> {
    str_field(body, "decision")
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
}
fn bool_field(body: &Value, k: &str) -> Option<bool> {
    body.get(k).and_then(|v| v.as_bool())
}
fn vec_field(body: &Value, k: &str) -> Option<Vec<String>> {
    body.get(k).and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect()
    })
}

/// A task write's lists (FTR-060): each present array is set, an empty one clears.
fn task_fields(body: &Value) -> crate::store::TaskFields {
    crate::store::TaskFields {
        depends_on: vec_field(body, "depends_on"),
        writes: vec_field(body, "writes"),
        reads: vec_field(body, "reads"),
        ..Default::default()
    }
}

/// A task write's `estimate_h` (FTR-060): absent `None`, `null` `Some(None)`, a number set.
fn task_estimate(body: &Value) -> Result<Option<Option<f64>>> {
    match body.get("estimate_h") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => v.as_f64().map(|h| Some(Some(h))).ok_or_else(|| {
            CoreError::Unsupported("estimate_h must be a number of agent-hours or null".into())
        }),
    }
}

/// A project's codes as `config codes` shows them (FEAT-217): the effective default, each kind's
/// prefix, and per prefix configured or in use, the next code and how many items have it.
#[derive(Debug, Serialize)]
pub struct CodesView {
    pub default: String,
    pub kinds: BTreeMap<String, String>,
    pub next: BTreeMap<String, String>,
    pub in_use: BTreeMap<String, usize>,
}

pub fn codes_view(project: &crate::Project) -> CodesView {
    let codes = &project.config.codes;
    let existing: Vec<String> = project.features.iter().map(|f| f.code.clone()).collect();
    let mut in_use: BTreeMap<String, usize> = BTreeMap::new();
    for code in &existing {
        if let Some((prefix, n)) = code.rsplit_once('-')
            && !prefix.is_empty()
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            *in_use.entry(prefix.to_string()).or_default() += 1;
        }
    }
    let mut prefixes = codes.prefixes();
    prefixes.extend(in_use.keys().cloned());
    let next = prefixes
        .into_iter()
        .map(|p| {
            let code = crate::validate::next_code(&p, &existing);
            (p, code)
        })
        .collect();
    CodesView {
        default: codes.default_prefix().to_string(),
        kinds: codes.kinds.clone(),
        next,
        in_use,
    }
}

/// If the body carries any optional feature attribute
/// (kind/priority/due/assignee/team/labels/depends_on), apply them to `code` and return the
/// serialized updated feature; otherwise `None`.
fn maybe_apply_attrs(store: &Store, p: &str, code: &str, b: &Value) -> Result<Option<String>> {
    let attr_keys = [
        "kind",
        "priority",
        "due",
        "assignee",
        "team",
        "labels",
        "depends_on",
    ];
    // Scheduling attributes (FEAT-035) take a dedicated store path so the broad set_feature_attrs
    // signature is untouched.
    let sched_keys = ["start", "estimate_days", "estimate", "points"];
    let has_attrs = attr_keys.iter().any(|k| b.get(k).is_some());
    let has_sched = sched_keys.iter().any(|k| b.get(k).is_some());
    if !has_attrs && !has_sched {
        return Ok(None);
    }
    // A present string key sets it (empty string clears); a present array replaces the list.
    let s = |k: &str| b.get(k).map(|v| v.as_str().unwrap_or("").to_string());
    let v = |k: &str| {
        b.get(k).map(|val| {
            val.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        })
    };
    let mut f = None;
    if has_attrs {
        f = Some(store.set_feature_attrs(
            p,
            code,
            s("kind"),
            s("priority"),
            s("due"),
            s("assignee"),
            s("team"),
            v("labels"),
            v("depends_on"),
        )?);
    }
    if has_sched {
        // `estimate` is an alias for `estimate_days`; either an empty string or <= 0 clears it.
        let estimate = b
            .get("estimate_days")
            .or_else(|| b.get("estimate"))
            .map(|val| val.as_f64().unwrap_or(0.0));
        if estimate.is_some() || b.get("start").is_some() {
            f = Some(store.set_feature_schedule(p, code, s("start"), estimate)?);
        }
        if let Some(points) = b.get("points") {
            f = Some(store.set_feature_points(p, code, points.as_f64().unwrap_or(0.0))?);
        }
    }
    match f {
        Some(f) => Ok(Some(ser(&f)?)),
        None => Ok(None),
    }
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 3 <= bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    query?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

/// Refuses a query string holding a key outside `allowed`, so a misspelt argument cannot silently
/// become a default (L-22).
pub(crate) fn only_params(query: Option<&str>, allowed: &[&str]) -> Result<()> {
    for kv in query.unwrap_or("").split('&').filter(|kv| !kv.is_empty()) {
        let key = kv.split_once('=').map_or(kv, |(k, _)| k);
        if !allowed.contains(&key) {
            return Err(CoreError::Unsupported(format!(
                "unknown query parameter '{key}' (allowed: {})",
                if allowed.is_empty() {
                    "none".to_string()
                } else {
                    allowed.join(", ")
                }
            )));
        }
    }
    Ok(())
}

/// `GET /projects/{p}/plan/flow` (FTR-061): `scope` is `sprint:SP`, `release:V`, `milestone:MS`
/// or `open` (the running sprint, else `open`, when absent); `lanes` a comma list replacing the
/// default lane counts; `slots` and `build_share` override the defaults. Anything malformed is
/// `Unsupported`.
fn plan_flow(store: &Store, p: &str, query: Option<&str>) -> Result<crate::forecast::FlowPlan> {
    use crate::forecast::{PlanScope, default_options, plan_for};
    let project = store.load_meta(p)?;
    let sprints = crate::sprints::load(store, p).unwrap_or_default();
    let scope = match query_param(query, "scope") {
        None => PlanScope::default_for(&sprints, crate::sprints::today()),
        Some(s) => match s.split_once(':') {
            Some(("sprint", c)) if !c.is_empty() => PlanScope::Sprint(c.to_string()),
            Some(("release", v)) if !v.is_empty() => PlanScope::Release(v.to_string()),
            Some(("milestone", m)) if !m.is_empty() => PlanScope::Milestone(m.to_string()),
            None if s == "open" => PlanScope::Open,
            _ => {
                return Err(CoreError::Unsupported(format!(
                    "scope={s}: sprint:<code>, release:<version>, milestone:<code> or open"
                )));
            }
        },
    };
    let sprint = match &scope {
        PlanScope::Sprint(c) => sprints.iter().find(|s| s.code == *c),
        _ => None,
    };
    let mut opts = default_options(&project.config, sprint);
    if let Some(l) = query_param(query, "lanes") {
        let mut lanes = Vec::new();
        for n in l.split(',') {
            match n.trim().parse::<usize>() {
                Ok(n) if n > 0 => lanes.push(n),
                _ => {
                    return Err(CoreError::Unsupported(format!(
                        "lanes={l}: whole numbers of lanes, comma-separated"
                    )));
                }
            }
        }
        lanes.sort_unstable();
        lanes.dedup();
        opts.lanes = lanes;
    }
    if let Some(n) = query_param(query, "slots") {
        opts.slots = match n.parse::<usize>() {
            Ok(n) if n > 0 => n,
            _ => {
                return Err(CoreError::Unsupported(format!(
                    "slots={n}: a whole number of build slots, at least 1"
                )));
            }
        };
    }
    if let Some(f) = query_param(query, "build_share") {
        opts.build_share = match f.parse::<f64>() {
            Ok(f) if f > 0.0 && f <= 1.0 => Some(f),
            _ => {
                return Err(CoreError::Unsupported(format!(
                    "build_share={f}: a share of a task's time, over 0 and at most 1"
                )));
            }
        };
    }
    plan_for(store, &project, &scope, &opts, &crate::now_rfc3339())
}

/// The code repository a position or view read needs (FTR-003).
fn position_repo(repo: Option<&std::path::Path>) -> Result<&std::path::Path> {
    repo.ok_or_else(|| CoreError::PositionUnavailable {
        wanted: "the checkout's position".into(),
        available: vec![],
    })
}

/// The pick `at` names for project `p` (FTR-004), resolved against `repo`: `main` when absent.
fn picked_at(
    store: &Store,
    p: &str,
    repo: &std::path::Path,
    query: Option<&str>,
) -> Result<crate::picks::Picked> {
    let project = store.load_meta(p)?;
    let pick = match query_param(query, "at").filter(|s| !s.trim().is_empty()) {
        None => crate::picks::Pick::Main,
        Some(text) => crate::picks::parse_pick(
            &text,
            &project,
            &crate::sprints::load(store, p).unwrap_or_default(),
            &crate::releases::load(store, p).unwrap_or_default(),
        )?,
    };
    crate::picks::resolve(store, p, repo, &pick)
}

/// The required `path` of a repository doc read (FTR-004).
fn repo_doc_path(query: Option<&str>) -> Result<String> {
    query_param(query, "path")
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| CoreError::Unsupported("path is required: a repository path".into()))
}

/// `GET /projects/{p}/repo-docs/raw?path=&at=` (FTR-004): the bytes of a repository file at the
/// pick, and their media type. The daemon serves these bytes as they are; a path missing at the
/// pick is a 404.
pub fn repo_doc_raw(
    store: &Store,
    p: &str,
    query: Option<&str>,
    repo: Option<&std::path::Path>,
) -> Result<(Vec<u8>, &'static str)> {
    only_params(query, &["path", "at"])?;
    let repo = position_repo(repo)?;
    let path = repo_doc_path(query)?;
    let at = picked_at(store, p, repo, query)?;
    match crate::repodocs::read_bytes(repo, &at, &path)? {
        Some(bytes) => Ok((bytes, crate::docs::content_type(&path))),
        None => Err(CoreError::DocNotFound(path)),
    }
}

/// A boolean query-string flag: present with no/empty/"true"/"1" value -> true.
fn query_flag(query: Option<&str>, key: &str) -> bool {
    match query_param(query, key) {
        Some(v) => v.is_empty() || v == "true" || v == "1",
        None => query
            .map(|q| q.split('&').any(|kv| kv == key))
            .unwrap_or(false),
    }
}

/// Build a feature [`Query`](crate::query::Query) from the URL query string. `project` scopes the
/// search to one project (the per-project route) or `None` for portfolio-wide.
fn build_query(project: Option<&str>, query: Option<&str>) -> crate::query::Query {
    let labels = query_param(query, "label")
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();
    crate::query::Query {
        project: project.map(String::from),
        status: query_param(query, "status"),
        milestone: query_param(query, "milestone"),
        kind: query_param(query, "kind"),
        priority: query_param(query, "priority"),
        labels,
        assignee: query_param(query, "assignee"),
        team: query_param(query, "team"),
        due_after: query_param(query, "due_after"),
        due_before: query_param(query, "due_before"),
        ready: query_flag(query, "ready"),
        blocked: query_flag(query, "blocked"),
        text: query_param(query, "text"),
        full_text: query_flag(query, "full_text"),
        goal: query_param(query, "goal"),
        gap: query_param(query, "gap"),
    }
}

// ---- project summaries (mirrors the server's home-tile shape, sans user filtering) ----------

/// One project on the board, as the home page lists it: `GET /projects`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProjectSummary {
    pub id: String,
    /// The project's name; its id when it has none.
    pub name: String,
    pub description: String,
    /// The statuses shown as board columns.
    pub displayed_states: Vec<String>,
    /// The statuses that are not columns.
    pub other_states: Vec<String>,
    pub no_op_states: Vec<String>,
    /// Items per status, for every status.
    pub counts: BTreeMap<String, usize>,
    pub total_features: usize,
    pub has_docs: bool,
    /// The board the project is served from, when the server runs over a list of boards
    /// (FTR-040); absent from the CLI's `project list`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
}

/// Every project of `store` as the home page lists it, each tagged with `board` (FTR-040).
pub fn list_summaries_for(store: &Store, board: Option<&str>) -> Result<Vec<ProjectSummary>> {
    let mut out = Vec::new();
    for id in store.list_projects()? {
        let Ok(project) = store.load(&id) else {
            continue;
        };
        let cfg = &project.config;
        let displayed = cfg.board_states().to_vec();
        let other_states: Vec<String> = cfg
            .statuses
            .iter()
            .filter(|s| !displayed.contains(s))
            .cloned()
            .collect();
        let mut counts = BTreeMap::new();
        for s in &cfg.statuses {
            counts.insert(
                s.clone(),
                project.features.iter().filter(|f| &f.status == s).count(),
            );
        }
        let has_docs = store
            .doc_tree(&id)
            .map(|t| !t.folders.is_empty() || !t.docs.is_empty())
            .unwrap_or(false);
        let name = if cfg.name.is_empty() {
            id.clone()
        } else {
            cfg.name.clone()
        };
        out.push(ProjectSummary {
            id,
            name,
            description: cfg.description.clone(),
            displayed_states: displayed,
            other_states,
            no_op_states: cfg.no_op_states.clone(),
            counts,
            total_features: project.features.len(),
            has_docs,
            board: board.map(str::to_string),
        });
    }
    Ok(out)
}

// ---- config builders shared with the server's create-project / workflow routes --------------

fn build_project_config(store: &Store, name: &str, body: &Value) -> Result<ProjectConfig> {
    let mut config = build_base_config(store, name, body)?;
    // The code repository the project tracks (DEF-053); an empty URL records none.
    if let Some(url) = str_field(body, "repository").filter(|u| !u.trim().is_empty()) {
        config.repository = Some(crate::config::CodeRepository {
            url: url.trim().to_string(),
            private: bool_field(body, "repository_private").unwrap_or(false),
            ..Default::default()
        });
    }
    Ok(config)
}

fn build_base_config(store: &Store, name: &str, body: &Value) -> Result<ProjectConfig> {
    // A process chosen at creation: `"workflow": "<preset>"` (FEAT-116). An unknown name is refused
    // with the list, where it used to fall back silently to the default.
    if let Some(preset) = str_field(body, "workflow") {
        let (file, library) = store.resolve_process(&preset)?;
        let source = file.source(&preset, library);
        let mut config = file.into_config(name);
        config.process = Some(source);
        if let Some(d) = str_field(body, "description") {
            config.description = d;
        }
        return Ok(config);
    }
    // A process from the user's own library (FEAT-169): the CLI sends the file and where it came
    // from, because the board cannot see the user's home folder.
    if let Some(v) = body.get("workflow_file").filter(|v| !v.is_null()) {
        let file: crate::config::WorkflowFile = serde_json::from_value(v.clone())
            .map_err(|e| CoreError::Unsupported(format!("invalid workflow file: {e}")))?;
        if let Some(problem) = file.problems().into_iter().next() {
            return Err(problem);
        }
        let mut config = file.into_config(name);
        config.process = process_source(body)?;
        if let Some(d) = str_field(body, "description") {
            config.description = d;
        }
        return Ok(config);
    }
    let statuses = vec_field(body, "statuses");
    let displayed = vec_field(body, "displayed_states");
    let default_state = str_field(body, "default_state");
    let no_op = vec_field(body, "no_op_states");
    let mut config = match &statuses {
        Some(s) => {
            let no_ops = no_op.clone().unwrap_or_default();
            ProjectConfig {
                extra: Default::default(),
                branch_pattern: None,
                schema_version: crate::config::BASE_SCHEMA_VERSION,
                name: name.to_string(),
                description: String::new(),
                repository: None,
                displayed_states: displayed
                    .clone()
                    .unwrap_or_else(|| s.iter().filter(|x| !no_ops.contains(x)).cloned().collect()),
                default_state: default_state
                    .clone()
                    .unwrap_or_else(|| s.first().cloned().unwrap_or_default()),
                transitions: Default::default(),
                no_op_states: no_ops,
                terminal_states: vec_field(body, "terminal_states").unwrap_or_default(),
                statuses: s.clone(),
                gates: Default::default(),
                estimate_unit: Default::default(),
                cadence: Default::default(),
                process: None,
                method: Default::default(),
                diagrams: Default::default(),
                codes: Default::default(),
                coverage: Vec::new(),
                paths: Default::default(),
                tasks: Default::default(),
                build: Default::default(),
            }
        }
        None => {
            let mut c = ProjectConfig::for_new_project(name);
            if let Some(ds) = &displayed {
                c.displayed_states = ds.clone();
            }
            if let Some(d) = &default_state {
                c.default_state = d.clone();
            }
            if let Some(n) = &no_op {
                c.no_op_states = n.clone();
                c.displayed_states.retain(|s| !n.contains(s));
            }
            if let Some(t) = vec_field(body, "terminal_states") {
                c.terminal_states = t;
            }
            c
        }
    };
    if let Some(d) = str_field(body, "description") {
        config.description = d;
    }
    Ok(config)
}

fn apply_workflow(store: &Store, p: &str, body: &Value) -> Result<String> {
    // A preset to start from (FEAT-116): `preset: <name>`, or the older `togaf` / `defaults` flags.
    let preset = str_field(body, "preset").or_else(|| {
        if bool_field(body, "togaf").unwrap_or(false) {
            Some("togaf".to_string())
        } else if bool_field(body, "defaults").unwrap_or(false) {
            Some("default".to_string())
        } else {
            None
        }
    });
    // The process this workflow comes from (FEAT-169): a named one, looked up on the board and
    // then among the built-in ones, or one the CLI read from the user's library and says so. A
    // workflow from a file or from flags comes from no saved process, and clears the record.
    let mut source = process_source(body)?;
    let base = match preset {
        Some(name) => {
            let (file, library) = store.resolve_process(&name)?;
            // Shaped by the project's own choice, so a part it turned off stays off (FEAT-193).
            let choice = store.load_meta(p)?.config.method;
            let file = crate::method::shaped(file, &choice);
            source = Some(file.source(&name, library));
            Some(file.into_config(p))
        }
        None => None,
    };
    let statuses = match (vec_field(body, "statuses"), &base) {
        (Some(s), _) => s,
        (None, Some(base)) => base.statuses.clone(),
        (None, None) => {
            return Err(CoreError::Unsupported(
                "provide statuses (or defaults)".into(),
            ));
        }
    };
    let transitions: BTreeMap<String, Vec<String>> = body
        .get("transitions")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .or_else(|| base.as_ref().map(|x| x.transitions.clone()))
        .unwrap_or_default();
    let default_state =
        str_field(body, "default_state").or_else(|| base.as_ref().map(|x| x.default_state.clone()));
    let displayed = vec_field(body, "displayed_states")
        .or_else(|| base.as_ref().map(|x| x.displayed_states.clone()));
    let no_ops =
        vec_field(body, "no_op_states").or_else(|| base.as_ref().map(|x| x.no_op_states.clone()));
    let terminals = vec_field(body, "terminal_states")
        .or_else(|| base.as_ref().map(|x| x.terminal_states.clone()));
    let gates: Option<BTreeMap<String, crate::config::Gate>> = match body.get("gates") {
        Some(v) => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid gates: {e}")))?,
        ),
        None => base.as_ref().map(|x| x.gates.clone()),
    };
    // Kind paths (FTR-019): null or absent means none, except that an applied process brings its own.
    let paths: Option<BTreeMap<String, crate::config::KindPath>> =
        match body.get("paths").filter(|v| !v.is_null()) {
            Some(v) => Some(
                serde_json::from_value(v.clone())
                    .map_err(|e| CoreError::Unsupported(format!("invalid paths: {e}")))?,
            ),
            None => base.as_ref().map(|x| x.paths.clone()),
        };
    let config = store.set_workflow_with_paths(
        p,
        statuses,
        transitions,
        default_state,
        displayed,
        no_ops,
        terminals,
        gates,
        paths,
    )?;
    // A process that works in sprints, releases or points brings that with it (FEAT-122); one that
    // does not leaves the project's cadence as it was.
    let cadence: Option<crate::config::Cadence> = match body.get("cadence").filter(|v| !v.is_null())
    {
        Some(v) => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid cadence: {e}")))?,
        ),
        None => base
            .as_ref()
            .map(|x| x.cadence.clone())
            .filter(|c| !c.is_off()),
    };
    let unit: Option<crate::config::EstimateUnit> =
        match body.get("estimate_unit").filter(|v| !v.is_null()) {
            Some(v) => Some(
                serde_json::from_value(v.clone())
                    .map_err(|e| CoreError::Unsupported(format!("invalid estimate unit: {e}")))?,
            ),
            None => base
                .as_ref()
                .map(|x| x.estimate_unit)
                .filter(|u| !u.is_days()),
        };
    let config = match (cadence, unit) {
        (None, None) => config,
        (cadence, unit) => {
            let mut cadence = cadence.unwrap_or_else(|| config.cadence.clone());
            // FEAT-212: a process that says nothing of the release stages' gates or preparation
            // keeps the project's own; if its stages no longer fit them, the refusal names the fix.
            if cadence.release_gates.is_empty() {
                cadence.release_gates = config.cadence.release_gates.clone();
                // The kept gates name the project's stages (the recommended ones, FTR-020), so a
                // process that names none keeps those too.
                if cadence.release_stages.is_empty() {
                    cadence.release_stages = config.cadence.release_stages.clone();
                }
            }
            if cadence.release_prepare.is_empty() {
                cadence.release_prepare = config.cadence.release_prepare.clone();
            }
            // DEF-042: the file's `true` wins, the first choice turns it on, and the project's own
            // `true` is kept: a process never turns the git model off.
            cadence.git_model = config
                .cadence
                .git_model_after(&cadence, cadence.git_model.then_some(true))
                || config.cadence.git_model;
            store.set_cadence(p, unit, cadence)?
        }
    };
    let config = if config.process == source {
        config
    } else {
        store.set_process_source(p, source)?
    };
    // DEF-042: a path that never enters the integrate stage is a warning, not a refusal.
    let skipping = config.paths_skipping_integrate();
    if skipping.is_empty() {
        return ser(&config);
    }
    let mut out =
        serde_json::to_value(&config).map_err(|e| CoreError::Unsupported(e.to_string()))?;
    out["warnings"] = json!(
        skipping
            .iter()
            .map(|(kind, stage)| crate::config::skipped_integrate_warning(kind, stage))
            .collect::<Vec<_>>()
    );
    ser(&out)
}

/// `"process": {name, library, version, rev}` in a body: where the workflow it carries came from.
fn process_source(body: &Value) -> Result<Option<crate::config::ProcessSource>> {
    match body.get("process").filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| CoreError::Unsupported(format!("invalid process source: {e}"))),
    }
}

// ---- the dispatcher ------------------------------------------------------------------------

/// The media type of what a successful `GET` of `path` (with its query) answers with (FEAT-177):
/// JSON, unless the route renders text — a diagram, a document, a Markdown export. The daemon
/// labels its bodies with this, so a body labelled JSON is JSON.
pub fn read_media_type(path: &str) -> &'static str {
    const TEXT: &str = "text/plain; charset=utf-8";
    const MARKDOWN: &str = "text/markdown; charset=utf-8";
    const JSON: &str = "application/json";
    let (path_only, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    let segs: Vec<&str> = path_only.split('/').filter(|s| !s.is_empty()).collect();
    let format = query_param(query, "format");
    match segs.as_slice() {
        ["projects", _, "workflow"] | ["projects", _, "gantt"] | ["gantt"] => TEXT,
        ["projects", _, "graph"] | ["graph"] if format.as_deref() == Some("dot") => TEXT,
        ["projects", _, "export"] | ["projects", _, "features", _, "export"]
            if format.as_deref() != Some("json") =>
        {
            MARKDOWN
        }
        ["projects", _, "docs", "content"] => MARKDOWN,
        // A repository file's bytes, labelled by its extension (FTR-004).
        ["projects", _, "repo-docs", "raw"] => {
            crate::docs::content_type(&query_param(query, "path").unwrap_or_default())
        }
        ["projects", _, "scopes", _, "brief"] => MARKDOWN,
        _ => JSON,
    }
}

/// Apply a data operation against the store, returning the response body string. Reads and writes
/// alike — the caller (e.g. the CLI's local backend) decides whether to commit afterwards based on
/// the HTTP method.
pub fn dispatch(store: &Store, method: &str, path: &str, body: Option<&Value>) -> Result<String> {
    dispatch_in(store, method, path, body, None)
}

/// [`dispatch`], with the project's code repository when the caller has one (FEAT-183). Only
/// release promotion reaches it: the CLI passes the repository it runs in, the daemon the one it
/// was started in when that folder's marker names the project. Without one, the promotions that
/// need it — the first stage and the cut — are refused, never guessed.
pub fn dispatch_in(
    store: &Store,
    method: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&std::path::Path>,
) -> Result<String> {
    let out = dispatch_routes(store, method, path, body, repo)?;
    // After a write, the coverage rules it now fails (FEAT-197). A dry run changed nothing.
    if !is_mutation(method) || is_dry_run(body) {
        return Ok(out);
    }
    Ok(crate::coverage::annotate(
        store,
        method,
        path,
        body.unwrap_or(&Value::Null),
        out,
    ))
}

/// The routes themselves: [`dispatch_in`] without what it adds after a write.
fn dispatch_routes(
    store: &Store,
    method: &str,
    path: &str,
    body: Option<&Value>,
    repo: Option<&std::path::Path>,
) -> Result<String> {
    let (path_only, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    let segs: Vec<&str> = path_only.split('/').filter(|s| !s.is_empty()).collect();
    let m = method.to_uppercase();
    let null = Value::Null;
    let b = body.unwrap_or(&null);

    match (m.as_str(), segs.as_slice()) {
        // ---- doctor (portfolio integrity) ----
        ("GET", ["doctor"]) => ser(&crate::doctor::run(store)?),
        // The board's README (FEAT-201): written now, keeping its owner's text.
        ("POST", ["readme"]) => ser(&json!({ "changed": crate::boardreadme::write(store)? })),
        ("GET", ["projects", p, "doctor"]) => {
            // Repository doc links are checked at each item's position (FTR-004).
            let mut report = crate::doctor::run_project_in(store, p, repo)?;
            // With the git model on, the branches are checked against the board too (FEAT-191).
            if store.load_meta(p)?.config.cadence.git_model {
                report.issues.extend(git_drift(store, p, repo));
            }
            // From the project folder, the evidence reconciled with the code (MNT-001 R-8). A
            // reconciliation that cannot run steps aside (ADR-0004).
            if let Some(repo) = repo
                && let Ok(r) = crate::evidence::reconcile(store, p, repo)
                && let Some(message) = crate::evidence::evidence_summary(&r)
            {
                report.issues.push(crate::doctor::Issue {
                    severity: crate::doctor::Severity::Warning,
                    project: p.to_string(),
                    code: None,
                    message,
                });
            }
            ser(&report)
        }
        // The checkout's live position (FTR-003).
        ("GET", ["projects", p, "position"]) => {
            only_params(query, &[])?;
            ser(&crate::position::position_answer(position_repo(repo)?, p)?)
        }
        // The docs view at the live position or `at` (FTR-003): the answer's own bytes.
        ("GET", ["projects", p, "views", "docs"]) => {
            only_params(query, &["root", "max-nodes", "at"])?;
            let root = position_repo(repo)?;
            let docs_root = query_param(query, "root").unwrap_or_else(|| "docs".into());
            let max_nodes = match query_param(query, "max-nodes") {
                Some(n) => n.parse::<usize>().map_err(|_| {
                    CoreError::Unsupported(format!("max-nodes must be a number, not '{n}'"))
                })?,
                None => 60,
            };
            let at = query_param(query, "at").filter(|s| !s.is_empty());
            store.load_meta(p)?;
            let facts = crate::views::project_facts(store, p);
            let answer = crate::views::docs_view_live(
                root,
                p,
                &facts,
                at.as_deref(),
                &docs_root,
                max_nodes,
            )?;
            Ok(String::from_utf8_lossy(&crate::views::answer_bytes(&answer)).into_owned())
        }
        // The positions a reader can pick, and one picked (FTR-004).
        ("GET", ["projects", p, "positions"]) => {
            only_params(query, &[])?;
            let repo = position_repo(repo)?;
            store.load_meta(p)?;
            ser(&crate::picks::picks(store, p, repo))
        }
        ("GET", ["projects", p, "positions", "resolve"]) => {
            only_params(query, &["at"])?;
            ser(&picked_at(store, p, position_repo(repo)?, query)?)
        }
        // Repository docs at a pick (FTR-004): the tree under docs/, a doc, its bytes.
        ("GET", ["projects", p, "repo-docs"]) => {
            only_params(query, &["at"])?;
            let repo = position_repo(repo)?;
            let at = picked_at(store, p, repo, query)?;
            ser(&crate::repodocs::tree(repo, &at, "docs/")?)
        }
        ("GET", ["projects", p, "repo-docs", "content"]) => {
            only_params(query, &["path", "at"])?;
            let repo = position_repo(repo)?;
            let path = repo_doc_path(query)?;
            let at = picked_at(store, p, repo, query)?;
            let main = if at.kind == "main" {
                at.state.clone()
            } else {
                crate::picks::resolve(store, p, repo, &crate::picks::Pick::Main)?.state
            };
            ser(&crate::repodocs::read(repo, &at, &path, &main)?)
        }
        ("GET", ["projects", p, "repo-docs", "raw"]) => {
            let (bytes, _) = repo_doc_raw(store, p, query, repo)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        // The docs and items that name what work changed and have not changed since (FTR-013).
        ("GET", ["projects", p, "drift"]) => {
            let since = query_param(query, "since").filter(|s| !s.is_empty());
            let item = query_param(query, "item").filter(|s| !s.is_empty());
            ser(&crate::docdrift::report(
                store,
                p,
                since.as_deref(),
                item.as_deref(),
                repo,
            )?)
        }
        ("GET", ["projects", p, "features", code, "drift"]) => {
            let since = query_param(query, "since").filter(|s| !s.is_empty());
            ser(&crate::docdrift::report(
                store,
                p,
                since.as_deref(),
                Some(code),
                repo,
            )?)
        }
        // The board's evidence reconciled with the code (MNT-001): derived, never stored.
        ("GET", ["projects", p, "tests", "reconcile"]) => {
            let repo = repo
                .ok_or_else(|| CoreError::Unsupported("run it from the project folder".into()))?;
            ser(&crate::evidence::reconcile(store, p, repo)?)
        }
        // Drift between the board and the branches (FEAT-191), on demand.
        ("GET", ["projects", p, "git", "doctor"]) => {
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "git doctor reads the code repository: run it from the project folder".into(),
                )
            })?;
            ser(&crate::gitdrift::scan(store, p, repo)?)
        }

        // ---- the board's process library (FEAT-169) ----
        ("GET", ["processes"]) => ser(&crate::library::listing(store)?),
        ("GET", ["processes", name]) => {
            let (file, library) = store.resolve_process(name)?;
            ser(&json!({ "name": name, "library": library, "file": file }))
        }
        ("PUT", ["processes", name]) => {
            let file: crate::config::WorkflowFile =
                serde_json::from_value(b.get("file").cloned().unwrap_or(Value::Null))
                    .map_err(|e| CoreError::Unsupported(format!("invalid process file: {e}")))?;
            ser(&store.save_process(name, file, str_field(b, "description"))?)
        }

        // ---- practice packs (FTR-033): the board's and the built-in libraries ----
        ("GET", ["profiles"]) => ser(&crate::profile::listing(store)?),
        ("GET", ["profiles", name]) => {
            let name = percent_decode(name);
            let (file, library) = store.resolve_profile(&name, None)?;
            ser(&json!({ "name": name, "library": library, "file": file }))
        }
        // `{file, description?}`: save a pack to the board's library, versioned.
        ("PUT", ["profiles", name]) => {
            let file: crate::profile::ProfileFile =
                serde_json::from_value(b.get("file").cloned().unwrap_or(Value::Null))
                    .map_err(|e| CoreError::Unsupported(format!("invalid pack file: {e}")))?;
            ser(&store.save_profile(&percent_decode(name), file, str_field(b, "description"))?)
        }

        // How a project stands against its saved process (FEAT-170): read-only, for the CLI and
        // the monitor. A personal process is the user's kanbanr's to compare; here it is named only.
        ("GET", ["projects", p, "process"]) => {
            let project = store.load(p)?;
            let drift = store.process_drift(&project, None)?;
            ser(&crate::library::ProcessStatus {
                source: project.config.process,
                messages: drift.as_ref().map(|d| d.messages()).unwrap_or_default(),
                drift,
            })
        }

        // The packs a project follows against their libraries (FTR-033). A personal pack is the
        // user's kanbanr's to compare; here it is named only.
        ("GET", ["projects", p, "profile"]) => {
            ser(&crate::baseline::profile_status(store, p, None)?)
        }

        // ---- portfolio / program hierarchy (FEAT-030) ----
        ("GET", ["portfolio"]) => ser(&crate::portfolio::view(store)?),
        ("GET", ["portfolio", "rollups"]) => ser(&crate::portfolio::rollups(store)?),
        ("GET", ["portfolio", "board"]) => ser(&crate::portfolio::cross_project_board(store)?),
        ("POST", ["portfolio", "programs"]) => {
            let id = str_field(b, "id").unwrap_or_default();
            let projects = vec_field(b, "projects").unwrap_or_default();
            let ws = crate::portfolio::add_program(
                store,
                &id,
                str_field(b, "name"),
                str_field(b, "description"),
                projects,
            )?;
            ser(&ws)
        }

        // ---- projects ----
        ("GET", ["projects"]) => ser(&list_summaries_for(store, None)?),
        ("POST", ["projects"]) => {
            let name = str_field(b, "name").unwrap_or_default();
            let config = build_project_config(store, &name, b)?;
            ser(&store.init_project(&name, config)?)
        }
        ("GET", ["projects", p]) => ser(&store.load(p)?),
        // The monitor's reads (DEF-081): the compact index, the configuration alone, and items
        // without the whole board.
        ("GET", ["projects", p, "index"]) => ser(&crate::monitorindex::compact(store, p)?),
        ("GET", ["projects", p, "config"]) => ser(&store.load_meta(p)?.config),

        // ---- derived dependency state (FEAT-027) ----
        ("GET", ["projects", p, "ready"]) => {
            store.load(p)?; // 404 on unknown project
            let view = DependencyView::build(store, None)?;
            ser(&view.ready(Some(p)))
        }
        // What can start now, task by task (FTR-060): `?lanes=N` proposes up to N to start
        // together, critical path first.
        ("GET", ["projects", p, "ready", "tasks"]) => {
            let project = store.load_meta(p)?;
            let lanes = match query_param(query, "lanes") {
                Some(n) => Some(n.parse::<usize>().map_err(|_| {
                    CoreError::Unsupported(format!("lanes={n}: a whole number of lanes"))
                })?),
                None => None,
            };
            let graph = crate::taskgraph::TaskGraph::build(store, p, Some(&project))?;
            ser(&graph.readiness(p, &project.config.tasks.append_only, lanes))
        }
        // The flow plan (FTR-061): a scope's work in agent-hours over its task graph, what runs,
        // is ready and is blocked, and a forecast per lane count bounded by build slots.
        ("GET", ["projects", p, "plan", "flow"]) => {
            only_params(query, &["scope", "lanes", "slots", "build_share"])?;
            ser(&plan_flow(store, p, query)?)
        }
        ("GET", ["projects", p, "blocked"]) => {
            store.load(p)?;
            let view = DependencyView::build(store, None)?;
            ser(&view.blocked(Some(p)))
        }
        ("GET", ["projects", p, "graph"]) => {
            store.load(p)?;
            let view = DependencyView::build(store, None)?;
            match query_param(query, "format").as_deref() {
                Some("dot") => Ok(view.to_dot(Some(p))),
                _ => Ok(view.to_json_value(Some(p)).to_string()),
            }
        }
        // ---- scheduling: critical path & Gantt (FEAT-035) ----
        ("GET", ["projects", p, "critical-path"]) => {
            store.load(p)?; // 404 on unknown project
            let view = DependencyView::build(store, None)?;
            Ok(view.schedule(Some(p)).to_json_value().to_string())
        }
        ("GET", ["projects", p, "gantt"]) => Ok(crate::gantt::project_gantt(store, p)?),
        ("GET", ["critical-path"]) => {
            let view = DependencyView::build(store, None)?;
            Ok(view.schedule(None).to_json_value().to_string())
        }
        ("GET", ["gantt"]) => Ok(crate::gantt::portfolio_gantt(store)?),
        // ---- the Plan view (FTR-043): counts, one tab's rows, one row's items ----
        ("GET", ["projects", p, "plan", "summary"]) => {
            only_params(query, &[])?;
            ser(&crate::planview::summary(
                store,
                p,
                crate::sprints::today(),
            )?)
        }
        ("GET", ["projects", p, "plan"]) => {
            only_params(query, &["scope"])?;
            let scope = match query_param(query, "scope").as_deref() {
                Some("current") => crate::planview::Scope::Current,
                Some("planned") => crate::planview::Scope::Planned,
                Some("past") => crate::planview::Scope::Past,
                other => {
                    return Err(CoreError::Unsupported(format!(
                        "scope must be current, planned or past, not {}",
                        other.unwrap_or("nothing")
                    )));
                }
            };
            ser(&crate::planview::slice(
                store,
                p,
                scope,
                crate::sprints::today(),
            )?)
        }
        ("GET", ["projects", p, "plan", "rows", key]) => ser(&crate::planview::row(
            store,
            p,
            &percent_decode(key),
            crate::sprints::today(),
        )?),
        ("GET", ["portfolio", "plan"]) => {
            ser(&crate::planview::portfolio(store, crate::sprints::today())?)
        }
        ("GET", ["projects", p, "features", code, "impact"]) => {
            let project = store.load(p)?;
            project.feature(code)?; // 404 on unknown feature
            let view = DependencyView::build(store, None)?;
            ser(&view.impact(&crate::graph::qualify(p, code)))
        }
        // Portfolio-wide (cross-project) variants.
        ("GET", ["ready"]) => {
            let view = DependencyView::build(store, None)?;
            ser(&view.ready(None))
        }
        ("GET", ["blocked"]) => {
            let view = DependencyView::build(store, None)?;
            ser(&view.blocked(None))
        }
        ("GET", ["graph"]) => {
            let view = DependencyView::build(store, None)?;
            match query_param(query, "format").as_deref() {
                Some("dot") => Ok(view.to_dot(None)),
                _ => Ok(view.to_json_value(None).to_string()),
            }
        }

        // ---- query: rich filters + full-text, scoped or portfolio-wide (FEAT-032) ----
        ("GET", ["query"]) => ser(&crate::query::run(store, &build_query(None, query))?),
        ("GET", ["projects", p, "query"]) => {
            store.load_meta(p)?; // 404 on unknown project
            ser(&crate::query::run(store, &build_query(Some(p), query))?)
        }
        ("GET", ["projects", p, "export"]) => {
            let project = store.load(p)?;
            match query_param(query, "format").as_deref() {
                Some("json") => ser(&project),
                _ => Ok(export::project_to_markdown(&project)),
            }
        }
        ("PATCH", ["projects", p]) => ser(&store.set_project_meta(
            p,
            str_field(b, "name"),
            str_field(b, "description"),
            str_field(b, "repository"),
            bool_field(b, "repository_private"),
        )?),
        ("DELETE", ["projects", p]) => {
            store.delete_project(p)?;
            Ok(String::new())
        }

        // ---- features ----
        ("GET", ["projects", p, "features", code, "export"]) => {
            let project = store.load(p)?;
            let feature = project.feature(code)?;
            let ms = project.milestone(&feature.milestone).ok();
            let line = if project.config.cadence.releases {
                crate::releases::release_line(&project, &crate::releases::load(store, p)?, feature)
            } else {
                None
            };
            match query_param(query, "format").as_deref() {
                Some("json") => export::to_json(feature, ms, line.as_deref())
                    .map_err(|e| CoreError::Unsupported(format!("export: {e}"))),
                _ => {
                    // The catalog rows the design changes (FTR-006), when the project has a catalog.
                    let views = crate::catalog::load(store, p).ok().flatten().and_then(|c| {
                        let def = feature.definition.as_ref()?;
                        let (from_docs, named) =
                            crate::catalog::views_changed_split(&c, p, &feature.specification, def);
                        Some(export::ViewsChanged { from_docs, named })
                    });
                    Ok(export::to_markdown_with(
                        feature,
                        ms,
                        line.as_deref(),
                        views.as_ref(),
                    ))
                }
            }
        }
        ("POST", ["projects", p, "features"]) => {
            let spec = str_field(b, "specification").unwrap_or_default();
            let kind = str_field(b, "kind");
            let f = store.add_feature_of_kind(
                p,
                &str_field(b, "title").unwrap_or_default(),
                &spec,
                &str_field(b, "milestone").unwrap_or_default(),
                str_field(b, "code"),
                kind.as_deref(),
            )?;
            let warnings = crate::diagrams::lint_markdown(&format!("{} spec", f.code), &spec);
            let attrs = maybe_apply_attrs(store, p, &f.code, b)?;
            // Feedback on a release says where it was found, the way a defect does (FEAT-120):
            // recorded with the item, in the same write.
            let body = if let Some(version) = str_field(b, "found_in") {
                let defect = crate::models::Defect {
                    found_in: version,
                    ..Default::default()
                };
                ser(&store.set_defect(p, &f.code, Some(defect))?)?
            } else {
                match attrs {
                    Some(json) => json,
                    None => ser(&f)?,
                }
            };
            with_diagram_warnings(body, &warnings)
        }
        // Items without their specs, by status and milestone when given (DEF-081).
        ("GET", ["projects", p, "features"]) => {
            only_params(query, &["status", "milestone"])?;
            let status = query_param(query, "status").filter(|s| !s.is_empty());
            let milestone = query_param(query, "milestone").filter(|s| !s.is_empty());
            let items: Vec<crate::models::FeatureItem> = store
                .load_meta(p)?
                .features
                .into_iter()
                .filter(|f| status.as_ref().is_none_or(|s| &f.status == s))
                .filter(|f| milestone.as_ref().is_none_or(|m| &f.milestone == m))
                .collect();
            ser(&items)
        }
        // One item with its spec, reading only its own files (DEF-081).
        ("GET", ["projects", p, "features", code]) => ser(&store.load_feature(p, code)?),
        ("PATCH", ["projects", p, "features", code]) => {
            let milestone = if bool_field(b, "clear_milestone").unwrap_or(false) {
                Some(None)
            } else {
                str_field(b, "milestone").map(Some)
            };
            let spec = str_field(b, "specification");
            let edited = store.edit_feature(
                p,
                code,
                str_field(b, "title"),
                spec.clone(),
                milestone,
                str_field(b, "new_code"),
            )?;
            let warnings = spec
                .map(|s| crate::diagrams::lint_markdown(&format!("{} spec", edited.code), &s))
                .unwrap_or_default();
            // new_code may have renamed it; attrs apply to the (possibly new) code.
            let body = match maybe_apply_attrs(store, p, &edited.code, b)? {
                Some(json) => json,
                None => ser(&edited)?,
            };
            with_diagram_warnings(body, &warnings)
        }
        ("POST", ["projects", p, "features", code, "move"]) => {
            // Entering an active status needs a current approval (FEAT-048); an explicit
            // `unapproved` reason is recorded rather than silently allowed.
            // `override` is the name since gates became declarable (FEAT-113); `unapproved` stays.
            let reason = str_field(b, "override").or_else(|| str_field(b, "unapproved"));
            // A dry run judges the move — status, edge and gate — and changes nothing, so a caller
            // can learn a move would be refused before it makes a branch (DEF-007).
            if is_dry_run(Some(b)) {
                let warnings = store.gate_preview(
                    p,
                    code,
                    &str_field(b, "to").unwrap_or_default(),
                    reason.as_deref(),
                )?;
                return ser(&json!({
                    "dry_run": true,
                    "gate_warnings": warnings.iter().map(|g| &g.message).collect::<Vec<_>>(),
                }));
            }
            // With the git model on, a move that integrates merges the item's branch (FEAT-190).
            let moved = crate::gitmodel::move_item(
                store,
                p,
                code,
                &str_field(b, "to").unwrap_or_default(),
                reason.as_deref(),
                repo,
                &str_field(b, "by").unwrap_or_else(|| "kanbanr".to_string()),
                &crate::origin::from_body(b),
            )?;
            let mut out = serde_json::to_value(&moved.feature)
                .map_err(|e| CoreError::Unsupported(e.to_string()))?;
            if !moved.warnings.is_empty() {
                out["gate_warnings"] = json!(
                    moved
                        .warnings
                        .iter()
                        .map(|g| &g.message)
                        .collect::<Vec<_>>()
                );
            }
            if let Some(merged) = &moved.merged {
                out["merged"] = json!(merged);
            }
            if let Some(v) = &moved.forward_needed {
                out["forward_needed"] = json!(v);
            }
            if let Some(next) = &moved.forward_next {
                out["forward_next"] = json!(next);
            }
            if let Some(sent) = &moved.sent_back {
                out["release_sent_back"] = json!(sent);
            }
            if let Some(path) = &moved.notes_redrafted {
                out["notes_redrafted"] = json!(path);
            }
            if let Some(note) = &moved.note {
                out["note"] = json!(note);
            }
            if !moved.drift.is_empty() {
                out["drift"] = json!(moved.drift);
            }
            ser(&out)
        }
        ("PUT", ["projects", p, "features", code, "tests", requirement, test]) => {
            // Path segments arrive percent-encoded: a test name is free text and routinely holds
            // `::`, spaces or `/`, none of which survive a raw path match.
            let (requirement, test) = (&percent_decode(requirement), &percent_decode(test));
            let state = crate::models::TestState::parse(&str_field(b, "state").unwrap_or_default())
                .ok_or_else(|| {
                    CoreError::Unsupported("state must be planned, red or green".into())
                })?;
            let mut checked_rev = str_field(b, "checked_rev").filter(|r| !r.trim().is_empty());
            // A manual result says a person checked something: keep who, when and from where
            // beside the test's state (DEF-040). The kept result goes first, so a failed write
            // leaves the state alone; if the state then fails, the kept result is taken back.
            // Before the item's build stage nothing exists to check (DEF-076): refused before
            // anything is kept.
            crate::manual::refuse_before_build(
                &store.load_meta(p)?,
                code,
                requirement,
                test,
                state,
            )?;
            let kept = if matches!(state, TestState::Green | TestState::Red)
                && is_manual_test(store, p, code, requirement, test)
            {
                if checked_rev.is_none() {
                    checked_rev = repo.and_then(|r| crate::coderepo::rev_parse(r, "HEAD").ok());
                }
                Some(crate::manual::record(
                    store,
                    p,
                    &crate::manual::ManualResult {
                        code: code.to_string(),
                        requirement: requirement.to_string(),
                        test: test.to_string(),
                        state: if state == TestState::Green {
                            "green"
                        } else {
                            "red"
                        }
                        .into(),
                        by: approver(b)?,
                        at: crate::now_rfc3339(),
                        origin: crate::origin::from_body(b),
                        checked_rev: checked_rev.clone().unwrap_or_default(),
                        note: str_field(b, "note").unwrap_or_default(),
                        prepared: str_field(b, "prepared").unwrap_or_default(),
                        ..Default::default()
                    },
                )?)
            } else {
                None
            };
            match store.set_test_state(
                p,
                code,
                requirement.as_str(),
                test.as_str(),
                state,
                checked_rev.as_deref(),
            ) {
                Ok(f) => ser(&f),
                Err(e) => {
                    if let Some(path) = kept {
                        let _ = std::fs::remove_file(path);
                    }
                    Err(e)
                }
            }
        }
        ("GET", ["projects", p, "manual-tests"]) => ser(&crate::manual::waiting(store, p)?),
        ("POST", ["projects", p, "features", code, "unapprove"]) => ser(&store
            .unapprove_feature_as(
                p,
                code,
                &approver(b)?,
                &str_field(b, "reason").unwrap_or_default(),
                &crate::origin::from_body(b),
            )?),
        ("POST", ["projects", p, "features", code, "ratify"]) => {
            shown_rev(store, p, code, b)?;
            if let Some(earlier) = repeat_ratification(store, p, code)? {
                return ser_already(store, p, code, &earlier);
            }
            ser(&store.ratify_feature_as(
                p,
                code,
                &approver(b)?,
                &str_field(b, "reason").unwrap_or_default(),
                &crate::origin::from_body(b),
                &crate::origin::delegation_from_body(b),
            )?)
        }
        ("POST", ["projects", p, "features", code, "approve"]) => {
            shown_rev(store, p, code, b)?;
            if let Some(earlier) = repeat_approval(store, p, code, "approved")? {
                return ser_already(store, p, code, &earlier);
            }
            ser(&store.approve_feature_as(
                p,
                code,
                &approver(b)?,
                &crate::origin::from_body(b),
                &crate::origin::delegation_from_body(b),
            )?)
        }
        // Withdraw a sign-off given in error (FEAT-182). Matched before the sign-off route.
        ("POST", ["projects", p, "features", code, "signoff", name, "withdraw"]) => ser(&store
            .withdraw_signoff_as(
                p,
                code,
                &percent_decode(name),
                &approver(b)?,
                &str_field(b, "reason").unwrap_or_default(),
                &crate::origin::from_body(b),
            )?),
        ("POST", ["projects", p, "features", code, "signoff", name]) => {
            shown_rev(store, p, code, b)?;
            shown_spec_rev(store, p, code, &percent_decode(name), b)?;
            let (name, note, doc) = (
                percent_decode(name),
                str_field(b, "note").unwrap_or_default(),
                str_field(b, "doc").unwrap_or_default(),
            );
            let item = store.load_meta(p)?.feature(code)?.clone();
            // `docs-folded` is judged against the item's branches (DEF-046): read them once.
            let folded = if name == crate::method::DOCS_FOLDED {
                let project = store.load_for_gates(p)?;
                let ctx = crate::readiness::Context::load_in(store, p, &project.config, repo);
                Some((project, ctx))
            } else {
                None
            };
            // A docs-folded whose branch moved on has lapsed, so giving it again is no repeat.
            let lapsed = folded.as_ref().is_some_and(|(project, ctx)| {
                project
                    .feature(code)
                    .is_ok_and(|f| crate::docsbuilt::moved_on(project, f, ctx).is_some())
            });
            if !lapsed
                && let Some(earlier) = crate::origin::duplicate_signoff(&item, &name, &note, &doc)
            {
                return ser_already_item(&item, &earlier);
            }
            // `docs-folded` is refused until the item is built and folded (DEF-033, DEF-046): the
            // same answer the Review page and `kanbanr check` give. Given, it pins the branches.
            let mut heads = std::collections::BTreeMap::new();
            if let Some((project, ctx)) = &folded {
                let feature = project.feature(code)?;
                if let Some(why) = crate::readiness::signoff_hold(project, feature, ctx, &name) {
                    return Err(CoreError::Unsupported(why));
                }
                heads = crate::docsbuilt::branch_heads(project, feature, ctx);
            }
            let signer = approver(b)?;
            let origin = crate::origin::from_body(b);
            let (checked, settled) =
                signoff_checked(store, p, code, &name, b, repo, (&signer, &origin))?;
            // Given with the code repository, it records where the item stood and the blob of each
            // repository doc it lists there (FTR-004). A position that cannot be resolved steps
            // aside (ADR-0004): the sign-off is recorded without it.
            let (position, docs_read) = signoff_location(store, p, code, repo);
            let (feature, held) = store.signoff_feature_located_as(
                p,
                code,
                &name,
                &signer,
                &note,
                &doc,
                &origin,
                &crate::origin::delegation_from_body(b),
                &checked,
                &heads,
                &position,
                &docs_read,
            )?;
            // The sign-off answers the settlements it took (DEF-039).
            for id in settled {
                crate::prepared::resolve(
                    store,
                    p,
                    &crate::prepared::Resolution {
                        id,
                        verdict: crate::prepared::Answer::Confirmed,
                        by: signer.clone(),
                        origin: origin.clone(),
                        at: crate::now_rfc3339(),
                        changed: Vec::new(),
                        reason: String::new(),
                        made: format!("POST /projects/{p}/features/{code}/signoff/{name}"),
                        extra: Default::default(),
                    },
                )?;
            }
            let mut out = serde_json::to_value(&feature)
                .map_err(|e| CoreError::Unsupported(e.to_string()))?;
            if let Some(held) = held {
                out["auto_advance_held"] = json!(held);
            }
            ser(&out)
        }
        ("POST", ["projects", p, "features", code, "todos"]) => ser(&store.add_todo_list(
            p,
            code,
            &str_field(b, "description").unwrap_or_default(),
            str_field(b, "code"),
        )?),
        // `{text, key?, depends_on?, writes?, reads?, estimate_h?}` (FTR-060 adds the last four).
        ("POST", ["projects", p, "features", code, "todos", todo, "tasks"]) => {
            let fields = crate::store::TaskFields {
                estimate_h: task_estimate(b)?.flatten().map(Some),
                ..task_fields(b)
            };
            ser(&store.add_task(
                p,
                code,
                todo,
                &str_field(b, "text").unwrap_or_default(),
                str_field(b, "key"),
                &fields,
            )?)
        }
        // `{text?, depends_on?, writes?, reads?, estimate_h?}` (FTR-060): what a task records; an
        // absent field stays, an empty list or `estimate_h: null` clears it. Never its state.
        ("PATCH", ["projects", p, "features", code, "todos", todo, "tasks", key]) => {
            let fields = crate::store::TaskFields {
                estimate_h: task_estimate(b)?,
                text: str_field(b, "text"),
                ..task_fields(b)
            };
            ser(&store.edit_task(p, code, todo, key, &fields)?)
        }
        ("PUT", ["projects", p, "features", code, "todos", todo, "tasks", key]) => {
            let raw = str_field(b, "state").unwrap_or_default();
            let state = TaskState::parse(&raw).ok_or(CoreError::InvalidTaskState(raw))?;
            let hold = str_field(b, "hold_advance");
            // What the agent reports with the change, kept in the task log (FTR-061).
            let build = match b.get("build").filter(|v| !v.is_null()) {
                Some(v) => Some(
                    serde_json::from_value::<crate::tasklog::BuildTotals>(v.clone())
                        .map_err(|e| CoreError::Unsupported(format!("build: {e}")))?,
                ),
                None => None,
            };
            let report = crate::store::TaskReport {
                tokens: b.get("tokens").and_then(|v| v.as_u64()),
                cost_usd: b.get("cost_usd").and_then(|v| v.as_f64()),
                model: str_field(b, "model"),
                build,
            };
            let (feature, held) = store.set_task_state_reporting(
                p,
                code,
                todo,
                key,
                state,
                hold.as_deref(),
                &report,
            )?;
            let mut out = serde_json::to_value(&feature)
                .map_err(|e| CoreError::Unsupported(e.to_string()))?;
            if let Some(held) = held {
                out["auto_advance_held"] = json!(held);
            }
            ser(&out)
        }

        // ---- milestones ----
        ("POST", ["projects", p, "milestones"]) => ser(&store.add_milestone(
            p,
            &str_field(b, "name").unwrap_or_default(),
            &str_field(b, "description").unwrap_or_default(),
            vec_field(b, "depends_on").unwrap_or_default(),
            str_field(b, "code"),
        )?),
        ("PATCH", ["projects", p, "milestones", code]) => ser(&store.edit_milestone(
            p,
            code,
            str_field(b, "name"),
            str_field(b, "description"),
            vec_field(b, "depends_on"),
        )?),
        ("DELETE", ["projects", p, "milestones", code]) => {
            store.delete_milestone(p, code)?;
            Ok(String::new())
        }

        // ---- config ----
        ("POST", ["projects", p, "config", "transition"]) => ser(&store.set_transition(
            p,
            &str_field(b, "from").unwrap_or_default(),
            &str_field(b, "to").unwrap_or_default(),
            bool_field(b, "allow").unwrap_or(false),
        )?),
        ("PUT", ["projects", p, "config", "displayed-states"]) => {
            ser(&store.set_displayed_states(p, vec_field(b, "states").unwrap_or_default())?)
        }
        ("PUT", ["projects", p, "config", "default-state"]) => {
            ser(&store.set_default_state(p, &str_field(b, "state").unwrap_or_default())?)
        }
        ("PUT", ["projects", p, "config", "no-op-states"]) => {
            ser(&store.set_no_op_states(p, vec_field(b, "states").unwrap_or_default())?)
        }
        ("POST", ["projects", p, "config", "rename-status"]) => ser(&store.rename_status(
            p,
            &str_field(b, "old").unwrap_or_default(),
            &str_field(b, "new").unwrap_or_default(),
        )?),
        ("PUT", ["projects", p, "config", "workflow"]) => apply_workflow(store, p, b),
        // ---- releases (FEAT-120) ----
        // `?scope=current|planned|past`: one Releases tab, newest first (FTR-041); without it,
        // every release as before.
        ("GET", ["projects", p, "releases"]) => match query_param(query, "scope").as_deref() {
            None => ser(&crate::releases::report_all(store, p)?),
            Some(scope) => {
                let scope = match scope {
                    "current" => crate::releases::Scope::Current,
                    "planned" => crate::releases::Scope::Planned,
                    "past" => crate::releases::Scope::Past,
                    _ => {
                        return Err(CoreError::Unsupported(
                            "scope is current, planned or past".into(),
                        ));
                    }
                };
                ser(&crate::releases::slice(store, p, scope)?)
            }
        },
        // The Releases page's counts per tab (FTR-041). Matched before the `{version}` arms; no
        // release can be called `summary`.
        ("GET", ["projects", p, "releases", "summary"]) => {
            ser(&crate::releases::summary(store, p)?)
        }
        // One release's item table (FTR-041), with the daemon's repository for docs-folded.
        ("GET", ["projects", p, "releases", version, "items"]) => ser(
            &crate::releases::items_table_in(store, p, &percent_decode(version), repo)?,
        ),
        // `patch_of`: a patch of a cut release, branched from its tag (FEAT-184).
        ("POST", ["projects", p, "releases"]) => match str_field(b, "patch_of") {
            Some(parent) => ser(&crate::releases::add_patch(
                store,
                p,
                &str_field(b, "version").unwrap_or_default(),
                &str_field(b, "target").unwrap_or_default(),
                &str_field(b, "name").unwrap_or_default(),
                &parent,
                &str_field(b, "by").unwrap_or_default(),
                repo,
                &crate::origin::from_body(b),
            )?),
            None => {
                let release = crate::releases::add(
                    store,
                    p,
                    &str_field(b, "version").unwrap_or_default(),
                    &str_field(b, "target").unwrap_or_default(),
                    &str_field(b, "name").unwrap_or_default(),
                )?;
                let mut out = serde_json::to_value(&release)
                    .map_err(|e| CoreError::Unsupported(format!("serialize: {e}")))?;
                // What the first release still needs (DEF-056): said, never enforced. A failure to
                // read leaves `next` out and the add stands.
                if let (Ok(meta), Ok(all)) = (store.load_meta(p), crate::releases::load(store, p)) {
                    let next = crate::releases::first_release_hints(&meta.config, &all);
                    if !next.is_empty()
                        && let Some(o) = out.as_object_mut()
                    {
                        o.insert("next".into(), serde_json::json!(next));
                    }
                }
                ser(&out)
            }
        },
        ("POST", ["projects", p, "releases", version, "plan"]) => {
            let items = vec_field(b, "items").unwrap_or_default();
            // `decision?: "PD-n"` carries out the person's planning decision (FTR-049): checked
            // before any write, its outcome written with the change, in this one dispatch.
            let release = match decision_of(b) {
                Some(d) => crate::releases::plan_for_decision(
                    store,
                    p,
                    &percent_decode(version),
                    &items,
                    &d,
                    &crate::planning::carrier(store, str_field(b, "by"))?,
                    &crate::origin::from_body(b),
                )?,
                None => crate::releases::plan(store, p, &percent_decode(version), &items)?,
            };
            // Items now in a release but in no running sprint are named, not planned in
            // (DEF-029): the reply is the release as ever, with `not_in_sprint` when there are any.
            let project = store.load(p)?;
            let outside = crate::sprints::outside_running_sprint(
                &project,
                &crate::sprints::load(store, p)?,
                &crate::releases::load(store, p)?,
                Some(&items),
            );
            let mut body = serde_json::to_value(&release)
                .map_err(|e| CoreError::Unsupported(format!("serialize: {e}")))?;
            if !outside.is_empty() {
                body["not_in_sprint"] = serde_json::Value::Array(
                    outside
                        .iter()
                        .map(|o| serde_json::json!({"code": o.code, "sprint": o.sprint}))
                        .collect(),
                );
            }
            ser(&body)
        }
        // DEF-043: `accept_untested?` names the requirements the person accepts shipping untested.
        ("POST", ["projects", p, "releases", version, "cut"]) => ser(&crate::releases::cut_as(
            store,
            p,
            &percent_decode(version),
            &str_field(b, "by").unwrap_or_default(),
            &crate::origin::from_body(b),
            &vec_field(b, "accept_untested").unwrap_or_default(),
        )?),
        // What cutting a release now would ship and carry over, by the cut's own rule, with nothing
        // written (FEAT-185): the monitor's production confirmation lists exactly this.
        ("GET", ["projects", p, "releases", version, "cut-preview"]) => ser(
            &crate::releases::cut_preview(store, p, &percent_decode(version))?,
        ),
        // FEAT-183: `{stage?, by, reason?, expect?}` — the next stage, or the named one. `expect`
        // is the release tip the caller's plan showed (FEAT-212): a branch moved since is refused.
        ("POST", ["projects", p, "releases", version, "promote"]) => {
            let expect = str_field(b, "expect");
            // DEF-043: the untested requirements the person accepts shipping at the cut.
            let accept = vec_field(b, "accept_untested").unwrap_or_default();
            ser(&crate::releases::promote_with(
                store,
                p,
                &percent_decode(version),
                str_field(b, "stage").as_deref(),
                &str_field(b, "by").unwrap_or_default(),
                &str_field(b, "reason").unwrap_or_default(),
                repo,
                crate::releases::PromoteOptions {
                    expect_tip: expect.as_deref(),
                    fetch: crate::releases::Fetch::Try,
                    origin: &crate::origin::from_body(b),
                    accept_untested: &accept,
                },
            )?)
        }
        // DEF-034: `{by, reason, origin}` — step the release's newest stage back while nothing is
        // pushed or tagged; undoing the change that made release/<V> deletes it.
        ("POST", ["projects", p, "releases", version, "undo"]) => {
            ser(&crate::releases::undo_stage(
                store,
                p,
                &percent_decode(version),
                &str_field(b, "by").unwrap_or_default(),
                &str_field(b, "reason").unwrap_or_default(),
                &crate::origin::from_body(b),
                repo,
            )?)
        }
        // FTR-026: `{version, by, link?, notes?}`: a patch release started from the release's
        // failed tag result, with a defect carrying the run's link and the notes.
        ("POST", ["projects", p, "releases", version, "patch-from-tag"]) => {
            ser(&crate::releases::patch_from_failed_tag(
                store,
                p,
                &percent_decode(version),
                &str_field(b, "version").unwrap_or_default(),
                &str_field(b, "by").unwrap_or_default(),
                &str_field(b, "link").unwrap_or_default(),
                &str_field(b, "notes").unwrap_or_default(),
                repo,
                &crate::origin::from_body(b),
            )?)
        }
        // What promoting a release would do, with nothing changed (FEAT-212): `?stage=` (default
        // the next), `?reason=` for a move back, `?fetch=1` to fetch main first (the CLI's
        // `--plan --fetch`; the monitor never asks).
        ("GET", ["projects", p, "releases", version, "plan"]) => {
            let fetch = match query_param(query, "fetch").as_deref() {
                Some("1") | Some("true") => crate::releases::Fetch::Try,
                _ => crate::releases::Fetch::No,
            };
            ser(&crate::releases::plan_promotion(
                store,
                p,
                &percent_decode(version),
                query_param(query, "stage").as_deref(),
                &query_param(query, "reason").unwrap_or_default(),
                repo,
                fetch,
            )?)
        }
        // A release's notes as they are on release/<V> (FEAT-216), read with the code repository.
        ("GET", ["projects", p, "releases", version, "notes"]) => ser(
            &crate::releases::release_notes(store, p, &percent_decode(version), repo)?,
        ),
        // `{text?, changelog?, by}`: commit the notes, the version's changelog section, or both
        // onto release/<V>, nothing checked out (FEAT-216, DEF-044).
        ("PUT", ["projects", p, "releases", version, "notes"]) => {
            ser(&crate::releases::set_release_notes(
                store,
                p,
                &percent_decode(version),
                repo,
                str_field(b, "text").as_deref(),
                str_field(b, "changelog").as_deref(),
                &str_field(b, "by").unwrap_or_default(),
            )?)
        }
        // Only an empty release that has not started (FTR-022).
        ("DELETE", ["projects", p, "releases", version]) => ser(&crate::releases::remove(
            store,
            p,
            &percent_decode(version),
        )?),
        // `{by}`: date the release's notes and changelog today, one commit onto release/<V>, main
        // moving with it when it holds the release (DEF-001).
        ("POST", ["projects", p, "releases", version, "redate"]) => {
            ser(&crate::releases::redate_release(
                store,
                p,
                &percent_decode(version),
                repo,
                &str_field(b, "by").unwrap_or_default(),
            )?)
        }
        // Recorded gate results, oldest first (FTR-002). `?name=` and `?commit=` narrow them.
        ("GET", ["projects", p, "gate-results"]) => {
            let name = query_param(query, "name");
            let commit = query_param(query, "commit").filter(|c| !c.is_empty());
            let mut all = crate::gates::results(store, p, name.as_deref())?;
            if let Some(c) = commit {
                all.retain(|r| r.commit.starts_with(&c));
            }
            ser(&all)
        }
        // Each test's last duration from the kept runs (FTR-058): an item's tracked tests with
        // `?code=`, read newest first; every test's without. Both read only the runs since the
        // last shipped release, or the newest 50 with none (R-11).
        ("GET", ["projects", p, "timings"]) => {
            only_params(query, &["code"])?;
            let code = query_param(query, "code").filter(|c| !c.is_empty());
            ser(&crate::timings::last_view(store, p, code.as_deref())?)
        }
        // The slowest tests and those that slowed most since the last shipped release (FTR-058),
        // over the runs since it (the newest 50 with none). `?top=` (default 10, at most 100).
        ("GET", ["projects", p, "timings", "report"]) => {
            only_params(query, &["top"])?;
            let top = match query_param(query, "top") {
                Some(n) => n.parse::<usize>().map_err(|_| {
                    CoreError::Unsupported(format!("top must be a number, not '{n}'"))
                })?,
                None => 10,
            };
            ser(&crate::timings::report_read(store, p, top)?)
        }
        // `{name, commit, passed, by, url?, note?, source?}`: keep a gate result as a new file
        // (FTR-002). The time is the server's; `source` is `person` unless `local`.
        ("POST", ["projects", p, "gate-results"]) => {
            let need = |k: &str| {
                str_field(b, k)
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| CoreError::Unsupported(format!("a gate result needs '{k}'")))
            };
            let passed = bool_field(b, "passed")
                .ok_or_else(|| CoreError::Unsupported("a gate result needs 'passed'".into()))?;
            let result = crate::gates::GateResult {
                name: need("name")?,
                commit: need("commit")?,
                passed,
                by: need("by")?,
                at: crate::now_rfc3339(),
                source: str_field(b, "source")
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| "person".into()),
                url: str_field(b, "url").unwrap_or_default().trim().to_string(),
                note: str_field(b, "note").unwrap_or_default().trim().to_string(),
                origin: crate::origin::from_body(b),
            };
            crate::gates::record(store, p, &result)?;
            // The result is a fact and stays; a release that waited on it ships (DEF-049).
            let settled = crate::releases::settle_tagged(
                store,
                p,
                &result,
                &result.by,
                &result.origin,
                repo,
            )?;
            match settled {
                Some(cut) => {
                    let mut v = serde_json::to_value(&result)
                        .map_err(|e| CoreError::Unsupported(format!("serialize: {e}")))?;
                    v["settled"] = serde_json::Value::String(cut.release.version);
                    ser(&v)
                }
                None => ser(&result),
            }
        }
        // ---- where decisions are taken, and delegations (FTR-028) ----
        ("GET", ["projects", p, "decisions"]) => ser(&crate::decisions::view(
            store,
            p,
            repo,
            None,
            &crate::now_rfc3339(),
        )?),
        // `{mode, reason?}`: the person's alone; answers what waits where now.
        ("PUT", ["projects", p, "decisions", "mode"]) => {
            let given = str_field(b, "mode").unwrap_or_default();
            let mode = crate::decisions::Mode::parse(given.trim()).ok_or_else(|| {
                CoreError::Unsupported(format!(
                    "the decision mode is monitor, delegated or conversation, not '{given}'"
                ))
            })?;
            ser(&crate::decisions::set_mode(
                store,
                p,
                mode,
                &str_field(b, "reason").unwrap_or_default(),
                decision_stamp(b)?,
                None,
            )?)
        }
        // `{verdicts, steps, items, kinds, milestone, all?, until, note}`: a proposal, inert until
        // the person grants it.
        ("POST", ["projects", p, "decisions", "delegations"]) => {
            let new: crate::decisions::NewDelegation = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("a delegation: {e}")))?;
            ser(&crate::decisions::propose(
                store,
                p,
                new,
                decision_stamp(b)?,
            )?)
        }
        // `{rev}`: the person grants the proposal as they saw it.
        ("POST", ["projects", p, "decisions", "delegations", id, "grant"]) => {
            ser(&crate::decisions::grant(
                store,
                p,
                id,
                &str_field(b, "rev").unwrap_or_default(),
                decision_stamp(b)?,
            )?)
        }
        // `{reason}`: anyone may revoke.
        ("POST", ["projects", p, "decisions", "delegations", id, "revoke"]) => {
            ser(&crate::decisions::revoke(
                store,
                p,
                id,
                &str_field(b, "reason").unwrap_or_default(),
                decision_stamp(b)?,
            )?)
        }
        // `{process?, dry_run?}`: propose the process's default delegations (R-5); with dry_run,
        // only what they would be.
        ("POST", ["projects", p, "decisions", "defaults"]) => {
            let process = str_field(b, "process").filter(|x| !x.trim().is_empty());
            match crate::decisions::propose_defaults(
                store,
                p,
                process.as_deref(),
                decision_stamp(b)?,
                is_dry_run(Some(b)),
            )? {
                Ok(made) => ser(&made),
                Err(preview) => ser(&preview),
            }
        }
        // `{decision, verdict: keep|undo, reason?}`: the person keeps or undoes a delegated
        // decision; an undo withdraws it, in the same commit.
        ("POST", ["projects", p, "decisions", "reviews"]) => {
            let keep = match str_field(b, "verdict").as_deref() {
                Some("keep") => true,
                Some("undo") => false,
                other => {
                    return Err(CoreError::Unsupported(format!(
                        "a review's verdict is keep or undo, not '{}'",
                        other.unwrap_or("")
                    )));
                }
            };
            ser(&crate::decisions::review(
                store,
                p,
                &str_field(b, "decision").unwrap_or_default(),
                keep,
                &str_field(b, "reason").unwrap_or_default(),
                decision_stamp(b)?,
            )?)
        }
        // ---- the engineering baseline (FTR-033) ----
        ("GET", ["projects", p, "baseline"]) => ser(&crate::baseline::view(store, p, None)?),
        // `{name, file?, dry_run?}`: follow a pack, or update it to its library's current copy;
        // `file` carries a personal pack whole, as a personal process is sent. With `dry_run`,
        // only what it would change.
        ("POST", ["projects", p, "baseline", "packs"]) => {
            let name = str_field(b, "name").unwrap_or_default().trim().to_string();
            if name.is_empty() {
                return Err(CoreError::Unsupported("a pack needs a name".into()));
            }
            let (file, library) = match b.get("file").filter(|f| !f.is_null()) {
                Some(f) => (
                    serde_json::from_value::<crate::profile::ProfileFile>(f.clone())
                        .map_err(|e| CoreError::Unsupported(format!("invalid pack file: {e}")))?,
                    crate::config::Library::Personal,
                ),
                None => store.resolve_profile(&name, None)?,
            };
            let before = crate::baseline::load(store, p)?;
            let applied = crate::baseline::applied_pack(store, p, &name)?;
            let following = before
                .packs
                .iter()
                .any(|r| r.source.name == name && r.removed.is_none());
            let diff = match &applied {
                Some(old) => crate::profile::diff(old, &file),
                None => Vec::new(),
            };
            let answers_kept = before.areas.len();
            if is_dry_run(Some(b)) {
                return ser(&json!({
                    "pack": file.source(&name, library),
                    "updated": following,
                    "diff": diff,
                    "answers_kept": answers_kept,
                    "dry_run": true,
                }));
            }
            let after =
                crate::baseline::add_pack(store, p, &name, &file, library, decision_stamp(b)?)?;
            let record = after.packs.iter().find(|r| r.source.name == name);
            ser(&json!({
                "pack": record,
                "updated": following,
                "diff": diff,
                "answers_kept": answers_kept,
            }))
        }
        ("DELETE", ["projects", p, "baseline", "packs", name]) => {
            let name = percent_decode(name);
            let after = crate::baseline::remove_pack(store, p, &name, decision_stamp(b)?)?;
            ser(&json!({
                "pack": after.packs.iter().find(|r| r.source.name == name),
                "answers_kept": after.areas.len(),
            }))
        }
        // `{item?, note?, answers}`: a proposed revision, inert until the person accepts it.
        ("POST", ["projects", p, "baseline", "revisions"]) => {
            let proposal: crate::baseline::Proposal = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("a baseline proposal: {e}")))?;
            let current = crate::baseline::load(store, p)?;
            let packs = crate::baseline::followed_packs(store, p, &current)?;
            ser(&crate::baseline::propose(
                store,
                p,
                proposal,
                &packs,
                decision_stamp(b)?,
            )?)
        }
        // `{rev, by}`: the person accepts the revision as they saw it; theirs in every mode.
        ("POST", ["projects", p, "baseline", "revisions", id, "accept"]) => {
            ser(&crate::baseline::accept(
                store,
                p,
                id,
                &str_field(b, "rev").unwrap_or_default(),
                decision_stamp(b)?,
            )?)
        }
        // `{reason, by}`: anyone may withdraw a proposed revision.
        ("POST", ["projects", p, "baseline", "revisions", id, "withdraw"]) => {
            ser(&crate::baseline::withdraw(
                store,
                p,
                id,
                &str_field(b, "reason").unwrap_or_default(),
                decision_stamp(b)?,
            )?)
        }
        // FTR-034: `{kind, subject, fields, sources, by?}`: the evidence for a decision, prepared for
        // the person to confirm. Not a verdict: nothing a gate reads changes.
        // A gate result no release stage asks for is recorded with a `warning` (DEF-095).
        ("POST", ["projects", p, "prepared"]) => {
            let record = crate::prepared::prepare(store, p, b, repo)?;
            let mut out = serde_json::to_value(&record)
                .map_err(|e| CoreError::Unsupported(format!("serialize: {e}")))?;
            if record.kind == "gate-result"
                && let Some(w) = crate::prepared::unrequired_gate_warning(
                    &store.load_meta(p)?,
                    &record.subject.gate,
                )
            {
                out["warning"] = Value::String(w);
            }
            ser(&out)
        }
        // `?state=open|all&item=CODE&release=V`: the prepared records, newest first (FTR-034).
        ("GET", ["projects", p, "prepared"]) => {
            let state = query_param(query, "state").unwrap_or_else(|| "open".into());
            if state != "open" && state != "all" {
                return Err(CoreError::Unsupported(format!(
                    "state is open or all, not '{state}'"
                )));
            }
            let q = crate::prepared::ListQuery {
                all: state == "all",
                item: query_param(query, "item").filter(|v| !v.is_empty()),
                release: query_param(query, "release").filter(|v| !v.is_empty()),
            };
            ser(&crate::prepared::list(store, p, &q, repo)?)
        }
        // FTR-049: `{action, target, items, by, note?}`: the person's planning decision from a
        // page. It changes no plan; Claude carries it out with `--decision` or declines it. A
        // verdict route: refused to Claude, and stamped with its origin.
        ("POST", ["projects", p, "planning"]) => ser(&crate::planning::record(
            store,
            p,
            b,
            &crate::origin::from_body(b),
        )?),
        // `?state=pending|carried|declined`: the planning decisions, oldest first (FTR-049).
        ("GET", ["projects", p, "planning"]) => {
            only_params(query, &["state"])?;
            let state = query_param(query, "state").filter(|v| !v.is_empty());
            ser(&crate::planning::list(store, p, state.as_deref())?)
        }
        // A decision checked against frozen scope, dependencies, capacity and dates (FTR-049).
        ("GET", ["projects", p, "planning", id, "check"]) => ser(&crate::planning::assess(
            store,
            p,
            id,
            crate::sprints::today(),
        )?),
        // `{by, reason, nearest?: {action, target}}`: Claude (or the person) declines a decision.
        // Not a verdict (FTR-049).
        ("POST", ["projects", p, "planning", id, "decline"]) => {
            let nearest = match b.get("nearest") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    serde_json::from_value::<crate::planning::Nearest>(v.clone()).map_err(|e| {
                        CoreError::Unsupported(format!("nearest is {{action, target}}: {e}"))
                    })?,
                ),
            };
            ser(&crate::planning::decline(
                store,
                p,
                id,
                &str_field(b, "by").unwrap_or_default(),
                &crate::origin::from_body(b),
                &str_field(b, "reason").unwrap_or_default(),
                nearest,
            )?)
        }
        // `{by, reason?}`: the person turns a prepared record down; nothing else changes (FTR-034).
        ("POST", ["projects", p, "prepared", id, "reject"]) => {
            ser(&crate::prepared::reject(store, p, id, b)?)
        }
        // `{by, reason?}`: the person turns a whole batch down at once (FTR-037).
        ("POST", ["projects", p, "prepared", "batch", batch, "reject"]) => ser(&json!({
            "batch": batch,
            "results": crate::prepared::reject_batch(store, p, batch, b)?,
        })),
        // A batch's confirmations make the person's verdicts, so only the write path takes them:
        // each expands into its verdict's own route, in one commit (FTR-037).
        ("POST", ["projects", _p, "prepared", "batch", _batch, "confirm"]) => {
            Err(CoreError::Unsupported(
                "a prepared batch is confirmed through the write path, which records each verdict \
                 as the person"
                    .into(),
            ))
        }
        // A group decision makes one verdict per row, each the person's, so only the write path
        // takes it: each row goes through its own route, in one commit (FTR-038).
        ("POST", ["projects", _p, "review", "decide"]) => Err(CoreError::Unsupported(
            "a group decision is taken through the write path, which records each verdict as \
             the person"
                .into(),
        )),
        // A confirmation makes the person's verdict, so only the write path takes it: it expands
        // into the verdict's own route (FTR-034).
        ("POST", ["projects", _p, "prepared", _id, "confirm"]) => Err(CoreError::Unsupported(
            "a prepared record is confirmed through the write path, which records the verdict \
             as the person"
                .into(),
        )),
        // `{gates: [name…]}`: the gates `gate_passed` requires; empty clears them (FTR-002).
        ("PUT", ["projects", p, "config", "commit-gates"]) => {
            let names: Vec<String> = vec_field(b, "gates")
                .unwrap_or_default()
                .iter()
                .map(|g| g.trim().to_string())
                .filter(|g| !g.is_empty())
                .collect();
            let mut cadence = store.load_meta(p)?.config.cadence;
            cadence.commit_gates = names;
            ser(&store.set_cadence(p, None, cadence)?)
        }
        // `{gate}`: the commit gate whose result judges a release's tag; empty goes back to the
        // first commit gate (FTR-026).
        ("PUT", ["projects", p, "config", "tag-gate"]) => {
            let gate = str_field(b, "gate")
                .map(|g| g.trim().to_string())
                .filter(|g| !g.is_empty());
            let mut cadence = store.load_meta(p)?.config.cadence;
            cadence.tag_gate = gate;
            ser(&store.set_cadence(p, None, cadence)?)
        }
        // `{paths: [entry…]}`: the paths that count as documentation when a recorded gate pass is
        // carried across a change (DEF-031). Empty goes back to the default; `{none: true}` says
        // that no path counts as documentation.
        ("PUT", ["projects", p, "config", "gate-docs"]) => {
            let paths: Vec<String> = vec_field(b, "paths")
                .unwrap_or_default()
                .iter()
                .map(|e| e.trim().to_string())
                .collect();
            let mut cadence = store.load_meta(p)?.config.cadence;
            cadence.gate_docs = if bool_field(b, "none").unwrap_or(false) {
                Some(Vec::new())
            } else if paths.is_empty() {
                None
            } else {
                Some(paths)
            };
            ser(&store.set_cadence(p, None, cadence)?)
        }
        // The release stages with the gate in force on each (FEAT-212).
        ("GET", ["projects", p, "release-stages"]) => {
            ser(&store.load_meta(p)?.config.cadence.release_stage_views())
        }
        // `{purpose?, requires?, after?, on_enter?, clear?}`: declare a stage's gate, starting from the
        // one in force, or clear it back to the default (FEAT-212).
        ("PUT", ["projects", p, "config", "release-stages", stage]) => {
            let stage = percent_decode(stage);
            let gate = if bool_field(b, "clear").unwrap_or(false) {
                None
            } else {
                let cadence = store.load_meta(p)?.config.cadence;
                let mut gate = cadence
                    .effective_release_gates()
                    .into_iter()
                    .find(|(s, _)| *s == stage)
                    .map(|(_, g)| g)
                    .ok_or_else(|| {
                        CoreError::Unsupported(format!(
                            "'{stage}' is not a release stage — this project's are {}",
                            cadence.stages().join(", ")
                        ))
                    })?;
                if let Some(purpose) = str_field(b, "purpose") {
                    gate.purpose = purpose.trim().to_string();
                }
                if let Some(v) = b.get("requires").filter(|v| !v.is_null()) {
                    gate.requires = serde_json::from_value(v.clone())
                        .map_err(|e| CoreError::Unsupported(e.to_string()))?;
                }
                if let Some(after) = vec_field(b, "after") {
                    gate.after = after
                        .into_iter()
                        .map(|a| a.trim().to_string())
                        .filter(|a| !a.is_empty())
                        .collect();
                }
                if let Some(v) = b.get("on_enter").filter(|v| !v.is_null()) {
                    gate.on_enter = serde_json::from_value(v.clone())
                        .map_err(|e| CoreError::Unsupported(e.to_string()))?;
                }
                Some(gate)
            };
            let config = store.set_release_gate(p, &stage, gate)?;
            ser(&config.cadence.release_stage_views())
        }
        // `{version_files?, changelog?, changelog_copies?, notes?, notes_listings?}`: what
        // `prepare` writes (FEAT-212). A field left out is kept; `""` clears a path.
        ("PUT", ["projects", p, "config", "release-prepare"]) => {
            let mut prep = store.load_meta(p)?.config.cadence.release_prepare;
            let invalid = |e: serde_json::Error| CoreError::Unsupported(e.to_string());
            if let Some(v) = b.get("version_files").filter(|v| !v.is_null()) {
                prep.version_files = serde_json::from_value(v.clone()).map_err(invalid)?;
            }
            if let Some(c) = str_field(b, "changelog") {
                prep.changelog = Some(c.trim().to_string()).filter(|c| !c.is_empty());
            }
            if let Some(copies) = vec_field(b, "changelog_copies") {
                prep.changelog_copies = copies
                    .into_iter()
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty())
                    .collect();
            }
            if let Some(n) = str_field(b, "notes") {
                prep.notes = Some(n.trim().to_string()).filter(|n| !n.is_empty());
            }
            if let Some(v) = b.get("notes_listings").filter(|v| !v.is_null()) {
                prep.notes_listings = serde_json::from_value(v.clone()).map_err(invalid)?;
            }
            ser(&store.set_release_prepare(p, prep)?.cadence.release_prepare)
        }
        // ---- sprints (FEAT-119) ----
        ("GET", ["projects", p, "sprints"]) => ser(&crate::sprints::as_of(
            &crate::sprints::load(store, p)?,
            crate::sprints::today(),
        )),
        ("GET", ["projects", p, "sprints", code]) => {
            let code = match *code {
                "active" => crate::sprints::active(&crate::sprints::load(store, p)?)
                    .map(|s| s.code.clone())
                    .ok_or_else(|| CoreError::Unsupported("no sprint is active".into()))?,
                c => c.to_string(),
            };
            ser(&crate::sprints::report(
                store,
                p,
                &code,
                crate::sprints::today(),
            )?)
        }
        ("POST", ["projects", p, "sprints"]) => ser(&crate::sprints::add(
            store,
            p,
            &str_field(b, "start").unwrap_or_default(),
            b.get("length_days")
                .and_then(Value::as_u64)
                .map(|d| d as u32),
            &str_field(b, "goal").unwrap_or_default(),
            b.get("capacity").and_then(Value::as_f64),
            &str_field(b, "name").unwrap_or_default(),
            b.get("lanes").and_then(Value::as_u64).map(|n| n as u32),
            b.get("build_slots")
                .and_then(Value::as_u64)
                .map(|n| n as u32),
        )?),
        // `{name?, goal?, start?, length_days?, end?, capacity?, lanes?, build_slots?}`: reshape a
        // sprint that has not started; `capacity: null` clears it (FTR-022), as `lanes: null` and
        // `build_slots: null` do (FTR-061).
        ("PATCH", ["projects", p, "sprints", code]) => {
            // `lanes` and `build_slots` (FTR-061): a count, or null or 0 to clear.
            let count = |k: &str| -> Result<Option<Option<u32>>> {
                match b.get(k) {
                    None => Ok(None),
                    Some(Value::Null) => Ok(Some(None)),
                    Some(v) => Ok(Some(Some(
                        v.as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or_else(|| {
                                CoreError::Unsupported(format!(
                                    "{k} must be a whole number or null"
                                ))
                            })?,
                    ))),
                }
            };
            let edit = crate::sprints::SprintEdit {
                name: str_field(b, "name"),
                goal: str_field(b, "goal"),
                start: str_field(b, "start"),
                length_days: b
                    .get("length_days")
                    .and_then(Value::as_u64)
                    .map(|d| d as u32),
                end: str_field(b, "end"),
                capacity: match b.get("capacity") {
                    None => None,
                    Some(Value::Null) => Some(None),
                    Some(v) => Some(Some(v.as_f64().ok_or_else(|| {
                        CoreError::Unsupported("capacity must be a number or null".into())
                    })?)),
                },
                lanes: count("lanes")?,
                build_slots: count("build_slots")?,
            };
            ser(&crate::sprints::edit(store, p, code, &edit)?)
        }
        // Only an empty sprint that has not started (FTR-022).
        ("DELETE", ["projects", p, "sprints", code]) => {
            ser(&crate::sprints::remove(store, p, code)?)
        }
        ("POST", ["projects", p, "sprints", code, "plan"]) => {
            let items = vec_field(b, "items").unwrap_or_default();
            // `decision?: "PD-n"` carries out the person's planning decision (FTR-049).
            match decision_of(b) {
                Some(d) => ser(&crate::sprints::plan_for_decision(
                    store,
                    p,
                    code,
                    &items,
                    &d,
                    &crate::planning::carrier(store, str_field(b, "by"))?,
                    &crate::origin::from_body(b),
                )?),
                None => ser(&crate::sprints::plan(store, p, code, &items)?),
            }
        }
        // Take items out of their sprint (DEF-018).
        ("POST", ["projects", p, "sprints", "unplan"]) => {
            let items = vec_field(b, "items").unwrap_or_default();
            if items.is_empty() {
                return Err(CoreError::Unsupported("name at least one item".into()));
            }
            // `decision?: "PD-n"` carries out the person's planning decision (FTR-049).
            match decision_of(b) {
                Some(d) => ser(&crate::sprints::unplan_for_decision(
                    store,
                    p,
                    &items,
                    &d,
                    &crate::planning::carrier(store, str_field(b, "by"))?,
                    &crate::origin::from_body(b),
                )?),
                None => ser(&crate::sprints::unplan(store, p, &items)?),
            }
        }
        // With the git model on, starting a sprint makes its branch (FEAT-189).
        ("POST", ["projects", p, "sprints", code, "start"]) => {
            ser(&crate::gitmodel::sprint_start(store, p, code, repo)?)
        }
        // The git model the cadence implies (FEAT-189): always derivable, with the repository's
        // facts when there is one to read.
        ("GET", ["projects", p, "git", "model"]) => {
            // `?sprints=on|off&releases=on|off`: the model that cadence would give, over this
            // board's (FEAT-194 R-2). A preview, so it needs no repository.
            let on = |k: &str| query_param(query, k).map(|v| v == "on");
            let (sprints_on, releases_on) = (on("sprints"), on("releases"));
            if sprints_on.is_some() || releases_on.is_some() {
                let config = store.load_meta(p)?.config;
                let sprints = crate::sprints::load(store, p)?;
                let model =
                    crate::gitmodel::preview(config, sprints_on, releases_on, &sprints, repo);
                return ser(&crate::gitmodel::view(&model));
            }
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "the git model reads the code repository: run it from the project folder"
                        .into(),
                )
            })?;
            let (model, facts) = crate::gitmodel::load(store, p, repo)?;
            // `?item=CODE`: where that item's branch starts (FEAT-189, R-2).
            let start = match query_param(query, "item") {
                Some(code) => {
                    let project = store.load_meta(p)?;
                    let item = project.feature(&code)?;
                    let releases = crate::releases::load(store, p).unwrap_or_default();
                    let sprints = crate::sprints::load(store, p)?;
                    Some(crate::gitmodel::start_base(
                        &model, &facts, item, &releases, &sprints,
                    ))
                }
                None => None,
            };
            let mut out = crate::gitmodel::view(&model);
            out["facts"] = serde_json::to_value(&facts).unwrap_or_default();
            out["start"] = serde_json::to_value(&start).unwrap_or_default();
            ser(&out)
        }
        // With the git model on, a sprints-only close moves production (FEAT-190).
        ("POST", ["projects", p, "sprints", code, "close"]) => {
            let closed = crate::gitmodel::sprint_close(
                store,
                p,
                code,
                str_field(b, "carry_to").as_deref(),
                repo,
            )?;
            let mut out = serde_json::to_value(&closed.sprint)
                .map_err(|e| CoreError::Unsupported(e.to_string()))?;
            if let Some(tag) = &closed.tagged {
                out["tagged"] = json!(tag);
            }
            if !closed.catch_up.is_empty() {
                out["catch_up"] = json!(closed.catch_up);
            }
            ser(&out)
        }
        // The merged branches that can go (DEF-051): the list is read-only; the POST re-reads,
        // re-checks and deletes them locally, never touching a remote.
        ("GET", ["projects", p, "git", "tidy"]) => {
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "git tidy reads the code repository: run it from the project folder".into(),
                )
            })?;
            let (candidates, model) = crate::gitdrift::tidy_scan(store, p, repo)?;
            ser(&json!({ "candidates": candidates, "model_line": model.line }))
        }
        ("POST", ["projects", p, "git", "tidy"]) => {
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "git tidy deletes branches in the code repository: run it from the project \
                     folder"
                        .into(),
                )
            })?;
            ser(&crate::gitdrift::tidy_run(store, p, repo)?)
        }
        // A release's fixes merged forward into the line, on request (FEAT-190).
        ("POST", ["projects", p, "git", "forward", version]) => {
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "a forward merge acts on the code repository: run it from the project folder"
                        .into(),
                )
            })?;
            ser(&crate::gitmodel::forward(
                store,
                p,
                &percent_decode(version),
                repo,
            )?)
        }
        // The gates moves are judged by (FEAT-199): role-addressed gates on their stage, and the
        // built-in rule where a workflow declares none. The monitor's Workflow page reads these.
        ("GET", ["projects", p, "gates"]) => ser(&store.load_meta(p)?.config.effective_gates()),
        // Every guard rail the project has on, in one read (FEAT-213): the monitor's Workflow page
        // lists them. Internal, not in the published API.
        ("GET", ["projects", p, "guards"]) => ser(&crate::guards::report(store, p, repo)?),
        // The method, on by default (FEAT-193): which parts a project keeps.
        ("GET", ["projects", p, "config", "method"]) => {
            ser(&crate::method::status(&store.load_meta(p)?.config))
        }
        ("PUT", ["projects", p, "config", "method"]) => {
            let on = bool_field(b, "on").ok_or_else(|| {
                CoreError::Unsupported("say whether to turn the parts on or off: `on`".into())
            })?;
            let parts = crate::method::parse_parts(&vec_field(b, "parts").unwrap_or_default())?;
            let (config, notes) = store.set_method(p, on, &parts)?;
            ser(&serde_json::json!({
                "parts": crate::method::status(&config),
                "notes": notes,
            }))
        }
        // How the project treats diagrams the lint finds problems in (FEAT-200).
        ("GET", ["projects", p, "config", "diagrams"]) => {
            ser(&store.read_project_config(p)?.diagrams)
        }
        ("PUT", ["projects", p, "config", "diagrams"]) => {
            let strict = bool_field(b, "strict").ok_or_else(|| {
                CoreError::Unsupported("say whether to check diagrams strictly: `strict`".into())
            })?;
            let current = store.read_project_config(p)?.diagrams;
            let diagrams = crate::config::DiagramsConfig {
                strict,
                extra: current.extra,
            };
            ser(&store.set_diagrams(p, diagrams)?.diagrams)
        }
        // How agents' builds share the machine (FTR-062): each named key is set, or cleared by
        // `null` or `""`; keys not named are kept.
        ("PUT", ["projects", p, "config", "build"]) => {
            let mut config = store.load_meta(p)?.config;
            config.build = apply_build_change(config.build, b)?;
            store.save_config(p, &config)?;
            ser(&config.build)
        }
        // Which prefix each kind's code gets, and the next code per prefix (FEAT-217).
        ("GET", ["projects", p, "config", "codes"]) => ser(&codes_view(&store.load_meta(p)?)),
        ("PUT", ["projects", p, "config", "codes"]) => {
            let change: crate::config::CodesChange = if b.is_null() {
                Default::default()
            } else {
                serde_json::from_value(b.clone())
                    .map_err(|e| CoreError::Unsupported(format!("a codes change: {e}")))?
            };
            store.set_codes(p, change)?;
            ser(&codes_view(&store.load_meta(p)?))
        }
        // Architecture coverage rules (FEAT-197): each rule and what it leaves uncovered; set from
        // YAML text (`text`, parsed here in core) or as `rules`, refused when invalid.
        ("GET", ["projects", p, "coverage"]) => {
            let project = store.load_meta(p)?;
            if project.config.coverage.is_empty() {
                return ser(&crate::coverage::CoverageReport::default());
            }
            let snap = crate::coverage::Snapshot::load(store, &project)?;
            ser(&snap.report(&project))
        }
        ("PUT", ["projects", p, "coverage"]) => {
            let rules = match (str_field(b, "text"), b.get("rules")) {
                (Some(text), _) => crate::coverage::parse_rules(&text)?,
                (None, Some(rules)) => serde_json::from_value(rules.clone())
                    .map_err(|e| CoreError::Unsupported(format!("invalid coverage rules: {e}")))?,
                (None, None) => {
                    return Err(CoreError::Unsupported(
                        "give the coverage rules: `text` (YAML) or `rules`".into(),
                    ));
                }
            };
            store.set_coverage(p, rules)?;
            let project = store.load_meta(p)?;
            let snap = crate::coverage::Snapshot::load(store, &project)?;
            ser(&snap.report(&project))
        }
        ("PUT", ["projects", p, "config", "cadence"]) => {
            let unit = match str_field(b, "estimate_unit").as_deref() {
                None => None,
                Some("points") => Some(crate::config::EstimateUnit::Points),
                Some("days") => Some(crate::config::EstimateUnit::Days),
                Some(other) => {
                    return Err(CoreError::Unsupported(format!(
                        "estimate unit '{other}' is not days or points"
                    )));
                }
            };
            let current = store.load_meta(p)?.config.cadence;
            let current_inert = current.inert_stages();
            let current_cadence = current.clone();
            let named_git_model = bool_field(b, "git_model");
            let mut cadence = crate::config::Cadence {
                extra: current.extra.clone(),
                // FEAT-212: the stages' gates and what `prepare` writes have routes of their own;
                // a cadence write that does not name them keeps them.
                release_gates: current.release_gates.clone(),
                release_prepare: current.release_prepare.clone(),
                // FTR-002: the gates a release stage requires have a route of their own.
                commit_gates: current.commit_gates.clone(),
                tag_gate: current.tag_gate.clone(),
                gate_docs: current.gate_docs.clone(),
                // FTR-020: whether the project has CI; kept when the body does not say.
                has_ci: bool_field(b, "has_ci").unwrap_or(current.has_ci),
                sprints: bool_field(b, "sprints").unwrap_or(current.sprints),
                releases: bool_field(b, "releases").unwrap_or(current.releases),
                sprint_length_days: b
                    .get("sprint_length_days")
                    .and_then(Value::as_u64)
                    .map(|d| d as u32)
                    .or(current.sprint_length_days),
                release: str_field(b, "release").or(current.release),
                // FEAT-183. A list given empty is refused rather than read as "the default": the
                // last stage is the cut, so a project with none could never ship.
                release_stages: match vec_field(b, "release_stages") {
                    Some(stages) if stages.is_empty() => {
                        return Err(CoreError::Unsupported(
                            "a release needs at least one stage — the last one is the cut".into(),
                        ));
                    }
                    Some(stages) => stages.iter().map(|s| s.trim().to_string()).collect(),
                    None => current.release_stages,
                },
                // An empty string clears the branch back to the repository's default.
                integration_branch: match str_field(b, "integration_branch") {
                    Some(name) => Some(name.trim().to_string()).filter(|n| !n.is_empty()),
                    None => current.integration_branch,
                },
                main_branch: match str_field(b, "main_branch") {
                    Some(name) => Some(name.trim().to_string()).filter(|n| !n.is_empty()),
                    None => current.main_branch,
                },
                // FEAT-189: branching follows the git model only when switched on.
                git_model: named_git_model.unwrap_or(current.git_model),
            };
            // DEF-042: the first choice of sprints or releases turns the git model on.
            cadence.git_model = current_cadence.git_model_after(&cadence, named_git_model);
            let turned_on =
                cadence.git_model && !current_cadence.git_model && named_git_model.is_none();
            let config = store.set_cadence(p, unit, cadence)?;
            // FTR-020: an inert stage that is newly there, and how to make the tag wait for CI.
            let was = current_inert;
            let mut warnings: Vec<String> = config
                .cadence
                .inert_stages()
                .iter()
                .filter(|s| !was.contains(s))
                .map(|s| crate::config::Cadence::inert_warning(s))
                .collect();
            warnings.extend(config.cadence.has_ci_note(&current_cadence));
            if turned_on {
                warnings.push(
                    "the git model is on: kanbanr merges items into their line as `kanbanr git model` shows; `kanbanr config cadence --git-model off` turns it off"
                        .to_string(),
                );
            }
            let mut body =
                serde_json::to_value(&config).map_err(|e| CoreError::Unsupported(e.to_string()))?;
            if !warnings.is_empty() {
                body["warnings"] = serde_json::json!(warnings);
            }
            ser(&body)
        }
        // Workflow export (FEAT-039): the config rendered as a Mermaid state diagram.
        ("GET", ["projects", p, "workflow"]) => {
            let project = store.load(p)?;
            match query_param(query, "format").as_deref() {
                Some("mermaid") => Ok(crate::mermaid::to_state_diagram(&project.config)),
                other => Err(CoreError::Unsupported(format!(
                    "workflow format '{}' (only 'mermaid' is supported)",
                    other.unwrap_or("")
                ))),
            }
        }

        // ---- docs ----
        ("GET", ["projects", p, "docs"]) => ser(&store.doc_tree(p)?),
        ("PUT", ["projects", p, "docs", "folder"]) => {
            let folder = str_field(b, "path").unwrap_or_default();
            store.write_folder_meta(
                p,
                &folder,
                str_field(b, "name"),
                str_field(b, "description"),
            )?;
            Ok(json!({ "path": folder }).to_string())
        }
        ("GET", ["projects", p, "docs", "content"]) => {
            let rel = query_param(query, "path").unwrap_or_default();
            store.read_doc(p, &rel)
        }
        ("PUT", ["projects", p, "docs", "content"]) => {
            let path = str_field(b, "path").unwrap_or_default();
            let content = str_field(b, "content").unwrap_or_default();
            // FEAT-200: a markdown doc's diagrams are linted before it is written — refused on a
            // strict project, reported as warnings otherwise.
            let warnings = if crate::diagrams::is_markdown_path(&path) {
                crate::diagrams::guard(
                    &store.read_project_config(p)?.diagrams,
                    &format!("docs/{path}"),
                    &content,
                )?
            } else {
                Vec::new()
            };
            let saved = store.write_doc(p, &path, &content)?;
            with_diagram_warnings(json!({ "path": saved }).to_string(), &warnings)
        }
        ("DELETE", ["projects", p, "docs", "content"]) => {
            let rel = query_param(query, "path").unwrap_or_default();
            store.delete_doc(p, &rel)?;
            Ok(String::new())
        }

        // ---- feature definition (FEAT-047) ----
        ("PUT", ["projects", p, "features", code, "definition"]) => {
            let definition: Option<crate::models::FeatureDefinition> = if b.is_null() {
                None // an explicit null clears the block
            } else {
                Some(
                    serde_json::from_value(b.clone())
                        .map_err(|e| CoreError::Unsupported(format!("invalid definition: {e}")))?,
                )
            };
            ser(&store.set_feature_definition(p, code, definition)?)
        }

        // ---- defect record (FEAT-053) ----
        ("PUT", ["projects", p, "features", code, "defect"]) => {
            let defect: Option<crate::models::Defect> = if b.is_null() {
                None // an explicit null clears the block
            } else {
                Some(
                    serde_json::from_value(b.clone())
                        .map_err(|e| CoreError::Unsupported(format!("invalid defect: {e}")))?,
                )
            };
            ser(&store.set_defect(p, code, defect)?)
        }

        // ---- lessons learned (FEAT-055) ----
        ("GET", ["projects", p, "lessons"]) => {
            // `for` narrows to the lessons that bear on one item; `all` includes retired ones.
            let all = query_param(query, "all").is_some();
            let lessons = match query_param(query, "for") {
                Some(code) => {
                    let project = store.load_meta(p)?;
                    let feature = project.feature(&code)?.clone();
                    crate::lessons::surfaced(store, p, Some(&feature))?
                }
                None if all => crate::lessons::load(store, p)?,
                None => crate::lessons::surfaced(store, p, None)?,
            };
            ser(&lessons)
        }
        ("POST", ["projects", p, "lessons"]) => {
            let lesson: crate::lessons::Lesson = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid lesson: {e}")))?;
            ser(&crate::lessons::add(store, p, lesson)?)
        }
        ("POST", ["projects", p, "lessons", id, verdict]) => {
            let affirm = match *verdict {
                "affirm" => true,
                "contradict" => false,
                other => {
                    return Err(CoreError::Unsupported(format!(
                        "a lesson is affirmed or contradicted, not '{other}'"
                    )));
                }
            };
            ser(&crate::lessons::judge(
                store,
                p,
                id,
                affirm,
                &str_field(b, "note").unwrap_or_default(),
            )?)
        }

        // ---- traceability and decisions (FEAT-057) ----
        ("GET", ["projects", p, "trace"]) => {
            let subject = query_param(query, "subject").unwrap_or_default();
            let item = query_param(query, "item");
            let subject =
                crate::trace::parse_subject(&subject, item.as_deref()).ok_or_else(|| {
                    CoreError::Unsupported(
                    "trace what? a goal (G-2), an item (FEAT-046), a requirement (FEAT-046/R-2) or a doc (architecture/data.md)"
                        .to_string(),
                )
                })?;
            ser(&crate::trace::run(store, p, &subject)?)
        }
        ("GET", ["projects", p, "zachman"]) => ser(&crate::trace::zachman(
            store,
            p,
            query_param(query, "scope").as_deref(),
        )?),
        // ---- what is waiting for agreement (FEAT-067) ----
        // What an item is missing, from the one engine every surface uses (FEAT-112).
        // The diagrams of an item and the lint's findings (FEAT-200), for the review page's parse.
        ("GET", ["projects", p, "features", code, "diagrams"]) => {
            ser(&crate::diagrams::ItemDiagrams::load(store, p, code)?)
        }
        ("GET", ["projects", p, "features", code, "readiness"]) => ser(
            &crate::readiness::ReadinessView::load_in(store, p, code, repo)?,
        ),
        // The board's cards, in one request: gaps per live item, keyed by code. Finished and
        // parked items are left out, as doctor leaves them out — they are history, not work.
        ("GET", ["projects", p, "readiness"]) => ser(&crate::readiness::ProjectReadiness::load_in(
            store, p, repo,
        )?),
        ("GET", ["projects", p, "review"]) => ser(&crate::review::pending(store, p, repo)?),
        // The Review page's reads (FTR-038): the counts per decision group, one group's rows, and
        // one row's evidence. A manual row's key holds slashes, so the page percent-encodes it.
        ("GET", ["projects", p, "review", "summary"]) => {
            ser(&crate::reviewgroups::summary(store, p, repo)?)
        }
        ("GET", ["projects", p, "review", "groups", key]) => ser(&crate::reviewgroups::slice(
            store,
            p,
            &percent_decode(key),
            repo,
        )?),
        ("GET", ["projects", p, "review", "groups", key, "rows", row]) => {
            ser(&crate::reviewgroups::evidence(
                store,
                p,
                &percent_decode(key),
                &percent_decode(row),
                repo,
            )?)
        }
        ("GET", ["projects", p, "adrs"]) => ser(&crate::adr::list(store, p)?),
        ("GET", ["projects", p, "claude-block"]) => ser(&crate::claude::block(store, p)?),
        ("POST", ["projects", p, "adrs"]) => {
            let title = str_field(b, "title").unwrap_or_default();
            let adr: crate::adr::Adr = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid decision: {e}")))?;
            ser(&crate::adr::create(store, p, &title, adr)?)
        }
        ("POST", ["projects", p, "adrs", id, "accept"]) => ser(&crate::adr::decide_as(
            store,
            p,
            id,
            true,
            &approver(b)?,
            "",
            &crate::origin::from_body(b),
        )?),
        ("POST", ["projects", p, "adrs", id, "reject"]) => ser(&crate::adr::decide_as(
            store,
            p,
            id,
            false,
            &approver(b)?,
            &str_field(b, "reason").unwrap_or_default(),
            &crate::origin::from_body(b),
        )?),
        ("POST", ["projects", p, "adrs", id, "supersede"]) => {
            let replaces = str_field(b, "replaces").unwrap_or_default();
            ser(&crate::adr::supersede(store, p, id, &replaces)?)
        }

        // ---- wave retrospective (FEAT-054) ----
        ("GET", ["projects", p, "retro"]) => {
            let wave = crate::retro::Wave {
                milestone: query_param(query, "milestone"),
                since: query_param(query, "since"),
                label: query_param(query, "label"),
                sprint: query_param(query, "sprint"),
            };
            // With the code repository at hand, the proofs naming a missing test (MNT-001 Q5).
            ser(&crate::retro::run_in(store, p, &wave, repo)?)
        }
        ("GET", ["projects", p, "retro", "due"]) => ser(&crate::retro::due(store, p)?),

        // ---- an item sliced out of another (FEAT-054) ----
        ("PUT", ["projects", p, "features", code, "split-from"]) => {
            let parent = str_field(b, "parent");
            ser(&store.set_split_from(p, code, parent)?)
        }

        // ---- flow and quality report (FEAT-053) ----
        ("GET", ["projects", p, "report"]) => {
            let window = crate::report::Window {
                since: query_param(query, "since"),
            };
            ser(&crate::report::run(
                store,
                p,
                &window,
                query_param(query, "rev").as_deref(),
            )?)
        }

        // ---- what needs the person (FTR-048) ----
        ("GET", ["projects", p, "attention"]) => {
            only_params(query, &[])?;
            ser(&crate::attention::attention(store, p, repo)?)
        }

        // ---- the project Overview (FTR-048) ----
        ("GET", ["projects", p, "overview"]) => {
            only_params(query, &[])?;
            ser(&crate::overview::overview(store, p, repo)?)
        }

        // ---- items, filtered (FTR-048) ----
        ("GET", ["projects", p, "items"]) => {
            let filter = crate::items::parse_filter(query)?;
            // Staleness is judged against the code repository's HEAD; with none, nothing is stale.
            let head = repo.and_then(|r| crate::coderepo::rev_parse(r, "HEAD").ok());
            ser(&crate::items::list(store, p, &filter, head.as_deref())?)
        }

        // ---- project charter (FEAT-046) ----
        ("GET", ["projects", p, "charter", "progress"]) => {
            only_params(query, &[])?;
            ser(&crate::charterview::charter_progress(store, p)?)
        }
        ("GET", ["projects", p, "charter"]) => {
            store.load_meta(p)?; // the project must exist; an absent charter is a valid default
            ser(&crate::charter::load(store, p)?)
        }
        ("PUT", ["projects", p, "charter"]) => {
            store.load_meta(p)?;
            let charter: crate::charter::Charter = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid charter: {e}")))?;
            ser(&crate::charter::save(store, p, &charter)?)
        }

        // ---- the project's facts and its content catalog (FTR-005) ----
        ("GET", ["projects", p, "facts"]) => {
            store.load_meta(p)?; // the project must exist; no facts record reads as `{}`
            match crate::facts::load(store, p)? {
                Some(facts) => ser(&facts),
                None => ser(&json!({})),
            }
        }
        ("PUT", ["projects", p, "facts"]) => {
            store.load_meta(p)?;
            let facts: crate::facts::Facts = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid facts record: {e}")))?;
            ser(&crate::facts::save(store, p, &facts)?)
        }
        ("GET", ["projects", p, "catalog"]) => {
            let project = store.load(p)?;
            ser(&crate::catalog::status(store, &project, repo)?)
        }
        ("PUT", ["projects", p, "catalog"]) => {
            store.load_meta(p)?;
            let mut catalog = match (str_field(b, "text"), b.get("rows")) {
                (Some(text), _) => crate::catalog::parse(&text)?,
                (None, Some(rows)) => crate::catalog::Catalog {
                    schema: crate::catalog::SCHEMA,
                    rows: serde_json::from_value(rows.clone()).map_err(|e| {
                        CoreError::Unsupported(format!("invalid catalog rows: {e}"))
                    })?,
                    ..Default::default()
                },
                (None, None) => {
                    return Err(CoreError::Unsupported(
                        "give the catalog: `text` (YAML) or `rows`".into(),
                    ));
                }
            };
            // A bare list of rows names no starter version: keep the one the catalog began from.
            if catalog.starter == 0 {
                catalog.starter = crate::catalog::load(store, p)
                    .ok()
                    .flatten()
                    .map(|c| c.starter)
                    .unwrap_or_default();
            }
            crate::catalog::save(store, p, &catalog)?;
            ser(&crate::catalog::status(store, &store.load(p)?, repo)?)
        }
        // ---- the system baseline (FTR-006) ----
        ("GET", ["projects", p, "catalog", "baseline"]) => {
            ser(&crate::review::system_baseline_review(store, p, repo)?)
        }
        ("POST", ["projects", p, "catalog", "baseline", "init"]) => {
            store.load_meta(p)?;
            crate::systembaseline::init(store, p)?;
            ser(&crate::review::system_baseline_review(store, p, repo)?)
        }
        // `{note?, prepared?, rev?, by}`: the person accepts the system baseline as they saw it
        // (`rev` is the pins' hash they were shown); theirs in every mode.
        ("POST", ["projects", p, "catalog", "baseline", "accept"]) => {
            let project = store.load_meta(p)?;
            let stamp = decision_stamp(b)?;
            if let Some(seen) = str_field(b, "rev").filter(|r| !r.trim().is_empty()) {
                let now = crate::systembaseline::state(store, &project, repo)?;
                if now.pins_rev != seen.trim() {
                    return Err(CoreError::Unsupported(
                        "the system baseline's sources changed since you looked: read it again \
                         before accepting"
                            .into(),
                    ));
                }
            }
            crate::systembaseline::accept(
                store,
                p,
                &stamp.by,
                &stamp.origin,
                &str_field(b, "note").unwrap_or_default(),
                &str_field(b, "prepared").unwrap_or_default(),
                repo,
            )?;
            ser(&crate::review::system_baseline_review(store, p, repo)?)
        }
        ("POST", ["projects", p, "catalog", "init"]) => {
            store.load_meta(p)?;
            crate::catalog::init(store, p, bool_field(b, "force").unwrap_or(false))?;
            ser(&crate::catalog::status(store, &store.load(p)?, repo)?)
        }
        ("POST", ["projects", p, "catalog", "add"]) => {
            store.load_meta(p)?;
            let id = str_field(b, "id")
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| CoreError::Unsupported("give the starter row's `id`".into()))?;
            crate::catalog::add_row(store, p, id.trim())?;
            ser(&crate::catalog::status(store, &store.load(p)?, repo)?)
        }
        ("POST", ["projects", p, "catalog", "rows", id]) => {
            store.load_meta(p)?;
            let applies = bool_field(b, "applies")
                .ok_or_else(|| CoreError::Unsupported("give `applies`: true or false".into()))?;
            let why = str_field(b, "why").unwrap_or_default();
            crate::catalog::set_row(store, p, &percent_decode(id), applies, &why)?;
            ser(&crate::catalog::status(store, &store.load(p)?, repo)?)
        }

        // ---- issue mirror (FEAT-043) ----
        ("GET", ["projects", p, "mirror"]) => ser(&crate::mirror::load_config(store, p)?),
        ("PUT", ["projects", p, "mirror"]) => {
            let config: crate::mirror::MirrorConfig = serde_json::from_value(b.clone())
                .map_err(|e| CoreError::Unsupported(format!("invalid mirror config: {e}")))?;
            crate::mirror::save_config(store, p, &config)?;
            ser(&config)
        }

        // ---- agent scopes (FEAT-214): internal, like the guards ----
        ("GET", ["projects", p, "scopes"]) => {
            store.load_meta(p)?;
            let now = crate::now_rfc3339();
            let views: Vec<crate::agentscope::ScopeView> = crate::agentscope::load(store, p)?
                .iter()
                .map(|s| crate::agentscope::ScopeView::new(s, &now))
                .collect();
            ser(&views)
        }
        ("GET", ["projects", p, "scopes", "drift"]) => {
            let repo = repo.ok_or_else(|| {
                CoreError::Unsupported(
                    "agent drift reads the code repository: run it from the project folder".into(),
                )
            })?;
            ser(&crate::agentscope::drift(
                store,
                p,
                repo,
                &crate::now_rfc3339(),
            )?)
        }
        ("GET", ["projects", p, "scopes", id]) => {
            let scope = find_scope(store, p, id)?;
            ser(&crate::agentscope::ScopeView::new(
                &scope,
                &crate::now_rfc3339(),
            ))
        }
        ("GET", ["projects", p, "scopes", id, "brief"]) => {
            let scope = find_scope(store, p, id)?;
            Ok(crate::agentscope::brief(&scope, &store.load_meta(p)?))
        }
        ("POST", ["projects", p, "scopes"]) => {
            let role = str_field(b, "role").unwrap_or_default();
            let role = crate::agentscope::Role::parse(&role).ok_or_else(|| {
                CoreError::Unsupported(format!("an agent scope is design or build, not '{role}'"))
            })?;
            let req = crate::agentscope::IssueRequest {
                role,
                item: str_field(b, "item").unwrap_or_default(),
                task: str_field(b, "task").filter(|t| !t.trim().is_empty()),
                hours: b.get("hours").and_then(Value::as_u64).map(|h| h as u32),
                note: str_field(b, "note").unwrap_or_default(),
                worktree: str_field(b, "worktree")
                    .filter(|w| !w.trim().is_empty())
                    .map(std::path::PathBuf::from),
                hooks_ready: bool_field(b, "hooks_ready").unwrap_or(false),
                claude_hooks_ready: bool_field(b, "claude_hooks_ready").unwrap_or(false),
            };
            let by = crate::git::verdict_author(store.data_dir(), str_field(b, "by").as_deref())?;
            ser(&crate::agentscope::issue(
                store,
                p,
                &req,
                &by,
                repo,
                &crate::now_rfc3339(),
            )?)
        }
        ("POST", ["projects", p, "scopes", id, "claim"]) => {
            let wt = repo
                .and_then(crate::coderepo::worktree_facts)
                .ok_or_else(|| {
                    CoreError::Unsupported(
                        "a scope is claimed from inside the agent's worktree: run it there".into(),
                    )
                })?;
            let now = crate::now_rfc3339();
            let scope = crate::agentscope::claim(store, p, id, &wt, &now)?;
            ser(&crate::agentscope::ScopeView::new(&scope, &now))
        }
        ("POST", ["projects", p, "scopes", id, "revoke"]) => {
            let by = crate::git::verdict_author(store.data_dir(), str_field(b, "by").as_deref())?;
            let now = crate::now_rfc3339();
            let scope = crate::agentscope::revoke(
                store,
                p,
                id,
                &by,
                &str_field(b, "reason").unwrap_or_default(),
                &now,
            )?;
            let mut view = crate::agentscope::ScopeView::new(&scope, &now);
            // The agent's merged branch goes with the scope; any refusal leaves it (DEF-051).
            if let Some(repo) = repo {
                let cadence = store
                    .load_meta(p)
                    .ok()
                    .and_then(|m| m.config.cadence.main_branch.clone());
                let main = cadence.unwrap_or_else(|| crate::coderepo::default_branch(repo));
                view.branch_deleted = crate::agentscope::delete_agent_branch(&scope, repo, &main);
            }
            ser(&view)
        }
        ("POST", ["projects", p, "scopes", id, "renew"]) => {
            let hours = b
                .get("hours")
                .and_then(Value::as_u64)
                .map(|h| h as u32)
                .unwrap_or(crate::agentscope::DEFAULT_HOURS);
            let now = crate::now_rfc3339();
            let scope = crate::agentscope::renew(store, p, id, hours, &now)?;
            ser(&crate::agentscope::ScopeView::new(&scope, &now))
        }

        // ---- batch ----
        ("POST", ["projects", p, "batch"]) => {
            let ops = crate::batch::parse(b.get("operations"))
                .map_err(|e| CoreError::Unsupported(format!("invalid batch: {e}")))?;
            let dry_run = b.get("dry_run").and_then(Value::as_bool).unwrap_or(false);
            // Under an agent scope the write path holds every item where it is (FEAT-214).
            let hold = str_field(b, "hold_advance");
            let results = store.apply_batch_held(
                p,
                ops,
                dry_run,
                hold.as_deref(),
                &crate::origin::from_body(b),
            )?;
            Ok(json!({ "results": results }).to_string())
        }

        _ => Err(CoreError::NoSuchRoute(format!("{method} {path_only}"))),
    }
}

/// One scope of project `p`, by id (FEAT-214).
fn find_scope(store: &Store, p: &str, id: &str) -> Result<crate::agentscope::Scope> {
    crate::agentscope::load(store, p)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| CoreError::Unsupported(format!("no agent scope {id} in {p}")))
}

/// Whether a write request is a dry run (`"dry_run": true` in the body, honored by the batch
/// route): it changes nothing, so writers must not log activity or commit for it. (FEAT-042)
pub fn is_dry_run(body: Option<&Value>) -> bool {
    body.and_then(|b| b.get("dry_run"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Whether an HTTP method mutates state (and therefore should be committed in local mode).
pub fn is_mutation(method: &str) -> bool {
    !matches!(method.to_uppercase().as_str(), "GET" | "HEAD" | "OPTIONS")
}

/// A meaningful commit message for a data write, derived from the route (mirrors the server). For
/// a batch, an explicit `message` in the body wins.
/// The git drift findings for a project's doctor (FEAT-191): what `gitdrift::scan` finds, or one
/// warning saying why it could not look. Never a failed doctor.
fn git_drift(store: &Store, p: &str, repo: Option<&std::path::Path>) -> Vec<crate::doctor::Issue> {
    let warn = |message: String| crate::doctor::Issue {
        severity: crate::doctor::Severity::Warning,
        project: p.to_string(),
        code: None,
        message,
    };
    match repo {
        None => vec![warn(
            "git drift not checked: run from the project folder".to_string(),
        )],
        Some(repo) => crate::gitdrift::scan(store, p, repo)
            .unwrap_or_else(|e| vec![warn(format!("git drift not checked: {e}"))]),
    }
}

/// [`commit_message`], completed from the write's response where only the response knows: the id
/// of an agent scope being issued (FEAT-214).
pub fn commit_message_after(
    method: &str,
    path: &str,
    body: Option<&Value>,
    response: &str,
) -> String {
    let msg = commit_message(method, path, body);
    let bare = path.split('?').next().unwrap_or(path);
    let segs: Vec<&str> = bare.split('/').filter(|s| !s.is_empty()).collect();
    if method.eq_ignore_ascii_case("POST")
        && matches!(segs.as_slice(), ["projects", _, "decisions", "delegations"])
        && let Ok(v) = serde_json::from_str::<Value>(response)
        && let Some(id) = v["id"].as_str()
    {
        return format!("propose delegation {id}");
    }
    // FTR-033: the revision's id, and whether a pack was added or updated at which version.
    if method.eq_ignore_ascii_case("POST")
        && matches!(segs.as_slice(), ["projects", _, "baseline", "revisions"])
        && let Ok(v) = serde_json::from_str::<Value>(response)
        && let Some(id) = v["id"].as_str()
    {
        return format!("propose baseline revision {id}");
    }
    if method.eq_ignore_ascii_case("POST")
        && matches!(segs.as_slice(), ["projects", _, "baseline", "packs"])
        && let Ok(v) = serde_json::from_str::<Value>(response)
        && let Some(name) = v["pack"]["name"].as_str()
    {
        let verb = if v["updated"].as_bool() == Some(true) {
            "update"
        } else {
            "add"
        };
        return match v["pack"]["version"].as_u64() {
            Some(n) if n > 0 => format!("{verb} pack {name} v{n}"),
            _ => format!("{verb} pack {name}"),
        };
    }
    if method.eq_ignore_ascii_case("POST")
        && matches!(segs.as_slice(), ["projects", _, "scopes"])
        && let Ok(v) = serde_json::from_str::<Value>(response)
        && let Some(id) = v["scope"]["id"].as_str()
    {
        return msg.replacen(
            "issue agent scope (",
            &format!("issue agent scope {id} ("),
            1,
        );
    }
    msg
}

pub fn commit_message(method: &str, path: &str, body: Option<&Value>) -> String {
    let path_only = path.split('?').next().unwrap_or(path);
    let s: Vec<&str> = path_only.split('/').filter(|x| !x.is_empty()).collect();
    let del = method.eq_ignore_ascii_case("DELETE");
    match s.as_slice() {
        ["readme"] => "write the board README".to_string(),
        ["portfolio", "programs"] => body
            .and_then(|b| b.get("id").and_then(|v| v.as_str()))
            .map(|id| format!("add program {id}"))
            .unwrap_or_else(|| "update portfolio".into()),
        ["processes", name] => format!("save process {}", percent_decode(name)),
        ["profiles", name] => format!("save profile {}", percent_decode(name)),
        ["projects"] => "create project".into(),
        ["projects", p] if del => format!("delete project {p}"),
        ["projects", p] => format!("edit project {p}"),
        ["projects", _p, "charter"] => "update project charter".into(),
        ["projects", _p, "facts"] => "facts: set".into(),
        ["projects", _p, "catalog"] => "catalog: set".into(),
        ["projects", _p, "catalog", "init"] => "catalog: init".into(),
        ["projects", _p, "catalog", "baseline", "init"] => "system baseline: init".into(),
        ["projects", _p, "catalog", "baseline", "accept"] => "system baseline: accepted".into(),
        ["projects", _p, "catalog", "add"] => format!(
            "catalog: add {}",
            body.and_then(|b| str_field(b, "id"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        ),
        ["projects", _p, "catalog", "rows", id] => format!(
            "catalog: row {} {}",
            percent_decode(id),
            if body.and_then(|b| bool_field(b, "applies")) == Some(true) {
                "on"
            } else {
                "off"
            }
        ),
        ["projects", _p, "features", c, "definition"] => format!("define feature {c}"),
        ["projects", _p, "features", c, "defect"] => format!("record defect details for {c}"),
        ["projects", _p, "lessons"] => "record a lesson".into(),
        ["projects", _p, "adrs"] => "record an architecture decision".into(),
        ["projects", _p, "adrs", a, "supersede"] => format!("{a} supersedes an earlier decision"),
        ["projects", _p, "adrs", a, "accept"] => format!("accept decision {a}"),
        ["projects", _p, "adrs", a, "reject"] => format!("reject decision {a}"),
        ["projects", _p, "lessons", l, v] => format!("{v} lesson {l}"),
        ["projects", _p, "features", c, "split-from"] => format!("record what {c} was split from"),
        ["projects", _p, "mirror"] => "configure issue mirror".into(),
        ["projects", _p, "features"] => "add feature item".into(),
        ["projects", _p, "features", c] => format!("edit feature {c}"),
        ["projects", _p, "features", c, "move"] => format!("move feature {c}"),
        ["projects", _p, "features", c, "approve"] => format!("approve definition of {c}"),
        ["projects", _p, "features", c, "signoff", name, "withdraw"] => {
            let reason = body
                .and_then(|b| b.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            format!(
                "withdraw sign-off {} on {c}: {}",
                percent_decode(name),
                reason.trim()
            )
        }
        ["projects", _p, "features", c, "signoff", name] => {
            format!("sign off {} on {c}", percent_decode(name))
        }
        ["projects", _p, "features", c, "ratify"] => {
            format!("ratify {c} — built under a recorded bypass, agreed to after the fact")
        }
        ["projects", _p, "features", c, "unapprove"] => {
            format!("withdraw the approval of {c}")
        }
        ["projects", _p, "features", c, "tests", r, t] => {
            // Decoded, because these arrive percent-encoded (a test name is free text and holds
            // `::`, spaces and `/`). Without this, `git log` carried lines like
            // `test doctor%3A%3Atests%3A%3Aan_unreconciled_bypass...` — the history of a tool whose
            // point is a readable record, written in an encoding meant for a URL.
            format!(
                "test {} of {c}/{} is now {}",
                percent_decode(t),
                percent_decode(r),
                str_field(body.unwrap_or(&Value::Null), "state").unwrap_or_default()
            )
        }
        ["projects", _p, "features", c, "todos"] => format!("add todo-list to {c}"),
        ["projects", _p, "features", c, "todos", t, "tasks"] => format!("add task to {c}/{t}"),
        ["projects", _p, "features", c, "todos", t, "tasks", k]
            if method.eq_ignore_ascii_case("PATCH") =>
        {
            format!("edit task {k} ({c}/{t})")
        }
        ["projects", _p, "features", c, "todos", t, "tasks", k] => {
            format!("update task {k} ({c}/{t})")
        }
        ["projects", _p, "milestones"] => "add milestone".into(),
        ["projects", _p, "milestones", c] if del => format!("delete milestone {c}"),
        ["projects", _p, "milestones", c] => format!("edit milestone {c}"),
        ["projects", p, "config", "method"] => {
            let on = body
                .and_then(|b| b.get("on"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let parts: Vec<String> = body
                .and_then(|b| b.get("parts"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or_else(|| vec!["all".to_string()]);
            format!(
                "method {} {} for {p}",
                parts.join(", "),
                if on { "on" } else { "off" }
            )
        }
        ["projects", _p, "gate-results"] => {
            let f = |k: &str| body.and_then(|b| str_field(b, k)).unwrap_or_default();
            let passed = body.and_then(|b| bool_field(b, "passed")).unwrap_or(false);
            format!(
                "record gate {} {} on {}",
                f("name"),
                if passed { "passed" } else { "failed" },
                f("commit").chars().take(7).collect::<String>()
            )
        }
        ["projects", _p, "prepared"] => format!(
            "prepare a {} for the person to confirm",
            body.and_then(|b| str_field(b, "kind"))
                .unwrap_or_else(|| "record".into())
        ),
        ["projects", _p, "prepared", id, "reject"] => format!("reject prepared record {id}"),
        ["projects", _p, "planning"] => {
            let f = |k: &str| body.and_then(|b| str_field(b, k)).unwrap_or_default();
            let items = body
                .and_then(|b| vec_field(b, "items"))
                .unwrap_or_default()
                .join(", ");
            match f("action").as_str() {
                "out-of-sprint" => format!("planning decision: {items} out of their sprint"),
                _ => format!("planning decision: {items} into {}", f("target")),
            }
        }
        ["projects", _p, "planning", id, "decline"] => format!("decline planning decision {id}"),
        [
            "projects",
            _p,
            "prepared",
            "batch",
            batch,
            action @ ("confirm" | "reject"),
        ] => {
            format!("{action} prepared batch {batch}")
        }
        ["projects", _p, "decisions", "mode"] => format!(
            "set the decision mode to {}",
            body.and_then(|b| str_field(b, "mode")).unwrap_or_default()
        ),
        ["projects", _p, "decisions", "delegations"] => "propose a delegation".into(),
        ["projects", _p, "decisions", "defaults"] => "propose the default delegations".into(),
        ["projects", _p, "decisions", "delegations", id, "grant"] => {
            format!("grant delegation {id}")
        }
        ["projects", _p, "decisions", "delegations", id, "revoke"] => {
            format!("revoke delegation {id}")
        }
        ["projects", _p, "decisions", "reviews"] => format!(
            "{} delegated decision {}",
            if body.and_then(|b| str_field(b, "verdict")).as_deref() == Some("undo") {
                "undo"
            } else {
                "keep"
            },
            body.and_then(|b| str_field(b, "decision"))
                .unwrap_or_default()
        ),
        // FTR-033. Whether a pack was added or updated, and its version, are the response's: see
        // `commit_message_after`.
        ["projects", _p, "baseline", "packs"] => format!(
            "add pack {}",
            body.and_then(|b| str_field(b, "name"))
                .unwrap_or_default()
                .trim()
        ),
        ["projects", _p, "baseline", "packs", name] => {
            format!("remove pack {}", percent_decode(name))
        }
        ["projects", _p, "baseline", "revisions"] => "propose a baseline revision".into(),
        ["projects", _p, "baseline", "revisions", id, "accept"] => {
            format!("accept baseline revision {id}")
        }
        ["projects", _p, "baseline", "revisions", id, "withdraw"] => format!(
            "withdraw baseline revision {id}: {}",
            body.and_then(|b| str_field(b, "reason"))
                .unwrap_or_default()
                .trim()
        ),
        ["projects", _p, "config", "commit-gates"] => "set the commit gates".into(),
        ["projects", _p, "config", "tag-gate"] => "set the tag gate".into(),
        ["projects", _p, "config", "gate-docs"] => "set the documentation paths".into(),
        ["projects", _p, "releases", v, "patch-from-tag"] => format!(
            "start a patch release from the failed tag of {}",
            percent_decode(v)
        ),
        ["projects", _p, "config", what] => format!("update config: {what}"),
        ["projects", _p, "coverage"] => "config: coverage rules".into(),
        ["projects", _p, "sprints"] => "add a sprint".into(),
        ["projects", _p, "releases"] => "add a release".into(),
        // FEAT-183: the stage it went to, which a request for "the next one" does not name.
        ["projects", _p, "releases", v, "promote"] => format!(
            "promote release {} to {}",
            percent_decode(v),
            body.and_then(|b| str_field(b, "stage"))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "its next stage".into())
        ),
        ["projects", _p, "releases", v, "undo"] => {
            format!("undo the newest stage of release {}", percent_decode(v))
        }
        ["projects", _p, "releases", v] if del => format!("remove release {}", percent_decode(v)),
        ["projects", _p, "sprints", c] if del => format!("remove sprint {c}"),
        ["projects", _p, "sprints", c] => format!("edit sprint {c}"),
        ["projects", _p, "releases", v, "notes"] => {
            format!("release {}: notes edited", percent_decode(v))
        }
        ["projects", _p, "releases", v, "redate"] => {
            format!("release {}: dated today", percent_decode(v))
        }
        ["projects", _p, "releases", v, what] => format!("{what} release {}", percent_decode(v)),
        ["projects", _p, "sprints", c, what] => format!("{what} sprint {c}"),
        ["projects", _p, "docs", "folder"] => "configure doc folder".into(),
        ["projects", _p, "docs", "content"] if del => "remove document".into(),
        ["projects", _p, "docs", "content"] => "update document".into(),
        // FEAT-214. The id of a scope being issued is the response's: see `commit_message_after`.
        ["projects", _p, "scopes"] => {
            let f = |k: &str| {
                body.and_then(|b| str_field(b, k))
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            };
            format!("issue agent scope ({}, {})", f("role"), f("item"))
        }
        ["projects", _p, "scopes", id, "claim"] => format!("claim agent scope {id}"),
        ["projects", _p, "scopes", id, "revoke"] => format!(
            "revoke agent scope {id}: {}",
            body.and_then(|b| str_field(b, "reason"))
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| "revoked".into())
        ),
        ["projects", _p, "scopes", id, "renew"] => format!("renew agent scope {id}"),
        ["projects", _p, "git", "tidy"] => "tidy merged branches".into(),
        ["projects", _p, "batch"] => body
            .and_then(|b| b.get("message").and_then(|m| m.as_str()).map(String::from))
            .unwrap_or_else(|| "batch update".into()),
        _ => format!("{method} {path_only}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A board with releases on, for the release stage routes.
    fn release_board(tag: &str) -> (Store, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-dispatch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("shop", crate::ProjectConfig::default_for("shop"))
            .unwrap();
        dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"releases": true, "release_stages": ["candidate", "main", "production"]})),
        )
        .unwrap();
        (store, dir)
    }

    /// FTR-023 R-9: a sign-off sent with the hash of a spec that has since changed is refused, and
    /// nothing is recorded.
    #[test]
    fn a_signoff_on_a_design_that_moved_on_is_refused() {
        let (store, _dir) = release_board("signoff-moved-on");
        store
            .add_milestone("shop", "M", "", vec![], Some("MS-001".into()))
            .unwrap();
        let f = store
            .add_feature("shop", "Cart", "## Design\nUse a map.", "MS-001", None)
            .unwrap();
        store
            .set_feature_definition(
                "shop",
                &f.code,
                Some(crate::models::FeatureDefinition {
                    statement: "Keep a cart".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        let mut gates = std::collections::BTreeMap::new();
        gates.insert(
            "Scheduled".to_string(),
            crate::config::Gate {
                requires: vec![crate::readiness::Condition::Check(
                    crate::readiness::Check::SpecSections,
                )],
                signoffs: vec!["design-review".into()],
                ..Default::default()
            },
        );
        store.set_gates("shop", gates).unwrap();
        let shown = crate::readiness::spec_pin("## Design\nUse a map.").1;
        store
            .edit_feature(
                "shop",
                &f.code,
                None,
                Some("## Design\nUse a vec.".into()),
                None,
                None,
            )
            .unwrap();
        let path = format!("/projects/shop/features/{}/signoff/design-review", f.code);
        let err = dispatch(
            &store,
            "POST",
            &path,
            Some(&serde_json::json!({"by": "Ada", "spec_rev": shown})),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("spec changed after it was shown"), "{err}");
        let after = store.load("shop").unwrap();
        assert!(
            after
                .feature(&f.code)
                .unwrap()
                .definition
                .as_ref()
                .unwrap()
                .signoffs
                .is_empty()
        );
        // Shown the current text, it is recorded.
        let now = crate::readiness::spec_pin("## Design\nUse a vec.").1;
        dispatch(
            &store,
            "POST",
            &path,
            Some(&serde_json::json!({"by": "Ada", "spec_rev": now})),
        )
        .unwrap();
    }

    /// FTR-004 R-7: a sign-off given with the code repository records the position the item
    /// resolved to and, for each repository doc its spec lists under Affects docs, the blob id at
    /// that position; a listed doc missing there is left out. Given without the repository, both
    /// stay empty and the sign-off is recorded as before.
    #[test]
    fn a_signoff_given_with_the_repository_records_its_position_and_each_docs_blob() {
        use crate::coderepo::git;
        use crate::coderepo::tests::{commit_file, scratch_repo};
        let (store, dir) = release_board("signoff-located");
        assert!(
            std::fs::canonicalize(&dir)
                .unwrap()
                .starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap())
        );
        store
            .add_milestone("shop", "M", "", vec![], Some("MS-001".into()))
            .unwrap();
        let spec = "## Design\nRead x.\n\n### Affects docs\n\
                    - [docs/guide/a.md](docs/guide/a.md)\n- `docs/guide/missing.md`\n";
        let f = store
            .add_feature("shop", "Cart", spec, "MS-001", None)
            .unwrap();
        store
            .set_feature_definition(
                "shop",
                &f.code,
                Some(crate::models::FeatureDefinition {
                    statement: "Keep a cart".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        let repo = scratch_repo("dispatch-signoff-located");
        assert!(
            std::fs::canonicalize(&repo)
                .unwrap()
                .starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap())
        );
        // The item's branch holds the doc; main holds a different text, and is checked out.
        std::fs::create_dir_all(repo.join("docs/guide")).unwrap();
        commit_file(&repo, "docs/guide/a.md", "on main\n", "main doc");
        let branch = format!("feat/{}-cart", f.code);
        git(&repo, &["checkout", "-q", "-b", &branch, "main"]).unwrap();
        commit_file(&repo, "docs/guide/a.md", "on the branch\n", "branch doc");
        let tip = crate::coderepo::rev_parse(&repo, "HEAD").unwrap();
        let blob = git(&repo, &["rev-parse", &format!("{tip}:docs/guide/a.md")])
            .unwrap()
            .trim()
            .to_string();
        git(&repo, &["checkout", "-q", "main"]).unwrap();

        let signoffs = |store: &Store| {
            store
                .load("shop")
                .unwrap()
                .feature(&f.code)
                .unwrap()
                .definition
                .clone()
                .unwrap()
                .signoffs
        };
        // Without the repository: recorded, with neither field.
        dispatch(
            &store,
            "POST",
            &format!("/projects/shop/features/{}/signoff/design-review", f.code),
            Some(&serde_json::json!({"by": "Ada"})),
        )
        .unwrap();
        let first = signoffs(&store).pop().unwrap();
        assert_eq!(first.id, "design-review");
        assert!(first.position.is_empty(), "{first:?}");
        assert!(first.docs_read.is_empty(), "{first:?}");

        // With it: the branch tip, and the blob of the one listed doc there.
        dispatch_in(
            &store,
            "POST",
            &format!("/projects/shop/features/{}/signoff/code-review", f.code),
            Some(&serde_json::json!({"by": "Ada"})),
            Some(&repo),
        )
        .unwrap();
        let second = signoffs(&store).pop().unwrap();
        assert_eq!(second.id, "code-review");
        assert_eq!(
            second.position,
            crate::position::PositionRef::at_commit("shop", &tip).text()
        );
        assert_eq!(
            second.docs_read,
            [("docs/guide/a.md".to_string(), blob)].into(),
            "the missing doc is left out"
        );
    }

    /// FEAT-217: the codes routes show the next code per prefix and set the map; a feature POSTed
    /// with a kind takes its prefix.
    #[test]
    fn codes_routes_show_set_and_allocate() {
        let (store, dir) = release_board("codes-routes");
        assert!(
            std::fs::canonicalize(&dir)
                .unwrap()
                .starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap())
        );
        let call = |method: &str, path: &str, body: Value| {
            dispatch(&store, method, path, Some(&body))
                .map(|s| serde_json::from_str::<Value>(&s).unwrap())
        };
        store
            .add_milestone("shop", "M", "", vec![], Some("MS-001".into()))
            .unwrap();
        let view = call("GET", "/projects/shop/config/codes", Value::Null).unwrap();
        assert_eq!(view["default"], "FEAT");
        assert_eq!(view["next"], serde_json::json!({"FEAT": "FEAT-001"}));
        let view = call(
            "PUT",
            "/projects/shop/config/codes",
            serde_json::json!({"set": {"defect": "DEF", "feature": "FTR"}}),
        )
        .unwrap();
        assert_eq!(view["kinds"]["defect"], "DEF");
        let f = call(
            "POST",
            "/projects/shop/features",
            serde_json::json!({"title": "Crash", "milestone": "MS-001", "kind": "defect"}),
        )
        .unwrap();
        assert_eq!(f["code"], "DEF-001");
        assert_eq!(f["kind"], "defect");
        let view = call("GET", "/projects/shop/config/codes", Value::Null).unwrap();
        assert_eq!(view["next"]["DEF"], "DEF-002");
        assert_eq!(view["next"]["FTR"], "FTR-001");
        assert_eq!(view["in_use"], serde_json::json!({"DEF": 1}));
        let err = call(
            "PUT",
            "/projects/shop/config/codes",
            serde_json::json!({"set": {"spike": "MS"}}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("kind 'spike'"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-212 R-1: a stage's gate is declared through its route, starting from the one in force,
    /// read back by `release-stages`, and cleared to the default; `release-prepare` sets what
    /// `prepare` writes. Bad names and orders are refused, saying why.
    #[test]
    fn release_stage_routes_round_trip() {
        let (store, dir) = release_board("stage-routes");
        let call = |method: &str, path: &str, body: Value| {
            dispatch(&store, method, path, Some(&body))
                .map(|s| serde_json::from_str::<Value>(&s).unwrap())
        };
        let stages = call("GET", "/projects/shop/release-stages", Value::Null).unwrap();
        assert_eq!(stages[0]["on_enter"], serde_json::json!(["branch"]));
        assert_eq!(
            stages[2]["on_enter"],
            serde_json::json!(["move_main", "tag"])
        );
        assert_eq!(stages[2]["cut"], true);
        assert_eq!(stages[2]["declared"], false);

        call(
            "PUT",
            "/projects/shop/config/release-stages/candidate",
            serde_json::json!({"on_enter": ["branch", "prepare"]}),
        )
        .unwrap();
        call(
            "PUT",
            "/projects/shop/config/release-stages/main",
            serde_json::json!({"purpose": "main holds the release", "requires": ["items_finished"], "on_enter": ["move_main"]}),
        )
        .unwrap();
        // Only `requires` given: the actions in force (here, `tag` alone now) are kept.
        let stages = call(
            "PUT",
            "/projects/shop/config/release-stages/production",
            serde_json::json!({"requires": ["main_at_release", "pushed"]}),
        )
        .unwrap();
        assert_eq!(stages[1]["purpose"], "main holds the release");
        assert_eq!(stages[2]["on_enter"], serde_json::json!(["tag"]));
        assert_eq!(
            stages[2]["requires"],
            serde_json::json!(["main_at_release", "pushed"])
        );
        assert_eq!(stages[2]["declared"], true);
        let cadence = store.load_meta("shop").unwrap().config.cadence;
        assert_eq!(cadence.release_stages, ["candidate", "main", "production"]);
        assert_eq!(cadence.release_gates.len(), 3);

        let err = call(
            "PUT",
            "/projects/shop/config/release-stages/main",
            serde_json::json!({"requires": ["pushd"]}),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("'pushd' is not a check"), "{err}");
        let err = call(
            "PUT",
            "/projects/shop/config/release-stages/main",
            serde_json::json!({"on_enter": ["move_main", "tag"]}),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("'tag' is placed at both"), "{err}");
        let err = call(
            "PUT",
            "/projects/shop/config/release-stages/staging",
            serde_json::json!({"on_enter": ["branch"]}),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("'staging' is not a release stage"), "{err}");
        // Renaming a stage that has a gate is refused, naming the fix.
        let err = call(
            "PUT",
            "/projects/shop/config/cadence",
            serde_json::json!({"release_stages": ["candidate", "production"]}),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("kanbanr config release-stage main --clear"),
            "{err}"
        );

        let prep = call(
            "PUT",
            "/projects/shop/config/release-prepare",
            serde_json::json!({
                "version_files": [{"path": "api/Cargo.toml"}, {"path": "api/Cargo.lock", "match": "name = \"x\"\nversion = \"{version}\""}],
                "changelog": "CHANGELOG.md",
                "notes": "docs/guide/releases/{version}.md",
            }),
        )
        .unwrap();
        assert_eq!(
            prep["version_files"][1]["match"],
            "name = \"x\"\nversion = \"{version}\""
        );
        let prep = call(
            "PUT",
            "/projects/shop/config/release-prepare",
            serde_json::json!({"changelog": ""}),
        )
        .unwrap();
        assert!(prep.get("changelog").is_none(), "{prep}");
        assert_eq!(prep["notes"], "docs/guide/releases/{version}.md");
        assert!(
            call(
                "PUT",
                "/projects/shop/config/release-prepare",
                serde_json::json!({"notes": "../out.md"}),
            )
            .is_err()
        );

        // Clearing goes back to the default.
        let stages = call(
            "PUT",
            "/projects/shop/config/release-stages/production",
            serde_json::json!({"clear": true}),
        )
        .unwrap();
        assert_eq!(stages[2]["declared"], false);
        assert_eq!(
            stages[2]["on_enter"],
            serde_json::json!(["tag"]),
            "main still moves at main"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-212 R-15: a cadence write that does not name the gates or the preparation keeps them.
    /// FTR-033: the ten pack and baseline routes, each with its commit message: list, read and
    /// save a pack; follow one (a dry run first); the project's pack status; propose, accept (the
    /// person's) and withdraw a revision; the view; and removing the pack.
    #[test]
    fn baseline_routes_round_trip() {
        use serde_json::json;
        let (store, dir) = release_board("baseline-routes");
        crate::git::ensure_repo(&dir);
        let call = |m: &str, path: &str, body: Option<Value>| -> Value {
            let out = dispatch(&store, m, path, body.as_ref())
                .unwrap_or_else(|e| panic!("{m} {path}: {e}"));
            serde_json::from_str(&out).unwrap()
        };
        let msg = |m: &str, path: &str, body: &Value, response: &Value| {
            commit_message_after(m, path, Some(body), &response.to_string())
        };

        // The libraries: the built-in packs, then one saved to the board.
        let listed = call("GET", "/profiles", None);
        assert!(
            listed
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["name"] == "core" && r["library"] == "builtin"),
            "{listed}"
        );
        assert_eq!(call("GET", "/profiles/core", None)["library"], "builtin");
        let file =
            crate::profile::ProfileFile::from_yaml(crate::baseline::tests::STORE_PACK).unwrap();
        let save = json!({"file": file, "description": "The database"});
        call("PUT", "/profiles/store", Some(save.clone()));
        assert_eq!(
            commit_message("PUT", "/profiles/store", Some(&save)),
            "save profile store"
        );
        assert_eq!(call("GET", "/profiles/store", None)["library"], "board");

        // Following it: a dry run writes nothing, then the pack is added.
        let add = json!({"name": "store", "by": "Ada", "origin": "monitor"});
        let mut dry = add.clone();
        dry["dry_run"] = json!(true);
        assert_eq!(
            call("POST", "/projects/shop/baseline/packs", Some(dry))["dry_run"],
            true
        );
        assert!(
            crate::baseline::load(&store, "shop")
                .unwrap()
                .packs
                .is_empty()
        );
        let added = call("POST", "/projects/shop/baseline/packs", Some(add.clone()));
        assert_eq!(added["pack"]["name"], "store");
        assert_eq!(
            msg("POST", "/projects/shop/baseline/packs", &add, &added),
            "add pack store v1"
        );
        let again = call("POST", "/projects/shop/baseline/packs", Some(add.clone()));
        assert_eq!(
            msg("POST", "/projects/shop/baseline/packs", &add, &again),
            "update pack store v1"
        );
        let status = call("GET", "/projects/shop/profile", None);
        assert_eq!(status["packs"][0]["name"], "store");
        assert_eq!(status["packs"][0]["removed"], false);

        // A proposal, from the conversation; its id names the commit.
        let propose = json!({
            "answers": [{"area": "database", "options": ["postgresql"]}],
            "by": "Claude",
            "origin": "conversation",
        });
        let proposed = call(
            "POST",
            "/projects/shop/baseline/revisions",
            Some(propose.clone()),
        );
        assert_eq!(proposed["id"], "REV-001");
        assert_eq!(proposed["state"], "proposed");
        assert_eq!(
            msg(
                "POST",
                "/projects/shop/baseline/revisions",
                &propose,
                &proposed
            ),
            "propose baseline revision REV-001"
        );
        let rev = proposed["rev"].as_str().unwrap().to_string();

        // Accepting is the person's: refused from the conversation, taken from the monitor.
        let accept_path = "/projects/shop/baseline/revisions/REV-001/accept";
        let err = dispatch(
            &store,
            "POST",
            accept_path,
            Some(&json!({"rev": rev, "by": "Claude", "origin": "conversation"})),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("accepting a baseline revision"), "{err}");
        let accept = json!({"rev": rev, "by": "Ada", "origin": "monitor"});
        let accepted = call("POST", accept_path, Some(accept.clone()));
        assert_eq!(accepted["state"], "accepted");
        assert_eq!(
            commit_message("POST", accept_path, Some(&accept)),
            "accept baseline revision REV-001"
        );

        // The view shows the answered area.
        let view = call("GET", "/projects/shop/baseline", None);
        let area = view["areas"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "database")
            .cloned()
            .unwrap();
        assert_eq!(area["status"], "answered", "{area}");

        // A second proposal, withdrawn with its reason.
        let second = call(
            "POST",
            "/projects/shop/baseline/revisions",
            Some(json!({
                "answers": [{"area": "database", "options": ["sqlite"]}],
                "by": "Claude",
                "origin": "conversation",
            })),
        );
        assert_eq!(second["id"], "REV-002");
        let withdraw_path = "/projects/shop/baseline/revisions/REV-002/withdraw";
        let withdraw = json!({"reason": "not now", "by": "Ada", "origin": "monitor"});
        assert_eq!(
            call("POST", withdraw_path, Some(withdraw.clone()))["state"],
            "withdrawn"
        );
        assert_eq!(
            commit_message("POST", withdraw_path, Some(&withdraw)),
            "withdraw baseline revision REV-002: not now"
        );

        // Removing the pack keeps its answers.
        let remove = json!({"by": "Ada", "origin": "monitor"});
        let removed = call(
            "DELETE",
            "/projects/shop/baseline/packs/store",
            Some(remove.clone()),
        );
        assert!(removed["pack"]["removed"].is_object(), "{removed}");
        assert_eq!(removed["answers_kept"], 1);
        assert_eq!(
            commit_message(
                "DELETE",
                "/projects/shop/baseline/packs/store",
                Some(&remove)
            ),
            "remove pack store"
        );
        assert_eq!(
            call("GET", "/projects/shop/profile", None)["packs"][0]["removed"],
            true
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_gates_survive_cadence_put() {
        let (store, dir) = release_board("gates-survive");
        dispatch(
            &store,
            "PUT",
            "/projects/shop/config/release-stages/main",
            Some(&serde_json::json!({"on_enter": ["move_main"]})),
        )
        .unwrap();
        dispatch(
            &store,
            "PUT",
            "/projects/shop/config/release-prepare",
            Some(&serde_json::json!({"changelog": "CHANGELOG.md"})),
        )
        .unwrap();
        let before = store.load_meta("shop").unwrap().config.cadence;
        dispatch(
            &store,
            "PUT",
            "/projects/shop/config/cadence",
            Some(&serde_json::json!({"sprints": true, "main_branch": "trunk"})),
        )
        .unwrap();
        let after = store.load_meta("shop").unwrap().config.cadence;
        assert_eq!(after.release_gates, before.release_gates);
        assert_eq!(after.release_prepare, before.release_prepare);
        assert_eq!(after.main_branch.as_deref(), Some("trunk"));
        assert!(after.sprints);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-200 R-1 and R-2: with strict off (the default), a spec or markdown doc whose diagram
    /// fails the lint is written, and the response names each finding as `diagram_warnings` —
    /// file, block, file line, rule and message; a clean write carries no such key.
    #[test]
    fn write_reports_diagram_warnings() {
        let dir = std::env::temp_dir().join(format!("kanbanr-dispatch-dw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let call = |method: &str, path: &str, body: Value| -> Value {
            serde_json::from_str(&dispatch(&store, method, path, Some(&body)).unwrap()).unwrap()
        };
        let spec = "# Spec\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n```mermaid\n\
                    flowchart LR\n  A[f(x)] --> end\n```\n";

        let added = call(
            "POST",
            "/projects/demo/features",
            json!({ "title": "T", "milestone": "M", "specification": spec }),
        );
        assert_eq!(added["code"], "FEAT-001", "the item is written");
        let w = added["diagram_warnings"].as_array().expect("warnings");
        assert_eq!(w.len(), 1, "one finding per statement: {w:?}");
        assert_eq!(w[0]["file"], "FEAT-001 spec");
        assert_eq!(w[0]["block"], 2);
        assert_eq!(w[0]["line"], 10);
        assert_eq!(w[0]["rule"], "label_punctuation");
        assert!(w[0]["message"].as_str().unwrap().contains("quotes"));
        assert_eq!(store.feature_spec("demo", "FEAT-001").unwrap(), spec);

        let edited = call(
            "PATCH",
            "/projects/demo/features/FEAT-001",
            json!({ "specification": "```mermaid\nsequenceDiagram\n  A->>end: hi\n```\n" }),
        );
        assert_eq!(edited["diagram_warnings"][0]["rule"], "end_as_id");
        assert_eq!(edited["diagram_warnings"][0]["line"], 3);
        let titled = call(
            "PATCH",
            "/projects/demo/features/FEAT-001",
            json!({ "title": "Only the title" }),
        );
        assert!(titled.get("diagram_warnings").is_none(), "no spec, no lint");

        let doc = call(
            "PUT",
            "/projects/demo/docs/content",
            json!({ "path": "arch/flow.md", "content": "```mermaid\nflowchrt\n```\n" }),
        );
        assert_eq!(doc["path"], "arch/flow.md");
        assert_eq!(doc["diagram_warnings"][0]["file"], "docs/arch/flow.md");
        assert_eq!(doc["diagram_warnings"][0]["rule"], "unknown_type");
        assert!(
            store.read_doc("demo", "arch/flow.md").is_ok(),
            "the doc is written"
        );
        let clean = call(
            "PUT",
            "/projects/demo/docs/content",
            json!({ "path": "arch/ok.md", "content": "```mermaid\npie\n```\n" }),
        );
        assert!(clean.get("diagram_warnings").is_none());

        let batch = call(
            "POST",
            "/projects/demo/batch",
            json!({ "operations": [
                { "op": "feature.add", "ref": "x", "title": "B", "milestone": "M",
                  "spec": "```mermaid\n```\n" },
                { "op": "feature.edit", "code": "x", "spec": "```mermaid\nflowchart LR\n  A\n```\n" },
                { "op": "doc.write", "path": "b.md", "content": "```mermaid\nsequenceDiagram\n  junk\n```\n" },
            ]}),
        );
        let r = batch["results"].as_array().unwrap();
        assert_eq!(r[0]["diagram_warnings"][0]["rule"], "empty_block");
        assert_eq!(r[0]["diagram_warnings"][0]["file"], "FEAT-002 spec");
        assert!(r[1].get("diagram_warnings").is_none());
        assert_eq!(r[2]["diagram_warnings"][0]["rule"], "unknown_statement");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-214 R-10: the scopes route lists each scope of a project with its derived state —
    /// issued, active, expired, revoked — and the issue, revoke and renew routes change it.
    #[test]
    fn scopes_route_lists_state() {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-dispatch-scopes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let board = dir.join("board");
        let repo = dir.join("repo");
        std::fs::create_dir_all(&board).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        crate::coderepo::git(&repo, &["init", "-q", "-b", "main"]).unwrap();
        let store = Store::new(board.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let call = |method: &str, path: &str, body: Value| -> Value {
            let out = dispatch_in(&store, method, path, Some(&body), Some(&repo)).unwrap();
            serde_json::from_str(&out).unwrap_or(Value::String(out))
        };
        call(
            "POST",
            "/projects/demo/features",
            json!({ "title": "T", "milestone": "M" }),
        );
        let refused = dispatch_in(
            &store,
            "POST",
            "/projects/demo/scopes",
            Some(&json!({ "role": "design", "item": "FEAT-001", "by": "Ada" })),
            Some(&repo),
        )
        .unwrap_err();
        assert!(
            refused.to_string().contains("kanbanr git install-hooks"),
            "{refused}"
        );

        let issue = |note: &str| {
            call(
                "POST",
                "/projects/demo/scopes",
                json!({ "role": "design", "item": "FEAT-001", "by": "Ada", "note": note,
                        "hooks_ready": true, "claude_hooks_ready": true }),
            )
        };
        let first = issue("one");
        assert_eq!(first["scope"]["id"], "SC-001");
        assert_eq!(first["scope"]["state"], "issued");
        assert_eq!(first["scope"]["issued_by"], "Ada");
        assert!(
            first["brief"]
                .as_str()
                .unwrap()
                .contains("kanbanr scope claim SC-001")
        );
        assert_eq!(
            commit_message_after(
                "POST",
                "/projects/demo/scopes",
                Some(&json!({"role": "design", "item": "FEAT-001"})),
                &first.to_string()
            ),
            "issue agent scope SC-001 (design, FEAT-001)"
        );
        issue("two");
        issue("three");
        call(
            "POST",
            "/projects/demo/scopes/SC-002/revoke",
            json!({ "by": "Ada", "reason": "done" }),
        );
        // SC-003 ran out: its record says so, nothing rewrote it.
        let path = store
            .project_dir("demo")
            .join(crate::agentscope::SCOPES_FILE);
        let raw = std::fs::read_to_string(&path).unwrap();
        let mut scopes: Vec<crate::agentscope::Scope> = crate::yaml::from_str(&raw).unwrap();
        scopes[2].expires_at = "2020-01-01T00:00:00Z".into();
        std::fs::write(&path, crate::yaml::to_string(&scopes).unwrap()).unwrap();

        let listed = call("GET", "/projects/demo/scopes", Value::Null);
        let states: Vec<(&str, &str)> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["id"].as_str().unwrap(), s["state"].as_str().unwrap()))
            .collect();
        assert_eq!(
            states,
            [
                ("SC-001", "issued"),
                ("SC-002", "revoked"),
                ("SC-003", "expired")
            ]
        );
        assert_eq!(listed[1]["revoked_reason"], "done");
        assert_eq!(listed[0]["project"], "demo");

        let renewed = call(
            "POST",
            "/projects/demo/scopes/SC-003/renew",
            json!({ "hours": 4 }),
        );
        assert_eq!(renewed["state"], "issued", "renewed, it is in force again");
        let too_long = dispatch_in(
            &store,
            "POST",
            "/projects/demo/scopes/SC-003/renew",
            Some(&json!({ "hours": 100 })),
            Some(&repo),
        );
        assert!(too_long.is_err(), "72 hours at most");
        assert_eq!(
            call("GET", "/projects/demo/scopes/SC-001", Value::Null)["note"],
            "one"
        );
        let brief = call("GET", "/projects/demo/scopes/SC-001/brief", Value::Null);
        assert!(brief.as_str().unwrap().starts_with("## Your boundaries"));
        assert_eq!(
            read_media_type("/projects/demo/scopes/SC-001/brief"),
            "text/markdown; charset=utf-8"
        );
        assert_eq!(
            commit_message(
                "POST",
                "/projects/demo/scopes/SC-002/revoke",
                Some(&json!({"reason": "done"}))
            ),
            "revoke agent scope SC-002: done"
        );
        // The Workflow page's guard rail: the roles, and the scopes in force across items.
        let g = call("GET", "/projects/demo/guards", Value::Null);
        assert_eq!(g["agents"]["roles"][0]["role"], "design");
        assert_eq!(g["agents"]["roles"][0]["operations"], json!([]));
        assert_eq!(
            g["agents"]["roles"][1]["operations"],
            json!(["task.state", "test.state", "commit"])
        );
        let live: Vec<&str> = g["agents"]["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert_eq!(live, ["SC-001", "SC-003"], "issued and active only");
        assert!(g["agents"]["push_hook_installed"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-213 R-1: one read reports the method's parts, the commit rules, diagram strictness,
    /// each coverage rule with what it leaves uncovered, and the board's push policy.
    /// DEF-051: both tidy routes read or delete in the code repository, so without one they refuse.
    #[test]
    fn tidy_route_needs_a_repository() {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-dispatch-tidy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        for method in ["GET", "POST"] {
            let err = dispatch(&store, method, "/projects/demo/git/tidy", Some(&json!({})))
                .unwrap_err()
                .to_string();
            assert!(err.contains("code repository"), "{method}: {err}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guards_route_reports_every_guard_rail() {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-dispatch-guards-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let call = |method: &str, path: &str, body: Value| -> Value {
            serde_json::from_str(&dispatch(&store, method, path, Some(&body)).unwrap()).unwrap()
        };
        call(
            "PUT",
            "/projects/demo/config/cadence",
            json!({ "sprints": true, "releases": true, "git_model": true }),
        );
        call(
            "POST",
            "/projects/demo/sprints",
            json!({ "start": "2026-10-19", "length_days": 14 }),
        );
        call(
            "POST",
            "/projects/demo/releases",
            json!({ "version": "v1.0.0" }),
        );
        call(
            "PUT",
            "/projects/demo/config/diagrams",
            json!({ "strict": true }),
        );
        call(
            "POST",
            "/projects/demo/features",
            json!({ "title": "T", "milestone": "M" }),
        );
        call(
            "PUT",
            "/projects/demo/coverage",
            json!({ "text": "- id: listed\n  every: item\n  lists: architecture/index.md\n" }),
        );

        let g = call("GET", "/projects/demo/guards", Value::Null);

        let parts: Vec<&str> = g["method"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["part"].as_str().unwrap())
            .collect();
        assert_eq!(
            parts,
            [
                "design",
                "design-review",
                "docs",
                "docs-folded",
                "links",
                "system-baseline"
            ],
            "{g}"
        );
        assert!(
            g["method"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["on"].is_boolean())
        );

        let commits = &g["commits"];
        assert_eq!(commits["item_branch"], true);
        assert_eq!(commits["trailer"], "Refs");
        assert_eq!(commits["branch_pattern"], "feat/{code}-{slug}");
        assert!(commits["escape"].as_str().unwrap().contains("[no-ref]"));
        let protected: Vec<&str> = commits["protected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_str().unwrap())
            .collect();
        for branch in ["main", "sprint/*", "release/*"] {
            assert!(
                protected.contains(&branch),
                "{branch} is protected: {protected:?}"
            );
        }
        assert!(
            commits["hooks_installed"].is_null(),
            "core cannot tell whether the CLI's hooks are installed"
        );

        assert_eq!(g["diagrams"]["strict"], true);
        assert_eq!(g["diagrams"]["lint_on_write"], true);

        let coverage = g["coverage"].as_array().unwrap();
        assert_eq!(coverage.len(), 1, "{g}");
        assert_eq!(coverage[0]["id"], "listed");
        assert_eq!(coverage[0]["subjects"], 1, "{g}");
        assert_eq!(coverage[0]["uncovered"], 1, "the one item lists no doc");

        let (policy, _) = crate::git::PushPolicy::for_board(&dir);
        assert_eq!(g["push_policy"]["policy"], policy.to_string());
        assert!(
            ["environment", "board", "default"]
                .contains(&g["push_policy"]["source"].as_str().unwrap())
        );
        assert_eq!(g["code_pushes"], "you");

        // With the git model off, only the main branch refuses commits.
        call(
            "PUT",
            "/projects/demo/config/cadence",
            json!({ "git_model": false }),
        );
        let off = call("GET", "/projects/demo/guards", Value::Null);
        assert_eq!(off["commits"]["protected"], json!(["main"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-042 R-1: applying a process never turns the git model off.
    #[test]
    fn applying_a_process_keeps_the_git_model_on() {
        let dir = std::env::temp_dir().join(format!("kanbanr-dispatch-gm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        let call = |method: &str, path: &str, body: Value| -> Value {
            serde_json::from_str(&dispatch(&store, method, path, Some(&body)).unwrap()).unwrap()
        };
        let set = call(
            "PUT",
            "/projects/demo/config/cadence",
            json!({ "sprints": true }),
        );
        assert_eq!(set["cadence"]["git_model"], true, "the first choice");
        assert!(set["warnings"].to_string().contains("git model is on"));
        let applied = call(
            "PUT",
            "/projects/demo/config/workflow",
            json!({ "preset": "default" }),
        );
        assert_eq!(applied["cadence"]["git_model"], true);
        // A later choice of releases leaves a person's `off` alone.
        call(
            "PUT",
            "/projects/demo/config/cadence",
            json!({ "git_model": false }),
        );
        let more = call(
            "PUT",
            "/projects/demo/config/cadence",
            json!({ "releases": true }),
        );
        assert!(!more["cadence"]["git_model"].as_bool().unwrap_or(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-018 R-5: a batch with a field an operation does not take is refused whole, in a dry run
    /// as in a real one, and the operation before it is not written.
    #[test]
    fn a_batch_with_an_unknown_field_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("kanbanr-dispatch-bf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            std::fs::canonicalize(&dir)
                .unwrap()
                .starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap())
        );
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let before = store.load("demo").unwrap().features.len();
        for dry_run in [true, false] {
            let body = json!({
                "dry_run": dry_run,
                "operations": [
                    { "op": "feature.add", "title": "T", "milestone": "M" },
                    { "op": "feature.edit", "code": "FEAT-001", "sprint": "" },
                ],
            });
            let err = dispatch(&store, "POST", "/projects/demo/batch", Some(&body))
                .unwrap_err()
                .to_string();
            assert!(err.contains("operation 1 (feature.edit)"), "{err}");
            assert!(err.contains("unknown field `sprint`"), "{err}");
            assert_eq!(store.load("demo").unwrap().features.len(), before);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-041 R-2: a sprint stored active but dated ahead is listed as planned, and reading it
    /// rewrites nothing.
    #[test]
    fn the_sprint_list_reports_a_future_sprint_as_planned() {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-dispatch-def041-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        dispatch(
            &store,
            "PUT",
            "/projects/demo/config/cadence",
            Some(&serde_json::json!({"sprints": true})),
        )
        .unwrap();
        let file = store.project_dir("demo").join("sprints.yaml");
        std::fs::write(
            &file,
            "- code: SP-001\n  start: 2099-01-05\n  end: 2099-01-18\n  state: active\n",
        )
        .unwrap();
        let before = std::fs::read(&file).unwrap();
        let listed: serde_json::Value =
            serde_json::from_str(&dispatch(&store, "GET", "/projects/demo/sprints", None).unwrap())
                .unwrap();
        assert_eq!(listed[0]["state"], "planned", "{listed}");
        assert_eq!(std::fs::read(&file).unwrap(), before);
        let _ = std::fs::remove_dir_all(dir);
    }
}
