//! Whether the documents an item says it will write are built (DEF-033).
//!
//! `docs-folded` is a person's word that the documents are folded into the record. It is offered
//! only once there is something to confirm: every board doc the item names has changed since it
//! entered its stage, every repository doc its Design lists under `Affects docs` has changed on its
//! branch, and every task that writes docs is complete. An item that lists no documents is offered
//! the sign-off once its tasks are done, and the offer says it changes no documents.
//!
//! The pure parts (what an item lists, what a task writes) come first; reading the board's history
//! and the code repository comes last, and only when the item lists repository docs.

use crate::models::{FeatureItem, TaskState};
use crate::readiness::{Check, Context, Gap, Level};
use serde::Serialize;
use std::collections::BTreeSet;

/// The kind of document that is not built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Board,
    Repository,
    Task,
}

impl DocKind {
    pub fn label(self) -> &'static str {
        match self {
            DocKind::Board => "board doc",
            DocKind::Repository => "repository doc",
            DocKind::Task => "task",
        }
    }
}

/// One document still unbuilt, and why, as a sentence a person can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unbuilt {
    pub kind: DocKind,
    pub path: String,
    pub why: String,
}

/// The answer for one item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocsBuilt {
    /// Everything the item says it will write: board docs, repository docs, docs tasks (as
    /// "T3: ...").
    pub listed: Vec<String>,
    pub unbuilt: Vec<Unbuilt>,
    /// Nothing listed and no docs task: the sign-off then waits only on the tasks.
    pub changes_no_documents: bool,
}

impl DocsBuilt {
    pub fn is_built(&self) -> bool {
        self.unbuilt.is_empty()
    }

    /// One gap per unbuilt document.
    pub fn gaps(&self) -> Vec<Gap> {
        self.unbuilt
            .iter()
            .map(|u| Gap {
                check: Check::DocsUpdated,
                requirement: None,
                level: Level::Warning,
                label: "docs not built".to_string(),
                message: u.why.clone(),
            })
            .collect()
    }

    /// The unbuilt documents as one sentence, for a refusal.
    pub fn sentence(&self) -> String {
        self.unbuilt
            .iter()
            .map(|u| u.why.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// The section of `spec` under the heading `Affects docs` (any level, 2 to 4 `#`), to the next
/// heading of the same or a higher level. Headings inside fenced code are not headings.
fn affects_section(spec: &str) -> String {
    let mut fenced = false;
    let mut level_in: Option<usize> = None;
    let mut out = String::new();
    for line in spec.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
        }
        let level = t.chars().take_while(|c| *c == '#').count();
        if !fenced && (1..=6).contains(&level) && t[level..].starts_with(' ') {
            if let Some(open) = level_in {
                if level <= open {
                    level_in = None;
                } else {
                    out.push_str(line);
                    out.push('\n');
                    continue;
                }
            }
            let text = t[level..].trim().trim_end_matches('#').trim();
            if (2..=4).contains(&level)
                && text.trim_end_matches(':').trim().eq_ignore_ascii_case("affects docs")
            {
                level_in = Some(level);
            }
            continue;
        }
        if level_in.is_some() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The repository paths named under the spec's `Affects docs` heading: every inline code span and
/// every link destination that looks like a repository file. Deliberately not prose parsing: a
/// path written without backticks or a link is not read.
pub fn repository_docs(spec: &str, board_docs: &BTreeSet<String>) -> Vec<String> {
    let section = affects_section(spec);
    let mut found: Vec<String> = Vec::new();
    // Inline code spans.
    let parts: Vec<&str> = section.split('`').collect();
    for (i, span) in parts.iter().enumerate() {
        if i % 2 == 1 && i + 1 < parts.len() {
            found.push(span.trim().to_string());
        }
    }
    // Link destinations: `](x)`.
    let mut rest = section.as_str();
    while let Some(at) = rest.find("](") {
        rest = &rest[at + 2..];
        if let Some(end) = rest.find(')') {
            found.push(rest[..end].trim().to_string());
            rest = &rest[end..];
        }
    }
    let mut keep: Vec<String> = found
        .into_iter()
        .filter(|x| {
            !x.is_empty()
                && !x.contains(char::is_whitespace)
                && !x.starts_with('/')
                && !x.starts_with("http")
                && !x.contains('*')
                && !x.contains('<')
                && (x.starts_with("docs/")
                    || [".md", ".json", ".yaml", ".txt"]
                        .iter()
                        .any(|e| x.ends_with(e)))
                && !board_docs.contains(&crate::refs::doc_key(x))
        })
        .collect();
    keep.sort();
    keep.dedup();
    keep
}

fn path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')
}

/// Whether `text` holds `word` as a whole word, not inside a longer name.
fn names_file(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        let rest = &text[at + word.len()..];
        // A full stop that ends a sentence is not part of the name.
        let continues = after.is_some_and(|c| {
            path_char(c) && !(c == '.' && !rest[1..].starts_with(|n: char| n.is_alphanumeric()))
        });
        !before.is_some_and(|c| path_char(c) && c != '/') && !continues
    })
}

/// Whether a task writes docs: its text names a listed document, a path under `docs/`, or the file
/// `CHANGELOG.md`, `SECURITY.md` or `README.md`.
pub fn is_docs_task(text: &str, listed: &[String]) -> bool {
    if listed.iter().any(|l| !l.is_empty() && text.contains(l.as_str())) {
        return true;
    }
    if text
        .match_indices("docs/")
        .any(|(at, _)| text[at + 5..].chars().next().is_some_and(path_char))
    {
        return true;
    }
    ["CHANGELOG.md", "SECURITY.md", "README.md"]
        .iter()
        .any(|f| names_file(text, f))
}

/// The whole answer for one item.
pub fn evaluate(
    project: &crate::store::Project,
    feature: &FeatureItem,
    ctx: &Context,
) -> DocsBuilt {
    let mut built = DocsBuilt::default();
    let declared: Vec<String> = feature
        .definition
        .as_ref()
        .map(|d| d.affects_docs.clone())
        .unwrap_or_default();
    let board_keys: BTreeSet<String> = declared
        .iter()
        .map(|d| crate::readiness::affects_doc_key(d))
        .collect();

    // Board docs: the existing gate's answer, reused.
    built.listed.extend(board_keys.iter().cloned());
    if let Some(docs) = ctx.docs_dir.as_deref() {
        for gap in crate::readiness::docs_updated_gaps(feature, docs, &ctx.history) {
            built.unbuilt.push(Unbuilt {
                kind: DocKind::Board,
                path: gap.label.clone(),
                why: gap.message,
            });
        }
    }

    // Repository docs.
    let repo_docs = repository_docs(&feature.specification, &board_keys);
    built.listed.extend(repo_docs.iter().cloned());
    if !repo_docs.is_empty() {
        built
            .unbuilt
            .extend(repository_unbuilt(project, feature, ctx, &repo_docs));
    }

    // Tasks that write docs.
    let listed_paths: Vec<String> = declared.iter().cloned().chain(built.listed.clone()).collect();
    for task in feature.all_tasks() {
        if is_docs_task(&task.text, &listed_paths) {
            built.listed.push(format!("{}: {}", task.key, task.text));
            if task.state != TaskState::Completed {
                built.unbuilt.push(Unbuilt {
                    kind: DocKind::Task,
                    path: task.key.clone(),
                    why: format!("task {} writes docs and is not complete", task.key),
                });
            }
        }
    }

    // Nothing listed: the sign-off confirms finished work and nothing else.
    built.changes_no_documents = built.listed.is_empty();
    if built.changes_no_documents && !feature.all_tasks_completed() {
        let why = if feature.all_tasks().next().is_none() {
            "it has no tasks, so nothing shows any work was done"
        } else {
            "its tasks are not all done"
        };
        built.unbuilt.push(Unbuilt {
            kind: DocKind::Task,
            path: String::new(),
            why: why.to_string(),
        });
    }
    built
}

/// The repository docs `paths` that have not changed on the item's branch against its line, or one
/// entry saying why they cannot be judged.
fn repository_unbuilt(
    project: &crate::store::Project,
    feature: &FeatureItem,
    ctx: &Context,
    paths: &[String],
) -> Vec<Unbuilt> {
    let held = |why: String| {
        vec![Unbuilt {
            kind: DocKind::Repository,
            path: String::new(),
            why,
        }]
    };
    let Some(root) = ctx.code_repo.as_deref() else {
        return held(
            "the code repository is not available here, so the repository docs cannot be \
             checked: start the monitor from the project folder"
                .to_string(),
        );
    };
    let Some(facts) = ctx.branch_facts() else {
        return held(
            "the code repository could not be read, so the repository docs cannot be checked"
                .to_string(),
        );
    };
    let config = &project.config;
    let branches = crate::gitmodel::item_branches(config, facts, &feature.code);
    if branches.is_empty() {
        return held(format!(
            "no branch for {} in the code repository, so the repository docs cannot be checked",
            feature.code
        ));
    }
    let line = if config.cadence.git_model {
        let model = crate::gitmodel::derive(config, &ctx.sprints, facts);
        crate::gitmodel::line_for(&model, feature, &ctx.releases).unwrap_or(model.main)
    } else {
        facts.main.clone()
    };
    let mut changed: BTreeSet<String> = BTreeSet::new();
    for branch in &branches {
        match crate::coderepo::changed_files(root, &line, branch) {
            Ok(files) => changed.extend(files),
            Err(_) => {
                return held(format!(
                    "{branch} cannot be compared with {line}, so the repository docs cannot be \
                     checked"
                ));
            }
        }
    }
    let shown = branches.join(", ");
    paths
        .iter()
        .filter(|p| !changed.contains(p.as_str()))
        .map(|p| Unbuilt {
            kind: DocKind::Repository,
            path: p.clone(),
            why: format!("{p} has not changed on {shown} against {line}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = "## Design\n\nChanges `api/src/x.rs` and `docs/src/not-here.md`.\n\n### Affects docs\nBoard docs: [design/data-flow.md](/p/k/docs/file/design/data-flow.md), `design/data-flow.md`.\nRepository docs: `docs/src/using/living-docs.md`, `SECURITY.md`, `CHANGELOG.md`, `docs/src/a/*.md`.\nRelated: [FEAT-1](/p/k/feature/FEAT-1), [guide](docs/src/guides/g.md), `some words.md`, `https://x/y.md`.\n\n### After\nSee `docs/src/after.md`.\n";

    #[test]
    fn repository_docs_are_read_from_the_affects_docs_section_only() {
        let board: BTreeSet<String> = ["design/data-flow.md".to_string()].into();
        let got = repository_docs(SPEC, &board);
        assert_eq!(
            got,
            vec![
                "CHANGELOG.md",
                "SECURITY.md",
                "docs/src/guides/g.md",
                "docs/src/using/living-docs.md"
            ]
        );
        // No heading, no documents.
        assert!(repository_docs("## Design\n`docs/src/x.md`\n", &board).is_empty());
    }

    #[test]
    fn a_task_writes_docs_when_it_names_a_listed_document() {
        let listed = vec!["skill/kanbanr/SKILL.md".to_string()];
        assert!(is_docs_task("Update skill/kanbanr/SKILL.md for the rule", &listed));
        assert!(is_docs_task("Write docs/src/using/x.md", &[]));
        assert!(is_docs_task("Changelog entry in CHANGELOG.md", &[]));
        assert!(is_docs_task("Mention it in README.md.", &[]));
        assert!(!is_docs_task("Refactor the parser", &listed));
        assert!(!is_docs_task("Touch docs/ folder", &[]));
        assert!(!is_docs_task("Edit MYREADME.md.bak", &[]));
    }

    /// A board with no history of its own and a code repository, as `(store, board dir, repo)`.
    fn fixture(tag: &str) -> (crate::Store, std::path::PathBuf, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-docsbuilt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let repo = crate::coderepo::tests::scratch_repo(&format!("docsbuilt-{tag}"));
        (store, dir, repo)
    }

    fn item(store: &crate::Store, title: &str, spec: &str, docs: &[&str]) -> String {
        let f = store.add_feature("demo", title, spec, "M", None).unwrap();
        store
            .set_feature_definition(
                "demo",
                &f.code,
                Some(crate::models::FeatureDefinition {
                    statement: title.into(),
                    affects_docs: docs.iter().map(|d| d.to_string()).collect(),
                    ..Default::default()
                }),
            )
            .unwrap();
        f.code
    }

    fn task(store: &crate::Store, code: &str, text: &str, state: TaskState) {
        let todo = store.add_todo_list("demo", code, "t", None).unwrap();
        let t = store
            .add_task("demo", code, &todo.code, text, None)
            .unwrap();
        store
            .set_task_state("demo", code, &todo.code, &t.key, state)
            .unwrap();
    }

    fn judge(
        store: &crate::Store,
        code: &str,
        repo: Option<&std::path::Path>,
    ) -> DocsBuilt {
        let project = store.load("demo").unwrap();
        let ctx = Context::load_in(store, "demo", &project.config, repo);
        evaluate(&project, project.feature(code).unwrap(), &ctx)
    }

    fn branch_for(store: &crate::Store, code: &str) -> String {
        let project = store.load("demo").unwrap();
        let f = project.feature(code).unwrap();
        crate::scm::branch_for(project.config.branch_pattern(), code, &f.title)
    }

    /// DEF-033 R-2: each rule holds the sign-off by itself, and is reported as its own kind.
    #[test]
    fn board_docs_repository_docs_and_docs_tasks_are_each_needed() {
        let (store, dir, repo) = fixture("each");
        // A board doc the board does not hold: only the board rule fails.
        let a = item(&store, "A", "", &["nope"]);
        task(&store, &a, "Build it", TaskState::Completed);
        let got = judge(&store, &a, Some(&repo));
        assert_eq!(got.unbuilt.len(), 1, "{got:?}");
        assert_eq!(got.unbuilt[0].kind, DocKind::Board);
        assert!(!got.changes_no_documents);

        // A repository doc its branch has not changed: only the repository rule fails.
        let b = item(
            &store,
            "B",
            "## Design\n\n### Affects docs\nRepository docs: `docs/src/b.md`.\n",
            &[],
        );
        task(&store, &b, "Build it", TaskState::Completed);
        let branch = branch_for(&store, &b);
        crate::coderepo::git(&repo, &["checkout", "-q", "-b", &branch]).unwrap();
        crate::coderepo::tests::commit_file(&repo, "code.rs", "x\n", "code");
        let got = judge(&store, &b, Some(&repo));
        assert_eq!(got.unbuilt.len(), 1, "{got:?}");
        assert_eq!(got.unbuilt[0].kind, DocKind::Repository);
        assert!(got.unbuilt[0].why.contains("docs/src/b.md has not changed"));
        // Once the branch changes it, the item is built.
        std::fs::create_dir_all(repo.join("docs/src")).unwrap();
        crate::coderepo::tests::commit_file(&repo, "docs/src/b.md", "b\n", "docs");
        let got = judge(&store, &b, Some(&repo));
        assert!(got.is_built(), "{got:?}");
        crate::coderepo::git(&repo, &["checkout", "-q", "main"]).unwrap();

        // A task that writes docs and is not complete: only the task rule fails.
        let c = item(&store, "C", "", &[]);
        task(&store, &c, "Write docs/src/c.md", TaskState::InProgress);
        let got = judge(&store, &c, Some(&repo));
        assert_eq!(got.unbuilt.len(), 1, "{got:?}");
        assert_eq!(got.unbuilt[0].kind, DocKind::Task);
        assert!(got.unbuilt[0].why.contains("writes docs and is not complete"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// DEF-033 R-3: no documents at all means the tasks decide, and the answer says so.
    #[test]
    fn an_item_with_no_documents_is_offered_the_signoff_once_its_tasks_are_done() {
        let (store, dir, _repo) = fixture("none");
        let code = item(&store, "Plain", "", &[]);
        let got = judge(&store, &code, None);
        assert!(!got.is_built());
        assert!(got.unbuilt[0].why.contains("no tasks"), "{got:?}");

        task(&store, &code, "Build it", TaskState::InProgress);
        let got = judge(&store, &code, None);
        assert!(got.unbuilt[0].why.contains("not all done"), "{got:?}");

        let todo = store.add_todo_list("demo", &code, "u", None).unwrap();
        let t = store
            .add_task("demo", &code, &todo.code, "Also", None)
            .unwrap();
        store
            .set_task_state("demo", &code, &todo.code, &t.key, TaskState::Completed)
            .unwrap();
        let project = store.load_for_gates("demo").unwrap();
        let first = project.feature(&code).unwrap().todo_lists[0].clone();
        store
            .set_task_state("demo", &code, &first.code, &first.tasks[0].key, TaskState::Completed)
            .unwrap();
        let got = judge(&store, &code, None);
        assert!(got.is_built(), "{got:?}");
        assert!(got.changes_no_documents);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEF-033 R-5: repository docs that cannot be read hold the sign-off, and say why.
    #[test]
    fn unreadable_repository_docs_hold_the_signoff_and_say_why() {
        let (store, dir, repo) = fixture("unreadable");
        let code = item(
            &store,
            "R",
            "### Affects docs\n`docs/src/r.md`\n",
            &[],
        );
        task(&store, &code, "Build it", TaskState::Completed);
        let got = judge(&store, &code, None);
        assert!(!got.is_built());
        assert!(got.unbuilt[0].why.contains("not available here"), "{got:?}");
        // A repository, but no branch for the item.
        let got = judge(&store, &code, Some(&repo));
        assert!(!got.is_built());
        assert!(
            got.unbuilt[0].why.contains(&format!("no branch for {code}")),
            "{got:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }
}
