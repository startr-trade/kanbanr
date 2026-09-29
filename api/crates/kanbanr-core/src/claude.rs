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
        && !config.gates.is_empty()
    {
        out.push_str(
            "**How work moves here** — each stage asks only for what it needs; \
             `kanbanr check <CODE>` names what the next one still lacks:\n\n",
        );
        for status in &config.statuses {
            if let Some(gate) = config.gates.get(status)
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
}
