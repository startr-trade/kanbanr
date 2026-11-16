//! The Claude Code skill this binary carries, and putting it where Claude Code finds it (FEAT-141).
//!
//! kanbanr used to be two installs: the program from a release, the skill from a clone or the
//! plugin marketplace — a second step, tied to no version, so a skill could describe commands its
//! program lacked. The skill is now embedded at build time (`build.rs`), and `kanbanr skill install`
//! writes it to Claude Code's personal skills folder. The installers run it; `self-update` keeps it
//! in step.
//!
//! A folder kanbanr did not write is never touched: a link to a clone (how this project's own
//! maintainer works on the skill), or one a plugin manager owns. kanbanr knows its own by a stamp
//! file it writes beside the skill.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/skill_files.rs"));

/// The stamp kanbanr leaves in a skill folder it wrote: its version and the content's digest.
const STAMP: &str = ".kanbanr-skill";

/// What is installed where the skill belongs.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// Nothing there.
    Missing,
    /// kanbanr's own copy; `current` when it is exactly the skill this binary carries.
    Installed { version: String, current: bool },
    /// Something kanbanr did not write — a link to a clone, or another tool's copy. Left alone.
    Foreign { link: Option<PathBuf> },
}

/// Claude Code's personal skills folder for kanbanr: `<claude dir>/skills/kanbanr`.
pub fn target() -> Result<PathBuf> {
    let dir = crate::hooks::claude_dir()
        .context("no home directory to install the skill into: set CLAUDE_CONFIG_DIR")?;
    Ok(dir.join("skills").join("kanbanr"))
}

/// The digest of the embedded skill: what an installed copy is compared against.
fn digest() -> String {
    let mut h = Sha256::new();
    for (rel, bytes) in SKILL_FILES {
        h.update(rel.as_bytes());
        h.update([0]);
        h.update(bytes);
    }
    format!("{:x}", h.finalize())
}

fn stamp_text() -> String {
    format!(
        "version: {}\ndigest: {}\n",
        env!("CARGO_PKG_VERSION"),
        digest()
    )
}

/// What is at `dir`.
pub fn status_at(dir: &Path) -> Status {
    if let Ok(meta) = std::fs::symlink_metadata(dir)
        && meta.file_type().is_symlink()
    {
        return Status::Foreign {
            link: std::fs::read_link(dir).ok(),
        };
    }
    if !dir.exists() {
        return Status::Missing;
    }
    let Ok(stamp) = std::fs::read_to_string(dir.join(STAMP)) else {
        return Status::Foreign { link: None };
    };
    let field = |name: &str| {
        stamp
            .lines()
            .find_map(|l| l.strip_prefix(name).map(|v| v.trim().to_string()))
            .unwrap_or_default()
    };
    Status::Installed {
        version: field("version:"),
        current: field("digest:") == digest(),
    }
}

/// Write the embedded skill to `dir`, replacing a copy kanbanr wrote earlier. Refuses to touch a
/// folder kanbanr did not write. Returns what happened, for the caller to print.
pub fn install_at(dir: &Path) -> Result<String> {
    if SKILL_FILES.is_empty() {
        bail!("this binary carries no skill (it was built outside the kanbanr repository)");
    }
    match status_at(dir) {
        Status::Foreign { link: Some(to) } => {
            return Ok(format!(
                "left {} alone: it links to {} — a skill you manage yourself",
                dir.display(),
                to.display()
            ));
        }
        Status::Foreign { link: None } => {
            return Ok(format!(
                "left {} alone: kanbanr did not write it (another tool's copy?). Remove it to let \
                 kanbanr install its own",
                dir.display()
            ));
        }
        Status::Installed {
            current: true,
            version,
        } => {
            return Ok(format!(
                "skill {version} already installed at {}",
                dir.display()
            ));
        }
        Status::Missing | Status::Installed { current: false, .. } => {}
    }
    // Written beside the target and swapped in, so a half-written skill is never what Claude
    // Code loads.
    let parent = dir.parent().context("the skill folder has no parent")?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(".kanbanr.installing");
    let _ = std::fs::remove_dir_all(&staging);
    for (rel, bytes) in SKILL_FILES {
        let path = staging.join(rel);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&path, bytes)
            .with_context(|| format!("could not write {}", path.display()))?;
        #[cfg(unix)]
        if rel.ends_with(".sh") || rel.ends_with(".py") {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    std::fs::write(staging.join(STAMP), stamp_text())?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::rename(&staging, dir)?;
    Ok(format!(
        "installed the kanbanr skill {} at {}",
        env!("CARGO_PKG_VERSION"),
        dir.display()
    ))
}

/// Remove kanbanr's own copy; leave anything else.
pub fn uninstall_at(dir: &Path) -> Result<String> {
    match status_at(dir) {
        Status::Missing => Ok(format!("no skill at {}", dir.display())),
        Status::Foreign { .. } => Ok(format!(
            "left {} alone: kanbanr did not install it",
            dir.display()
        )),
        Status::Installed { .. } => {
            std::fs::remove_dir_all(dir)?;
            Ok(format!("removed {}", dir.display()))
        }
    }
}

/// `kanbanr skill status`, in words.
pub fn describe(dir: &Path) -> String {
    match status_at(dir) {
        Status::Missing => format!(
            "no kanbanr skill at {} — `kanbanr skill install` adds the one this program carries",
            dir.display()
        ),
        Status::Installed {
            version,
            current: true,
        } => {
            format!("skill {version} at {}: matches this program", dir.display())
        }
        Status::Installed {
            version,
            current: false,
        } => format!(
            "skill {version} at {}: does NOT match this program ({}) — `kanbanr skill install` \
             updates it",
            dir.display(),
            env!("CARGO_PKG_VERSION")
        ),
        Status::Foreign { link: Some(to) } => format!(
            "{} links to {}: you manage it; kanbanr leaves it alone",
            dir.display(),
            to.display()
        ),
        Status::Foreign { link: None } => format!(
            "{} was not written by kanbanr (a plugin or another tool's copy); kanbanr leaves it alone",
            dir.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kanbanr-skill-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn skill_flat() -> String {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        skill.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The flattened "Tests are evidence, not intentions" section of the skill.
    fn tests_section(flat: &str) -> &str {
        flat.split("## Tests are evidence, not intentions")
            .nth(1)
            .and_then(|r| r.split("## Every change points back").next())
            .expect("tests section")
    }

    /// DEF-073 R-1: the skill keeps walkthrough tests to what a kanbanr runbook can observe on the
    /// board, the monitor or the remote, never the product's own behaviour.
    #[test]
    fn skill_keeps_walkthrough_tests_to_what_a_runbook_can_observe() {
        let flat = skill_flat();
        let section = tests_section(&flat);
        for needle in [
            "A test of kind `walkthrough` is only for what a kanbanr runbook can observe on the board, the monitor or the remote",
            "The product's own behaviour is never a walkthrough test",
            "A runbook drives kanbanr, not the project's app",
        ] {
            assert!(section.contains(needle), "the skill lacks: {needle}");
        }
        assert!(
            !flat.contains("Anything a run can see"),
            "the skill still says: Anything a run can see"
        );
    }

    /// DEF-073 R-2: the skill records a test the person named with the kind and name they gave,
    /// and never changes either without their agreement.
    #[test]
    fn skill_keeps_the_kind_and_name_of_a_test_the_person_named() {
        let flat = skill_flat();
        let section = tests_section(&flat);
        for needle in [
            "A test the person named keeps its kind and its name",
            "record it with the kind and the words they gave",
            "Never change it afterwards on your own",
            "propose it and make it only when the person agrees",
        ] {
            assert!(section.contains(needle), "the skill lacks: {needle}");
        }
        let needle = "a test the person named keeps theirs";
        assert!(flat.contains(needle), "the skill lacks: {needle}");
    }

    /// DEF-073 R-3: the skill settles each test's name in the runner's form before it asks the
    /// person to approve a definition.
    #[test]
    fn skill_settles_test_names_in_runner_form_before_approval() {
        let flat = skill_flat();
        let section = tests_section(&flat);
        for needle in [
            "Settle test names before asking for approval",
            "the form the project's runner prints and capture reads",
            "before the approval request, never after it",
            "Settle that form before you ask for approval",
        ] {
            assert!(section.contains(needle), "the skill lacks: {needle}");
        }
        for needle in [
            "Before you ask for approval, settle what would otherwise change after it",
            "So settle test kinds and runner-form names before the first approval, not after it",
        ] {
            assert!(flat.contains(needle), "the skill lacks: {needle}");
        }
    }

    /// DEF-084 R-1: the skill still asks the styling approach when the person has described only
    /// the look, and records as the person's decision only what they said.
    #[test]
    fn skill_asks_the_styling_approach_even_when_the_look_is_given() {
        let flat = skill_flat();
        for needle in [
            "Ask how a screen should look before designing it",
            "If the person has already described the look",
            "still ask the styling approach",
            "Record as the person's decision only what they said",
            "Anything you chose is your recommendation until they agree",
        ] {
            assert!(flat.contains(needle), "the skill lacks: {needle}");
        }
    }

    /// DEF-084 R-2: the coordinator tells each build agent to run the project's formatter and
    /// linter before each commit.
    #[test]
    fn skill_has_build_agents_format_and_lint_before_each_commit() {
        let flat = skill_flat();
        let duties = flat
            .split("As the coordinator you:")
            .nth(1)
            .and_then(|r| r.split("**report changes, not events.**").next())
            .expect("the coordinator's duties");
        for needle in [
            "run the project's formatter and linter",
            "before each commit",
        ] {
            assert!(
                duties.contains(needle),
                "the coordinator's duties lack: {needle}"
            );
        }
    }

    /// DEF-084 R-3: the skill reads the queue and `kanbanr check` before saying what waits, names a
    /// sign-off only when offered, and never lists a recorded decision or check as waiting.
    #[test]
    fn skill_reads_the_queue_before_saying_what_waits() {
        let flat = skill_flat();
        let section = flat
            .split("### Waiting decisions")
            .nth(1)
            .and_then(|r| r.split("### Designing a process with the user").next())
            .expect("waiting decisions section");
        for needle in [
            "Read before you report what waits",
            "`kanbanr check <CODE> --json`",
            "Name a sign-off only when the queue offers it now",
            "never \"you can do both now\"",
            "a manual check is waiting only while its test is not green",
        ] {
            assert!(
                section.contains(needle),
                "the waiting decisions section lacks: {needle}"
            );
        }
    }

    /// DEF-084 R-4: the skill gives a monitor page's address with the `/p/<project>/` prefix.
    #[test]
    fn skill_links_pages_with_the_project_prefix() {
        let flat = skill_flat();
        for needle in [
            "`<monitor>/p/<project>/review`",
            "Never shorten it to `<monitor>/review`",
        ] {
            assert!(flat.contains(needle), "the skill lacks: {needle}");
        }
    }

    /// DEF-084 R-5: the skill never names a kanbanr command or a shell line as a test, a chore's
    /// check included.
    #[test]
    fn skill_names_no_kanbanr_command_as_a_test() {
        let flat = skill_flat();
        let section = tests_section(&flat);
        for needle in [
            "a chore's check included",
            "It is never a kanbanr command (`kanbanr doctor`, `kanbanr check`) or a shell line",
            "capture records nothing",
        ] {
            assert!(section.contains(needle), "the skill lacks: {needle}");
        }
    }

    /// DEF-056 R-1: the first release proposes release-prepare and the pipeline before Promote.
    #[test]
    fn the_first_release_proposes_release_prepare_and_the_pipeline() {
        let flat = skill_flat();
        let section = flat
            .split("**The first release has a checklist")
            .nth(1)
            .and_then(|r| r.split("A started sprint is a record").next())
            .expect("first-release checklist");
        for w in [
            "kanbanr config release-prepare",
            "CHANGELOG",
            "has_ci",
            "pipeline item",
            "before you name Promote to candidate",
        ] {
            assert!(section.contains(w), "missing {w}");
        }
        assert!(!flat.contains("`docs/src/guides/delivery-pipeline.md` says"));
        for w in ["manual trigger", "scan", "publish"] {
            assert!(flat.contains(w), "missing {w}");
        }
    }

    /// DEF-056 R-2: an override follows a fresh check.
    #[test]
    fn an_override_follows_a_fresh_check() {
        let flat = skill_flat();
        let section = flat
            .split("### Moving work")
            .nth(1)
            .and_then(|r| r.split("###").next())
            .expect("moving work");
        assert!(
            section.contains("Before any `--override`, run `kanbanr check <CODE> --json` again")
        );
        assert!(section.contains("move without `--override`"));
        assert!(section.contains("records no override a gate did not need"));
    }

    /// DEF-056 R-3: a chore Claude files asks the once-only questions.
    #[test]
    fn a_chore_claude_files_asks_the_once_only_questions() {
        let flat = skill_flat();
        assert!(flat.contains("A chore you file yourself"));
        assert!(flat.contains("statement, goal and one EARS invariant"));
        assert!(flat.contains(
            "**one** AskUserQuestion (confirm, or change) before `kanbanr feature add --kind chore`"
        ));
    }

    /// DEF-060 R-1: the skill offers the allow rule for a granted delegation, once, and never edits
    /// Claude Code's settings by hand.
    #[test]
    fn skill_offers_the_permission_rule_for_a_granted_delegation() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        let flat = skill.split_whitespace().collect::<Vec<_>>().join(" ");
        for needle in [
            "kanbanr claude permissions --write",
            "permission_rules_missing",
            "**ask once**",
            "fall back to `kanbanr prepare",
            "Never edit `.claude/settings*.json` by hand",
            "kanbanr offers the Claude Code allow rule",
        ] {
            assert!(flat.contains(needle), "the skill lacks: {needle}");
        }
    }

    /// DEF-045 R-1: the skill has Claude bring a release fix forward itself and report the merge.
    #[test]
    fn skill_has_claude_bring_a_release_fix_forward() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        let flat = skill.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains("run `kanbanr git forward <V>` yourself"),
            "the skill does not have Claude run the forward"
        );
        assert!(!flat.contains("it is never done for you"));
    }

    /// DEF-027 R-9: the skill tells the coordinator to stay in the project folder and look inside an
    /// agent's worktree with `git -C`.
    #[test]
    fn skill_keeps_the_coordinator_out_of_agent_worktrees() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        let section = skill
            .split("**Issue a scope before each sub-agent.**")
            .nth(1)
            .expect("the skill has the sub-agent scope section")
            .split("\n## ")
            .next()
            .unwrap();
        for want in [
            "git -C",
            "never `cd` into an agent",
            "project folder",
            "never restricts you",
        ] {
            assert!(
                section.contains(want),
                "the sub-agent section says {want:?}"
            );
        }
    }

    /// DEF-015 R-4: the skill tells the coordinator what a temporary board is and the form an agent
    /// uses to name one.
    #[test]
    fn skill_names_the_temporary_board_form() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        for want in ["temporary board", "$TMPDIR", "--data-dir"] {
            assert!(skill.contains(want), "the skill says {want:?}");
        }
        assert!(!skill.contains("temporary boards only"));
    }

    /// FEAT-200 R-18: the skill tells Claude how to write diagrams that draw — quote flowchart
    /// labels, plain words in sequence messages, never `end` as an id, `kanbanr doctor` after — and
    /// what a `diagrams_valid` gate asks of it.
    #[test]
    fn skill_carries_diagram_guidance() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        let section = skill
            .split("### Diagrams")
            .nth(1)
            .expect("the skill has a Diagrams section")
            .split("\n## ")
            .next()
            .unwrap();
        for want in [
            "Quote flowchart labels",
            "plain words",
            "Never use `end` as an id",
            "kanbanr doctor",
        ] {
            assert!(section.contains(want), "the Diagrams section says {want:?}");
        }
        assert!(
            skill.contains("| `diagrams_valid` |"),
            "the checks table covers it"
        );
        assert!(skill.contains("`diagrams_valid` gate"));
    }

    /// FTR-023: the skill says a design-review covers the whole spec as it was when given.
    #[test]
    fn skill_says_design_review_covers_the_whole_spec() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        assert!(
            skill.contains("**A design-review covers the whole spec as it was when given.**"),
            "the skill says a design-review covers the whole spec"
        );
    }

    /// DEF-023: the skill says a `--worktree` scope is bound at once and its brief has no claim step.
    #[test]
    fn skill_says_a_worktree_flag_binds_the_scope() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        assert!(
            skill.contains("--worktree") && skill.contains("no claim step"),
            "the skill says --worktree binds the scope and the brief has no claim step"
        );
    }

    /// FEAT-197 R-16 (its automatable half): the skill says when to list the view docs, index a
    /// new ADR, fix coverage warnings before a move and run doctor before a sprint merge or a cut,
    /// what a `docs_cover` gate asks, and names `config coverage`. Whether the guidance matches the
    /// book's is the manual check.
    #[test]
    fn skill_carries_coverage_guidance() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        for want in [
            "| `docs_cover` | **never a question** — fix the coverage",
            "Include the project's view\n  docs",
            "add the links each one names before you move the item on",
            "**Index every decision.**",
            "**Before merging a sprint or cutting a release**, run `kanbanr doctor`",
            "kanbanr config coverage [--from-file rules.yaml | --clear]",
        ] {
            assert!(skill.contains(want), "the skill says {want:?}");
        }
    }

    /// The skill's text.
    fn skill_md() -> String {
        SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap()
    }

    /// FTR-024 R-1, R-2: every verdict goes to the Review or Releases page, with what is waiting
    /// said in one message; with no monitor on writes Claude offers to start one, never records.
    #[test]
    fn skill_sends_decisions_to_the_monitor() {
        let skill = skill_md();
        for want in [
            "## Agreement before work: decided on the pages",
            "You never record a verdict, and never ask for one in a plan or a question",
            "kanbanr open --check",
            "kanbanr review --ui",
        ] {
            assert!(skill.contains(want), "the skill says {want:?}");
        }
        for gone in [
            "work the queue as questions",
            "The user accepting the plan is their approval",
        ] {
            assert!(!skill.contains(gone), "the skill no longer says {gone:?}");
        }
    }

    /// FTR-028 R-5: the skill says where decisions are taken is the project's choice, how Claude
    /// acts under a delegation, and that setup proposes the process's delegations under
    /// **Decisions** in the plan.
    #[test]
    fn skill_proposes_delegations_at_setup() {
        let skill = skill_md();
        for want in [
            "Where decisions are taken is the project's choice",
            "`--under DLG-n`",
            "**Decisions**",
            "kanbanr decisions defaults --process <name> --dry-run --json",
            "Only you, the coordinator, act under a delegation",
            // Back-moves are the person's (decided by the maintainer, 13 Nov 2026).
            "When an item has moved back to an earlier stage, ask the user before taking any\ndecision or making any change for it, even if a delegation would cover it",
        ] {
            assert!(skill.contains(want), "the skill says {want:?}");
        }
    }

    /// DEF-083 R-3: the skill has Claude commit a refreshed CLAUDE.md block on its own at once,
    /// the charter agreed after setup's commit included; setup ignores the worktrees folder (Q2).
    #[test]
    fn skill_commits_a_refreshed_block_on_its_own() {
        let skill = skill_md();
        let flat = skill.split_whitespace().collect::<Vec<_>>().join(" ");
        for want in [
            "If the charter is agreed, or changed, after that commit",
            "git add CLAUDE.md && git commit -m \"[no-ref] refresh the kanbanr block\"",
            "on an item's branch it goes in that item's next commit",
        ] {
            assert!(flat.contains(want), "the skill says {want:?}");
        }
        let printf = skill
            .lines()
            .find(|l| l.trim_start().starts_with("printf '.claude/settings.json"))
            .expect("the plan's .gitignore line");
        assert!(printf.contains(".claude/worktrees/"), "{printf}");
    }

    /// FTR-024 R-6: plan mode is for setup and the process, and never while a sub-agent runs.
    #[test]
    fn skill_keeps_plan_mode_for_setup_and_process() {
        let skill = skill_md();
        assert!(skill.contains("never while a sub-agent is running"));
        assert!(skill.contains("### A new item: scope it in the conversation"));
        assert!(!skill.contains("scope it in plan mode"));
    }

    /// FTR-024 R-7: progress is reported as changes, not events.
    #[test]
    fn skill_reports_changes_not_events() {
        let skill = skill_md();
        assert!(skill.contains("**report changes, not events.**"));
        assert!(skill.contains("Never write 'already reported'"));
    }

    /// FTR-024 R-8: rework goes on the item's todo-list before anyone starts it.
    #[test]
    fn skill_adds_rework_before_starting_it() {
        assert!(skill_md().contains("**put rework on the board before it starts.**"));
    }

    /// DEF-029 R-6: the skill tells Claude to plan a release item into the running sprint, and to
    /// leave the sign-off that holds a merged item to the person.
    #[test]
    fn skill_plans_release_items_into_the_running_sprint() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).replace("\n  ", " "))
            .unwrap();
        for want in [
            "When a sprint is running, plan the item into it too (`kanbanr sprint plan <SP> <CODE>`) unless the person said otherwise; `release plan` names the ones it missed.",
            "A promotion held for a merged item names the sign-off it lacks: send the person to the Review page; never record it.",
        ] {
            assert!(skill.contains(want), "the skill says {want:?}");
        }
    }

    /// FEAT-215 R-1: the skill states each sub-agent role's boundaries, and the coordinator's
    /// duty to brief with them and keep the board current.
    #[test]
    fn skill_states_sub_agent_boundaries() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        for want in [
            "### Sub-agents: what each may do",
            "A **build** agent",
            "A **design** agent",
            "the brief states them **verbatim**",
            "never: no `git push`, no `kanbanr sync`",
            "so the board never lags the code",
        ] {
            assert!(skill.contains(want), "the skill says {want:?}");
        }
    }

    /// DEF-031 R-9: the skill tells Claude that a merge sends a promoted release back, to report
    /// it, and to confirm no stage.
    #[test]
    fn skill_reports_a_release_sent_back_and_confirms_no_stage() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        for want in [
            "**A merge sends a promoted release back.**",
            "sends the release back to candidate and says so",
            "Confirm no stage yourself",
            "needs a patch release",
        ] {
            assert!(skill.contains(want), "the skill says {want:?}");
        }
    }

    /// FEAT-214 R-12: the skill tells Claude to issue a scope with one command before starting
    /// each sub-agent, to start it in its own worktree (design agents too), to put the printed
    /// brief into its brief, and to revoke the scope when the work is reviewed.
    #[test]
    fn skill_issues_a_scope_before_each_sub_agent() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        let section = skill
            .split("### Sub-agents: what each may do")
            .nth(1)
            .expect("the sub-agents subsection")
            .split("\n### ")
            .next()
            .unwrap();
        for want in [
            "**Issue a scope before each sub-agent.**",
            "kanbanr scope issue <design|build> <CODE>",
            "`isolation: worktree`, design agents included",
            "paste the printed brief **verbatim** at the top of the agent's brief",
            "kanbanr scope claim SC-…",
            "kanbanr scope revoke SC-… --reason\n  done",
            "never issue scopes from inside an agent",
            "a `batch` only of these ops",
            "decide it or ask the user",
        ] {
            assert!(section.contains(want), "the subsection says {want:?}");
        }
        assert!(
            skill.contains("under a build scope issued for the item (next section)"),
            "the build role points to it"
        );
        assert!(
            skill.contains("A design subagent works under a design scope"),
            "the design role points to it"
        );
    }

    /// FEAT-194 R-1: the skill states the method once, by stage role: a section with one
    /// subsection per role, naming the commands and gates that now carry each rule.
    #[test]
    fn skill_states_method_by_role() {
        let skill = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).to_string())
            .unwrap();
        let section = skill
            .split("\n## The method by stage role\n")
            .nth(1)
            .expect("the skill has the method section")
            .split("\n## ")
            .next()
            .unwrap();
        for role in ["design", "build", "integrate", "release"] {
            assert!(
                section.contains(&format!("\n### At the {role} role\n")),
                "a subsection for the {role} role"
            );
        }
        assert!(
            section.contains("### The design stage: the spec carries the build-ready design"),
            "the design stage subsection lives in it"
        );
        for want in [
            "kanbanr git model",
            "kanbanr git doctor",
            "kanbanr config method",
            "kanbanr release promote",
            "kanbanr sprint close",
            "affects_docs",
            "docs-folded",
            "design-review",
        ] {
            assert!(section.contains(want), "the method section names {want:?}");
        }
    }

    /// FEAT-141 R-1: the skill written is the one embedded, stamped, and recognised as current; a
    /// stale copy kanbanr wrote is replaced.
    #[test]
    fn install_writes_the_embedded_skill_and_status_matches() {
        let root = temp("install");
        let dir = root.join("skills").join("kanbanr");
        assert_eq!(status_at(&dir), Status::Missing);
        install_at(&dir).unwrap();
        let skill = std::fs::read(dir.join("SKILL.md")).unwrap();
        let embedded = SKILL_FILES
            .iter()
            .find(|(r, _)| *r == "SKILL.md")
            .unwrap()
            .1;
        assert_eq!(
            skill, embedded,
            "the skill on disk is the one this binary carries"
        );
        assert!(dir.join("hooks").join("session-start.sh").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("hooks/session-start.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "hook scripts are executable");
        }
        assert!(matches!(
            status_at(&dir),
            Status::Installed { current: true, .. }
        ));

        // An older copy of ours is noticed and replaced.
        std::fs::write(dir.join("SKILL.md"), "an older skill").unwrap();
        std::fs::write(dir.join(STAMP), "version: 0.0.1\ndigest: stale\n").unwrap();
        assert!(matches!(
            status_at(&dir),
            Status::Installed { current: false, .. }
        ));
        install_at(&dir).unwrap();
        assert_eq!(std::fs::read(dir.join("SKILL.md")).unwrap(), embedded);
        assert!(uninstall_at(&dir).unwrap().starts_with("removed"));
        assert_eq!(status_at(&dir), Status::Missing);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// FEAT-141 R-2: a skill kanbanr did not write — a link to a clone, or another tool's folder — is
    /// left exactly as it is, by install and by uninstall.
    #[cfg(unix)]
    #[test]
    fn a_linked_skill_is_left_alone() {
        let root = temp("linked");
        let clone = root.join("clone-skill");
        std::fs::create_dir_all(&clone).unwrap();
        std::fs::write(clone.join("SKILL.md"), "the maintainer's working copy").unwrap();
        let skills = root.join("skills");
        std::fs::create_dir_all(&skills).unwrap();
        let dir = skills.join("kanbanr");
        std::os::unix::fs::symlink(&clone, &dir).unwrap();

        assert!(matches!(status_at(&dir), Status::Foreign { link: Some(_) }));
        assert!(install_at(&dir).unwrap().starts_with("left"));
        assert!(uninstall_at(&dir).unwrap().starts_with("left"));
        assert_eq!(
            std::fs::read_to_string(clone.join("SKILL.md")).unwrap(),
            "the maintainer's working copy"
        );
        assert!(
            std::fs::symlink_metadata(&dir)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        // Another tool's plain folder, with no stamp, is just as off-limits.
        let other = root.join("other").join("kanbanr");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("SKILL.md"), "someone else's").unwrap();
        assert!(install_at(&other).unwrap().starts_with("left"));
        assert_eq!(
            std::fs::read_to_string(other.join("SKILL.md")).unwrap(),
            "someone else's"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
