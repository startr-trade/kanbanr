//! How a run's session signs in to Claude (RC2 of the v0.7.0 preview-4 triage).
//!
//! A copy of the person's `.credentials.json` carries a refresh token the person's own client
//! rotates: whichever side refreshes second is logged out. So a run prefers a long-lived token the
//! person made once with `claude setup-token`, kept in a file of their own (mode 0600, outside the
//! fence), and passes it to the session as `CLAUDE_CODE_OAUTH_TOKEN`; no credentials file is
//! copied. Without that file the run copies the access token alone, the refresh token blanked, and
//! starts only if that token outlives the run. `setup.log` names the mode, never a token.

use anyhow::{Result, anyhow};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The variable naming the token file.
pub const TOKEN_FILE_VAR: &str = "KANBANR_WALKTHROUGH_TOKEN_FILE";
/// The variable the session reads the token from.
pub const SESSION_TOKEN_VAR: &str = "CLAUDE_CODE_OAUTH_TOKEN";
/// How much longer than the run a copied access token must live.
pub const MARGIN: Duration = Duration::from_secs(15 * 60);

/// A secret that never prints: `Debug` shows only that it is there.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// How the session signs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credentials {
    /// The token from the token file, passed as `CLAUDE_CODE_OAUTH_TOKEN`.
    Token { file: PathBuf, token: Secret },
    /// The person's access token alone, written into the run's config as `.credentials.json`
    /// with the refresh token blanked; `expires_at_ms` as the person's file had it.
    AccessOnly { json: Secret, expires_at_ms: u64 },
    /// Neither: the session starts with no sign-in of the run's making.
    None,
}

impl Credentials {
    /// The mode `setup.log` records: never the token.
    pub fn mode(&self) -> String {
        match self {
            Credentials::Token { file, .. } => {
                format!("token file ({}), as {SESSION_TOKEN_VAR}", file.display())
            }
            Credentials::AccessOnly { expires_at_ms, .. } => format!(
                "access token copied, refresh token blanked; it expires at {expires_at_ms} ms"
            ),
            Credentials::None => "none: no token file and no credentials file".into(),
        }
    }

    /// The token for the session's environment, when there is one.
    pub fn session_token(&self) -> Option<Secret> {
        match self {
            Credentials::Token { token, .. } => Some(token.clone()),
            _ => None,
        }
    }
}

/// The token file: `KANBANR_WALKTHROUGH_TOKEN_FILE`, else `~/.config/kanbanr/walkthrough-token`.
pub fn token_file() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(TOKEN_FILE_VAR).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    kanbanr_core::project::home_dir().map(|h| h.join(".config/kanbanr/walkthrough-token"))
}

/// Reads the token file, if there is one. Refused: a file group or others may read, or one
/// inside the fence, where the session could read it.
pub fn read_token_file(path: &Path, fence: &Path) -> Result<Option<Secret>> {
    if !path.is_file() {
        return Ok(None);
    }
    if kanbanr_core::project::check_fence(path, fence).is_ok() {
        return Err(anyhow!(
            "the walkthrough token file {} is inside the fence ({}): keep it outside, where the \
             session cannot read it",
            path.display(),
            fence.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(anyhow!(
                "the walkthrough token file {} has mode {mode:o}: make it 0600 (chmod 600)",
                path.display()
            ));
        }
    }
    let token = std::fs::read_to_string(path)?.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!(
            "the walkthrough token file {} is empty: write the token `claude setup-token` gives",
            path.display()
        ));
    }
    Ok(Some(Secret(token)))
}

/// The fallback: the person's credentials file with its refresh token blanked, refused unless the
/// access token lives `run + MARGIN` past `now_ms`. The refusal names `claude setup-token` and
/// the token file.
pub fn access_only(
    person_credentials: &str,
    now_ms: u64,
    run: Duration,
    token_path: &Path,
) -> Result<Credentials> {
    let mut v: Value = serde_json::from_str(person_credentials)
        .map_err(|e| anyhow!("the Claude Code credentials file does not parse: {e}"))?;
    let oauth = v
        .get_mut("claudeAiOauth")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("the Claude Code credentials file holds no claudeAiOauth entry"))?;
    let expires_at_ms = oauth.get("expiresAt").and_then(Value::as_u64).unwrap_or(0);
    let needed = now_ms + (run + MARGIN).as_millis() as u64;
    if expires_at_ms < needed {
        let left_min = expires_at_ms.saturating_sub(now_ms) / 60_000;
        return Err(anyhow!(
            "the Claude Code access token expires in {left_min} minutes, before the run's {} \
             minutes plus 15: make a long-lived token with `claude setup-token` and keep it in {} \
             (mode 0600)",
            run.as_secs() / 60,
            token_path.display()
        ));
    }
    if oauth.contains_key("refreshToken") {
        oauth.insert("refreshToken".into(), Value::String(String::new()));
    }
    Ok(Credentials::AccessOnly {
        json: Secret(serde_json::to_string_pretty(&v)?),
        expires_at_ms,
    })
}

/// How this run signs in: the token file first, else the access-only copy of the person's
/// credentials in `claude_dir`, else none.
pub fn resolve(fence: &Path, claude_dir: Option<&Path>, run: Duration) -> Result<Credentials> {
    let path = token_file()
        .ok_or_else(|| anyhow!("no home folder: set {TOKEN_FILE_VAR} to the token file"))?;
    if let Some(token) = read_token_file(&path, fence)? {
        return Ok(Credentials::Token { file: path, token });
    }
    let Some(src) = claude_dir.map(|d| d.join(".credentials.json")) else {
        return Ok(Credentials::None);
    };
    let Ok(text) = std::fs::read_to_string(&src) else {
        return Ok(Credentials::None);
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    access_only(&text, now_ms, run, &path)
}

/// Writes the access-only copy into the run's config folder, mode 0600, the folder 0700; does
/// nothing for the other modes.
pub fn install(config_dir: &Path, creds: &Credentials) -> Result<()> {
    let Credentials::AccessOnly { json, .. } = creds else {
        return Ok(());
    };
    std::fs::create_dir_all(config_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(config_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let file = config_dir.join(".credentials.json");
    kanbanr_core::atomic::write(&file, json.expose())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// True when a session's error text is a sign-in failure: a harness stop, never a product
/// finding.
pub fn is_auth_failure(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("failed to authenticate")
        || t.contains("authentication_failed")
        || t.contains("not logged in")
        || t.contains("invalid api key")
        || t.contains("oauth access token is invalid")
        || t.contains("api error: 401")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("kanbanr-wt-creds-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(unix)]
    #[test]
    fn a_world_readable_token_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("mode");
        let fence = d.join("fence");
        std::fs::create_dir_all(&fence).unwrap();
        let file = d.join("walkthrough-token");
        std::fs::write(&file, "tok-not-a-real-one\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = read_token_file(&file, &fence).unwrap_err().to_string();
        assert!(err.contains("0600"), "{err}");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let ok = read_token_file(&file, &fence).unwrap().unwrap();
        assert_eq!(ok.expose(), "tok-not-a-real-one");
        assert!(!format!("{ok:?}").contains("tok-not"));
        // setup.log records the mode, never the token.
        let mode = Credentials::Token {
            file: file.clone(),
            token: ok,
        }
        .mode();
        assert!(
            mode.contains("token file") && !mode.contains("tok-not"),
            "{mode}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_token_file_inside_the_fence_is_refused() {
        let d = scratch("fence");
        let fence = d.join("fence");
        std::fs::create_dir_all(&fence).unwrap();
        let file = fence.join("walkthrough-token");
        std::fs::write(&file, "tok-not-a-real-one\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let fence = std::fs::canonicalize(&fence).unwrap();
        let err = read_token_file(&file, &fence).unwrap_err().to_string();
        assert!(err.contains("inside the fence"), "{err}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_fallback_refuses_a_short_lived_copy_and_blanks_the_refresh_token() {
        let now = 1_000_000_000u64;
        let run = Duration::from_secs(180 * 60);
        let creds = |exp: u64| {
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"acc-not-real","refreshToken":"ref-not-real","expiresAt":{exp}}}}}"#
            )
        };
        let path = Path::new("/home/x/.config/kanbanr/walkthrough-token");
        // 3 h of run plus 10 min left: short of the 15-minute margin.
        let short = now + (180 + 10) * 60_000;
        let err = access_only(&creds(short), now, run, path)
            .unwrap_err()
            .to_string();
        assert!(err.contains("claude setup-token"), "{err}");
        assert!(err.contains("walkthrough-token"), "{err}");
        let long = now + (180 + 20) * 60_000;
        let got = access_only(&creds(long), now, run, path).unwrap();
        let Credentials::AccessOnly {
            json,
            expires_at_ms,
        } = &got
        else {
            panic!("{got:?}")
        };
        assert_eq!(*expires_at_ms, long);
        assert!(json.expose().contains("acc-not-real"));
        assert!(!json.expose().contains("ref-not-real"));
        assert!(!got.mode().contains("acc-not-real"));
    }

    #[test]
    fn a_sign_in_failure_is_told_apart() {
        assert!(is_auth_failure(
            "Failed to authenticate. API Error: 401 OAuth access token is invalid"
        ));
        assert!(is_auth_failure("Not logged in · Please run /login"));
        assert!(!is_auth_failure("Reached max budget of $2"));
    }
}
