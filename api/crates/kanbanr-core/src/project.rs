//! Resolution of the data directory and the active project name.
//!
//! A project folder is tied to its board by a `.kanbanr` **marker** file. The marker names the
//! project and (optionally) where its data folder lives, relative to the marker:
//!
//! ```yaml
//! project: app
//! data_dir: ../app.kanbanr
//! ```
//!
//! A legacy one-line marker (just the project name) is still read. The marker is found by walking
//! up from the current directory, so commands work from any subfolder of the project.
//!
//! The recommended data folder is a **sibling of the project's git repo root** named
//! `<repo>.kanbanr`: the board is its own git repo and never nests inside the project's repo.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// File name of the per-project marker.
pub const MARKER_FILE: &str = ".kanbanr";
/// Suffix of the recommended sibling data folder (`<repo>.kanbanr`).
pub const DATA_DIR_SUFFIX: &str = ".kanbanr";

/// Contents of a `.kanbanr` marker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Data folder, relative to the marker's folder (or absolute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
}

impl Marker {
    /// Parse a marker: a YAML mapping, or a legacy single line holding just the project name.
    pub fn parse(content: &str) -> Marker {
        if let Ok(v @ serde_yaml::Value::Mapping(_)) = serde_yaml::from_str(content)
            && let Ok(m) = serde_yaml::from_value::<Marker>(v)
        {
            return Marker {
                project: non_empty(m.project),
                data_dir: non_empty(m.data_dir),
            };
        }
        let name = content.lines().next().unwrap_or("").trim();
        Marker {
            project: (!name.is_empty()).then(|| name.to_string()),
            data_dir: None,
        }
    }

    /// Render the marker. Without a data dir the legacy one-line form is written, so markers that
    /// don't need the new field stay readable by older kanbanr versions.
    pub fn render(&self) -> String {
        match (&self.project, &self.data_dir) {
            (Some(p), None) => format!("{p}\n"),
            _ => format!(
                "# kanbanr: this folder's project and where its board lives (relative to this file)\n{}",
                serde_yaml::to_string(self).unwrap_or_default()
            ),
        }
    }
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// A marker found on disk.
#[derive(Debug, Clone)]
pub struct FoundMarker {
    /// Path of the marker file itself.
    pub path: PathBuf,
    pub marker: Marker,
}

impl FoundMarker {
    /// The folder holding the marker.
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    /// The marker's data dir, resolved against the marker's folder.
    pub fn data_dir(&self, home: Option<&Path>) -> Option<PathBuf> {
        let raw = self.marker.data_dir.as_deref()?;
        Some(self.dir().join(expand_tilde(raw, home)))
    }
}

/// Walk up from `start` to the nearest `.kanbanr` marker file. `$HOME/.kanbanr` is skipped: older
/// kanbanr versions kept login profiles there.
pub fn find_marker(start: &Path, home: Option<&Path>) -> Option<FoundMarker> {
    start
        .ancestors()
        .filter(|dir| home.is_none_or(|h| *dir != h))
        .map(|dir| dir.join(MARKER_FILE))
        .find(|p| p.is_file())
        .and_then(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            Some(FoundMarker {
                marker: Marker::parse(&content),
                path,
            })
        })
}

/// Where the data dir came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataDirSource {
    /// `--data-dir`
    Flag,
    /// `$KANBANR_DATA_DIR`
    Env,
    /// `data_dir` in a `.kanbanr` marker
    Marker,
    /// Legacy fallback `./data`
    Default,
}

/// A resolved data dir with its provenance.
#[derive(Debug, Clone)]
pub struct ResolvedDataDir {
    pub path: PathBuf,
    pub source: DataDirSource,
    /// The marker consulted, if one was found.
    pub marker: Option<FoundMarker>,
}

/// Resolve the data dir: `--data-dir` → `$KANBANR_DATA_DIR` → marker `data_dir` → `./data`.
pub fn resolve_data_dir_in(
    explicit: Option<&str>,
    env: Option<&str>,
    cwd: &Path,
    home: Option<&Path>,
) -> ResolvedDataDir {
    let marker = find_marker(cwd, home);
    let (path, source) = if let Some(dir) = explicit.filter(|s| !s.is_empty()) {
        (PathBuf::from(dir), DataDirSource::Flag)
    } else if let Some(dir) = env.filter(|s| !s.is_empty()) {
        (PathBuf::from(dir), DataDirSource::Env)
    } else if let Some(dir) = marker.as_ref().and_then(|m| m.data_dir(home)) {
        (dir, DataDirSource::Marker)
    } else {
        (PathBuf::from("data"), DataDirSource::Default)
    };
    ResolvedDataDir {
        path,
        source,
        marker,
    }
}

/// [`resolve_data_dir_in`] against the real environment and current directory.
pub fn resolve_data_dir_detailed(explicit: Option<&str>) -> ResolvedDataDir {
    let env = std::env::var("KANBANR_DATA_DIR").ok();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_data_dir_in(explicit, env.as_deref(), &cwd, home_dir().as_deref())
}

/// Resolve the data directory (see [`resolve_data_dir_in`] for the order).
pub fn resolve_data_dir(explicit: Option<&str>) -> PathBuf {
    resolve_data_dir_detailed(explicit).path
}

/// Resolve the active project name: explicit arg → `KANBANR_PROJECT` → nearest `.kanbanr` marker
/// → the marker's folder name (or the current directory's name when there is no marker).
pub fn resolve_project_in(
    explicit: Option<&str>,
    env: Option<&str>,
    cwd: &Path,
    home: Option<&Path>,
) -> Option<String> {
    if let Some(p) = explicit {
        return Some(p.to_string());
    }
    if let Some(p) = env.filter(|s| !s.is_empty()) {
        return Some(p.to_string());
    }
    match find_marker(cwd, home) {
        Some(found) => found
            .marker
            .project
            .clone()
            .or_else(|| basename(found.dir())),
        None => basename(cwd),
    }
}

/// [`resolve_project_in`] against the real environment and current directory.
pub fn resolve_project(explicit: Option<&str>) -> Option<String> {
    let env = std::env::var("KANBANR_PROJECT").ok();
    let cwd = std::env::current_dir().ok()?;
    resolve_project_in(explicit, env.as_deref(), &cwd, home_dir().as_deref())
}

/// Write a marker into `dir`, returning its path.
pub fn write_marker(dir: &Path, marker: &Marker) -> std::io::Result<PathBuf> {
    let path = dir.join(MARKER_FILE);
    std::fs::write(&path, marker.render())?;
    Ok(path)
}

/// The project's root: the enclosing git work tree, else `cwd`. A repo rooted at (or above) the
/// home directory — e.g. a dotfiles repo — is ignored, since it isn't the project.
pub fn project_root(cwd: &Path, home: Option<&Path>) -> PathBuf {
    let cwd = normalize(cwd);
    git_worktree(&cwd, home).unwrap_or(cwd)
}

/// The project repo's HEAD commit (12 hex digits), recorded as provenance for imports.
pub fn project_revision(cwd: &Path, home: Option<&Path>) -> Option<String> {
    let root = git_worktree(&normalize(cwd), home)?;
    let repo = git2::Repository::open(root).ok()?;
    let oid = repo.head().ok()?.target()?.to_string();
    Some(oid[..12].to_string())
}

/// The recommended data dir: a sibling of the project root named `<root>.kanbanr`.
pub fn suggested_data_dir(cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let root = project_root(cwd, home);
    let name = root.file_name()?.to_str()?;
    Some(root.parent()?.join(format!("{name}{DATA_DIR_SUFFIX}")))
}

/// Existing kanbanr data folders next to the project root (`*.kanbanr` folders with a `projects/`
/// dir), sorted. Offered so several projects can share one data folder.
pub fn nearby_data_dirs(cwd: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let root = project_root(cwd, home);
    let Some(parent) = root.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(DATA_DIR_SUFFIX) && n != DATA_DIR_SUFFIX)
                && p.join("projects").is_dir()
        })
        .collect();
    dirs.sort();
    dirs
}

/// If `path` lies inside some other git work tree, return that work tree's root. A folder that is
/// itself a repo root (e.g. an existing data folder) doesn't count.
pub fn enclosing_git_worktree(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let path = normalize(path);
    if path.join(".git").exists() {
        return None;
    }
    let existing = path.ancestors().find(|a| a.exists())?;
    git_worktree(existing, home)
}

/// The work tree root of the repo containing `dir`, ignoring repos rooted at or above `home`.
fn git_worktree(dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let repo = git2::Repository::discover(dir).ok()?;
    let root = normalize(repo.workdir()?);
    let home = home.map(normalize);
    match home {
        Some(h) if h.starts_with(&root) => None,
        _ => Some(root),
    }
}

/// Express `path` relative to `base` (both made absolute first). Falls back to the absolute path
/// when they share no root, or when the relative form would climb more than two levels — an
/// unrelated location reads better as an absolute path.
pub fn relative_to(path: &Path, base: &Path) -> PathBuf {
    let path = normalize(path);
    let base = normalize(base);
    let p: Vec<Component> = path.components().collect();
    let b: Vec<Component> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let ups = b.len() - common;
    if common == 0 || ups > 2 {
        return path;
    }
    let mut rel = PathBuf::new();
    for _ in 0..ups {
        rel.push("..");
    }
    for c in &p[common..] {
        rel.push(c.as_os_str());
    }
    if rel.as_os_str().is_empty() {
        rel.push(".");
    }
    rel
}

/// `\\?\C:\x` becomes `C:\x` (FEAT-129). On Windows `canonicalize` returns the verbatim form,
/// which then appeared in every message that names a path — `added kanbanr hooks to
/// \\?\C:\Users\…` — and in anything written from one. A verbatim UNC path (`\\?\UNC\…`) is left
/// as it is: dropping the prefix there would change which path it means.
pub fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with(r"UNC\") => PathBuf::from(rest),
        _ => path,
    }
}

/// Make a path absolute and resolve `.`/`..` and symlinks as far as the path exists; a missing
/// tail is appended as-is.
pub fn normalize(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    if let Ok(c) = std::fs::canonicalize(&abs) {
        return without_verbatim_prefix(c);
    }
    match (abs.parent(), abs.file_name()) {
        (Some(parent), Some(name)) => normalize(parent).join(name),
        // A trailing `..` or `.` on a missing path: resolve the parent, then apply it.
        (Some(parent), None) => {
            let p = normalize(parent);
            match abs.components().next_back() {
                Some(Component::ParentDir) => p.parent().map(Path::to_path_buf).unwrap_or(p),
                _ => p,
            }
        }
        _ => abs,
    }
}

/// Expand a leading `~/` (or a bare `~`) to the home directory.
pub fn expand_tilde(raw: &str, home: Option<&Path>) -> PathBuf {
    match (raw.strip_prefix("~"), home) {
        (Some(rest), Some(h)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            h.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(raw),
    }
}

/// The user's home directory (`$HOME`, or `%USERPROFILE%` on Windows).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

fn basename(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A fresh, canonical temp dir (so path comparisons aren't tripped by symlinked /tmp).
    fn temp_dir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "kanbanr-project-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        std::fs::canonicalize(p).unwrap()
    }

    #[test]
    fn marker_parses_yaml_and_legacy_forms() {
        assert_eq!(
            Marker::parse("alpha\n"),
            Marker {
                project: Some("alpha".into()),
                data_dir: None
            }
        );
        assert_eq!(
            Marker::parse("# comment\nproject: app\ndata_dir: ../app.kanbanr\n"),
            Marker {
                project: Some("app".into()),
                data_dir: Some("../app.kanbanr".into())
            }
        );
        assert_eq!(Marker::parse(""), Marker::default());
        assert_eq!(Marker::parse("data_dir: '  '\n"), Marker::default());
    }

    #[test]
    fn marker_render_round_trips_and_keeps_legacy_form_without_data_dir() {
        let legacy = Marker {
            project: Some("app".into()),
            data_dir: None,
        };
        assert_eq!(legacy.render(), "app\n");
        let full = Marker {
            project: Some("app".into()),
            data_dir: Some("../app.kanbanr".into()),
        };
        assert_eq!(Marker::parse(&full.render()), full);
    }

    #[test]
    fn marker_is_found_from_a_subfolder_and_data_dir_is_relative_to_it() {
        let root = temp_dir();
        let proj = root.join("app");
        let sub = proj.join("src").join("deep");
        std::fs::create_dir_all(&sub).unwrap();
        write_marker(
            &proj,
            &Marker {
                project: Some("app".into()),
                data_dir: Some("../app.kanbanr".into()),
            },
        )
        .unwrap();

        let r = resolve_data_dir_in(None, None, &sub, None);
        assert_eq!(r.source, DataDirSource::Marker);
        assert_eq!(normalize(&r.path), root.join("app.kanbanr"));
        assert_eq!(
            resolve_project_in(None, None, &sub, None).as_deref(),
            Some("app")
        );
    }

    #[test]
    fn data_dir_order_is_flag_then_env_then_marker_then_default() {
        let root = temp_dir();
        write_marker(
            &root,
            &Marker {
                project: Some("p".into()),
                data_dir: Some("m".into()),
            },
        )
        .unwrap();
        let r = resolve_data_dir_in(Some("f"), Some("e"), &root, None);
        assert_eq!(
            (r.path, r.source),
            (PathBuf::from("f"), DataDirSource::Flag)
        );
        let r = resolve_data_dir_in(None, Some("e"), &root, None);
        assert_eq!((r.path, r.source), (PathBuf::from("e"), DataDirSource::Env));
        let r = resolve_data_dir_in(None, Some(""), &root, None);
        assert_eq!((r.path, r.source), (root.join("m"), DataDirSource::Marker));

        let bare = temp_dir();
        let r = resolve_data_dir_in(None, None, &bare, None);
        assert_eq!(
            (r.path, r.source),
            (PathBuf::from("data"), DataDirSource::Default)
        );
    }

    #[test]
    fn home_marker_is_ignored() {
        let home = temp_dir();
        std::fs::write(home.join(MARKER_FILE), "old-profile\n").unwrap();
        let proj = home.join("code").join("app");
        std::fs::create_dir_all(&proj).unwrap();
        assert!(find_marker(&proj, Some(&home)).is_none());
        assert_eq!(
            resolve_project_in(None, None, &proj, Some(&home)).as_deref(),
            Some("app")
        );
    }

    #[test]
    fn suggestion_is_a_sibling_of_the_git_root_not_the_cwd() {
        let root = temp_dir();
        let repo = root.join("app");
        let sub = repo.join("services").join("api");
        std::fs::create_dir_all(&sub).unwrap();
        git2::Repository::init(&repo).unwrap();

        assert_eq!(
            suggested_data_dir(&sub, None),
            Some(root.join("app.kanbanr"))
        );
        // Outside any repo, the folder itself is the root.
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(
            suggested_data_dir(&plain, None),
            Some(root.join("plain.kanbanr"))
        );
    }

    #[test]
    fn a_repo_at_or_above_home_is_not_the_project_root() {
        let home = temp_dir();
        git2::Repository::init(&home).unwrap(); // a dotfiles repo
        let proj = home.join("code").join("app");
        std::fs::create_dir_all(&proj).unwrap();
        assert_eq!(project_root(&proj, Some(&home)), proj);
        assert_eq!(
            enclosing_git_worktree(&proj.join("data"), Some(&home)),
            None
        );
    }

    #[test]
    fn nested_folders_are_flagged_and_siblings_are_not() {
        let root = temp_dir();
        let repo = root.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        git2::Repository::init(&repo).unwrap();

        assert_eq!(
            enclosing_git_worktree(&repo.join("data"), None),
            Some(repo.clone())
        );
        assert_eq!(
            enclosing_git_worktree(&root.join("app.kanbanr"), None),
            None
        );
        // A folder that is its own repo root doesn't count as nested.
        let own = repo.join("own");
        git2::Repository::init(&own).unwrap();
        assert_eq!(enclosing_git_worktree(&own, None), None);
    }

    #[test]
    fn nearby_data_dirs_lists_only_kanbanr_folders_with_projects() {
        let root = temp_dir();
        let repo = root.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(root.join("work.kanbanr").join("projects")).unwrap();
        std::fs::create_dir_all(root.join("empty.kanbanr")).unwrap();
        std::fs::create_dir_all(root.join("other").join("projects")).unwrap();
        assert_eq!(
            nearby_data_dirs(&repo, None),
            vec![root.join("work.kanbanr")]
        );
    }

    #[test]
    fn relative_to_prefers_short_relative_paths() {
        let root = temp_dir();
        let proj = root.join("a").join("app");
        std::fs::create_dir_all(&proj).unwrap();
        assert_eq!(
            relative_to(&root.join("a").join("app.kanbanr"), &proj),
            PathBuf::from("../app.kanbanr")
        );
        assert_eq!(relative_to(&proj, &proj), PathBuf::from("."));
        assert_eq!(
            relative_to(&proj.join("data"), &proj),
            PathBuf::from("data")
        );
        // Far away → absolute.
        let far = Path::new("/");
        assert_eq!(relative_to(far, &proj), PathBuf::from("/"));
    }

    #[test]
    fn tilde_expands_only_as_a_prefix() {
        let home = Path::new("/home/u");
        assert_eq!(
            expand_tilde("~/boards", Some(home)),
            PathBuf::from("/home/u/boards")
        );
        assert_eq!(expand_tilde("~", Some(home)), PathBuf::from("/home/u"));
        assert_eq!(expand_tilde("~bob/x", Some(home)), PathBuf::from("~bob/x"));
        assert_eq!(expand_tilde("rel", Some(home)), PathBuf::from("rel"));
    }
}
