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
    web_url(&url)
}

/// `git@github.com:o/r.git` and `https://github.com/o/r.git` both become `https://github.com/o/r`;
/// a local path or an unfamiliar scheme is no address a reader can open.
fn web_url(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    if let Some(rest) = trimmed.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some(format!("https://{host}/{path}"));
    }
    if let Some(rest) = trimmed.strip_prefix("ssh://git@") {
        return Some(format!("https://{rest}"));
    }
    trimmed.starts_with("https://").then(|| trimmed.to_string())
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

/// The board path to suggest for a file the guard refuses, or `None` when the write is not the
/// guard's business.
///
/// The guard judges only files **inside the tracked repository** (FEAT-111). A path outside it —
/// Claude Code's own plan file in `~/.claude/plans`, a scratch file, anything reached through
/// `..` — is none of the board's concern, and refusing it once blocked plan mode outright. The
/// suggestion is built from the repo-relative path, never the absolute one, which used to come out
/// as `notes//home/…`.
pub fn board_path_for(path: &str, root: &std::path::Path, cwd: &std::path::Path) -> Option<String> {
    let given = std::path::Path::new(path);
    let absolute = if given.is_absolute() {
        given.to_path_buf()
    } else {
        cwd.join(given)
    };
    // Resolve `.` and `..` by the text alone: the file may not exist yet, so it cannot be
    // canonicalised, and an escape through `..` must not count as inside the repository.
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
    let relative = relative.join("/");
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

    /// FEAT-136: the board's remote becomes the address a contributor clones; a local path does not.
    #[test]
    fn the_block_names_where_the_board_is_published() {
        assert_eq!(
            web_url("git@github.com:startr-trade/kanbanr-board.git").as_deref(),
            Some("https://github.com/startr-trade/kanbanr-board")
        );
        assert_eq!(
            web_url("https://github.com/startr-trade/kanbanr-board.git").as_deref(),
            Some("https://github.com/startr-trade/kanbanr-board")
        );
        assert_eq!(web_url("/srv/boards/shop.git"), None);

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
}
