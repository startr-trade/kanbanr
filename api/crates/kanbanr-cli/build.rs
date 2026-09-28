//! Stamp the build's provenance into the binary (FEAT-088).
//!
//! `--version` alone answers "which release is this?" and not "which build?". Those differ whenever
//! a release is rebuilt under the same tag — an interim fix, a corrected packaging step — which is
//! exactly the case `self-update` exists to notice. The commit hash is the human-readable half of
//! that answer; the binary's checksum is the machine-readable one.
//!
//! Everything here degrades to `unknown` rather than failing the build: a crates.io tarball has no
//! `.git`, and a build that breaks because git is missing is worse than a version string that says
//! so plainly.

use std::process::Command;

fn main() {
    // Re-run when HEAD moves. `.git/HEAD` covers a checkout; the packed ref covers a commit on the
    // current branch. Neither existing is fine — the fallbacks below handle it.
    for path in [".git/HEAD", ".git/refs/heads"] {
        let p = std::path::Path::new("../../..").join(path);
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
    println!("cargo:rerun-if-env-changed=KANBANR_GIT_SHA");

    // CI can pass it in (a shallow clone or an exported tree may have no usable git metadata).
    let sha = std::env::var("KANBANR_GIT_SHA")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| git(&["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());

    // A build from a modified tree is not the commit it names, and saying so is the whole point.
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty());
    let sha = if dirty && sha != "unknown" {
        format!("{sha}-dirty")
    } else {
        sha
    };

    let date = git(&["log", "-1", "--format=%cd", "--date=short"])
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=KANBANR_GIT_SHA={sha}");
    println!("cargo:rustc-env=KANBANR_BUILD_DATE={date}");
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir("../../..")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}
