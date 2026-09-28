//! Replace this binary with the one the release publishes (FEAT-088).
//!
//! * **Only when asked.** Nothing checks on a timer or as a side effect of another command.
//! * **Two reasons to update, not one.** A newer tag is the obvious one. The other is a release
//!   *rebuilt under the same tag* — an interim fix, a corrected packaging step — which a version
//!   comparison cannot see. `--check` compares the running binary's SHA-256 against the checksum
//!   the release publishes for this platform, so a republished asset is visible rather than
//!   reported as "nothing to do".
//! * **Always verified, twice.** The archive is checked against the release's `SHA256SUMS`, and the
//!   binary extracted from it against the per-binary checksum. A mismatch aborts and there is no
//!   `--force` past it: someone who genuinely wants an unverified binary can download it by hand
//!   and see what they are doing.
//! * **Atomic.** The new binary is staged next to the current one and moved in by rename, so an
//!   interrupted update cannot leave a half-written executable on `PATH`. Windows cannot rename
//!   over a running image, so the old one is moved aside first.
//!
//! The arithmetic ([`decide`], [`target_triple`], [`asset_name`]) is pure and unit-tested; only
//! [`fetch`] and [`install`] touch the network or the filesystem.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const REPO: &str = "startr-trade/kanbanr";

/// Why an update is on offer — or why it is not. The distinction matters to the person reading it:
/// "v0.1.0 -> v0.1.0" with no explanation reads as a bug rather than as a rebuild (R-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The running build is what the release publishes.
    UpToDate,
    /// A different release exists.
    NewVersion { from: String, to: String },
    /// Same tag, different binary: the release was rebuilt after this copy was installed.
    ///
    /// `same_commit` says WHY, which the checksum alone cannot. Same commit means the artifact was
    /// rebuilt or re-uploaded — a re-run workflow, a corrected packaging step, a toolchain bump —
    /// with no source change. A different commit means the tag was moved to point at other code,
    /// which is unusual enough to say out loud.
    Rebuilt { tag: String, same_commit: bool },
    /// The release publishes no checksum for this build, so a rebuild cannot be detected. Reported
    /// rather than assumed either way — see the note on `decide`.
    Unknown { tag: String, why: String },
}

impl Decision {
    pub fn update_available(&self) -> bool {
        matches!(self, Decision::NewVersion { .. } | Decision::Rebuilt { .. })
    }

    /// One line, saying which of the two reasons applies.
    pub fn describe(&self) -> String {
        match self {
            Decision::UpToDate => "up to date".into(),
            Decision::NewVersion { from, to } => format!("an update is available: {from} -> {to}"),
            Decision::Rebuilt {
                tag,
                same_commit: true,
            } => format!(
                "{tag} was rebuilt after this copy was installed — same version, same commit, \
                 different binary"
            ),
            Decision::Rebuilt {
                tag,
                same_commit: false,
            } => format!(
                "{tag} now points at a different commit than the one this binary was built from \
                 — same version, different source"
            ),
            Decision::Unknown { tag, why } => {
                format!("on {tag}; cannot tell whether it was rebuilt ({why})")
            }
        }
    }
}

/// The whole comparison, as a pure function so the interesting case is testable without a network.
///
/// **The checksum decides; the commit explains.** They answer different questions and neither
/// replaces the other:
///
/// * The **SHA-256 of the binary** identifies the *artifact*. It is the only thing that can answer
///   "is the binary I am running the one this release publishes?" — it catches a rebuild, a
///   re-upload, a corrupted install, and a locally built binary that merely shares a version.
/// * The **commit hash** identifies the *source*. It cannot decide the question, because two
///   different binaries routinely share one commit: re-running a release workflow after a transient
///   failure, a toolchain bump, a corrected packaging step. But when the checksums differ it is the
///   only thing that says *why*, and it is what the GitHub release page shows, so a person can match
///   what they are running against what is published.
///
/// So: `published_sha` vs `running_sha` decides; `published_commit` vs `running_commit` is carried
/// into the answer as the reason.
///
/// A missing `published_sha` is reported as [`Decision::Unknown`], never silently treated as
/// up-to-date: releases made before the checksum was published cannot answer the question, and
/// answering "you are current" on their behalf is the failure this whole item exists to remove.
pub fn decide(
    current_tag: &str,
    latest_tag: &str,
    published_sha: Option<&str>,
    running_sha: &str,
    published_commit: Option<&str>,
    running_commit: &str,
) -> Decision {
    let (current, latest) = (normalize(current_tag), normalize(latest_tag));
    if current != latest {
        return Decision::NewVersion {
            from: current.to_string(),
            to: latest.to_string(),
        };
    }
    match published_sha {
        None => Decision::Unknown {
            tag: latest.to_string(),
            why: "the release publishes no checksum for this platform's binary".into(),
        },
        Some(published) if published.eq_ignore_ascii_case(running_sha) => Decision::UpToDate,
        Some(_) => Decision::Rebuilt {
            tag: latest.to_string(),
            // Unknown published commit is treated as "same": the artifact differs either way, and
            // claiming the source moved without evidence would be the more misleading guess.
            same_commit: published_commit
                .map(|c| commits_match(c, running_commit))
                .unwrap_or(true),
        },
    }
}

/// Do two commit identifiers name the same commit? Abbreviations differ in length between what a
/// build stamps, what the API returns and what a release page shows, so the shorter is compared as
/// a prefix of the longer. A `-dirty` suffix means the build was not that commit at all.
fn commits_match(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().to_ascii_lowercase();
    let (a, b) = (norm(a), norm(b));
    // A build from a modified tree is not the commit it names, whatever the hex says, and a build
    // that could not determine its commit cannot claim to match one.
    if [&a, &b]
        .iter()
        .any(|s| s.is_empty() || s.ends_with("-dirty") || *s == "unknown")
    {
        return false;
    }
    let n = a.len().min(b.len());
    a[..n] == b[..n]
}

/// A tag or version string reduced to its comparable part: `v0.1.0` and `0.1.0` are the same
/// release, and `--version` carries build provenance after the number that is not part of it.
fn normalize(tag: &str) -> &str {
    tag.trim()
        .trim_start_matches('v')
        .split_whitespace()
        .next()
        .unwrap_or("")
}

/// The release target triple for the platform this binary was built for.
///
/// Derived from the compile-time constants rather than stamped by `build.rs`: these five are every
/// target `release.yml` publishes, and a sixth would need a release asset before it could be
/// installed anyway. `None` means "no published build for this platform", which is a thing to say
/// rather than a triple to guess.
pub fn target_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

/// The archive a release publishes for a target — `.zip` on Windows, `.tar.gz` elsewhere.
pub fn asset_name(tag: &str, target: &str) -> String {
    let ext = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("kanbanr-{tag}-{target}.{ext}")
}

/// The file holding that target's BINARY checksum — not the archive's, which is what `SHA256SUMS`
/// covers and what cannot answer "is the binary I am running current?".
pub fn binary_sum_name(tag: &str, target: &str) -> String {
    format!("kanbanr-{tag}-{target}.bin.sha256")
}

/// Lowercase hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("could not read {} to check it", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Pull one checksum out of a `SHA256SUMS` body, by the file it covers.
pub fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        // `sha256sum` writes `<hash>  <name>`; the second field may carry a `*` binary marker.
        let name = parts.next()?.trim_start_matches('*');
        (name == file).then(|| hash.to_ascii_lowercase())
    })
}

/// Stage `bytes` beside `target` and move it into place by rename (R-4).
///
/// Same filesystem by construction, so the rename is atomic: either the old binary or the new one
/// is on `PATH`, never half of either. Windows cannot rename over a running image, so the current
/// one is moved aside first and cleaned up on a later run.
pub fn replace_binary(target: &Path, bytes: &[u8]) -> Result<()> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let staged = dir.join(format!(".kanbanr-update-{}", std::process::id()));
    std::fs::write(&staged, bytes)
        .with_context(|| format!("could not stage the new binary in {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    if cfg!(windows) {
        let aside = dir.join("kanbanr.exe.old");
        let _ = std::fs::remove_file(&aside);
        std::fs::rename(target, &aside)
            .with_context(|| "could not move the running binary aside".to_string())?;
    }
    std::fs::rename(&staged, target).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        anyhow::anyhow!(
            "could not move the new binary into {}: {e}. \
             Nothing was replaced; try again with write access to that directory.",
            target.display()
        )
    })?;
    Ok(())
}

/// Verify `bytes` against `expected`, or refuse. There is deliberately no way past this (R-3).
pub fn verify(what: &str, bytes: &[u8], expected: &str) -> Result<()> {
    let got = sha256_bytes(bytes);
    if !got.eq_ignore_ascii_case(expected.trim()) {
        bail!(
            "CHECKSUM MISMATCH for {what}\n  expected {expected}\n  got      {got}\n\
             Nothing was installed. Do not use this download."
        );
    }
    Ok(())
}

/// Where the running executable lives, resolved through any symlink so a rename replaces the real
/// file rather than the link pointing at it.
pub fn running_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("could not find this executable's own path")?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// How many redirects the updater will follow before giving up.
const MAX_HOPS: usize = 5;

/// Is this a URL the updater may fetch? (FEAT-089/R-1)
///
/// Only `https`. Checksum verification does **not** make plaintext acceptable here: the checksum
/// file arrives over the same channel as the archive it vouches for, so anyone able to rewrite one
/// can rewrite the other. Verification only holds when the thing doing the vouching arrived over a
/// channel that was authenticated.
pub fn is_https(url: &str) -> bool {
    url.len() > 8 && url[..8].eq_ignore_ascii_case("https://")
}

/// Should this request carry the GitHub token?
///
/// Only `api.github.com`, which is the endpoint that rate-limits (60 requests an hour anonymous,
/// 5000 with one). The download host needs no credential and must not receive one, and neither does
/// a redirect target — an updater that forwards your token to every host it touches is a worse
/// trade than a slow retry. (R-3)
pub fn takes_token(url: &str) -> bool {
    url.starts_with("https://api.github.com/")
}

/// One HTTPS GET, following redirects **in this code** rather than in the HTTP client.
///
/// `ureq` follows five redirects on its own with no restriction on the scheme it redirects *to*,
/// and a release download IS a redirect by design — `github.com/.../releases/download/…` answers a
/// 302 to `objects.githubusercontent.com`. So the bytes that become the binary on your PATH used to
/// arrive from a hop nothing had looked at, and a redirect to `http://` would have been followed in
/// silence. Automatic following is off; every hop is checked here (R-2).
fn fetch(url: &str) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300))
        // Off, so a redirect cannot silently change the scheme underneath us.
        .redirects(0)
        .build();

    let mut current = url.to_string();
    // Only the first request may carry the token; a redirect never does, whatever it points at.
    let mut first = true;
    for _ in 0..=MAX_HOPS {
        if !is_https(&current) {
            bail!(
                "refusing to fetch {current} — updates are fetched over https only.\n\
                 An update downloaded over plaintext is not made safe by a checksum, because the \
                 checksum arrives the same way."
            );
        }
        let mut req = agent.get(&current).set("User-Agent", "kanbanr-self-update");
        if first
            && takes_token(&current)
            && let Ok(token) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN"))
            && !token.trim().is_empty()
        {
            req = req.set("Authorization", &format!("Bearer {}", token.trim()));
        }
        first = false;

        let resp = req
            .call()
            .with_context(|| format!("could not fetch {current}"))?;
        if let Some(next) = redirect_target(&resp, &current) {
            current = next;
            continue;
        }
        let mut bytes = Vec::new();
        std::io::copy(&mut resp.into_reader(), &mut bytes)
            .context("could not read the response")?;
        return Ok(bytes);
    }
    bail!("gave up after {MAX_HOPS} redirects starting at {url}")
}

/// The absolute URL a redirect response points at, if it is one.
///
/// Relative `Location` values are resolved against the current URL, because a server is entitled to
/// send one and treating it as an unknown scheme would refuse a perfectly good redirect.
fn redirect_target(resp: &ureq::Response, current: &str) -> Option<String> {
    if !(300..400).contains(&resp.status()) {
        return None;
    }
    resolve_location(resp.header("location")?, current)
}

/// Resolve a `Location` value against the URL it came from. Split out from [`redirect_target`] so
/// the interesting part — what a redirect can turn into — is testable without an HTTP response.
///
/// An absolute target passes through unchanged, so the scheme check in [`fetch`] sees exactly what
/// the server asked for rather than something this function normalised.
pub fn resolve_location(location: &str, current: &str) -> Option<String> {
    let location = location.trim();
    if location.is_empty() {
        return None;
    }
    if location.contains("://") {
        return Some(location.to_string());
    }
    let origin_end = current.find("://").map(|i| i + 3)?;
    let origin = match current[origin_end..].find('/') {
        Some(i) => &current[..origin_end + i],
        None => current,
    };
    Some(if location.starts_with('/') {
        format!("{origin}{location}")
    } else {
        format!("{origin}/{location}")
    })
}

/// The newest release tag, from three endpoints in order.
///
/// Each has a failure mode the next covers, learned by the installer: `/releases/latest` 404s while
/// every release is a pre-release, the release list has been observed answering an empty array
/// while the release was fetchable by tag, and tags are a different endpoint again. When all three
/// come up empty the caller is told to pin `--version`, which needs no lookup at all.
fn resolve_latest() -> Result<String> {
    let api = format!("https://api.github.com/repos/{REPO}");
    let tag_of = |body: &[u8]| -> Option<String> {
        let v: serde_json::Value = serde_json::from_slice(body).ok()?;
        let first = v.get(0).unwrap_or(&v);
        first.get("tag_name")?.as_str().map(str::to_string)
    };
    if let Ok(b) = fetch(&format!("{api}/releases/latest"))
        && let Some(t) = tag_of(&b)
    {
        return Ok(t);
    }
    if let Ok(b) = fetch(&format!("{api}/releases?per_page=1"))
        && let Some(t) = tag_of(&b)
    {
        return Ok(t);
    }
    if let Ok(b) = fetch(&format!("{api}/tags?per_page=100"))
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b)
        && let Some(arr) = v.as_array()
    {
        let mut tags: Vec<String> = arr
            .iter()
            .filter_map(|t| t.get("name")?.as_str().map(str::to_string))
            .filter(|n| n.starts_with('v'))
            .collect();
        tags.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
        if let Some(t) = tags.into_iter().next() {
            return Ok(t);
        }
    }
    bail!(
        "could not resolve a release tag from {api} (rate-limited, or nothing published yet).\n\
         Pin one instead: `kanbanr self-update --version v0.1.0`, and set GH_TOKEN to lift the \
         API rate limit if you are retrying."
    )
}

/// A sortable key for a `v`-prefixed tag: numeric segments compare as numbers, so v0.10.0 sorts
/// above v0.9.0 where a plain string compare would not.
fn version_key(tag: &str) -> Vec<u64> {
    normalize(tag)
        .split(['.', '-'])
        .map(|seg| seg.parse::<u64>().unwrap_or(0))
        .collect()
}

/// `kanbanr self-update`. Returns the decision so the caller can report it in whatever format.
pub fn run(check: bool, pinned: Option<&str>, json: bool) -> Result<()> {
    let Some(target) = target_triple() else {
        bail!(
            "no release is published for this platform ({} {}). Build from source: \
             `git clone https://github.com/{REPO}.git && cd kanbanr && make install`",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    };
    let current = env!("CARGO_PKG_VERSION");
    let tag = match pinned {
        Some(t) => t.to_string(),
        None => resolve_latest()?,
    };
    let dl = format!("https://github.com/{REPO}/releases/download/{tag}");

    // The BINARY's checksum, not the archive's — the running binary is the extracted file.
    let published = fetch(&format!("{dl}/{}", binary_sum_name(&tag, target)))
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .map(|s| {
            s.split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
        })
        .filter(|s| s.len() == 64);

    // What the release page shows for this tag. Only ever the *reason* in the answer — the checksum
    // decides — so a failure to fetch it degrades the explanation, never the verdict.
    let published_commit = fetch(&format!(
        "https://api.github.com/repos/{REPO}/git/ref/tags/{tag}"
    ))
    .ok()
    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    .and_then(|v| v.get("object")?.get("sha")?.as_str().map(|s| s.to_string()));

    let exe = running_binary()?;
    let running = sha256_file(&exe)?;
    let decision = decide(
        current,
        &tag,
        published.as_deref(),
        &running,
        published_commit.as_deref(),
        env!("KANBANR_GIT_SHA"),
    );

    if json {
        println!(
            "{}",
            serde_json::json!({
                "current": current,
                "build": env!("KANBANR_GIT_SHA"),
                "latest": normalize(&tag),
                "updateAvailable": decision.update_available(),
                "reason": match &decision {
                    Decision::UpToDate => "up-to-date",
                    Decision::NewVersion { .. } => "new-version",
                    Decision::Rebuilt { .. } => "rebuilt",
                    Decision::Unknown { .. } => "unknown",
                },
                "runningSha256": running,
                "publishedSha256": published,
                "runningCommit": env!("KANBANR_GIT_SHA"),
                "publishedCommit": published_commit,
            })
        );
    } else {
        println!("{}", decision.describe());
    }

    if check || !decision.update_available() {
        if !check && !json {
            println!("nothing to do");
        }
        return Ok(());
    }

    // Install: verify the archive against SHA256SUMS, then the binary against its own checksum.
    let asset = asset_name(&tag, target);
    if !json {
        println!("  downloading {asset}…");
    }
    let sums = String::from_utf8(fetch(&format!("{dl}/SHA256SUMS"))?)
        .context("SHA256SUMS was not valid UTF-8")?;
    let want = sum_for(&sums, &asset)
        .with_context(|| format!("SHA256SUMS carries no entry for {asset}"))?;
    let archive = fetch(&format!("{dl}/{asset}"))?;
    verify(&asset, &archive, &want)?;

    let binary = extract_binary(&archive, target)?;
    if let Some(expected) = &published {
        verify("the kanbanr binary", &binary, expected)?;
    }
    replace_binary(&exe, &binary)?;
    if !json {
        println!("  installed {} ({})", exe.display(), normalize(&tag));
        println!("run `kanbanr --version` to confirm");
    }
    Ok(())
}

/// Pull the `kanbanr` executable out of a release archive, in memory.
fn extract_binary(archive: &[u8], target: &str) -> Result<Vec<u8>> {
    if target.contains("windows") {
        bail!(
            "unpacking a .zip is not implemented here — on Windows, re-run the installer:\n  \
             irm https://github.com/{REPO}/releases/latest/download/install.ps1 | iex"
        );
    }
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in archive.entries().context("the archive could not be read")? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if path.file_name().is_some_and(|n| n == "kanbanr") {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes)?;
            return Ok(bytes);
        }
    }
    bail!("the archive did not contain a 'kanbanr' binary")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FEAT-088/R-1 and R-2 — the case this item exists for. A release re-uploaded under the same
    /// tag is invisible to a version comparison, which is how a version-only `differs()` behaves and why
    /// it would answer "nothing to do".
    #[test]
    fn a_rebuilt_release_at_the_same_version_is_an_update() {
        let running = "aa".repeat(32);
        let rebuilt = "bb".repeat(32);

        let commit = "01e5be91b80e";
        let other_commit = "ffffffffffff";

        // Same tag, same binary: nothing to do.
        assert_eq!(
            decide(
                "v0.1.0",
                "v0.1.0",
                Some(&running),
                &running,
                Some(commit),
                commit
            ),
            Decision::UpToDate
        );

        // Same tag, DIFFERENT binary: the release was rebuilt. A version compare cannot see this,
        // and neither can a commit compare — the commit is identical here.
        let d = decide(
            "v0.1.0",
            "v0.1.0",
            Some(&rebuilt),
            &running,
            Some(commit),
            commit,
        );
        assert_eq!(
            d,
            Decision::Rebuilt {
                tag: "0.1.0".into(),
                same_commit: true
            }
        );
        assert!(d.update_available());
        // R-2: the reason is named, or "0.1.0 -> 0.1.0" reads as a bug.
        assert!(d.describe().contains("rebuilt"), "{}", d.describe());
        assert!(d.describe().contains("same commit"), "{}", d.describe());

        // Same tag, different binary AND the tag now names other source: say that instead.
        let d = decide(
            "v0.1.0",
            "v0.1.0",
            Some(&rebuilt),
            &running,
            Some(other_commit),
            commit,
        );
        assert_eq!(
            d,
            Decision::Rebuilt {
                tag: "0.1.0".into(),
                same_commit: false
            }
        );
        assert!(
            d.describe().contains("different commit"),
            "{}",
            d.describe()
        );

        // The commit alone can NEVER decide: identical commit, identical checksum is up to date,
        // and identical commit with a different checksum is not. That asymmetry is the whole point.
        assert_eq!(
            decide(
                "v0.1.0",
                "v0.1.0",
                Some(&running),
                &running,
                Some(commit),
                commit
            ),
            Decision::UpToDate
        );

        // Abbreviations of different lengths are the same commit.
        let d = decide(
            "v0.1.0",
            "v0.1.0",
            Some(&rebuilt),
            &running,
            Some("01e5be91b80e4c5d6a7b"),
            "01e5be91b80e",
        );
        assert!(matches!(
            d,
            Decision::Rebuilt {
                same_commit: true,
                ..
            }
        ));
        // …but a dirty build is not the commit it names.
        let d = decide(
            "v0.1.0",
            "v0.1.0",
            Some(&rebuilt),
            &running,
            Some(commit),
            "01e5be91b80e-dirty",
        );
        assert!(matches!(
            d,
            Decision::Rebuilt {
                same_commit: false,
                ..
            }
        ));

        // A newer tag wins regardless of checksums, and says so differently.
        let d = decide(
            "v0.1.0",
            "v0.2.0",
            Some(&rebuilt),
            &running,
            Some(other_commit),
            commit,
        );
        assert_eq!(
            d,
            Decision::NewVersion {
                from: "0.1.0".into(),
                to: "0.2.0".into()
            }
        );
        assert!(d.describe().contains("0.1.0 -> 0.2.0"));

        // `v` prefixes and the build provenance `--version` carries are not part of the comparison.
        assert_eq!(
            decide(
                "0.1.0 (abc123, built 2026-09-28)",
                "v0.1.0",
                Some(&running),
                &running,
                Some(commit),
                commit,
            ),
            Decision::UpToDate
        );

        // No published checksum: say so. Never answer "you are current" on a release that cannot
        // be asked — that is the silence-as-absence failure this item removes. Note the commit
        // matches here, and it is still not enough to claim up-to-date.
        let d = decide("v0.1.0", "v0.1.0", None, &running, Some(commit), commit);
        assert!(matches!(d, Decision::Unknown { .. }));
        assert!(!d.update_available());
        assert!(d.describe().contains("cannot tell"), "{}", d.describe());
    }

    /// R-3: a mismatch aborts, and the message says what was expected rather than just "failed".
    #[test]
    fn a_checksum_mismatch_aborts_and_leaves_the_binary_alone() {
        let good = sha256_bytes(b"the real binary");
        verify("kanbanr", b"the real binary", &good).expect("a matching checksum must pass");
        // Case and surrounding whitespace are not a mismatch.
        verify(
            "kanbanr",
            b"the real binary",
            &format!("  {}  ", good.to_uppercase()),
        )
        .unwrap();

        let err = verify("kanbanr", b"something else", &good)
            .expect_err("a mismatch must abort")
            .to_string();
        assert!(err.contains("CHECKSUM MISMATCH"), "{err}");
        assert!(
            err.contains(&good),
            "the message must name what was expected: {err}"
        );
        assert!(err.contains("Nothing was installed"), "{err}");
    }

    /// R-4: the replacement is staged beside the target and renamed in, so a crash mid-write cannot
    /// leave a partial executable behind.
    #[test]
    fn the_replacement_is_staged_beside_the_target_and_renamed() {
        let dir = std::env::temp_dir().join(format!("kanbanr-selfupd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("kanbanr");
        std::fs::write(&target, b"old binary").unwrap();

        replace_binary(&target, b"new binary").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new binary");

        // Nothing staged is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".kanbanr-update-"))
            .collect();
        assert!(leftovers.is_empty(), "staging files left: {leftovers:?}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "the new binary must be executable");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// FEAT-089: every hop must be https, not just the URL we typed.
    ///
    /// `ureq` follows five redirects on its own with no scheme restriction, and a release download
    /// IS a redirect — so the bytes that become the binary arrived from a hop nothing had checked.
    /// A checksum does not rescue that: the checksum file comes over the same channel.
    #[test]
    fn only_https_is_fetched_and_a_downgrade_redirect_is_refused() {
        // R-1: what may be fetched at all.
        assert!(is_https("https://github.com/x"));
        assert!(
            is_https("HTTPS://github.com/x"),
            "the scheme is case-insensitive"
        );
        assert!(!is_https("http://github.com/x"));
        assert!(
            !is_https("http://github.com/x?u=https://y"),
            "not a prefix match on 'https'"
        );
        assert!(!is_https("ftp://github.com/x"));
        assert!(!is_https("https://"), "a scheme with no host is not a URL");
        // A URL that merely CONTAINS https is not an https URL.
        assert!(!is_https("//evil/?https://github.com"));

        // R-3: the token's blast radius. api.github.com only — never the download host, never a
        // redirect target, even one still on a github.com domain.
        assert!(takes_token(
            "https://api.github.com/repos/x/y/releases/latest"
        ));
        assert!(!takes_token(
            "https://github.com/x/y/releases/download/v1/a.tar.gz"
        ));
        assert!(!takes_token(
            "https://objects.githubusercontent.com/whatever"
        ));
        assert!(!takes_token("https://api.github.com.evil.test/repos"));
        assert!(!takes_token("http://api.github.com/repos"));

        // R-2: where a redirect is allowed to send us. Absolute targets pass through as-is so the
        // scheme check above sees exactly what the server asked for.
        let abs = |loc: &str| {
            let origin = "https://github.com/o/r/releases/download/v1/a.tar.gz";
            resolve_location(loc, origin)
        };
        assert_eq!(
            abs("https://objects.githubusercontent.com/a"),
            Some("https://objects.githubusercontent.com/a".into())
        );
        // The dangerous one: a downgrade is resolved, then refused by is_https.
        let downgrade = abs("http://objects.githubusercontent.com/a").unwrap();
        assert!(
            !is_https(&downgrade),
            "a downgrade must not pass the scheme check"
        );

        // Relative targets keep the current origin, which is https by construction.
        assert_eq!(
            abs("/o/r/other.tar.gz"),
            Some("https://github.com/o/r/other.tar.gz".into())
        );
        assert_eq!(
            abs("other.tar.gz"),
            Some("https://github.com/other.tar.gz".into())
        );
        assert_eq!(abs(""), None, "an empty Location is not a redirect");
    }

    /// R-5: nothing checks for updates unless asked. A tool that phones home as a side effect of
    /// an unrelated command is a tool people learn to run offline, so the property is asserted
    /// structurally: the network entry point is reachable from exactly one command arm.
    #[test]
    fn nothing_checks_for_updates_unless_asked() {
        let main = include_str!("main.rs");
        // `mod self_update;` is the declaration, not a call.
        let calls: Vec<&str> = main
            .lines()
            .map(str::trim)
            .filter(|l| l.contains("self_update::") && !l.starts_with("mod "))
            .collect();
        assert_eq!(
            calls.len(),
            1,
            "self_update is reached from more than one place: {calls:?}"
        );
        assert!(
            calls[0].starts_with("return self_update::run("),
            "the only call must be the command's own: {}",
            calls[0]
        );
        // And the module itself starts nothing in the background. Only the production half is
        // scanned: this test names those constructs in order to forbid them, and a scan of the
        // whole file would match its own prohibition.
        let me = include_str!("self_update.rs");
        let me = me.split("#[cfg(test)]").next().unwrap_or(me);
        for forbidden in ["thread::spawn", "tokio::spawn", "set_interval"] {
            assert!(
                !me.contains(forbidden),
                "self-update must not run anything in the background ({forbidden})"
            );
        }
    }

    #[test]
    fn asset_names_match_what_the_release_publishes() {
        assert_eq!(
            asset_name("v0.1.0", "x86_64-unknown-linux-gnu"),
            "kanbanr-v0.1.0-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            asset_name("v0.1.0", "x86_64-pc-windows-msvc"),
            "kanbanr-v0.1.0-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            binary_sum_name("v0.1.0", "aarch64-apple-darwin"),
            "kanbanr-v0.1.0-aarch64-apple-darwin.bin.sha256"
        );
        // This host must be a target we publish, or the test suite is running somewhere the tool
        // cannot update itself — worth knowing.
        assert!(
            target_triple().is_some(),
            "no published target for this host"
        );
    }

    #[test]
    fn a_checksum_is_read_out_of_the_sums_file_by_name() {
        let sums = "\
aaaa  kanbanr-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
bbbb *kanbanr-v0.1.0-aarch64-apple-darwin.tar.gz
cccc  install.sh
";
        assert_eq!(
            sum_for(sums, "kanbanr-v0.1.0-x86_64-unknown-linux-gnu.tar.gz").as_deref(),
            Some("aaaa")
        );
        // The `*` binary marker is part of sha256sum's format, not part of the name.
        assert_eq!(
            sum_for(sums, "kanbanr-v0.1.0-aarch64-apple-darwin.tar.gz").as_deref(),
            Some("bbbb")
        );
        // A name that merely looks similar is not a match.
        assert_eq!(
            sum_for(sums, "kanbanr-v0.1.0-x86_64-unknown-linux-gnu.zip"),
            None
        );
    }
}
