//! Architecture coverage rules (FEAT-197): the cross-cutting views stay whole.
//!
//! The fold-in check (`docs_updated`, FEAT-180) covers the docs an item names. The views that span
//! every item — a Zachman matrix, an ADR index, a component catalog — are rarely named, so nothing
//! noticed when they fell behind. A project names them here, as rules: every subject of one kind
//! (items in a stage range, accepted ADRs, docs under a folder) is referenced from a source doc, or
//! any doc under a source folder, or (for items) lists a named doc in `affects_docs`.
//!
//! References are what [`crate::refs::find`] finds: live links, relative links and board-shaped
//! tokens. The rules are judged in three places: after a write, as `coverage_warnings` on its
//! response ([`annotate`]); at a gate that asks for `docs_cover`, as if the item were already in
//! the target stage; and in `doctor`, one issue per uncovered rule.
//!
//! With no rules, nothing here runs: no docs are read, no fields are added, no gaps appear.

use crate::config::ProjectConfig;
use crate::models::FeatureItem;
use crate::readiness::Level;
use crate::refs::{self, BoardIndex, Ref};
use crate::{CoreError, Project, Result, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// What a rule's subjects are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Every {
    /// Items in a stage range, optionally of some kinds.
    #[default]
    Item,
    /// Accepted ADRs (superseded and rejected ones are not subjects).
    Adr,
    /// Docs under a folder.
    Doc,
}

impl Every {
    fn name(self) -> &'static str {
        match self {
            Every::Item => "item",
            Every::Adr => "adr",
            Every::Doc => "doc",
        }
    }
}

/// One coverage rule, as a project's config holds it under `coverage:`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CoverageRule {
    /// `^[a-z0-9][a-z0-9-]*$`, unique in the project; what `{docs_cover: [id]}` names.
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// What the subjects are; absent, items.
    #[serde(default)]
    pub every: Every,
    /// Items only: this stage or later (a status, or `"@<role>"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Items only: exactly these stages (excludes `from`).
    #[serde(default, rename = "in", skip_serializing_if = "Vec::is_empty")]
    pub in_stages: Vec<String>,
    /// Items only: only these kinds (an item with no kind is a `feature`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// Docs only, required: the folder whose docs are the subjects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under: Option<String>,
    /// The doc, or a folder ending `/` (any doc under it), a subject must be referenced from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referenced_from: Option<String>,
    /// Items only, instead of `referenced_from`: the doc every subject lists in `affects_docs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lists: Option<String>,
    /// Subjects grandfathered out: item codes, ADR ids or doc paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub except: Vec<String>,
    /// Keys a newer kanbanr wrote here, kept and written back unchanged (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: crate::models::Extra,
}

impl CoverageRule {
    /// The doc or folder this rule judges against, as written in messages.
    pub fn source(&self) -> String {
        self.referenced_from
            .as_deref()
            .or(self.lists.as_deref())
            .unwrap_or_default()
            .to_string()
    }

    /// The rule in words, for doctor: "every item at Design or later is referenced from …".
    pub fn describe(&self) -> String {
        let subject = match self.every {
            Every::Item => {
                let kinds = if self.kinds.is_empty() {
                    "item".to_string()
                } else {
                    format!("{} item", self.kinds.join("/"))
                };
                match (&self.from, self.in_stages.is_empty()) {
                    (_, false) => format!("{kinds} in {}", self.in_stages.join(", ")),
                    (Some(f), true) => format!("{kinds} at {f} or later"),
                    (None, true) => kinds,
                }
            }
            Every::Adr => "accepted ADR".to_string(),
            Every::Doc => format!("doc under {}", self.under.as_deref().unwrap_or_default()),
        };
        match &self.lists {
            Some(doc) => format!("every {subject} lists {doc} in affects_docs"),
            None => format!("every {subject} is referenced from {}", self.source()),
        }
    }

    /// The rule with its paths as the board stores them (FEAT-197): a doc through
    /// [`refs::doc_key`], a folder keeping its trailing `/`; `under` is always a folder.
    pub fn normalized(&self) -> CoverageRule {
        let mut r = self.clone();
        r.referenced_from = r.referenced_from.as_deref().map(norm_path);
        r.lists = r.lists.as_deref().map(refs::doc_key);
        r.under = r.under.as_deref().map(|u| {
            let u = u.trim();
            if u.ends_with('/') || u.is_empty() {
                norm_path(u)
            } else {
                norm_path(&format!("{u}/"))
            }
        });
        r
    }
}

/// A doc path, or a folder ending `/`, as the board stores it.
pub fn norm_path(p: &str) -> String {
    let t = p.trim().replace('\\', "/");
    if t.ends_with('/') {
        let s = t.trim_start_matches("./").trim_start_matches('/');
        s.strip_prefix("docs/").unwrap_or(s).to_string()
    } else {
        refs::doc_key(&t)
    }
}

/// A path that could name a file outside the doc tree: empty, absolute, or with `..`.
pub fn bad_path(p: &str) -> bool {
    let t = p.trim().replace('\\', "/");
    t.is_empty()
        || t.starts_with('/')
        || std::path::Path::new(&t).is_absolute()
        || t.split('/').any(|seg| seg == "..")
        || t.chars().nth(1) == Some(':')
}

/// Coverage rules from YAML (or JSON) text: a bare list, or a mapping with a `coverage:` key. An
/// empty text is no rules. Parsed in core, where board YAML is read.
pub fn parse_rules(text: &str) -> Result<Vec<CoverageRule>> {
    let invalid = |e: String| CoreError::Unsupported(format!("invalid coverage rules: {e}"));
    let value: Value = if text.trim().is_empty() {
        Value::Null
    } else {
        crate::yaml::parse(text).map_err(|e| invalid(e.to_string()))?
    };
    let list = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(_) => value,
        Value::Object(mut map) => match map.remove("coverage") {
            Some(v) if v.is_null() => return Ok(Vec::new()),
            Some(v) => v,
            None => {
                return Err(invalid(
                    "expected a list of rules, or a mapping with a `coverage:` key".into(),
                ));
            }
        },
        _ => return Err(invalid("expected a list of rules".into())),
    };
    serde_json::from_value(list).map_err(|e| invalid(e.to_string()))
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// What is wrong with one rule, in its own words (no id prefix).
fn problems_of(rule: &CoverageRule, config: &ProjectConfig) -> Vec<String> {
    let mut out = Vec::new();
    if !valid_id(&rule.id) {
        out.push("the id must be lower-case letters, digits and hyphens".to_string());
    }
    let item = rule.every == Every::Item;
    if rule.from.is_some() && !rule.in_stages.is_empty() {
        out.push("`from` and `in` cannot both be set".to_string());
    }
    if !item {
        for (key, set) in [
            ("from", rule.from.is_some()),
            ("in", !rule.in_stages.is_empty()),
            ("kinds", !rule.kinds.is_empty()),
            ("lists", rule.lists.is_some()),
        ] {
            if set {
                out.push(format!(
                    "`{key}` is for item rules, and this is a rule on every {}",
                    rule.every.name()
                ));
            }
        }
    }
    match (rule.every, &rule.under) {
        (Every::Doc, None) => out.push("a doc rule needs `under`: the folder".to_string()),
        (Every::Doc, Some(_)) | (_, None) => {}
        (_, Some(_)) => out.push("`under` is for doc rules".to_string()),
    }
    match (&rule.referenced_from, &rule.lists) {
        (Some(_), None) | (None, Some(_)) => {}
        _ => out.push("set exactly one of `referenced_from` and `lists`".to_string()),
    }
    if item {
        for stage in rule.from.iter().chain(&rule.in_stages) {
            if config.resolve_stage(stage).is_none() {
                out.push(
                    if stage.trim().starts_with(crate::config::ROLE_KEY_PREFIX) {
                        format!("'{stage}' names a role no status of the workflow holds")
                    } else {
                        format!("'{stage}' is not a status of the workflow")
                    },
                );
            }
        }
    }
    for path in [&rule.referenced_from, &rule.lists, &rule.under]
        .into_iter()
        .flatten()
    {
        if bad_path(path) && !(rule.under.as_ref() == Some(path) && path.trim().is_empty()) {
            out.push(format!(
                "'{path}' is not a board doc path (relative, under docs/, no `..`)"
            ));
        }
    }
    out
}

/// Every problem with a rule set, each naming its rule: an empty, malformed or repeated id; `from`
/// with `in`; an item-only key on another kind; `under` missing or misplaced; not exactly one of
/// `referenced_from` and `lists`; a stage that does not resolve; a path outside the doc tree.
pub fn rule_problems(rules: &[CoverageRule], config: &ProjectConfig) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for rule in rules {
        let name = if rule.id.trim().is_empty() {
            "(no id)".to_string()
        } else {
            rule.id.clone()
        };
        if !rule.id.trim().is_empty() && !seen.insert(rule.id.clone()) {
            out.push(format!("coverage rule '{name}': the id is used twice"));
        }
        for p in problems_of(rule, config) {
            out.push(format!("coverage rule '{name}': {p}"));
        }
    }
    out
}

/// A subject a source doc can reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Item(String),
    Adr(String),
    Doc(String),
}

/// What one rule judged one subject: uncovered, or the rule could not be judged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub rule: String,
    pub subject: Subject,
    pub level: Level,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    pub kind: Every,
    pub id: String,
}

/// One rule, and the subjects it leaves uncovered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleReport {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub every: Every,
    pub source: String,
    pub subjects: usize,
    pub uncovered: Vec<Subject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// Every rule of a project, judged: what `GET /projects/{p}/coverage` answers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CoverageReport {
    pub rules: Vec<RuleReport>,
}

/// What the rules' source docs reference, read once (FEAT-197). The only I/O coverage does: each
/// distinct source doc is read once, and no other doc is read.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub config: ProjectConfig,
    /// The project's rules, paths normalised; invalid ones too, so a report can name them.
    pub rules: Vec<CoverageRule>,
    /// Rule id → the statuses its stage range takes in (item rules).
    pub stages: BTreeMap<String, BTreeSet<String>>,
    /// Source doc path → the subjects it references.
    pub sources: BTreeMap<String, BTreeSet<Key>>,
    /// Rule id → its source doc paths.
    pub rule_sources: BTreeMap<String, Vec<String>>,
    pub docs: BTreeSet<String>,
    /// (id, doc path, accepted).
    pub adrs: Vec<(String, String, bool)>,
    /// Rule id → the problem that stopped it; `"*"` when nothing could be judged.
    pub problems: BTreeMap<String, String>,
    pub docs_read: usize,
}

/// `dir/of/source.md` + `../x.md` → `dir/x.md`.
fn resolve_relative(source: &str, path: &str) -> String {
    let mut parts: Vec<&str> = source.split('/').collect();
    parts.pop();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn is_exempt(f: &FeatureItem) -> bool {
    f.definition
        .as_ref()
        .is_some_and(|d| !d.exempt.trim().is_empty())
}

impl Snapshot {
    /// A snapshot that judges nothing, because it could not be built: every gate asking for
    /// `docs_cover` reports why.
    pub fn failed(config: &ProjectConfig, why: String) -> Snapshot {
        let mut s = Snapshot::empty(config);
        s.problems.insert("*".into(), why);
        s
    }

    fn empty(config: &ProjectConfig) -> Snapshot {
        Snapshot {
            config: config.clone(),
            rules: Vec::new(),
            stages: BTreeMap::new(),
            sources: BTreeMap::new(),
            rule_sources: BTreeMap::new(),
            docs: BTreeSet::new(),
            adrs: Vec::new(),
            problems: BTreeMap::new(),
            docs_read: 0,
        }
    }

    /// Build the snapshot of a loaded project: its rules' sources read once each, their references
    /// found. A project with no rules reads nothing.
    pub fn load(store: &Store, project: &Project) -> Result<Snapshot> {
        let config = &project.config;
        let mut snap = Snapshot::empty(config);
        if config.coverage.is_empty() {
            return Ok(snap);
        }
        snap.rules = config.coverage.iter().map(|r| r.normalized()).collect();
        let index = BoardIndex::of(store, project)?;
        snap.docs = index.docs.clone();
        if snap.rules.iter().any(|r| r.every == Every::Adr) {
            snap.adrs = crate::adr::list(store, &project.id)?
                .into_iter()
                .map(|a| {
                    let accepted = a.status.trim().eq_ignore_ascii_case("accepted");
                    (a.id, a.path, accepted)
                })
                .collect();
        }
        let mut seen = BTreeSet::new();
        for rule in &snap.rules {
            let first = !seen.contains(&rule.id);
            seen.insert(rule.id.clone());
            if let Some(p) = problems_of(rule, config).into_iter().next() {
                snap.problems
                    .entry(rule.id.clone())
                    .or_insert(format!("[{}] {p}", rule.id));
                continue;
            }
            if !first {
                snap.problems
                    .entry(rule.id.clone())
                    .or_insert(format!("[{}] the id is used twice", rule.id));
                continue;
            }
            if rule.every == Every::Item
                && let Some(set) = config.stage_set(rule.from.as_deref(), &rule.in_stages)
            {
                snap.stages.insert(rule.id.clone(), set);
            }
            let Some(from) = rule.referenced_from.as_deref() else {
                continue;
            };
            let sources: Vec<String> = if from.ends_with('/') || from.is_empty() {
                index
                    .docs
                    .iter()
                    .filter(|d| d.starts_with(from))
                    .cloned()
                    .collect()
            } else if index.docs.contains(from) {
                vec![from.to_string()]
            } else {
                Vec::new()
            };
            if sources.is_empty() {
                snap.problems.insert(
                    rule.id.clone(),
                    format!("[{}] {from} is not on the board", rule.id),
                );
                continue;
            }
            snap.rule_sources.insert(rule.id.clone(), sources);
        }
        let adr_of_path: BTreeMap<&String, &String> =
            index.adrs.iter().map(|(id, path)| (path, id)).collect();
        let distinct: BTreeSet<String> = snap.rule_sources.values().flatten().cloned().collect();
        for source in distinct {
            let text = match store.read_doc(&project.id, &source) {
                Ok(t) => t,
                Err(e) => {
                    for (id, srcs) in &snap.rule_sources {
                        if srcs.contains(&source) {
                            snap.problems.insert(
                                id.clone(),
                                format!("[{id}] {source} could not be read: {e}"),
                            );
                        }
                    }
                    continue;
                }
            };
            snap.docs_read += 1;
            let mut keys = BTreeSet::new();
            for found in refs::find(&text, &index) {
                match found.reference {
                    Ref::Item { code, .. } => {
                        keys.insert(Key::Item(code));
                    }
                    Ref::Adr { id } => {
                        keys.insert(Key::Adr(id));
                    }
                    Ref::Doc { path } => {
                        let path = if index.docs.contains(&path) {
                            path
                        } else {
                            let rel = resolve_relative(&source, &path);
                            if index.docs.contains(&rel) { rel } else { path }
                        };
                        if let Some(id) = adr_of_path.get(&path) {
                            keys.insert(Key::Adr((*id).clone()));
                        }
                        keys.insert(Key::Doc(path));
                    }
                    Ref::Milestone { .. } | Ref::Goal { .. } => {}
                }
            }
            snap.sources.insert(source, keys);
        }
        for (id, _) in snap.problems.iter() {
            snap.rule_sources.remove(id);
        }
        Ok(snap)
    }

    /// Whether one subject is covered by one rule. A `referenced_from` rule: some source of the
    /// rule other than the subject's own file references it. A `lists` rule: the item lists the doc
    /// in `affects_docs`, or will once it enters `entering` (the stage's default docs).
    pub fn covered(
        &self,
        rule: &CoverageRule,
        subject: &Key,
        item: Option<&FeatureItem>,
        entering: Option<&str>,
        config: &ProjectConfig,
    ) -> bool {
        if let Some(list) = &rule.lists {
            let want = refs::doc_key(list);
            let Some(def) = item.and_then(|f| f.definition.as_ref()) else {
                return false;
            };
            return def.affects_docs.iter().any(|a| refs::doc_key(a) == want)
                || entering.is_some_and(|s| config.default_affects_docs(s).contains(&want));
        }
        let own = match subject {
            Key::Doc(p) => Some(p.clone()),
            Key::Adr(id) => self.adrs.iter().find(|a| a.0 == *id).map(|a| a.1.clone()),
            Key::Item(_) => None,
        };
        self.rule_sources
            .get(&rule.id)
            .into_iter()
            .flatten()
            .filter(|s| Some(*s) != own.as_ref())
            .any(|s| {
                self.sources
                    .get(s)
                    .is_some_and(|keys| keys.contains(subject))
            })
    }

    /// Is `f`, at `status`, a subject of item rule `rule`?
    fn item_subject(&self, rule: &CoverageRule, f: &FeatureItem, status: &str) -> bool {
        if rule.every != Every::Item || is_exempt(f) || rule.except.contains(&f.code) {
            return false;
        }
        let kind = f
            .kind
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .unwrap_or("feature");
        if !rule.kinds.is_empty() && !rule.kinds.iter().any(|k| k.eq_ignore_ascii_case(kind)) {
            return false;
        }
        self.stages
            .get(&rule.id)
            .is_some_and(|s| s.contains(status))
    }

    fn finding(&self, rule: &CoverageRule, kind: Every, id: &str) -> Finding {
        let message = match &rule.lists {
            Some(doc) => format!("[{}] {id} does not list {doc} in affects_docs", rule.id),
            None => format!(
                "[{}] {id} is not referenced from {} — add a link to it there",
                rule.id,
                rule.source()
            ),
        };
        Finding {
            rule: rule.id.clone(),
            subject: Subject {
                kind,
                id: id.to_string(),
            },
            level: Level::Warning,
            message,
        }
    }

    fn problem_finding(&self, rule: &str, kind: Every, id: &str, problem: &str) -> Finding {
        Finding {
            rule: rule.to_string(),
            subject: Subject {
                kind,
                id: id.to_string(),
            },
            level: Level::Error,
            message: problem.to_string(),
        }
    }

    /// The item rules `f` fails as a subject at `at_status`, as a move into it judges them: the
    /// stage's default docs count as listed. `only` narrows to those rule ids; a narrowed rule that
    /// cannot be judged is an Error finding, an unnarrowed one is skipped (doctor names it).
    pub fn item_findings(
        &self,
        config: &ProjectConfig,
        f: &FeatureItem,
        at_status: &str,
        only: Option<&[String]>,
    ) -> Vec<Finding> {
        self.item_findings_at(config, f, at_status, Some(at_status), only)
    }

    fn item_findings_at(
        &self,
        config: &ProjectConfig,
        f: &FeatureItem,
        at_status: &str,
        entering: Option<&str>,
        only: Option<&[String]>,
    ) -> Vec<Finding> {
        if let Some(why) = self.problems.get("*") {
            return vec![self.problem_finding(
                "*",
                Every::Item,
                &f.code,
                &format!("coverage could not be judged: {why}"),
            )];
        }
        let mut out = Vec::new();
        for rule in self.rules.iter().filter(|r| r.every == Every::Item) {
            if only.is_some_and(|o| !o.contains(&rule.id)) {
                continue;
            }
            if let Some(problem) = self.problems.get(&rule.id) {
                if only.is_some() {
                    out.push(self.problem_finding(&rule.id, Every::Item, &f.code, problem));
                }
                continue;
            }
            if self.item_subject(rule, f, at_status)
                && !self.covered(rule, &Key::Item(f.code.clone()), Some(f), entering, config)
            {
                out.push(self.finding(rule, Every::Item, &f.code));
            }
        }
        out
    }

    /// Every rule, with its subjects counted and its uncovered ones listed.
    pub fn report(&self, project: &Project) -> CoverageReport {
        let config = &project.config;
        let mut rules = Vec::new();
        for rule in &self.rules {
            let mut r = RuleReport {
                id: rule.id.clone(),
                description: rule.description.clone(),
                every: rule.every,
                source: rule.source(),
                subjects: 0,
                uncovered: Vec::new(),
                problem: None,
            };
            if let Some(p) = self.problems.get("*").or(self.problems.get(&rule.id)) {
                r.problem = Some(p.clone());
                rules.push(r);
                continue;
            }
            let mut judge = |key: Key, item: Option<&FeatureItem>, id: &str| {
                r.subjects += 1;
                if !self.covered(rule, &key, item, None, config) {
                    r.uncovered.push(Subject {
                        kind: rule.every,
                        id: id.to_string(),
                    });
                }
            };
            match rule.every {
                Every::Item => {
                    for f in &project.features {
                        if self.item_subject(rule, f, &f.status) {
                            judge(Key::Item(f.code.clone()), Some(f), &f.code);
                        }
                    }
                }
                Every::Adr => {
                    for (id, _, accepted) in &self.adrs {
                        if *accepted && !rule.except.contains(id) {
                            judge(Key::Adr(id.clone()), None, id);
                        }
                    }
                }
                Every::Doc => {
                    let under = rule.under.clone().unwrap_or_default();
                    let single = rule
                        .referenced_from
                        .as_deref()
                        .filter(|s| !s.ends_with('/'));
                    for d in &self.docs {
                        if d.starts_with(&under)
                            && !rule.except.contains(d)
                            && Some(d.as_str()) != single
                        {
                            judge(Key::Doc(d.clone()), None, d);
                        }
                    }
                }
            }
            rules.push(r);
        }
        CoverageReport { rules }
    }

    /// The findings of a report: one per uncovered subject, one Error per rule that could not be
    /// judged.
    fn report_findings(&self, report: &CoverageReport, only: Option<&str>) -> Vec<Finding> {
        let mut out = Vec::new();
        for r in report
            .rules
            .iter()
            .filter(|r| only.is_none_or(|o| o == r.id))
        {
            let Some(rule) = self.rules.iter().find(|x| x.id == r.id) else {
                continue;
            };
            if let Some(p) = &r.problem {
                out.push(self.problem_finding(&r.id, r.every, &r.source, p));
                continue;
            }
            for s in &r.uncovered {
                out.push(self.finding(rule, s.kind, &s.id));
            }
        }
        out
    }

    /// Does rule `rule` take `path` as a source — by its configured path, whether or not the doc
    /// is on the board now (a deleted source is a problem worth saying)?
    fn sources_include(rule: &CoverageRule, path: &str) -> bool {
        match rule.referenced_from.as_deref() {
            Some(from) if from.ends_with('/') || from.is_empty() => path.starts_with(from),
            Some(from) => from == path,
            None => false,
        }
    }
}

/// What a write is about, for the coverage it may have changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    Item(String),
    Items(Vec<String>),
    Adr(String),
    Doc(String),
    All,
}

/// The coverage trigger of a write, from its route: which project, and what to judge. Pure.
pub fn trigger_of(
    method: &str,
    segs: &[&str],
    body: &Value,
    out: &Value,
) -> Option<(String, Trigger)> {
    let m = method.to_uppercase();
    let ["projects", p, rest @ ..] = segs else {
        return None;
    };
    let code_of = |v: &Value| v.get("code").and_then(Value::as_str).map(String::from);
    let trigger = match (m.as_str(), rest) {
        ("POST", ["features"]) => Trigger::Item(code_of(out)?),
        ("PATCH", ["features", c]) => Trigger::Item(code_of(out).unwrap_or_else(|| c.to_string())),
        ("PUT", ["features", c, "definition"]) | ("POST", ["features", c, "move"]) => {
            Trigger::Item(c.to_string())
        }
        ("POST", ["adrs", id, "accept"]) => Trigger::Adr(id.to_string()),
        ("PUT" | "DELETE", ["docs", "content"]) => {
            let path = out
                .get("path")
                .or_else(|| body.get("path"))
                .and_then(Value::as_str)?;
            let ext = std::path::Path::new(path).extension();
            if ext.is_some_and(|e| e != "md") {
                return None;
            }
            Trigger::Doc(refs::doc_key(path))
        }
        ("POST", ["sprints", _, "close"])
        | ("POST", ["releases", _, "cut"])
        | ("POST", ["releases", _, "promote"]) => Trigger::All,
        ("POST", ["batch"])
            if !body
                .get("dry_run")
                .and_then(Value::as_bool)
                .unwrap_or(false) =>
        {
            let codes: Vec<String> = out
                .get("results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(code_of)
                .collect();
            Trigger::Items(codes)
        }
        _ => return None,
    };
    Some((p.to_string(), trigger))
}

/// The most warnings a write reports; the rest are counted, and `doctor` lists them all.
pub const WARNINGS_CAP: usize = 100;

/// The coverage findings a trigger calls for, capped at [`WARNINGS_CAP`], with how many were
/// dropped. Nothing when the project has no rules or the snapshot cannot be built.
pub fn warnings_for(store: &Store, project: &Project, t: &Trigger) -> (Vec<Finding>, usize) {
    if project.config.coverage.is_empty() {
        return (Vec::new(), 0);
    }
    let Ok(snap) = Snapshot::load(store, project) else {
        return (Vec::new(), 0);
    };
    let config = &project.config;
    let item = |code: &str| -> Vec<Finding> {
        project
            .feature(code)
            .map(|f| snap.item_findings_at(config, f, &f.status, None, None))
            .unwrap_or_default()
    };
    let mut found = match t {
        Trigger::Item(code) => item(code),
        Trigger::Items(codes) => codes.iter().flat_map(|c| item(c)).collect(),
        Trigger::Adr(id) => {
            let report = snap.report(project);
            snap.report_findings(&report, None)
                .into_iter()
                .filter(|f| f.subject.kind == Every::Adr && f.subject.id == *id)
                .collect()
        }
        Trigger::Doc(path) => {
            let report = snap.report(project);
            let mut out = Vec::new();
            for rule in &snap.rules {
                if Snapshot::sources_include(rule, path) {
                    out.extend(snap.report_findings(&report, Some(&rule.id)));
                } else if rule.every == Every::Doc {
                    out.extend(
                        snap.report_findings(&report, Some(&rule.id))
                            .into_iter()
                            .filter(|f| f.subject.kind == Every::Doc && f.subject.id == *path),
                    );
                }
            }
            out
        }
        Trigger::All => snap.report_findings(&snap.report(project), None),
    };
    let mut seen = BTreeSet::new();
    found.retain(|f| seen.insert(f.message.clone()));
    let dropped = found.len().saturating_sub(WARNINGS_CAP);
    found.truncate(WARNINGS_CAP);
    (found, dropped)
}

/// A write's response with the coverage rules it now fails added as `coverage_warnings` (and
/// `coverage_more` when the cap dropped some). Cheap when the route triggers nothing or the
/// project has no rules. **Never fails**: the write is already done, so any failure here returns
/// `out` as it was.
pub fn annotate(store: &Store, method: &str, path: &str, body: &Value, out: String) -> String {
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        annotated(store, method, path, body, &out)
    }));
    match attempt {
        Ok(Some(annotated)) => annotated,
        _ => out,
    }
}

fn annotated(store: &Store, method: &str, path: &str, body: &Value, out: &str) -> Option<String> {
    let (path_only, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    let segs: Vec<&str> = path_only.split('/').filter(|s| !s.is_empty()).collect();
    // A delete answers with nothing; the doc it removed is named by the query.
    let parsed: Value = if out.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(out).ok()?
    };
    if !parsed.is_object() {
        return None;
    }
    let mut body = body.clone();
    if body.get("path").is_none()
        && let Some(rel) = crate::dispatch::query_param(query, "path")
    {
        body = serde_json::json!({ "path": rel });
    }
    let (project, trigger) = trigger_of(method, &segs, &body, &parsed)?;
    let project = store.load_meta(&project).ok()?;
    if project.config.coverage.is_empty() {
        return None;
    }
    let (mut findings, more) = warnings_for(store, &project, &trigger);
    // A move's gate already said these, under `gate_warnings`.
    let said: BTreeSet<&str> = parsed
        .get("gate_warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    findings.retain(|f| !said.contains(f.message.as_str()));
    if findings.is_empty() {
        return None;
    }
    let mut parsed = parsed;
    parsed["coverage_warnings"] = serde_json::to_value(&findings).ok()?;
    if more > 0 {
        parsed["coverage_more"] = more.into();
    }
    serde_json::to_string(&parsed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Gate;
    use crate::models::FeatureDefinition;

    fn temp(tag: &str) -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-coverage-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        (store, dir)
    }

    fn rules(yaml: &str) -> Vec<CoverageRule> {
        parse_rules(yaml).unwrap()
    }

    fn with_rules(store: &Store, yaml: &str) {
        store.set_coverage("demo", rules(yaml)).unwrap();
    }

    /// FEAT-197 R-1: rules read as a bare list or under `coverage:`, keep unknown keys, and an
    /// invalid set is refused naming the rule and the problem.
    #[test]
    fn rules_parse_and_validate() {
        let config = ProjectConfig::default_for("p");
        let text = "coverage:\n  - id: zachman\n    every: item\n    from: Scheduled\n    \
                    kinds: [feature]\n    referenced_from: architecture/zachman\n    \
                    except: [FEAT-001]\n    x_newer: 1\n  - id: adr-index\n    every: adr\n    \
                    referenced_from: architecture/\n  - id: catalog\n    every: doc\n    \
                    under: architecture/components\n    referenced_from: architecture/catalog.md\n  \
                    - id: listed\n    in: [Scheduled, No Action]\n    lists: architecture/zachman.md\n";
        let parsed = rules(text);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].every, Every::Item);
        assert_eq!(parsed[0].from.as_deref(), Some("Scheduled"));
        assert_eq!(parsed[0].except, vec!["FEAT-001"]);
        assert_eq!(parsed[0].extra["x_newer"], 1, "unknown keys are kept");
        assert_eq!(parsed[3].every, Every::Item, "no `every` is an item rule");
        assert_eq!(parsed[3].in_stages, vec!["Scheduled", "No Action"]);
        assert!(rule_problems(&parsed, &config).is_empty());
        let bare = rules("- id: a\n  every: adr\n  referenced_from: x.md\n");
        assert_eq!(bare.len(), 1);
        assert!(rules("").is_empty() && rules("coverage:\n").is_empty());
        assert!(parse_rules("just: words").is_err());
        assert!(parse_rules("- id: a\n  every: everything\n").is_err());
        // Normalised paths: a doc gets `.md`, a folder keeps its `/`, `under` is a folder.
        let n = parsed[0].normalized();
        assert_eq!(
            n.referenced_from.as_deref(),
            Some("architecture/zachman.md")
        );
        assert_eq!(
            parsed[2].normalized().under.as_deref(),
            Some("architecture/components/")
        );
        assert_eq!(
            parsed[1].normalized().referenced_from.as_deref(),
            Some("architecture/")
        );

        let bad = rules(concat!(
            "- id: Bad_Id\n  referenced_from: a.md\n",
            "- id: twice\n  referenced_from: a.md\n",
            "- id: twice\n  referenced_from: a.md\n",
            "- id: both\n  from: Planned\n  in: [Scheduled]\n  referenced_from: a.md\n",
            "- id: adr-from\n  every: adr\n  from: Planned\n  referenced_from: a.md\n",
            "- id: doc-no-under\n  every: doc\n  referenced_from: a.md\n",
            "- id: item-under\n  under: x/\n  referenced_from: a.md\n",
            "- id: neither\n",
            "- id: two-sources\n  referenced_from: a.md\n  lists: b.md\n",
            "- id: no-status\n  from: Nowhere\n  referenced_from: a.md\n",
            "- id: no-role\n  from: \"@design\"\n  referenced_from: a.md\n",
            "- id: escapes\n  referenced_from: ../outside.md\n",
            "- id: absolute\n  referenced_from: /etc/passwd\n",
        ));
        let problems = rule_problems(&bad, &config);
        let has = |id: &str, what: &str| {
            assert!(
                problems
                    .iter()
                    .any(|p| p.contains(&format!("coverage rule '{id}'")) && p.contains(what)),
                "no '{what}' for {id}: {problems:#?}"
            )
        };
        has("Bad_Id", "lower-case");
        has("twice", "used twice");
        has("both", "`from` and `in`");
        has("adr-from", "`from` is for item rules");
        has("doc-no-under", "needs `under`");
        has("item-under", "`under` is for doc rules");
        has("neither", "exactly one");
        has("two-sources", "exactly one");
        has("no-status", "'Nowhere' is not a status");
        has("no-role", "names a role no status");
        has("escapes", "not a board doc path");
        has("absolute", "not a board doc path");
    }

    /// FEAT-197 R-2: a subject counts as referenced only as the reference finder finds it — a live
    /// link, a relative link, a board-shaped token; a requirement reference covers its item; a
    /// reference in code does not count.
    #[test]
    fn references_count_links_tokens_and_relative_links() {
        let (store, dir) = temp("refs");
        for t in ["Linked", "Token", "Req", "InCode", "Unnamed"] {
            store.add_feature("demo", t, "", "M", None).unwrap();
        }
        store
            .set_feature_definition(
                "demo",
                "FEAT-003",
                Some(FeatureDefinition {
                    statement: "s".into(),
                    requirements: vec![crate::models::Requirement {
                        text: "THE SYSTEM SHALL work.".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            )
            .unwrap();
        store
            .write_doc("demo", "architecture/components/store.md", "# Store\n")
            .unwrap();
        store
            .write_doc("demo", "architecture/components/web.md", "# Web\n")
            .unwrap();
        store
            .write_doc("demo", "architecture/components/cli.md", "# CLI\n")
            .unwrap();
        store
            .write_doc(
                "demo",
                "architecture/zachman.md",
                "| [Linked](/p/demo/feature/FEAT-001) | FEAT-002 | FEAT-003/R-1 |\n\n\
                 `FEAT-004` in code\n\n```\nFEAT-005\n```\n",
            )
            .unwrap();
        store
            .write_doc(
                "demo",
                "architecture/catalog.md",
                "- [store](components/store.md)\n- [web](./components/web.md)\n",
            )
            .unwrap();
        with_rules(
            &store,
            "- id: zachman\n  referenced_from: architecture/zachman.md\n\
             - id: catalog\n  every: doc\n  under: architecture/components/\n  \
             referenced_from: architecture/catalog.md\n",
        );
        let project = store.load_meta("demo").unwrap();
        let snap = Snapshot::load(&store, &project).unwrap();
        let report = snap.report(&project);
        let uncovered = |id: &str| -> Vec<String> {
            report
                .rules
                .iter()
                .find(|r| r.id == id)
                .unwrap()
                .uncovered
                .iter()
                .map(|s| s.id.clone())
                .collect()
        };
        assert_eq!(uncovered("zachman"), vec!["FEAT-004", "FEAT-005"]);
        assert_eq!(
            uncovered("catalog"),
            vec!["architecture/components/cli.md"],
            "relative links resolve against the source's folder"
        );
        assert_eq!(report.rules[0].subjects, 5);
        assert_eq!(report.rules[1].subjects, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-197 R-3: "S or later" is every status at or after S in the workflow's order, no-ops
    /// excluded; S may be `@role`; `in` takes exact statuses, no-ops allowed.
    #[test]
    fn stage_order_skips_no_ops_and_resolves_roles() {
        let mut config = ProjectConfig::default_for("p");
        // Deferred, Planned, Scheduled, Completed, No Action, Not Applicable, Out-of-Scope.
        let set = |c: &ProjectConfig, from: Option<&str>, ins: &[&str]| {
            c.stage_set(from, &ins.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                .map(|s| s.into_iter().collect::<Vec<_>>())
        };
        assert_eq!(
            set(&config, Some("Planned"), &[]).unwrap(),
            vec!["Completed", "Planned", "Scheduled"]
        );
        assert_eq!(
            set(&config, None, &[]).unwrap(),
            vec!["Completed", "Deferred", "Planned", "Scheduled"]
        );
        assert_eq!(
            set(&config, None, &["No Action"]).unwrap(),
            vec!["No Action"]
        );
        assert_eq!(set(&config, Some("Nowhere"), &[]), None);
        assert_eq!(set(&config, Some("@design"), &[]), None, "no design role");
        config.gates.insert(
            "Scheduled".into(),
            Gate {
                roles: vec!["design".into()],
                ..Default::default()
            },
        );
        assert_eq!(
            set(&config, Some("@design"), &[]).unwrap(),
            vec!["Completed", "Scheduled"]
        );
        assert_eq!(set(&config, None, &["@design"]).unwrap(), vec!["Scheduled"]);
    }

    /// FEAT-197 R-4: with no rules nothing is read, nothing is added to a write, and the report
    /// is empty.
    #[test]
    fn no_rules_no_effect() {
        let (store, dir) = temp("none");
        store.add_feature("demo", "A", "", "M", None).unwrap();
        store
            .write_doc("demo", "architecture/zachman.md", "nothing")
            .unwrap();
        let project = store.load_meta("demo").unwrap();
        let snap = Snapshot::load(&store, &project).unwrap();
        assert_eq!(snap.docs_read, 0);
        assert!(snap.docs.is_empty(), "not even the doc list is read");
        assert!(snap.report(&project).rules.is_empty());
        let f = project.feature("FEAT-001").unwrap();
        assert!(
            snap.item_findings(&project.config, f, "Scheduled", None)
                .is_empty()
        );
        let out = r#"{"code":"FEAT-001","status":"Planned"}"#.to_string();
        assert_eq!(
            annotate(
                &store,
                "POST",
                "/projects/demo/features",
                &Value::Null,
                out.clone()
            ),
            out
        );
        for t in [Trigger::All, Trigger::Item("FEAT-001".into())] {
            assert!(warnings_for(&store, &project, &t).0.is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-197 R-15: the snapshot reads only the rules' source docs, once each, and builds in
    /// under 2 s in a debug build with 2,000 items and 200 source docs.
    #[test]
    fn snapshot_reads_only_source_docs_linearly() {
        let (store, dir) = temp("perf");
        let mut project = store.load_meta("demo").unwrap();
        let template = store.add_feature("demo", "T", "", "M", None).unwrap();
        project.features = (1..=2000)
            .map(|n| {
                let mut f = template.clone();
                f.code = format!("FEAT-{n:04}");
                f
            })
            .collect();
        for n in 0..200 {
            let mut text = String::new();
            for k in 0..10 {
                text.push_str(&format!(
                    "- [x](/p/demo/feature/FEAT-{:04}) and FEAT-{:04}\n",
                    n * 10 + k + 1,
                    (n * 10 + k + 1) % 2000 + 1
                ));
            }
            store
                .write_doc("demo", &format!("views/v{n:03}.md"), &text)
                .unwrap();
        }
        for n in 0..50 {
            store
                .write_doc("demo", &format!("other/o{n:03}.md"), "FEAT-0001")
                .unwrap();
        }
        project.config.coverage = rules(
            "- id: views\n  referenced_from: views/\n- id: again\n  every: doc\n  under: views/\n  \
             referenced_from: views/\n",
        );
        let start = std::time::Instant::now();
        let snap = Snapshot::load(&store, &project).unwrap();
        let report = snap.report(&project);
        let took = start.elapsed();
        assert_eq!(
            snap.docs_read, 200,
            "each distinct source doc once, no others"
        );
        assert_eq!(report.rules[0].subjects, 2000);
        assert!(
            report.rules[0].uncovered.is_empty(),
            "{:?}",
            report.rules[0]
        );
        assert!(took < std::time::Duration::from_secs(2), "took {took:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-197 R-13: whatever goes wrong judging coverage after a write, the write's own output
    /// comes back unchanged.
    #[test]
    fn annotate_never_fails_a_write() {
        let (store, dir) = temp("never");
        store.add_feature("demo", "A", "", "M", None).unwrap();
        store
            .write_doc("demo", "architecture/zachman.md", "nothing")
            .unwrap();
        with_rules(
            &store,
            "- id: zachman\n  referenced_from: architecture/zachman.md\n",
        );
        let ok = r#"{"code":"FEAT-001"}"#.to_string();
        let annotated = annotate(
            &store,
            "POST",
            "/projects/demo/features",
            &Value::Null,
            ok.clone(),
        );
        assert!(annotated.contains("coverage_warnings"), "{annotated}");
        // Not JSON, an unknown project, an unknown item, a non-object: the output as it was.
        for (path, out) in [
            ("/projects/demo/features", "not json".to_string()),
            ("/projects/nowhere/features", ok.clone()),
            (
                "/projects/demo/features",
                r#"{"code":"FEAT-999"}"#.to_string(),
            ),
            ("/projects/demo/features", "[1,2]".to_string()),
            (
                "/projects/demo/features/FEAT-001/move",
                r#"{"status":"x"}"#.to_string(),
            ),
        ] {
            let path_out = annotate(&store, "POST", path, &Value::Null, out.clone());
            if path.ends_with("move") {
                assert!(path_out.contains("coverage_warnings"));
            } else {
                assert_eq!(path_out, out, "{path}");
            }
        }
        // The config turned unreadable after the write: still the output as it was.
        std::fs::write(dir.join("projects/demo/config.yaml"), "statuses: [unclosed").unwrap();
        assert_eq!(
            annotate(
                &store,
                "POST",
                "/projects/demo/features",
                &Value::Null,
                ok.clone()
            ),
            ok
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The route map: which writes trigger what.
    #[test]
    fn triggers_follow_the_route() {
        let t = |m: &str, path: &str, body: Value, out: Value| {
            let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            trigger_of(m, &segs, &body, &out).map(|(_, t)| t)
        };
        let n = Value::Null;
        assert_eq!(
            t(
                "POST",
                "/projects/p/features",
                n.clone(),
                serde_json::json!({"code": "F-1"})
            ),
            Some(Trigger::Item("F-1".into()))
        );
        assert_eq!(
            t(
                "PATCH",
                "/projects/p/features/F-1",
                n.clone(),
                serde_json::json!({"code": "F-2"})
            ),
            Some(Trigger::Item("F-2".into()))
        );
        assert_eq!(
            t(
                "POST",
                "/projects/p/adrs/ADR-0001/accept",
                n.clone(),
                n.clone()
            ),
            Some(Trigger::Adr("ADR-0001".into()))
        );
        assert_eq!(
            t(
                "PUT",
                "/projects/p/docs/content",
                n.clone(),
                serde_json::json!({"path": "a/b"})
            ),
            Some(Trigger::Doc("a/b.md".into()))
        );
        assert_eq!(
            t(
                "PUT",
                "/projects/p/docs/content",
                n.clone(),
                serde_json::json!({"path": "a.png"})
            ),
            None
        );
        assert_eq!(
            t(
                "POST",
                "/projects/p/sprints/SP-1/close",
                n.clone(),
                n.clone()
            ),
            Some(Trigger::All)
        );
        for route in ["releases/v1/cut", "releases/v1/promote"] {
            assert_eq!(
                t(
                    "POST",
                    &format!("/projects/p/{route}"),
                    n.clone(),
                    n.clone()
                ),
                Some(Trigger::All)
            );
        }
        assert_eq!(
            t(
                "DELETE",
                "/projects/p/docs/content",
                serde_json::json!({"path": "a/b.md"}),
                serde_json::json!({})
            ),
            Some(Trigger::Doc("a/b.md".into()))
        );
        assert_eq!(
            t(
                "POST",
                "/projects/p/batch",
                serde_json::json!({"dry_run": false}),
                serde_json::json!({"results": [{"code": "F-1"}, {"path": "x.md"}]})
            ),
            Some(Trigger::Items(vec!["F-1".into()]))
        );
        assert_eq!(
            t(
                "POST",
                "/projects/p/batch",
                serde_json::json!({"dry_run": true}),
                n.clone()
            ),
            None
        );
        assert_eq!(
            t("POST", "/projects/p/features/F-1/approve", n.clone(), n),
            None
        );
    }
}
