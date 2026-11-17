//! A position you pick (FTR-004): `main`, the working tree, an item, a sprint, a release or a
//! commit, turned into the [`crate::position::WorkingState`] a read at that position uses.
//!
//! Named apart from `position.rs` (FTR-003), which holds the position value and the reads at it.
//! Everything here reads; nothing checks out, nothing writes.

use crate::coderepo;
use crate::error::{CoreError, Result};
use crate::position::{self, PositionRef, WorkingState};
use crate::releases::{Release, ReleaseState};
use crate::scm::RELEASE_PREFIX;
use crate::sprints::{Sprint, SprintState};
use crate::store::{Project, Store};
use serde::Serialize;
use std::path::Path;

/// What a person picked to read at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Main,
    WorkingTree,
    /// An item code.
    Item(String),
    /// A sprint code.
    Sprint(String),
    /// A release version.
    Release(String),
    /// A commit (7 to 40 hex digits), or a CD-2 position text.
    Commit(String),
}

impl std::fmt::Display for Pick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pick::Main => f.write_str("main"),
            Pick::WorkingTree => f.write_str("worktree"),
            Pick::Item(c) | Pick::Sprint(c) | Pick::Release(c) | Pick::Commit(c) => f.write_str(c),
        }
    }
}

impl Pick {
    /// The kind a [`Picked`] and a [`PickView`] carry: `main`, `worktree`, `item`, `sprint`,
    /// `release` or `commit`.
    pub fn kind(&self) -> &'static str {
        match self {
            Pick::Main => "main",
            Pick::WorkingTree => "worktree",
            Pick::Item(_) => "item",
            Pick::Sprint(_) => "sprint",
            Pick::Release(_) => "release",
            Pick::Commit(_) => "commit",
        }
    }
}

fn is_short_hex(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Read a pick from its text: the whole string, trimmed, in this order: `main`; `worktree`; a
/// sprint code in `sprints`; a version in `releases`; an item code of `project`; a CD-2 position
/// text; 7 to 40 lower-case hex. Whole-string matches only, never substrings (L-23).
///
/// A position text with a dirty part is taken as [`Pick::Commit`] holding the whole text;
/// [`resolve`] accepts it only when it is the live position of the checkout.
pub fn parse_pick(
    text: &str,
    project: &Project,
    sprints: &[Sprint],
    releases: &[Release],
) -> Result<Pick> {
    let t = text.trim();
    if t == "main" {
        return Ok(Pick::Main);
    }
    if t == "worktree" {
        return Ok(Pick::WorkingTree);
    }
    if !t.is_empty() && sprints.iter().any(|s| s.code == t) {
        return Ok(Pick::Sprint(t.to_string()));
    }
    if !t.is_empty() && releases.iter().any(|r| r.version == t) {
        return Ok(Pick::Release(t.to_string()));
    }
    if !t.is_empty() && project.features.iter().any(|f| f.code == t) {
        return Ok(Pick::Item(t.to_string()));
    }
    if PositionRef::parse(&project.id, t).is_ok() || is_short_hex(t) {
        return Ok(Pick::Commit(t.to_string()));
    }
    Err(CoreError::Unsupported(format!(
        "{t} is not a position: use main, worktree, an item code, a sprint code, a release version or a commit"
    )))
}

/// Where a read at a pick takes its bytes from: git objects at a commit, or the working tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Git,
    Disk,
}

/// A pick resolved to a position (FTR-004). `state.position.commit` is `Some` for every
/// [`Source::Git`] value, and then `state.changed` is empty. Serialises the position as
/// `position`; the changed paths are not sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Picked {
    /// The pick's text, as [`Pick`]'s Display gives it.
    pub pick: String,
    /// `main`, `worktree`, `item`, `sprint`, `release` or `commit`.
    pub kind: &'static str,
    #[serde(rename = "position", serialize_with = "position_only")]
    #[cfg_attr(feature = "schema", schemars(with = "PositionRef"))]
    pub state: WorkingState,
    pub source: Source,
    /// What the banner shows: `FTR-004 · feat/FTR-004-read-docs @ 1a2b3c4`, `main @ 6a973c6`.
    pub label: String,
    /// The rule used: `branch tip`, `merge commit recorded on the item`, `tag` …
    pub resolved_from: String,
}

fn position_only<S: serde::Serializer>(
    state: &WorkingState,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    state.position.serialize(s)
}

fn short(commit: &str) -> String {
    commit.chars().take(7).collect()
}

/// What `resolve` needs from the board and the repository, read once.
struct Ctx<'a> {
    id: &'a str,
    repo: &'a Path,
    project: Project,
    main: String,
}

impl Ctx<'_> {
    fn git(&self, pick: &Pick, commit: String, label: String, from: &str) -> Picked {
        Picked {
            pick: pick.to_string(),
            kind: pick.kind(),
            state: WorkingState {
                position: PositionRef::at_commit(self.id, &commit),
                changed: vec![],
            },
            source: Source::Git,
            label,
            resolved_from: from.to_string(),
        }
    }

    fn tip(&self, branch: &str) -> Option<String> {
        coderepo::branch_exists(self.repo, branch)
            .then(|| coderepo::rev_parse(self.repo, branch).ok())
            .flatten()
    }

    fn live(&self) -> Result<WorkingState> {
        position::read_position(self.repo, self.id)
    }
}

/// Resolve `pick` for the board project `id` against the code repository `repo` (FTR-004). Every
/// pick but the working tree is a clean position at a commit, read from git objects; the working
/// tree is the live position, read from disk.
pub fn resolve(store: &Store, id: &str, repo: &Path, pick: &Pick) -> Result<Picked> {
    let project = store.load_meta(id)?;
    let main = crate::gitmodel::main_of(&project.config, repo);
    let cx = Ctx {
        id,
        repo,
        project,
        main,
    };
    match pick {
        Pick::Main => {
            let commit = coderepo::rev_parse(repo, &cx.main).map_err(|_| {
                CoreError::Unsupported(format!("{} has no commit in this repository yet", cx.main))
            })?;
            let label = format!("{} @ {}", cx.main, short(&commit));
            Ok(cx.git(pick, commit, label, "branch tip"))
        }
        Pick::WorkingTree => Ok(working_tree(pick.to_string(), cx.live()?)),
        Pick::Item(code) => resolve_item(&cx, pick, code),
        Pick::Sprint(code) => resolve_sprint(store, &cx, pick, code),
        Pick::Release(v) => resolve_release(store, &cx, pick, v),
        Pick::Commit(text) => resolve_commit(&cx, pick, text),
    }
}

/// The working tree, read from disk. `pick` is `worktree`, or the live position text it was asked
/// for by.
fn working_tree(pick: String, state: WorkingState) -> Picked {
    Picked {
        pick,
        kind: Pick::WorkingTree.kind(),
        label: format!("working tree @ {}", state.position.short()),
        state,
        source: Source::Disk,
        resolved_from: "HEAD and the uncommitted changes".to_string(),
    }
}

fn resolve_item(cx: &Ctx<'_>, pick: &Pick, code: &str) -> Result<Picked> {
    let feature = cx.project.feature(code)?;
    if let Some(sha) = feature.merge_commit() {
        let label = format!("{code} · merged @ {}", short(sha));
        return Ok(cx.git(
            pick,
            sha.to_string(),
            label,
            "merge commit recorded on the item",
        ));
    }
    let facts = coderepo::branch_facts(cx.repo, &cx.main)?;
    let branches = crate::gitmodel::item_branches(&cx.project.config, &facts, code);
    match branches.as_slice() {
        [one] => {
            let tip = facts.branches[one].clone();
            let label = format!("{code} · {one} @ {}", short(&tip));
            return Ok(cx.git(pick, tip, label, "branch tip"));
        }
        [] => {}
        many => {
            return Err(CoreError::Unsupported(format!(
                "{code} has more than one branch ({}): keep one to read at it",
                many.join(", ")
            )));
        }
    }
    if let Some(sha) = coderepo::merge_commit_for(cx.repo, &cx.main, code)? {
        let label = format!("{code} · merged @ {}", short(&sha));
        return Ok(cx.git(pick, sha, label, "merge found on main by its Refs trailer"));
    }
    Err(CoreError::Unsupported(format!(
        "{code} has no branch and no recorded merge in this repository"
    )))
}

fn resolve_sprint(store: &Store, cx: &Ctx<'_>, pick: &Pick, code: &str) -> Result<Picked> {
    let sprints = crate::sprints::load(store, cx.id)?;
    let sprint = sprints
        .iter()
        .find(|s| s.code == code)
        .ok_or_else(|| CoreError::Unsupported(format!("no sprint {code}")))?;
    let branch = crate::gitmodel::sprint_branch(code);
    if sprint.state == SprintState::Closed {
        let recorded = sprint.closed_commit.trim();
        if !recorded.is_empty() {
            let label = format!("{code} · closed @ {}", short(recorded));
            return Ok(cx.git(
                pick,
                recorded.to_string(),
                label,
                "closing commit recorded on the sprint",
            ));
        }
        if let Some(sha) = coderepo::tag_commit(cx.repo, code) {
            let label = format!("{code} · tag {code} @ {}", short(&sha));
            return Ok(cx.git(pick, sha, label, "tag"));
        }
        if let Some(tip) = cx.tip(&branch) {
            let label = format!("{code} · {branch} @ {}", short(&tip));
            return Ok(cx.git(pick, tip, label, "branch tip"));
        }
        return Err(CoreError::Unsupported(format!(
            "sprint {code} has no closing commit, tag or branch in this repository"
        )));
    }
    match cx.tip(&branch) {
        Some(tip) => {
            let label = format!("{code} · {branch} @ {}", short(&tip));
            Ok(cx.git(pick, tip, label, "branch tip"))
        }
        None => Err(CoreError::Unsupported(format!(
            "sprint {code} has no branch yet"
        ))),
    }
}

fn resolve_release(store: &Store, cx: &Ctx<'_>, pick: &Pick, v: &str) -> Result<Picked> {
    let releases = crate::releases::load(store, cx.id)?;
    let release = releases
        .iter()
        .find(|r| r.version == v)
        .ok_or_else(|| CoreError::Unsupported(format!("no release {v}")))?;
    let recorded = release.tag_commit.trim();
    if !recorded.is_empty() {
        let label = format!("{v} · tag {v} @ {}", short(recorded));
        return Ok(cx.git(
            pick,
            recorded.to_string(),
            label,
            "tag commit recorded on the release",
        ));
    }
    if let Some(sha) = coderepo::tag_commit(cx.repo, v) {
        let label = format!("{v} · tag {v} @ {}", short(&sha));
        return Ok(cx.git(pick, sha, label, "tag"));
    }
    let branch = if release.branch.trim().is_empty() {
        format!("{RELEASE_PREFIX}{v}")
    } else {
        release.branch.trim().to_string()
    };
    if let Some(tip) = cx.tip(&branch) {
        let label = format!("{v} · {branch} @ {}", short(&tip));
        return Ok(cx.git(pick, tip, label, "branch tip"));
    }
    Err(CoreError::Unsupported(format!(
        "{v} has no branch or tag yet"
    )))
}

fn resolve_commit(cx: &Ctx<'_>, pick: &Pick, text: &str) -> Result<Picked> {
    if let Ok(wanted) = PositionRef::parse(cx.id, text)
        && !wanted.is_clean()
    {
        let live = cx.live()?;
        if live.position == wanted {
            return Ok(working_tree(text.to_string(), live));
        }
        return Err(CoreError::Unsupported(format!(
            "{text} is not the working tree's position: it is at {}",
            live.position.text()
        )));
    }
    match coderepo::rev_parse(cx.repo, text) {
        Ok(commit) => {
            let label = format!("commit @ {}", short(&commit));
            Ok(cx.git(pick, commit, label, "commit"))
        }
        Err(_) => Err(CoreError::PositionUnavailable {
            wanted: text.to_string(),
            available: vec![cx.live()?.position],
        }),
    }
}

/// One entry of the position picker (FTR-004): whether it resolves here, and why not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PickView {
    pub pick: String,
    pub kind: &'static str,
    pub label: String,
    pub available: bool,
    /// Why it is not available; empty when it is.
    pub why: String,
}

/// An error's words without the kind's prefix, as the picker shows them.
fn reason(e: &CoreError) -> String {
    match e {
        CoreError::Unsupported(m) => m.clone(),
        other => other.to_string(),
    }
}

fn view(store: &Store, id: &str, repo: &Path, pick: Pick) -> PickView {
    match resolve(store, id, repo, &pick) {
        Ok(at) => PickView {
            pick: at.pick,
            kind: at.kind,
            label: at.label,
            available: true,
            why: String::new(),
        },
        Err(e) => PickView {
            pick: pick.to_string(),
            kind: pick.kind(),
            label: pick.to_string(),
            available: false,
            why: reason(&e),
        },
    }
}

/// What the position picker lists (FTR-004): `main`, `worktree`, each sprint (running first, then
/// closed, newest first), each release (unshipped first, newest first). Items are not listed: the
/// picker takes a typed code, and item pages pass their own.
pub fn picks(store: &Store, id: &str, repo: &Path) -> Vec<PickView> {
    let mut out = vec![
        view(store, id, repo, Pick::Main),
        view(store, id, repo, Pick::WorkingTree),
    ];
    let mut sprints = crate::sprints::load(store, id).unwrap_or_default();
    sprints.reverse();
    let (closed, open): (Vec<_>, Vec<_>) = sprints
        .into_iter()
        .partition(|s| s.state == SprintState::Closed);
    for s in open.into_iter().chain(closed) {
        out.push(view(store, id, repo, Pick::Sprint(s.code)));
    }
    let mut releases = crate::releases::load(store, id).unwrap_or_default();
    releases.reverse();
    let (shipped, unshipped): (Vec<_>, Vec<_>) = releases
        .into_iter()
        .partition(|r| r.state == ReleaseState::Shipped);
    for r in unshipped.into_iter().chain(shipped) {
        out.push(view(store, id, repo, Pick::Release(r.version)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coderepo::git;
    use crate::coderepo::tests::{commit_file, scratch_repo};
    use crate::config::ProjectConfig;
    use std::path::PathBuf;

    /// A scratch board (project `demo`, one milestone) under the temp dir, never the real board
    /// (L-41), and a scratch repository on `main`.
    fn board(tag: &str) -> (Store, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kanbanr-picks-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        assert!(store.data_dir().starts_with(std::env::temp_dir()));
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let repo = scratch_repo(&format!("picks-{tag}"));
        assert!(repo.starts_with(std::env::temp_dir()));
        (store, dir, repo)
    }

    fn item(store: &Store, title: &str) -> String {
        store
            .add_feature("demo", title, "# x", "M", None)
            .unwrap()
            .code
    }

    fn write_sprints(store: &Store, sprints: &[Sprint]) {
        std::fs::write(
            store.project_dir("demo").join("sprints.yaml"),
            crate::yaml::to_string(sprints).unwrap(),
        )
        .unwrap();
    }

    fn write_releases(store: &Store, releases: &[Release]) {
        std::fs::write(
            store.project_dir("demo").join("releases.yaml"),
            crate::yaml::to_string(releases).unwrap(),
        )
        .unwrap();
    }

    fn on_branch(repo: &Path, branch: &str, file: &str, text: &str) -> String {
        git(repo, &["checkout", "-q", "-b", branch, "main"]).unwrap();
        commit_file(repo, file, text, "work");
        let tip = coderepo::rev_parse(repo, "HEAD").unwrap();
        git(repo, &["checkout", "-q", "main"]).unwrap();
        tip
    }

    fn pick_of(store: &Store, text: &str) -> Result<Pick> {
        let project = store.load_meta("demo").unwrap();
        let sprints = crate::sprints::load(store, "demo").unwrap();
        let releases = crate::releases::load(store, "demo").unwrap();
        parse_pick(text, &project, &sprints, &releases)
    }

    fn commit_of(p: &Picked) -> &str {
        p.state.position.commit.as_deref().unwrap()
    }

    /// FTR-004 R-1: an item resolves to the merge commit recorded on it; else to its one branch's
    /// tip; else, on an older board with nothing recorded, to the merge on main its Refs trailer
    /// names.
    #[test]
    fn an_item_resolves_to_its_merge_commit_then_its_branch_tip() {
        let (store, dir, repo) = board("item");
        let code = item(&store, "Read docs");
        // In progress: its branch's tip.
        let branch = format!("feat/{code}-read-docs");
        let tip = on_branch(&repo, &branch, "docs.md", "branch\n");
        let pick = pick_of(&store, &code).unwrap();
        assert_eq!(pick, Pick::Item(code.clone()));
        let at = resolve(&store, "demo", &repo, &pick).unwrap();
        assert_eq!(commit_of(&at), tip);
        assert_eq!(at.source, Source::Git);
        assert!(at.state.changed.is_empty());
        assert_eq!(at.resolved_from, "branch tip");
        assert_eq!(at.label, format!("{code} · {branch} @ {}", &tip[..7]));
        assert_eq!(at.kind, "item");

        // Merged on main by an older kanbanr, branch gone, nothing recorded: the Refs trailer.
        let (_, merge) = coderepo::merge_no_ff(
            &repo,
            "main",
            &branch,
            &format!("Merge {branch} into main\n\nRefs: kanbanr:{code}"),
        )
        .unwrap();
        git(&repo, &["branch", "-q", "-D", &branch]).unwrap();
        let at = resolve(&store, "demo", &repo, &pick).unwrap();
        assert_eq!(commit_of(&at), merge);
        assert_eq!(at.resolved_from, "merge found on main by its Refs trailer");

        // Recorded on the item: that wins, even over a branch of the same item.
        let recorded = on_branch(&repo, "elsewhere", "e.md", "e\n");
        store
            .set_feature_definition(
                "demo",
                &code,
                Some(crate::models::FeatureDefinition {
                    statement: "s".into(),
                    ..Default::default()
                }),
            )
            .unwrap();
        store.approve_feature("demo", &code, "Ada").unwrap();
        store.move_feature("demo", &code, "Scheduled").unwrap();
        store
            .record_merge_commit("demo", &code, "Scheduled", &recorded)
            .unwrap();
        on_branch(&repo, &format!("feat/{code}-again"), "x.md", "x\n");
        let at = resolve(&store, "demo", &repo, &pick).unwrap();
        assert_eq!(commit_of(&at), recorded);
        assert_eq!(at.resolved_from, "merge commit recorded on the item");
        assert_eq!(at.label, format!("{code} · merged @ {}", &recorded[..7]));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-004 R-1: a closed sprint resolves to its closing commit, then its tag, then its branch;
    /// a running one to its branch's tip.
    #[test]
    fn a_closed_sprint_resolves_to_its_closing_commit_and_a_running_one_to_its_branch() {
        let (store, dir, repo) = board("sprint");
        let one = on_branch(&repo, "sprint/SP-001", "a.md", "one\n");
        let two = on_branch(&repo, "sprint/SP-002", "a.md", "two\n");
        let closing = on_branch(&repo, "closing", "a.md", "closing\n");
        let sprint = |code: &str, state: SprintState, closed_commit: &str| Sprint {
            code: code.into(),
            start: "2026-01-01".into(),
            end: "2026-01-14".into(),
            state,
            closed_commit: closed_commit.into(),
            ..Default::default()
        };
        write_sprints(
            &store,
            &[
                sprint("SP-001", SprintState::Closed, &closing),
                sprint("SP-002", SprintState::Active, ""),
            ],
        );
        let p1 = pick_of(&store, "SP-001").unwrap();
        assert_eq!(p1, Pick::Sprint("SP-001".into()));
        let at = resolve(&store, "demo", &repo, &p1).unwrap();
        assert_eq!(commit_of(&at), closing);
        assert_eq!(at.resolved_from, "closing commit recorded on the sprint");
        assert_eq!(at.label, format!("SP-001 · closed @ {}", &closing[..7]));
        let at = resolve(&store, "demo", &repo, &Pick::Sprint("SP-002".into())).unwrap();
        assert_eq!(commit_of(&at), two);
        assert_eq!(at.resolved_from, "branch tip");

        // An older binary dropped closed_commit: the tag, then the branch.
        write_sprints(
            &store,
            &[
                sprint("SP-001", SprintState::Closed, ""),
                sprint("SP-002", SprintState::Active, ""),
            ],
        );
        let at = resolve(&store, "demo", &repo, &p1).unwrap();
        assert_eq!(commit_of(&at), one);
        assert_eq!(at.resolved_from, "branch tip");
        coderepo::create_tag(&repo, "SP-001", &closing, "SP-001").unwrap();
        let at = resolve(&store, "demo", &repo, &p1).unwrap();
        assert_eq!(commit_of(&at), closing);
        assert_eq!(at.resolved_from, "tag");

        // The picker lists the running sprint first.
        let listed: Vec<String> = picks(&store, "demo", &repo)
            .into_iter()
            .map(|v| v.pick)
            .collect();
        assert_eq!(listed, ["main", "worktree", "SP-002", "SP-001"]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-004 R-1: a release resolves to the tag commit recorded on it, then its tag, then its
    /// branch.
    #[test]
    fn a_release_resolves_to_its_tag_commit_or_its_branch() {
        let (store, dir, repo) = board("release");
        let branch_tip = on_branch(&repo, "release/v0.2.0", "a.md", "rel\n");
        let tagged = on_branch(&repo, "tagged", "a.md", "tagged\n");
        let recorded = on_branch(&repo, "recorded", "a.md", "recorded\n");
        let release = |tag_commit: &str| Release {
            version: "v0.2.0".into(),
            tag_commit: tag_commit.into(),
            ..Default::default()
        };
        write_releases(&store, &[release("")]);
        let pick = pick_of(&store, "v0.2.0").unwrap();
        assert_eq!(pick, Pick::Release("v0.2.0".into()));
        let at = resolve(&store, "demo", &repo, &pick).unwrap();
        assert_eq!(commit_of(&at), branch_tip);
        assert_eq!(at.resolved_from, "branch tip");
        assert_eq!(
            at.label,
            format!("v0.2.0 · release/v0.2.0 @ {}", &branch_tip[..7])
        );
        coderepo::create_tag(&repo, "v0.2.0", &tagged, "v0.2.0").unwrap();
        let at = resolve(&store, "demo", &repo, &pick).unwrap();
        assert_eq!(commit_of(&at), tagged);
        assert_eq!(at.resolved_from, "tag");
        assert_eq!(at.label, format!("v0.2.0 · tag v0.2.0 @ {}", &tagged[..7]));
        write_releases(&store, &[release(&recorded)]);
        let at = resolve(&store, "demo", &repo, &pick).unwrap();
        assert_eq!(commit_of(&at), recorded);
        assert_eq!(at.resolved_from, "tag commit recorded on the release");
        assert_eq!(at.kind, "release");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-004 R-1: main resolves to its tip from git; the working tree to the live position from
    /// disk, carrying the dirty hash; a commit and the live position text resolve too.
    #[test]
    fn main_and_the_working_tree_resolve_and_the_working_tree_carries_the_dirty_hash() {
        let (store, dir, repo) = board("main");
        let main = coderepo::rev_parse(&repo, "main").unwrap();
        assert_eq!(pick_of(&store, "main").unwrap(), Pick::Main);
        let at = resolve(&store, "demo", &repo, &Pick::Main).unwrap();
        assert_eq!(commit_of(&at), main);
        assert_eq!(at.source, Source::Git);
        assert_eq!(at.label, format!("main @ {}", &main[..7]));
        assert_eq!(at.state.position.repo, "demo");
        let json = serde_json::to_value(&at).unwrap();
        assert_eq!(json["position"]["commit"], serde_json::json!(main));
        assert_eq!(json["source"], "git");
        assert!(json.get("changed").is_none() && json.get("state").is_none());

        // Clean working tree: HEAD, from disk.
        assert_eq!(pick_of(&store, " worktree ").unwrap(), Pick::WorkingTree);
        let at = resolve(&store, "demo", &repo, &Pick::WorkingTree).unwrap();
        assert_eq!(at.source, Source::Disk);
        assert!(at.state.position.is_clean());
        // Uncommitted: the dirty hash rides along.
        std::fs::write(repo.join("README"), "edited\n").unwrap();
        let at = resolve(&store, "demo", &repo, &Pick::WorkingTree).unwrap();
        let live = position::read_position(&repo, "demo").unwrap();
        assert_eq!(at.state.position, live.position);
        assert!(at.state.position.dirty.is_some());
        assert_eq!(at.state.changed, live.changed);
        assert_eq!(
            at.label,
            format!("working tree @ {}", live.position.short())
        );
        assert_eq!(at.resolved_from, "HEAD and the uncommitted changes");
        assert_eq!(serde_json::to_value(&at).unwrap()["source"], "disk");

        // The live position text is the working tree; a short commit is that commit.
        let text = live.position.text();
        let p = pick_of(&store, &text).unwrap();
        assert_eq!(p, Pick::Commit(text.clone()));
        let at = resolve(&store, "demo", &repo, &p).unwrap();
        assert_eq!(at.source, Source::Disk);
        assert_eq!(at.state.position, live.position);
        let p = pick_of(&store, &main[..9]).unwrap();
        assert_eq!(p, Pick::Commit(main[..9].to_string()));
        let at = resolve(&store, "demo", &repo, &p).unwrap();
        assert_eq!(commit_of(&at), main);
        assert_eq!(at.kind, "commit");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// FTR-004 R-1: what cannot resolve says why, in the words the design gives; a commit the
    /// repository lacks is PositionUnavailable naming the live position.
    #[test]
    fn an_unresolvable_pick_says_why() {
        let (store, dir, repo) = board("why");
        let code = item(&store, "No branch");
        let err = |p: &Pick| reason(&resolve(&store, "demo", &repo, p).unwrap_err());
        assert_eq!(
            err(&Pick::Item(code.clone())),
            format!("{code} has no branch and no recorded merge in this repository")
        );
        on_branch(&repo, &format!("feat/{code}-one"), "a", "a\n");
        on_branch(&repo, &format!("fix/{code}-two"), "b", "b\n");
        let many = err(&Pick::Item(code.clone()));
        assert!(many.contains("more than one branch"), "{many}");
        assert!(many.contains(&format!("feat/{code}-one")), "{many}");

        write_sprints(
            &store,
            &[Sprint {
                code: "SP-001".into(),
                start: "2026-01-01".into(),
                end: "2026-01-14".into(),
                state: SprintState::Active,
                ..Default::default()
            }],
        );
        assert_eq!(
            err(&Pick::Sprint("SP-001".into())),
            "sprint SP-001 has no branch yet"
        );
        write_releases(
            &store,
            &[Release {
                version: "v9.0.0".into(),
                ..Default::default()
            }],
        );
        assert_eq!(
            err(&Pick::Release("v9.0.0".into())),
            "v9.0.0 has no branch or tag yet"
        );

        let missing = "0123456789abcdef0123456789abcdef01234567";
        let live = position::read_position(&repo, "demo").unwrap();
        match resolve(&store, "demo", &repo, &Pick::Commit(missing.into())).unwrap_err() {
            CoreError::PositionUnavailable { wanted, available } => {
                assert_eq!(wanted, missing);
                assert_eq!(available, vec![live.position.clone()]);
            }
            other => panic!("{other:?}"),
        }
        // A dirty position text that is not the live one.
        let dirty = format!("{missing}+{}", "a".repeat(64));
        let e = err(&pick_of(&store, &dirty).unwrap());
        assert!(e.contains(&live.position.text()), "{e}");

        // Not a pick at all: whole-string matches only (L-23).
        for bad in ["mainline", "SP-0011", "abc12", "FTR-004x", "ABCDEF0123", ""] {
            let e = reason(&pick_of(&store, bad).unwrap_err());
            assert!(
                e.ends_with(
                    "is not a position: use main, worktree, an item code, a sprint code, a release version or a commit"
                ),
                "{bad}: {e}"
            );
        }
        // The picker says why an entry is not available.
        let views = picks(&store, "demo", &repo);
        let sp = views.iter().find(|v| v.pick == "SP-001").unwrap();
        assert!(!sp.available && sp.why == "sprint SP-001 has no branch yet");
        assert!(views.iter().find(|v| v.pick == "main").unwrap().available);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }
}
