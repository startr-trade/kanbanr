//! `kanbanr recover`: the plan to resume from, sized for a session start (DEF-093).
//!
//! Gathers the [`Inputs`] over the backend, from the same routes and printers the 0.7.0
//! SessionStart script called one by one, and prints what [`recover::render`] makes of them.
//! Read-only (ADR-0005), and best effort: a section whose source errors is left out, and with no
//! project to recover the command prints nothing and succeeds, so it never breaks a session
//! (ADR-0004).

use crate::backend::Backend;
use crate::{
    Cli, activity_text, board_text, decisions_summary, get_project, hookhealth, lessons_text,
};
use crate::{process_status_text, project_drift, scope_drift_text, scope_list_text};
use kanbanr_core::recover::{self, Inputs, Line, Section, Waiting};
use kanbanr_core::{Project, project};
use serde_json::{Value, json};

/// The environment variable that sets the budget when `--budget` is not given.
pub const BUDGET_ENV: &str = "KANBANR_SESSION_BUDGET";

const LESSONS_NOTE: &str = "_Before starting an item, check `kanbanr lessons --for <CODE>`. If one of\n\
these turns out to be wrong, say so: `kanbanr lesson contradict <L-n> --note '…'`._";

const FOOTER: &str = "_kanbanr is the system of record for this project's PLAN (features/tasks/\n\
specs/decisions/progress). Resume from the board above; record new work in\n\
kanbanr as you go. Every document (requested or self-initiated) goes in\n\
kanbanr docs (`kanbanr doc add …`), NOT the working folder — unless the user\n\
asks for it in the project folder._";

/// The brief form's closing paragraph: it lists no board, so it points at the summary instead
/// (coordinator decision, 17 Nov 2026).
const BRIEF_FOOTER: &str = "_kanbanr is the system of record for this project's PLAN (features/tasks/\n\
specs/decisions/progress). Resume from the summary above; kanbanr board gives the whole board.\n\
Record new work in kanbanr as you go. Every document (requested or self-initiated) goes in\n\
kanbanr docs (`kanbanr doc add …`), NOT the working folder — unless the user\n\
asks for it in the project folder._";

/// The budget in force: the flag, else the environment, else the default; never below the
/// minimum.
fn budget(flag: Option<usize>) -> usize {
    flag.or_else(|| {
        std::env::var(BUDGET_ENV)
            .ok()
            .and_then(|v| v.trim().parse().ok())
    })
    .unwrap_or(recover::DEFAULT_BUDGET)
    .max(recover::MIN_BUDGET)
}

fn section(title: &str, body: String) -> Section {
    Section {
        title: title.to_string(),
        body,
    }
}

/// The first `n` lessons of `lessons_text`: each starts on an unindented line.
fn first_lessons(text: &str, n: usize) -> String {
    let mut seen = 0;
    let mut out = String::new();
    for line in text.lines() {
        if !line.starts_with(' ') {
            seen += 1;
            if seen > n {
                break;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn first_lines(text: &str, n: usize) -> String {
    text.lines().take(n).map(|l| format!("{l}\n")).collect()
}

/// The ids in a JSON list whose `key` holds a string.
fn ids(v: &Value, key: &str) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x[key].as_str().map(str::to_string))
        .collect()
}

fn sprint_line(client: &Backend, p: &str, project: &Project) -> Option<String> {
    let sprints = kanbanr_core::sprints::load(client.store(), p).ok()?;
    let s = kanbanr_core::sprints::active(&sprints)?;
    let items: Vec<_> = project
        .features
        .iter()
        .filter(|f| f.sprint.as_deref() == Some(s.code.as_str()))
        .collect();
    let done = items
        .iter()
        .filter(|f| kanbanr_core::graph::is_terminal_status(&project.config, &f.status))
        .count();
    Some(format!(
        "Sprint {} \"{}\", {} to {}: {done}/{} items finished.",
        s.code,
        s.goal,
        s.start,
        s.end,
        items.len()
    ))
}

fn release_line(client: &Backend, p: &str, project: &Project) -> Option<String> {
    use kanbanr_core::releases::ReleaseState;
    let releases = kanbanr_core::releases::load(client.store(), p).ok()?;
    let stages = project.config.cadence.stages();
    let lines: Vec<String> = releases
        .iter()
        .rev()
        .filter(|r| r.state == ReleaseState::Planned)
        .take(2)
        .map(|r| {
            if r.waits_on_tag() {
                return format!(
                    "Release {}: tagged in {}, waiting for the tag workflow.",
                    r.version, r.version
                );
            }
            let next = match stages.iter().position(|s| *s == r.stage) {
                Some(at) => stages.get(at + 1),
                None if r.stage.is_empty() => stages.first(),
                None => None,
            };
            format!(
                "Release {}: at {}, next {}.",
                r.version,
                if r.stage.is_empty() {
                    "not started"
                } else {
                    &r.stage
                },
                next.map_or("cut", String::as_str)
            )
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Is an item in this status in progress: on the board, past the default state, and neither
/// finished nor a no-op disposition. Finished as `graph::is_terminal_status` decides it, so a board
/// that declares no `terminal_states` treats Completed as finished, as the rest of kanbanr does.
fn in_progress(config: &kanbanr_core::ProjectConfig, status: &str) -> bool {
    status != config.default_state
        && !kanbanr_core::graph::is_terminal_status(config, status)
        && !config.is_no_op(status)
        && config.board_states().iter().any(|s| s == status)
}

/// Everything `render` needs, gathered best effort.
fn gather(client: &Backend, p: &str, project: &Project) -> Inputs {
    let config = &project.config;
    let mut full = Vec::new();

    let mut board = board_text(project);
    if let (Some(line), _) = decisions_summary(client, p) {
        board.push_str(&line);
        board.push('\n');
    }
    full.push(section("Board", board));

    let activity = activity_text(client, p).ok();
    if let Some(a) = &activity {
        full.push(section("Recent activity", a.clone()));
    }

    let lessons = lessons_text(client, p, None, false)
        .ok()
        .filter(|l| l.trim() != "(nothing learned here yet)" && !l.trim().is_empty());
    if let Some(l) = &lessons {
        full.push(section(
            "Lessons this project has learned",
            format!("{}\n{LESSONS_NOTE}", first_lines(l, 10)),
        ));
    }

    // The queue's entries; those every decision of which a delegation in force holds are Claude's,
    // not the person's (DEF-104).
    let queue = client
        .get(&format!("/projects/{p}/review"))
        .ok()
        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .unwrap_or_default();
    let entries = queue.as_array().cloned().unwrap_or_default();
    let items: Vec<String> = ids(
        &Value::Array(
            entries
                .iter()
                .filter(|e| e["claudes"] != true)
                .cloned()
                .collect(),
        ),
        "code",
    );
    let mut held: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for e in &entries {
        let mut ids: Vec<&str> = e["held"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| h["delegation"].as_str())
            .collect();
        ids.sort();
        ids.dedup();
        for id in ids {
            *held.entry(id.to_string()).or_default() += 1;
        }
    }
    let held: Vec<(String, usize)> = held.into_iter().collect();
    let adrs: Vec<String> = client
        .get(&format!("/projects/{p}/adrs"))
        .ok()
        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .map(|v| {
            v.as_array()
                .into_iter()
                .flatten()
                .filter(|a| a["status"] == "proposed")
                .filter_map(|a| a["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let prepared = client
        .get(&format!("/projects/{p}/prepared?state=open"))
        .ok()
        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .and_then(|v| v.as_array().map(Vec::len))
        .unwrap_or(0);
    // FTR-033: the baseline revisions Claude proposed that wait for the person.
    let baseline = kanbanr_core::baseline::load(client.store(), p)
        .map(|b| {
            b.revisions
                .iter()
                .filter(|r| r.state() == kanbanr_core::baseline::RevisionState::Proposed)
                .count()
        })
        .unwrap_or(0);
    if !items.is_empty() || !adrs.is_empty() || baseline > 0 || !held.is_empty() {
        let mut body = format!(
            "{} item(s) and {} proposed decision(s) need a verdict. Offer to review them here,\n\
             one question each — see the skill's \"Waiting decisions\" — rather than sending the user\n\
             to the browser.",
            items.len(),
            adrs.len()
        );
        if !held.is_empty() {
            body.push_str(&format!("\n{}", recover::held_line(&held)));
        }
        if baseline > 0 {
            body.push_str(&format!("\n{}", recover::baseline_line(baseline)));
        }
        full.push(section("Waiting for the user", body));
    }

    let scopes = scope_list_text(client, p, false, None)
        .ok()
        .filter(|s| s.trim() != "(no agent scopes in force)" && !s.trim().is_empty());
    let drift = scope_drift_text(client, p, true)
        .ok()
        .filter(|d| !d.trim().is_empty());
    let agents = (scopes.is_some() || drift.is_some()).then(|| {
        let mut body = scopes.unwrap_or_default();
        if let Some(d) = drift {
            body.push_str("Ahead of the board — bring these items' tasks up to date first:\n");
            body.push_str(&d);
        }
        section("Agents at work", body)
    });
    if let Some(a) = &agents {
        full.push(a.clone());
    }

    let process = project_drift(client, project)
        .ok()
        .map(|d| process_status_text(p, d.as_ref(), true))
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            section(
                "The project's process",
                format!(
                    "{t}Mention it and ask once whether to update; never run `kanbanr process update` unasked."
                ),
            )
        });
    if let Some(s) = &process {
        full.push(s.clone());
    }

    // FTR-033 R-5: the practice packs whose library copy moved on, reported, never applied.
    let personal = kanbanr_core::profile::personal_dir(project::home_dir().as_deref());
    let packs = client
        .store()
        .profile_drift(p, personal.as_deref())
        .ok()
        .and_then(|d| recover::packs_section(&d));
    if let Some(s) = &packs {
        full.push(s.clone());
    }

    // FTR-059 R-1, R-8: the kanbanr hooks that do not run, git's then Claude Code's, one line
    // each. The first optional section, so the last to be dropped.
    let hooks = crate::code_repo()
        .filter(|root| project::find_marker(root, project::home_dir().as_deref()).is_some())
        .map(|root| {
            // A hook this kanbanr registers that the settings lack: the agent offers a refresh
            // (DEF-126 R-5). The project's two settings files, then the user's.
            let files = kanbanr_core::claude::settings_files(&root);
            let (project_files, user_files) = files.split_at(files.len().min(2));
            let refresh = crate::hooks::out_of_date_line(project_files, user_files)
                .map(|line| format!("- {line}\n"))
                .unwrap_or_default();
            // The refresh line already names the missing hook and how to add it: the generic
            // "not registered in any settings file" line would only contradict it.
            let claude = if refresh.is_empty() {
                hookhealth::claude_hook_findings(&root)
            } else {
                Vec::new()
            };
            refresh
                + &hookhealth::git_hook_findings(&root)
                    .into_iter()
                    .chain(claude)
                    .map(|f| format!("- {f}\n"))
                    .collect::<String>()
        })
        .filter(|body| !body.is_empty())
        .map(|body| section("kanbanr's hooks", body));
    if let Some(h) = &hooks {
        full.push(h.clone());
    }

    let mut optional: Vec<Section> = hooks.into_iter().collect();
    optional.extend(agents);
    optional.extend(process);
    optional.extend(packs);
    if let Some(l) = &lessons {
        optional.push(section("Lessons", first_lessons(l, 3)));
    }
    if let Some(a) = &activity {
        // The activity lists the newest first: its first lines are the latest.
        optional.push(section("Recent activity", first_lines(a, 5)));
    }

    let in_progress = project
        .features
        .iter()
        .filter(|f| in_progress(config, &f.status))
        .map(|f| Line {
            code: f.code.clone(),
            status: f.status.clone(),
            title: f.title.clone(),
            tasks: (f.done_count(), f.task_count()),
        })
        .collect();
    let counts = config
        .board_states()
        .iter()
        .map(|s| {
            (
                s.clone(),
                project.features.iter().filter(|f| &f.status == s).count(),
            )
        })
        .collect();

    Inputs {
        project: p.to_string(),
        full,
        in_progress,
        counts,
        waiting: Waiting {
            items,
            adrs,
            prepared,
            baseline,
            held,
        },
        sprint: sprint_line(client, p, project),
        release: release_line(client, p, project),
        optional,
        footer: FOOTER.to_string(),
        brief_footer: BRIEF_FOOTER.to_string(),
    }
}

/// Print the session-start summary within the budget (flag, `KANBANR_SESSION_BUDGET`, default).
/// With no project to recover, print nothing and succeed: the hook's contract.
pub fn run_recover(cli: &Cli, client: &Backend, flag: Option<usize>) -> anyhow::Result<()> {
    let Some(p) = project::resolve_project(cli.project.as_deref()) else {
        return Ok(());
    };
    let Ok(project) = get_project(client, &p) else {
        return Ok(());
    };
    let inputs = gather(client, &p, &project);
    let budget = budget(flag);
    let text = recover::render(&inputs, budget);
    if cli.json {
        let form = if text == recover::render(&inputs, usize::MAX) {
            "full"
        } else {
            "brief"
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "bytes": text.len(),
                "form": form,
                "text": text,
            }))?
        );
    } else {
        print!("{text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DEF-093 R-1: a board that declares no `terminal_states` (one made before they existed, like
    /// kanbanr's own) still counts Completed as finished, so the summary does not list it as work
    /// in progress.
    #[test]
    fn a_board_without_terminal_states_does_not_list_completed_items_as_in_progress() {
        let mut config = kanbanr_core::ProjectConfig::for_new_project("demo");
        config.terminal_states.clear();
        assert!(config.board_states().iter().any(|s| s == "Completed"));
        assert!(in_progress(&config, "In Progress"));
        assert!(!in_progress(&config, "Completed"), "Completed is finished");
        assert!(!in_progress(&config, &config.default_state.clone()));

        // A workflow that names its own end state is taken at its word.
        config.terminal_states = vec!["Completed".into()];
        assert!(!in_progress(&config, "Completed"));
        assert!(in_progress(&config, "In Progress"));
    }
}
