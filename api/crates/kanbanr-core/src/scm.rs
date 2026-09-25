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
//! hooks, the daemon and the tests all agree on what a reference means.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
