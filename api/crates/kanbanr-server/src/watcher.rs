//! Filesystem watcher: turns board file changes into broadcast events keyed by project id, and
//! into `slices` events that say which parts of the board changed (DEF-081).

use crate::memo;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;

/// How long the watcher collects a project's changes before it sends one `slices` event: one CLI
/// write touches 2–6 files within about 50 ms (DEF-081).
pub const COALESCE_MS: u64 = 150;

/// What changed on one project's board, as the `slices` SSE event carries it (DEF-081, A1 shared
/// API §2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Slices {
    pub project: String,
    /// The project's generation when the event was sent.
    pub r#gen: u64,
    pub tags: Vec<String>,
}

/// The slice tags a changed path under `projects/<p>/` names (A1 shared API §2). `rel` is the path
/// below the project folder, with `/` separators on every platform. Matching is on whole path
/// components (L-23). A temporary file of an atomic write names nothing (DEF-025); a path this
/// table does not know, or one that is not plain components, names `*`.
pub fn tags_for(rel: &str) -> Vec<String> {
    let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let star = || vec!["*".to_string()];
    if segs.is_empty() || segs.iter().any(|s| *s == "." || *s == "..") {
        return star();
    }
    if segs
        .last()
        .is_some_and(|s| s.ends_with(kanbanr_core::atomic::TEMP_SUFFIX))
    {
        return Vec::new();
    }
    let one = |t: &str| vec![t.to_string()];
    match segs.as_slice() {
        ["features", _, file] => match file.strip_suffix(".yaml") {
            Some(code) if !code.is_empty() => vec![format!("item:{code}"), "items".to_string()],
            _ => star(),
        },
        ["features", _, "features-spec", file] => match file.strip_suffix(".md") {
            Some(code) if !code.is_empty() => vec![format!("spec:{code}")],
            _ => star(),
        },
        ["index.yaml"] => one("items"),
        ["config.yaml"] => one("config"),
        ["releases.yaml"] => one("releases"),
        ["sprints.yaml"] => one("sprints"),
        ["charter.yaml"] => one("charter"),
        ["lessons.yaml"] => one("lessons"),
        ["decisions.yaml"] => one("decisions"),
        ["scopes.yaml"] => one("scopes"),
        ["system-baseline.yaml"] => one("catalog"),
        ["milestones", _, ..] => one("milestones"),
        ["timings", _, ..] => one("timings"),
        [
            folder @ ("prepared" | "manual" | "gates" | "pins" | "activity" | "events"),
            _,
            ..,
        ] => one(folder),
        ["docs", rest @ ..] if !rest.is_empty() => {
            let mut tags = vec![format!("doc:{}", rest.join("/")), "docs".to_string()];
            if rest.len() > 1 && rest[0] == "decisions" {
                tags.push("adrs".to_string());
            }
            tags
        }
        _ => star(),
    }
}

/// Watch `projects_dir` recursively; on any change, broadcast the affected project id (and always
/// "*" so the projects-list view refreshes too), as before. Also (DEF-081): bump `gens` for each
/// changed project at once, so the memo misses from then on, and collect each project's slice tags
/// over `COALESCE_MS` from its first change, then send one `Slices` on `slices_tx`. A tokio task does
/// the collecting; the notify callback only forwards `(project, tags)` into a channel and never
/// blocks. A change with no project (a file directly in the projects folder) bumps every project.
/// Must be called inside a tokio runtime.
pub fn spawn(
    projects_dir: PathBuf,
    tx: broadcast::Sender<String>,
    slices_tx: broadcast::Sender<Slices>,
    gens: Arc<memo::Generations>,
) -> anyhow::Result<RecommendedWatcher> {
    let base = projects_dir.clone();
    // macOS reports an event under the folder's resolved path (`/private/var/…` for `/var/…`),
    // Linux and Windows under the path as watched: match either (DEF-062).
    let resolved = std::fs::canonicalize(&projects_dir).ok();
    let (fwd, rx) = mpsc::unbounded_channel::<(String, Vec<String>)>();
    tokio::spawn(coalesce(rx, slices_tx, Arc::clone(&gens)));
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let mut bumped: BTreeSet<String> = BTreeSet::new();
            for path in &event.paths {
                let found = project_rel(path, &base)
                    .or_else(|| resolved.as_deref().and_then(|r| project_rel(path, r)));
                match found {
                    Some((id, rel)) => {
                        if bumped.insert(id.clone()) {
                            gens.bump(&id);
                        }
                        let _ = tx.send(id.clone());
                        let tags = tags_for(&rel);
                        if !tags.is_empty() {
                            let _ = fwd.send((id, tags));
                        }
                    }
                    None => gens.bump_all(),
                }
            }
            let _ = tx.send("*".to_string());
        }
    })?;
    watcher.watch(&projects_dir, RecursiveMode::Recursive)?;
    Ok(watcher)
}

/// Collect `(project, tags)` and send one `Slices` per project `COALESCE_MS` after the first change
/// of its burst, holding the union of the burst's tags. Ends when the watcher is dropped.
async fn coalesce(
    mut rx: mpsc::UnboundedReceiver<(String, Vec<String>)>,
    slices_tx: broadcast::Sender<Slices>,
    gens: Arc<memo::Generations>,
) {
    let window = Duration::from_millis(COALESCE_MS);
    let mut pending: HashMap<String, (BTreeSet<String>, Instant)> = HashMap::new();
    let flush = |pending: &mut HashMap<String, (BTreeSet<String>, Instant)>, all: bool| {
        let now = Instant::now();
        let due: Vec<String> = pending
            .iter()
            .filter(|(_, (_, first))| all || *first + window <= now)
            .map(|(p, _)| p.clone())
            .collect();
        for project in due {
            if let Some((tags, _)) = pending.remove(&project) {
                let _ = slices_tx.send(Slices {
                    r#gen: gens.of(Some(&project)),
                    project,
                    tags: tags.into_iter().collect(),
                });
            }
        }
    };
    loop {
        let next = pending.values().map(|(_, first)| *first + window).min();
        let got = match next {
            Some(at) => tokio::select! {
                got = rx.recv() => Some(got),
                _ = tokio::time::sleep_until(at) => None,
            },
            None => Some(rx.recv().await),
        };
        match got {
            Some(Some((project, tags))) => {
                pending
                    .entry(project)
                    .or_insert_with(|| (BTreeSet::new(), Instant::now()))
                    .0
                    .extend(tags);
            }
            Some(None) => {
                flush(&mut pending, true);
                return;
            }
            None => flush(&mut pending, false),
        }
    }
}

/// The project id (first path component under projects_dir) of a changed path, and the rest of
/// the path below the project folder with `/` separators.
fn project_rel(path: &Path, base: &Path) -> Option<(String, String)> {
    let rel = path.strip_prefix(base).ok()?;
    let mut comps = rel.components();
    let id = match comps.next()? {
        Component::Normal(s) => s.to_str()?.to_string(),
        _ => return None,
    };
    let rest: Vec<String> = comps
        .map(|c| match c {
            Component::Normal(s) => s.to_string_lossy().into_owned(),
            Component::ParentDir => "..".to_string(),
            _ => ".".to_string(),
        })
        .collect();
    Some((id, rest.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A watcher on `base` with its own channels and generations.
    fn watch(
        base: &Path,
    ) -> (
        RecommendedWatcher,
        broadcast::Receiver<String>,
        broadcast::Receiver<Slices>,
        Arc<memo::Generations>,
    ) {
        let (tx, rx) = broadcast::channel(256);
        let (stx, srx) = broadcast::channel(256);
        let gens = Arc::new(memo::Generations::default());
        let w = spawn(base.to_path_buf(), tx, stx, Arc::clone(&gens)).unwrap();
        (w, rx, srx, gens)
    }

    /// Wait up to 5 s for `id` on the `changed` channel.
    async fn heard(rx: &mut broadcast::Receiver<String>, id: &str) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            while let Ok(got) = rx.try_recv() {
                if got == id {
                    return true;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }

    /// DEF-025: a board file is now replaced by a rename, not modified in place; the page must
    /// still hear about it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_atomic_write_still_fires_a_change_event() {
        let base = std::env::temp_dir().join(format!("kanbanr-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let folder = base.join("demo");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("item.yaml");
        std::fs::write(&file, "old").unwrap();
        let (_watcher, mut rx, _srx, _gens) = watch(&base);
        tokio::time::sleep(Duration::from_millis(200)).await;
        kanbanr_core::atomic::write(&file, "new").unwrap();
        assert!(
            heard(&mut rx, "demo").await,
            "no change event for the project after an atomic write"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// DEF-062 R-2: a board watched through a linked folder still hears an atomic write, though
    /// macOS reports the event under the folder's resolved path.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_board_under_a_linked_folder_still_fires_a_change_event() {
        let root = std::env::temp_dir().join(format!("kanbanr-watch-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let real = root.join("real");
        std::fs::create_dir_all(real.join("demo")).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let file = link.join("demo").join("item.yaml");
        std::fs::write(&file, "old").unwrap();
        let (_watcher, mut rx, _srx, _gens) = watch(&link);
        tokio::time::sleep(Duration::from_millis(200)).await;
        kanbanr_core::atomic::write(&file, "new").unwrap();
        assert!(
            heard(&mut rx, "demo").await,
            "no change event for the project under a linked folder"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// DEF-081 R-7: every row of the slice table, a temporary file and a path that is not plain
    /// components.
    #[test]
    fn a_changed_path_maps_to_its_slice_tags() {
        let rows: &[(&str, &[&str])] = &[
            ("features/Planned/FTR-1.yaml", &["item:FTR-1", "items"]),
            ("features/In Progress/DEF-2.yaml", &["item:DEF-2", "items"]),
            ("features/Planned/features-spec/FTR-1.md", &["spec:FTR-1"]),
            ("index.yaml", &["items"]),
            ("config.yaml", &["config"]),
            ("milestones/MS-001.yaml", &["milestones"]),
            ("docs/notes/a.md", &["doc:notes/a.md", "docs"]),
            ("docs/x.md", &["doc:x.md", "docs"]),
            (
                "docs/decisions/x.md",
                &["doc:decisions/x.md", "docs", "adrs"],
            ),
            ("releases.yaml", &["releases"]),
            ("sprints.yaml", &["sprints"]),
            ("charter.yaml", &["charter"]),
            ("lessons.yaml", &["lessons"]),
            ("decisions.yaml", &["decisions"]),
            ("scopes.yaml", &["scopes"]),
            ("system-baseline.yaml", &["catalog"]),
            ("prepared/FTR-1.yaml", &["prepared"]),
            ("manual/FTR-1.yaml", &["manual"]),
            ("gates/v1.yaml", &["gates"]),
            ("pins/a.yaml", &["pins"]),
            ("activity/2026-11.yaml", &["activity"]),
            ("events/e.yaml", &["events"]),
            ("timings/20261116T172031Z-0123456.yaml", &["timings"]),
            ("processes/p.yaml", &["*"]),
            ("", &["*"]),
            ("x/../y", &["*"]),
            ("./config.yaml", &["*"]),
            ("features/Planned/.FTR-1.yaml.123.0.tmp", &[]),
            ("docs/decisions/.x.md.9.1.tmp", &[]),
            ("features/Planned/notes.txt", &["*"]),
        ];
        for (rel, want) in rows {
            let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
            assert_eq!(tags_for(rel), want, "tags for {rel:?}");
        }
        // Components, not substrings: a folder merely named like one is not it (L-23).
        assert_eq!(tags_for("my-docs/a.md"), vec!["*".to_string()]);
        assert_eq!(tags_for("config.yaml.bak"), vec!["*".to_string()]);
    }

    /// DEF-081 R-7: three atomic writes in one burst send one `Slices` holding the union of their
    /// tags, and the project's generation was bumped before it arrived. An atomic write syncs to
    /// disk, so three take about 50 ms; the burst only has to land inside the coalescing window.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_burst_of_changes_sends_one_slices_event() {
        let base = std::env::temp_dir().join(format!("kanbanr-watch-burst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let project = base.join("demo");
        let status = project.join("features").join("Planned");
        std::fs::create_dir_all(status.join("features-spec")).unwrap();
        let item = status.join("FTR-1.yaml");
        let spec = status.join("features-spec").join("FTR-1.md");
        let config = project.join("config.yaml");
        for f in [&item, &spec, &config] {
            std::fs::write(f, "old").unwrap();
        }
        let (_watcher, _rx, mut srx, gens) = watch(&base);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let before = gens.of(Some("demo"));
        let started = std::time::Instant::now();
        kanbanr_core::atomic::write(&item, "new").unwrap();
        kanbanr_core::atomic::write(&spec, "new").unwrap();
        kanbanr_core::atomic::write(&config, "new").unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(COALESCE_MS),
            "the three writes took {:?}, longer than one coalescing window",
            started.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(COALESCE_MS * 6)).await;
        let mut got = Vec::new();
        while let Ok(s) = srx.try_recv() {
            got.push(s);
        }
        assert_eq!(got.len(), 1, "one slices event per burst, got {got:?}");
        let s = &got[0];
        assert_eq!(s.project, "demo");
        assert_eq!(
            s.tags,
            vec!["config", "item:FTR-1", "items", "spec:FTR-1"],
            "the union of the burst's tags"
        );
        assert!(s.r#gen > before, "the generation was bumped");
        assert_eq!(s.r#gen, gens.of(Some("demo")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
