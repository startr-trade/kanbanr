//! Several boards behind one read-only server (FTR-040).
//!
//! A `Boards` is the ordered list of data folders a `kanbanr serve --board …` shows. Each board
//! keeps its own `Store`, so every file and git read for a project is made under that project's
//! own folder; nothing resolves across folders (ADR-0007). A data folder served the usual way is
//! a list of one ([`Boards::single`]).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::store::Store;

/// One board: its name (the folder name the operator wrote), its folder, and its store.
pub struct Board {
    pub name: String,
    pub dir: PathBuf,
    pub store: Arc<Store>,
}

/// The boards one server shows, in the order given. Built once; immutable.
pub struct Boards {
    boards: Vec<Board>,
}

/// Why a list of boards is refused before anything is served (FTR-040 R-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardsError {
    NoName(PathBuf),
    NoProjects(PathBuf),
    SameName {
        name: String,
        first: PathBuf,
        second: PathBuf,
    },
    SameProject {
        project: String,
        first: String,
        second: String,
    },
}

impl std::fmt::Display for BoardsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardsError::NoName(p) => write!(
                f,
                "board path {} has no folder name; name the board's own folder",
                p.display()
            ),
            BoardsError::NoProjects(p) => write!(
                f,
                "board {} has no projects/ folder: is it a kanbanr board?",
                p.display()
            ),
            BoardsError::SameName {
                name,
                first,
                second,
            } => write!(
                f,
                "boards {} and {} are both named '{name}': give each board folder its own name",
                first.display(),
                second.display()
            ),
            BoardsError::SameProject {
                project,
                first,
                second,
            } => write!(
                f,
                "project '{project}' is on two boards, '{first}' and '{second}': one server shows \
                 a project from one board only"
            ),
        }
    }
}

impl std::error::Error for BoardsError {}

/// One board as `GET /api/boards` lists it. Names no server path and no remote URL.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BoardInfo {
    pub name: String,
    /// The projects served from this board, sorted; a project an earlier board holds is not here.
    pub projects: Vec<String>,
    /// The board repository's HEAD commit, 40 hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// The HEAD commit's committer time, RFC 3339 UTC.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_at: Option<String>,
    /// The kanbanr that last wrote this board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writer: Option<crate::server::BoardWriter>,
    /// Why the board's git history cannot be read, without its path (DEF-080).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_unreadable: Option<String>,
    /// Projects on this board that an earlier board also holds, so they are served from there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed: Vec<String>,
}

/// The board's name: the last component of the path as written, when it is a folder name.
fn name_of(path: &Path) -> Option<String> {
    match path.components().next_back()? {
        Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
        _ => None,
    }
}

impl Boards {
    /// Open the boards at `paths`, in order, refusing a list one server cannot show. Writes
    /// nothing.
    pub fn open(paths: &[PathBuf]) -> Result<Boards, BoardsError> {
        let cwd = std::env::current_dir().ok();
        let mut boards: Vec<(Board, PathBuf)> = Vec::new();
        for given in paths {
            let name = name_of(given).ok_or_else(|| BoardsError::NoName(given.clone()))?;
            let dir = match (&cwd, given.is_relative()) {
                (Some(cwd), true) => cwd.join(given),
                _ => given.clone(),
            };
            if !dir.join("projects").is_dir() {
                return Err(BoardsError::NoProjects(given.clone()));
            }
            if let Some((_, first)) = boards.iter().find(|(b, _)| b.name == name) {
                return Err(BoardsError::SameName {
                    name,
                    first: first.clone(),
                    second: given.clone(),
                });
            }
            let store = Arc::new(Store::new(dir.clone()));
            boards.push((Board { name, dir, store }, given.clone()));
        }
        let mut seen: Vec<(String, String)> = Vec::new();
        for (board, _) in &boards {
            for project in board.store.list_projects().unwrap_or_default() {
                if let Some((_, first)) = seen.iter().find(|(p, _)| *p == project) {
                    return Err(BoardsError::SameProject {
                        project,
                        first: first.clone(),
                        second: board.name.clone(),
                    });
                }
                seen.push((project, board.name.clone()));
            }
        }
        Ok(Boards {
            boards: boards.into_iter().map(|(b, _)| b).collect(),
        })
    }

    /// A data folder served the usual way: a list of one, named after its folder. No checks.
    pub fn single(dir: PathBuf) -> Boards {
        let name = name_of(&dir).unwrap_or_else(|| "board".to_string());
        let store = Arc::new(Store::new(dir.clone()));
        Boards {
            boards: vec![Board { name, dir, store }],
        }
    }

    pub fn all(&self) -> &[Board] {
        &self.boards
    }

    pub fn first(&self) -> &Board {
        &self.boards[0]
    }

    pub fn by_name(&self, name: &str) -> Option<&Board> {
        self.boards.iter().find(|b| b.name == name)
    }

    /// The first board, in order, that holds `project`; asked afresh on each call, so a project a
    /// pull added is found.
    pub fn for_project(&self, project: &str) -> Option<&Board> {
        self.boards.iter().find(|b| b.store.project_exists(project))
    }

    pub fn board_of(&self, project: &str) -> Option<&str> {
        self.for_project(project).map(|b| b.name.as_str())
    }

    /// One entry per board, in order. A project an earlier board holds is listed as shadowed,
    /// with a note printed once per (board, project).
    pub fn info(&self) -> Vec<BoardInfo> {
        let mut earlier: Vec<(String, &str)> = Vec::new();
        let mut out = Vec::new();
        for board in &self.boards {
            let mut projects = Vec::new();
            let mut shadowed = Vec::new();
            for p in board.store.list_projects().unwrap_or_default() {
                if let Some((_, first)) = earlier.iter().find(|(q, _)| *q == p) {
                    crate::git::warn_once(
                        &format!("boards-shadowed:{}:{p}", board.name),
                        &format!(
                            "project '{p}' is also on board '{}'; serving it from '{first}'",
                            board.name
                        ),
                    );
                    shadowed.push(p);
                } else {
                    projects.push(p);
                }
            }
            earlier.extend(projects.iter().map(|p| (p.clone(), board.name.as_str())));
            out.push(BoardInfo {
                name: board.name.clone(),
                projects,
                head: crate::git::head_commit(&board.dir),
                head_at: crate::git::head_time(&board.dir),
                writer: crate::writer::recorded(&board.dir).map(|w| crate::server::BoardWriter {
                    version: w.version,
                    commit: w.commit,
                    at: w.at,
                    dirty: w.dirty,
                }),
                history_unreadable: crate::git::open_history(&board.dir)
                    .err()
                    .map(|e| e.reason_text()),
                shadowed,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectConfig;

    /// A fresh scratch folder under the system temp dir (L-41), unique to the test.
    fn scratch(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-boards-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(dir.starts_with(std::env::temp_dir()));
        dir
    }

    /// A board folder `<root>/<name>` holding `projects`, a committed repository when `git` is set.
    fn board(root: &Path, name: &str, projects: &[&str], git: bool) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        let store = Store::new(dir.clone());
        for p in projects {
            store
                .init_project(p, ProjectConfig::default_for(p))
                .unwrap();
        }
        if git {
            crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
            // The first commit holds the projects made above.
            assert!(crate::git::head_commit(&dir).is_some());
        }
        dir
    }

    /// R-1: each board's projects, by board, in the order the boards were given.
    #[test]
    fn open_lists_each_boards_projects_in_list_order() {
        let root = scratch("order");
        let b = board(&root, "b", &["beta", "gamma"], true);
        let a = board(&root, "a", &["alpha"], true);
        let boards = Boards::open(&[b.clone(), a.clone()]).unwrap();
        let info = boards.info();
        let got: Vec<(&str, Vec<&str>)> = info
            .iter()
            .map(|i| {
                (
                    i.name.as_str(),
                    i.projects.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(got, [("b", vec!["beta", "gamma"]), ("a", vec!["alpha"])]);
        assert_eq!(boards.board_of("alpha"), Some("a"));
        assert_eq!(boards.board_of("gamma"), Some("b"));
        assert_eq!(boards.board_of("nope"), None);
        assert_eq!(boards.by_name("a").map(|x| x.dir.clone()), Some(a));
        assert_eq!(boards.first().name, "b");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// R-3: a folder that is no board is refused, naming it.
    #[test]
    fn a_folder_without_projects_is_refused() {
        let root = scratch("noprojects");
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let err = Boards::open(std::slice::from_ref(&plain)).err().unwrap();
        assert_eq!(err, BoardsError::NoProjects(plain.clone()));
        assert_eq!(
            err.to_string(),
            format!(
                "board {} has no projects/ folder: is it a kanbanr board?",
                plain.display()
            )
        );
        let err = Boards::open(&[PathBuf::from("/")]).err().unwrap();
        assert_eq!(
            err.to_string(),
            "board path / has no folder name; name the board's own folder"
        );
        assert!(matches!(
            Boards::open(&[root.join("..")]),
            Err(BoardsError::NoName(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// R-3: two board folders of one name are refused, naming both paths.
    #[test]
    fn two_boards_with_one_name_are_refused() {
        let root = scratch("samename");
        let first = board(&root.join("x"), "a", &["alpha"], false);
        let second = board(&root.join("y"), "a", &["beta"], false);
        let err = Boards::open(&[first.clone(), second.clone()])
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            format!(
                "boards {} and {} are both named 'a': give each board folder its own name",
                first.display(),
                second.display()
            )
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// R-3: a project on two boards is refused, naming the project and both boards.
    #[test]
    fn a_project_on_two_boards_is_refused_naming_both() {
        let root = scratch("sameproject");
        let a = board(&root, "a", &["alpha", "shared"], false);
        let b = board(&root, "b", &["shared"], false);
        let err = Boards::open(&[a, b]).err().unwrap();
        assert_eq!(
            err,
            BoardsError::SameProject {
                project: "shared".into(),
                first: "a".into(),
                second: "b".into()
            }
        );
        assert_eq!(
            err.to_string(),
            "project 'shared' is on two boards, 'a' and 'b': one server shows a project from one \
             board only"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// R-4: each project's store is its own board's, so its git reads are of that repository.
    #[test]
    fn each_boards_store_reads_its_own_folder() {
        let root = scratch("ownfolder");
        let a = board(&root, "a", &["alpha"], true);
        let b = board(&root, "b", &["beta"], true);
        let boards = Boards::open(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(boards.for_project("beta").unwrap().store.data_dir(), b);
        assert_eq!(boards.for_project("alpha").unwrap().store.data_dir(), a);
        let info = boards.info();
        assert_eq!(info[0].head, crate::git::head_commit(&a));
        assert_eq!(info[1].head, crate::git::head_commit(&b));
        assert_ne!(info[0].head, info[1].head);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// R-7: a project a later board gains is still served from the first, and listed as shadowed.
    #[test]
    fn a_project_added_later_to_a_second_board_is_served_from_the_first() {
        let root = scratch("later");
        let a = board(&root, "a", &["alpha"], false);
        let b = board(&root, "b", &["beta"], false);
        let boards = Boards::open(&[a.clone(), b.clone()]).unwrap();
        Store::new(b.clone())
            .init_project("alpha", ProjectConfig::default_for("alpha"))
            .unwrap();
        assert!(b.join("projects/alpha/config.yaml").is_file());
        assert_eq!(boards.for_project("alpha").unwrap().name, "a");
        let info = boards.info();
        assert_eq!(info[1].shadowed, ["alpha"]);
        assert_eq!(info[1].projects, ["beta"]);
        assert!(info[0].shadowed.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// R-8: a plain folder is served; it says why it has no history and gives no head.
    #[test]
    fn a_board_that_is_not_a_repository_is_served_and_says_why_it_has_no_history() {
        let root = scratch("notrepo");
        let plain = board(&root, "plain", &["alpha"], false);
        let repo = board(&root, "repo", &["beta"], true);
        let boards = Boards::open(&[plain, repo.clone()]).unwrap();
        let info = boards.info();
        assert_eq!(info[0].projects, ["alpha"]);
        assert!(info[0].head.is_none());
        assert!(info[0].head_at.is_none());
        let why = info[0].history_unreadable.as_deref().unwrap();
        assert!(why.contains("not a git repository"), "{why}");
        assert!(!why.contains(&root.display().to_string()), "{why}");
        assert!(info[1].history_unreadable.is_none());
        assert_eq!(info[1].head, crate::git::head_commit(&repo));
        let at = info[1].head_at.as_deref().unwrap();
        time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339).unwrap();
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains(&root.display().to_string()), "{json}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
