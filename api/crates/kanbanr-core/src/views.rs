//! Views at a position (SPK-001, taken over by FTR-003): one generated view, the markdown docs
//! under a folder at a position, in the shape of katalogr's `ViewAnswer`, and the proof that it
//! repeats.

use crate::diagrams;
use crate::error::{CoreError, Result};
use crate::hash::stable_hash;
use crate::position::{
    PositionRef, ViewQuery, WorkingState, blob_id, read_at, read_position, resolve_position,
    tree_at, view_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const VIEW_SCHEMA: u32 = 1;

/// The facts part F of a kanbanr view ID (CD-3): SHA-256 hex of `<board commit>\0<facts revision>`.
/// The facts revision is "" until FTR-005's facts record exists.
pub fn facts_version(board_commit: &str, facts_rev: &str) -> String {
    let mut h = Sha256::new();
    h.update(board_commit.as_bytes());
    h.update([0u8]);
    h.update(facts_rev.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The facts part for a project: the board's HEAD commit (or "none") and the project's facts
/// revision, through [`facts_version`]. FTR-005's facts record does not exist in this build, so the
/// revision is "" (the Design's rule while the record is absent); when it lands, `facts_rev` reads
/// `Facts::content_rev()` here and nothing else changes.
pub fn project_facts(store: &crate::Store, _project: &str) -> String {
    let board = crate::git::head_commit(store.data_dir());
    let facts_rev = "";
    facts_version(board.as_deref().unwrap_or("none"), facts_rev)
}

/// The one text for a repository answered from an imported snapshot with no checkout here
/// (katalogr FEAT-097): its position is the snapshot's commit and uncommitted changes cannot be
/// seen. FTR-016 prefixes it "katalogr: " when it forwards a katalogr answer.
pub fn snapshot_warning(repo: &str, commit: &str) -> String {
    format!(
        "{repo} has no checkout on this machine: its position is the imported snapshot's commit \
         {commit}; uncommitted changes there cannot be seen"
    )
}

/// What a view answers with: the same shape as katalogr's ViewAnswer, so one renderer draws both.
/// Field order is the wire order; no `serde_json::Value` is in it (its key order would follow the
/// build's `preserve_order` feature and break "same bytes").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ViewAnswer {
    pub schema: u32,
    pub view_format: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    pub exact: bool,
    pub positions: Vec<PositionRef>,
    pub query: ViewQueryJson,
    pub mermaid: String,
    pub graph: ViewGraph,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ViewQueryJson {
    pub op: String,
    pub args: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ViewGraph {
    pub edges: Vec<GraphEdge>,
    pub nodes: Vec<GraphNode>,
    pub shown: usize,
    pub title: String,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GraphNode {
    pub id: String,
    /// "folder" or "doc".
    pub kind: String,
    pub label: String,
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagrams: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GraphEdge {
    pub from: String,
    /// "holds".
    pub kind: String,
    pub to: String,
}

/// One shown markdown file: path, blob id, size and Mermaid block count.
struct Doc {
    path: String,
    blob: String,
    bytes: u64,
    diagrams: usize,
}

fn label_text(s: &str) -> String {
    s.replace('"', "#quot;").replace(['\n', '\r'], " ")
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

fn name_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, n)| n)
}

/// The answer for `docs` already read: pure, so a test needs no repository.
fn render(
    position: &PositionRef,
    facts: &str,
    docs_root: &str,
    max_nodes: usize,
    total: usize,
    docs: &[Doc],
) -> ViewAnswer {
    let title = format!("docs --max-nodes {max_nodes} --root {docs_root}");
    let folder_label = |f: &str| format!("{f}/");
    let mut folders: BTreeSet<&str> = docs.iter().map(|d| parent_of(&d.path)).collect();
    folders.insert(docs_root);
    let mut nodes: Vec<GraphNode> = Vec::new();
    for f in &folders {
        nodes.push(GraphNode {
            id: format!("d{}", stable_hash(f)),
            kind: "folder".into(),
            label: folder_label(f),
            name: name_of(f).to_string(),
            path: (*f).to_string(),
            blob: None,
            bytes: None,
            diagrams: None,
        });
    }
    for d in docs {
        let name = name_of(&d.path);
        let label = match d.diagrams {
            0 => name.to_string(),
            1 => format!("{name} (1 diagram)"),
            n => format!("{name} ({n} diagrams)"),
        };
        nodes.push(GraphNode {
            id: format!("n{}", stable_hash(&d.path)),
            kind: "doc".into(),
            label,
            name: name.to_string(),
            path: d.path.clone(),
            blob: Some(d.blob.clone()),
            bytes: Some(d.bytes),
            diagrams: Some(d.diagrams),
        });
    }
    nodes.sort_by(|a, b| (&a.path, &a.kind).cmp(&(&b.path, &b.kind)));
    let mut edges = Vec::new();
    for n in nodes.iter().filter(|n| n.kind == "doc") {
        edges.push(GraphEdge {
            from: format!("d{}", stable_hash(parent_of(&n.path))),
            kind: "holds".into(),
            to: n.id.clone(),
        });
    }
    let mut mermaid = format!(
        "flowchart LR\n%% kanbanr view: {}\n",
        title.replace(['\n', '\r'], " ")
    );
    for n in &nodes {
        mermaid.push_str(&format!("  {}[\"{}\"]\n", n.id, label_text(&n.label)));
    }
    for e in &edges {
        mermaid.push_str(&format!("  {} --> {}\n", e.from, e.to));
    }
    let query = ViewQuery {
        op: "docs".into(),
        args: BTreeMap::from([
            ("max-nodes".to_string(), max_nodes.to_string()),
            ("root".to_string(), docs_root.to_string()),
        ]),
    };
    let positions = vec![position.clone()];
    ViewAnswer {
        schema: VIEW_SCHEMA,
        view_format: crate::position::VIEW_FORMAT,
        view_id: Some(view_id(&positions, facts, &query)),
        exact: true,
        positions,
        query: ViewQueryJson {
            op: query.op,
            args: query.args,
        },
        mermaid,
        graph: ViewGraph {
            edges,
            nodes,
            shown: docs.len(),
            title,
            total,
        },
        warnings: vec![],
    }
}

/// The spike's view: the markdown docs under `docs_root` at `state`'s position. Every byte comes
/// through [`read_at`], so an edit during the read is an error, never a silent mix.
pub fn docs_view(
    root: &Path,
    state: &WorkingState,
    facts: &str,
    docs_root: &str,
    max_nodes: usize,
) -> Result<ViewAnswer> {
    let docs_root = docs_root.trim_matches('/');
    let files: Vec<(String, String)> = tree_at(root, state, docs_root)?
        .into_iter()
        .filter(|(p, _)| p.ends_with(".md"))
        .collect();
    let total = files.len();
    let mut docs = Vec::new();
    for (path, blob) in files.into_iter().take(max_nodes) {
        let Some(bytes) = read_at(root, state, &path)? else {
            return Err(crate::error::CoreError::Git(format!(
                "position moved: {path} is gone since the position was read"
            )));
        };
        let diagrams = diagrams::blocks(&path, None, &String::from_utf8_lossy(&bytes)).len();
        docs.push(Doc {
            path,
            blob,
            bytes: bytes.len() as u64,
            diagrams,
        });
    }
    Ok(render(
        &state.position,
        facts,
        docs_root,
        max_nodes,
        total,
        &docs,
    ))
}

/// A view at the live checkout, or at `at` (a position text, see [`resolve_position`]): reads the
/// position, builds the view, and if an uncommitted file changed under it reads the position once
/// more and tries again. After that it answers `exact: false`, no `view_id`, the last position
/// read, and the warning [`MOVED_WARNING`].
pub fn docs_view_live(
    root: &Path,
    repo: &str,
    facts: &str,
    at: Option<&str>,
    docs_root: &str,
    max_nodes: usize,
) -> Result<ViewAnswer> {
    docs_view_hooked(root, repo, facts, at, docs_root, max_nodes, &mut |_| {})
}

/// What a not-exact answer says.
pub const MOVED_WARNING: &str = "the checkout changed while the view was read; try again";

/// [`docs_view_live`] with a hook (given the attempt number, 0 or 1) that runs between the
/// position read and the file reads: the test's way to edit the checkout in the middle.
fn docs_view_hooked(
    root: &Path,
    repo: &str,
    facts: &str,
    at: Option<&str>,
    docs_root: &str,
    max_nodes: usize,
    between: &mut dyn FnMut(u32),
) -> Result<ViewAnswer> {
    let mut last = None;
    for attempt in 0..2 {
        let resolved = resolve_position(root, repo, at)?;
        between(attempt);
        match docs_view(root, resolved.state(), facts, docs_root, max_nodes) {
            Ok(answer) => return Ok(answer),
            Err(CoreError::Git(why)) if why.starts_with("position moved") => {
                last = Some(resolved.state().position.clone());
            }
            Err(e) => return Err(e),
        }
    }
    let position = last.unwrap_or_else(|| PositionRef::at_commit(repo, ""));
    let mut answer = render(
        &position,
        facts,
        docs_root.trim_matches('/'),
        max_nodes,
        0,
        &[],
    );
    answer.exact = false;
    answer.view_id = None;
    answer.warnings.push(MOVED_WARNING.to_string());
    Ok(answer)
}

/// The bytes a view is printed, stored and compared as: compact JSON of `ViewAnswer` in field
/// order.
pub fn answer_bytes(a: &ViewAnswer) -> Vec<u8> {
    serde_json::to_vec(a).unwrap_or_default()
}

/// The state at `at`: a branch or commit name as a clean position (nothing uncommitted), or None
/// for the live checkout.
pub fn state_at(root: &Path, repo: &str, at: Option<&str>) -> Result<WorkingState> {
    match at {
        Some(name) => Ok(WorkingState {
            position: PositionRef::at_commit(repo, &crate::coderepo::rev_parse(root, name)?),
            changed: vec![],
        }),
        None => read_position(root, repo),
    }
}

/// One row of the proof: a position, the written doc's blob there, the view's ID, and whether a
/// second read at that position repeated the bytes and the ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofRow {
    pub at: String,
    pub position: String,
    pub doc_blob: Option<String>,
    pub view_id: String,
    pub bytes_repeat: bool,
    pub id_repeat: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofTable {
    pub rows: Vec<ProofRow>,
    /// Every pair repeated and the IDs are pairwise distinct.
    pub ok: bool,
}

/// The proof (R-5) as a library function so a test can run it: for each `at` (a branch or commit
/// name, or None for the live checkout) the doc's blob id, the view's ID and whether a second
/// read at that position repeated the bytes and the ID.
pub fn proof(
    root: &Path,
    repo: &str,
    facts: &str,
    doc: &str,
    ats: &[Option<&str>],
    docs_root: &str,
    max_nodes: usize,
) -> Result<ProofTable> {
    proof_with(
        root,
        repo,
        facts,
        doc,
        ats,
        docs_root,
        max_nodes,
        &mut |_| {},
    )
}

/// [`proof`] with a hook that runs between the two reads of each position (the test's way to edit
/// the checkout in the middle).
#[allow(clippy::too_many_arguments)]
fn proof_with(
    root: &Path,
    repo: &str,
    facts: &str,
    doc: &str,
    ats: &[Option<&str>],
    docs_root: &str,
    max_nodes: usize,
    between: &mut dyn FnMut(Option<&str>),
) -> Result<ProofTable> {
    let mut rows = Vec::new();
    for at in ats {
        let first = state_at(root, repo, *at)?;
        let a = docs_view(root, &first, facts, docs_root, max_nodes)?;
        let doc_blob = read_at(root, &first, doc)?.map(|b| blob_id(&b));
        between(*at);
        let second = state_at(root, repo, *at)?;
        let b = docs_view(root, &second, facts, docs_root, max_nodes)?;
        rows.push(ProofRow {
            at: at.unwrap_or("working tree").to_string(),
            position: first.position.text(),
            doc_blob,
            view_id: a.view_id.clone().unwrap_or_default(),
            bytes_repeat: answer_bytes(&a) == answer_bytes(&b),
            id_repeat: a.view_id == b.view_id,
        });
    }
    let ids: BTreeSet<&str> = rows.iter().map(|r| r.view_id.as_str()).collect();
    let ok = rows.iter().all(|r| r.bytes_repeat && r.id_repeat) && ids.len() == rows.len();
    Ok(ProofTable { rows, ok })
}

impl ProofTable {
    /// The table as the proof script prints it.
    pub fn render(&self) -> String {
        let yn = |b: bool| if b { "yes" } else { "NO" };
        let mut out = format!(
            "{:<20} {:<50} {:<12} {:<36} {:<12} {}\n",
            "at", "position", "doc blob", "view id", "bytes repeat", "id repeats"
        );
        for r in &self.rows {
            let pos = if r.position.len() > 48 {
                format!(
                    "{}..{}",
                    &r.position[..22],
                    &r.position[r.position.len() - 24..]
                )
            } else {
                r.position.clone()
            };
            out.push_str(&format!(
                "{:<20} {:<50} {:<12} {:<36} {:<12} {}\n",
                r.at,
                pos,
                r.doc_blob
                    .as_deref()
                    .map_or("absent".to_string(), |b| b[..10].to_string()),
                r.view_id,
                yn(r.bytes_repeat),
                yn(r.id_repeat)
            ));
        }
        out.push_str(&format!(
            "positions distinct, every second read identical: {}\n",
            if self.ok { "yes" } else { "NO" }
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coderepo::{git, tests::commit_file, tests::scratch_repo};
    use crate::position::Resolved;
    use crate::position::read_position;
    use std::path::PathBuf;

    const C: &str = "e410760849abb8cd9fa8fbba305f5dd5d96ba954";

    fn repo(name: &str) -> PathBuf {
        let dir = scratch_repo(&format!("views-{name}"));
        let empty = dir.join(".git/empty-excludes");
        std::fs::write(&empty, "").unwrap();
        git(&dir, &["config", "core.autocrlf", "false"]).unwrap();
        git(
            &dir,
            &["config", "core.excludesFile", empty.to_str().unwrap()],
        )
        .unwrap();
        dir
    }

    /// R-4: the Mermaid of a view with quotes and odd names in labels passes the lint.
    #[test]
    fn docs_view_mermaid_passes_the_lint() {
        let docs = [
            Doc {
                path: "docs/we\"ird [name] (x).md".into(),
                blob: "a".repeat(40),
                bytes: 10,
                diagrams: 2,
            },
            Doc {
                path: "docs/sub dir/plain.md".into(),
                blob: "b".repeat(40),
                bytes: 5,
                diagrams: 1,
            },
            Doc {
                path: "docs/sub dir/none.md".into(),
                blob: "c".repeat(40),
                bytes: 5,
                diagrams: 0,
            },
        ];
        let a = render(&PositionRef::at_commit("r", C), "f", "docs", 60, 3, &docs);
        let findings = mermaid_lint::lint_source(&a.mermaid);
        assert!(findings.is_empty(), "{findings:?}\n{}", a.mermaid);
        assert!(a.mermaid.contains("#quot;"));
        assert!(a.mermaid.contains("(2 diagrams)"));
        assert!(a.mermaid.contains("(1 diagram)"));
        assert_eq!(a.graph.nodes.len(), 5);
        assert_eq!(a.graph.edges.len(), 3);
        assert!(crate::position::is_view_id(a.view_id.as_deref().unwrap()));
        // The empty root folder reads as `/`.
        let b = render(&PositionRef::at_commit("r", C), "f", "", 60, 0, &[]);
        assert!(b.mermaid.contains("[\"/\"]"));
        assert!(mermaid_lint::lint_source(&b.mermaid).is_empty());
    }

    /// R-4: one position gives identical bytes and ID; a commit on a branch and an uncommitted edit
    /// each give another.
    #[test]
    fn docs_view_repeats_and_moves_with_position() {
        let d = repo("repeat");
        std::fs::create_dir_all(d.join("docs")).unwrap();
        commit_file(&d, "docs/overview.md", "# Overview\n", "overview");
        commit_file(
            &d,
            "docs/guide.md",
            "# Guide\n\n```mermaid\nflowchart LR\n  A --> B\n```\n",
            "guide",
        );
        let facts = facts_version("board0", "");
        let view = |st: &WorkingState| docs_view(&d, st, &facts, "docs", 60).unwrap();

        let st = read_position(&d, "r").unwrap();
        let a = view(&st);
        let b = view(&read_position(&d, "r").unwrap());
        assert_eq!(answer_bytes(&a), answer_bytes(&b));
        assert_eq!(a.view_id, b.view_id);
        assert_eq!(a.graph.shown, 2);
        let guide = a.graph.nodes.iter().find(|n| n.name == "guide.md").unwrap();
        assert_eq!(guide.diagrams, Some(1));
        let root = d.canonicalize().unwrap().display().to_string();
        let body = String::from_utf8(answer_bytes(&a)).unwrap();
        assert!(!body.contains(&root) && !body.contains(&d.display().to_string()));

        // A commit on a branch.
        git(&d, &["checkout", "-q", "-b", "feat/x"]).unwrap();
        commit_file(&d, "docs/guide.md", "# Guide, edited\n", "edit");
        let c = view(&read_position(&d, "r").unwrap());
        assert_ne!(c.view_id, a.view_id);
        // An uncommitted edit.
        std::fs::write(d.join("docs/overview.md"), "# Overview, edited\n").unwrap();
        let e = view(&read_position(&d, "r").unwrap());
        assert_ne!(e.view_id, c.view_id);
        assert_ne!(e.view_id, a.view_id);
        // A different board commit is a different question's facts.
        let st2 = read_position(&d, "r").unwrap();
        let f = docs_view(&d, &st2, &facts_version("board1", ""), "docs", 60).unwrap();
        assert_ne!(f.view_id, e.view_id);
        // A changed argument is a different ID.
        let g = docs_view(&d, &st2, &facts, "docs", 1).unwrap();
        assert_ne!(g.view_id, e.view_id);
        assert_eq!((g.graph.shown, g.graph.total), (1, 2));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// R-5: main, a branch and an uncommitted position give three distinct positions and IDs, each
    /// repeated; an edit between the two reads of one position makes `ok` false.
    #[test]
    fn the_proof_table_shows_three_positions_each_repeating() {
        let d = repo("proof");
        std::fs::create_dir_all(d.join("docs")).unwrap();
        commit_file(&d, "docs/overview.md", "# Overview\n", "overview");
        commit_file(&d, "docs/guide.md", "# Guide\n", "guide");
        git(&d, &["checkout", "-q", "-b", "feat/SPK-001-proof"]).unwrap();
        commit_file(
            &d,
            "docs/guide.md",
            "# Guide, edited on the branch\n",
            "edit",
        );
        std::fs::write(
            d.join("docs/overview.md"),
            "# Overview\n\n```mermaid\nflowchart LR\n  A --> B\n```\n",
        )
        .unwrap();
        std::fs::write(d.join("docs/new.md"), "# New\n").unwrap();
        std::fs::remove_file(d.join("docs/guide.md")).unwrap();
        let facts = facts_version("board0", "");
        let ats = [Some("main"), Some("feat/SPK-001-proof"), None];
        let t = proof(&d, "r", &facts, "docs/guide.md", &ats, "docs", 60).unwrap();
        assert_eq!(t.rows.len(), 3);
        assert!(t.ok, "{}", t.render());
        assert!(t.rows.iter().all(|r| r.bytes_repeat && r.id_repeat));
        let pos: BTreeSet<&str> = t.rows.iter().map(|r| r.position.as_str()).collect();
        assert_eq!(pos.len(), 3);
        assert!(t.rows[0].doc_blob.is_some());
        assert_ne!(t.rows[0].doc_blob, t.rows[1].doc_blob);
        assert_eq!(t.rows[2].doc_blob, None, "deleted in the working tree");
        assert!(t.rows[2].position.contains('+'));
        assert!(t.render().contains("main"));

        // An edit between the two reads of the live position breaks the repeat.
        let mut n = 0;
        let t = proof_with(
            &d,
            "r",
            &facts,
            "docs/guide.md",
            &[None],
            "docs",
            60,
            &mut |_| {
                n += 1;
                std::fs::write(d.join("docs/new.md"), "# New, edited between reads\n").unwrap();
            },
        )
        .unwrap();
        assert_eq!(n, 1);
        assert!(!t.ok);
        assert!(!t.rows[0].id_repeat);
        let _ = std::fs::remove_dir_all(&d);
    }

    fn docs_repo(name: &str) -> PathBuf {
        let d = repo(name);
        std::fs::create_dir_all(d.join("docs")).unwrap();
        commit_file(&d, "docs/overview.md", "# Overview\n", "overview");
        commit_file(&d, "docs/guide.md", "# Guide\n", "guide");
        d
    }

    /// R-6: `--at <main's commit>` while a branch with other content is checked out shows main's
    /// docs from objects; HEAD and the index do not move; the ID equals the one computed when main
    /// was checked out.
    #[test]
    fn at_a_clean_commit_answers_from_objects() {
        let d = docs_repo("at-clean");
        let facts = facts_version("board0", "");
        let main = read_position(&d, "r").unwrap();
        let on_main = docs_view(&d, &main, &facts, "docs", 60).unwrap();
        git(&d, &["checkout", "-q", "-b", "feat/x"]).unwrap();
        commit_file(&d, "docs/extra.md", "# Extra\n", "extra");
        let head = git(&d, &["rev-parse", "HEAD"]).unwrap();
        let status = git(&d, &["status", "--porcelain=v1"]).unwrap();
        let text = main.position.text();
        let Resolved::Commit(st) = resolve_position(&d, "r", Some(&text)).unwrap() else {
            panic!("a clean commit elsewhere resolves from objects");
        };
        let there = docs_view(&d, &st, &facts, "docs", 60).unwrap();
        assert!(there.exact);
        assert_eq!(there.view_id, on_main.view_id);
        assert_eq!(answer_bytes(&there), answer_bytes(&on_main));
        assert_eq!(there.graph.shown, 2, "main has two docs, the branch three");
        assert_eq!(git(&d, &["rev-parse", "HEAD"]).unwrap(), head);
        assert_eq!(git(&d, &["status", "--porcelain=v1"]).unwrap(), status);
        assert!(!d.join("docs/extra.md").exists() || head != main.position.text());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// R-6: the live dirty position's own text answers, as the live state.
    #[test]
    fn at_the_live_dirty_position_answers() {
        let d = docs_repo("at-live");
        std::fs::write(d.join("docs/overview.md"), "# Overview, edited\n").unwrap();
        let live = read_position(&d, "r").unwrap();
        assert!(live.position.dirty.is_some());
        let text = live.position.text();
        let Resolved::Live(st) = resolve_position(&d, "r", Some(&text)).unwrap() else {
            panic!("the live text resolves live");
        };
        assert_eq!(st, live);
        let a = docs_view(&d, &st, &facts_version("b", ""), "docs", 60).unwrap();
        assert!(a.exact && a.view_id.is_some());
        assert!(matches!(
            resolve_position(&d, "r", None).unwrap(),
            Resolved::Live(_)
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// R-6: an old dirty text and an unknown commit both name the live position; bad text is a
    /// usage error, not an unavailable position.
    #[test]
    fn at_an_unavailable_position_lists_the_live_one() {
        let d = docs_repo("at-gone");
        std::fs::write(d.join("docs/overview.md"), "# one\n").unwrap();
        let old = read_position(&d, "r").unwrap().position.text();
        std::fs::write(d.join("docs/overview.md"), "# two\n").unwrap();
        let live = read_position(&d, "r").unwrap().position;
        let unknown = format!("{}", "1".repeat(40));
        for wanted in [old, unknown] {
            match resolve_position(&d, "r", Some(&wanted)) {
                Err(crate::error::CoreError::PositionUnavailable {
                    wanted: w,
                    available,
                }) => {
                    assert_eq!(w, wanted);
                    assert_eq!(available, vec![live.clone()]);
                }
                other => panic!("expected PositionUnavailable, got {other:?}"),
            }
        }
        let err = resolve_position(&d, "r", Some("zz")).unwrap_err();
        assert_eq!(err.status(), 400);
        let e = crate::error::CoreError::PositionUnavailable {
            wanted: "w".into(),
            available: vec![live.clone()],
        };
        assert_eq!(e.status(), 409);
        assert_eq!(
            e.to_string(),
            format!(
                "position w is not here: this checkout is at {}",
                live.text()
            )
        );
        let none = crate::error::CoreError::PositionUnavailable {
            wanted: "w".into(),
            available: vec![],
        };
        assert!(none.to_string().ends_with("no checkout on this machine"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// R-4: F depends on the board commit and the facts revision and on nothing else.
    #[test]
    fn facts_cover_board_commit_and_facts_record_not_the_binary() {
        let a = facts_version("c0", "r0");
        assert_ne!(a, facts_version("c1", "r0"));
        assert_ne!(a, facts_version("c0", "r1"));
        assert_eq!(a, facts_version("c0", "r0"));
        // Fixed hex: SHA-256 of "board0\0" - the inputs are the two strings, never a version.
        assert_eq!(
            facts_version("board0", ""),
            "19b30b50475b4f566d70653bb06b03a528277e6d09c8dc8ccc34aa632784b06a"
        );
        // The board commit is the board's HEAD, "none" before it has one.
        let dir = std::env::temp_dir().join(format!("kanbanr-views-{}-facts", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::Store::new(dir.canonicalize().unwrap());
        assert_eq!(super::project_facts(&store, "p"), facts_version("none", ""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R-9: warnings round-trip through the bytes and are absent when empty; the snapshot text is
    /// fixed.
    #[test]
    fn warnings_travel_with_the_answer() {
        let mut a = render(&PositionRef::at_commit("r", C), "f", "docs", 60, 0, &[]);
        assert!(
            !String::from_utf8(answer_bytes(&a))
                .unwrap()
                .contains("warnings")
        );
        a.warnings.push("first".into());
        a.warnings.push("second".into());
        let back: ViewAnswer = serde_json::from_slice(&answer_bytes(&a)).unwrap();
        assert_eq!(back.warnings, vec!["first", "second"]);
        assert_eq!(
            snapshot_warning("app", C),
            format!(
                "app has no checkout on this machine: its position is the imported snapshot's \
                 commit {C}; uncommitted changes there cannot be seen"
            )
        );
    }

    /// R-7: an answer that is not exact has no `view_id` key.
    #[test]
    fn id_only_when_exact() {
        let mut a = render(&PositionRef::at_commit("r", C), "f", "docs", 60, 0, &[]);
        assert!(
            String::from_utf8(answer_bytes(&a))
                .unwrap()
                .contains("\"view_id\"")
        );
        a.exact = false;
        a.view_id = None;
        assert!(
            !String::from_utf8(answer_bytes(&a))
                .unwrap()
                .contains("view_id\":")
        );
    }

    /// R-7: with a file edited between the position read and the file read, the answer is not
    /// exact, has no ID and carries the warning; an edit on the first attempt only is retried and
    /// exact; with no hook it is exact.
    #[test]
    fn a_file_changed_during_the_read_gives_no_id() {
        let d = docs_repo("moved");
        std::fs::write(d.join("docs/new.md"), "# New\n").unwrap();
        let facts = facts_version("b", "");
        let plain = docs_view_live(&d, "r", &facts, None, "docs", 60).unwrap();
        assert!(plain.exact && plain.view_id.is_some() && plain.warnings.is_empty());
        let mut n = 0;
        let hook = |d: &Path, k: &mut u32| {
            *k += 1;
            std::fs::write(d.join("docs/new.md"), format!("# New {k}\n")).unwrap();
        };
        let a =
            docs_view_hooked(&d, "r", &facts, None, "docs", 60, &mut |_| hook(&d, &mut n)).unwrap();
        assert_eq!(n, 2, "read twice, then gave up");
        assert!(!a.exact);
        assert_eq!(a.view_id, None);
        assert_eq!(a.warnings, vec![MOVED_WARNING.to_string()]);
        assert_eq!(a.positions.len(), 1);
        // One edit, on the first attempt only: the second read is exact.
        let mut m = 0;
        let b = docs_view_hooked(&d, "r", &facts, None, "docs", 60, &mut |attempt| {
            if attempt == 0 {
                hook(&d, &mut m);
            }
        })
        .unwrap();
        assert!(b.exact && b.view_id.is_some() && b.warnings.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// R-5: two answers at one position and board commit are byte-identical.
    #[test]
    fn docs_view_repeats_byte_for_byte() {
        let d = docs_repo("bytes");
        std::fs::write(d.join("docs/new.md"), "# New\n").unwrap();
        let facts = facts_version("b", "");
        let a = docs_view_live(&d, "r", &facts, None, "docs", 60).unwrap();
        let b = docs_view_live(&d, "r", &facts, None, "docs", 60).unwrap();
        assert_eq!(answer_bytes(&a), answer_bytes(&b));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// R-5: the body holds no scratch root, temporary directory, host name or date.
    #[test]
    fn docs_view_body_holds_no_clock_host_or_absolute_path() {
        let d = docs_repo("body");
        let a = docs_view_live(&d, "r", &facts_version("b", ""), None, "docs", 60).unwrap();
        let body = String::from_utf8(answer_bytes(&a)).unwrap();
        for bad in [
            d.display().to_string(),
            d.canonicalize().unwrap().display().to_string(),
            std::env::temp_dir().display().to_string(),
        ] {
            assert!(!body.contains(&bad), "{bad} in the body");
        }
        let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
        let host = host.trim();
        assert!(
            host.len() < 3 || !body.contains(host),
            "host name in the body"
        );
        let year = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1970, |t| 1970 + t.as_secs() / 31_556_952);
        assert!(!body.contains(&format!("{year}-")), "a date in the body");
        assert!(!body.contains("T0") && !body.contains(".000Z"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// R-4: an edit, a commit, another `max-nodes`, another board commit each change the ID;
    /// repeating any state repeats it.
    #[test]
    fn id_changes_with_position_question_and_facts() {
        let d = docs_repo("ids");
        let f0 = facts_version("board0", "");
        let id = |facts: &str, max: usize| {
            docs_view_live(&d, "r", facts, None, "docs", max)
                .unwrap()
                .view_id
                .unwrap()
        };
        let base = id(&f0, 60);
        assert_eq!(id(&f0, 60), base);
        assert_ne!(id(&facts_version("board1", ""), 60), base, "board commit");
        assert_ne!(
            id(&facts_version("board0", "rev"), 60),
            base,
            "facts record"
        );
        assert_ne!(id(&f0, 1), base, "question");
        std::fs::write(d.join("docs/overview.md"), "# edited\n").unwrap();
        let edited = id(&f0, 60);
        assert_ne!(edited, base, "uncommitted edit");
        git(&d, &["add", "-A"]).unwrap();
        git(&d, &["commit", "-q", "-m", "edit"]).unwrap();
        let committed = id(&f0, 60);
        assert_ne!(committed, edited, "a commit");
        assert_ne!(committed, base);
        assert_eq!(id(&f0, 60), committed);
        let _ = std::fs::remove_dir_all(&d);
    }
}
