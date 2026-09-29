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

/// Render a project charter as markdown — the human-readable "why" of a project, used by
/// `kanbanr charter show`. (FEAT-046)
pub fn charter_to_markdown(charter: &crate::Charter) -> String {
    let mut out = String::from("# Charter\n\n");
    if charter.purpose.trim().is_empty() {
        out.push_str("_No purpose recorded._\n");
    } else {
        out.push_str(charter.purpose.trim());
        out.push('\n');
    }
    if !charter.vision.trim().is_empty() {
        out.push_str(&format!("\n**Vision:** {}\n", charter.vision.trim()));
    }

    out.push_str("\n## Goals\n\n");
    if charter.goals.is_empty() {
        out.push_str("_No goals recorded — work items have nothing to link to._\n");
    }
    for goal in &charter.goals {
        out.push_str(&format!("- **{}** {}", goal.id, goal.statement.trim()));
        if !goal.measure.trim().is_empty() {
            out.push_str(&format!("\n  - _Measure:_ {}", goal.measure.trim()));
        }
        if !goal.horizon.trim().is_empty() {
            out.push_str(&format!("\n  - _Horizon:_ {}", goal.horizon.trim()));
        }
        out.push('\n');
    }

    fn bullets(out: &mut String, title: &str, items: &[String]) {
        if items.is_empty() {
            return;
        }
        out.push_str(&format!("\n## {title}\n\n"));
        for item in items {
            out.push_str(&format!("- {}\n", item.trim()));
        }
    }
    bullets(&mut out, "Non-goals", &charter.non_goals);

    if !charter.stakeholders.is_empty() {
        out.push_str("\n## Stakeholders\n\n");
        for s in &charter.stakeholders {
            out.push_str(&format!("- **{}**", s.name.trim()));
            if !s.role.trim().is_empty() {
                out.push_str(&format!(" ({})", s.role.trim()));
            }
            if !s.interest.trim().is_empty() {
                out.push_str(&format!(" — {}", s.interest.trim()));
            }
            out.push('\n');
        }
    }
    bullets(&mut out, "Constraints", &charter.constraints);

    if !charter.adopted_at.trim().is_empty() {
        let day = charter.adopted_at.get(..10).unwrap_or(&charter.adopted_at);
        out.push_str(&format!("\n_Adopted {day}._\n"));
    }
    out
}

/// The decision brief for an item: what is proposed and why, on one screen, so agreement happens
/// BEFORE the work rather than after it (FEAT-048). Gaps are rendered, never stored.
pub fn definition_brief(feature: &FeatureItem) -> String {
    let Some(def) = feature.definition.as_ref() else {
        return format!(
            "# {} — {}\n\n_No definition yet. Write one with `kanbanr feature define {}`._\n",
            feature.code, feature.title, feature.code
        );
    };
    let mut out = format!("# {} — {}\n\n", feature.code, feature.title);
    if def.statement.trim().is_empty() {
        out.push_str("_[MISSING: statement]_\n");
    } else {
        out.push_str(&format!("> {}\n", def.statement.trim()));
    }
    out.push_str(&format!(
        "\n**Serves:** {}\n",
        if def.goals.is_empty() {
            "[MISSING: goal link]".to_string()
        } else {
            def.goals.join(", ")
        }
    ));

    out.push_str("\n| Dimension | Answer |\n|---|---|\n");
    for (column, answer) in def.zachman.columns() {
        let answer = if answer.trim().is_empty() {
            format!("[MISSING: {column}]")
        } else {
            answer.trim().replace('|', "\\|")
        };
        out.push_str(&format!("| {column} | {answer} |\n"));
    }

    out.push_str("\n## Requirements\n\n");
    if def.requirements.is_empty() {
        out.push_str("_None yet — an item with no requirement cannot be verified._\n");
    }
    for r in &def.requirements {
        let kind = match r.kind {
            crate::models::RequirementKind::Functional => "functional",
            crate::models::RequirementKind::Nfr => "nfr",
        };
        out.push_str(&format!("- **{}** ({kind}) {}\n", r.id, r.text.trim()));
        if !r.violates.trim().is_empty() {
            out.push_str(&format!("  - violates `{}`\n", r.violates.trim()));
        }
        if !r.iso.is_empty() {
            out.push_str(&format!("  - quality: {}\n", r.iso.join(", ")));
        }
        if let Some(s) = &r.scenario
            && !s.measure.trim().is_empty()
        {
            out.push_str(&format!("  - measure: {}\n", s.measure.trim()));
        }
        if r.tests.is_empty() {
            out.push_str("  - _[MISSING: test]_\n");
        } else {
            for t in &r.tests {
                let state = match t.state {
                    crate::models::TestState::Planned => "planned",
                    crate::models::TestState::Red => "red",
                    crate::models::TestState::Green => "green",
                };
                out.push_str(&format!("  - test `{}` ({state})\n", t.name));
            }
        }
    }

    let approval = match def.approval_state() {
        crate::models::ApprovalState::Ratified => def
            .approval
            .as_ref()
            .map(|a| format!("Ratified after the work, by {}", a.by))
            .unwrap_or_else(|| "ratified".into()),
        crate::models::ApprovalState::Current => def
            .approval
            .as_ref()
            .map(|a| {
                format!(
                    "approved by {} on {}",
                    a.by,
                    a.at.get(..10).unwrap_or(&a.at)
                )
            })
            .unwrap_or_default(),
        crate::models::ApprovalState::Lapsed => {
            "**approval lapsed** — the definition changed after it was approved".to_string()
        }
        crate::models::ApprovalState::Missing => "**not approved**".to_string(),
    };
    out.push_str(&format!("\n_Status: {approval}._\n"));
    if !def.started_unapproved.trim().is_empty() {
        out.push_str(&format!(
            "_Started without approval: {}._\n",
            def.started_unapproved.trim()
        ));
    }
    out
}

/// The definition as markdown: why the item exists, the six dimensions with their gaps named, and
/// each requirement with its evidence. This is what `kanbanr feature show` prints, which is the
/// whole of what Claude sees about an item — a field rendered nowhere may as well not exist.
fn definition_sections(def: &crate::models::FeatureDefinition) -> String {
    let mut out = String::from("## Definition\n\n");
    if def.statement.trim().is_empty() {
        out.push_str("_[MISSING: statement]_\n");
    } else {
        out.push_str(&format!("> {}\n", def.statement.trim()));
    }
    out.push_str(&format!(
        "\n- **Serves:** {}\n",
        if def.goals.is_empty() {
            "[MISSING: goal link]".to_string()
        } else {
            def.goals.join(", ")
        }
    ));
    if !def.design_doc.trim().is_empty() {
        out.push_str(&format!("- **Design:** `{}`\n", def.design_doc.trim()));
    }
    match def.approval_state() {
        // Said differently on purpose: agreement after the fact is a real resolution and not the
        // same claim as agreement before the work (FEAT-080).
        crate::models::ApprovalState::Ratified => {
            if let Some(a) = &def.approval {
                out.push_str(&format!(
                    "- **Ratified after the work** by {} on {} — it was built under a recorded \
                     bypass and agreed to afterwards\n",
                    a.by,
                    a.at.get(..10).unwrap_or(&a.at)
                ));
            }
        }
        crate::models::ApprovalState::Current => {
            if let Some(a) = &def.approval {
                out.push_str(&format!(
                    "- **Approved** by {} on {}\n",
                    a.by,
                    a.at.get(..10).unwrap_or(&a.at)
                ));
            }
        }
        crate::models::ApprovalState::Lapsed => {
            out.push_str("- **Approval lapsed** — the definition changed after it was approved\n")
        }
        crate::models::ApprovalState::Missing => out.push_str("- **Not approved**\n"),
    }
    if !def.started_unapproved.trim().is_empty() {
        out.push_str(&format!(
            "- **Started without approval:** {}\n",
            def.started_unapproved.trim()
        ));
    }
    if !def.exempt.trim().is_empty() {
        out.push_str(&format!(
            "- **Exempt from gap reporting:** {}\n",
            def.exempt.trim()
        ));
    }
    // Sign-offs, latest per name (FEAT-114): whether each still covers the definition as it stands.
    let mut named: Vec<&str> = Vec::new();
    for s in def.signoffs.iter().rev() {
        if named.contains(&s.id.as_str()) {
            continue;
        }
        named.push(&s.id);
        let current = def.signoff_current(&s.id).is_some();
        out.push_str(&format!(
            "- **Signed off `{}`** by {} on {}{}{}{}\n",
            s.id,
            s.by,
            s.at.get(..10).unwrap_or(&s.at),
            if s.status.is_empty() {
                String::new()
            } else {
                format!(" (at {})", s.status)
            },
            if s.note.is_empty() {
                String::new()
            } else {
                format!(" — {}", s.note)
            },
            if current {
                ""
            } else {
                " — **lapsed**: the definition changed since"
            },
        ));
    }

    out.push_str("\n| Dimension | Answer |\n|---|---|\n");
    for (column, answer) in def.zachman.columns() {
        let answer = if answer.trim().is_empty() {
            format!("_[MISSING: {column}]_")
        } else {
            answer.trim().replace('|', "\\|")
        };
        out.push_str(&format!("| {column} | {answer} |\n"));
    }

    out.push_str("\n## Requirements\n\n");
    if def.requirements.is_empty() {
        out.push_str("_None yet — nothing states what must be true for this to be done._\n");
    }
    for r in &def.requirements {
        let kind = match r.kind {
            crate::models::RequirementKind::Functional => "functional",
            crate::models::RequirementKind::Nfr => "nfr",
        };
        let pattern = crate::ears::classify(&r.text)
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| "not EARS".to_string());
        out.push_str(&format!(
            "### {} · {kind} · _{pattern}_\n\n{}\n",
            r.id,
            r.text.trim()
        ));
        if !r.violates.trim().is_empty() {
            out.push_str(&format!("\n- Violates `{}`\n", r.violates.trim()));
        }
        if !r.iso.is_empty() {
            out.push_str(&format!("\n- Quality: {}\n", r.iso.join(", ")));
        }
        if let Some(sc) = &r.scenario {
            out.push_str(&format!(
                "- Scenario: {} / {} / {} — **{}**\n",
                blank_as_gap(&sc.stimulus),
                blank_as_gap(&sc.environment),
                blank_as_gap(&sc.response),
                blank_as_gap(&sc.measure),
            ));
        }
        out.push('\n');
        if r.tests.is_empty() {
            out.push_str("- _[MISSING: test]_ — this requirement cannot be shown to be met\n");
        }
        for t in &r.tests {
            let mark = match t.state {
                crate::models::TestState::Planned => "[ ]",
                crate::models::TestState::Red => "[~]",
                crate::models::TestState::Green => "[x]",
            };
            let kind = if t.kind.trim().is_empty() {
                String::new()
            } else {
                format!(" ({})", t.kind.trim())
            };
            out.push_str(&format!("- {mark} `{}`{kind}\n", t.name));
        }
        out.push('\n');
    }
    out
}

fn blank_as_gap(s: &str) -> String {
    if s.trim().is_empty() {
        "_[MISSING]_".to_string()
    } else {
        s.trim().to_string()
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

    // The definition, when there is one. Rendered only if present, so every item written before
    // the method keeps producing byte-identical markdown.
    if let Some(def) = feature.definition.as_ref() {
        out.push_str(&definition_sections(def));
    }

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
