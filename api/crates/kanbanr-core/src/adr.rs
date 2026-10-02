//! Architecture decisions, in the graph (FEAT-057).
//!
//! A decision is not deliverable work: it has no estimate, no branch, no tests of its own, and a
//! flow metric that counted it would be measuring the wrong thing. So ADRs stay **documents** —
//! they just gain front-matter, which is what lets the rest of the tooling see them.
//!
//! The links are canonical **on the decision**: an ADR says what it `affects` and what
//! requirements `drove` it. The item's view of "decisions that shaped this" is derived from that,
//! never stored, so the two sides cannot drift apart. `supersedes` is the one exception, because
//! superseding is a fact about both documents: it is written on both, in one change.
//!
//! The decision text is the document body, not a front-matter field. Prose belongs in prose;
//! structure holds identifiers, enums and links. A `text:` field would duplicate the body and rot.

use crate::error::{CoreError, Result};
use crate::{Project, Store};
use serde::{Deserialize, Serialize};

/// Where decisions live in the board's doc tree.
pub const FOLDER: &str = "decisions";

/// The sections `adr new` scaffolds. A decision missing the middle two is a note, not a record:
/// Consequences is the part a later reader needs most, because it says what was paid.
pub const SECTIONS: [&str; 5] = [
    "Context",
    "Decision",
    "Alternatives considered",
    "Consequences",
    "Compliance",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TradeOff {
    /// The quality bought.
    pub gain: String,
    /// The quality spent for it. A decision with no cost is usually one nobody examined.
    pub cost: String,
}

/// An architecture decision record: front-matter plus the prose it belongs to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Adr {
    /// `ADR-0003`.
    #[serde(default)]
    pub id: String,
    /// proposed | accepted | rejected | superseded.
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deciders: Vec<String>,
    /// Items resting on this decision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects: Vec<String>,
    /// The requirements that forced it, usually quality ones: `FEAT-046/R-2`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub driven_by: Vec<String>,
    /// ISO/IEC 25010 characteristics at stake — the shared vocabulary with NFRs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality: Vec<String>,
    /// The machine-readable summary of the Consequences prose.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trade_offs: Vec<TradeOff>,
    /// Which Zachman columns it answers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zachman: Vec<String>,
    /// conceptual | logical | physical.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub layer: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
    /// Written by `adr supersede`, never by hand.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub superseded_by: String,
    /// When the decider accepted or rejected it (FEAT-153). `date` is when it was proposed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub decided: String,
    /// Why it was rejected: a rejected decision is still a record, and its reason is the record.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// Front-matter keys this version does not know, kept (FEAT-151).
    #[serde(
        flatten,
        default,
        skip_serializing_if = "crate::models::Extra::is_empty"
    )]
    pub extra: crate::models::Extra,

    // The three below are part of the document, not of its front-matter: `render` strips them
    // before writing, so the title is never stored twice and the body is never stored at all.
    // They travel over the API, though, because a caller listing decisions wants them.
    /// The document's first heading.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Everything below the front-matter.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// Where it lives in the doc tree.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// The scaffolded sections it leaves unanswered, as `list` reports them — so the Review page
    /// can say why Accept is refused without a second copy of the rule (FEAT-153). Never stored.
    #[serde(default, skip_deserializing, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

impl Adr {
    pub fn is_superseded(&self) -> bool {
        self.status.eq_ignore_ascii_case("superseded") || !self.superseded_by.trim().is_empty()
    }

    pub fn is_accepted(&self) -> bool {
        self.status.eq_ignore_ascii_case("accepted")
    }

    /// Which scaffolded sections this decision actually fills in. A heading with nothing under it
    /// counts as missing — an empty Consequences section is the most common way an ADR lies — and
    /// so does one still holding the scaffold's prompt, which is the second most common way.
    pub fn filled_sections(&self) -> Vec<&'static str> {
        SECTIONS
            .into_iter()
            .filter(|name| {
                section_body(&self.body, name).is_some_and(|section| {
                    section
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .any(|l| !(l.starts_with("<!--") && l.ends_with("-->")))
                })
            })
            .collect()
    }

    pub fn missing_sections(&self) -> Vec<&'static str> {
        let filled = self.filled_sections();
        SECTIONS
            .into_iter()
            .filter(|s| !filled.contains(s))
            .collect()
    }
}

/// The text under a `## Section` heading, in either the heading form or the `**Section.**` form the
/// project's first ADR used — an older document should not read as broken because the scaffold
/// changed.
fn section_body<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let lower = body.to_lowercase();
    let name = name.to_lowercase();
    let start = lower
        .find(&format!("## {name}"))
        .map(|at| at + 3 + name.len())
        .or_else(|| {
            lower
                .find(&format!("**{name}.**"))
                .map(|at| at + 5 + name.len())
        })?;
    let rest = &body[start..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Parse a document into a decision. `None` when it carries no ADR front-matter — most documents
/// are prose, and prose is not a broken ADR.
pub fn parse(path: &str, text: &str) -> Option<Adr> {
    let (front, body) = crate::docs::split_front_matter(text);
    let mut adr: Adr = serde_yaml::from_str(front?).ok()?;
    if adr.id.trim().is_empty() {
        return None;
    }
    adr.title = body
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or("")
        .trim()
        .to_string();
    adr.body = body.to_string();
    adr.path = path.to_string();
    Some(adr)
}

/// Front-matter plus prose. The document's own text — its title and body — is written once, as
/// prose; duplicating it into the front-matter would give the two copies a chance to disagree.
pub fn render(adr: &Adr) -> String {
    let front = Adr {
        title: String::new(),
        body: String::new(),
        path: String::new(),
        missing: Vec::new(),
        ..adr.clone()
    };
    let front = serde_yaml::to_string(&front).unwrap_or_default();
    crate::docs::with_front_matter(&front, &adr.body)
}

/// Every decision on the board, newest id first.
pub fn list(store: &Store, project: &str) -> Result<Vec<Adr>> {
    let mut out: Vec<Adr> = store
        .list_docs(project)?
        .into_iter()
        .filter(|p| p.starts_with(FOLDER))
        .filter_map(|p| store.read_doc(project, &p).ok().and_then(|t| parse(&p, &t)))
        .map(|mut adr| {
            adr.missing = adr
                .missing_sections()
                .into_iter()
                .map(String::from)
                .collect();
            adr
        })
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

pub fn get(store: &Store, project: &str, id: &str) -> Result<Adr> {
    list(store, project)?
        .into_iter()
        .find(|a| a.id.eq_ignore_ascii_case(id))
        .ok_or_else(|| CoreError::Unsupported(format!("no decision {id} on this board")))
}

/// The next free number — counting the **files** in the decisions folder, not only the documents
/// that parse as ADRs. A decision written before front-matter existed is invisible to the parser
/// but very much visible to a reader, and handing its number to a new decision would leave two
/// documents both calling themselves 0001.
fn next_id(store: &Store, project: &str, existing: &[Adr]) -> String {
    let from_ids = existing
        .iter()
        .filter_map(|a| a.id.rsplit('-').next()?.parse::<u32>().ok());
    let from_files = store
        .list_docs(project)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.starts_with(FOLDER))
        .filter_map(|p| {
            let name = p.rsplit('/').next()?.to_string();
            let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u32>().ok()
        })
        .collect::<Vec<_>>();
    let highest = from_ids.chain(from_files).max().unwrap_or(0);
    format!("ADR-{:04}", highest + 1)
}

/// The scaffolded body: the five sections, each with a one-line prompt for what belongs there.
/// Prompts, not placeholder prose — a template that reads like an answer gets left in.
fn scaffold(title: &str) -> String {
    format!(
        "# {title}\n\n\
         ## Context\n\n\
         <!-- The forces in play. Name the quality requirements that drive this. -->\n\n\
         ## Decision\n\n\
         <!-- \"We will …\" — one paragraph, in the present tense. -->\n\n\
         ## Alternatives considered\n\n\
         <!-- What else was on the table, and why it lost. -->\n\n\
         ## Consequences\n\n\
         <!-- What this buys and what it costs. The part a later reader needs before overturning it. -->\n\n\
         ## Compliance\n\n\
         <!-- How we would notice this being violated: the test, check or review that enforces it. -->\n"
    )
}

/// Scaffold a new decision and write it into the board's doc tree.
pub fn create(store: &Store, project: &str, title: &str, mut adr: Adr) -> Result<Adr> {
    let existing = list(store, project)?;
    adr.id = next_id(store, project, &existing);
    if adr.status.trim().is_empty() {
        adr.status = "proposed".to_string();
    }
    if adr.date.trim().is_empty() {
        adr.date = crate::now_rfc3339()
            .get(..10)
            .unwrap_or_default()
            .to_string();
    }
    adr.title = title.to_string();
    adr.body = scaffold(title);
    let slug = crate::scm::slug(title);
    adr.path = format!(
        "{FOLDER}/{}-{slug}.md",
        adr.id.trim_start_matches("ADR-").to_lowercase()
    );
    store.write_doc(project, &adr.path, &render(&adr))?;
    Ok(adr)
}

/// Replace a decision's front-matter, keeping its prose untouched.
pub fn update(store: &Store, project: &str, adr: &Adr) -> Result<()> {
    store.write_doc(project, &adr.path, &render(adr))?;
    Ok(())
}

/// Accept or reject a proposed decision (FEAT-153). A decision is the user's to make: the CLI and
/// the Review page call this with the commit identity, and nothing else does. Accepting needs every
/// section answered — an accepted decision with an empty Consequences is the record lying — and a
/// rejection needs its reason.
pub fn decide(
    store: &Store,
    project: &str,
    id: &str,
    accept: bool,
    by: &str,
    reason: &str,
) -> Result<Adr> {
    let mut adr = get(store, project, id)?;
    if !adr.status.eq_ignore_ascii_case("proposed") {
        return Err(CoreError::Unsupported(format!(
            "{} is {}, not proposed: only a proposed decision is accepted or rejected",
            adr.id, adr.status
        )));
    }
    if by.trim().is_empty() {
        return Err(CoreError::Unsupported(
            "a decision must name who made it".to_string(),
        ));
    }
    if accept {
        let missing = adr.missing_sections();
        if !missing.is_empty() {
            return Err(CoreError::Unsupported(format!(
                "{} cannot be accepted with unanswered sections: {}",
                adr.id,
                missing.join(", ")
            )));
        }
    } else if reason.trim().is_empty() {
        return Err(CoreError::Unsupported(format!(
            "rejecting {} needs a reason: it is what a later reader of the rejection needs",
            adr.id
        )));
    }
    adr.status = if accept { "accepted" } else { "rejected" }.to_string();
    if !adr.deciders.iter().any(|d| d == by) {
        adr.deciders.push(by.to_string());
    }
    adr.decided = crate::now_rfc3339()
        .get(..10)
        .unwrap_or_default()
        .to_string();
    adr.reason = if accept {
        String::new()
    } else {
        reason.trim().to_string()
    };
    update(store, project, &adr)?;
    Ok(adr)
}

/// What superseding one decision with another produced: both sides written, and the work that was
/// resting on the overturned one — those items now stand on a decision nobody holds any more.
#[derive(Debug, Clone, Serialize)]
pub struct Superseded {
    pub new: String,
    pub old: String,
    /// Items the old decision claimed to affect, for review.
    pub affected: Vec<String>,
    /// Requirements that drove it, which may need a new decision.
    pub driven_by: Vec<String>,
}

/// Write both sides of a supersede in one change. This is the one two-sided link in the model,
/// because it is a fact about both documents, and a reader arriving at either one needs it.
pub fn supersede(store: &Store, project: &str, new_id: &str, old_id: &str) -> Result<Superseded> {
    if new_id.eq_ignore_ascii_case(old_id) {
        return Err(CoreError::Unsupported(
            "a decision cannot supersede itself".to_string(),
        ));
    }
    let mut new = get(store, project, new_id)?;
    let mut old = get(store, project, old_id)?;
    // A cycle would make the lineage unwalkable, and `adr history` follows these links. The
    // check is the plain one: if the decision being overturned is already an ancestor of the one
    // overturning it, the chain would close on itself.
    if lineage(&list(store, project)?, &old.id)
        .iter()
        .any(|a| a.eq_ignore_ascii_case(new_id))
    {
        return Err(CoreError::Unsupported(format!(
            "{old_id} already stands on {new_id}; superseding it that way would be a cycle"
        )));
    }
    if !new
        .supersedes
        .iter()
        .any(|s| s.eq_ignore_ascii_case(old_id))
    {
        new.supersedes.push(old.id.clone());
    }
    old.superseded_by = new.id.clone();
    old.status = "superseded".to_string();
    update(store, project, &new)?;
    update(store, project, &old)?;
    Ok(Superseded {
        new: new.id,
        affected: old.affects.clone(),
        driven_by: old.driven_by.clone(),
        old: old.id,
    })
}

/// The chain of decisions leading to this one: 0001 → 0003 → 0007. Reading it is how a later
/// reader learns why the approach changed over time, rather than only what it is now.
pub fn lineage(adrs: &[Adr], id: &str) -> Vec<String> {
    let mut chain = vec![id.to_string()];
    let mut current = id.to_string();
    // Bounded by the number of decisions, so a malformed pair cannot spin forever.
    for _ in 0..adrs.len() {
        let Some(adr) = adrs.iter().find(|a| a.id.eq_ignore_ascii_case(&current)) else {
            break;
        };
        match adr.supersedes.first() {
            Some(previous) if !chain.contains(previous) => {
                chain.insert(0, previous.clone());
                current = previous.clone();
            }
            _ => break,
        }
    }
    chain
}

/// The decisions bearing on one item — derived from what each decision claims, so the item side
/// never has to be edited and can never disagree.
pub fn for_item(adrs: &[Adr], code: &str) -> Vec<String> {
    adrs.iter()
        .filter(|a| {
            a.affects.iter().any(|x| x == code)
                || a.driven_by
                    .iter()
                    .any(|d| d.split('/').next().is_some_and(|c| c == code))
        })
        .map(|a| a.id.clone())
        .collect()
}

/// Does this reference resolve? Used by the commit hook and doctor: an unknown decision in a
/// trailer is a broken link, exactly like a dangling dependency.
pub fn exists(adrs: &[Adr], id: &str) -> bool {
    adrs.iter().any(|a| a.id.eq_ignore_ascii_case(id))
}

/// Decisions whose `affects`/`driven_by` name something that is not on the board.
pub fn dangling(adrs: &[Adr], project: &Project) -> Vec<String> {
    let mut out = Vec::new();
    for adr in adrs {
        for code in &adr.affects {
            if project.feature(code).is_err() {
                out.push(format!("{} affects {code}, which is not an item", adr.id));
            }
        }
        for driver in &adr.driven_by {
            let (code, requirement) = match driver.split_once('/') {
                Some((c, r)) => (c, Some(r)),
                None => (driver.as_str(), None),
            };
            match project.feature(code) {
                Err(_) => out.push(format!(
                    "{} is driven by {driver}, which is not an item",
                    adr.id
                )),
                Ok(feature) => {
                    if let Some(id) = requirement
                        && !feature.definition.as_ref().is_some_and(|d| {
                            d.requirements.iter().any(|r| r.id.eq_ignore_ascii_case(id))
                        })
                    {
                        out.push(format!(
                            "{} is driven by {driver}, but {code} has no requirement {id}",
                            adr.id
                        ));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;

    fn fixture() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-adr-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        (store, dir)
    }

    /// FEAT-153 R-1: accepting and rejecting record the verdict, who gave it, when, and a
    /// rejection's reason — and only a proposed decision takes a verdict.
    #[test]
    fn adr_accept_and_reject_record_status_decider_date_and_reason() {
        let (store, dir) = fixture();
        let filled = |title: &str| {
            let mut adr = create(&store, "demo", title, Adr::default()).unwrap();
            adr.body = format!(
                "# {title}\n\n## Context\n\nc\n\n## Decision\n\nd\n\n\
                 ## Alternatives considered\n\na\n\n## Consequences\n\nq\n\n## Compliance\n\nt\n"
            );
            update(&store, "demo", &adr).unwrap();
            adr.id
        };
        let yes = filled("Keep it");
        let no = filled("Drop it");

        let accepted = decide(&store, "demo", &yes, true, "Ada L", "").unwrap();
        assert_eq!(accepted.status, "accepted");
        assert_eq!(accepted.deciders, vec!["Ada L".to_string()]);
        assert_eq!(accepted.decided.len(), 10, "a date: {}", accepted.decided);
        let reread = get(&store, "demo", &yes).unwrap();
        assert_eq!(
            (reread.status.as_str(), reread.decided.as_str()),
            ("accepted", accepted.decided.as_str())
        );
        assert!(
            reread.body.contains("## Consequences\n\nq"),
            "the prose is untouched"
        );

        assert!(
            decide(&store, "demo", &no, false, "Ada L", " ").is_err(),
            "a rejection needs a reason"
        );
        let rejected = decide(&store, "demo", &no, false, "Ada L", "too costly").unwrap();
        assert_eq!(
            (rejected.status.as_str(), rejected.reason.as_str()),
            ("rejected", "too costly")
        );

        let again = decide(&store, "demo", &yes, false, "Ada L", "changed my mind").unwrap_err();
        assert!(again.to_string().contains("not proposed"), "{again}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FEAT-153 R-2: a decision with an unanswered section cannot be accepted, and the refusal
    /// names what is missing.
    #[test]
    fn accepting_a_decision_with_an_empty_section_is_refused() {
        let (store, dir) = fixture();
        let adr = create(&store, "demo", "Half written", Adr::default()).unwrap();
        let err = decide(&store, "demo", &adr.id, true, "Ada L", "")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Consequences") && err.contains("Context"),
            "{err}"
        );
        assert_eq!(get(&store, "demo", &adr.id).unwrap().status, "proposed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_decision_round_trips_with_its_front_matter_and_sections() {
        let (store, dir) = fixture();
        let item = store.add_feature("demo", "Mirror", "", "M", None).unwrap();
        let created = create(
            &store,
            "demo",
            "Mirror writes are one-way",
            Adr {
                status: "accepted".into(),
                deciders: vec!["Venkatraman".into()],
                affects: vec![item.code.clone()],
                driven_by: vec![format!("{}/R-1", item.code)],
                quality: vec!["Reliability".into()],
                trade_offs: vec![TradeOff {
                    gain: "Reliability".into(),
                    cost: "Interaction Capability".into(),
                }],
                zachman: vec!["How".into()],
                layer: "logical".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(created.id, "ADR-0001");
        assert_eq!(created.path, "decisions/0001-mirror-writes-are-one-way.md");

        // A decision written before front-matter existed still holds its number: the parser
        // cannot see it, but a reader can, and two documents called 0001 help nobody.
        store
            .write_doc(
                "demo",
                "decisions/0007-older-style.md",
                "# An older decision\n",
            )
            .unwrap();
        let after_legacy = create(&store, "demo", "Next one", Adr::default()).unwrap();
        assert_eq!(
            after_legacy.id, "ADR-0008",
            "the legacy file's number is taken"
        );

        // Read back from the board exactly as written: front-matter and prose both survive.
        let read = get(&store, "demo", "ADR-0001").unwrap();
        assert_eq!(read.affects, vec![item.code.clone()]);
        assert_eq!(read.trade_offs[0].cost, "Interaction Capability");
        assert_eq!(read.title, "Mirror writes are one-way");
        assert!(read.is_accepted());

        // The scaffold puts the five sections there; none of them is filled in yet, and an empty
        // Consequences section is reported rather than counted.
        assert_eq!(read.filled_sections(), Vec::<&str>::new());
        assert!(read.missing_sections().contains(&"Consequences"));

        let written = Adr {
            body: read.body.replace(
                "<!-- What this buys and what it costs. The part a later reader needs before overturning it. -->",
                "Fewer surprises for the reader; one more thing to keep in step.",
            ),
            ..read
        };
        update(&store, "demo", &written).unwrap();
        let read = get(&store, "demo", "ADR-0001").unwrap();
        assert_eq!(read.filled_sections(), vec!["Consequences"]);

        // Prose with no front-matter is not a broken decision, it is just prose.
        assert!(parse("design/overview.md", "# Overview\n\nSome prose.").is_none());
        // Neither is the project's older ADR style — but its sections still read.
        let legacy = "---\nid: ADR-0009\nstatus: accepted\n---\n\n# Old style\n\n**Decision.** We will keep it.\n";
        let legacy = parse("decisions/0009-old.md", legacy).unwrap();
        assert_eq!(legacy.filled_sections(), vec!["Decision"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn superseding_writes_both_sides_and_names_what_rested_on_it() {
        let (store, dir) = fixture();
        let item = store.add_feature("demo", "Sync", "", "M", None).unwrap();
        create(
            &store,
            "demo",
            "Poll every minute",
            Adr {
                status: "accepted".into(),
                affects: vec![item.code.clone()],
                driven_by: vec![format!("{}/R-1", item.code)],
                ..Default::default()
            },
        )
        .unwrap();
        create(
            &store,
            "demo",
            "Push on write instead",
            Adr {
                status: "accepted".into(),
                affects: vec![item.code.clone()],
                ..Default::default()
            },
        )
        .unwrap();

        let out = supersede(&store, "demo", "ADR-0002", "ADR-0001").unwrap();
        assert_eq!(
            out.affected,
            vec![item.code.clone()],
            "these now rest on an overturned decision"
        );
        assert_eq!(out.driven_by, vec![format!("{}/R-1", item.code)]);

        // Both sides, in one change: whichever document a reader arrives at tells them.
        let old = get(&store, "demo", "ADR-0001").unwrap();
        let new = get(&store, "demo", "ADR-0002").unwrap();
        assert_eq!(old.superseded_by, "ADR-0002");
        assert_eq!(old.status, "superseded");
        assert!(old.is_superseded());
        assert_eq!(new.supersedes, vec!["ADR-0001".to_string()]);
        assert!(!new.is_superseded());

        // The lineage reads forwards, so why the approach changed stays legible.
        let adrs = list(&store, "demo").unwrap();
        assert_eq!(lineage(&adrs, "ADR-0002"), vec!["ADR-0001", "ADR-0002"]);
        // Superseding itself, or reversing an existing supersede, is refused.
        assert!(supersede(&store, "demo", "ADR-0002", "ADR-0002").is_err());
        assert!(supersede(&store, "demo", "ADR-0001", "ADR-0002").is_err());

        // The item's view is derived from the decisions, so it needs no second edit.
        assert_eq!(for_item(&adrs, &item.code), vec!["ADR-0002", "ADR-0001"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_decision_naming_something_absent_is_a_broken_link() {
        let (store, dir) = fixture();
        let item = store.add_feature("demo", "Sync", "", "M", None).unwrap();
        create(
            &store,
            "demo",
            "Rest on nothing",
            Adr {
                affects: vec!["FEAT-404".into()],
                driven_by: vec![format!("{}/R-9", item.code)],
                ..Default::default()
            },
        )
        .unwrap();
        let project = store.load("demo").unwrap();
        let problems = dangling(&list(&store, "demo").unwrap(), &project);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("FEAT-404"));
        assert!(problems[1].contains("no requirement R-9"));
        assert!(
            exists(&list(&store, "demo").unwrap(), "adr-0001"),
            "case is not identity"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
