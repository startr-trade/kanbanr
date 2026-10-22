//! References between board artifacts, found in markdown (FEAT-181).
//!
//! Specs, docs and ADRs name each other in plain text — `FEAT-012`, `ADR-0005`,
//! `architecture/data.md`. This finds those names so the monitor can link them, doctor can report
//! the ones that point at nothing, and trace can follow them. One finder, with a TypeScript port in
//! the monitor (`web/src/boardRefs.ts`) checked against the same fixture
//! (`web/scripts/refs-fixture.json`), so the two cannot disagree about what a reference is.
//!
//! A token is a reference only when its shape belongs to this board: an item or milestone code
//! with a prefix the board uses (one its codes configure, or one an existing item has — FEAT-217),
//! an ADR id, a goal id when the charter has goals, or a doc path. That keeps `UTF-8`, `ISO-9001`,
//! `SHA-256` and `RUSTSEC-2024-0449` out. Code — fenced, indented after a
//! blank line is not treated specially, inline spans — and HTML comments are skipped, and link
//! text is not scanned twice: a markdown link is read once, by its target.

use crate::{Result, Store};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Something on the board a reference points at.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ref {
    /// An item, optionally one of its requirements (`FEAT-012`, `FEAT-012/R-3`).
    Item {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requirement: Option<String>,
    },
    Milestone {
        code: String,
    },
    /// `ADR-0005`.
    Adr {
        id: String,
    },
    /// A charter goal, `G-2`.
    Goal {
        id: String,
    },
    /// A board doc, by its path under `docs/` (`architecture/data.md`).
    Doc {
        path: String,
    },
}

/// A reference found in a text: what it points at, the line it is on (1-based), and whether it
/// was written as a link rather than named in prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    pub reference: Ref,
    pub line: usize,
    pub explicit: bool,
}

/// What the board has, to resolve references against.
#[derive(Debug, Clone, Default)]
pub struct BoardIndex {
    pub project: String,
    pub item_prefixes: BTreeSet<String>,
    pub milestone_prefixes: BTreeSet<String>,
    pub items: BTreeMap<String, BTreeSet<String>>, // code -> requirement ids
    pub milestones: BTreeSet<String>,
    pub adrs: BTreeMap<String, String>, // id -> doc path
    pub goals: BTreeSet<String>,
    pub docs: BTreeSet<String>,
}

/// The letters before the number in a code: `FEAT` of `FEAT-012`, `MS` of `MS-004`.
fn prefix_of(code: &str) -> Option<&str> {
    let (p, n) = code.rsplit_once('-')?;
    (!p.is_empty() && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).then_some(p)
}

impl BoardIndex {
    /// Everything a project's references can point at.
    pub fn load(store: &Store, project: &str) -> Result<Self> {
        Self::of(store, &store.load(project)?)
    }

    /// The index of a project already loaded (its items' requirements come from it).
    pub fn of(store: &Store, p: &crate::Project) -> Result<Self> {
        let project = p.id.as_str();
        let mut index = BoardIndex {
            project: project.to_string(),
            // FEAT-217: a configured prefix is this board's before any item has it (FEAT when
            // nothing is configured, which a board with items already has).
            item_prefixes: p.config.codes.prefixes(),
            ..Default::default()
        };
        for f in &p.features {
            if let Some(prefix) = prefix_of(&f.code) {
                index.item_prefixes.insert(prefix.to_string());
            }
            let reqs = f
                .definition
                .as_ref()
                .map(|d| d.requirements.iter().map(|r| r.id.clone()).collect())
                .unwrap_or_default();
            index.items.insert(f.code.clone(), reqs);
        }
        for m in &p.milestones {
            if let Some(prefix) = prefix_of(&m.code) {
                index.milestone_prefixes.insert(prefix.to_string());
            }
            index.milestones.insert(m.code.clone());
        }
        index.docs = store.list_docs(project)?.into_iter().collect();
        for adr in crate::adr::list(store, project)? {
            index.adrs.insert(adr.id.clone(), adr.path.clone());
        }
        index.goals = crate::charter::load(store, project)?.goal_ids();
        Ok(index)
    }

    /// Is the reference something the board has? A requirement reference also needs the
    /// requirement.
    pub fn resolves(&self, r: &Ref) -> bool {
        match r {
            Ref::Item { code, requirement } => match (self.items.get(code), requirement) {
                (Some(reqs), Some(req)) => reqs.contains(req),
                (Some(_), None) => true,
                (None, _) => false,
            },
            Ref::Milestone { code } => self.milestones.contains(code),
            Ref::Adr { id } => self.adrs.contains_key(id),
            Ref::Goal { id } => self.goals.contains(id),
            Ref::Doc { path } => self.docs.contains(path),
        }
    }

    /// The monitor path a reference links to, when the board has it.
    pub fn href(&self, r: &Ref) -> Option<String> {
        if !self.resolves(r) {
            return None;
        }
        let p = &self.project;
        Some(match r {
            Ref::Item { code, requirement } => match requirement {
                Some(req) => format!("/p/{p}/feature/{code}#{req}"),
                None => format!("/p/{p}/feature/{code}"),
            },
            Ref::Milestone { code } => format!("/p/{p}/milestone/{code}"),
            Ref::Goal { id } => format!("/p/{p}/charter#{id}"),
            Ref::Adr { id } => format!("/p/{p}/docs/file/{}", self.adrs[id]),
            Ref::Doc { path } => format!("/p/{p}/docs/file/{path}"),
        })
    }

    /// Is a broken reference worth reporting? An explicit link always is; prose only when its shape
    /// is unmistakably this board's (a bare path that names no doc is just a path).
    pub fn is_broken(&self, found: &Found) -> bool {
        !self.resolves(&found.reference)
            && (found.explicit || !matches!(found.reference, Ref::Doc { .. }))
    }
}

/// A doc path written in prose or a relative link, as the board stores it: no leading `./`, `/` or
/// `docs/`, with `.md`.
pub fn doc_key(path: &str) -> String {
    let mut p = path.trim().trim_start_matches("./").trim_start_matches('/');
    p = p.strip_prefix("docs/").unwrap_or(p);
    if p.ends_with(".md") {
        p.to_string()
    } else {
        format!("{p}.md")
    }
}

/// Every reference in `markdown`, in order.
pub fn find(markdown: &str, index: &BoardIndex) -> Vec<Found> {
    let mut out = Vec::new();
    let mut in_fence: Option<&str> = None;
    let mut in_comment = false;
    for (n, raw) in markdown.lines().enumerate() {
        let line_no = n + 1;
        let trimmed = raw.trim_start();
        if let Some(fence) = in_fence {
            if trimmed.starts_with(fence) {
                in_fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = Some(&trimmed[..3]);
            continue;
        }
        let mut line = raw.to_string();
        // HTML comments, possibly spanning lines.
        if in_comment {
            match line.find("-->") {
                Some(end) => {
                    line = line[end + 3..].to_string();
                    in_comment = false;
                }
                None => continue,
            }
        }
        while let Some(start) = line.find("<!--") {
            match line[start..].find("-->") {
                Some(end) => line.replace_range(start..start + end + 3, " "),
                None => {
                    line.truncate(start);
                    in_comment = true;
                }
            }
        }
        scan_line(&line, line_no, index, &mut out);
    }
    out
}

/// One line, outside code fences and comments.
fn scan_line(line: &str, line_no: usize, index: &BoardIndex, out: &mut Vec<Found>) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut Vec<Found>| {
        scan_prose(text, line_no, index, out);
        text.clear();
    };
    while i < chars.len() {
        match chars[i] {
            // An inline code span: skipped whole.
            '`' => {
                flush(&mut text, out);
                let ticks = chars[i..].iter().take_while(|c| **c == '`').count();
                let rest: String = chars[i + ticks..].iter().collect();
                let close = "`".repeat(ticks);
                match rest.find(&close) {
                    Some(end) => i += ticks + rest[..end].chars().count() + ticks,
                    None => i = chars.len(),
                }
            }
            // A markdown link `[text](target)`: read by its target, its text not scanned.
            '[' => {
                let rest: String = chars[i..].iter().collect();
                if let Some((target, len)) = link_at(&rest) {
                    flush(&mut text, out);
                    if let Some(r) = target_ref(&target, index) {
                        out.push(Found {
                            reference: r,
                            line: line_no,
                            explicit: true,
                        });
                    }
                    i += len;
                } else {
                    text.push('[');
                    i += 1;
                }
            }
            c => {
                text.push(c);
                i += 1;
            }
        }
    }
    flush(&mut text, out);
}

/// `[text](target)` at the start of `s`: the target and the link's length in chars.
fn link_at(s: &str) -> Option<(String, usize)> {
    let close = s.find("](")?;
    if s[1..close].contains('[') && !s[1..close].contains(']') {
        return None;
    }
    let after = &s[close + 2..];
    let end = after.find(')')?;
    let target = after[..end]
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    Some((target, s[..close + 2 + end + 1].chars().count()))
}

/// What a link target points at on this board, if anything.
fn target_ref(target: &str, index: &BoardIndex) -> Option<Ref> {
    if target.contains("://") || target.starts_with("mailto:") || target.starts_with('#') {
        return None;
    }
    let prefix = format!("/p/{}/", index.project);
    if let Some(rest) = target.strip_prefix(&prefix) {
        let (path, anchor) = match rest.split_once('#') {
            Some((p, a)) => (p, Some(a)),
            None => (rest, None),
        };
        if let Some(code) = path.strip_prefix("feature/") {
            return Some(Ref::Item {
                code: code.trim_end_matches('/').to_string(),
                requirement: anchor.map(String::from),
            });
        }
        if let Some(code) = path.strip_prefix("milestone/") {
            return Some(Ref::Milestone {
                code: code.trim_end_matches('/').to_string(),
            });
        }
        if path == "charter" {
            return anchor.map(|g| Ref::Goal { id: g.to_string() });
        }
        // The monitor's path is already relative to the doc tree: taken as written.
        if let Some(doc) = path.strip_prefix("docs/file/") {
            return Some(Ref::Doc {
                path: doc.to_string(),
            });
        }
        return None;
    }
    // Another project's page is not this board's to check.
    if target.starts_with("/p/") {
        return None;
    }
    let path = target.split('#').next().unwrap_or(target);
    path.ends_with(".md").then(|| Ref::Doc {
        path: doc_key(path),
    })
}

/// Prose: board-shaped tokens only.
fn scan_prose(text: &str, line_no: usize, index: &BoardIndex, out: &mut Vec<Found>) {
    let chars: Vec<char> = text.chars().collect();
    let boundary = |i: usize| i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
    let mut i = 0;
    while i < chars.len() {
        // A doc path: a run of path characters ending in `.md`.
        if boundary(i) && (chars[i].is_alphanumeric() || chars[i] == '.' || chars[i] == '/') {
            let mut j = i;
            while j < chars.len()
                && (chars[j].is_alphanumeric() || matches!(chars[j], '/' | '-' | '_' | '.'))
            {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let word = word.trim_end_matches('.');
            if word.ends_with(".md") && word.len() > 3 {
                let path = doc_key(word);
                if index.docs.contains(&path) {
                    out.push(Found {
                        reference: Ref::Doc { path },
                        line: line_no,
                        explicit: false,
                    });
                }
                i = j;
                continue;
            }
        }
        if boundary(i) && chars[i].is_ascii_uppercase() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_uppercase() || chars[j].is_ascii_digit()) {
                j += 1;
            }
            if j < chars.len() && chars[j] == '-' {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_ascii_digit() {
                    k += 1;
                }
                let ends = k == chars.len() || !(chars[k].is_alphanumeric() || chars[k] == '_');
                if k > j + 1 && ends {
                    let prefix: String = chars[i..j].iter().collect();
                    let code: String = chars[i..k].iter().collect();
                    // `/R-n` after an item code names one of its requirements.
                    let mut end = k;
                    let mut requirement = None;
                    let rest: String = chars[k..].iter().take(8).collect();
                    if let Some(r) = rest.strip_prefix("/R-") {
                        let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if !digits.is_empty() {
                            requirement = Some(format!("R-{digits}"));
                            end = k + 3 + digits.len();
                        }
                    }
                    let reference = if prefix == "ADR" {
                        Some(Ref::Adr { id: code })
                    } else if prefix == "G" && !index.goals.is_empty() {
                        Some(Ref::Goal { id: code })
                    } else if index.items.contains_key(&code)
                        || index.item_prefixes.contains(&prefix)
                    {
                        Some(Ref::Item { code, requirement })
                    } else if index.milestones.contains(&code)
                        || index.milestone_prefixes.contains(&prefix)
                    {
                        Some(Ref::Milestone { code })
                    } else {
                        None
                    };
                    if let Some(reference) = reference {
                        out.push(Found {
                            reference,
                            line: line_no,
                            explicit: false,
                        });
                    }
                    i = end;
                    continue;
                }
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture both finders are checked against: an index, and markdown with what it holds.
    #[derive(serde::Deserialize)]
    struct Fixture {
        index: FixtureIndex,
        cases: Vec<Case>,
    }
    #[derive(serde::Deserialize)]
    struct FixtureIndex {
        project: String,
        #[serde(default)]
        codes: crate::config::CodesConfig,
        items: BTreeMap<String, Vec<String>>,
        milestones: Vec<String>,
        adrs: BTreeMap<String, String>,
        goals: Vec<String>,
        docs: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        markdown: String,
        /// `[kind, id, line, explicit, broken]`, id as `CODE`, `CODE/R-n` or a path.
        expect: Vec<(String, String, usize, bool, bool)>,
    }

    fn index_of(f: &FixtureIndex) -> BoardIndex {
        let mut index = BoardIndex {
            project: f.project.clone(),
            item_prefixes: f.codes.prefixes(),
            ..Default::default()
        };
        for (code, reqs) in &f.items {
            index
                .item_prefixes
                .insert(prefix_of(code).unwrap().to_string());
            index
                .items
                .insert(code.clone(), reqs.iter().cloned().collect());
        }
        for m in &f.milestones {
            index
                .milestone_prefixes
                .insert(prefix_of(m).unwrap().to_string());
            index.milestones.insert(m.clone());
        }
        index.adrs = f.adrs.clone();
        index.goals = f.goals.iter().cloned().collect();
        index.docs = f.docs.iter().cloned().collect();
        index
    }

    fn describe(r: &Ref) -> (String, String) {
        match r {
            Ref::Item { code, requirement } => (
                "item".into(),
                match requirement {
                    Some(req) => format!("{code}/{req}"),
                    None => code.clone(),
                },
            ),
            Ref::Milestone { code } => ("milestone".into(), code.clone()),
            Ref::Adr { id } => ("adr".into(), id.clone()),
            Ref::Goal { id } => ("goal".into(), id.clone()),
            Ref::Doc { path } => ("doc".into(), path.clone()),
        }
    }

    /// FEAT-181: the finder against the shared fixture the monitor's port is checked against too.
    #[test]
    fn finder_matches_the_shared_fixture() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../../web/scripts/refs-fixture.json"))
                .unwrap();
        let index = index_of(&fixture.index);
        for case in &fixture.cases {
            let got: Vec<(String, String, usize, bool, bool)> = find(&case.markdown, &index)
                .iter()
                .map(|f| {
                    let (kind, id) = describe(&f.reference);
                    (kind, id, f.line, f.explicit, index.is_broken(f))
                })
                .collect();
            assert_eq!(got, case.expect, "case: {}", case.name);
        }
    }

    #[test]
    fn links_go_to_monitor_pages() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../../web/scripts/refs-fixture.json"))
                .unwrap();
        let index = index_of(&fixture.index);
        let item = Ref::Item {
            code: "FEAT-012".into(),
            requirement: Some("R-1".into()),
        };
        assert_eq!(
            index.href(&item).as_deref(),
            Some("/p/shop/feature/FEAT-012#R-1")
        );
        assert_eq!(
            index
                .href(&Ref::Adr {
                    id: "ADR-0005".into()
                })
                .as_deref(),
            Some("/p/shop/docs/file/decisions/0005-carts.md")
        );
        assert_eq!(index.href(&Ref::Goal { id: "G-9".into() }), None);
    }
}
