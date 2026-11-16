//! The block the board writes into a project's `CLAUDE.md` (FEAT-065).
//!
//! An agent working on a project reads its instructions from `CLAUDE.md`; the project's reasoning
//! lives on the board. Left unconnected, the instructions either restate the charter — and drift
//! from it within a week — or omit it, and the agent never sees the non-goals it should not
//! propose against.
//!
//! So the block is **generated, marked and regenerable**, and it **references** rather than
//! restates: purpose, goals and non-goals are short enough to be worth repeating in front of the
//! reader, and everything else is a command that reads the live board. Anything outside the
//! markers is the author's and is never touched.
//!
//! What it deliberately does not do is turn prose into rules. A non-goal is context for a reader
//! who can weigh it; it is not a condition a hook can check (ADR-0004).

use crate::error::Result;
use crate::{Charter, Store};

pub const BEGIN: &str = "<!-- kanbanr:begin — generated, edits here are overwritten -->";
pub const END: &str = "<!-- kanbanr:end -->";

/// The board's published address: its first remote, as a browsable https URL. `None` for a board
/// that is not shared, or shared at an address that is not a web host.
fn board_url(store: &Store) -> Option<String> {
    let (_, url) = crate::git::list_remotes(store.data_dir())
        .into_iter()
        .next()?;
    crate::git::web_url(&url)
}

/// Render the block for a project. `None` when there is nothing worth saying — a project with no
/// charter has no reasoning to put in front of anyone, and an empty block would be noise that
/// still has to be maintained.
pub fn block(store: &Store, project: &str) -> Result<Option<String>> {
    let charter: Charter = crate::charter::load(store, project)?;
    if charter.is_empty() {
        return Ok(None);
    }
    let mut out = String::new();
    out.push_str(BEGIN);
    out.push_str("\n\n## This project is tracked with kanbanr\n\n");
    out.push_str(
        "The board is the system of record for scope, reasoning, progress and documentation. It is \
         a separate git repository beside this one. **Recover from it at the start of every \
         session** and record work there as you go — this block is a pointer, not a copy.\n\n",
    );
    // Where it is published, when it is (FEAT-136): a contributor's Claude, in a fresh clone, has
    // no board until it is told where to get one.
    if let Some(url) = board_url(store) {
        let folder = store
            .data_dir()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "the board".into());
        out.push_str(&format!(
            "The board is published at <{url}>. Clone it beside this repository as `{folder}` \
             (`git clone {url} ../{folder}`) and kanbanr finds it.\n\n"
        ));
    }

    if !charter.purpose.trim().is_empty() {
        let purpose = charter.purpose.trim().replace('\n', " ");
        out.push_str(&format!("**Why this project exists.** {purpose}\n\n"));
    }
    if !charter.goals.is_empty() {
        out.push_str("**What it commits to** (work items link these by id):\n\n");
        for goal in &charter.goals {
            out.push_str(&format!("- `{}` {}\n", goal.id, goal.statement.trim()));
        }
        out.push('\n');
    }
    if !charter.non_goals.is_empty() {
        // The highest-value thing to put in front of an agent: what not to propose. Shown, never
        // enforced — a hook cannot judge whether a change touches a non-goal.
        out.push_str("**Deliberately out of scope** — do not propose these as gaps:\n\n");
        for non_goal in &charter.non_goals {
            out.push_str(&format!("- {}\n", non_goal.trim()));
        }
        out.push('\n');
    }
    if !charter.constraints.is_empty() {
        out.push_str("**Constraints:**\n\n");
        for constraint in &charter.constraints {
            out.push_str(&format!("- {}\n", constraint.trim()));
        }
        out.push('\n');
    }

    // The process, stage by stage (FEAT-117): what each status is for, so the agent grows an
    // item's definition one stage at a time instead of filling everything in up front. Only a
    // workflow that declares its gates has stages to describe.
    if let Ok(config) = store.load_meta(project).map(|p| p.config)
        && config.declares_gates()
    {
        out.push_str(
            "**How work moves here** — each stage asks only for what it needs; \
             `kanbanr check <CODE>` names what the next one still lacks:\n\n",
        );
        let gates = config.effective_gates();
        for status in &config.statuses {
            if let Some(gate) = gates.get(status)
                && !gate.purpose.trim().is_empty()
            {
                out.push_str(&format!("- **{status}** — {}\n", gate.purpose.trim()));
            }
        }
        out.push('\n');
    }

    out.push_str(
        "**Before starting an item:** `kanbanr lessons --for <CODE>` — what this project already \
         learned. **Before calling one done:** `kanbanr check <CODE>`.\n\n\
         | Question | Command |\n\
         |---|---|\n\
         | What is planned, in progress and done? | `kanbanr board` |\n\
         | What can I pick up now? | `kanbanr ready` |\n\
         | Why does this item exist, and how is it verified? | `kanbanr feature show <CODE>` |\n\
         | Why is the architecture like this? | `kanbanr adr list` |\n\
         | Why does this line of code exist? | `kanbanr why <file>:<line>` |\n\
         | What did the last wave cost? | `kanbanr retro <MS-00x>` |\n\n\
         Regenerate this block with `kanbanr claude sync`.\n\n",
    );
    out.push_str(END);
    out.push('\n');
    Ok(Some(out))
}

/// Put the block into an existing `CLAUDE.md`, replacing a previous one. Everything outside the
/// markers is returned untouched — the file belongs to the author, and only the block is ours.
pub fn merge(existing: &str, block: &str) -> String {
    match (existing.find(BEGIN), existing.find(END)) {
        (Some(start), Some(end)) if end > start => {
            let tail = &existing[end + END.len()..];
            format!(
                "{}{}{}",
                &existing[..start],
                block.trim_end(),
                if tail.starts_with('\n') {
                    tail.to_string()
                } else {
                    format!("\n{tail}")
                }
            )
        }
        // No block yet: append, leaving whatever the author wrote first.
        _ if existing.trim().is_empty() => block.to_string(),
        _ => format!("{}\n\n{}", existing.trim_end(), block),
    }
}

/// The author's part of a CLAUDE.md: the text before BEGIN and after END, each trimmed and joined
/// by one "\n" (an empty part dropped), or the whole text trimmed when there is no BEGIN…END pair
/// with END after BEGIN. "\r\n" is read as "\n" first. Pure (DEF-083).
pub fn outside_block(text: &str) -> String {
    let t = text.replace("\r\n", "\n");
    let Some(s) = t.find(BEGIN) else {
        return t.trim().to_string();
    };
    let Some(e) = t[s..].find(END).map(|i| s + i) else {
        return t.trim().to_string();
    };
    [t[..s].trim(), t[e + END.len()..].trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `before` and `after` differ, and only inside the generated block:
/// `before != after && outside_block(before) == outside_block(after)`. Pure (DEF-083).
pub fn block_only_change(before: &str, after: &str) -> bool {
    before != after && outside_block(before) == outside_block(after)
}

/// The board path to suggest for a file the guard refuses, or `None` when the write is not the
/// guard's business.
///
/// The guard judges only files **inside the tracked repository** (FEAT-111). A path outside it —
/// Claude Code's own plan file in `~/.claude/plans`, a scratch file, anything reached through
/// `..` — is none of the board's concern, and refusing it once blocked plan mode outright. The
/// suggestion is built from the repo-relative path, never the absolute one, which used to come out
/// as `notes//home/…`.
pub fn board_path_for(path: &str, root: &std::path::Path, cwd: &std::path::Path) -> Option<String> {
    let resolved = resolve_lexically(path, cwd);
    // Then resolve on disk as far as the path exists (FEAT-129). The root comes from the working
    // directory, which the OS reports with symlinks resolved; the file path arrives as Claude Code
    // spelled it. On macOS the temp folder is `/var/…`, a symlink to `/private/var/…`, so the two
    // never shared a prefix and a loose note there was waved through as "outside the repository".
    let (resolved, root) = (
        crate::project::normalize(&resolved),
        crate::project::normalize(root),
    );
    let relative = resolved.strip_prefix(&root).ok()?;
    let relative: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    // A linked worktree's file seen from the main checkout (DEF-065): judge it by its path inside
    // the worktree, or `docs/guide/x.md` there counts as a loose note in the main checkout.
    let relative = match relative.as_slice() {
        [claude, worktrees, _name, rest @ ..]
            if claude == ".claude" && worktrees == "worktrees" =>
        {
            rest.join("/")
        }
        _ => relative.join("/"),
    };
    if relative.is_empty() || !belongs_on_board(&relative) {
        return None;
    }
    Some(
        relative
            .trim_end_matches(".md")
            .trim_end_matches(".markdown")
            .replace(' ', "-")
            .to_lowercase(),
    )
}

/// `path` made absolute against `cwd`, with `.` and `..` resolved by the text alone: the file may
/// not exist yet, so it cannot be canonicalised, and an escape through `..` must not count as
/// inside the repository.
fn resolve_lexically(path: &str, cwd: &std::path::Path) -> std::path::PathBuf {
    let given = std::path::Path::new(path);
    let absolute = if given.is_absolute() {
        given.to_path_buf()
    } else {
        cwd.join(given)
    };
    let mut resolved = std::path::PathBuf::new();
    for part in absolute.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}

/// The docs guard's single entry point (DEF-065): [`board_path_for`], judged against the git top
/// level of the folder the file is in rather than the session's checkout.
///
/// The file's own repository decides: in a linked worktree that is the worktree, so its
/// `docs/guide/x.md` is the book, not a stray note in the main checkout. The file, and often its
/// folder, does not exist yet, so git is asked from the nearest existing ancestor. That top level is
/// taken only when it shares `fallback_root`'s git common directory, so it is the session
/// repository or one of its worktrees. Otherwise, or when git gives no answer, `fallback_root` is
/// used, and the guard still never fails a write.
pub fn docs_guard_suggestion(
    path: &str,
    cwd: &std::path::Path,
    fallback_root: &std::path::Path,
) -> Option<String> {
    let resolved = resolve_lexically(path, cwd);
    // One call for the file's top level and the repository it belongs to, one for the session's.
    let rev_parse = |dir: &std::path::Path, args: &[&str]| {
        let mut all = vec!["rev-parse", "--path-format=absolute"];
        all.extend_from_slice(args);
        crate::coderepo::git(dir, &all).ok()
    };
    let own = resolved
        .ancestors()
        .skip(1)
        .find(|dir| dir.is_dir())
        .and_then(|dir| rev_parse(dir, &["--show-toplevel", "--git-common-dir"]))
        .and_then(|out| {
            let mut lines = out.lines().map(|l| std::path::PathBuf::from(l.trim()));
            Some((lines.next()?, lines.next()?))
        });
    // Only a worktree of the session's own repository is judged by its own root: a file in an
    // unrelated repository, a scratch one under the temporary folder, stays outside the guard
    // (FEAT-111 R-1).
    let session = || {
        rev_parse(fallback_root, &["--git-common-dir"])
            .map(|out| crate::project::normalize(std::path::Path::new(out.trim())))
    };
    let top_level = own.and_then(|(top, common)| {
        (session() == Some(crate::project::normalize(&common))).then_some(top)
    });
    let root = top_level.as_deref().unwrap_or(fallback_root);
    board_path_for(path, root, cwd)
}

/// Is this markdown file project *reasoning* (board) rather than a shipped *deliverable* (repo)?
///
/// The allow-list is what a repository publishes. Everything else that is markdown, written loose
/// in a checkout, is the kind of note that should have gone on the board.
pub fn belongs_on_board(relative: &str) -> bool {
    let lower = relative.to_lowercase();
    if !lower.ends_with(".md") && !lower.ends_with(".markdown") {
        return false;
    }
    const SHIPPED_DIRS: [&str; 8] = [
        "docs/", "doc/", "book/", "site/", "web/", "skill/", ".github/", "editor/",
    ];
    const SHIPPED_FILES: [&str; 9] = [
        "readme.md",
        "changelog.md",
        "contributing.md",
        "security.md",
        "code_of_conduct.md",
        "license.md",
        "third_party.md",
        "claude.md",
        "agents.md",
    ];
    if SHIPPED_DIRS.iter().any(|d| lower.starts_with(d)) {
        return false;
    }
    if SHIPPED_FILES.contains(&lower.as_str()) {
        return false;
    }
    // A test fixture or a vendored file is not project reasoning either.
    if ["target/", "node_modules/", "tests/", "fixtures/", "vendor/"]
        .iter()
        .any(|d| lower.contains(d))
    {
        return false;
    }
    true
}

/// Why the docs guard refuses a write, and what to run instead (FEAT-065): `suggestion` is
/// [`board_path_for`]'s answer.
pub fn docs_guard_reason(suggestion: &str) -> String {
    format!(
        "Documentation for this project belongs on its board, not in the code repository \
         — the board is where it can be found, searched, and linked to the work it \
         explains.\n\nWrite it there instead:\n    kanbanr doc add notes/{suggestion}.md \
         --file <path>\n\nIf this file is part of what the repository ships (README, \
         CHANGELOG, docs/, CONTRIBUTING, LICENSE, a doc-site source or a fixture), say so \
         and write it: those are deliverables, not project reasoning."
    )
}

// ---- Permission rules for delegated commands (DEF-060) ----

use std::path::{Path, PathBuf};

/// The Claude Code allow rule that lets Claude run the command for one delegable verdict:
/// `approve`, `ratify`, or `signoff:<name>`. Anything else has none.
pub fn permission_rule(verdict: &str) -> Option<String> {
    let verdict = verdict.trim();
    match verdict {
        "approve" => Some("Bash(kanbanr approve:*)".into()),
        "ratify" => Some("Bash(kanbanr ratify:*)".into()),
        v => match v.strip_prefix("signoff:") {
            Some(name) if !name.trim().is_empty() => Some("Bash(kanbanr signoff:*)".into()),
            _ => None,
        },
    }
}

/// The rules every granted delegation in force needs, sorted and deduplicated. None unless the
/// mode is delegated.
pub fn permission_rules(d: &crate::decisions::Decisions, now: &str) -> Vec<String> {
    if d.mode() != crate::decisions::Mode::Delegated {
        return Vec::new();
    }
    let mut rules: Vec<String> = d
        .delegations
        .iter()
        .filter(|g| g.state(now) == crate::decisions::DelegationState::Granted)
        .flat_map(|g| g.verdicts.iter())
        .filter_map(|v| permission_rule(v))
        .collect();
    rules.sort();
    rules.dedup();
    rules
}

/// The Claude Code config dir: `$CLAUDE_CONFIG_DIR` when set and non-empty, else `~/.claude`.
pub fn claude_config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::project::home_dir().map(|h| h.join(".claude")))
}

/// The settings files that can allow a rule for a repository, in the order they are reported:
/// the project's `settings.local.json` and `settings.json`, then the user's `settings.json`.
pub fn settings_files(repo: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        repo.join(".claude").join("settings.local.json"),
        repo.join(".claude").join("settings.json"),
    ];
    files.extend(claude_config_dir().map(|d| d.join("settings.json")));
    files
}

/// `Bash(<prefix>:*)` -> `<prefix>`.
fn bash_prefix(rule: &str) -> Option<&str> {
    rule.trim()
        .strip_prefix("Bash(")?
        .strip_suffix(":*)")
        .map(str::trim)
}

/// Does `permissions.allow` hold `rule`, exactly or through a covering prefix rule
/// (`Bash(kanbanr:*)` covers `Bash(kanbanr signoff:*)`)? Nothing else is interpreted.
pub fn allows(settings: &serde_json::Value, rule: &str) -> bool {
    let rule = rule.trim();
    let want = bash_prefix(rule);
    settings["permissions"]["allow"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e.as_str())
        .any(|e| {
            let e = e.trim();
            if e == rule {
                return true;
            }
            match (bash_prefix(e), want) {
                (Some(p), Some(q)) => {
                    q == p || q.strip_prefix(p).is_some_and(|rest| rest.starts_with(' '))
                }
                _ => false,
            }
        })
}

fn read_settings(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The first of `files` that allows `rule`.
pub fn allowed_in(files: &[PathBuf], rule: &str) -> Option<PathBuf> {
    files
        .iter()
        .find(|f| read_settings(f).is_some_and(|s| allows(&s, rule)))
        .cloned()
}

/// The rules none of `files` allows. An absent, empty or invalid file allows nothing.
pub fn rules_missing(files: &[PathBuf], rules: &[String]) -> Vec<String> {
    let all: Vec<serde_json::Value> = files.iter().filter_map(|f| read_settings(f)).collect();
    rules
        .iter()
        .filter(|r| !all.iter().any(|s| allows(s, r)))
        .cloned()
        .collect()
}

/// Add each rule not already in `permissions.allow` of the settings file at `path`, keeping every
/// other key and its order. Returns the rules added; nothing is written when none was. Invalid
/// JSON, or an `allow` that is not an array, is an error naming the file, and the file is left
/// alone.
pub fn add_permission_rules(path: &Path, rules: &[String]) -> Result<Vec<String>> {
    let bad = |what: &str| {
        crate::error::CoreError::Refused(format!("{} {what}; left alone", path.display()))
    };
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: serde_json::Value = if raw.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&raw).map_err(|e| bad(&format!("is not valid JSON ({e})")))?
    };
    let root = doc
        .as_object_mut()
        .ok_or_else(|| bad("is not a JSON object"))?;
    let permissions = root
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| bad("has a `permissions` that is not an object"))?;
    let allow = permissions
        .entry("allow")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| bad("has a `permissions.allow` that is not an array"))?;
    let mut added = Vec::new();
    for rule in rules {
        if !allow
            .iter()
            .any(|e| e.as_str().map(str::trim) == Some(rule.trim()))
        {
            allow.push(serde_json::Value::String(rule.clone()));
            added.push(rule.clone());
        }
    }
    if added.is_empty() {
        return Ok(added);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = serde_json::to_string_pretty(&doc).map_err(|e| bad(&e.to_string()))?;
    out.push('\n');
    crate::atomic::write(path, out)?;
    Ok(added)
}

/// Does the repository's `.gitignore` have a line equal to `relative` or `**/<file name>`?
/// Read-only; false when unreadable.
pub fn gitignore_covers(repo: &Path, relative: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(repo.join(".gitignore")) else {
        return false;
    };
    let name = relative.rsplit('/').next().unwrap_or(relative);
    let wide = format!("**/{name}");
    text.lines()
        .map(str::trim)
        .any(|l| l == relative || l == wide)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;

    fn fixture() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-claude-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        (store, dir)
    }

    #[test]
    fn the_block_is_replaced_and_the_rest_of_the_file_is_left_alone() {
        let (store, dir) = fixture();
        // No charter: nothing worth saying, so nothing is written.
        assert!(block(&store, "demo").unwrap().is_none());

        crate::charter::save(
            &store,
            "demo",
            &Charter {
                purpose: "Keep the reasoning with the work.".into(),
                goals: vec![crate::charter::Goal {
                    id: "G-1".into(),
                    statement: "A session resumes without a recap".into(),
                    ..Default::default()
                }],
                non_goals: vec!["An enterprise PM platform".into()],
                constraints: vec!["One developer, one machine".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let generated = block(&store, "demo").unwrap().unwrap();
        assert!(generated.contains("Keep the reasoning with the work."));
        assert!(generated.contains("`G-1` A session resumes without a recap"));
        assert!(generated.contains("An enterprise PM platform"));
        assert!(generated.contains("One developer, one machine"));
        assert!(generated.starts_with(BEGIN) && generated.trim_end().ends_with(END));

        // Appended below the author's own instructions, not in place of them.
        let mine = "# My project\n\nAlways run the linter before committing.\n";
        let merged = merge(mine, &generated);
        assert!(merged.starts_with("# My project"));
        assert!(merged.contains("Always run the linter"));
        assert!(merged.contains("`G-1` A session resumes"));

        // Regenerating replaces only the block — including when the author has written below it.
        let with_tail = format!("{merged}\n## My own notes\n\nKeep these.\n");
        let changed = generated.replace("A session resumes without a recap", "Something else");
        let again = merge(&with_tail, &changed);
        assert!(again.contains("Always run the linter"), "{again}");
        assert!(again.contains("## My own notes"), "{again}");
        assert!(again.contains("Something else"));
        assert!(
            !again.contains("A session resumes without a recap"),
            "the stale copy is gone, not duplicated"
        );
        assert_eq!(again.matches(BEGIN).count(), 1, "exactly one block");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEF-083: a refresh of the generated block is told apart from an edit of the author's text.
    #[test]
    fn a_block_only_change_leaves_the_authors_text_alone() {
        let block_a = format!("{BEGIN}\n\nA\n\n{END}\n");
        let block_b = format!("{BEGIN}\n\nB\n\n{END}\n");
        let first = merge("# app\n", &block_a);
        // The block appended to a file that had none.
        assert!(block_only_change("# app\n", &first), "{first}");
        // The block replaced.
        let second = merge(&first, &block_b);
        assert!(block_only_change(&first, &second), "{second}");
        // No change is no refresh.
        assert!(!block_only_change(&first, &first));
        // The author's text changed too.
        let edited = merge(&first.replace("# app", "# app2"), &block_b);
        assert!(!block_only_change(&first, &edited), "{edited}");
        // CRLF outside the block reads as LF.
        let crlf = first.replace('\n', "\r\n");
        assert!(block_only_change(&crlf, &second));
        // BEGIN with no END after it is no block: any change is outside.
        let open = format!("# app\n\n{BEGIN}\n\nA\n");
        assert!(!block_only_change(&open, &open.replace("A\n", "B\n")));
        let backwards = format!("{END}\n# app\n{BEGIN}\nA\n");
        assert!(!block_only_change(
            &backwards,
            &backwards.replace("A\n", "B\n")
        ));
        // HEAD with no CLAUDE.md reads as empty: a block-only file is a refresh.
        assert!(block_only_change("", &block_a));
    }

    /// FEAT-136: the board's remote becomes the address a contributor clones; a local path does not.
    #[test]
    fn the_block_names_where_the_board_is_published() {
        assert_eq!(
            crate::git::web_url("git@github.com:startr-trade/kanbanr-board.git").as_deref(),
            Some("https://github.com/startr-trade/kanbanr-board")
        );
        assert_eq!(
            crate::git::web_url("https://github.com/startr-trade/kanbanr-board.git").as_deref(),
            Some("https://github.com/startr-trade/kanbanr-board")
        );
        assert_eq!(crate::git::web_url("/srv/boards/shop.git"), None);

        let (store, dir) = fixture();
        crate::charter::save(
            &store,
            "demo",
            &Charter {
                purpose: "Keep the reasoning with the work.".into(),
                ..Default::default()
            },
        )
        .unwrap();
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let without = block(&store, "demo").unwrap().unwrap();
        assert!(!without.contains("published at"), "{without}");
        crate::git::add_remote(&dir, "origin", "git@github.com:o/shop-board.git").unwrap();
        let with = block(&store, "demo").unwrap().unwrap();
        assert!(
            with.contains("published at <https://github.com/o/shop-board>"),
            "{with}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-065: the guard decides from a path and nothing else. Every judgement it gets wrong is
    /// either a note scattered into a codebase or a legitimate write refused — and a guard that
    /// refuses legitimate writes gets turned off.
    #[test]
    fn the_docs_guard_ignores_paths_outside_the_repo() {
        let root = std::path::Path::new("/work/shop");
        // FEAT-111 R-1: Claude Code's plan file, a scratch file, and an escape through `..` are
        // all outside the repository, so the guard has no opinion on them.
        for path in [
            "/home/someone/.claude/plans/a-plan.md",
            "/tmp/scratch/NOTES.md",
            "../elsewhere/PLAN.md",
            "/work/shop/../shop-notes/PLAN.md",
        ] {
            assert_eq!(board_path_for(path, root, root), None, "{path}");
        }
        // Inside it, the rule is unchanged: loose notes are refused, deliverables are not.
        assert!(board_path_for("/work/shop/NOTES.md", root, root).is_some());
        assert_eq!(board_path_for("/work/shop/README.md", root, root), None);
    }

    /// FEAT-129: on macOS the temp folder is `/var/…`, a symlink to `/private/var/…`. The root is
    /// read from the working directory with symlinks resolved; the file path is not. They shared no
    /// prefix, so a loose note inside the repository counted as outside it and was let through —
    /// which only a CI runner, not a Linux laptop, showed.
    #[cfg(unix)]
    #[test]
    fn the_docs_guard_follows_a_symlinked_folder() {
        let base = std::env::temp_dir().join(format!("kanbanr-guard-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let real = base.join("real");
        std::fs::create_dir_all(real.join("shop")).unwrap();
        std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
        let root = std::fs::canonicalize(real.join("shop")).unwrap();

        // Spelled through the link, it is still the repository's file: refused with the same
        // suggestion as when spelled through the real path.
        let through_link = base.join("link/shop/NOTES.md");
        let through_real = real.join("shop/NOTES.md");
        let suggestion = board_path_for(through_real.to_str().unwrap(), &root, &root);
        assert!(suggestion.is_some());
        assert_eq!(
            board_path_for(through_link.to_str().unwrap(), &root, &root),
            suggestion
        );
        // A deliverable stays allowed, and an escape through `..` stays outside.
        let readme = base.join("link/shop/README.md");
        assert_eq!(board_path_for(readme.to_str().unwrap(), &root, &root), None);
        let escape = base.join("link/shop/../elsewhere/PLAN.md");
        assert_eq!(board_path_for(escape.to_str().unwrap(), &root, &root), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// FEAT-111 R-2: the suggestion is a board path someone can actually run — built from the
    /// repo-relative path, never `notes//home/…`.
    #[test]
    fn the_docs_guard_suggests_a_usable_board_path() {
        let root = std::path::Path::new("/work/shop");
        assert_eq!(
            board_path_for("/work/shop/api/Design Notes.md", root, root).as_deref(),
            Some("api/design-notes")
        );
        assert_eq!(
            board_path_for("./PLAN.md", root, &root.join("sub/..")).as_deref(),
            Some("plan")
        );
    }

    #[test]
    fn the_docs_guard_allows_deliverables_and_stops_the_rest() {
        // Project reasoning written loose in a checkout: this is what belongs on the board.
        for path in [
            "NOTES.md",
            "design-thoughts.md",
            "api/PLAN.md",
            "some/deep/folder/retrospective.md",
        ] {
            assert!(belongs_on_board(path), "{path} should go to the board");
        }

        // What the repository ships, and what is not prose at all.
        for path in [
            "README.md",
            "CHANGELOG.md",
            "CONTRIBUTING.md",
            "SECURITY.md",
            "CLAUDE.md",
            "docs/USER_GUIDE.md",
            "skill/kanbanr/SKILL.md",
            ".github/PULL_REQUEST_TEMPLATE.md",
            "web/README.md",
            "api/crates/kanbanr-cli/tests/fixtures/board.md",
            "api/target/doc/index.md",
            "src/main.rs",
            "Cargo.toml",
        ] {
            assert!(!belongs_on_board(path), "{path} should be allowed through");
        }

        // Case is not identity: a repository's README is a README however it is spelled.
        assert!(!belongs_on_board("readme.md"));
        assert!(!belongs_on_board("Docs/design.md"));
    }

    // ---- Permission rules (DEF-060) ----

    fn scratch(tag: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-perm-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn deleg(
        id: &str,
        verdicts: &[&str],
        until: &str,
        state: &str,
    ) -> crate::decisions::Delegation {
        use crate::decisions::Stamp;
        let stamp = Stamp {
            by: "p".into(),
            at: "2026-11-01T00:00:00Z".into(),
            origin: "monitor".into(),
            ..Default::default()
        };
        crate::decisions::Delegation {
            id: id.into(),
            verdicts: verdicts.iter().map(|v| v.to_string()).collect(),
            all: true,
            until: until.into(),
            proposed: stamp.clone(),
            granted: (state != "proposed").then(|| stamp.clone()),
            revoked: (state == "revoked").then(|| stamp.clone()),
            ..Default::default()
        }
    }

    #[test]
    fn permission_rules_follow_the_delegations_in_force_in_delegated_mode() {
        use crate::decisions::{Decisions, Mode, ModeChange};
        let now = "2026-11-15T00:00:00Z";
        let future = "2027-01-01T00:00:00Z";
        let mut d = Decisions {
            delegations: vec![
                deleg(
                    "DLG-001",
                    &["signoff:design-review", "signoff:docs-folded"],
                    future,
                    "granted",
                ),
                deleg("DLG-002", &["approve"], future, "proposed"),
                deleg("DLG-003", &["ratify"], "2026-11-01T00:00:00Z", "granted"),
                deleg("DLG-004", &["ratify"], future, "revoked"),
            ],
            ..Default::default()
        };
        assert!(permission_rules(&d, now).is_empty(), "monitor mode: none");
        d.modes.push(ModeChange {
            mode: Mode::Delegated,
            by: "p".into(),
            at: now.into(),
            origin: "monitor".into(),
            reason: String::new(),
            extra: Default::default(),
        });
        assert_eq!(permission_rules(&d, now), vec!["Bash(kanbanr signoff:*)"]);
        d.delegations
            .push(deleg("DLG-005", &["approve"], future, "granted"));
        assert_eq!(
            permission_rules(&d, now),
            vec!["Bash(kanbanr approve:*)", "Bash(kanbanr signoff:*)"]
        );
        assert_eq!(permission_rule("release"), None);
        assert_eq!(permission_rule("signoff:"), None);
        assert_eq!(permission_rule("ratify").unwrap(), "Bash(kanbanr ratify:*)");
    }

    #[test]
    fn a_rule_is_added_to_settings_local_without_touching_the_rest() {
        let dir = scratch("add");
        let rule = "Bash(kanbanr signoff:*)".to_string();
        let path = dir.join(".claude").join("settings.local.json");

        // Absent: created with the rule.
        assert_eq!(
            add_permission_rules(&path, std::slice::from_ref(&rule)).unwrap(),
            vec![rule.clone()]
        );
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["permissions"]["allow"], serde_json::json!([rule]));

        // Existing keys and entries are kept, in order.
        std::fs::write(
            &path,
            r#"{"theme": "dark", "hooks": {"a": 1}, "permissions": {"allow": ["Bash(ls:*)"]}}"#,
        )
        .unwrap();
        assert_eq!(
            add_permission_rules(&path, std::slice::from_ref(&rule))
                .unwrap()
                .len(),
            1
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(
            v["permissions"]["allow"],
            serde_json::json!(["Bash(ls:*)", rule])
        );
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["theme", "hooks", "permissions"]);
        // A second add changes nothing.
        assert!(
            add_permission_rules(&path, std::slice::from_ref(&rule))
                .unwrap()
                .is_empty()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        // Invalid JSON: an error naming the file, text unchanged.
        std::fs::write(&path, "{ not json").unwrap();
        let err = add_permission_rules(&path, std::slice::from_ref(&rule))
            .unwrap_err()
            .to_string();
        assert!(err.contains("settings.local.json"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");

        // allows / rules_missing read the result.
        let ok = dir.join("ok.json");
        add_permission_rules(&ok, std::slice::from_ref(&rule)).unwrap();
        assert!(rules_missing(&[ok.clone(), path.clone()], std::slice::from_ref(&rule)).is_empty());
        assert_eq!(
            rules_missing(std::slice::from_ref(&path), std::slice::from_ref(&rule)),
            vec![rule.clone()]
        );

        // A covering prefix rule allows; near misses do not.
        let allow = |r: &str| serde_json::json!({"permissions": {"allow": [r]}});
        assert!(allows(&allow("Bash(kanbanr:*)"), &rule));
        assert!(!allows(&allow("Bash(kanbanrx:*)"), &rule));
        assert!(!allows(&allow("Bash(kanbanr sign:*)"), &rule));
        assert!(allows(&allow("Bash(kanbanr signoff:*)"), &rule));

        // gitignore_covers
        assert!(!gitignore_covers(&dir, ".claude/settings.local.json"));
        std::fs::write(dir.join(".gitignore"), "target\n**/settings.local.json\n").unwrap();
        assert!(gitignore_covers(&dir, ".claude/settings.local.json"));

        // settings_files lists the project's two files, then the user's.
        let files = settings_files(&dir);
        assert_eq!(files[0], dir.join(".claude").join("settings.local.json"));
        assert_eq!(files[1], dir.join(".claude").join("settings.json"));
        assert!(files.len() >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-065 R-1, R-3: a linked worktree under `.claude/worktrees/<name>` is judged by its path
    /// inside the worktree, whether the root is the main checkout or the worktree itself, and the
    /// suggestion for a refused file is built from that path.
    #[test]
    fn the_docs_guard_judges_a_worktree_path_by_its_own_root() {
        let main = std::path::Path::new("/work/shop");
        let worktree = main.join(".claude/worktrees/ftr");
        for root in [main, worktree.as_path()] {
            for path in [
                "/work/shop/.claude/worktrees/ftr/docs/guide/x.md",
                "/work/shop/.claude/worktrees/ftr/CHANGELOG.md",
                "/work/shop/.claude/worktrees/ftr/skill/kanbanr/SKILL.md",
                "/work/shop/.claude/worktrees/ftr/crates/a/tests/fixtures/b.md",
            ] {
                assert_eq!(
                    board_path_for(path, root, root),
                    None,
                    "{path} from {root:?}"
                );
            }
            assert_eq!(
                board_path_for("/work/shop/.claude/worktrees/ftr/notes.md", root, root).as_deref(),
                Some("notes"),
                "from {root:?}"
            );
            assert_eq!(
                board_path_for(
                    "/work/shop/.claude/worktrees/ftr/api/Design Notes.md",
                    root,
                    root
                )
                .as_deref(),
                Some("api/design-notes"),
                "from {root:?}"
            );
        }
    }
}
