//! The raw record of a run (FTR-037 W6, R-4): everything the run saw, kept as files under `raw/`
//! and listed in `MANIFEST.sha256` so nothing can go missing unnoticed. Ported from SPK-002's
//! `snapshot.sh` and `fetch-remote.sh`. A snapshot that fails leaves `raw/SNAPSHOT-FAILED` with the
//! reason and no manifest, never a manifest over half a capture.

use super::guard::{RemotesEnv, RunDirs, sha256_file};
use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

/// The manifest's file, in `raw/`.
pub const MANIFEST: &str = "MANIFEST.sha256";
/// The marker a failed snapshot leaves.
pub const FAILED: &str = "SNAPSHOT-FAILED";

/// `sha256  path` lines over every file in `raw/` but itself, paths relative to `raw/`, sorted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    pub entries: Vec<(String, String)>,
}

impl Manifest {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (rel, sum) in &self.entries {
            out.push_str(sum);
            out.push_str("  ");
            out.push_str(rel);
            out.push('\n');
        }
        out
    }

    pub fn parse(text: &str) -> Manifest {
        Manifest {
            entries: text
                .lines()
                .filter_map(|l| {
                    let (sum, rel) = l.split_once("  ")?;
                    Some((rel.to_string(), sum.to_string()))
                })
                .collect(),
        }
    }

    pub fn has(&self, rel: &str) -> bool {
        self.entries.iter().any(|(r, _)| r == rel)
    }
}

/// Every regular file under `dir`, as paths relative to it with `/`, sorted.
pub fn files_under(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.is_file() {
                out.push(
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Copy a folder, skipping `.git` folders.
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        if name == ".git" {
            continue;
        }
        let src = e.path();
        let dst = to.join(&name);
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else if src.is_file() {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// The slug Claude Code files a project's sessions under: the path with every `/` as `-`.
pub fn session_slug(project: &Path) -> String {
    project.to_string_lossy().replace(['/', '\\'], "-")
}

/// The `git ls-remote` listings of a remote: all refs, then tags only. A remote that cannot be
/// reached is recorded as such, never skipped silently.
fn ls_remote(remote: &str, tags_only: bool) -> String {
    let mut args = vec!["ls-remote"];
    if tags_only {
        args.push("--tags");
    }
    args.push(remote);
    match std::process::Command::new("git").args(&args).output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        Ok(out) => format!(
            "# unreachable: {}\n",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => format!("# unreachable: {e}\n"),
    }
}

fn do_snapshot(run: &RunDirs, remotes: Option<&RemotesEnv>) -> Result<()> {
    let raw = run.raw();
    std::fs::create_dir_all(&raw)?;
    // 1. The session files Claude Code kept for the run project.
    let sessions = run
        .claude_config()
        .join("projects")
        .join(session_slug(&run.project()));
    let transcripts = raw.join("transcripts");
    std::fs::create_dir_all(&transcripts)?;
    if let Ok(entries) = std::fs::read_dir(&sessions) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().is_some_and(|x| x == "jsonl") {
                std::fs::copy(&p, transcripts.join(e.file_name()))?;
            }
        }
    }
    // 2. Both repositories as bundles.
    for (name, repo) in [("code", run.project()), ("board", run.board())] {
        if repo.join(".git").exists() {
            let bundle = raw.join(format!("{name}.bundle"));
            kanbanr_core::coderepo::git(
                &repo,
                &["bundle", "create", &bundle.display().to_string(), "--all"],
            )
            .map_err(|e| anyhow!("bundle {name}: {e}"))?;
        }
    }
    // 3. The board's files, without .git.
    if run.board().is_dir() {
        let copy = raw.join("board");
        if copy.exists() {
            std::fs::remove_dir_all(&copy)?;
        }
        copy_tree(&run.board(), &copy)?;
    }
    // 4. What the remotes hold.
    if let Some(r) = remotes {
        let mut refs = String::new();
        let mut tags = String::new();
        for (which, url) in [("code", &r.code), ("board", &r.board)] {
            refs.push_str(&format!("# {which}\n{}", ls_remote(url, false)));
            tags.push_str(&format!("# {which}\n{}", ls_remote(url, true)));
        }
        std::fs::write(raw.join("remote-refs.txt"), refs)?;
        std::fs::write(raw.join("remote-tags.txt"), tags)?;
    }
    Ok(())
}

/// Snapshot the run: the session transcripts, both repositories as bundles, the board's files, the
/// remotes' refs and tags, beside what the run already wrote (`monitor.log`, `screenshots/`,
/// `questions.tsv`, `prepared.tsv`, `truth.txt`, `setup.log`, `findings.md`), then the manifest.
pub fn snapshot(run: &RunDirs, remotes: Option<&RemotesEnv>) -> Result<Manifest> {
    let raw = run.raw();
    let _ = std::fs::remove_file(raw.join(FAILED));
    let _ = std::fs::remove_file(raw.join(MANIFEST));
    if let Err(e) = do_snapshot(run, remotes) {
        std::fs::create_dir_all(&raw)?;
        let _ = std::fs::write(raw.join(FAILED), format!("{e}\n"));
        return Err(e.context("the snapshot failed; raw/ has SNAPSHOT-FAILED and no manifest"));
    }
    let manifest = manifest_of(&raw);
    std::fs::write(raw.join(MANIFEST), manifest.to_text())?;
    Ok(manifest)
}

/// The manifest `raw/` would have now: every file but the manifest itself.
pub fn manifest_of(raw: &Path) -> Manifest {
    Manifest {
        entries: files_under(raw)
            .into_iter()
            .filter(|rel| rel != MANIFEST)
            .map(|rel| {
                let sum = sha256_file(&raw.join(&rel));
                (rel, sum)
            })
            .collect(),
    }
}

/// Re-check `raw/MANIFEST.sha256`: every listed file is there with its checksum, and no file
/// under `raw/` is missing from the list. The error names each difference.
pub fn verify(run: &RunDirs) -> Result<()> {
    let raw = run.raw();
    let text = std::fs::read_to_string(raw.join(MANIFEST))
        .with_context(|| format!("no {MANIFEST} in {}", raw.display()))?;
    let listed = Manifest::parse(&text);
    let now = manifest_of(&raw);
    let mut problems = Vec::new();
    for (rel, sum) in &listed.entries {
        match now.entries.iter().find(|(r, _)| r == rel) {
            None => problems.push(format!("{rel}: missing")),
            Some((_, have)) if have != sum => problems.push(format!("{rel}: changed")),
            _ => {}
        }
    }
    for (rel, _) in &now.entries {
        if !listed.has(rel) {
            problems.push(format!("{rel}: not in the manifest"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "the manifest does not match raw/: {}",
            problems.join("; ")
        ))
    }
}

/// Where a run keeps its screenshots.
pub fn screenshots_dir(run: &RunDirs) -> PathBuf {
    run.raw().join("screenshots")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walkthrough::guard::tests::scratch_run;

    fn git(dir: &Path, args: &[&str]) -> String {
        kanbanr_core::coderepo::git(dir, args).unwrap()
    }

    /// FTR-037 R-4: the manifest lists every capture — the session transcripts, both bundles, the
    /// board copy, the remotes' refs and tags, the logs and tables already there — with its
    /// checksum, verify passes over it, and a file changed or added afterwards fails verify by name.
    #[test]
    fn the_manifest_lists_every_capture_and_verifies() {
        let (run, base) = scratch_run("snapshot");
        let remotes = RemotesEnv::read(&run).unwrap();
        // A code repository with a commit pushed to the run's remote, and a tag.
        let project = run.project();
        git(&project, &["init", "-q", "-b", "main"]);
        std::fs::write(project.join("app.py"), "print('shop')\n").unwrap();
        git(&project, &["add", "."]);
        git(
            &project,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-qm",
                "shop",
            ],
        );
        git(&project, &["tag", "v0.1.0"]);
        git(&project, &["remote", "add", "origin", &remotes.code]);
        crate::walkthrough::guard::git_push(&project, "origin", "main").unwrap();
        crate::walkthrough::guard::git_push(&project, "origin", "v0.1.0").unwrap();
        // A board, as a repository with files.
        let board = run.board();
        std::fs::create_dir_all(board.join("projects/shop")).unwrap();
        std::fs::write(board.join("projects/shop/project.yaml"), "id: shop\n").unwrap();
        kanbanr_core::git::ensure_repo_as(&board, Some(("Tester", "t@example.com")));
        // A session file where Claude Code keeps them.
        let sessions = run
            .claude_config()
            .join("projects")
            .join(session_slug(&project));
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("abc.jsonl"), "{\"type\":\"user\"}\n").unwrap();
        // What the run wrote already.
        let raw = run.raw();
        std::fs::create_dir_all(raw.join("screenshots")).unwrap();
        for (name, text) in [
            ("monitor.log", "kanbanr view daemon on http://127.0.0.1:1\n"),
            ("questions.tsv", "time\tstep\n"),
            ("prepared.tsv", "time\tstep\n"),
            ("truth.txt", "OK x | board: a | truth: a\n"),
            ("setup.log", "model sonnet\n"),
            ("findings.md", "- none\n"),
            ("screenshots/1.1-before.png", "png"),
        ] {
            std::fs::write(raw.join(name), text).unwrap();
        }
        let manifest = snapshot(&run, Some(&remotes)).unwrap();
        for wanted in [
            "transcripts/abc.jsonl",
            "code.bundle",
            "board.bundle",
            "board/projects/shop/project.yaml",
            "remote-refs.txt",
            "remote-tags.txt",
            "monitor.log",
            "questions.tsv",
            "prepared.tsv",
            "truth.txt",
            "setup.log",
            "findings.md",
            "screenshots/1.1-before.png",
        ] {
            assert!(
                manifest.has(wanted),
                "{wanted} is not in the manifest: {manifest:?}"
            );
        }
        assert!(!manifest.has(MANIFEST), "the manifest never lists itself");
        assert!(
            !raw.join("board/.git").exists(),
            "the board copy has no .git"
        );
        let tags = std::fs::read_to_string(raw.join("remote-tags.txt")).unwrap();
        assert!(tags.contains("refs/tags/v0.1.0"), "{tags}");
        let text = std::fs::read_to_string(raw.join(MANIFEST)).unwrap();
        assert!(
            text.lines().all(|l| l.len() > 66 && &l[64..66] == "  "),
            "{text}"
        );
        assert_eq!(Manifest::parse(&text), manifest);
        verify(&run).unwrap();
        // A changed file and an added file are both named.
        std::fs::write(raw.join("truth.txt"), "MISMATCH x | board: a | truth: b\n").unwrap();
        std::fs::write(raw.join("extra.txt"), "late\n").unwrap();
        let why = verify(&run).unwrap_err().to_string();
        assert!(why.contains("truth.txt: changed"), "{why}");
        assert!(why.contains("extra.txt: not in the manifest"), "{why}");
        // The bundle holds the tag: the capture is the repository, not a listing.
        let heads = git(
            &project,
            &[
                "bundle",
                "list-heads",
                &raw.join("code.bundle").display().to_string(),
            ],
        );
        assert!(heads.contains("refs/tags/v0.1.0"), "{heads}");
        assert!(!raw.join(FAILED).exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_failed_snapshot_leaves_the_marker_and_no_manifest() {
        let (run, base) = scratch_run("failed");
        // A board folder whose `.git` is a file git cannot bundle.
        std::fs::create_dir_all(run.board()).unwrap();
        std::fs::write(run.board().join(".git"), "not a repository\n").unwrap();
        let err = snapshot(&run, None).unwrap_err().to_string();
        assert!(err.contains("SNAPSHOT-FAILED"), "{err}");
        assert!(run.raw().join(FAILED).exists());
        assert!(!run.raw().join(MANIFEST).exists());
        assert_eq!(session_slug(Path::new("/tmp/x/project")), "-tmp-x-project");
        let _ = std::fs::remove_dir_all(&base);
    }
}
