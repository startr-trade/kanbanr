//! A doc's query blocks asked of katalogr at a position (FTR-016 T4, R-3, R-5, R-11). The daemon
//! asks; the browser never does (katalogr's loopback and Host guard). Every failure on katalogr's
//! side is a value on the block, never an error of the route; nothing is cached or written here.

use super::KatalogrState;
use super::client::{Client, KatalogrView, PositionRef};
use super::query::{QueryBlock, QuerySpec, find_query_blocks};
use crate::store::Store;
use serde::Serialize;
use std::path::Path;

/// At most this many blocks are asked per doc; the rest say "not asked: over MAX_BLOCKS blocks".
pub const MAX_BLOCKS: usize = 20;

/// Why a doc is asked at katalogr's own current context.
const NO_REPO: &str = "no code repository here: katalogr's current checkout";
/// Where the position came from when the daemon has the project's checkout.
const LIVE: &str = "the live position of the project's checkout";

#[derive(Debug, Clone, Serialize)]
pub struct DocViews {
    pub doc: String,
    /// The position asked for, or empty with `position_from` saying why.
    pub position: String,
    pub position_from: String,
    pub katalogr: KatalogrState,
    pub blocks: Vec<DocViewBlock>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocViewBlock {
    pub index: usize,
    pub line: usize,
    pub source: String,
    /// The parsed question, or the grammar's reason it is not one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<QuerySpec>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub problem: String,
    /// What the monitor draws: only when katalogr is Ready and the block parsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<BlockView>,
    /// Why there is no view, in one sentence; empty when `view` is set.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub said: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockView {
    pub mermaid: String,
    pub positions: Vec<PositionRef>,
    pub exact: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    pub pele: String,
    /// `<serve url>/#/<view>?at=<position>` when `web-view` is among the capabilities.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub open: String,
    /// Plain sentences shown under the caption: `"katalogr: " + views::snapshot_warning(..)` for
    /// each position of an exact answer whose repo has no checkout here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// What `GET /projects/{p}/katalogr` answers: the state, and where a doc would be asked.
#[derive(Debug, Clone, Serialize)]
pub struct KatalogrStatus {
    pub state: KatalogrState,
    pub position: String,
    pub position_from: String,
    pub url: String,
}

/// The live position of the checkout at `repo` for `project`, and where it came from. With no
/// checkout (or one that cannot be read) the position is `None` and the second part says why.
pub fn live_position(repo: Option<&Path>, project: &str) -> (Option<String>, String) {
    match repo {
        None => (None, NO_REPO.to_string()),
        Some(root) => match crate::position::read_position(root, project) {
            Ok(state) => (Some(state.position.text()), LIVE.to_string()),
            Err(e) => (
                None,
                format!(
                    "the checkout's position could not be read ({e}): katalogr's current checkout"
                ),
            ),
        },
    }
}

impl KatalogrStatus {
    pub fn load(repo: Option<&Path>, project: &str, client: &Client) -> KatalogrStatus {
        let (position, position_from) = live_position(repo, project);
        KatalogrStatus {
            state: client.state(),
            position: position.unwrap_or_default(),
            position_from,
            url: client.settings().url.clone(),
        }
    }
}

impl DocViews {
    /// Read the doc through the store, find its blocks, parse each; ask `client.state()` once and,
    /// when Ready, `client.view` per parsed block with `at` = `position`. Reads nothing twice;
    /// writes nothing.
    pub fn load(
        store: &Store,
        project: &str,
        doc: &str,
        position: Option<&str>,
        client: &Client,
    ) -> crate::Result<DocViews> {
        let markdown = store.read_doc(project, doc)?;
        let found = find_query_blocks(&markdown);
        let state = client.state();
        let at = position.filter(|p| !p.is_empty());
        let blocks = found
            .iter()
            .enumerate()
            .map(|(index, block)| ask(index, block, &state, at, project, client))
            .collect();
        Ok(DocViews {
            doc: doc.to_string(),
            position: at.unwrap_or_default().to_string(),
            position_from: if at.is_some() { LIVE } else { NO_REPO }.to_string(),
            katalogr: state,
            blocks,
        })
    }
}

fn ask(
    index: usize,
    block: &QueryBlock,
    state: &KatalogrState,
    at: Option<&str>,
    project: &str,
    client: &Client,
) -> DocViewBlock {
    let mut out = DocViewBlock {
        index,
        line: block.line,
        source: block.source.clone(),
        query: None,
        problem: String::new(),
        view: None,
        said: String::new(),
    };
    let spec = match QuerySpec::parse(&block.source) {
        Ok(spec) => spec,
        Err(reason) => {
            out.said = format!("this block is not a query: {reason}");
            out.problem = reason;
            return out;
        }
    };
    out.query = Some(spec.clone());
    if index >= MAX_BLOCKS {
        out.said = format!("not asked: over {MAX_BLOCKS} blocks");
        return out;
    }
    if !matches!(state, KatalogrState::Ready { .. }) {
        out.said = match state {
            KatalogrState::Absent { .. } => {
                format!("{} — the block shows its source", state.sentence())
            }
            _ => state.sentence(),
        };
        return out;
    }
    match client.view(&spec, at) {
        Ok(view) => out.view = Some(block_view(&view, &spec, state, at, project)),
        Err(e) => out.said = e.sentence(),
    }
    out
}

fn block_view(
    view: &KatalogrView,
    spec: &QuerySpec,
    state: &KatalogrState,
    at: Option<&str>,
    project: &str,
) -> BlockView {
    let warnings = if view.exact {
        view.positions
            .iter()
            .filter(|p| p.dirty.is_none() && p.repo != project)
            .filter_map(|p| {
                let commit = p.commit.as_deref()?;
                Some(format!(
                    "katalogr: {}",
                    crate::views::snapshot_warning(&p.repo, commit)
                ))
            })
            .collect()
    } else {
        Vec::new()
    };
    let open = match state {
        KatalogrState::Ready { url, .. } if state.has("web-view") => {
            let page = if spec.op == "impact" {
                "impact"
            } else {
                "graph"
            };
            let position = at
                .map(str::to_string)
                .or_else(|| view.positions.first().map(PositionRef::text))
                .unwrap_or_default();
            if position.is_empty() {
                format!("{url}/#/{page}")
            } else {
                format!("{url}/#/{page}?at={}", super::query::encode(&position))
            }
        }
        _ => String::new(),
    };
    BlockView {
        mermaid: view.mermaid.clone(),
        positions: view.positions.clone(),
        exact: view.exact,
        view_id: view.view_id.clone(),
        pele: view.pele.clone(),
        open,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::super::Settings;
    use super::super::fake;
    use super::*;
    use std::time::{Duration, Instant};

    const COMMIT: &str = fake::DEFAULT_COMMIT;

    fn board(doc: &str) -> (std::path::PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-docviews-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        store
            .init_project("shop", crate::ProjectConfig::default_for("shop"))
            .unwrap();
        store.write_doc("shop", "views.md", doc).unwrap();
        (dir, store)
    }

    fn client(url: &str) -> Client {
        Client::new(Settings::from_env().with_url(url))
    }

    const TWO: &str = "# Views\n\n```katalogr-query\nstructure --level package\n```\n\ntext\n\n```katalogr-query\ncallers main\n```\n";

    /// FTR-016 R-3: each block is asked at the position, in order, with the consistency flags and
    /// sorted args, and the answer is kept as the monitor draws it.
    #[test]
    fn doc_views_ask_each_block_at_the_position_and_keep_the_answer() {
        let (dir, store) = board(TWO);
        let fake = fake::serve(&["views"], &[], Duration::ZERO);
        let views =
            DocViews::load(&store, "shop", "views.md", Some(COMMIT), &client(&fake.url)).unwrap();
        assert_eq!(views.position, COMMIT);
        assert!(matches!(views.katalogr, KatalogrState::Ready { .. }));
        assert_eq!(views.blocks.len(), 2);
        let asked: Vec<String> = fake
            .requests()
            .into_iter()
            .filter(|r| r.contains("/views/"))
            .collect();
        assert_eq!(asked.len(), 2, "{asked:?}");
        assert!(
            asked[0].starts_with("/api/v1/views/structure?"),
            "{asked:?}"
        );
        assert!(asked[1].starts_with("/api/v1/views/callers?"), "{asked:?}");
        for r in &asked {
            assert!(r.contains(&format!("at={COMMIT}")), "{r}");
            assert!(r.contains("consistency=wait"), "{r}");
        }
        assert!(asked[0].contains("level=package"), "{asked:?}");
        for (i, b) in views.blocks.iter().enumerate() {
            assert_eq!(b.index, i);
            let v = b.view.as_ref().expect("a view");
            assert!(!v.mermaid.is_empty() && !v.pele.is_empty());
            assert!(!v.positions.is_empty());
            assert!(v.exact);
            assert!(
                v.view_id
                    .as_deref()
                    .is_some_and(crate::position::is_view_id)
            );
            assert!(b.said.is_empty() && b.problem.is_empty());
            assert!(v.open.is_empty(), "no web-view capability, no link");
        }
        assert_eq!(views.blocks[0].line, 3);
        assert_eq!(views.blocks[1].line, 9);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-016 R-3: the open link names `graph`, or `impact` for an impact block, and the
    /// position, when katalogr lists `web-view`.
    #[test]
    fn the_open_link_follows_the_web_view_capability() {
        let doc = "```katalogr-query\nimpact main\n```\n\n```katalogr-query\nflow main\n```\n";
        let (dir, store) = board(doc);
        let fake = fake::serve(&["views", "web-view"], &[], Duration::ZERO);
        let views =
            DocViews::load(&store, "shop", "views.md", Some(COMMIT), &client(&fake.url)).unwrap();
        let open = |i: usize| views.blocks[i].view.as_ref().unwrap().open.clone();
        assert_eq!(open(0), format!("{}/#/impact?at={COMMIT}", fake.url));
        assert_eq!(open(1), format!("{}/#/graph?at={COMMIT}", fake.url));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-016 R-5: a position no katalogr context holds is named, with what is held; the block
    /// keeps its source and the others are still answered.
    #[test]
    fn a_position_no_context_holds_names_the_missing_position() {
        let (dir, store) = board(TWO);
        let wanted = "fedcba9876543210fedcba9876543210fedcba98";
        let fake = fake::serve(&["views"], &[wanted], Duration::ZERO);
        let views =
            DocViews::load(&store, "shop", "views.md", Some(wanted), &client(&fake.url)).unwrap();
        for b in &views.blocks {
            assert!(b.view.is_none());
            assert!(b.said.contains(wanted), "{}", b.said);
            assert!(b.said.contains("it holds"), "{}", b.said);
            assert!(!b.source.is_empty());
        }
        // Another block asked at a position the fake holds is answered beside a missing one.
        let (dir2, store2) = board(TWO);
        let views = DocViews::load(
            &store2,
            "shop",
            "views.md",
            Some(COMMIT),
            &client(&fake.url),
        )
        .unwrap();
        assert!(views.blocks.iter().all(|b| b.view.is_some()));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    /// FTR-016 R-5: an absent katalogr leaves every block with its source and one plain sentence;
    /// a block that is not a query says why, and a doc with no blocks asks for no view.
    #[test]
    fn an_absent_katalogr_and_a_bad_block_leave_the_source_with_a_sentence() {
        let doc = "```katalogr-query\nstructure\n```\n\n```katalogr-query\nnonsense here\n```\n";
        let (dir, store) = board(doc);
        let url = fake::closed_port_url();
        let views = DocViews::load(&store, "shop", "views.md", None, &client(&url)).unwrap();
        assert!(matches!(views.katalogr, KatalogrState::Absent { .. }));
        assert_eq!(views.position, "");
        assert!(views.position_from.contains("no code repository"));
        assert!(
            views.blocks[0].said.contains("katalogr is not running at"),
            "{}",
            views.blocks[0].said
        );
        assert!(
            views.blocks[1].said.contains("not a query"),
            "{}",
            views.blocks[1].said
        );
        assert!(!views.blocks[1].problem.is_empty());
        assert!(views.blocks[1].query.is_none());
        store
            .write_doc("shop", "plain.md", "# no blocks\n")
            .unwrap();
        let fake = fake::serve(&["views"], &[], Duration::ZERO);
        let views = DocViews::load(&store, "shop", "plain.md", None, &client(&fake.url)).unwrap();
        assert!(views.blocks.is_empty());
        assert!(
            fake.requests().iter().all(|r| !r.contains("/views/")),
            "no blocks, no view asked: {:?}",
            fake.requests()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-016 R-3: blocks past MAX_BLOCKS are not asked.
    #[test]
    fn blocks_over_the_cap_are_not_asked() {
        let one = "```katalogr-query\ncycles\n```\n\n";
        let (dir, store) = board(&one.repeat(MAX_BLOCKS + 2));
        let fake = fake::serve(&["views"], &[], Duration::ZERO);
        let views = DocViews::load(&store, "shop", "views.md", None, &client(&fake.url)).unwrap();
        assert_eq!(views.blocks.len(), MAX_BLOCKS + 2);
        assert!(views.blocks[MAX_BLOCKS - 1].view.is_some());
        assert!(views.blocks[MAX_BLOCKS].said.contains("not asked: over"));
        assert_eq!(
            fake.requests()
                .iter()
                .filter(|r| r.contains("/views/"))
                .count(),
            MAX_BLOCKS
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FTR-016 R-11: a stored view costs the daemon at most 50 ms (median of 20 loads of a
    /// one-block doc against the fake with no delay).
    #[test]
    fn a_stored_view_costs_the_daemon_at_most_fifty_milliseconds() {
        let (dir, store) = board("```katalogr-query\nstructure\n```\n");
        let fake = fake::serve(&["views"], &[], Duration::ZERO);
        let c = client(&fake.url);
        let _ = DocViews::load(&store, "shop", "views.md", Some(COMMIT), &c).unwrap();
        let mut times: Vec<Duration> = (0..20)
            .map(|_| {
                let t = Instant::now();
                let v = DocViews::load(&store, "shop", "views.md", Some(COMMIT), &c).unwrap();
                assert!(v.blocks[0].view.is_some());
                t.elapsed()
            })
            .collect();
        times.sort();
        let median = times[10];
        assert!(median < Duration::from_millis(50), "median {median:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An exact answer naming a repo that is not the project, with no dirty part, carries
    /// FTR-003's snapshot warning, prefixed "katalogr: ".
    #[test]
    fn a_position_of_another_repo_carries_the_snapshot_warning() {
        let view: KatalogrView = serde_json::from_str(&format!(
            r#"{{"schema":1,"view_format":1,"exact":true,"view_id":"{}","positions":[{{"repo":"lib","commit":"{COMMIT}","dirty":null}},{{"repo":"shop","commit":"{COMMIT}","dirty":null}}],"query":{{"op":"structure","args":{{}}}},"mermaid":"graph TD","pele":"x"}}"#,
            fake::VIEW_ID
        ))
        .unwrap();
        let spec = QuerySpec::parse("structure").unwrap();
        let state = KatalogrState::Absent {
            url: String::new(),
            said: String::new(),
        };
        let b = block_view(&view, &spec, &state, None, "shop");
        assert_eq!(b.warnings.len(), 1, "{:?}", b.warnings);
        assert!(b.warnings[0].starts_with("katalogr: lib has no checkout on this machine"));
    }
}
