//! The commit stamp follows HEAD in a linked git worktree (DEF-058).
//!
//! `build.rs` once watched `.git/HEAD` and `.git/refs/heads`, which exist only in a plain clone; in
//! a worktree nothing was watched and a rebuild kept the first build's commit. This test clones the
//! repository into a scratch folder, adds a worktree, checks the CLI around an empty commit and
//! reads the stamp the build script emitted.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// Run git in `dir`, panicking with the command and its stderr on failure.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The scratch folder; removes the worktree and the folder on the way out, failure included.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(self.0.join("wt"))
            .current_dir(self.0.join("main"))
            .output();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn empty_commit(wt: &Path, msg: &str) -> String {
    git(
        wt,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@x",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            msg,
        ],
    );
    git(wt, &["rev-parse", "--short=12", "HEAD"])
}

fn target_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("CARGO_TARGET_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    // target/debug/deps/<exe>
    let exe = std::env::current_exe().expect("current_exe");
    exe.ancestors().nth(3).expect("target dir").to_path_buf()
}

fn cargo_check(wt: &Path, target: &Path) {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| env!("CARGO").to_string());
    let out = Command::new(cargo)
        .args(["check", "--locked", "-p", "kanbanr-cli"])
        .current_dir(wt.join("api"))
        .env("CARGO_TARGET_DIR", target)
        .env_remove("KANBANR_GIT_SHA")
        .env_remove("KANBANR_BUILD_DATE")
        .output()
        .expect("run cargo");
    assert!(
        out.status.success(),
        "cargo check failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The one `kanbanr-cli` build script output naming `sha`.
fn output_naming(target: &Path, sha: &str) -> PathBuf {
    let want = format!("cargo:rustc-env=KANBANR_GIT_SHA={sha}");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(target.join("debug/build"))
        .expect("build dir")
        .flatten()
    {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("kanbanr-cli-")
        {
            continue;
        }
        let out = entry.path().join("output");
        if std::fs::read_to_string(&out).is_ok_and(|t| t.lines().any(|l| l == want)) {
            found.push(out);
        }
    }
    assert_eq!(
        found.len(),
        1,
        "expected one build output naming {sha}: {found:?}"
    );
    found.remove(0)
}

fn under(path: &Path, base: &Path) -> bool {
    let (p, b): (Vec<Component>, Vec<Component>) =
        (path.components().collect(), base.components().collect());
    p.len() > b.len() && p[..b.len()] == b[..]
}

#[test]
#[ignore = "checks kanbanr-cli twice in a scratch worktree (a minute or two): cargo test -p kanbanr-cli --test build_stamp -- --ignored; the release gate runs it"]
fn build_in_a_worktree_stamps_the_moved_head() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    assert!(
        repo.join(".git").exists(),
        "no git repository at {}: nothing to prove",
        repo.display()
    );
    let scratch = std::env::temp_dir().join(format!("kanbanr-build-stamp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let scratch = scratch.canonicalize().unwrap();
    let _guard = Scratch(scratch.clone());
    let main = scratch.join("main");
    let wt = scratch.join("wt");

    git(
        &scratch,
        &[
            "clone",
            "-q",
            "--no-local",
            repo.to_str().unwrap(),
            main.to_str().unwrap(),
        ],
    );
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            wt.to_str().unwrap(),
            "HEAD",
        ],
    );

    let target = target_dir();
    let sha1 = empty_commit(&wt, "[no-ref] stamp 1");
    cargo_check(&wt, &target);
    let output = output_naming(&target, &sha1);

    let text = std::fs::read_to_string(&output).unwrap();
    let watched: Vec<PathBuf> = text
        .lines()
        .filter_map(|l| l.strip_prefix("cargo:rerun-if-changed="))
        .map(PathBuf::from)
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    let main_git = main.join(".git");
    assert!(
        watched
            .iter()
            .any(|p| p.ends_with("HEAD") && under(p, &main_git.join("worktrees"))),
        "no worktree HEAD watched: {watched:?}"
    );
    assert!(
        watched
            .iter()
            .any(|p| p.ends_with("refs/heads") && under(p, &main_git)),
        "no refs/heads watched: {watched:?}"
    );

    let sha2 = empty_commit(&wt, "[no-ref] stamp 2");
    cargo_check(&wt, &target);
    let text = std::fs::read_to_string(&output).unwrap();
    assert!(
        text.lines()
            .any(|l| l == format!("cargo:rustc-env=KANBANR_GIT_SHA={sha2}")),
        "stamp did not follow HEAD to {sha2}: {text}"
    );
}
