//! The run's own remotes (FTR-037 W3): two private repositories made with `gh repo create` when a
//! `--remotes <prefix>` is given, else two bare repositories under the run folder. Either way they
//! are written to `remotes.env`, which is the only list of places a run may push to, and the
//! created ones are appended to `<runs>/remotes.created.tsv` so `remotes list` and `remotes clean`
//! can find them later.

use super::guard::{RemotesEnv, RunDirs};
use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

/// The file beside the runs that lists every repository a run created on the remote server.
pub const CREATED_FILE: &str = "remotes.created.tsv";

/// One repository a run created: when, which run, which one, where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub at: String,
    pub run: String,
    pub which: String,
    pub url: String,
}

/// Where `remotes.created.tsv` lives: beside the run folders.
pub fn created_file(runs_dir: &Path) -> PathBuf {
    runs_dir.join(CREATED_FILE)
}

fn append_created(runs_dir: &Path, rows: &[Created]) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(runs_dir)?;
    let mut f = super::append_to(created_file(runs_dir))?;
    for r in rows {
        writeln!(f, "{}\t{}\t{}\t{}", r.at, r.run, r.which, r.url)?;
    }
    Ok(())
}

/// Every repository the runs under `runs_dir` created, oldest first.
pub fn list(runs_dir: &Path) -> Vec<Created> {
    let Ok(text) = std::fs::read_to_string(created_file(runs_dir)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let mut it = l.split('\t');
            Some(Created {
                at: it.next()?.to_string(),
                run: it.next()?.to_string(),
                which: it.next()?.to_string(),
                url: it.next()?.to_string(),
            })
        })
        .collect()
}

/// `gh <args>`: stdout on success, the error text otherwise.
fn gh(gh: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new(gh)
        .args(args)
        .output()
        .with_context(|| format!("could not run {}", gh.display()))?;
    if !out.status.success() {
        return Err(anyhow!(
            "gh {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Make the run's two remotes and write `remotes.env`. With a prefix (`owner` or `owner/name-`),
/// `gh repo create <prefix><run>-code --private` and `-board`, recorded in `remotes.created.tsv`;
/// without one, two bare repositories under `<run>/remotes/`. `gh_program` is `gh` on PATH, or
/// the one a test names.
pub fn create(run: &RunDirs, prefix: Option<&str>, gh_program: &Path) -> Result<RemotesEnv> {
    let remotes = match prefix {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            let mut made = Vec::new();
            let mut urls = Vec::new();
            for which in ["code", "board"] {
                let name = format!("{prefix}/{}-{which}", run.name());
                let out = gh(gh_program, &["repo", "create", &name, "--private"])?;
                // `gh repo create` prints the repository's URL as its last line.
                let url = out
                    .lines()
                    .rev()
                    .find(|l| l.contains("://") || l.contains('@'))
                    .map(str::trim)
                    .map(str::to_string)
                    .unwrap_or_else(|| name.clone());
                made.push(Created {
                    at: kanbanr_core::now_rfc3339(),
                    run: run.name(),
                    which: which.to_string(),
                    url: url.clone(),
                });
                urls.push(url);
            }
            let runs_dir = run
                .root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| run.root.clone());
            append_created(&runs_dir, &made)?;
            RemotesEnv {
                code: urls[0].clone(),
                board: urls[1].clone(),
            }
        }
        None => {
            let dir = run.local_remotes();
            let mut paths = Vec::new();
            for which in ["code", "board"] {
                let p = dir.join(format!("{which}.git"));
                if !p.join("HEAD").exists() {
                    std::fs::create_dir_all(&p)?;
                    kanbanr_core::coderepo::git(&p, &["init", "--bare", "-q", "-b", "main"])
                        .map_err(|e| anyhow!("{e}"))?;
                }
                paths.push(p.display().to_string());
            }
            RemotesEnv {
                code: paths[0].clone(),
                board: paths[1].clone(),
            }
        }
    };
    remotes.write(run)?;
    Ok(remotes)
}

/// What `clean` did: the repositories deleted, and those left with the reason each.
#[derive(Debug, Default)]
pub struct Cleaned {
    pub deleted: Vec<Created>,
    pub left: Vec<(Created, String)>,
}

/// Delete every repository the runs created, with `gh repo delete --yes`. Those it could not
/// delete (no scope on the token, already gone) are returned for the person to remove; the file is
/// rewritten with only those.
pub fn clean(runs_dir: &Path, gh_program: &Path) -> Result<Cleaned> {
    let mut deleted = Vec::new();
    let mut left = Vec::new();
    for c in list(runs_dir) {
        match gh(gh_program, &["repo", "delete", &c.url, "--yes"]) {
            Ok(_) => deleted.push(c),
            Err(e) => left.push((c, e.to_string())),
        }
    }
    let _ = std::fs::remove_file(created_file(runs_dir));
    if !left.is_empty() {
        append_created(
            runs_dir,
            &left.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>(),
        )?;
    }
    Ok(Cleaned { deleted, left })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walkthrough::guard::allowed_remotes;

    fn scratch(name: &str) -> (RunDirs, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "kanbanr-wt-remotes-{}-{}-{name}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&base);
        let run = RunDirs::new(&base.join("runs").join(name));
        run.make().unwrap();
        (run, base)
    }

    /// FTR-037 W3: with no prefix the run's remotes are two bare repositories under the run folder,
    /// `remotes.env` names them, the push guard allows exactly those two, and a push to one works.
    #[test]
    fn without_a_prefix_two_bare_remotes_are_made_under_the_run() {
        let (run, base) = scratch("bare");
        let remotes = create(&run, None, Path::new("gh-that-is-not-run")).unwrap();
        for p in [&remotes.code, &remotes.board] {
            let p = Path::new(p);
            assert!(p.starts_with(run.local_remotes()), "{}", p.display());
            assert_eq!(
                kanbanr_core::coderepo::git(p, &["rev-parse", "--is-bare-repository"]).unwrap(),
                "true"
            );
        }
        assert_eq!(RemotesEnv::read(&run).unwrap(), remotes);
        assert_eq!(
            allowed_remotes(&run),
            [remotes.code.clone(), remotes.board.clone()]
        );
        assert!(
            list(&base.join("runs")).is_empty(),
            "nothing was created on a server"
        );
        // A second call keeps them.
        assert_eq!(
            create(&run, None, Path::new("gh-that-is-not-run")).unwrap(),
            remotes
        );
        // A push to the run's own remote lands.
        let project = run.project();
        kanbanr_core::coderepo::git(&project, &["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(project.join("a.txt"), "a\n").unwrap();
        kanbanr_core::coderepo::git(&project, &["add", "."]).unwrap();
        kanbanr_core::coderepo::git(
            &project,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-qm",
                "a",
            ],
        )
        .unwrap();
        kanbanr_core::coderepo::git(&project, &["remote", "add", "origin", &remotes.code]).unwrap();
        crate::walkthrough::guard::git_push(&project, "origin", "main").unwrap();
        let refs =
            kanbanr_core::coderepo::git(Path::new(&remotes.code), &["branch", "--list"]).unwrap();
        assert!(refs.contains("main"), "{refs}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// With a prefix, `gh repo create --private` is run once per repository (a script stands in for
    /// `gh`: nothing reaches any server), the URLs it prints go to `remotes.env`, and both are
    /// recorded in `remotes.created.tsv`, which `list` reads and `clean` empties through `gh repo
    /// delete --yes`.
    #[cfg(unix)]
    #[test]
    fn with_a_prefix_gh_makes_two_private_repositories_and_records_them() {
        use std::os::unix::fs::PermissionsExt;
        let (run, base) = scratch("gh");
        let log = base.join("gh-calls.log");
        let fake = base.join("gh");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}\nif [ \"$1\" = repo ] && [ \"$2\" = create ]; then echo \"https://example.invalid/$3\"; fi\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let remotes = create(&run, Some("someone"), &fake).unwrap();
        assert_eq!(
            remotes.code,
            format!("https://example.invalid/someone/{}-code", run.name())
        );
        assert!(remotes.board.ends_with("-board"), "{}", remotes.board);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.matches("repo create").count(), 2, "{calls}");
        assert_eq!(calls.matches("--private").count(), 2, "{calls}");
        let runs_dir = base.join("runs");
        let created = list(&runs_dir);
        assert_eq!(created.len(), 2);
        assert_eq!(created[0].which, "code");
        assert_eq!(created[1].which, "board");
        assert_eq!(created[0].run, run.name());
        let cleaned = clean(&runs_dir, &fake).unwrap();
        assert_eq!(cleaned.deleted.len(), 2);
        assert!(cleaned.left.is_empty());
        assert!(list(&runs_dir).is_empty());
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.matches("repo delete").count(), 2, "{calls}");
        assert!(calls.contains("--yes"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
