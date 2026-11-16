//! The link from a code change back to the item that justifies it (FEAT-056).
//!
//! Two halves, both deliberately small. A **commit trailer** (`Refs: kanbanr:FEAT-046/R-2`) is the
//! durable machine-readable link: it survives squashes, rebases and the refactors that destroy
//! `git blame`. A **branch per item** (`feat/FEAT-046-measurement`) is the runtime signal of what
//! is being worked on right now, which is what lets everything else stop asking — the commit hook
//! knows which reference to suggest, the test-capture hook knows which item a run belongs to, and
//! `finish` knows which definition to check the work against.
//!
//! Nothing here touches git. It parses text and validates ids against a loaded board, so the CLI
//! hooks, the daemon and the tests all agree on what a reference means. The commit and branch
//! rules are here too (FEAT-176), as verdicts over [`RepoFacts`] the front end collected: core
//! decides, the front end reads the repository and says the verdict its own way.

use crate::config::ProjectConfig;
use crate::gitmodel::{BranchFacts, GitModel, LineVerdict, line_commit_verdict};
use crate::store::Project;

/// A reference from a commit to somewhere on the board, at the level the author chose.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "level", rename_all = "lowercase")]
pub enum ItemRef {
    /// `kanbanr:FEAT-046` — the item as a whole.
    Feature { code: String },
    /// `kanbanr:FEAT-046/R-2` — the requirement this change exists to satisfy.
    Requirement { code: String, requirement: String },
    /// `kanbanr:FEAT-046/TL-001/T3` — the task it completes.
    Task {
        code: String,
        todo: String,
        task: String,
    },
}

impl ItemRef {
    pub fn code(&self) -> &str {
        match self {
            ItemRef::Feature { code }
            | ItemRef::Requirement { code, .. }
            | ItemRef::Task { code, .. } => code,
        }
    }

    pub fn as_token(&self) -> String {
        match self {
            ItemRef::Feature { code } => format!("kanbanr:{code}"),
            ItemRef::Requirement { code, requirement } => format!("kanbanr:{code}/{requirement}"),
            ItemRef::Task { code, todo, task } => format!("kanbanr:{code}/{todo}/{task}"),
        }
    }
}

/// The marker that opens a reference anywhere in the message.
const MARKER: &str = "kanbanr:";

/// The other two things a commit can point at (FEAT-057): a document that explains it, and a
/// decision it rests on. These are trailers proper — `Docs:` and `ADR:` at the start of a line —
/// because unlike `kanbanr:` they have no distinctive marker, and matching a bare path anywhere
/// in prose would fire on every sentence that mentions a file.
const DOC_TRAILER: &str = "Docs:";
const ADR_TRAILER: &str = "ADR:";

/// Paths named by `Docs:` trailers.
pub fn parse_doc_refs(message: &str) -> Vec<String> {
    trailer_values(message, DOC_TRAILER)
}

/// Decision ids named by `ADR:` trailers.
pub fn parse_adr_refs(message: &str) -> Vec<String> {
    trailer_values(message, ADR_TRAILER)
}

/// Values of a trailer, comma- or space-separated, continuation lines included — a long list
/// wraps, and a rule that ignored the wrapped part would teach people the check is unreliable.
fn trailer_values(message: &str, trailer: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_trailer = false;
    for line in message.lines() {
        let body = match line.trim_start().strip_prefix(trailer) {
            Some(rest) => {
                in_trailer = true;
                rest
            }
            // A continuation is an indented line directly under the trailer.
            None if in_trailer
                && line.starts_with(char::is_whitespace)
                && !line.trim().is_empty() =>
            {
                line
            }
            None => {
                in_trailer = false;
                continue;
            }
        };
        for value in body.split([',', ' ', '\t']) {
            let value = value.trim().trim_end_matches(['.', ';']);
            if !value.is_empty() && !out.iter().any(|v| v == value) {
                out.push(value.to_string());
            }
        }
    }
    out
}

/// Every reference in a commit message, in the order they appear, de-duplicated.
///
/// References are collected from the whole message rather than only from a `Refs:` line, because a
/// long list of them wraps, and a rule that silently ignores the continuation line would teach
/// people that the hook is unreliable. A token in prose is still a claim about the board, and is
/// validated like any other.
pub fn parse_refs(message: &str) -> Vec<ItemRef> {
    let mut out: Vec<ItemRef> = Vec::new();
    let mut rest = message;
    while let Some(at) = rest.find(MARKER) {
        rest = &rest[at + MARKER.len()..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .unwrap_or(rest.len());
        let token = rest[..end].trim_end_matches(['.', ')', ']', '"', '\'']);
        rest = &rest[end..];
        if let Some(parsed) = parse_token(token)
            && !out.contains(&parsed)
        {
            out.push(parsed);
        }
    }
    out
}

fn parse_token(token: &str) -> Option<ItemRef> {
    let mut parts = token.split('/').map(str::trim).filter(|p| !p.is_empty());
    let code = parts.next()?.to_string();
    if code.is_empty() {
        return None;
    }
    match (parts.next(), parts.next(), parts.next()) {
        (None, _, _) => Some(ItemRef::Feature { code }),
        (Some(second), None, _) => Some(ItemRef::Requirement {
            code,
            requirement: second.to_string(),
        }),
        (Some(todo), Some(task), None) => Some(ItemRef::Task {
            code,
            todo: todo.to_string(),
            task: task.to_string(),
        }),
        // Deeper than the board goes: report it as unparseable rather than guessing a level.
        _ => None,
    }
}

/// The recorded escape: a commit that deliberately references nothing, and says why.
///
/// It has to exist and it has to cost a sentence. Without an escape people learn `--no-verify`,
/// which removes the check entirely and leaves no record at all; with one, the reason lives in the
/// commit message forever, where a reviewer reads it.
/// It has to **open a line**, too. This module found that out the hard way: a commit message that
/// merely explained the escape in prose was read as one, and a commit with a perfectly good
/// trailer was waved through as unreferenced.
pub fn escape_reason(message: &str) -> Option<&str> {
    message.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix("[no-ref]")?;
        let reason = rest.trim_start_matches([':', ' ', '\t']).trim();
        (!reason.is_empty()).then_some(reason)
    })
}

/// Commits git writes on the author's behalf, which carry no reference of their own.
pub fn is_generated_commit(message: &str) -> bool {
    let first = message.lines().next().unwrap_or("").trim_start();
    first.starts_with("Merge ")
        || first.starts_with("Revert ")
        || first.starts_with("fixup!")
        || first.starts_with("squash!")
        || first.starts_with("amend!")
}

/// What is wrong with these references, as sentences a person can act on. Empty means they all
/// resolve — an id that does not exist is a broken link, exactly like a dangling dependency.
pub fn validate_refs(project: &Project, refs: &[ItemRef]) -> Vec<String> {
    let mut problems = Vec::new();
    for r in refs {
        let Ok(feature) = project.feature(r.code()) else {
            problems.push(format!(
                "{} names {}, which is not an item on this board",
                r.as_token(),
                r.code()
            ));
            continue;
        };
        match r {
            ItemRef::Feature { .. } => {}
            ItemRef::Requirement { code, requirement } => {
                let known = feature
                    .definition
                    .as_ref()
                    .is_some_and(|d| d.requirements.iter().any(|req| &req.id == requirement));
                if !known {
                    problems.push(format!(
                        "{} names {requirement}, which is not a requirement of {code}",
                        r.as_token()
                    ));
                }
            }
            ItemRef::Task { code, todo, task } => {
                let list = feature.todo_lists.iter().find(|l| &l.code == todo);
                match list {
                    None => problems.push(format!(
                        "{} names {todo}, which is not a todo-list of {code}",
                        r.as_token()
                    )),
                    Some(list) if !list.tasks.iter().any(|t| &t.key == task) => {
                        problems.push(format!(
                            "{} names {task}, which is not a task in {todo}",
                            r.as_token()
                        ))
                    }
                    Some(_) => {}
                }
            }
        }
    }
    problems
}

// ---- branch per item ---------------------------------------------------------------------

/// The default branch pattern. `{code}` and `{slug}` are the only placeholders.
pub const DEFAULT_BRANCH_PATTERN: &str = "feat/{code}-{slug}";

/// Branches that are allowed to exist without an item: exploratory work whose output is a
/// definition change, never merged code.
pub const SPIKE_PREFIX: &str = "spike/";

/// A sprint's line of integration, `sprint/SP-001` (FEAT-189).
pub const SPRINT_PREFIX: &str = "sprint/";
/// A release's branch, `release/1.2.0` (FEAT-183, FEAT-189).
pub const RELEASE_PREFIX: &str = "release/";

/// The `fix/` twin of a pattern that starts `feat/`: where a defect's branch goes (FEAT-189, Q3).
pub fn fix_twin(pattern: &str) -> Option<String> {
    let pattern = if pattern.trim().is_empty() {
        DEFAULT_BRANCH_PATTERN
    } else {
        pattern
    };
    pattern
        .strip_prefix("feat/")
        .map(|rest| format!("fix/{rest}"))
}

/// The item a branch names, through the project's pattern or its `fix/` twin (FEAT-189).
pub fn item_code_from_branch(pattern: &str, branch: &str) -> Option<String> {
    code_from_branch(pattern, branch).or_else(|| {
        let twin = fix_twin(pattern)?;
        code_from_branch(&twin, branch)
    })
}

/// A git-safe, readable form of a title: lowercase words joined by `-`, cut to a sane length.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn branch_for(pattern: &str, code: &str, title: &str) -> String {
    let pattern = if pattern.trim().is_empty() {
        DEFAULT_BRANCH_PATTERN
    } else {
        pattern
    };
    pattern
        .replace("{code}", code)
        .replace("{slug}", &slug(title))
}

/// The item a branch names, if any. Matched against the pattern's fixed parts so a renamed slug
/// still resolves — the code is the identity, the slug is for the human reading `git branch`.
pub fn code_from_branch(pattern: &str, branch: &str) -> Option<String> {
    let pattern = if pattern.trim().is_empty() {
        DEFAULT_BRANCH_PATTERN
    } else {
        pattern
    };
    let (before, after) = pattern.split_once("{code}")?;
    let rest = branch.strip_prefix(before)?;
    // A code is `PREFIX-123`, and the separator that follows it in the pattern is usually `-` too
    // — so the code has to be recognised by its own shape, not by the next dash.
    if let Some(code) = code_prefix(rest) {
        return Some(code);
    }
    // No code shape: fall back to whatever the pattern's next fixed part bounds.
    let end = after
        .split("{slug}")
        .next()
        .filter(|s| !s.is_empty())
        .and_then(|sep| rest.find(sep))
        .unwrap_or(rest.len());
    let code = rest[..end].trim_matches('-');
    (!code.is_empty()).then(|| code.to_string())
}

/// `FEAT-053` out of `FEAT-053-measurement-history`: letters, a dash, digits.
fn code_prefix(rest: &str) -> Option<String> {
    let (prefix, tail) = rest.split_once('-')?;
    if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then(|| format!("{prefix}-{digits}"))
}

// ---- commit and branch policy (FEAT-176) -------------------------------------------------

/// What the commit rules read from the code repository, collected by the front end (the CLI's
/// hooks and guard) so that the rules themselves stay pure and the same everywhere
/// ([`crate::coderepo::repo_facts`] reads them from a working tree).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoFacts {
    /// The checked-out branch; `None` when the head is detached (a rebase, a bisect).
    pub branch: Option<String>,
    /// The branch work is not supposed to land on directly.
    pub default_branch: String,
    /// A merge is waiting to be committed (FEAT-094).
    pub merging: bool,
    /// The repository has a commit yet: a fresh `git init` has none (FEAT-105).
    pub has_commits: bool,
    /// The commit message, where it is known: the commit-msg hook reads the real one, the guard
    /// only an inline `-m`; `None` where there is none to read.
    pub message: Option<String>,
    /// The repository has commits, its HEAD holds no top-level `.kanbanr`, and the working tree
    /// does: setup's marker has not been committed yet (DEF-067).
    pub setup_pending: bool,
    /// The paths staged for this commit, repository-relative with `/` separators, where the caller
    /// read them (the pre-commit hook); `None` where they are not known (the guard, commit-msg).
    pub staged: Option<Vec<String>>,
    /// CLAUDE.md differs from HEAD's only inside generated blocks (kanbanr's or any other tool's,
    /// `<!-- <tool>:begin -->` … `<!-- <tool>:end -->`), in what the caller read: the index for
    /// pre-commit, the index or the working tree for the guard and commit-msg (DEF-083, DEF-092).
    /// HEAD with no CLAUDE.md reads as an empty file.
    pub block_refresh: bool,
}

/// What setup's own first commit holds (DEF-067): the marker, the CLAUDE.md block, the .gitignore.
pub const SETUP_FILES: [&str; 3] = [".kanbanr", "CLAUDE.md", ".gitignore"];

/// Is this setup's own first commit in a repository that already has history (DEF-067)? True
/// when all of these hold: `has_commits`, `setup_pending`, `!merging`, `branch == Some(default_branch)`;
/// the message, where known, has no reference and has `[no-ref] <why>`; the staged paths, where
/// known, are non-empty, all in SETUP_FILES and include `.kanbanr`. Pure.
///
/// The root commit has its own exception (FEAT-105); this is its sibling for a repository that
/// had history before kanbanr came: `kanbanr init` writes the marker and registers the guard in
/// one step, so without it the marker could never reach the default branch, and worktrees cut
/// from that branch would find no board.
pub fn setup_commit(facts: &RepoFacts) -> bool {
    if !facts.has_commits
        || !facts.setup_pending
        || facts.merging
        || facts.branch.as_deref() != Some(facts.default_branch.as_str())
    {
        return false;
    }
    if let Some(message) = facts.message.as_deref()
        && !(parse_refs(message).is_empty() && escape_reason(message).is_some())
    {
        return false;
    }
    if let Some(staged) = &facts.staged
        && (staged.is_empty()
            || !staged.iter().all(|p| SETUP_FILES.contains(&p.as_str()))
            || !staged.iter().any(|p| p == crate::project::MARKER_FILE))
    {
        return false;
    }
    true
}

/// The sentence added to a default-branch or line refusal while `setup_pending` holds on the
/// default branch; `None` otherwise.
pub fn setup_hint(facts: &RepoFacts) -> Option<&'static str> {
    (facts.setup_pending
        && !facts.merging
        && facts.branch.as_deref() == Some(facts.default_branch.as_str()))
    .then_some(
        "Setup's own first commit is let through: stage only .kanbanr, CLAUDE.md and \
         .gitignore, and commit them with `[no-ref] initial setup`.",
    )
}

/// The one file a block refresh holds (DEF-083).
pub const BLOCK_FILE: &str = "CLAUDE.md";

/// Is this a refresh of CLAUDE.md's generated blocks on the default branch (DEF-083, DEF-092)?
/// Any tool's block counts, kanbanr's or katalogr's alike. True when
/// all hold: `has_commits`, `block_refresh`, `!merging`, `branch == Some(default_branch)`; the
/// message, where known, has no reference and has `[no-ref] <why>`; the staged paths, where
/// known, are exactly `[BLOCK_FILE]`. Pure.
///
/// A sibling of setup's commit ([`setup_commit`]): `kanbanr claude sync` rewrites the block on
/// the default branch whenever the charter or the board's address changes after setup, and
/// without this the change could never be committed there, leaving the checkout dirty.
pub fn block_commit(facts: &RepoFacts) -> bool {
    if !facts.has_commits
        || !facts.block_refresh
        || facts.merging
        || facts.branch.as_deref() != Some(facts.default_branch.as_str())
    {
        return false;
    }
    if let Some(message) = facts.message.as_deref()
        && !(parse_refs(message).is_empty() && escape_reason(message).is_some())
    {
        return false;
    }
    if let Some(staged) = &facts.staged
        && !(staged.len() == 1 && staged[0] == BLOCK_FILE)
    {
        return false;
    }
    true
}

/// The sentence added to a default-branch or line refusal while `block_refresh` holds on the
/// default branch; `None` otherwise.
pub fn block_hint(facts: &RepoFacts) -> Option<&'static str> {
    (facts.block_refresh
        && !facts.merging
        && facts.branch.as_deref() == Some(facts.default_branch.as_str()))
    .then_some(
        "A refresh of CLAUDE.md's generated blocks is let through too: stage only CLAUDE.md, \
         change nothing outside the `<!-- <tool>:begin -->` … `<!-- <tool>:end -->` markers, \
         and commit it with `[no-ref] refresh the generated block`.",
    )
}

/// A refusal with every hint that applies after it, setup's first and then the block's, each
/// after a blank line (DEF-067, DEF-083).
fn refuse_with_hint(facts: &RepoFacts, why: String) -> Verdict {
    let mut text = why;
    for hint in [setup_hint(facts), block_hint(facts)].into_iter().flatten() {
        text.push_str("\n\n");
        text.push_str(hint);
    }
    Verdict::refuse(text)
}

/// What a commit rule says: whether the commit may happen, and what the person should hear
/// either way. Every refusal says what to do instead — a guardrail that only says "no" gets
/// bypassed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Why the commit may not happen, in words to show as they are; `None` lets it through.
    pub refusal: Option<String>,
    /// Said whether or not it is refused: a recorded escape, a superseded decision.
    pub notes: Vec<String>,
}

impl Verdict {
    pub fn allow() -> Verdict {
        Verdict::default()
    }

    pub fn refuse(why: impl Into<String>) -> Verdict {
        Verdict {
            refusal: Some(why.into()),
            notes: Vec::new(),
        }
    }

    pub fn is_allowed(&self) -> bool {
        self.refusal.is_none()
    }
}

/// The board's documents and decisions, as a commit message's `Docs:` and `ADR:` trailers are
/// checked against them. Asked for only when a trailer needs them: the front end may have to
/// fetch them from a server.
pub struct BoardRefs<'a> {
    /// Is this path a document on the board?
    pub doc_on_board: &'a dyn Fn(&str) -> bool,
    /// The board's decisions.
    pub adrs: &'a dyn Fn() -> Vec<crate::adr::Adr>,
}

/// What is wrong with a message's trailers, and what is worth saying about them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrailerReport {
    /// References that resolve to nothing on the board, as sentences a person can act on.
    pub problems: Vec<String>,
    /// Not refusals: a decision referenced after it was superseded.
    pub notes: Vec<String>,
}

/// The item a branch names, if it is an item on this board. This is the whole reason the branch
/// rule exists: with it, nothing else has to ask what is being worked on.
pub fn branch_item(project: &Project, branch: Option<&str>) -> Option<String> {
    let code = item_code_from_branch(project.config.branch_pattern(), branch?)?;
    project
        .features
        .iter()
        .any(|f| f.code == code)
        .then_some(code)
}

/// The git model the hooks judge by, when the project has it on (FEAT-189). Which branches take
/// merges only does not depend on the running sprint, so it is derived without reading one.
pub fn line_model(config: &ProjectConfig, default_branch: &str) -> Option<GitModel> {
    if !config.cadence.git_model {
        return None;
    }
    let facts = BranchFacts {
        main: default_branch.to_string(),
        ..Default::default()
    };
    Some(crate::gitmodel::derive(config, &[], &facts))
}

/// The Claude Code guard's rules (FEAT-056): the same as the git hooks', but answered *before* the
/// command runs, with the reference to use. Feedback that arrives as a failed command teaches
/// avoidance; feedback that arrives with the fix teaches the rule.
///
/// `facts.message` is the inline `-m` the front end could read, or `None` for a commit that opens
/// an editor — that one is left to the commit-msg hook, which reads the real message.
pub fn commit_verdict(facts: &RepoFacts, project: &Project) -> Verdict {
    // Setup's own first commit (DEF-067). The guard never sees the staged set, so it asks for an
    // inline message: a commit that opens an editor gets no exception here.
    // So is a refresh of CLAUDE.md's generated block (DEF-083), on the same terms.
    if facts.message.is_some() && (setup_commit(facts) || block_commit(facts)) {
        return Verdict::allow();
    }
    // A line of integration takes merges only (FEAT-189).
    if let Some(branch) = &facts.branch
        && let Some(model) = line_model(&project.config, &facts.default_branch)
        && !facts.merging
        && facts.has_commits
        && let LineVerdict::Refused(why) = line_commit_verdict(&model, branch, false, None)
    {
        return refuse_with_hint(facts, why);
    }
    // Completing a merge is the intended way work reaches the default branch (FEAT-094).
    // So is a repository's first commit (FEAT-105): nothing can branch from a repo with none.
    if let Some(branch) = &facts.branch
        && branch == &facts.default_branch
        && !facts.merging
        && facts.has_commits
    {
        return refuse_with_hint(
            facts,
            format!(
                "This would commit straight to {branch}. Work belongs on a branch for its item: \
                 run `kanbanr start <CODE>` first."
            ),
        );
    }
    let Some(message) = facts.message.as_deref() else {
        return Verdict::allow();
    };
    if is_generated_commit(message) {
        return Verdict::allow();
    }
    let refs = parse_refs(message);
    if refs.is_empty() {
        if escape_reason(message).is_some() {
            return Verdict::allow();
        }
        let suggestion = branch_item(project, facts.branch.as_deref())
            .map(|code| format!("Refs: kanbanr:{code}"))
            .unwrap_or_else(|| "Refs: kanbanr:<CODE>".to_string());
        return Verdict::refuse(format!(
            "This commit does not say which item it serves. Add the trailer `{suggestion}` (a \
             requirement is better: `kanbanr:<CODE>/R-2`), or run `kanbanr commit -m \"…\"`, \
             which fills it in from the branch."
        ));
    }
    let problems = validate_refs(project, &refs);
    if !problems.is_empty() {
        return Verdict::refuse(format!(
            "This commit references something that is not on the board:\n{}",
            problems.join("\n")
        ));
    }
    Verdict::allow()
}

/// The commit-msg hook's rules: the message git is about to record names the item it serves, or
/// says on the record why it serves none. `facts.message` is that message, git's comment lines
/// already removed; `board` is where `Docs:` and `ADR:` trailers resolve, `None` where there is
/// no board to ask.
pub fn message_verdict(facts: &RepoFacts, project: &Project, board: Option<&BoardRefs>) -> Verdict {
    let message = facts.message.as_deref().unwrap_or_default();
    // A merge into a line of integration says which items it brings, generated message or not
    // (FEAT-189): a line is where "which items reached it" is read from.
    // Setup's own first commit is not refused as a line commit (DEF-067): its `[no-ref]` is read
    // and noted below like any other escape. Nor is a refresh of CLAUDE.md's block (DEF-083).
    if !setup_commit(facts)
        && !block_commit(facts)
        && let Some(model) = line_model(&project.config, &facts.default_branch)
        && let Some(branch) = facts.branch.as_deref()
        && let LineVerdict::Refused(why) =
            line_commit_verdict(&model, branch, facts.merging, Some(message))
    {
        return Verdict::refuse(why);
    }
    if is_generated_commit(message) {
        return Verdict::allow(); // git wrote it; there is no author to ask for a reference
    }
    // References are read before the escape is considered: a message that carries both is a
    // referenced commit that happens to mention the escape, not an escape.
    if parse_refs(message).is_empty() {
        if let Some(reason) = escape_reason(message) {
            return Verdict {
                refusal: None,
                notes: vec![format!(
                    "committing with no reference — recorded reason: {reason}"
                )],
            };
        }
        let suggestion = branch_item(project, facts.branch.as_deref())
            .map(|code| format!("Refs: kanbanr:{code}"))
            .unwrap_or_else(|| {
                format!(
                    "Refs: kanbanr:{}-001",
                    project.config.codes.prefix_for(None)
                )
            });
        return Verdict::refuse(format!(
            "this commit says which code changed but not which item it serves.\n\
             Add a trailer:\n\n    {suggestion}\n\n\
             or `kanbanr commit -m \"…\"`, which fills it in from the branch.\n\
             If it genuinely serves no item, say so on the record: `[no-ref] <why>`."
        ));
    }
    let report = validate_trailers(project, message, board);
    Verdict {
        refusal: (!report.problems.is_empty()).then(|| {
            format!(
                "this commit references something that is not on the board:\n  {}",
                report.problems.join("\n  ")
            )
        }),
        notes: report.notes,
    }
}

/// Every trailer of a message resolved against the board: `Refs:` against its items,
/// requirements and tasks, and — where `board` is given — `Docs:` and `ADR:` against its documents
/// and decisions (FEAT-057). A stale document path or an invented decision id is a broken link,
/// exactly like a dangling dependency, and the kind that is hardest to notice later, because it
/// still reads as a reference.
pub fn validate_trailers(
    project: &Project,
    message: &str,
    board: Option<&BoardRefs>,
) -> TrailerReport {
    let mut report = TrailerReport {
        problems: validate_refs(project, &parse_refs(message)),
        notes: Vec::new(),
    };
    let Some(board) = board else {
        return report;
    };
    for path in parse_doc_refs(message) {
        if !(board.doc_on_board)(&path) {
            report
                .problems
                .push(format!("Docs: {path} is not a document on this board"));
        }
    }
    let adrs = parse_adr_refs(message);
    if !adrs.is_empty() {
        let listed = (board.adrs)();
        for id in adrs {
            match listed.iter().find(|a| a.id.eq_ignore_ascii_case(&id)) {
                None => report
                    .problems
                    .push(format!("ADR: {id} is not a decision on this board")),
                // Not a refusal: sometimes the commit IS the one replacing it. Saying so beats
                // blocking, because a rule that blocks legitimate work gets bypassed wholesale.
                Some(adr) if adr.is_superseded() => report.notes.push(format!(
                    "{id} was superseded by {} — make sure this is deliberate",
                    adr.superseded_by
                )),
                Some(_) => {}
            }
        }
    }
    report
}

/// The pre-commit hook's rules: work belongs on a branch that names the item it serves.
pub fn branch_verdict(facts: &RepoFacts, project: &Project) -> Verdict {
    // Detached head: a rebase or a bisect, not a place for a policy argument.
    let Some(branch) = facts.branch.as_deref() else {
        return Verdict::allow();
    };
    // A commit completing a merge is how an item's branch is SUPPOSED to arrive (FEAT-094), and
    // every branch rule below is about where new work is written — none of them applies to it.
    // The first fix exempted only the default-branch check, and the very next check then refused
    // the same commit because `master` names no item; running the recovery for real caught that.
    if facts.merging {
        return Verdict::allow();
    }
    if branch.starts_with(SPIKE_PREFIX) {
        return Verdict::allow(); // a spike is allowed to exist; `finish` is where it is refused
    }
    // The root commit has nowhere else to go: an item branch needs a commit to branch from, so a
    // new repository's first commit lands on its default branch (FEAT-105). Its message is still
    // checked by commit-msg, so it names an item or says `[no-ref] <why>` on the record.
    if !facts.has_commits && branch == facts.default_branch {
        return Verdict::allow();
    }
    // Setup's own first commit in a repository with history (DEF-067), judged by what is staged.
    // An unknown staged set gives no exception (ADR-0004): the old rule stands.
    // So is a refresh of CLAUDE.md's generated block (DEF-083), which stages that file alone.
    if facts.staged.is_some() && (setup_commit(facts) || block_commit(facts)) {
        return Verdict::allow();
    }
    // A line of integration takes merges only (FEAT-189).
    if let Some(model) = line_model(&project.config, &facts.default_branch) {
        match line_commit_verdict(&model, branch, false, None) {
            LineVerdict::Refused(why) => return refuse_with_hint(facts, why),
            LineVerdict::Allowed => return Verdict::allow(),
            LineVerdict::NotALine => {}
        }
    }
    if branch == facts.default_branch {
        return refuse_with_hint(
            facts,
            format!(
                "this is {branch}, the default branch. Work belongs on a branch for its item:\n\n    \
                 kanbanr start <CODE>\n\n\
                 (or `spike/<name>` to explore — spikes produce a definition change, not merged code.)"
            ),
        );
    }
    if branch_item(project, Some(branch)).is_none() {
        return Verdict::refuse(format!(
            "the branch '{branch}' does not name an item on the board, so nothing here can be \
             traced back to a reason. Use `kanbanr start <CODE>` (pattern: {}), or `spike/<name>`.",
            project.config.branch_pattern()
        ));
    }
    Verdict::allow()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FEAT-189: a defect's `fix/` branch names its item as its `feat/` twin does; a custom
    /// pattern has no twin.
    #[test]
    fn fix_twin_names_the_item() {
        assert_eq!(
            item_code_from_branch("", "fix/FEAT-012-cart-loses-items").as_deref(),
            Some("FEAT-012")
        );
        assert_eq!(
            item_code_from_branch("feat/{code}-{slug}", "feat/FEAT-012-cart").as_deref(),
            Some("FEAT-012")
        );
        assert_eq!(item_code_from_branch("work/{code}", "fix/FEAT-012-x"), None);
        assert_eq!(fix_twin("work/{code}"), None);
    }

    #[test]
    fn a_trailer_may_name_a_document_or_a_decision() {
        let message = "feat(x): do the thing\n\n\
                       Refs: kanbanr:FEAT-046/R-2\n\
                       Docs: design/mirror.md, design/flow.md\n\
                       \x20     design/third.md\n\
                       ADR: ADR-0003\n";
        assert_eq!(
            parse_doc_refs(message),
            vec!["design/mirror.md", "design/flow.md", "design/third.md"]
        );
        assert_eq!(parse_adr_refs(message), vec!["ADR-0003"]);

        // Prose that merely mentions a path or a decision is not a reference: these are trailers,
        // and a bare path matched anywhere would fire on every sentence naming a file.
        let prose = "docs: explain the layout\n\n\
                     See design/mirror.md and ADR-0003 for the reasoning.\n";
        assert!(parse_doc_refs(prose).is_empty());
        assert!(parse_adr_refs(prose).is_empty());
    }

    #[test]
    fn references_are_read_at_whatever_level_the_author_wrote() {
        let message = "feat(x): do the thing\n\n\
                       Refs: kanbanr:FEAT-046/TL-001/T3, kanbanr:FEAT-046/R-2,\n\
                       \x20     kanbanr:FEAT-050.\n\
                       Refs: kanbanr:FEAT-046/R-2\n";
        let refs = parse_refs(message);
        assert_eq!(
            refs,
            vec![
                ItemRef::Task {
                    code: "FEAT-046".into(),
                    todo: "TL-001".into(),
                    task: "T3".into()
                },
                ItemRef::Requirement {
                    code: "FEAT-046".into(),
                    requirement: "R-2".into()
                },
                ItemRef::Feature {
                    code: "FEAT-050".into()
                },
            ],
            "a wrapped list is still a list, and a repeat is not a second reference"
        );
        assert!(parse_refs("no references here").is_empty());
    }

    #[test]
    fn the_escape_has_to_say_why() {
        assert_eq!(
            escape_reason(
                "chore: rotate a leaked key\n\n[no-ref] incident response, item filed after"
            ),
            Some("incident response, item filed after")
        );
        // Present but silent: not an escape. An empty excuse is how a guardrail becomes a habit.
        assert_eq!(escape_reason("chore: whatever\n\n[no-ref]"), None);
        assert_eq!(escape_reason("chore: whatever"), None);
        // Talking about the escape is not taking it — the token has to open a line.
        assert_eq!(
            escape_reason(
                "docs: explain the guardrail\n\nA commit may use `[no-ref] <why>` to opt out."
            ),
            None
        );
    }

    #[test]
    fn commits_git_writes_itself_carry_no_reference() {
        assert!(is_generated_commit("Merge branch 'feat/FEAT-046'"));
        assert!(is_generated_commit("Revert \"feat: something\""));
        assert!(is_generated_commit("fixup! feat: something"));
        assert!(!is_generated_commit("feat: merge the two paths"));
    }

    #[test]
    fn branch_names_round_trip_to_the_item_they_belong_to() {
        assert_eq!(
            slug("Measurement: history, defects & report"),
            "measurement-history-defects-report"
        );
        let branch = branch_for(DEFAULT_BRANCH_PATTERN, "FEAT-053", "Measurement: history");
        assert_eq!(branch, "feat/FEAT-053-measurement-history");
        assert_eq!(
            code_from_branch(DEFAULT_BRANCH_PATTERN, &branch).as_deref(),
            Some("FEAT-053")
        );
        // A renamed slug still resolves: the code is the identity.
        assert_eq!(
            code_from_branch(DEFAULT_BRANCH_PATTERN, "feat/FEAT-053-anything-else").as_deref(),
            Some("FEAT-053")
        );
        // A project may spell it differently.
        assert_eq!(
            code_from_branch("work/{code}", "work/FEAT-007").as_deref(),
            Some("FEAT-007")
        );
        // Branches that belong to nothing are reported as such, not guessed at.
        assert_eq!(code_from_branch(DEFAULT_BRANCH_PATTERN, "master"), None);
        assert_eq!(
            code_from_branch(DEFAULT_BRANCH_PATTERN, "spike/try-git2"),
            None
        );
    }

    // ---- commit and branch policy (FEAT-176) ---------------------------------------------

    fn board(git_model: bool) -> Project {
        let mut config = ProjectConfig::default_for("p");
        config.cadence.git_model = git_model;
        let item: crate::FeatureItem = serde_json::from_value(serde_json::json!({
            "code": "FEAT-001", "title": "t", "status": "Planned", "milestone": "MS-001",
            "created_at": "2026-10-21T00:00:00Z", "updated_at": "2026-10-21T00:00:00Z"
        }))
        .unwrap();
        Project {
            id: "p".into(),
            config,
            features: vec![item],
            milestones: Vec::new(),
        }
    }

    fn on(branch: Option<&str>, message: Option<&str>) -> RepoFacts {
        RepoFacts {
            branch: branch.map(String::from),
            default_branch: "main".into(),
            merging: false,
            has_commits: true,
            message: message.map(String::from),
            setup_pending: false,
            staged: None,
            block_refresh: false,
        }
    }

    fn refusal(v: Verdict) -> String {
        v.refusal.unwrap_or_default()
    }

    /// The pre-commit rules: an item's branch, a spike, a merge being completed and a new repo's
    /// first commit pass; the default branch and a branch naming no item are refused.
    #[test]
    fn branch_verdict_wants_a_branch_for_an_item() {
        let p = board(false);
        let feat = Some("feat/FEAT-001-t");
        assert!(branch_verdict(&on(feat, None), &p).is_allowed());
        assert!(branch_verdict(&on(None, None), &p).is_allowed(), "detached");
        assert!(branch_verdict(&on(Some("spike/try"), None), &p).is_allowed());
        let mut merging = on(Some("main"), None);
        merging.merging = true;
        assert!(branch_verdict(&merging, &p).is_allowed(), "FEAT-094");
        let mut unborn = on(Some("main"), None);
        unborn.has_commits = false;
        assert!(branch_verdict(&unborn, &p).is_allowed(), "FEAT-105");
        assert!(refusal(branch_verdict(&on(Some("main"), None), &p)).contains("default branch"));
        assert!(
            refusal(branch_verdict(&on(Some("feat/FEAT-999-x"), None), &p))
                .contains("does not name an item")
        );
    }

    /// FEAT-189: with the git model on, a line of integration takes merges only.
    #[test]
    fn a_line_takes_merges_only() {
        let p = board(true);
        let sprint = Some("sprint/SP-001");
        assert!(refusal(branch_verdict(&on(sprint, None), &p)).contains("line of integration"));
        assert!(
            refusal(commit_verdict(&on(sprint, Some("x")), &p)).contains("line of integration")
        );
        let mut merge = on(sprint, Some("Merge feat/FEAT-001-t"));
        merge.merging = true;
        assert!(refusal(message_verdict(&merge, &p, None)).contains("must say which items"));
        merge.message = Some("Merge feat/FEAT-001-t\n\nRefs: kanbanr:FEAT-001".into());
        assert!(message_verdict(&merge, &p, None).is_allowed());
    }

    /// The guard's rules: the default branch is refused before the commit runs, and a message
    /// it can read must name an item that is on the board.
    #[test]
    fn commit_verdict_refuses_with_the_fix() {
        let p = board(false);
        let feat = Some("feat/FEAT-001-t");
        assert!(
            refusal(commit_verdict(&on(Some("main"), Some("x")), &p)).contains("straight to main")
        );
        let mut merging = on(Some("main"), None);
        merging.merging = true;
        assert!(commit_verdict(&merging, &p).is_allowed());
        assert!(
            commit_verdict(&on(feat, None), &p).is_allowed(),
            "no inline message"
        );
        assert!(
            refusal(commit_verdict(&on(feat, Some("wip")), &p)).contains("Refs: kanbanr:FEAT-001")
        );
        assert!(commit_verdict(&on(feat, Some("[no-ref] typo")), &p).is_allowed());
        assert!(commit_verdict(&on(feat, Some("x\n\nRefs: kanbanr:FEAT-001")), &p).is_allowed());
        assert!(
            refusal(commit_verdict(
                &on(feat, Some("x\n\nRefs: kanbanr:FEAT-404")),
                &p
            ))
            .contains("not on the board")
        );
    }

    /// FEAT-217 R-8: a kind-prefixed code round-trips through its `fix/` branch and its trailer,
    /// and the hooks pass it; with no item on the branch, the suggested trailer uses the prefix a
    /// new feature would get.
    #[test]
    fn kind_prefixed_codes_round_trip_branch_and_trailer() {
        let mut p = board(false);
        p.features[0].code = "DEF-001".into();
        p.config.codes.kinds.extend([
            ("defect".to_string(), "DEF".to_string()),
            ("feature".to_string(), "FTR".to_string()),
        ]);
        let pattern = p.config.branch_pattern().to_string();
        let fix = fix_twin(&pattern).unwrap();
        let branch = branch_for(&fix, "DEF-001", "Crash on save");
        assert_eq!(branch, "fix/DEF-001-crash-on-save");
        assert_eq!(
            item_code_from_branch(&pattern, &branch).as_deref(),
            Some("DEF-001")
        );
        assert_eq!(branch_item(&p, Some(&branch)).as_deref(), Some("DEF-001"));
        let on_fix = Some(branch.as_str());
        assert!(branch_verdict(&on(on_fix, None), &p).is_allowed());
        assert!(
            refusal(commit_verdict(&on(on_fix, Some("wip")), &p)).contains("Refs: kanbanr:DEF-001")
        );
        let message = "fix: save\n\nRefs: kanbanr:DEF-001";
        assert_eq!(parse_refs(message).len(), 1);
        assert!(validate_refs(&p, &parse_refs(message)).is_empty());
        assert!(commit_verdict(&on(on_fix, Some(message)), &p).is_allowed());
        // A branch that names no item: the suggestion is in this board's codes.
        let mut detached = on(Some("spike/x"), Some("wip"));
        detached.has_commits = true;
        let verdict = message_verdict(&detached, &p, None);
        assert!(refusal(verdict).contains("Refs: kanbanr:FTR-001"));
    }

    /// The commit-msg rules: an escape passes and is said, documents and decisions resolve against
    /// the board, and a superseded decision is mentioned rather than refused.
    #[test]
    fn message_verdict_checks_every_trailer() {
        let p = board(false);
        let feat = Some("feat/FEAT-001-t");
        let escaped = message_verdict(&on(feat, Some("[no-ref] typo")), &p, None);
        assert!(escaped.is_allowed());
        assert_eq!(
            escaped.notes,
            vec!["committing with no reference — recorded reason: typo"]
        );
        assert!(message_verdict(&on(feat, Some("Merge branch x")), &p, None).is_allowed());
        assert!(
            refusal(message_verdict(&on(feat, Some("wip")), &p, None))
                .contains("not which item it serves")
        );

        let docs = |path: &str| path == "design/a.md";
        let adrs = || {
            vec![crate::adr::Adr {
                id: "ADR-0001".into(),
                superseded_by: "ADR-0002".into(),
                ..Default::default()
            }]
        };
        let refs = BoardRefs {
            doc_on_board: &docs,
            adrs: &adrs,
        };
        let message = "x\n\nRefs: kanbanr:FEAT-001\nDocs: design/b.md\nADR: ADR-0001\n";
        let v = message_verdict(&on(feat, Some(message)), &p, Some(&refs));
        assert!(refusal(v.clone()).contains("Docs: design/b.md is not a document"));
        assert_eq!(v.notes.len(), 1, "{:?}", v.notes);
        assert!(v.notes[0].contains("superseded by ADR-0002"));
        // No board to ask: documents and decisions are not checked.
        assert!(message_verdict(&on(feat, Some(message)), &p, None).is_allowed());
        assert_eq!(
            validate_trailers(
                &p,
                "x\n\nRefs: kanbanr:FEAT-001\nDocs: design/a.md\n",
                Some(&refs)
            ),
            TrailerReport::default()
        );
    }

    // ---- setup's own first commit (DEF-067) ----------------------------------------------

    const HINT: &str = "Setup's own first commit is let through";

    /// Setup's facts: on main of a repository with history, the marker on disk and not in HEAD.
    fn setup(message: Option<&str>, staged: Option<&[&str]>) -> RepoFacts {
        let mut facts = on(Some("main"), message);
        facts.setup_pending = true;
        facts.staged = staged.map(|s| s.iter().map(|p| p.to_string()).collect());
        facts
    }

    // ---- a refresh of CLAUDE.md's generated block (DEF-083) ------------------------------

    const BLOCK_HINT: &str = "A refresh of CLAUDE.md's generated blocks";

    /// A block refresh's facts: on main of a repository with history, CLAUDE.md changed only
    /// inside the kanbanr markers.
    fn block(message: Option<&str>, staged: Option<&[&str]>) -> RepoFacts {
        let mut facts = on(Some("main"), message);
        facts.block_refresh = true;
        facts.staged = staged.map(|s| s.iter().map(|p| p.to_string()).collect());
        facts
    }

    /// DEF-083 R-1: the block refresh passes the guard, pre-commit and commit-msg, with the git
    /// model off and on.
    #[test]
    fn a_block_refresh_lands_on_the_default_branch() {
        let msg = "[no-ref] refresh the kanbanr block";
        for git_model in [false, true] {
            let p = board(git_model);
            assert!(
                commit_verdict(&block(Some(msg), None), &p).is_allowed(),
                "guard, git model {git_model}"
            );
            assert!(
                branch_verdict(&block(None, Some(&["CLAUDE.md"])), &p).is_allowed(),
                "pre-commit, git model {git_model}"
            );
            let noted = message_verdict(&block(Some(msg), None), &p, None);
            assert!(noted.is_allowed(), "commit-msg, git model {git_model}");
            assert_eq!(
                noted.notes,
                vec!["committing with no reference — recorded reason: refresh the kanbanr block"]
            );
        }
    }

    /// DEF-083 R-2: anything but exactly the block refresh is refused as before, with the hint
    /// saying what the refresh takes; with no refresh pending the refusals are today's.
    #[test]
    fn a_block_refresh_takes_only_claude_md() {
        let msg = "[no-ref] refresh the kanbanr block";
        for git_model in [false, true] {
            let p = board(git_model);
            // pre-commit: another path, or nothing staged.
            for staged in [&["CLAUDE.md", "src/x.txt"][..], &[][..]] {
                let facts = block(None, Some(staged));
                assert!(!block_commit(&facts), "{staged:?}");
                let why = refusal(branch_verdict(&facts, &p));
                assert!(
                    why.contains(BLOCK_HINT),
                    "{staged:?}, git model {git_model}: {why}"
                );
            }
            // guard and commit-msg: no escape, or a reference.
            for message in ["wip", "x\n\nRefs: kanbanr:FEAT-001"] {
                let facts = block(Some(message), None);
                assert!(!block_commit(&facts), "{message}");
                let why = refusal(commit_verdict(&facts, &p));
                assert!(
                    why.contains(BLOCK_HINT),
                    "{message}, git model {git_model}: {why}"
                );
                if git_model {
                    // commit-msg refuses it as a line commit, as before.
                    assert!(
                        refusal(message_verdict(&facts, &p, None)).contains("line of integration")
                    );
                } else if message == "wip" {
                    assert!(!message_verdict(&facts, &p, None).is_allowed());
                }
            }
            // A merge in progress is not a refresh.
            let mut merging = block(Some(msg), Some(&["CLAUDE.md"]));
            merging.merging = true;
            assert!(!block_commit(&merging));
            assert_eq!(block_hint(&merging), None);
            // On an item's branch nothing changes.
            let mut feat = block(Some("wip"), Some(&["CLAUDE.md"]));
            feat.branch = Some("feat/FEAT-001-t".into());
            assert!(!block_commit(&feat));
            assert_eq!(block_hint(&feat), None);
            assert!(branch_verdict(&feat, &p).is_allowed());
            assert!(refusal(commit_verdict(&feat, &p)).contains("Refs: kanbanr:FEAT-001"));
            // No refresh pending: today's refusals, word for word, with no hint.
            let mut closed = block(Some(msg), Some(&["CLAUDE.md"]));
            closed.block_refresh = false;
            assert_eq!(block_hint(&closed), None);
            assert_eq!(
                commit_verdict(&closed, &p),
                commit_verdict(&on(Some("main"), Some(msg)), &p)
            );
            assert_eq!(
                branch_verdict(&closed, &p),
                branch_verdict(&on(Some("main"), None), &p)
            );
            assert!(!refusal(commit_verdict(&closed, &p)).contains(BLOCK_HINT));
            assert!(!refusal(branch_verdict(&closed, &p)).contains(BLOCK_HINT));
            // Setup pending too: both hints, setup's first.
            let mut both = block(Some("wip"), None);
            both.setup_pending = true;
            let why = refusal(commit_verdict(&both, &p));
            let (s, b) = (why.find(HINT), why.find(BLOCK_HINT));
            assert!(s.is_some() && b.is_some() && s < b, "{why}");
        }
        // The exact wording, git model off.
        let p = board(false);
        assert_eq!(
            refusal(commit_verdict(&block(Some("wip"), None), &p)),
            "This would commit straight to main. Work belongs on a branch for its item: run \
             `kanbanr start <CODE>` first.\n\nA refresh of CLAUDE.md's generated blocks is let \
             through too: stage only CLAUDE.md, change nothing outside the \
             `<!-- <tool>:begin -->` … `<!-- <tool>:end -->` markers, and commit it with \
             `[no-ref] refresh the generated block`."
        );
    }

    /// DEF-067 R-1, R-3: setup's commit passes the guard, pre-commit and commit-msg, with the git
    /// model off and on; once the marker is in HEAD the same commit is refused as before.
    #[test]
    fn the_setup_commit_lands_on_the_default_branch_once() {
        let msg = "[no-ref] initial setup";
        let all: &[&str] = &[".kanbanr", "CLAUDE.md", ".gitignore"];
        for git_model in [false, true] {
            let p = board(git_model);
            assert!(
                commit_verdict(&setup(Some(msg), None), &p).is_allowed(),
                "guard, git model {git_model}"
            );
            assert!(
                branch_verdict(&setup(None, Some(all)), &p).is_allowed(),
                "pre-commit, git model {git_model}"
            );
            let noted = message_verdict(&setup(Some(msg), None), &p, None);
            assert!(noted.is_allowed(), "commit-msg, git model {git_model}");
            assert_eq!(
                noted.notes,
                vec!["committing with no reference — recorded reason: initial setup"]
            );
            assert!(
                branch_verdict(&setup(None, Some(&[".kanbanr"])), &p).is_allowed(),
                "the marker alone, git model {git_model}"
            );
            // R-3: the marker is in HEAD, so the window has closed.
            let mut done = setup(Some(msg), None);
            done.setup_pending = false;
            assert!(!commit_verdict(&done, &p).is_allowed());
            let mut done = setup(None, Some(all));
            done.setup_pending = false;
            assert!(!branch_verdict(&done, &p).is_allowed());
            if git_model {
                let mut done = setup(Some(msg), None);
                done.setup_pending = false;
                assert!(!message_verdict(&done, &p, None).is_allowed());
            }
        }
    }

    /// DEF-067 R-2: anything but exactly setup's commit is refused as before, with the hint saying
    /// what setup's commit takes; outside the window the refusals are today's, word for word.
    #[test]
    fn the_setup_window_takes_only_the_setup_files() {
        let msg = "[no-ref] initial setup";
        for git_model in [false, true] {
            let p = board(git_model);
            // pre-commit: another path, or no marker.
            for staged in [
                &[".kanbanr", "CLAUDE.md", "src/x.txt"][..],
                &["CLAUDE.md"][..],
                &["sub/.kanbanr"][..],
                &[][..],
            ] {
                let facts = setup(None, Some(staged));
                assert!(!setup_commit(&facts), "{staged:?}");
                let why = refusal(branch_verdict(&facts, &p));
                assert!(
                    why.contains(HINT),
                    "{staged:?}, git model {git_model}: {why}"
                );
            }
            // An unknown staged set gives no exception at pre-commit.
            assert!(refusal(branch_verdict(&setup(None, None), &p)).contains(HINT));
            // guard and commit-msg: no escape, or a reference.
            for message in [
                "wip",
                "x

Refs: kanbanr:FEAT-001",
            ] {
                let facts = setup(Some(message), None);
                assert!(!setup_commit(&facts), "{message}");
                let why = refusal(commit_verdict(&facts, &p));
                assert!(
                    why.contains(HINT),
                    "{message}, git model {git_model}: {why}"
                );
                if git_model {
                    // commit-msg refuses it as a line commit, as before.
                    assert!(
                        refusal(message_verdict(&facts, &p, None)).contains("line of integration")
                    );
                }
            }
            // A merge in progress is not setup.
            let mut merging = setup(Some(msg), Some(&[".kanbanr"]));
            merging.merging = true;
            assert!(!setup_commit(&merging));
            assert_eq!(setup_hint(&merging), None);
            // On an item's branch nothing changes.
            let mut feat = setup(Some("wip"), Some(&[".kanbanr"]));
            feat.branch = Some("feat/FEAT-001-t".into());
            assert!(!setup_commit(&feat));
            assert_eq!(setup_hint(&feat), None);
            assert!(branch_verdict(&feat, &p).is_allowed());
            assert!(refusal(commit_verdict(&feat, &p)).contains("Refs: kanbanr:FEAT-001"));
            // Outside the window: today's refusals, word for word, with no hint.
            let mut closed = setup(Some(msg), Some(&[".kanbanr"]));
            closed.setup_pending = false;
            let mut today = closed.clone();
            today.staged = None;
            assert_eq!(setup_hint(&closed), None);
            assert_eq!(
                commit_verdict(&closed, &p),
                commit_verdict(&on(Some("main"), Some(msg)), &p)
            );
            assert_eq!(
                branch_verdict(&closed, &p),
                branch_verdict(&on(Some("main"), None), &p)
            );
            assert!(!refusal(commit_verdict(&closed, &p)).contains(HINT));
            assert!(!refusal(branch_verdict(&today, &p)).contains(HINT));
        }
        // The exact wording, git model off.
        let p = board(false);
        assert_eq!(
            refusal(commit_verdict(&setup(Some("wip"), None), &p)),
            "This would commit straight to main. Work belongs on a branch for its item: run \
             `kanbanr start <CODE>` first.\n\nSetup's own first commit is let through: \
             stage only .kanbanr, CLAUDE.md and .gitignore, and commit them with \
             `[no-ref] initial setup`."
        );
    }

    /// DEF-085 R-1: with both hints on one refusal, setup's no longer claims to be the only
    /// exception before the block's says "too".
    #[test]
    fn a_refusal_with_both_hints_names_no_one_exception() {
        let mut facts = setup(Some("wip"), None);
        facts.block_refresh = true;
        let text = refusal(commit_verdict(&facts, &board(false)));
        let setup_at = text
            .find("Setup's own first commit is let through")
            .expect("setup's hint");
        let block_at = text.find("is let through too").expect("the block's hint");
        assert!(setup_at < block_at, "setup's hint comes first: {text}");
        assert!(!text.contains("one exception"), "{text}");
    }
}
