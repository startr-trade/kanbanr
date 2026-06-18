//! Export a feature item (or a whole project) in a Claude-ready format (markdown or JSON).

use crate::models::{FeatureItem, Milestone, TaskState};
use crate::store::Project;
use serde::Serialize;

/// Render an ENTIRE project as one portable markdown bundle: a board summary, the milestones, then
/// every feature item in full. Good for a snapshot/report or handing the whole plan to Claude.
pub fn project_to_markdown(project: &Project) -> String {
    let cfg = &project.config;
    let name = if cfg.name.is_empty() {
        &project.id
    } else {
        &cfg.name
    };
    let mut out = String::new();
    out.push_str(&format!("# {name} — project export\n\n"));
    if !cfg.description.is_empty() {
        out.push_str(&format!("{}\n\n", cfg.description));
    }

    out.push_str("## Board\n\n");
    for s in &cfg.statuses {
        let items: Vec<_> = project.features.iter().filter(|f| &f.status == s).collect();
        if items.is_empty() {
            continue;
        }
        out.push_str(&format!("### {} ({})\n\n", s, items.len()));
        for f in items {
            let kind = f
                .kind
                .as_deref()
                .map(|k| format!(" · _{k}_"))
                .unwrap_or_default();
            out.push_str(&format!(
                "- **{}** {} [{}/{}]{}\n",
                f.code,
                f.title,
                f.done_count(),
                f.task_count(),
                kind
            ));
        }
        out.push('\n');
    }

    if !project.milestones.is_empty() {
        out.push_str("## Milestones\n\n");
        for m in &project.milestones {
            let deps = if m.depends_on.is_empty() {
                String::new()
            } else {
                format!(" — depends on {}", m.depends_on.join(", "))
            };
            out.push_str(&format!("- **{}** {}{}\n", m.code, m.name, deps));
        }
        out.push('\n');
    }

    out.push_str("---\n\n# Feature items\n\n");
    let mut feats: Vec<_> = project.features.iter().collect();
    feats.sort_by(|a, b| a.code.cmp(&b.code));
    for f in feats {
        let ms = project.milestone(&f.milestone).ok();
        out.push_str(&to_markdown(f, ms));
        out.push_str("\n---\n\n");
    }
    out
}

fn state_label(s: TaskState) -> &'static str {
    match s {
        TaskState::NotStarted => "Not started",
        TaskState::InProgress => "In progress",
        TaskState::Completed => "Completed",
    }
}

fn checkbox(s: TaskState) -> &'static str {
    match s {
        TaskState::Completed => "[x]",
        TaskState::InProgress => "[~]",
        TaskState::NotStarted => "[ ]",
    }
}

/// Render a feature as a self-contained markdown brief Claude can act on.
pub fn to_markdown(feature: &FeatureItem, milestone: Option<&Milestone>) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} — {}\n\n", feature.code, feature.title));
    out.push_str(&format!("- **Status:** {}\n", feature.status));
    if let Some(m) = milestone {
        out.push_str(&format!("- **Milestone:** {} ({})\n", m.name, m.code));
    } else if !feature.milestone.is_empty() {
        out.push_str(&format!("- **Milestone:** {}\n", feature.milestone));
    }
    out.push_str(&format!(
        "- **Tasks:** {}/{} completed across {} todo-list(s)\n\n",
        feature.done_count(),
        feature.task_count(),
        feature.todo_lists.len()
    ));

    out.push_str("## Specification\n\n");
    if feature.specification.trim().is_empty() {
        out.push_str("_No specification provided._\n\n");
    } else {
        out.push_str(feature.specification.trim());
        out.push_str("\n\n");
    }

    out.push_str("## Todo-lists\n\n");
    if feature.todo_lists.is_empty() {
        out.push_str("_No todo-lists yet._\n");
    } else {
        // Newest-first.
        let mut lists: Vec<_> = feature.todo_lists.iter().collect();
        lists.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.code.cmp(&a.code)));
        for list in lists {
            let desc = if list.description.is_empty() {
                String::new()
            } else {
                format!(" — {}", list.description)
            };
            out.push_str(&format!(
                "### {}{} ({}/{})\n\n",
                list.code,
                desc,
                list.done_count(),
                list.tasks.len()
            ));
            if list.tasks.is_empty() {
                out.push_str("_No tasks._\n\n");
            } else {
                for t in &list.tasks {
                    out.push_str(&format!(
                        "- {} `{}` {} — _{}_\n",
                        checkbox(t.state),
                        t.key,
                        t.text,
                        state_label(t.state)
                    ));
                }
                out.push('\n');
            }
        }
    }
    out
}

#[derive(Serialize)]
struct FeatureExport<'a> {
    #[serde(flatten)]
    feature: &'a FeatureItem,
    #[serde(skip_serializing_if = "Option::is_none")]
    milestone_detail: Option<&'a Milestone>,
}

/// Render a feature as pretty JSON (includes resolved milestone when available).
pub fn to_json(feature: &FeatureItem, milestone: Option<&Milestone>) -> serde_json::Result<String> {
    let export = FeatureExport {
        feature,
        milestone_detail: milestone,
    };
    serde_json::to_string_pretty(&export)
}
