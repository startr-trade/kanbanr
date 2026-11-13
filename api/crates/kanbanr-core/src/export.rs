//! Export a feature item (or a whole project) in a Claude-ready format (markdown or JSON).

use crate::models::{FeatureItem, Milestone, TaskState};
use crate::store::Project;
use serde::Serialize;

/// "N part(s) added since: keys" for an approval that has only grown (FTR-018).
fn added_since(def: &crate::models::FeatureDefinition) -> String {
    let added = def.part_changes().map(|c| c.added).unwrap_or_default();
    format!("{} part(s) added since: {}", added.len(), added.join(", "))
}

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
        out.push_str(&to_markdown(f, ms, None));
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
        crate::models::ApprovalState::Extended => {
            let approver = def.approval.as_ref();
            format!(
                "approved by {} on {}, with {}",
                approver.map_or("", |a| a.by.as_str()),
                approver.map_or("", |a| a.at.get(..10).unwrap_or(&a.at)),
                added_since(def)
            )
        }
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
pub(crate) fn definition_sections(
    feature: &FeatureItem,
    def: &crate::models::FeatureDefinition,
) -> String {
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
    if !def.affects_docs.is_empty() {
        let docs: Vec<String> = def.affects_docs.iter().map(|d| format!("`{d}`")).collect();
        out.push_str(&format!("- **Affects docs:** {}\n", docs.join(", ")));
    }
    match def.approval_state() {
        // Said differently on purpose: agreement after the fact is a real resolution and not the
        // same claim as agreement before the work (FEAT-080).
        crate::models::ApprovalState::Ratified => {
            if let Some(a) = &def.approval {
                out.push_str(&format!(
                    "- **Ratified after the work** by {} on {}{} — it was built under a recorded \
                     bypass and agreed to afterwards\n",
                    a.by,
                    a.at.get(..10).unwrap_or(&a.at),
                    crate::origin::via(&a.origin)
                ));
            }
        }
        crate::models::ApprovalState::Current => {
            if let Some(a) = &def.approval {
                out.push_str(&format!(
                    "- **Approved** by {} on {}{}\n",
                    a.by,
                    a.at.get(..10).unwrap_or(&a.at),
                    crate::origin::via(&a.origin)
                ));
            }
        }
        crate::models::ApprovalState::Extended => {
            if let Some(a) = &def.approval {
                out.push_str(&format!(
                    "- **Approved** by {} on {}{}, with {}\n",
                    a.by,
                    a.at.get(..10).unwrap_or(&a.at),
                    crate::origin::via(&a.origin),
                    added_since(def)
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
        // The latest entry is a withdrawal (FEAT-182): say who took it back and why.
        if !s.verdict.is_given() {
            out.push_str(&format!(
                "- **Sign-off `{}` withdrawn** by {} on {}{} — {}\n",
                s.id,
                s.by,
                s.at.get(..10).unwrap_or(&s.at),
                crate::origin::via(&s.origin),
                s.reason
            ));
            continue;
        }
        let current = def.signoff_current(&s.id).is_some();
        // The definition still matches but the spec the sign-off pinned has moved (FTR-023).
        let spec_moved = feature.signoff_spec_changed(&s.id).is_some();
        out.push_str(&format!(
            "- **Signed off `{}`** by {} on {}{}{}{}{}\n",
            s.id,
            s.by,
            s.at.get(..10).unwrap_or(&s.at),
            crate::origin::via(&s.origin),
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
            if spec_moved {
                " — **changed since sign-off**: the spec changed"
            } else if current {
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
            // A test checked later says when (DEF-043). The stage a `pushed` or `cut` test waits
            // for is the project's, which this rendering doesn't read; the plan names it.
            let when = match t.at.trim() {
                "" => String::new(),
                "pushed" => " — checked after main is pushed".to_string(),
                "cut" => " — checked at the cut".to_string(),
                stage => format!(" — checked at {stage}"),
            };
            out.push_str(&format!("- {mark} `{}`{kind}{when}\n", t.name));
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
///
/// `release` is the item's release line (`releases::release_line`), printed after the milestone.
pub fn to_markdown(
    feature: &FeatureItem,
    milestone: Option<&Milestone>,
    release: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} — {}\n\n", feature.code, feature.title));
    out.push_str(&format!("- **Status:** {}\n", feature.status));
    if let Some(m) = milestone {
        out.push_str(&format!("- **Milestone:** {} ({})\n", m.name, m.code));
    } else if !feature.milestone.is_empty() {
        out.push_str(&format!("- **Milestone:** {}\n", feature.milestone));
    }
    if let Some(line) = release {
        out.push_str(&format!("- **Release:** {line}\n"));
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
        out.push_str(&definition_sections(feature, def));
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

/// An item as `GET /projects/{p}/features/{code}/export?format=json` gives it: the item, and its
/// milestone resolved.
#[derive(Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FeatureExport<'a> {
    #[serde(flatten)]
    pub feature: &'a FeatureItem,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone_detail: Option<&'a Milestone>,
    /// Which release shipped it, or will (FTR-001). Absent without releases.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_line: Option<&'a str>,
}

/// Render a feature as pretty JSON (includes resolved milestone when available).
pub fn to_json(
    feature: &FeatureItem,
    milestone: Option<&Milestone>,
    release: Option<&str>,
) -> serde_json::Result<String> {
    let export = FeatureExport {
        feature,
        milestone_detail: milestone,
        release_line: release,
    };
    serde_json::to_string_pretty(&export)
}

#[cfg(test)]
mod tests {
    use crate::releases::Release;
    use crate::store::Project;
    use crate::{FeatureItem, ProjectConfig};

    fn item(code: &str, release: Option<&str>) -> FeatureItem {
        let mut f: FeatureItem = crate::yaml::from_str(&format!(
            "code: {code}\ntitle: T\nspecification: ''\nstatus: Completed\nmilestone: ''\n\
             created_at: '2026-10-19T00:00:00Z'\nupdated_at: '2026-10-19T00:00:00Z'\n"
        ))
        .unwrap();
        f.release = release.map(String::from);
        f
    }

    fn project(releases_on: bool) -> Project {
        let mut config = ProjectConfig::default_for("shop");
        config.cadence.releases = releases_on;
        Project {
            id: "shop".into(),
            config,
            features: vec![],
            milestones: vec![],
        }
    }

    fn releases() -> Vec<Release> {
        vec![
            Release {
                version: "v1.0.0".into(),
                shipped: vec!["FEAT-001".into()],
                ..Default::default()
            },
            Release {
                version: "v1.0.1".into(),
                parent: "v1.0.0".into(),
                shipped: vec!["FEAT-001".into()],
                ..Default::default()
            },
            Release {
                version: "v1.1.0".into(),
                ..Default::default()
            },
        ]
    }

    /// FTR-001 R-1: shipped, planned, and in no release each read differently.
    #[test]
    fn item_says_its_release() {
        let p = project(true);
        let rs = releases();
        let line = |f: &FeatureItem| crate::releases::release_line(&p, &rs, f);
        assert_eq!(
            line(&item("FEAT-001", Some("v1.0.0"))).as_deref(),
            Some("Released in v1.0.0")
        );
        assert_eq!(
            line(&item("FEAT-002", Some("v1.1.0"))).as_deref(),
            Some("Planned for v1.1.0, not released yet")
        );
        assert_eq!(
            line(&item("FEAT-003", None)).as_deref(),
            Some("Not in a release")
        );
        let md = super::to_markdown(&item("FEAT-002", None), None, Some("Not in a release"));
        assert!(md.contains("- **Release:** Not in a release\n"), "{md}");
        let json = super::to_json(&item("FEAT-002", None), None, Some("Not in a release")).unwrap();
        assert!(
            json.contains("\"release_line\": \"Not in a release\""),
            "{json}"
        );
    }

    /// FTR-001 R-2: a project without releases shows no release line.
    #[test]
    fn feature_show_marks_a_design_changed_since_signoff() {
        let mut f = item("FTR-001", None);
        f.specification = "## Design\nUse a map.".into();
        let mut def = crate::models::FeatureDefinition {
            statement: "Keep a cart".into(),
            ..Default::default()
        };
        def.signoffs.push(crate::models::Signoff {
            id: "design-review".into(),
            by: "Ada".into(),
            at: "2026-11-06T10:00:00Z".into(),
            rev: def.content_rev(),
            spec_rev: crate::readiness::spec_pin(&f.specification).1,
            ..Default::default()
        });
        f.definition = Some(def);
        let md = super::to_markdown(&f, None, None);
        assert!(md.contains("Signed off `design-review`"), "{md}");
        assert!(!md.contains("changed since sign-off"), "{md}");
        f.specification = "## Design\nUse a vec.".into();
        let md = super::to_markdown(&f, None, None);
        assert!(
            md.contains("**changed since sign-off**: the spec changed"),
            "{md}"
        );
    }

    #[test]
    fn no_release_line_without_releases() {
        let p = project(false);
        let f = item("FEAT-001", Some("v1.0.0"));
        assert_eq!(crate::releases::release_line(&p, &releases(), &f), None);
        let md = super::to_markdown(&f, None, None);
        assert!(!md.contains("**Release:**"), "{md}");
        let json = super::to_json(&f, None, None).unwrap();
        assert!(!json.contains("release_line"), "{json}");
    }

    /// DEF-028 R-5: `feature show` ends each verdict line with where it was taken, and says
    /// "origin unknown" for a record that has none.
    #[test]
    fn feature_show_names_the_origin_of_each_verdict() {
        use crate::models::{Approval, FeatureDefinition, Signoff, SignoffVerdict};
        let signed = |name: &str, origin: &str| Signoff {
            id: name.into(),
            by: "Ada".into(),
            at: "2026-11-06T10:00:00Z".into(),
            origin: origin.into(),
            ..Default::default()
        };
        let show = |def: FeatureDefinition| {
            let mut f = item("FEAT-001", None);
            f.definition = Some(def);
            super::to_markdown(&f, None, None)
        };
        let mut def = FeatureDefinition {
            statement: "s".into(),
            ..Default::default()
        };
        let rev = def.content_rev();
        def.approval = Some(Approval {
            by: "Ada".into(),
            at: "2026-11-06T10:00:00Z".into(),
            rev: rev.clone(),
            origin: "monitor".into(),
            ..Default::default()
        });
        for (name, origin) in [
            ("a", "monitor"),
            ("b", "conversation"),
            ("c", "terminal"),
            ("d", "delegated"),
            ("e", ""),
        ] {
            let mut s = signed(name, origin);
            s.rev = rev.clone();
            def.signoffs.push(s);
        }
        let mut gone = signed("f", "terminal");
        gone.verdict = SignoffVerdict::Withdrawn;
        gone.reason = "wrong".into();
        def.signoffs.push(gone);
        let md = show(def.clone());
        assert!(
            md.contains("**Approved** by Ada on 2026-11-06 via the monitor"),
            "{md}"
        );
        for (name, ends) in [
            ("a", "via the monitor"),
            ("b", "via the conversation"),
            ("c", "via the terminal"),
            ("d", "under a delegation"),
            ("e", "origin unknown"),
        ] {
            let line = md
                .lines()
                .find(|l| l.contains(&format!("Signed off `{name}`")))
                .unwrap_or_else(|| panic!("no line for {name}: {md}"));
            assert!(line.contains(ends), "{line}");
        }
        let line = md.lines().find(|l| l.contains("`f` withdrawn")).unwrap();
        assert!(line.contains("via the terminal"), "{line}");
        // A definition written before origin existed.
        def.approval.as_mut().unwrap().origin.clear();
        assert!(show(def).contains("**Approved** by Ada on 2026-11-06 — origin unknown"));
    }
}
