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
        if let Ok(v @ crate::yaml::Value::Object(_)) = crate::yaml::from_str(content)
            && let Ok(m) = serde_json::from_value::<Marker>(v)
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
                crate::yaml::to_string(self).unwrap_or_default()
            ),
        }
    }

    /// What this marker points at, in words: `project <p>, board <dir>`.
    pub fn summary(&self) -> String {
        format!(
            "project {}, board {}",
            self.project.as_deref().unwrap_or("<none>"),
            self.data_dir.as_deref().unwrap_or("<not recorded>")
        )
    }

    /// Whether `init` may replace this marker with `wanted` (FEAT-085). The marker is the only
    /// pointer from a folder to its board, so it is data, not scratch state: one that names
    /// another project or board is replaced only when forced, and the refusal says what to run.
    pub fn may_repoint(&self, wanted: &Marker, force: bool) -> crate::Result<()> {
        if force || self == wanted {
            return Ok(());
        }
        Err(crate::CoreError::Refused(format!(
            "this folder already names a board, and init would replace it:\n  \
             now:      {}\n  \
             would be: {}\n\
             Nothing was changed. To switch project within the SAME board, use \
             `kanbanr project use <name>`; to repoint deliberately, re-run with `--force`.",
            self.summary(),
            wanted.summary()
        )))
    }
}

/// How a data folder is written into a marker in `marker_dir`: relative when nearby, else absolute,
/// with forward slashes ([`portable_path`]).
pub fn marker_path(data_dir: &Path, marker_dir: &Path) -> String {
    portable_path(
        &relative_to(data_dir, marker_dir).display().to_string(),
        cfg!(windows),
    )
}

/// A path as a marker stores it: with forward slashes, which every platform reads (FEAT-129). A
/// marker written on Windows said `data_dir: ..\app.kanbanr`, which Linux and macOS read as one
/// file name — so a marker committed from one machine broke the board on the next.
pub fn portable_path(path: &str, windows: bool) -> String {
    if windows {
        path.replace('\\', "/")
    } else {
        path.to_string()
    }
}

/// The questions a setup here must put to the user (FEAT-171, FEAT-173, FEAT-194): `where --json`'s
/// `ask_the_user`. Said by the tool, because a walkthrough showed Claude filling these in itself
/// when the skill alone asked it to ask: each has an obvious answer, and an obvious answer is still
/// the user's to give. `tracked`: this folder already points at an existing board.
/// `git_identity`: git has a name and an email here.
pub fn setup_questions(tracked: bool, git_identity: bool) -> Vec<String> {
    let mut ask: Vec<String> = Vec::new();
    if tracked {
        return ask;
    }
    ask.push(
        "Where the board lives: suggested_data_dir first, then each of existing_data_dirs, or a \
         path the user types."
            .into(),
    );
    ask.push(
        "Which process — only after the board is chosen, in a later question, because the \
         options depend on it: that board's saved processes first (`kanbanr process list \
         --json --data-dir <board>`, library board, then personal; recommend one when the board \
         has it), then the built-in ones, or designing their own."
            .into(),
    );
    // FEAT-194 Q1: asked for every process, because the git model follows from it.
    ask.push(
        "Whether to work in sprints and releases — only after the process is chosen, because a \
         process may switch them on (scrum, agile): recommend its own setting. Then show the \
         resulting git model (`kanbanr git model --sprints on|off --releases on|off`): where \
         items branch from and merge into, what kanbanr merges, and that pushing is always the \
         user's — before the plan applies it."
            .into(),
    );
    if !git_identity {
        ask.push(
            "This repository's git identity — only after the board's commit identity is \
             given, in a later question: git has none here, so nobody can commit after setup. \
             Offer that identity: Yes (this repository only, never --global), another \
             name/email, or No (the first commit only)."
                .into(),
        );
    }
    ask
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
    ///
    /// A linked git worktree inside the repository (`.claude/worktrees/w1`) checks out its own copy
    /// of the marker, so a relative path resolves from the worktree and finds nothing. When that
    /// happens, the same relative path is resolved from the main worktree's matching folder, where
    /// the board actually is (FEAT-192). Anything that exists as written, or is absolute or `~`,
    /// is used as written.
    pub fn data_dir(&self, home: Option<&Path>) -> Option<PathBuf> {
        let raw = self.marker.data_dir.as_deref()?;
        let resolved = self.dir().join(expand_tilde(raw, home));
        let relative = !Path::new(raw).is_absolute() && !raw.starts_with('~');
        if resolved.exists() || !relative {
            return Some(resolved);
        }
        let fallback = main_worktree_equivalent(self.dir())
            .map(|dir| dir.join(raw))
            .filter(|p| p.exists());
        Some(fallback.unwrap_or(resolved))
    }
}

/// The folder in the main worktree that corresponds to `dir` in a linked git worktree, or `None`
/// when `dir` is not inside a linked worktree (FEAT-192).
fn main_worktree_equivalent(dir: &Path) -> Option<PathBuf> {
    let repo = git2::Repository::discover(dir).ok()?;
    if !repo.is_worktree() {
        return None;
    }
    let worktree_root = normalize(repo.workdir()?);
    // The common dir is the main repository's `.git`; its parent is the main worktree.
    let main_root = normalize(repo.commondir()).parent()?.to_path_buf();
    let inside = normalize(dir);
    let rel = inside.strip_prefix(&worktree_root).ok()?;
    Some(main_root.join(rel))
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

/// The environment variable that confines kanbanr to boards inside one directory (FEAT-195).
pub const FENCE_VAR: &str = "KANBANR_BOARD_FENCE";

/// The fence a `KANBANR_BOARD_FENCE` value names: `temp` is the system temporary directory, any
/// other value a directory. None when unset or blank.
pub fn fence_from(value: Option<&str>) -> Option<PathBuf> {
    let value = value.map(str::trim).filter(|v| !v.is_empty())?;
    let dir = if value == "temp" {
        std::env::temp_dir()
    } else {
        PathBuf::from(value)
    };
    Some(canonical_ish(&dir))
}

/// The directory boards must be inside, from `KANBANR_BOARD_FENCE`. Cargo sets it to `temp` for
/// every test (api/.cargo/config.toml), so no test can reach a real board.
pub fn fence() -> Option<PathBuf> {
    fence_from(std::env::var(FENCE_VAR).ok().as_deref())
}

/// `dir` made absolute and canonical as far as it exists: its nearest existing ancestor is
/// canonicalised (symlinks, macOS's `/var` → `/private/var`) and the rest appended.
fn canonical_ish(dir: &Path) -> PathBuf {
    let absolute = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(dir)
    };
    // `..` resolved first, by name: a folder not yet made cannot be canonicalised, and its `..`
    // parts would otherwise be carried through and climb out of the fence unseen.
    let mut lexical = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other),
        }
    }
    let absolute = lexical;
    let mut rest = Vec::new();
    let mut cur = absolute.as_path();
    loop {
        if let Ok(real) = cur.canonicalize() {
            return rest.iter().rev().fold(real, |p, part| p.join(part));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                cur = parent;
            }
            _ => return absolute,
        }
    }
}

/// Is `dir` inside `fence`? The reason, naming both, when it is not.
pub fn check_fence(dir: &Path, fence: &Path) -> std::result::Result<(), String> {
    let real = canonical_ish(dir);
    if real.starts_with(fence) {
        Ok(())
    } else {
        Err(format!(
            "refusing board {}: outside {FENCE_VAR} ({})",
            real.display(),
            fence.display()
        ))
    }
}

/// Stop the process if `dir` is outside the fence (FEAT-195). The fence is set only under test,
/// where a board outside it is a test about to write someone's real data. A panic rather than an
/// error, on purpose: an error can be caught and the run carried on with a fallback board; a panic
/// ends it before anything is read or written. Without a fence this does nothing.
pub fn assert_inside_fence(dir: &Path) {
    if let Some(fence) = fence()
        && let Err(why) = check_fence(dir, &fence)
    {
        panic!("{why}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_unset_or_blank_is_none() {
        assert_eq!(fence_from(None), None);
        assert_eq!(fence_from(Some("  ")), None);
    }

    #[test]
    fn fence_temp_is_the_temporary_directory() {
        let fence = fence_from(Some("temp")).unwrap();
        assert_eq!(fence, std::env::temp_dir().canonicalize().unwrap());
    }

    #[test]
    fn fence_inside_passes_and_outside_is_refused() {
        let fence = fence_from(Some("temp")).unwrap();
        let inside = std::env::temp_dir().join(format!("kanbanr-fence-{}", std::process::id()));
        assert!(
            check_fence(&inside, &fence).is_ok(),
            "a folder not yet made is judged by where it would be"
        );
        let outside = Path::new(env!("CARGO_MANIFEST_DIR"));
        let why = check_fence(outside, &fence).unwrap_err();
        assert!(why.contains("outside KANBANR_BOARD_FENCE"), "{why}");
        // `..` cannot climb out.
        let sneaky = inside.join("..").join("..").join("home");
        assert!(
            check_fence(&sneaky, &fence).is_err(),
            "{}",
            sneaky.display()
        );
    }
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// FEAT-192: inside a linked worktree, a relative board path that does not exist from there
    /// resolves from the main worktree; one that exists, or a missing board, is left as written.
    #[test]
    fn linked_worktree_falls_back_only_when_missing() {
        let base = temp_dir();
        let main = base.join("shop");
        std::fs::create_dir_all(&main).unwrap();
        let repo = git2::Repository::init(&main).unwrap();
        // A worktree needs a commit to check out.
        let sig = git2::Signature::now("T", "t@x").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        write_marker(
            &main,
            &Marker {
                project: Some("shop".into()),
                data_dir: Some("../shop.kanbanr".into()),
            },
        )
        .unwrap();
        let board = base.join("shop.kanbanr");
        std::fs::create_dir_all(board.join("projects")).unwrap();
        let wt_dir = main.join(".claude").join("worktrees").join("w1");
        std::fs::create_dir_all(wt_dir.parent().unwrap()).unwrap();
        repo.worktree("w1", &wt_dir, None).unwrap();
        // The worktree has its own copy of the marker (it would be committed in a real repo).
        std::fs::copy(main.join(MARKER_FILE), wt_dir.join(MARKER_FILE)).unwrap();

        let found = find_marker(&wt_dir, None).unwrap();
        assert_eq!(normalize(&found.data_dir(None).unwrap()), normalize(&board));
        // From the main checkout: as written.
        let found = find_marker(&main, None).unwrap();
        assert_eq!(normalize(&found.data_dir(None).unwrap()), normalize(&board));
        // No board anywhere: the answer is the path as written, so "no board here" still says so.
        std::fs::remove_dir_all(&board).unwrap();
        let found = find_marker(&wt_dir, None).unwrap();
        assert!(!found.data_dir(None).unwrap().exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A fresh, canonical temp dir (so path comparisons aren't tripped by symlinked /tmp).
    fn temp_dir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "kanbanr-project-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        // As `normalize` resolves: on Windows, without canonicalize's `\\?\` prefix (FEAT-129).
        without_verbatim_prefix(std::fs::canonicalize(p).unwrap())
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
        // Far away → absolute: the root, resolved — `/` here, `D:\` (the current drive) on Windows.
        let far = Path::new("/");
        assert_eq!(relative_to(far, &proj), normalize(far));
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

    /// FEAT-085, FEAT-176: init replaces a marker that points elsewhere only when forced, and
    /// the refusal names both pointers.
    #[test]
    fn a_marker_is_repointed_only_when_forced() {
        let now = Marker {
            project: Some("shop".into()),
            data_dir: Some("../shop.kanbanr".into()),
        };
        let other = Marker {
            project: Some("shop".into()),
            data_dir: None,
        };
        assert!(now.may_repoint(&now.clone(), false).is_ok());
        assert!(now.may_repoint(&other, true).is_ok());
        let why = now.may_repoint(&other, false).unwrap_err().to_string();
        assert!(why.contains("project shop, board ../shop.kanbanr"), "{why}");
        assert!(why.contains("project shop, board <not recorded>"), "{why}");
    }

    /// FEAT-171, FEAT-173, FEAT-194: a tracked folder is asked nothing; an untracked one is asked
    /// where, which process and its cadence, and for git's identity only when git has none.
    #[test]
    fn setup_asks_what_only_the_user_can_answer() {
        assert!(setup_questions(true, false).is_empty());
        assert_eq!(setup_questions(false, true).len(), 3);
        let all = setup_questions(false, false);
        assert_eq!(all.len(), 4);
        assert!(all[2].contains("sprints and releases"));
        assert!(all[3].contains("git identity"));
    }
    /// FEAT-129: Windows' CI run showed two path forms leaking out — a marker with backslashes,
    /// which no other platform can read, and canonicalize's `\\?\` verbatim prefix in messages.
    #[test]
    fn paths_written_and_shown_are_portable() {
        assert_eq!(portable_path(r"..\app.kanbanr", true), "../app.kanbanr");
        assert_eq!(
            portable_path(r"C:\work\app.kanbanr", true),
            "C:/work/app.kanbanr"
        );
        // On Linux and macOS a backslash is an ordinary file-name character: left alone.
        assert_eq!(portable_path(r"odd\name", false), r"odd\name");

        let plain = |s: &str| without_verbatim_prefix(PathBuf::from(s));
        assert_eq!(
            plain(r"\\?\C:\Users\a\.claude"),
            PathBuf::from(r"C:\Users\a\.claude")
        );
        assert_eq!(
            plain(r"\\?\UNC\server\share"),
            PathBuf::from(r"\\?\UNC\server\share")
        );
        assert_eq!(plain("/home/a/.claude"), PathBuf::from("/home/a/.claude"));
    }
}
