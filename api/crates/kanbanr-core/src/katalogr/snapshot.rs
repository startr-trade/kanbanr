//! The release snapshot (FTR-016 T6, R-7, R-8, R-9): at the cut, `katalogr view snapshot` runs as
//! a child, and its folder is stored on the board under `docs/releases/<V>/views/`. Everything
//! here is a value: a missing or failing katalogr leaves a `skipped` record with the reason and
//! never stops the promotion (ADR-0014). katalogr never writes to a board; kanbanr copies.

use super::Settings;
use super::client::PositionRef;
use crate::releases::Release;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What a release's cut saved, or why it saved nothing. Kept in `releases.yaml` beside the tag.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleaseSnapshot {
    /// `saved` or `skipped`.
    #[serde(default)]
    pub state: String,
    /// The docs folder holding the views, e.g. `releases/v0.8.0/views`.
    #[serde(default)]
    pub dir: String,
    /// The position the views were taken at: the tag's commit.
    #[serde(default)]
    pub at: String,
    /// The repos the snapshot names, for the person to check before pushing a public board.
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub blocks: u32,
    #[serde(default)]
    pub views: u32,
    /// Why it was skipped; empty when saved.
    #[serde(default)]
    pub reason: String,
}

impl ReleaseSnapshot {
    pub fn skipped(reason: &str) -> ReleaseSnapshot {
        ReleaseSnapshot {
            state: "skipped".to_string(),
            reason: reason.to_string(),
            ..Default::default()
        }
    }

    pub fn is_saved(&self) -> bool {
        self.state == "saved"
    }

    /// The line the promote command prints before `next:`.
    pub fn sentence(&self) -> String {
        if !self.is_saved() {
            return format!("no system views saved: {}", self.reason);
        }
        let repos = if self.repos.is_empty() {
            "no repos named".to_string()
        } else {
            format!("repos named: {}", self.repos.join(", "))
        };
        format!(
            "system views: {} block(s), {} view(s) saved in docs/{} — {repos} (check before pushing the board)",
            self.blocks, self.views, self.dir
        )
    }
}

/// The `index.json` katalogr writes (FEAT-035), read leniently.
#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotIndex {
    pub schema: u32,
    #[serde(default)]
    pub positions: Vec<PositionRef>,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub blocks: Vec<SnapshotBlock>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotBlock {
    pub doc: String,
    pub line: u32,
    pub source: String,
    pub view_id: String,
    #[serde(default)]
    pub partial: bool,
}

#[cfg(test)]
thread_local! {
    static TEST_SETTINGS: std::cell::RefCell<Option<Settings>> = const { std::cell::RefCell::new(None) };
}

/// The settings a promotion uses: the environment's. Under test, a thread's own settings (set by
/// [`with_test_settings`]) or, when none were set, a katalogr that is never found, so no test
/// depends on what is installed on the machine.
pub(crate) fn current_settings() -> Settings {
    #[cfg(test)]
    {
        if let Some(s) = TEST_SETTINGS.with(|c| c.borrow().clone()) {
            return s;
        }
        let mut s = Settings::from_env();
        s.bin = "kanbanr-test-no-such-katalogr".to_string();
        s
    }
    #[cfg(not(test))]
    {
        Settings::from_env()
    }
}

/// Run `f` with these settings on this thread (tests only: the environment is shared by the
/// process's threads).
#[cfg(test)]
pub(crate) fn with_test_settings<T>(settings: Settings, f: impl FnOnce() -> T) -> T {
    TEST_SETTINGS.with(|c| *c.borrow_mut() = Some(settings));
    let out = f();
    TEST_SETTINGS.with(|c| *c.borrow_mut() = None);
    out
}

/// Is `settings.bin` on PATH (or an absolute or relative path that exists)? `None` when not, so
/// the caller records "katalogr is not installed" and goes on.
pub fn find_binary(settings: &Settings) -> Option<PathBuf> {
    let bin = settings.bin.trim();
    if bin.is_empty() {
        return None;
    }
    let given = Path::new(bin);
    if given.is_absolute() || given.components().count() > 1 {
        return runnable(given).then(|| given.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
    for dir in std::env::split_paths(&path) {
        let plain = dir.join(bin);
        if runnable(&plain) {
            return Some(plain);
        }
        for ext in &exts {
            let with = dir.join(format!("{bin}{ext}"));
            if runnable(&with) {
                return Some(with);
            }
        }
    }
    None
}

fn runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Run `<bin> view snapshot --docs <docs> --at <at> --out <out> --json` with cwd `repo`, within
/// `settings.snapshot_secs`; on exit 0 parse `out/index.json`. Any other outcome is `Err(reason)`
/// with katalogr's own last stderr line (katalogr redacts its messages) or "timed out after N s"
/// (the child is killed), and `out` is removed. Never a shell: fixed arguments.
pub fn run_snapshot(
    settings: &Settings,
    repo: &Path,
    docs: &Path,
    at: &str,
    out: &Path,
) -> Result<SnapshotIndex, String> {
    let Some(bin) = find_binary(settings) else {
        return Err("katalogr is not installed (nothing named katalogr on PATH)".to_string());
    };
    let fail = |reason: String| {
        let _ = std::fs::remove_dir_all(out);
        Err(reason)
    };
    let mut child = match Command::new(&bin)
        .args(["view", "snapshot", "--docs"])
        .arg(docs)
        .args(["--at", at, "--out"])
        .arg(out)
        .arg("--json")
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return fail(format!("katalogr could not be started: {}", e.kind())),
    };
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut text = String::new();
            let mut bytes = Vec::new();
            let _ = (&mut err).take(64 * 1024).read_to_end(&mut bytes);
            text.push_str(&String::from_utf8_lossy(&bytes));
            let _ = tx.send(text);
        });
    }
    let limit = Duration::from_secs(settings.snapshot_secs.max(1));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return fail(format!("timed out after {} s", settings.snapshot_secs));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return fail(format!("katalogr could not be waited for: {}", e.kind())),
        }
    };
    if !status.success() {
        let stderr = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        let last = stderr
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| l.chars().take(300).collect::<String>());
        return fail(last.unwrap_or_else(|| match status.code() {
            Some(c) => format!("katalogr exited {c} without saying why"),
            None => "katalogr was stopped before it finished".to_string(),
        }));
    }
    let parsed = std::fs::read(out.join("index.json"))
        .map_err(|_| "katalogr wrote no index.json".to_string())
        .and_then(|b| {
            serde_json::from_slice::<SnapshotIndex>(&b)
                .map_err(|_| "katalogr's index.json is not a snapshot index".to_string())
        });
    match parsed {
        Ok(index) => Ok(index),
        Err(reason) => fail(reason),
    }
}

/// A file name this store takes: `[A-Za-z0-9._-]+` with an extension, so the docs store keeps the
/// name as it is (it adds `.md` to a name without one).
fn storable_name(name: &str) -> bool {
    !name.is_empty()
        && name.contains('.')
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The spellings of `path` a file could hold: as given, resolved, and with forward slashes.
fn spellings(path: &Path) -> Vec<String> {
    let mut all = vec![path.to_string_lossy().to_string()];
    if let Ok(real) = std::fs::canonicalize(path) {
        let text = real.to_string_lossy().to_string();
        // Windows' verbatim prefix (L-46): a file may hold the path without it.
        if let Some(plain) = text.strip_prefix(r"\\?\") {
            all.push(plain.to_string());
        }
        all.push(text);
    }
    let more: Vec<String> = all.iter().map(|s| s.replace('\\', "/")).collect();
    all.extend(more);
    all.retain(|s| s.len() > 1 && s != "/");
    all.sort();
    all.dedup();
    all
}

/// Copy every regular file of `out` (one level, no symlinks, names of `[A-Za-z0-9._-]+`) into the
/// board doc folder `releases/<version>/views/` through `Store::write_doc_bytes`, refusing a file
/// whose bytes hold an absolute path of `repo`, `docs` or `out` (every path in `forbidden`): a
/// snapshot may land on a public board. A refusal is a `skipped` record with nothing stored.
pub fn store_snapshot(
    store: &Store,
    project: &str,
    version: &str,
    at: &str,
    index: &SnapshotIndex,
    out: &Path,
    forbidden: &[&Path],
) -> crate::Result<ReleaseSnapshot> {
    let entries = std::fs::read_dir(out)?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = std::fs::symlink_metadata(entry.path())?;
        if !meta.is_file() || !storable_name(&name) {
            return Ok(ReleaseSnapshot::skipped(&format!(
                "the snapshot holds something this version will not store ({name}); nothing stored"
            )));
        }
        files.push((name, std::fs::read(entry.path())?));
    }
    files.sort();
    let names: Vec<String> = forbidden.iter().flat_map(|p| spellings(p)).collect();
    for (_, bytes) in &files {
        let text = String::from_utf8_lossy(bytes);
        if names.iter().any(|n| text.contains(n.as_str())) {
            return Ok(ReleaseSnapshot::skipped(
                "the snapshot named an absolute path; nothing stored",
            ));
        }
    }
    let dir = format!("releases/{version}/views");
    let mut written: Vec<String> = Vec::new();
    for (name, bytes) in &files {
        let rel = format!("{dir}/{name}");
        if let Err(e) = store.write_doc_bytes(project, &rel, bytes) {
            remove_stored(store, project, version, &written);
            return Err(e);
        }
        written.push(rel);
    }
    let views: BTreeSet<&str> = index.blocks.iter().map(|b| b.view_id.as_str()).collect();
    Ok(ReleaseSnapshot {
        state: "saved".to_string(),
        dir,
        at: at.to_string(),
        repos: index.repos.clone(),
        blocks: u32::try_from(index.blocks.len()).unwrap_or(u32::MAX),
        views: u32::try_from(views.len()).unwrap_or(u32::MAX),
        reason: String::new(),
    })
}

/// Take back the files `store_snapshot` wrote (and the folder, when empty).
fn remove_stored(store: &Store, project: &str, version: &str, written: &[String]) {
    for rel in written {
        let _ = store.delete_doc(project, rel);
    }
    remove_snapshot_folder(store, project, version);
}

/// Remove `docs/releases/<version>/views` and every file in it: a failed promotion leaves no
/// partial snapshot behind (R-8).
pub fn remove_snapshot_folder(store: &Store, project: &str, version: &str) {
    let dir = store
        .project_dir(project)
        .join("docs")
        .join("releases")
        .join(version)
        .join("views");
    let _ = std::fs::remove_dir_all(&dir);
    if let Some(parent) = dir.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

/// What the tag does: find the binary, pick a temp folder under `std::env::temp_dir()`, run and
/// store, or a `skipped` record with the reason. Never errors: the promotion goes on.
/// `tag_commit` is `Release.tag_commit` as `tag_release` just recorded it (empty: skipped).
pub fn snapshot_at_tag(
    store: &Store,
    project: &str,
    version: &str,
    repo: Option<&Path>,
    tag_commit: &str,
    settings: &Settings,
) -> ReleaseSnapshot {
    let (Some(repo), false) = (repo, tag_commit.is_empty()) else {
        return ReleaseSnapshot::skipped(
            "no tagged commit at hand: the snapshot needs the project folder and the tag",
        );
    };
    if find_binary(settings).is_none() {
        return ReleaseSnapshot::skipped(
            "katalogr is not installed (nothing named katalogr on PATH)",
        );
    }
    let safe: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = std::env::temp_dir().join(format!("kanbanr-snapshot-{}-{safe}", std::process::id()));
    let out = base.join("views");
    let _ = std::fs::remove_dir_all(&base);
    let docs = store.project_dir(project).join("docs");
    let record = (|| {
        std::fs::create_dir_all(&base).map_err(|e| format!("no temp folder: {}", e.kind()))?;
        let index = run_snapshot(settings, repo, &docs, tag_commit, &out)?;
        store_snapshot(
            store,
            project,
            version,
            tag_commit,
            &index,
            &out,
            &[repo, &docs, &out, &base],
        )
        .map_err(|e| format!("the snapshot could not be stored: {e}"))
    })();
    let _ = std::fs::remove_dir_all(&base);
    record.unwrap_or_else(|reason| ReleaseSnapshot::skipped(&reason))
}

/// "## System views" for the notes doc at ship time: the folder and the repos, or nothing when the
/// release has no saved snapshot.
pub fn system_views_section(release: &Release) -> String {
    let Some(s) = release.snapshot.as_ref().filter(|s| s.is_saved()) else {
        return String::new();
    };
    let repos = if s.repos.is_empty() {
        "No repos are named.".to_string()
    } else {
        format!(
            "Repos named: {}. Check them before pushing a public board.",
            s.repos.join(", ")
        )
    };
    let at: String = s.at.chars().take(12).collect();
    format!(
        "\n## System views\n\nThe cut saved katalogr's views of the project's docs in `docs/{}`: {} block(s), {} view(s), at {at}.\n\n{repos}\n",
        s.dir, s.blocks, s.views
    )
}

/// One sentence for `ReleasePlan.warnings` at the cut: whether tagging will save system views.
/// Read-only: a PATH walk and a count of the query blocks in the project's docs.
pub fn cut_warning(store: &Store, project: &str, version: &str, settings: &Settings) -> String {
    let Some(bin) = find_binary(settings) else {
        return "katalogr is not installed: no system views are saved".to_string();
    };
    let blocks = count_blocks(store, project);
    if blocks == 0 {
        return "no query blocks in this project's docs: nothing to snapshot".to_string();
    }
    format!(
        "katalogr found at {}: tagging saves the system views of {blocks} query block(s) into docs/releases/{version}/views",
        bin.display()
    )
}

fn count_blocks(store: &Store, project: &str) -> usize {
    let Ok(docs) = store.list_docs(project) else {
        return 0;
    };
    docs.iter()
        .filter(|d| crate::diagrams::is_markdown_path(d))
        .filter_map(|d| store.read_doc(project, d).ok())
        .map(|text| super::query::find_query_blocks(&text).len())
        .sum()
}
