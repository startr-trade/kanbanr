//! The spike's views (SPK-001): one generated view, the markdown docs under a folder at a
//! position, in the shape of katalogr's `ViewAnswer`, and the proof that it repeats.
//!
//! Nothing here is reachable from the CLI, the daemon or the monitor.

use crate::diagrams;
use crate::error::Result;
use crate::hash::stable_hash;
use crate::position::{PositionRef, ViewQuery, WorkingState, read_at, tree_at, view_id};
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

/// What a view answers with: the same shape as katalogr's ViewAnswer, so one renderer draws both.
/// Field order is the wire order; no `serde_json::Value` is in it (its key order would follow the
/// build's `preserve_order` feature and break "same bytes").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct ViewQueryJson {
    pub op: String,
    pub args: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewGraph {
    pub edges: Vec<GraphEdge>,
    pub nodes: Vec<GraphNode>,
    pub shown: usize,
    pub title: String,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// The bytes a view is printed, stored and compared as: compact JSON of `ViewAnswer` in field
/// order.
pub fn answer_bytes(a: &ViewAnswer) -> Vec<u8> {
    serde_json::to_vec(a).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coderepo::{git, tests::commit_file, tests::scratch_repo};
    use crate::position::read_position;
    use std::path::PathBuf;

    const C: &str = "e410760849abb8cd9fa8fbba305f5dd5d96ba954";

    fn repo(name: &str) -> PathBuf {
        let dir = scratch_repo(&format!("views-{name}"));
        let empty = dir.join(".git/empty-excludes");
        std::fs::write(&empty, "").unwrap();
        git(&dir, &["config", "core.autocrlf", "false"]).unwrap();
        git(&dir, &["config", "core.excludesFile", empty.to_str().unwrap()]).unwrap();
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
        commit_file(&d, "docs/guide.md", "# Guide\n\n```mermaid\nflowchart LR\n  A --> B\n```\n", "guide");
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
}
