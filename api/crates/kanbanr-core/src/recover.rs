//! The session-start summary, sized to fit (DEF-093).
//!
//! The SessionStart hook used to print the whole board and every recovery section: about 40 KB
//! on a board of 400 items, which Claude Code saves to a file and previews in about 2 KB, so what
//! is in progress and what waits for the person never reached the session. [`render`] keeps the
//! full form whenever it fits the budget and otherwise builds a brief form that leads with the
//! work in progress, what waits for the person, the sprint and release and where to read more,
//! counts the rest, and adds the optional sections only while they still fit.
//!
//! Pure: the CLI gathers the [`Inputs`] (read-only, ADR-0005) and prints what this returns.

/// The default budget in bytes (decided 16 Nov 2026, B-37): under Claude Code's inline limit for
/// hook output, with room left for katalogr's map in the same session start.
pub const DEFAULT_BUDGET: usize = 8000;
/// The smallest budget accepted: the preview size, below which the brief form cannot hold its
/// mandatory part on a real board.
pub const MIN_BUDGET: usize = 2000;

/// The heading both forms start with.
pub const HEADER: &str = "## kanbanr — recovered project state (SessionStart)";

/// A titled block of text, printed as `### title` then its body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub body: String,
}

/// One in-progress item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    pub code: String,
    pub status: String,
    pub title: String,
    /// Tasks (done, total).
    pub tasks: (usize, usize),
}

/// What waits for the person.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Waiting {
    /// Item codes to approve or sign off.
    pub items: Vec<String>,
    /// Proposed decisions (ADR ids).
    pub adrs: Vec<String>,
    /// Prepared records to confirm.
    pub prepared: usize,
    /// Proposed baseline revisions (FTR-033).
    pub baseline: usize,
    /// Per delegation in force, the items whose decisions Claude holds under it (DEF-104): not in
    /// `items`, which are the person's.
    pub held: Vec<(String, usize)>,
}

/// Everything [`render`] needs, gathered by the CLI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inputs {
    pub project: String,
    /// The 0.7.0 sections in their order (`Board`, `Recent activity`, `Lessons this project has
    /// learned`, `Waiting for the user`, `Agents at work`, `The project's process`), each omitted
    /// where 0.7.0 omitted it.
    pub full: Vec<Section>,
    pub in_progress: Vec<Line>,
    /// Every board state with its count, in board order.
    pub counts: Vec<(String, usize)>,
    pub waiting: Waiting,
    /// The sprint line, when a sprint is active.
    pub sprint: Option<String>,
    /// The release line(s), when a release is planned.
    pub release: Option<String>,
    /// The brief form's tail, in priority order (dropped last-first): `kanbanr's hooks`, `Agents
    /// at work`, `The project's process`, `The project's practice packs`, `Lessons`, `Recent
    /// activity`. An empty section is skipped.
    pub optional: Vec<Section>,
    /// The closing paragraph.
    pub footer: String,
    /// The brief form's closing paragraph, which has no board above it to point at; the full
    /// form's `footer` when empty.
    pub brief_footer: String,
}

/// The sentence on offering a review in the conversation, as 0.7.0 printed it.
const OFFER_REVIEW: &str = "Offer to review them here, one question each — see the skill's \
\"Waiting decisions\" — rather than sending the user to the browser.";

const READ_MORE: &str = "`kanbanr board` lists every item; `kanbanr ready` what can start; \
`kanbanr review --pending` what waits; `kanbanr activity` the recent changes; \
`kanbanr lessons --for <CODE>` before starting an item.";

/// The line under "Waiting for the user" when baseline revisions wait (FTR-033).
pub fn baseline_line(n: usize) -> String {
    format!("{n} baseline revision(s) wait on the Baseline page.")
}

/// The sentence on what Claude holds under delegations in force (DEF-104); empty when nothing is.
pub fn held_line(held: &[(String, usize)]) -> String {
    if held.is_empty() {
        return String::new();
    }
    let total: usize = held.iter().map(|(_, n)| n).sum();
    let each: Vec<String> = held.iter().map(|(id, n)| format!("{id} ({n})")).collect();
    format!(
        "Claude holds {total} decision(s) under {}: take them with `--under`, not the person's.",
        each.join(", ")
    )
}

/// The title of the practice-packs section (FTR-033 R-5).
pub const PACKS_TITLE: &str = "The project's practice packs";

/// The practice-packs section: each drifted pack's messages and the rule to ask once; `None`
/// when every pack the project follows is current (FTR-033 R-5).
pub fn packs_section(drift: &[crate::profile::Drift]) -> Option<Section> {
    let lines: Vec<String> = drift.iter().flat_map(|d| d.messages()).collect();
    if lines.is_empty() {
        return None;
    }
    let mut body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    body.push_str(
        "Mention it and ask once whether to update; never run `kanbanr profile update` unasked.",
    );
    Some(Section {
        title: PACKS_TITLE.to_string(),
        body,
    })
}

/// How far the in-progress lines are shortened.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Titles {
    Whole,
    Cut,
    Dropped,
}

/// `### title\nbody\n\n`, the body's trailing newlines folded into one.
fn section(title: &str, body: &str) -> String {
    format!("### {title}\n{}\n\n", body.trim_end_matches('\n'))
}

fn footer(text: &str) -> String {
    let mut f = text.trim_end_matches('\n').to_string();
    f.push('\n');
    f
}

/// `s` cut to `max` characters, marked with an ellipsis when cut.
fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

fn full_form(i: &Inputs) -> String {
    let mut out = format!("{HEADER}\n\n");
    for s in &i.full {
        out.push_str(&section(&s.title, &s.body));
    }
    out.push_str(&footer(&i.footer));
    out
}

/// The brief form's mandatory part before `Read more`, and the part from `Read more` on.
fn mandatory(i: &Inputs, titles: Titles, cut_waiting: bool) -> (String, String) {
    let total: usize = i.counts.iter().map(|(_, n)| n).sum();
    let counts = i
        .counts
        .iter()
        .map(|(s, n)| format!("{s} {n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut head = format!("{HEADER}\n\n{}: {total} items — {counts}\n\n", i.project);

    let lines = i
        .in_progress
        .iter()
        .map(|l| match titles {
            Titles::Whole => format!(
                "  - {} [{}] {} [{}/{}]",
                l.code, l.status, l.title, l.tasks.0, l.tasks.1
            ),
            Titles::Cut => format!(
                "  - {} [{}] {} [{}/{}]",
                l.code,
                l.status,
                cut(&l.title, 60),
                l.tasks.0,
                l.tasks.1
            ),
            Titles::Dropped => format!("  - {} [{}]", l.code, l.status),
        })
        .collect::<Vec<_>>()
        .join("\n");
    head.push_str(&section(
        &format!("In progress ({})", i.in_progress.len()),
        &lines,
    ));

    let w = &i.waiting;
    if !w.items.is_empty()
        || !w.adrs.is_empty()
        || w.prepared > 0
        || w.baseline > 0
        || !w.held.is_empty()
    {
        let list = |ids: &[String]| -> String {
            if ids.is_empty() {
                return String::new();
            }
            if cut_waiting && ids.len() > 10 {
                format!(" ({} and {} more)", ids[..10].join(", "), ids.len() - 10)
            } else {
                format!(" ({})", ids.join(", "))
            }
        };
        let mut body = format!(
            "{} item(s) to approve or sign off{}, {} proposed decision(s){}, {} prepared \
             record(s) to confirm. {OFFER_REVIEW}",
            w.items.len(),
            list(&w.items),
            w.adrs.len(),
            list(&w.adrs),
            w.prepared
        );
        if !w.held.is_empty() {
            body.push(' ');
            body.push_str(&held_line(&w.held));
        }
        if w.baseline > 0 {
            body.push('\n');
            body.push_str(&baseline_line(w.baseline));
        }
        head.push_str(&section("Waiting for the person", &body));
    }

    let plan: Vec<&str> = [i.sprint.as_deref(), i.release.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    if !plan.is_empty() {
        head.push_str(&section("Sprint and release", &plan.join("\n")));
    }

    let closing = if i.brief_footer.trim().is_empty() {
        &i.footer
    } else {
        &i.brief_footer
    };
    let tail = format!("{}{}", section("Read more", READ_MORE), footer(closing));
    (head, tail)
}

/// The summary within `budget` bytes: the full form when it fits, else the brief form (see the
/// module docs). The in-progress codes are never dropped, so on a board with more than about 150
/// items in progress the brief form goes over the budget rather than lose one.
pub fn render(i: &Inputs, budget: usize) -> String {
    let full = full_form(i);
    if full.len() <= budget {
        return full;
    }
    let stages = [
        (Titles::Whole, false),
        (Titles::Cut, false),
        (Titles::Dropped, false),
        (Titles::Dropped, true),
    ];
    let (mut head, tail) = stages
        .iter()
        .map(|&(t, w)| mandatory(i, t, w))
        .find(|(h, t)| h.len() + t.len() <= budget)
        .unwrap_or_else(|| mandatory(i, Titles::Dropped, true));
    for s in &i.optional {
        if s.body.trim().is_empty() {
            continue;
        }
        let block = section(&s.title, &s.body);
        if head.len() + block.len() + tail.len() <= budget {
            head.push_str(&block);
        }
    }
    head.push_str(&tail);
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sec(title: &str, bytes: usize) -> Section {
        Section {
            title: title.to_string(),
            body: "x".repeat(bytes),
        }
    }

    fn base() -> Inputs {
        Inputs {
            project: "demo".into(),
            full: vec![sec("Board", 20_000), sec("Recent activity", 500)],
            in_progress: vec![Line {
                code: "DEF-001".into(),
                status: "In Progress".into(),
                title: "Fix it".into(),
                tasks: (1, 3),
            }],
            counts: vec![
                ("Planned".into(), 300),
                ("In Progress".into(), 1),
                ("Completed".into(), 99),
            ],
            waiting: Waiting {
                items: vec!["FTR-002".into()],
                adrs: vec!["ADR-0009".into()],
                prepared: 2,
                baseline: 0,
                held: Vec::new(),
            },
            sprint: Some(
                "Sprint SP-004 \"ship\", 2026-11-10 to 2026-11-24: 3/9 items finished.".into(),
            ),
            release: Some("Release v0.8.0: at build, next candidate.".into()),
            optional: vec![
                sec("Hooks", 1000),
                sec("Agents at work", 1000),
                sec("The project's process", 1000),
                sec("Lessons", 1000),
                sec("Recent activity", 1000),
            ],
            footer: "_kanbanr is the system of record._".into(),
            brief_footer: String::new(),
        }
    }

    fn has(out: &str, title: &str) -> bool {
        out.contains(&format!("### {title}\n"))
    }

    #[test]
    fn sections_are_dropped_lowest_first_to_fit_the_budget() {
        let i = base();
        let out = render(&i, DEFAULT_BUDGET);
        assert!(out.len() <= DEFAULT_BUDGET, "{} bytes", out.len());
        assert!(out.starts_with(HEADER));
        assert!(!has(&out, "Board"), "the brief form lists no board");
        assert!(out.contains("demo: 400 items — Planned 300, In Progress 1, Completed 99"));
        assert!(out.contains("  - DEF-001 [In Progress] Fix it [1/3]"));
        assert!(out.contains("1 item(s) to approve or sign off (FTR-002), 1 proposed decision(s) (ADR-0009), 2 prepared record(s) to confirm."));
        assert!(has(&out, "Sprint and release"));
        assert!(has(&out, "Read more"));
        assert!(out.contains("`kanbanr board`") && out.contains("`kanbanr ready`"));
        assert!(
            out.trim_end()
                .ends_with("_kanbanr is the system of record._")
        );
        let names = [
            "Hooks",
            "Agents at work",
            "The project's process",
            "Lessons",
            "Recent activity",
        ];
        // With room for all five, all five come, before Read more.
        assert!(names.iter().all(|n| has(&out, n)));
        assert!(out.find("### Recent activity").unwrap() < out.find("### Read more").unwrap());

        // Tighten the budget one section at a time: the lowest left goes first each time.
        let all = out.len();
        for dropped in 1..=names.len() {
            let budget = all
                - i.optional[names.len() - dropped..]
                    .iter()
                    .map(|s| section(&s.title, &s.body).len())
                    .sum::<usize>();
            let out = render(&i, budget);
            assert!(out.len() <= budget, "{} > {budget}", out.len());
            for (k, n) in names.iter().enumerate() {
                assert_eq!(
                    has(&out, n),
                    k < names.len() - dropped,
                    "budget {budget}: {n}"
                );
            }
            assert!(has(&out, "Read more"));
            assert!(
                out.trim_end()
                    .ends_with("_kanbanr is the system of record._")
            );
        }

        // A budget that fits everything gives the full form, in order.
        let full = render(&i, 1_000_000);
        assert!(has(&full, "Board"));
        assert!(full.find("### Board").unwrap() < full.find("### Recent activity").unwrap());
        assert!(!has(&full, "In progress (1)"));
        assert!(!has(&full, "Read more"));
    }

    #[test]
    fn every_in_progress_item_is_named_even_when_titles_are_cut() {
        let mut i = base();
        i.in_progress = (0..120)
            .map(|n| Line {
                code: format!("FTR-{n:03}"),
                status: "In Progress".into(),
                title: "t".repeat(200),
                tasks: (0, 4),
            })
            .collect();
        i.counts = vec![("Planned".into(), 280), ("In Progress".into(), 120)];
        // The full form's board lists every item, so it cannot fit.
        i.full[0].body = (0..280)
            .map(|n| format!("  - PLN-{n:03} planned [0/0]"))
            .chain(
                i.in_progress
                    .iter()
                    .map(|l| format!("  - {} {}", l.code, l.title)),
            )
            .collect::<Vec<_>>()
            .join("\n");
        let out = render(&i, DEFAULT_BUDGET);
        assert!(out.len() <= DEFAULT_BUDGET, "{} bytes", out.len());
        for n in 0..120 {
            assert!(out.contains(&format!("FTR-{n:03}")), "FTR-{n:03} missing");
        }
        assert!(!out.contains(&"t".repeat(61)), "titles are cut or dropped");
        assert!(!out.contains("PLN-"), "no planned item is named");

        // Fewer items: titles are cut to 60 characters rather than dropped.
        i.in_progress.truncate(40);
        let out = render(&i, DEFAULT_BUDGET);
        assert!(out.len() <= DEFAULT_BUDGET);
        assert!(out.contains(&format!("{}…", "t".repeat(60))));
        assert!(!out.contains(&"t".repeat(61)));

        // Too many to fit even as codes: over budget rather than lose one.
        i.in_progress = (0..400)
            .map(|n| Line {
                code: format!("FTR-{n:03}"),
                status: "In Progress".into(),
                title: String::new(),
                tasks: (0, 0),
            })
            .collect();
        let out = render(&i, MIN_BUDGET);
        assert!((0..400).all(|n| out.contains(&format!("FTR-{n:03}"))));
    }

    /// FTR-033 R-5: a proposed baseline revision is one line under what waits, and a drifted pack
    /// is an optional section that obeys the budget like the others.
    #[test]
    fn the_baseline_waits_and_drifted_packs_are_an_optional_section() {
        let mut i = base();
        i.waiting.baseline = 2;
        let out = render(&i, DEFAULT_BUDGET);
        assert!(
            out.contains("\n2 baseline revision(s) wait on the Baseline page."),
            "{out}"
        );

        let drift = crate::profile::Drift {
            source: crate::profile::ProfileSource {
                name: "web".into(),
                library: crate::config::Library::Builtin,
                version: 1,
                rev: "r1".into(),
            },
            newer: Some(2),
            gone: false,
            new_areas: vec!["ui-style".into()],
            changed_recommendations: vec![],
        };
        assert!(packs_section(&[]).is_none());
        let s = packs_section(&[drift]).unwrap();
        assert_eq!(s.title, PACKS_TITLE);
        assert!(s.body.contains("kanbanr profile update web"), "{}", s.body);
        assert!(
            s.body
                .ends_with("never run `kanbanr profile update` unasked.")
        );
        i.optional.insert(3, s.clone());
        let out = render(&i, DEFAULT_BUDGET);
        assert!(out.contains(&format!("### {PACKS_TITLE}\n")), "{out}");
        let tight = out.len() - section(&s.title, &s.body).len() - 1;
        assert!(render(&i, tight).len() <= tight);
    }

    /// DEF-104 (supports R-4): what Claude holds is one sentence naming each delegation; none
    /// held, none printed; the brief form keeps it.
    #[test]
    fn the_held_line_names_each_delegation() {
        let mut i = base();
        let out = render(&i, DEFAULT_BUDGET);
        assert!(!out.contains("Claude holds"), "{out}");
        i.waiting.held = vec![("DLG-001".into(), 3), ("DLG-002".into(), 1)];
        let out = render(&i, DEFAULT_BUDGET);
        assert!(
            out.contains(
                "Claude holds 4 decision(s) under DLG-001 (3), DLG-002 (1): take them with \
                 `--under`, not the person's."
            ),
            "{out}"
        );
        assert!(out.len() <= DEFAULT_BUDGET);
        // Held only: the section is still printed.
        i.waiting.items.clear();
        i.waiting.adrs.clear();
        i.waiting.prepared = 0;
        assert!(render(&i, DEFAULT_BUDGET).contains("### Waiting for the person"));
    }
}
