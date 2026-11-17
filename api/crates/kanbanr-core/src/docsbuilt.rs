//! Whether the documents an item says it will write are built (DEF-033).
//!
//! `docs-folded` is a person's word that the documents are folded into the record. It is offered
//! only once there is something to confirm: every board doc the item names in `affects_docs` or
//! links under `Affects docs` has changed since it entered its stage, every repository doc its
//! Design lists there has changed on its branch, and every task is complete (DEF-046). An item that
//! lists no documents is offered the sign-off once its tasks are done, and the offer says it changes
//! no documents. A sign-off given with the code repository pins the item's branch tips, and a
//! later commit on any of them that changes shipped code or a doc the item names lapses it
//! ([`moved_on`], DEF-072).
//!
//! The pure parts (what an item lists, what a task writes) come first; reading the board's history
//! and the code repository comes last, and only when the item lists repository docs.

use crate::models::{FeatureItem, TaskState};
use crate::readiness::{Check, Context, Gap, Level};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The kind of document that is not built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Board,
    Repository,
    Task,
}

impl DocKind {
    pub fn label(self) -> &'static str {
        match self {
            DocKind::Board => "board doc",
            DocKind::Repository => "repository doc",
            DocKind::Task => "task",
        }
    }
}

/// One document still unbuilt, and why, as a sentence a person can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unbuilt {
    pub kind: DocKind,
    pub path: String,
    pub why: String,
}

/// The answer for one item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocsBuilt {
    /// Everything the item says it will write: board docs, repository docs, docs tasks (as
    /// "T3: ...").
    pub listed: Vec<String>,
    pub unbuilt: Vec<Unbuilt>,
    /// Nothing listed and no docs task: the sign-off then waits only on the tasks.
    pub changes_no_documents: bool,
}

impl DocsBuilt {
    pub fn is_built(&self) -> bool {
        self.unbuilt.is_empty()
    }

    /// One gap per unbuilt document.
    pub fn gaps(&self) -> Vec<Gap> {
        self.unbuilt
            .iter()
            .map(|u| Gap {
                check: Check::DocsUpdated,
                requirement: None,
                level: Level::Warning,
                label: "docs not built".to_string(),
                message: u.why.clone(),
            })
            .collect()
    }

    /// The unbuilt documents as one sentence, for a refusal.
    pub fn sentence(&self) -> String {
        self.unbuilt
            .iter()
            .map(|u| u.why.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// The section of `spec` under the heading `Affects docs` (any level, 2 to 4 `#`), to the next
/// heading of the same or a higher level. Headings inside fenced code are not headings.
fn affects_section(spec: &str) -> String {
    let mut fenced = false;
    let mut level_in: Option<usize> = None;
    let mut out = String::new();
    for line in spec.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
        }
        let level = t.chars().take_while(|c| *c == '#').count();
        if !fenced && (1..=6).contains(&level) && t[level..].starts_with(' ') {
            if let Some(open) = level_in {
                if level <= open {
                    level_in = None;
                } else {
                    out.push_str(line);
                    out.push('\n');
                    continue;
                }
            }
            let text = t[level..].trim().trim_end_matches('#').trim();
            if (2..=4).contains(&level)
                && text
                    .trim_end_matches(':')
                    .trim()
                    .eq_ignore_ascii_case("affects docs")
            {
                level_in = Some(level);
            }
            continue;
        }
        if level_in.is_some() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The repository paths named under the spec's `Affects docs` heading: every inline code span and
/// every link destination that looks like a repository file. Deliberately not prose parsing: a
/// path written without backticks or a link is not read.
pub fn repository_docs(spec: &str, board_docs: &BTreeSet<String>) -> Vec<String> {
    let section = affects_section(spec);
    let mut found: Vec<String> = Vec::new();
    // Inline code spans.
    let parts: Vec<&str> = section.split('`').collect();
    for (i, span) in parts.iter().enumerate() {
        if i % 2 == 1 && i + 1 < parts.len() {
            found.push(span.trim().to_string());
        }
    }
    // Link destinations: `](x)`.
    let mut rest = section.as_str();
    while let Some(at) = rest.find("](") {
        rest = &rest[at + 2..];
        if let Some(end) = rest.find(')') {
            found.push(rest[..end].trim().to_string());
            rest = &rest[end..];
        }
    }
    let mut keep: Vec<String> = found
        .into_iter()
        .filter(|x| {
            !x.is_empty()
                && !x.contains(char::is_whitespace)
                && !x.starts_with('/')
                && !x.starts_with("http")
                && !x.contains('*')
                && !x.contains('<')
                && (x.starts_with("docs/")
                    || [".md", ".json", ".yaml", ".txt"]
                        .iter()
                        .any(|e| x.ends_with(e)))
                && !board_docs.contains(&crate::refs::doc_key(x))
        })
        .collect();
    keep.sort();
    keep.dedup();
    keep
}

/// The board docs linked under the spec's `Affects docs` heading as this project's doc pages,
/// `/p/<project>/docs/file/<path>` (DEF-046), as doc keys, sorted and deduplicated. A `#fragment`
/// or `?query` is cut first. Links to another project, and to items or milestones, are not docs.
pub fn linked_board_docs(spec: &str, project: &str) -> Vec<String> {
    let mut keys: Vec<String> = linked_lines(spec, project)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Each board doc linked under `Affects docs`, with the text of the line that links it (DEF-047).
fn linked_lines(spec: &str, project: &str) -> Vec<(String, String)> {
    let section = affects_section(spec);
    let prefix = format!("/p/{project}/docs/file/");
    let mut out: Vec<(String, String)> = Vec::new();
    for line in section.lines() {
        let mut rest = line;
        while let Some(at) = rest.find("](") {
            rest = &rest[at + 2..];
            let Some(end) = rest.find(')') else {
                break;
            };
            let dest = rest[..end].trim();
            rest = &rest[end..];
            let Some(path) = dest.strip_prefix(prefix.as_str()) else {
                continue;
            };
            let path = path.split(['#', '?']).next().unwrap_or_default().trim();
            if !path.is_empty() {
                out.push((crate::refs::doc_key(path), line.to_string()));
            }
        }
    }
    out
}

/// The linked board docs the item must change (DEF-047): [`linked_board_docs`] minus those whose
/// list line says `unchanged` or `not changed` (any case), which the Design only reads. A doc
/// linked on two lines is read-only only when every line says so. Sorted, deduplicated, pure.
pub fn docs_to_change(spec: &str, project: &str) -> Vec<String> {
    let mut keys: Vec<String> = linked_lines(spec, project)
        .into_iter()
        .filter(|(_, line)| {
            let l = line.to_lowercase();
            !(l.contains("unchanged") || l.contains("not changed"))
        })
        .map(|(key, _)| key)
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')
}

/// Whether `text` holds `word` as a whole word, not inside a longer name.
fn names_file(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        let rest = &text[at + word.len()..];
        // A full stop that ends a sentence is not part of the name.
        let continues = after.is_some_and(|c| {
            path_char(c) && !(c == '.' && !rest[1..].starts_with(|n: char| n.is_alphanumeric()))
        });
        !before.is_some_and(|c| path_char(c) && c != '/') && !continues
    })
}

/// Whether a task writes docs: its text names a listed document, a path under `docs/`, or the file
/// `CHANGELOG.md`, `SECURITY.md` or `README.md`.
pub fn is_docs_task(text: &str, listed: &[String]) -> bool {
    if listed
        .iter()
        .any(|l| !l.is_empty() && text.contains(l.as_str()))
    {
        return true;
    }
    if text
        .match_indices("docs/")
        .any(|(at, _)| text[at + 5..].chars().next().is_some_and(path_char))
    {
        return true;
    }
    ["CHANGELOG.md", "SECURITY.md", "README.md"]
        .iter()
        .any(|f| names_file(text, f))
}

/// The whole answer for one item.
pub fn evaluate(
    project: &crate::store::Project,
    feature: &FeatureItem,
    ctx: &Context,
) -> DocsBuilt {
    let mut built = DocsBuilt::default();
    let declared: Vec<String> = feature
        .definition
        .as_ref()
        .map(|d| d.affects_docs.clone())
        .unwrap_or_default();
    let board_keys: BTreeSet<String> = declared
        .iter()
        .map(|d| crate::readiness::affects_doc_key(d))
        .collect();

    // Board docs: the existing gate's answer, reused, then the docs the Design links under
    // `Affects docs` (DEF-046), judged the same way.
    let linked: Vec<String> = docs_to_change(&feature.specification, &project.id)
        .into_iter()
        .filter(|k| !board_keys.contains(k))
        .collect();
    built.listed.extend(board_keys.iter().cloned());
    built.listed.extend(linked.iter().cloned());
    if let Some(docs) = ctx.docs_dir.as_deref() {
        let window = crate::readiness::DocsWindow::current(feature);
        let gaps = crate::readiness::docs_updated_gaps(feature, docs, &ctx.history, window)
            .into_iter()
            .chain(crate::readiness::docs_unchanged_gaps(
                feature,
                &linked,
                docs,
                &ctx.history,
                "linked under Affects docs",
                window,
            ));
        for gap in gaps {
            built.unbuilt.push(Unbuilt {
                kind: DocKind::Board,
                path: gap.label.clone(),
                why: gap.message,
            });
        }
    }

    // Repository docs.
    let repo_docs: Vec<String> = named_repository_docs(project, feature)
        .into_iter()
        .collect();
    built.listed.extend(repo_docs.iter().cloned());
    if !repo_docs.is_empty() {
        built
            .unbuilt
            .extend(repository_unbuilt(project, feature, ctx, &repo_docs));
    }

    // Every open task holds the sign-off (DEF-046); one that writes docs says so, and is listed.
    let listed_paths: Vec<String> = declared
        .iter()
        .cloned()
        .chain(built.listed.clone())
        .collect();
    for task in feature.all_tasks() {
        let writes_docs = is_docs_task(&task.text, &listed_paths);
        if writes_docs {
            built.listed.push(format!("{}: {}", task.key, task.text));
        }
        if task.state != TaskState::Completed {
            built.unbuilt.push(Unbuilt {
                kind: DocKind::Task,
                path: task.key.clone(),
                why: if writes_docs {
                    format!("task {} writes docs and is not complete", task.key)
                } else {
                    format!("task {} is not complete", task.key)
                },
            });
        }
    }

    // Nothing listed: the sign-off confirms finished work and nothing else.
    built.changes_no_documents = built.listed.is_empty();
    if built.changes_no_documents && feature.all_tasks().next().is_none() {
        built.unbuilt.push(Unbuilt {
            kind: DocKind::Task,
            path: String::new(),
            why: "it has no tasks, so nothing shows any work was done".to_string(),
        });
    }
    built
}

/// The repository docs `feature` names (the repo paths `evaluate` lists): the same set
/// `repository_docs(spec, board_named)` gives inside `evaluate`, worked out once here and used by both.
pub fn named_repository_docs(
    project: &crate::store::Project,
    feature: &FeatureItem,
) -> BTreeSet<String> {
    let board_keys: BTreeSet<String> = feature
        .definition
        .as_ref()
        .map(|d| d.affects_docs.clone())
        .unwrap_or_default()
        .iter()
        .map(|d| crate::readiness::affects_doc_key(d))
        .collect();
    let board_named: BTreeSet<String> = board_keys
        .iter()
        .cloned()
        .chain(docs_to_change(&feature.specification, &project.id))
        .collect();
    repository_docs(&feature.specification, &board_named)
        .into_iter()
        .collect()
}

/// The repository docs `feature` names (the set [`named_repository_docs`] gives) with their blob ids
/// at the position `at` (FTR-004), as a sign-off records them: one `tree_at` call, the same for git
/// and disk. A doc not there is left out; a `tree_at` error leaves the map empty, because a record
/// that cannot be made steps aside (ADR-0004).
pub fn docs_read_at(
    project: &crate::store::Project,
    feature: &FeatureItem,
    repo: &std::path::Path,
    at: &crate::picks::Picked,
) -> crate::Result<BTreeMap<String, String>> {
    if named_repository_docs(project, feature).is_empty() {
        return Ok(BTreeMap::new());
    }
    let named = resolved_repository_docs(project, feature, repo, "HEAD");
    let Ok(tree) = crate::position::tree_at(repo, &at.state, "") else {
        return Ok(BTreeMap::new());
    };
    Ok(tree
        .into_iter()
        .filter(|(path, _)| named.contains(path))
        .collect())
}

/// Does a change to `path` lapse a docs-folded sign-off (DEF-072)? True for a doc in `named`, and
/// for any path that is neither a test file (`docdrift::is_test_path`) nor documentation
/// (`docdrift::is_documentation`, except lockfiles, which ship).
pub fn lapses_signoff(path: &str, named: &BTreeSet<String>) -> bool {
    if named.contains(path) {
        return true;
    }
    if crate::docdrift::is_test_path(path) {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    let lockfile = base.ends_with(".lock") || base == "package-lock.json";
    lockfile || !crate::docdrift::is_documentation(path)
}

/// The item's branches and their tips now (DEF-046), as a `docs-folded` sign-off records them.
/// Empty without the code repository's branches, or when the item has no branch.
pub fn branch_heads(
    project: &crate::store::Project,
    feature: &FeatureItem,
    ctx: &Context,
) -> BTreeMap<String, String> {
    let Some(facts) = ctx.branch_facts() else {
        return BTreeMap::new();
    };
    crate::gitmodel::item_branches(&project.config, facts, &feature.code)
        .into_iter()
        .filter_map(|b| facts.branches.get(&b).map(|tip| (b.clone(), tip.clone())))
        .collect()
}

/// Why the item's current `docs-folded` sign-off no longer covers what was built (DEF-046): one
/// of its branches is new since, was rewritten, or changed shipped code or a doc the item names
/// after the tip recorded ([`lapses_signoff`]); a branch whose later commits change only tests, or
/// docs the item does not name, leaves it standing (DEF-072). Without the code repository, or when
/// the files changed cannot be read, a moved tip lapses it. `None` when it is not
/// signed, recorded no branches, or the branches cannot be read: the lapse cannot be shown, so the
/// sign-off stands. A recorded branch since deleted is not a lapse. Tips are compared, never commit
/// times, which are whole seconds and can run out of order (DEF-035).
pub fn moved_on(
    project: &crate::store::Project,
    feature: &FeatureItem,
    ctx: &Context,
) -> Option<String> {
    let signed = feature.signoff_current(crate::method::DOCS_FOLDED)?;
    if signed.branch_heads.is_empty() {
        return None;
    }
    let facts = ctx.branch_facts()?;
    let name = crate::method::DOCS_FOLDED;
    let mut named: Option<BTreeSet<String>> = None;
    for b in crate::gitmodel::item_branches(&project.config, facts, &feature.code) {
        let moved = format!("{b} has commits after sign-off '{name}' was given");
        let (recorded, tip) = match (signed.branch_heads.get(&b), facts.branches.get(&b)) {
            (r, t) if r == t => continue,
            (Some(r), Some(t)) => (r, t),
            _ => return Some(moved),
        };
        let Some(root) = ctx.code_repo.as_deref() else {
            return Some(moved);
        };
        let Some(files) = crate::coderepo::changed_between(root, recorded, tip) else {
            if !crate::coderepo::is_ancestor(root, recorded, tip) {
                return Some(format!(
                    "{b} was rewritten after sign-off '{name}' was given"
                ));
            }
            return Some(moved);
        };
        let named = named.get_or_insert_with(|| {
            resolved_repository_docs(
                project,
                feature,
                root,
                &line_of(project, feature, ctx, facts),
            )
        });
        if let Some(path) = files.iter().find(|p| lapses_signoff(p, named)) {
            return Some(format!(
                "{b} changed {path} after sign-off '{name}' was given"
            ));
        }
    }
    None
}

/// When the item first entered its build stage (DEF-105): the `at` of the first transition in its
/// history whose `to` is the build role's status (`In Progress` once `kanbanr start` moves it
/// there). Without a build role, or with no such transition, the time it was created.
pub fn build_started_at<'a>(config: &crate::ProjectConfig, feature: &'a FeatureItem) -> &'a str {
    config
        .branch_status()
        .and_then(|status| feature.history.iter().find(|t| t.to == status))
        .map_or(feature.created_at.as_str(), |t| t.at.as_str())
}

/// Old path to new path for every rename on `rev`'s history (DEF-105), newest rename winning when a
/// path moved more than once. One git log.
fn renames_on(root: &std::path::Path, rev: &str) -> crate::Result<BTreeMap<String, String>> {
    if rev.is_empty() || rev.starts_with('-') {
        return Err(crate::CoreError::Refused(format!(
            "refusing revision {rev:?}"
        )));
    }
    #[cfg(test)]
    GIT_LOGS.with(|c| c.set((c.get().0, c.get().1 + 1)));
    let out = crate::coderepo::git(
        root,
        &[
            "log",
            rev,
            "--no-merges",
            "-M",
            "--diff-filter=R",
            "--name-status",
            "--format=%x1e",
            "--",
        ],
    )?;
    let mut renames = BTreeMap::new();
    for line in out.lines() {
        let mut cols = line.split('\t');
        if cols.next().is_some_and(|s| s.starts_with('R'))
            && let (Some(old), Some(new)) = (cols.next(), cols.next())
        {
            renames
                .entry(old.to_string())
                .or_insert_with(|| new.to_string());
        }
    }
    Ok(renames)
}

/// Follow `path` forward through `renames`, at most 16 steps, to the name it has now. `None` when
/// it was never renamed.
fn moved_to(renames: &BTreeMap<String, String>, path: &str) -> Option<String> {
    let mut at = path;
    for _ in 0..16 {
        match renames.get(at) {
            Some(next) => at = next,
            None => break,
        }
    }
    (at != path).then(|| at.to_string())
}

/// A listed repository doc and the path it is judged at (DEF-105).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoDoc {
    pub listed: String,
    /// The path today: `listed`, unless the line's history moved it.
    pub path: String,
    /// False when no tree asked holds `path`.
    pub exists: bool,
}

/// Where each `listed` doc is, over the files of `revs` (the line first). One `ls-tree` per rev,
/// and the line's rename log once, only when a listed name is missing.
fn resolve(
    root: &std::path::Path,
    revs: &[&str],
    listed: &BTreeSet<String>,
) -> crate::Result<Vec<RepoDoc>> {
    let mut files: BTreeSet<String> = BTreeSet::new();
    for rev in revs {
        if rev.is_empty() || rev.starts_with('-') {
            return Err(crate::CoreError::Refused(format!(
                "refusing revision {rev:?}"
            )));
        }
        let raw = crate::coderepo::git_raw(root, &["ls-tree", "-r", "--name-only", "-z", rev])?;
        files.extend(
            raw.split(|b| *b == 0)
                .filter(|n| !n.is_empty())
                .map(|n| String::from_utf8_lossy(n).into_owned()),
        );
    }
    let renames = match (listed.iter().any(|p| !files.contains(p)), revs.first()) {
        (true, Some(line)) => renames_on(root, line)?,
        _ => BTreeMap::new(),
    };
    Ok(listed
        .iter()
        .map(|p| {
            if files.contains(p) {
                return RepoDoc {
                    listed: p.clone(),
                    path: p.clone(),
                    exists: true,
                };
            }
            match moved_to(&renames, p) {
                Some(now) if files.contains(&now) => RepoDoc {
                    listed: p.clone(),
                    path: now,
                    exists: true,
                },
                _ => RepoDoc {
                    listed: p.clone(),
                    path: p.clone(),
                    exists: false,
                },
            }
        })
        .collect())
}

/// The repository docs `feature` names, each at the path it has on `rev` (DEF-105). On a git error
/// the named set is kept as it is.
pub fn resolved_repository_docs(
    project: &crate::store::Project,
    feature: &FeatureItem,
    root: &std::path::Path,
    rev: &str,
) -> BTreeSet<String> {
    let named = named_repository_docs(project, feature);
    if named.is_empty() {
        return named;
    }
    match resolve(root, &[rev], &named) {
        Ok(docs) => docs.into_iter().map(|d| d.path).collect(),
        Err(_) => named,
    }
}

// Git logs run by the repository-docs judgement, per thread, for the cost test (DEF-105 R-6).
#[cfg(test)]
thread_local! {
    static GIT_LOGS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// `at` (a board time such as `2026-11-16 21:33:32.48+00:00`) as `2026-11-16T21:33:32Z`, whole
/// seconds in UTC, for git's `--since`. `None` when it is not a time.
fn git_time(at: &str) -> Option<String> {
    use time::format_description::well_known::Rfc3339;
    let at = at.trim().replacen(' ', "T", 1);
    let t = time::OffsetDateTime::parse(&at, &Rfc3339)
        .ok()?
        .to_offset(time::UtcOffset::UTC);
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    ))
}

/// An item code: letters, a dash, digits. `docdrift` holds the same rule, privately.
fn valid_code(code: &str) -> bool {
    code.split_once('-').is_some_and(|(p, n)| {
        !p.is_empty()
            && p.chars().all(|c| c.is_ascii_alphabetic())
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
    })
}

/// The repository-relative paths touched by every non-merge commit, on any ref, whose `Refs:`
/// trailer names one of `codes` and that was committed at or after `since` (DEF-105). One git log.
/// A renamed file is named by its new path.
fn refs_touched(
    root: &std::path::Path,
    codes: &[&str],
    since: &str,
) -> crate::Result<BTreeSet<String>> {
    if codes.is_empty() {
        return Err(crate::CoreError::Refused("no item code to look for".into()));
    }
    for c in codes {
        if !valid_code(c) {
            return Err(crate::CoreError::Refused(format!(
                "{c:?} is not an item code (letters, a dash, digits)"
            )));
        }
    }
    let since = git_time(since).ok_or_else(|| {
        crate::CoreError::Refused(format!("{since:?} is not a time git can read"))
    })?;
    let grep = format!(
        "--grep=^Refs:.*kanbanr:({})([^0-9A-Za-z]|$)",
        codes.join("|")
    );
    let since = format!("--since={since}");
    #[cfg(test)]
    GIT_LOGS.with(|c| c.set((c.get().0 + 1, c.get().1)));
    let out = crate::coderepo::git(
        root,
        &[
            "log",
            "--all",
            "--no-merges",
            "-M",
            "-E",
            &grep,
            &since,
            "--name-only",
            "--format=%x1e",
        ],
    )?;
    Ok(out
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('\x1e'))
        .map(String::from)
        .collect())
}

/// The line the item's docs are judged against: the git model's line for the item, else main.
fn line_of(
    project: &crate::store::Project,
    feature: &FeatureItem,
    ctx: &Context,
    facts: &crate::gitmodel::BranchFacts,
) -> String {
    let config = &project.config;
    if config.cadence.git_model {
        let model = crate::gitmodel::derive(config, &ctx.sprints, facts);
        crate::gitmodel::line_for(&model, feature, &ctx.releases).unwrap_or(model.main)
    } else {
        facts.main.clone()
    }
}

/// The repository docs `paths` that have not changed on the item's branch against its line, or one
/// entry saying why they cannot be judged.
fn repository_unbuilt(
    project: &crate::store::Project,
    feature: &FeatureItem,
    ctx: &Context,
    paths: &[String],
) -> Vec<Unbuilt> {
    let held = |why: String| {
        vec![Unbuilt {
            kind: DocKind::Repository,
            path: String::new(),
            why,
        }]
    };
    let Some(root) = ctx.code_repo.as_deref() else {
        return held(
            "the code repository is not available here, so the repository docs cannot be \
             checked: start the monitor from the project folder"
                .to_string(),
        );
    };
    let Some(facts) = ctx.branch_facts() else {
        return held(
            "the code repository could not be read, so the repository docs cannot be checked"
                .to_string(),
        );
    };
    let config = &project.config;
    let branches = crate::gitmodel::item_branches(config, facts, &feature.code);
    if branches.is_empty() {
        return held(format!(
            "no branch for {} in the code repository, so the repository docs cannot be checked",
            feature.code
        ));
    }
    let line = line_of(project, feature, ctx, facts);
    let mut changed: BTreeSet<String> = BTreeSet::new();
    for branch in &branches {
        match crate::coderepo::changed_since_fork(root, &line, branch) {
            Ok(files) => changed.extend(files),
            Err(_) => {
                return held(format!(
                    "{branch} cannot be compared with {line}, so the repository docs cannot be \
                     checked"
                ));
            }
        }
    }
    // Every commit that names the item since it entered its build stage, merged or not (DEF-105).
    let codes: Vec<&str> = feature.codes().collect();
    let started = build_started_at(config, feature);
    match refs_touched(root, &codes, started) {
        Ok(files) => changed.extend(files),
        Err(_) => {
            return held(format!(
                "the code repository's history could not be read for {}'s commits, so the \
                 repository docs cannot be checked",
                feature.code
            ));
        }
    }
    let shown = branches.join(", ");
    let since = config
        .branch_status()
        .map_or("it was created".to_string(), |s| format!("it entered {s}"));
    let paths: BTreeSet<String> = paths.iter().cloned().collect();
    let revs: Vec<&str> = std::iter::once(line.as_str())
        .chain(branches.iter().map(String::as_str))
        .collect();
    let docs = match resolve(root, &revs, &paths) {
        Ok(docs) => docs,
        Err(_) => {
            return held(format!(
                "the code repository's files could not be read for {}, so the repository docs \
                 cannot be checked",
                feature.code
            ));
        }
    };
    docs.into_iter()
        .filter(|d| !changed.contains(&d.listed) && !changed.contains(&d.path))
        .map(|d| {
            if !d.exists {
                return Unbuilt {
                    kind: DocKind::Repository,
                    why: format!(
                        "{} is not a file in the code repository: not written yet, or Affects \
                         docs names it wrongly",
                        d.listed
                    ),
                    path: d.listed,
                };
            }
            let moved = if d.path == d.listed {
                String::new()
            } else {
                format!(" (listed as {})", d.listed)
            };
            Unbuilt {
                kind: DocKind::Repository,
                why: format!(
                    "{}{moved} has not changed on {shown} against {line}, and no commit for {} \
                     since {since} touched it",
                    d.path, feature.code
                ),
                path: d.path,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = "## Design\n\nChanges `api/src/x.rs` and `docs/guide/not-here.md`.\n\n### Affects docs\nBoard docs: [design/data-flow.md](/p/k/docs/file/design/data-flow.md), `design/data-flow.md`.\nRepository docs: `docs/guide/using/living-docs.md`, `SECURITY.md`, `CHANGELOG.md`, `docs/guide/a/*.md`.\nRelated: [FEAT-1](/p/k/feature/FEAT-1), [guide](docs/guide/guides/g.md), `some words.md`, `https://x/y.md`.\n\n### After\nSee `docs/guide/after.md`.\n";

    #[test]
    fn repository_docs_are_read_from_the_affects_docs_section_only() {
        let board: BTreeSet<String> = ["design/data-flow.md".to_string()].into();
        let got = repository_docs(SPEC, &board);
        assert_eq!(
            got,
            vec![
                "CHANGELOG.md",
                "SECURITY.md",
                "docs/guide/guides/g.md",
                "docs/guide/using/living-docs.md"
            ]
        );
        // No heading, no documents.
        assert!(repository_docs("## Design\n`docs/guide/x.md`\n", &board).is_empty());
    }

    /// DEF-046 R-1: board docs linked under `Affects docs` are read as this project's doc keys.
    #[test]
    fn board_docs_linked_under_affects_docs_are_read() {
        let spec = "## Design\n\nSee [x](/p/demo/docs/file/design/before.md).\n\n\
            ### Affects docs\n\
            - [data](/p/demo/docs/file/architecture/data.md#fields): fields.\n\
            - [again](/p/demo/docs/file/architecture/data.md).\n\
            - [ADR](/p/demo/docs/file/decisions/0015-pin.md?v=2).\n\
            - [other](/p/elsewhere/docs/file/design/theirs.md).\n\
            - [FEAT-1](/p/demo/feature/FEAT-1), [MS-1](/p/demo/milestone/MS-1).\n\
            - Repository docs: `docs/guide/x.md`.\n\n\
            ### After\n[later](/p/demo/docs/file/design/after.md)\n";
        assert_eq!(
            linked_board_docs(spec, "demo"),
            vec!["architecture/data.md", "decisions/0015-pin.md"]
        );
        assert!(linked_board_docs(spec, "other").is_empty());
        assert!(linked_board_docs("## Design\n[x](/p/demo/docs/file/a.md)\n", "demo").is_empty());
    }

    #[test]
    fn docs_to_change_drops_read_only_links() {
        let spec = "## Design\n\n### Affects docs\n\
            - [data](/p/demo/docs/file/architecture/data.md): fields.\n\
            - [ADR](/p/demo/docs/file/decisions/0005-x.md): Unchanged, only read.\n\
            - [flow](/p/demo/docs/file/design/flow.md) is not changed here.\n\
            - [other](/p/demo/docs/file/design/other.md) (unchanged) and \
            [again](/p/demo/docs/file/architecture/data.md).\n";
        assert_eq!(docs_to_change(spec, "demo"), vec!["architecture/data.md"]);
        assert_eq!(
            linked_board_docs(spec, "demo"),
            vec![
                "architecture/data.md",
                "decisions/0005-x.md",
                "design/flow.md",
                "design/other.md"
            ]
        );
        assert!(docs_to_change("## Design\nnothing\n", "demo").is_empty());
    }

    #[test]
    fn a_task_writes_docs_when_it_names_a_listed_document() {
        let listed = vec!["skill/kanbanr/SKILL.md".to_string()];
        assert!(is_docs_task(
            "Update skill/kanbanr/SKILL.md for the rule",
            &listed
        ));
        assert!(is_docs_task("Write docs/guide/using/x.md", &[]));
        assert!(is_docs_task("Changelog entry in CHANGELOG.md", &[]));
        assert!(is_docs_task("Mention it in README.md.", &[]));
        assert!(!is_docs_task("Refactor the parser", &listed));
        assert!(!is_docs_task("Touch docs/ folder", &[]));
        assert!(!is_docs_task("Edit MYREADME.md.bak", &[]));
    }

    /// A board with no history of its own and a code repository, as `(store, board dir, repo)`.
    fn fixture(tag: &str) -> (crate::Store, std::path::PathBuf, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-docsbuilt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let repo = crate::coderepo::tests::scratch_repo(&format!("docsbuilt-{tag}"));
        (store, dir, repo)
    }

    fn item(store: &crate::Store, title: &str, spec: &str, docs: &[&str]) -> String {
        let f = store.add_feature("demo", title, spec, "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(crate::models::FeatureDefinition {
                    statement: title.into(),
                    affects_docs: docs.iter().map(|d| d.to_string()).collect(),
                    ..Default::default()
                }),
            )
            .unwrap();
        f.code
    }

    fn task(store: &crate::Store, code: &str, text: &str, state: TaskState) {
        let todo = store.add_todo_list("demo", code, "t", None).unwrap();
        let t = store
            .add_task(
                "demo",
                code,
                &todo.code,
                text,
                None,
                &crate::store::TaskFields::default(),
            )
            .unwrap();
        store
            .set_task_state("demo", code, &todo.code, &t.key, state)
            .unwrap();
    }

    fn judge(store: &crate::Store, code: &str, repo: Option<&std::path::Path>) -> DocsBuilt {
        let project = store.load("demo").unwrap();
        let ctx = Context::load_in(store, "demo", &project.config, repo);
        evaluate(&project, project.feature(code).unwrap(), &ctx)
    }

    fn branch_for(store: &crate::Store, code: &str) -> String {
        let project = store.load("demo").unwrap();
        let f = project.feature(code).unwrap();
        crate::scm::branch_for(project.config.branch_pattern(), code, &f.title)
    }

    /// DEF-033 R-2: each rule holds the sign-off by itself, and is reported as its own kind.
    #[test]
    fn board_docs_repository_docs_and_docs_tasks_are_each_needed() {
        let (store, dir, repo) = fixture("each");
        // A board doc the board does not hold: only the board rule fails.
        let a = item(&store, "A", "", &["nope"]);
        task(&store, &a, "Build it", TaskState::Completed);
        let got = judge(&store, &a, Some(&repo));
        assert_eq!(got.unbuilt.len(), 1, "{got:?}");
        assert_eq!(got.unbuilt[0].kind, DocKind::Board);
        assert!(!got.changes_no_documents);

        // A repository doc its branch has not changed: only the repository rule fails.
        let b = item(
            &store,
            "B",
            "## Design\n\n### Affects docs\nRepository docs: `docs/guide/b.md`.\n",
            &[],
        );
        task(&store, &b, "Build it", TaskState::Completed);
        let branch = branch_for(&store, &b);
        // The doc exists on the line, so only its change is missing (DEF-105).
        std::fs::create_dir_all(repo.join("docs/guide")).unwrap();
        crate::coderepo::tests::commit_file(&repo, "docs/guide/b.md", "b0\n", "base doc");
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        crate::coderepo::tests::commit_file(&repo, "code.rs", "x\n", "code");
        let got = judge(&store, &b, Some(&repo));
        assert_eq!(got.unbuilt.len(), 1, "{got:?}");
        assert_eq!(got.unbuilt[0].kind, DocKind::Repository);
        assert!(
            got.unbuilt[0]
                .why
                .contains("docs/guide/b.md has not changed")
        );
        // Once the branch changes it, the item is built.
        std::fs::create_dir_all(repo.join("docs/guide")).unwrap();
        crate::coderepo::tests::commit_file(&repo, "docs/guide/b.md", "b\n", "docs");
        let got = judge(&store, &b, Some(&repo));
        assert!(got.is_built(), "{got:?}");
        crate::coderepo::git(&repo, &["checkout", "-q", "main"]).unwrap();

        // A task that writes docs and is not complete: only the task rule fails.
        let c = item(&store, "C", "", &[]);
        task(&store, &c, "Write docs/guide/c.md", TaskState::InProgress);
        let got = judge(&store, &c, Some(&repo));
        assert_eq!(got.unbuilt.len(), 1, "{got:?}");
        assert_eq!(got.unbuilt[0].kind, DocKind::Task);
        assert!(
            got.unbuilt[0]
                .why
                .contains("writes docs and is not complete")
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-046 R-1: an item that lists a document is held by an open task that writes none.
    #[test]
    fn every_open_task_holds_the_signoff() {
        let (store, dir, repo) = fixture("open-task");
        let code = item(
            &store,
            "Open",
            "## Design\n\n### Affects docs\nRepository docs: `docs/guide/o.md`.\n",
            &[],
        );
        task(&store, &code, "Build the parser", TaskState::InProgress);
        let branch = branch_for(&store, &code);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        std::fs::create_dir_all(repo.join("docs/guide")).unwrap();
        crate::coderepo::tests::commit_file(&repo, "docs/guide/o.md", "o\n", "docs");
        let got = judge(&store, &code, Some(&repo));
        assert!(!got.changes_no_documents);
        assert_eq!(
            got.unbuilt,
            vec![Unbuilt {
                kind: DocKind::Task,
                path: "T1".into(),
                why: "task T1 is not complete".into(),
            }]
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-033 R-3: no documents at all means the tasks decide, and the answer says so.
    #[test]
    fn an_item_with_no_documents_is_offered_the_signoff_once_its_tasks_are_done() {
        let (store, dir, _repo) = fixture("none");
        let code = item(&store, "Plain", "", &[]);
        let got = judge(&store, &code, None);
        assert!(!got.is_built());
        assert!(got.unbuilt[0].why.contains("no tasks"), "{got:?}");

        task(&store, &code, "Build it", TaskState::InProgress);
        let got = judge(&store, &code, None);
        assert!(
            got.unbuilt[0].why.contains("task T1 is not complete"),
            "{got:?}"
        );

        let todo = store.add_todo_list("demo", &code, "u", None).unwrap();
        let t = store
            .add_task(
                "demo",
                &code,
                &todo.code,
                "Also",
                None,
                &crate::store::TaskFields::default(),
            )
            .unwrap();
        store
            .set_task_state("demo", &code, &todo.code, &t.key, TaskState::Completed)
            .unwrap();
        let project = store.load_for_gates("demo").unwrap();
        let first = project.feature(&code).unwrap().todo_lists[0].clone();
        store
            .set_task_state(
                "demo",
                &code,
                &first.code,
                &first.tasks[0].key,
                TaskState::Completed,
            )
            .unwrap();
        let got = judge(&store, &code, None);
        assert!(got.is_built(), "{got:?}");
        assert!(got.changes_no_documents);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An item naming `CHANGELOG.md` whose branch commits `file`, merged `--no-ff` into main (the
    /// line `judge` uses without the git model), as what `judge` makes of it.
    fn merged_item(tag: &str, file: &str) -> DocsBuilt {
        let (store, dir, repo) = fixture(tag);
        let code = item(
            &store,
            "Merged",
            "## Design\n\n### Affects docs\nRepository docs: `CHANGELOG.md`.\n",
            &[],
        );
        task(&store, &code, "Build it", TaskState::Completed);
        let branch = branch_for(&store, &code);
        crate::coderepo::tests::commit_file(&repo, "CHANGELOG.md", "base\n", "base changelog");
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        crate::coderepo::tests::commit_file(&repo, file, "x\n", "the item");
        crate::coderepo::git(&repo, &["checkout", "-q", "main"]).unwrap();
        crate::coderepo::merge_no_ff(&repo, "main", &branch, "Merge the item").unwrap();
        assert!(crate::coderepo::is_ancestor(&repo, &branch, "main"));
        let got = judge(&store, &code, Some(&repo));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
        got
    }

    /// DEF-091 R-1: once the line contains the branch, the docs it changed still read as built.
    #[test]
    fn a_branch_merged_into_its_release_still_reads_its_changed_docs_as_built() {
        let got = merged_item("merged-built", "CHANGELOG.md");
        assert!(
            got.unbuilt.iter().all(|u| u.kind != DocKind::Repository),
            "{got:?}"
        );
    }

    /// DEF-091 R-2: a merged branch that did not change the doc still reads as not built.
    #[test]
    fn a_merged_branch_that_changed_nothing_still_reads_as_not_built() {
        let got = merged_item("merged-unbuilt", "src.rs");
        let repo: Vec<_> = got
            .unbuilt
            .iter()
            .filter(|u| u.kind == DocKind::Repository)
            .collect();
        assert_eq!(repo.len(), 1, "{got:?}");
        assert_eq!(repo[0].path, "CHANGELOG.md");
        assert!(
            repo[0].why.contains("CHANGELOG.md has not changed on")
                && repo[0].why.contains("no commit for"),
            "{got:?}"
        );
    }

    /// DEF-033 R-5: repository docs that cannot be read hold the sign-off, and say why.
    #[test]
    fn unreadable_repository_docs_hold_the_signoff_and_say_why() {
        let (store, dir, repo) = fixture("unreadable");
        let code = item(&store, "R", "### Affects docs\n`docs/guide/r.md`\n", &[]);
        task(&store, &code, "Build it", TaskState::Completed);
        let got = judge(&store, &code, None);
        assert!(!got.is_built());
        assert!(got.unbuilt[0].why.contains("not available here"), "{got:?}");
        // A repository, but no branch for the item.
        let got = judge(&store, &code, Some(&repo));
        assert!(!got.is_built());
        assert!(
            got.unbuilt[0]
                .why
                .contains(&format!("no branch for {code}")),
            "{got:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// Write `file` in `repo` (making its folders) and commit it on the checked-out branch.
    fn commit_at(repo: &std::path::Path, file: &str, content: &str) {
        if let Some(parent) = std::path::Path::new(file).parent() {
            std::fs::create_dir_all(repo.join(parent)).unwrap();
        }
        crate::coderepo::tests::commit_file(repo, file, content, file);
    }

    /// An item with `spec`, its task done, on its own branch with `first` committed, signed
    /// docs-folded through the dispatch route with the code repository, as
    /// `(store, board dir, repo, item, branch)`.
    fn signed_item(
        tag: &str,
        spec: &str,
        first: &str,
    ) -> (
        crate::Store,
        std::path::PathBuf,
        std::path::PathBuf,
        String,
        String,
    ) {
        let (store, dir, repo) = fixture(tag);
        let code = item(&store, "Folded", spec, &[]);
        task(&store, &code, "Build it", TaskState::Completed);
        let branch = branch_for(&store, &code);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_at(&repo, first, "v1\n");
        sign(&store, &code, &repo, "");
        (store, dir, repo, code, branch)
    }

    fn sign(store: &crate::Store, code: &str, repo: &std::path::Path, note: &str) {
        crate::dispatch::dispatch_in(
            store,
            "POST",
            &format!("/projects/demo/features/{code}/signoff/docs-folded"),
            Some(&serde_json::json!({"by": "Ada L", "note": note})),
            Some(repo),
        )
        .unwrap();
        let project = store.load_for_gates("demo").unwrap();
        let signed = project
            .feature(code)
            .unwrap()
            .signoff_current(crate::method::DOCS_FOLDED)
            .cloned()
            .expect("signed");
        assert!(
            !signed.branch_heads.is_empty(),
            "the sign-off pins the branch"
        );
    }

    /// `moved_on` as a caller sees it. The board here has no gate asking for docs-folded, so
    /// `load_for_gates` would leave the spec out; a full load reads the docs the item names.
    fn lapse(store: &crate::Store, code: &str, repo: &std::path::Path) -> Option<String> {
        let project = store.load("demo").unwrap();
        let ctx = Context::load_in(store, "demo", &project.config, Some(repo));
        moved_on(&project, project.feature(code).unwrap(), &ctx)
    }

    /// DEF-072 R-1: commits after the sign-off that change only tests leave it standing.
    #[test]
    fn a_test_only_commit_keeps_docs_folded() {
        let (store, dir, repo, code, _branch) = signed_item("test-only", "", "src/lib.rs");
        assert_eq!(lapse(&store, &code, &repo), None);
        commit_at(&repo, "tests/local_mode.rs", "#[test]\nfn t() {}\n");
        commit_at(&repo, "src/cart.test.ts", "test('x', () => {});\n");
        assert_eq!(lapse(&store, &code, &repo), None);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-072 R-1: a commit to shipped code, a lockfile or a doc the item names lapses it, and the
    /// reason names the file; a doc the item does not name does not.
    #[test]
    fn a_commit_to_shipped_code_or_a_named_doc_lapses_docs_folded() {
        let (store, dir, repo, code, branch) = signed_item("shipped", "", "src/lib.rs");
        commit_at(&repo, "src/lib.rs", "v2\n");
        let why = lapse(&store, &code, &repo).expect("lapsed");
        assert_eq!(
            why,
            format!("{branch} changed src/lib.rs after sign-off 'docs-folded' was given")
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);

        let spec = "## Design\n\n### Affects docs\nRepository docs: `CHANGELOG.md`.\n";
        let (store, dir, repo, code, _branch) = signed_item("named", spec, "CHANGELOG.md");
        commit_at(&repo, "notes/other.md", "named nowhere\n");
        assert_eq!(lapse(&store, &code, &repo), None);
        commit_at(&repo, "Cargo.lock", "# lock\n");
        let why = lapse(&store, &code, &repo).expect("a lockfile ships");
        assert!(why.contains("changed Cargo.lock"), "{why}");
        sign(&store, &code, &repo, "again");
        assert_eq!(lapse(&store, &code, &repo), None);
        commit_at(&repo, "CHANGELOG.md", "v2\n");
        let why = lapse(&store, &code, &repo).expect("a named doc lapses it");
        assert!(why.contains("changed CHANGELOG.md"), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-072 R-2: a branch rewritten after the sign-off lapses it, even when its new commits
    /// change only tests, and the reason says it was rewritten.
    #[test]
    fn a_rewritten_branch_still_lapses_docs_folded() {
        let (store, dir, repo, code, branch) = signed_item("rewritten", "", "src/lib.rs");
        crate::coderepo::git(&repo, &["reset", "-q", "--hard", "main"]).unwrap();
        commit_at(&repo, "tests/a.rs", "#[test]\nfn t() {}\n");
        let why = lapse(&store, &code, &repo).expect("lapsed");
        assert_eq!(
            why,
            format!("{branch} was rewritten after sign-off 'docs-folded' was given")
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    // ---- DEF-105: an item's Refs commits count, merged or not -------------------------------

    /// Write `file` (making its folders) and commit it with `message` on the checked-out branch.
    fn commit_with(repo: &std::path::Path, file: &str, body: &str, message: &str) {
        if let Some(parent) = std::path::Path::new(file).parent() {
            std::fs::create_dir_all(repo.join(parent)).unwrap();
        }
        crate::coderepo::tests::commit_file(repo, file, body, message);
    }

    /// As `commit_with`, with the committer and author date set to `date`.
    fn commit_dated(repo: &std::path::Path, file: &str, message: &str, date: &str) {
        std::fs::write(repo.join(file), "x\n").unwrap();
        crate::coderepo::git(repo, &["add", file]).unwrap();
        let out = std::process::Command::new("git")
            .current_dir(repo)
            .args(["commit", "-q", "-m", message])
            .env("GIT_COMMITTER_DATE", date)
            .env("GIT_AUTHOR_DATE", date)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }

    fn refs(code: &str) -> String {
        format!("work\n\nRefs: kanbanr:{code}/TL-001/T1")
    }

    /// An item whose Design lists `docs`, its task done, as `(store, board dir, repo, code, branch)`.
    fn doc_item(
        tag: &str,
        docs: &[&str],
    ) -> (
        crate::Store,
        std::path::PathBuf,
        std::path::PathBuf,
        String,
        String,
    ) {
        let (store, dir, repo) = fixture(tag);
        let list: Vec<String> = docs.iter().map(|d| format!("`{d}`")).collect();
        let spec = format!(
            "## Design\n\n### Affects docs\nRepository docs: {}.\n",
            list.join(", ")
        );
        let code = item(&store, "Doc item", &spec, &[]);
        task(&store, &code, "Build it", TaskState::Completed);
        let branch = branch_for(&store, &code);
        (store, dir, repo, code, branch)
    }

    fn repo_unbuilt_of(got: &DocsBuilt) -> Vec<&Unbuilt> {
        got.unbuilt
            .iter()
            .filter(|u| u.kind == DocKind::Repository)
            .collect()
    }

    fn merge_branch(repo: &std::path::Path, branch: &str) {
        crate::coderepo::git(repo, &["checkout", "-q", "main"]).unwrap();
        crate::coderepo::merge_no_ff(repo, "main", branch, "Merge the item").unwrap();
    }

    /// DEF-105 R-1: a doc a Refs commit wrote in an earlier wave, merged before the branch's last
    /// merge, reads as built.
    #[test]
    fn a_doc_written_in_an_earlier_merged_wave_reads_as_built() {
        let (store, dir, repo, code, branch) = doc_item("wave", &["CHANGELOG.md"]);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, "CHANGELOG.md", "one\n", &refs(&code));
        merge_branch(&repo, &branch);
        crate::coderepo::git(&repo, &["checkout", "-q", &branch]).unwrap();
        crate::coderepo::git(&repo, &["merge", "-q", "--ff-only", "main"]).unwrap();
        commit_with(&repo, "src.rs", "two\n", "second wave");
        merge_branch(&repo, &branch);
        let got = judge(&store, &code, Some(&repo));
        assert!(repo_unbuilt_of(&got).is_empty(), "{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-105 R-1: a doc a Refs commit wrote on another branch (here one the item's pattern does not
    /// name) reads as built.
    #[test]
    fn a_doc_touched_by_a_refs_commit_on_another_branch_reads_as_built() {
        let (store, dir, repo, code, branch) = doc_item("other-branch", &["CHANGELOG.md"]);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, "src.rs", "x\n", "code");
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", "scratch/wip", "main"]).unwrap();
        commit_with(&repo, "CHANGELOG.md", "x\n", &refs(&code));
        crate::coderepo::git(&repo, &["checkout", "-q", "main"]).unwrap();
        let got = judge(&store, &code, Some(&repo));
        assert!(repo_unbuilt_of(&got).is_empty(), "{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-105 R-1: a commit whose trailer names the item's earlier code counts (FTR-056).
    #[test]
    fn a_commit_under_an_earlier_code_counts_for_the_item() {
        let (store, dir, repo, code, branch) = doc_item("alias", &["CHANGELOG.md"]);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, "CHANGELOG.md", "x\n", &refs("OLD-7"));
        merge_branch(&repo, &branch);
        crate::coderepo::git(&repo, &["checkout", "-q", &branch]).unwrap();
        crate::coderepo::git(&repo, &["merge", "-q", "--ff-only", "main"]).unwrap();
        commit_with(&repo, "src.rs", "two\n", "second wave");
        merge_branch(&repo, &branch);
        let got = judge(&store, &code, Some(&repo));
        assert_eq!(repo_unbuilt_of(&got).len(), 1, "not recoded: {got:?}");
        let mut project = store.load("demo").unwrap();
        project
            .features
            .iter_mut()
            .find(|f| f.code == code)
            .unwrap()
            .aliases = vec!["OLD-7".into()];
        let ctx = Context::load_in(&store, "demo", &project.config, Some(&repo));
        let got = evaluate(&project, project.feature(&code).unwrap(), &ctx);
        assert!(repo_unbuilt_of(&got).is_empty(), "{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-105 R-2: an untrailered commit still on the unmerged branch counts, as under DEF-091.
    #[test]
    fn an_unmerged_commit_without_a_trailer_on_the_branch_still_counts() {
        let (store, dir, repo, code, branch) = doc_item("untrailed", &["CHANGELOG.md"]);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, "CHANGELOG.md", "x\n", "no trailer here");
        let got = judge(&store, &code, Some(&repo));
        assert!(repo_unbuilt_of(&got).is_empty(), "{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-105 R-3: a Refs commit older than the item's build stage does not count.
    #[test]
    fn a_refs_commit_before_the_build_stage_does_not_count() {
        let (store, dir, repo, code, branch) = doc_item("before", &["CHANGELOG.md"]);
        commit_dated(
            &repo,
            "CHANGELOG.md",
            &refs(&code),
            "2020-01-02T03:04:05+00:00",
        );
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, "src.rs", "x\n", "code");
        merge_branch(&repo, &branch);
        let got = judge(&store, &code, Some(&repo));
        let unbuilt = repo_unbuilt_of(&got);
        assert_eq!(unbuilt.len(), 1, "{got:?}");
        assert_eq!(unbuilt[0].path, "CHANGELOG.md");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-105 R-3: another item's commit brought in by a catch-up merge does not count.
    #[test]
    fn another_items_commit_through_a_catch_up_merge_does_not_count() {
        let (store, dir, repo, code, branch) = doc_item("catch-up", &["CHANGELOG.md"]);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, "src.rs", "x\n", &refs(&code));
        crate::coderepo::git(&repo, &["checkout", "-q", "main"]).unwrap();
        commit_with(&repo, "CHANGELOG.md", "x\n", &refs("OTHER-1"));
        crate::coderepo::git(&repo, &["checkout", "-q", &branch]).unwrap();
        crate::coderepo::git(&repo, &["merge", "-q", "--no-ff", "-m", "catch up", "main"]).unwrap();
        let got = judge(&store, &code, Some(&repo));
        let unbuilt = repo_unbuilt_of(&got);
        assert_eq!(unbuilt.len(), 1, "{got:?}");
        assert_eq!(unbuilt[0].path, "CHANGELOG.md");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-105: the build stage is the first move into the build status, else the creation time.
    #[test]
    fn the_build_stage_starts_at_the_first_move_into_the_build_status() {
        let config = crate::ProjectConfig::default_for("demo");
        let status = config.branch_status().expect("a build status");
        let store_dir =
            std::env::temp_dir().join(format!("kanbanr-docsbuilt-bs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&store_dir);
        std::fs::create_dir_all(&store_dir).unwrap();
        let store = crate::Store::new(store_dir.clone());
        store.init_project("demo", config.clone()).unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let f = store.add_feature("demo", "T", "", "M", None).unwrap();
        let mut f = store
            .load("demo")
            .unwrap()
            .feature(&f.code)
            .unwrap()
            .clone();
        assert_eq!(build_started_at(&config, &f), f.created_at);
        let t = |at: &str, to: &str| crate::models::Transition {
            at: at.into(),
            from: "x".into(),
            to: to.into(),
            ..Default::default()
        };
        f.history = vec![
            t("2026-01-01 00:00:00+00:00", "Other"),
            t("2026-02-02 00:00:00+00:00", &status),
            t("2026-03-03 00:00:00+00:00", &status),
        ];
        assert_eq!(build_started_at(&config, &f), "2026-02-02 00:00:00+00:00");
        let _ = std::fs::remove_dir_all(&store_dir);
    }

    // ---- DEF-105: moved docs and names that are no file --------------------------------------

    /// Move `from` to `to` on the checked-out branch and commit it.
    fn move_file(repo: &std::path::Path, from: &str, to: &str) {
        if let Some(parent) = std::path::Path::new(to).parent() {
            std::fs::create_dir_all(repo.join(parent)).unwrap();
        }
        crate::coderepo::git(repo, &["mv", from, to]).unwrap();
        crate::coderepo::git(repo, &["commit", "-q", "-m", "move"]).unwrap();
    }

    /// The item lists `docs/old/x.md`, which main holds at `docs/guide/x.md` since a move; its
    /// branch commits `file` (with the item's trailer), and is merged into main.
    fn moved_doc_item(tag: &str, file: &str) -> DocsBuilt {
        let (store, dir, repo, code, branch) = doc_item(tag, &["docs/old/x.md"]);
        commit_with(&repo, "docs/old/x.md", "v1\n", "add");
        move_file(&repo, "docs/old/x.md", "docs/guide/x.md");
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, file, "v2\n", &refs(&code));
        merge_branch(&repo, &branch);
        let got = judge(&store, &code, Some(&repo));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
        got
    }

    /// DEF-105 R-4: a doc listed under its old path is judged, and shown, at the path it has now.
    #[test]
    fn a_doc_listed_under_its_old_path_is_judged_at_its_new_path() {
        let got = moved_doc_item("moved-built", "docs/guide/x.md");
        assert!(repo_unbuilt_of(&got).is_empty(), "{got:?}");
        let got = moved_doc_item("moved-unbuilt", "src.rs");
        let unbuilt = repo_unbuilt_of(&got);
        assert_eq!(unbuilt.len(), 1, "{got:?}");
        assert_eq!(unbuilt[0].path, "docs/guide/x.md");
        assert!(
            unbuilt[0]
                .why
                .starts_with("docs/guide/x.md (listed as docs/old/x.md) has not changed on"),
            "{got:?}"
        );
    }

    /// DEF-105 R-4: `a -> b -> c` is followed to `c`.
    #[test]
    fn a_chain_of_renames_is_followed_to_the_path_today() {
        let (store, dir, repo, code, branch) = doc_item("chain", &["a.md"]);
        commit_with(&repo, "a.md", "v1\n", "add");
        move_file(&repo, "a.md", "b.md");
        move_file(&repo, "b.md", "c.md");
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, "src.rs", "v2\n", "code");
        merge_branch(&repo, &branch);
        let got = judge(&store, &code, Some(&repo));
        let unbuilt = repo_unbuilt_of(&got);
        assert_eq!(unbuilt.len(), 1, "{got:?}");
        assert_eq!(unbuilt[0].path, "c.md");
        assert!(unbuilt[0].why.contains("(listed as a.md)"), "{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);

        let mut renames = BTreeMap::new();
        renames.insert("a.md".to_string(), "b.md".to_string());
        renames.insert("b.md".to_string(), "c.md".to_string());
        assert_eq!(moved_to(&renames, "a.md").as_deref(), Some("c.md"));
        assert_eq!(moved_to(&renames, "b.md").as_deref(), Some("c.md"));
        assert_eq!(moved_to(&renames, "c.md"), None);
        // A cycle stops instead of looping.
        renames.insert("c.md".to_string(), "a.md".to_string());
        assert!(moved_to(&renames, "a.md").is_some());
    }

    /// DEF-105 R-4: a docs-folded sign-off pins the moved doc under its path today, and a later
    /// change there lapses it.
    #[test]
    fn a_signoff_pins_and_lapses_on_the_moved_path() {
        let (store, dir, repo, code, branch) = doc_item("signoff-moved", &["docs/old/x.md"]);
        commit_with(&repo, "docs/old/x.md", "v1\n", "add");
        move_file(&repo, "docs/old/x.md", "docs/guide/x.md");
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, "docs/guide/x.md", "v2\n", &refs(&code));
        sign(&store, &code, &repo, "");
        let project = store.load("demo").unwrap();
        let feature = project.feature(&code).unwrap();
        let at = crate::picks::resolve(
            &store,
            "demo",
            &repo,
            &crate::picks::Pick::Item(code.clone()),
        )
        .unwrap();
        let pinned = docs_read_at(&project, feature, &repo, &at).unwrap();
        assert_eq!(
            pinned.keys().cloned().collect::<Vec<_>>(),
            vec!["docs/guide/x.md".to_string()]
        );
        assert_eq!(lapse(&store, &code, &repo), None);
        commit_with(&repo, "docs/guide/x.md", "v3\n", "edit the doc");
        let why = lapse(&store, &code, &repo).expect("a change to the moved doc lapses it");
        assert!(why.contains("changed docs/guide/x.md"), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-105 R-5: a listed name that is no file anywhere says so.
    #[test]
    fn a_name_that_is_no_file_says_so() {
        let (store, dir, repo, code, branch) = doc_item("no-file", &["openapi.json"]);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        commit_with(&repo, "src.rs", "x\n", &refs(&code));
        merge_branch(&repo, &branch);
        let got = judge(&store, &code, Some(&repo));
        let unbuilt = repo_unbuilt_of(&got);
        assert_eq!(unbuilt.len(), 1, "{got:?}");
        assert_eq!(unbuilt[0].path, "openapi.json");
        assert_eq!(
            unbuilt[0].why,
            "openapi.json is not a file in the code repository: not written yet, or Affects docs \
             names it wrongly"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-105 R-6: 14 listed docs cost one Refs log, and a rename log only when a name is missing.
    #[test]
    fn the_repository_docs_cost_one_log_and_at_most_one_rename_log() {
        let names: Vec<String> = (0..14).map(|i| format!("docs/guide/d{i}.md")).collect();
        let listed: Vec<&str> = names.iter().map(String::as_str).collect();
        let (store, dir, repo, code, branch) = doc_item("cost-present", &listed);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        for n in &names {
            commit_with(&repo, n, "x\n", &refs(&code));
        }
        merge_branch(&repo, &branch);
        GIT_LOGS.with(|c| c.set((0, 0)));
        let got = judge(&store, &code, Some(&repo));
        assert_eq!(GIT_LOGS.with(|c| c.get()), (1, 0));
        assert!(repo_unbuilt_of(&got).is_empty(), "{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);

        // Three listed under old names: one more log, for the renames.
        let old: Vec<String> = (0..3).map(|i| format!("docs/old/o{i}.md")).collect();
        let mut listed: Vec<&str> = names[..11].iter().map(String::as_str).collect();
        listed.extend(old.iter().map(String::as_str));
        let (store, dir, repo, code, branch) = doc_item("cost-moved", &listed);
        for (i, o) in old.iter().enumerate() {
            commit_with(&repo, o, "x\n", "add");
            move_file(&repo, o, &format!("docs/guide/o{i}.md"));
        }
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        for n in &names[..11] {
            commit_with(&repo, n, "x\n", &refs(&code));
        }
        for i in 0..3 {
            commit_with(&repo, &format!("docs/guide/o{i}.md"), "y\n", &refs(&code));
        }
        merge_branch(&repo, &branch);
        GIT_LOGS.with(|c| c.set((0, 0)));
        let got = judge(&store, &code, Some(&repo));
        assert_eq!(GIT_LOGS.with(|c| c.get()), (1, 1));
        assert!(repo_unbuilt_of(&got).is_empty(), "{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }
}
