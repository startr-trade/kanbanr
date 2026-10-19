//! Following the graph downward, and naming what is missing (FEAT-057).
//!
//! `trace` answers "what hangs off this?" for a goal, an item or a requirement: the requirements
//! that serve it, the tests that prove them, the documents that explain them, the decisions that
//! shaped them. The **gaps are the output**, not a footnote — a chain that merely lists what
//! exists lets a requirement with no test read as fine.
//!
//! Nothing here touches git. The board-side chain is complete on its own, so a trace still works
//! outside a checkout; the CLI adds the commit evidence when there is a repository to read.
//!
//! The traceability view is **derived every time**. A stored manifest would be a third copy of
//! links that already exist in the board, the ADRs and the commit trailers — and the copy that
//! goes stale first.

use crate::error::{CoreError, Result};
use crate::models::TestState;
use crate::{FeatureItem, Store};
use serde::{Deserialize, Serialize};

/// What a trace was asked about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Subject {
    Goal {
        id: String,
    },
    Item {
        code: String,
    },
    Requirement {
        code: String,
        id: String,
    },
    /// A board doc, by its path under `docs/` (FEAT-181).
    Doc {
        path: String,
    },
}

/// Read a subject from what the user typed: `G-2`, `FEAT-046`, `FEAT-046/R-2`, a bare `R-2`
/// when the item is known from the branch, or a doc path (`architecture/data.md`).
pub fn parse_subject(text: &str, default_item: Option<&str>) -> Option<Subject> {
    let text = text.trim();
    if text.ends_with(".md") || text.starts_with("docs/") {
        return Some(Subject::Doc {
            path: crate::refs::doc_key(text),
        });
    }
    if let Some((code, id)) = text.split_once('/') {
        return Some(Subject::Requirement {
            code: code.to_string(),
            id: id.to_string(),
        });
    }
    let upper = text.to_uppercase();
    if upper.starts_with("G-") {
        return Some(Subject::Goal { id: text.into() });
    }
    if upper.starts_with("R-") {
        return default_item.map(|code| Subject::Requirement {
            code: code.to_string(),
            id: text.to_string(),
        });
    }
    (!text.is_empty()).then(|| Subject::Item {
        code: text.to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub subject: Subject,
    /// One line describing what was asked about, for the top of the report.
    pub heading: String,
    /// The goal this reaches, and the charter purpose behind it.
    pub goal: Option<String>,
    pub purpose: Option<String>,
    pub items: Vec<TracedItem>,
    /// Documents whose front-matter or text references anything in this chain.
    pub documents: Vec<String>,
    /// What the subject's own text links to: an item's spec, a doc's body (FEAT-181).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links_out: Vec<crate::refs::Ref>,
    /// Specs and docs whose text refers to the subject: `FEAT-012 spec`, `docs/architecture/data.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links_in: Vec<String>,
    /// Decisions that affect the items, or are driven by their requirements.
    pub decisions: Vec<String>,
    /// What the chain does not have. This is the point of the command.
    pub gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracedItem {
    pub code: String,
    pub title: String,
    pub status: String,
    pub requirements: Vec<TracedRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracedRequirement {
    pub id: String,
    pub text: String,
    pub kind: String,
    /// Test name and whether it is currently green.
    pub tests: Vec<(String, bool)>,
}

/// Build the trace. `head_rev` is not consulted: whether evidence is current is the report's
/// question, and mixing it in here would make a structural view fluctuate with the last commit.
pub fn run(store: &Store, project_id: &str, subject: &Subject) -> Result<Trace> {
    // In full: specs carry the links that make document edges (FEAT-181).
    let project = store.load(project_id)?;
    let index = crate::refs::BoardIndex::of(store, &project)?;
    let charter = crate::charter::load(store, project_id)?;
    let adrs = crate::adr::list(store, project_id).unwrap_or_default();

    let (heading, goal, items): (String, Option<String>, Vec<&FeatureItem>) = match subject {
        Subject::Goal { id } => {
            let goal = charter.goal(id).ok_or_else(|| {
                CoreError::Unsupported(format!("no goal {id} in this project's charter"))
            })?;
            let serving: Vec<&FeatureItem> = project
                .features
                .iter()
                .filter(|f| {
                    f.definition
                        .as_ref()
                        .is_some_and(|d| d.goals.iter().any(|g| g == id))
                })
                .collect();
            (
                format!("{id} — {}", goal.statement.trim()),
                Some(goal.statement.trim().to_string()),
                serving,
            )
        }
        Subject::Item { code } => {
            let feature = project.feature(code)?;
            (
                format!("{code} — {}", feature.title),
                first_goal(&charter, feature),
                vec![feature],
            )
        }
        Subject::Requirement { code, id } => {
            let feature = project.feature(code)?;
            (
                format!("{code}/{id}"),
                first_goal(&charter, feature),
                vec![feature],
            )
        }
        Subject::Doc { path } => {
            if !index.docs.contains(path) {
                return Err(CoreError::Unsupported(format!(
                    "no doc {path} on this board"
                )));
            }
            let text = store.read_doc(project_id, path)?;
            let named: Vec<String> = crate::refs::find(&text, &index)
                .into_iter()
                .filter_map(|f| match f.reference {
                    crate::refs::Ref::Item { code, .. } => Some(code),
                    _ => None,
                })
                .collect();
            let items: Vec<&FeatureItem> = project
                .features
                .iter()
                .filter(|f| named.contains(&f.code))
                .collect();
            (format!("docs/{path}"), None, items)
        }
    };

    let mut gaps = Vec::new();
    let wanted_requirement = match subject {
        Subject::Requirement { id, .. } => Some(id.as_str()),
        _ => None,
    };

    let mut traced = Vec::new();
    for feature in &items {
        let definition = feature.definition.as_ref();
        if definition.is_none() {
            gaps.push(format!(
                "{} has no definition, so nothing below it can be traced",
                feature.code
            ));
        }
        if definition.is_some_and(|d| d.goals.is_empty()) {
            gaps.push(format!("{} serves no stated goal", feature.code));
        }
        let mut requirements = Vec::new();
        for requirement in definition.map(|d| d.requirements.as_slice()).unwrap_or(&[]) {
            if wanted_requirement.is_some_and(|id| !requirement.id.eq_ignore_ascii_case(id)) {
                continue;
            }
            let tests: Vec<(String, bool)> = requirement
                .tests
                .iter()
                .map(|t| (t.name.clone(), t.state == TestState::Green))
                .collect();
            if tests.is_empty() {
                gaps.push(format!(
                    "{}/{} has no test, so nothing can show it is met",
                    feature.code, requirement.id
                ));
            } else if !tests.iter().any(|(_, green)| *green) {
                gaps.push(format!(
                    "{}/{} has tests but none is green",
                    feature.code, requirement.id
                ));
            }
            // A measured quality requirement with no decision behind it is an aspiration: nothing
            // was chosen to achieve it, so nothing is protecting it.
            if matches!(requirement.kind, crate::models::RequirementKind::Nfr)
                && !adrs.iter().any(|a| {
                    a.driven_by
                        .iter()
                        .any(|d| d == &format!("{}/{}", feature.code, requirement.id))
                })
            {
                gaps.push(format!(
                    "{}/{} is a quality requirement that no decision claims to serve",
                    feature.code, requirement.id
                ));
            }
            requirements.push(TracedRequirement {
                id: requirement.id.clone(),
                text: requirement.text.clone(),
                kind: format!("{:?}", requirement.kind).to_lowercase(),
                tests,
            });
        }
        if wanted_requirement.is_some() && requirements.is_empty() {
            return Err(CoreError::Unsupported(format!(
                "{} has no requirement {}",
                feature.code,
                wanted_requirement.unwrap_or_default()
            )));
        }
        traced.push(TracedItem {
            code: feature.code.clone(),
            title: feature.title.clone(),
            status: feature.status.clone(),
            requirements,
        });
    }

    // The subject as the board spells it, so `feat-046` finds what `FEAT-046` is linked from.
    let subject = &match subject {
        Subject::Item { .. } => Subject::Item {
            code: items[0].code.clone(),
        },
        Subject::Requirement { id, .. } => Subject::Requirement {
            code: items[0].code.clone(),
            id: id.clone(),
        },
        other => other.clone(),
    };
    let (links_out, links_in) = links(store, &project, &index, subject);

    if items.is_empty() {
        gaps.push(match subject {
            Subject::Doc { path } if links_in.is_empty() => {
                format!("docs/{path} names no item, and nothing links to it")
            }
            Subject::Doc { .. } => "this doc names no item".to_string(),
            _ => "nothing on the board serves this".to_string(),
        });
    }

    // Documents that say they are about any of this, and decisions that claim these items.
    let codes: Vec<&str> = traced.iter().map(|t| t.code.as_str()).collect();
    let mut documents = documents_referencing(store, project_id, &codes, subject);
    if !matches!(subject, Subject::Doc { .. }) {
        for source in &links_in {
            if let Some(path) = source.strip_prefix("docs/")
                && !path.starts_with(crate::adr::FOLDER)
                && !documents.iter().any(|d| d == path)
            {
                documents.push(path.to_string());
            }
        }
        documents.sort();
    }
    let mut decisions: Vec<String> = codes
        .iter()
        .flat_map(|code| crate::adr::for_item(&adrs, code))
        .collect();
    decisions.sort();
    decisions.dedup();
    for id in &decisions {
        if adrs.iter().any(|a| &a.id == id && a.is_superseded()) {
            gaps.push(format!(
                "{id} has been superseded — this rests on an overturned decision"
            ));
        }
    }

    Ok(Trace {
        subject: subject.clone(),
        heading,
        goal,
        purpose: (!charter.purpose.trim().is_empty()).then(|| {
            charter
                .purpose
                .trim()
                .lines()
                .next()
                .unwrap_or("")
                .to_string()
        }),
        items: traced,
        documents,
        links_out,
        links_in,
        decisions,
        gaps,
    })
}

fn first_goal(charter: &crate::Charter, feature: &FeatureItem) -> Option<String> {
    let id = feature.definition.as_ref()?.goals.first()?;
    charter
        .goal(id)
        .map(|g| format!("{id} — {}", g.statement.trim()))
}

/// Documents whose `refs:` front-matter names something in this chain. Decisions are reported
/// separately, so they are not listed twice.
fn documents_referencing(
    store: &Store,
    project: &str,
    codes: &[&str],
    subject: &Subject,
) -> Vec<String> {
    let wanted: Vec<String> = match subject {
        Subject::Goal { id } => vec![id.clone()],
        Subject::Item { code } => vec![code.clone()],
        Subject::Requirement { code, id } => vec![format!("{code}/{id}"), code.clone()],
        Subject::Doc { .. } => return Vec::new(),
    };
    store
        .list_docs(project)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.starts_with(crate::adr::FOLDER))
        .filter(|path| {
            let text = store.read_doc(project, path).unwrap_or_default();
            crate::docs::refs_of(&text)
                .iter()
                .any(|r| wanted.iter().any(|w| w == r) || codes.iter().any(|c| r == c))
        })
        .collect()
}

/// Does a reference point at the subject? A requirement is reached by a reference to it or to its
/// item; an ADR reference reaches the ADR's doc.
fn points_at(r: &crate::refs::Ref, subject: &Subject, index: &crate::refs::BoardIndex) -> bool {
    use crate::refs::Ref;
    match (subject, r) {
        (Subject::Item { code }, Ref::Item { code: c, .. }) => c == code,
        (
            Subject::Requirement { code, id },
            Ref::Item {
                code: c,
                requirement,
            },
        ) => {
            c == code
                && requirement
                    .as_deref()
                    .is_none_or(|q| q.eq_ignore_ascii_case(id))
        }
        (Subject::Goal { id }, Ref::Goal { id: g }) => g == id,
        (Subject::Doc { path }, Ref::Doc { path: p }) => p == path,
        (Subject::Doc { path }, Ref::Adr { id }) => index.adrs.get(id) == Some(path),
        _ => false,
    }
}

/// The document edges (FEAT-181): what the subject's own text links to, and which specs and docs
/// link to it. Only references the board resolves count — a broken one is doctor's to report.
fn links(
    store: &Store,
    project: &crate::Project,
    index: &crate::refs::BoardIndex,
    subject: &Subject,
) -> (Vec<crate::refs::Ref>, Vec<String>) {
    let own_text = match subject {
        Subject::Item { code } | Subject::Requirement { code, .. } => project
            .features
            .iter()
            .find(|f| &f.code == code)
            .map(|f| f.specification.clone()),
        Subject::Doc { path } => store.read_doc(&project.id, path).ok(),
        Subject::Goal { .. } => None,
    };
    let mut out: Vec<crate::refs::Ref> = Vec::new();
    for found in crate::refs::find(own_text.as_deref().unwrap_or(""), index) {
        if index.resolves(&found.reference)
            && !points_at(&found.reference, subject, index)
            && !out.contains(&found.reference)
        {
            out.push(found.reference);
        }
    }

    let mut incoming = Vec::new();
    let refers = |text: &str| {
        crate::refs::find(text, index)
            .iter()
            .any(|f| points_at(&f.reference, subject, index))
    };
    for f in &project.features {
        let own = matches!(subject, Subject::Item { code } | Subject::Requirement { code, .. } if code == &f.code);
        if !own && refers(&f.specification) {
            incoming.push(format!("{} spec", f.code));
        }
        if let Subject::Doc { path } = subject
            && f.definition.as_ref().is_some_and(|d| {
                !d.design_doc.is_empty() && &crate::refs::doc_key(&d.design_doc) == path
            })
        {
            incoming.push(format!("{} design_doc", f.code));
        }
    }
    for path in &index.docs {
        if matches!(subject, Subject::Doc { path: p } if p == path) {
            continue;
        }
        if refers(&store.read_doc(&project.id, path).unwrap_or_default()) {
            incoming.push(format!("docs/{path}"));
        }
    }
    (out, incoming)
}

// ---- the Zachman view ----------------------------------------------------------------------

/// The six columns, in the order the framework states them.
pub const COLUMNS: [&str; 6] = ["What", "How", "Where", "When", "Who", "Why"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZachmanView {
    pub scope: String,
    /// Per column: the items that answer it, and the decisions tagged to it.
    pub cells: Vec<ZachmanCell>,
    /// Columns nothing addresses — the output that matters.
    pub gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZachmanCell {
    pub column: String,
    /// How many in-scope items answer this column.
    pub answered_by: usize,
    pub items: usize,
    pub decisions: Vec<String>,
}

/// Derive the matrix from what items and decisions already record. Nothing is stored: a grid kept
/// as data would be a second place to maintain the same six sentences.
pub fn zachman(store: &Store, project_id: &str, scope: Option<&str>) -> Result<ZachmanView> {
    let project = store.load_meta(project_id)?;
    let adrs = crate::adr::list(store, project_id).unwrap_or_default();
    let items: Vec<&FeatureItem> = project
        .features
        .iter()
        .filter(|f| match scope {
            None => true,
            Some(s) => f.code.eq_ignore_ascii_case(s) || f.milestone.eq_ignore_ascii_case(s),
        })
        .collect();
    if items.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "nothing in scope for '{}'",
            scope.unwrap_or("this project")
        )));
    }

    let mut cells = Vec::new();
    let mut gaps = Vec::new();
    for column in COLUMNS {
        let answered_by = items
            .iter()
            .filter(|f| {
                f.definition
                    .as_ref()
                    .is_some_and(|d| !d.zachman.column(column).trim().is_empty())
            })
            .count();
        let decisions: Vec<String> = adrs
            .iter()
            .filter(|a| a.zachman.iter().any(|z| z.eq_ignore_ascii_case(column)))
            .filter(|a| {
                scope.is_none() || a.affects.iter().any(|c| items.iter().any(|f| &f.code == c))
            })
            .map(|a| a.id.clone())
            .collect();
        if answered_by == 0 {
            gaps.push(format!("nothing in scope says anything about {column}"));
        } else if answered_by < items.len() {
            gaps.push(format!(
                "{} of {} items leave {column} blank",
                items.len() - answered_by,
                items.len()
            ));
        }
        cells.push(ZachmanCell {
            column: column.to_string(),
            answered_by,
            items: items.len(),
            decisions,
        });
    }
    Ok(ZachmanView {
        scope: scope.unwrap_or("the whole project").to_string(),
        cells,
        gaps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::models::{FeatureDefinition, Requirement, RequirementKind, TestRef, Zachman};

    fn fixture() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-trace-{}-{}",
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
        crate::charter::save(
            &store,
            "demo",
            &crate::Charter {
                purpose: "Keep the reasoning with the work.".into(),
                goals: vec![crate::charter::Goal {
                    id: "G-1".into(),
                    statement: "Any line of code reaches the reason it exists".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
        (store, dir)
    }

    /// FEAT-181: a link in a spec or a doc is an edge trace follows both ways — an item lists the
    /// docs that name it and what its spec links to, and a doc can be traced on its own.
    #[test]
    fn doc_links_are_trace_edges() {
        let (store, _dir) = fixture();
        let a = store.add_feature("demo", "Cart", "", "M", None).unwrap();
        let b = store
            .add_feature("demo", "Checkout", "", "M", None)
            .unwrap();
        store
            .edit_feature(
                "demo",
                &b.code,
                None,
                Some(format!(
                    "Needs {} and follows architecture/data.md.",
                    a.code
                )),
                None,
                None,
            )
            .unwrap();
        store
            .write_doc(
                "demo",
                "architecture/data.md",
                &format!("The cart ([{0}](/p/demo/feature/{0})) keeps G-1.", a.code),
            )
            .unwrap();
        assert_eq!(
            parse_subject("docs/architecture/data.md", None),
            Some(Subject::Doc {
                path: "architecture/data.md".into()
            })
        );

        let item = run(
            &store,
            "demo",
            &Subject::Item {
                code: a.code.clone(),
            },
        )
        .unwrap();
        assert_eq!(item.documents, vec!["architecture/data.md".to_string()]);
        assert_eq!(
            item.links_in,
            vec![
                format!("{} spec", b.code),
                "docs/architecture/data.md".to_string()
            ]
        );

        let checkout = run(
            &store,
            "demo",
            &Subject::Item {
                code: b.code.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            checkout.links_out,
            vec![
                crate::refs::Ref::Item {
                    code: a.code.clone(),
                    requirement: None
                },
                crate::refs::Ref::Doc {
                    path: "architecture/data.md".into()
                },
            ]
        );

        let doc = run(
            &store,
            "demo",
            &Subject::Doc {
                path: "architecture/data.md".into(),
            },
        )
        .unwrap();
        assert_eq!(doc.items.len(), 1);
        assert_eq!(doc.items[0].code, a.code);
        assert_eq!(doc.links_in, vec![format!("{} spec", b.code)]);
        assert!(
            doc.links_out
                .contains(&crate::refs::Ref::Goal { id: "G-1".into() })
        );
    }

    #[test]
    fn a_subject_is_read_from_what_the_user_typed() {
        assert_eq!(
            parse_subject("G-2", None),
            Some(Subject::Goal { id: "G-2".into() })
        );
        assert_eq!(
            parse_subject("FEAT-046", None),
            Some(Subject::Item {
                code: "FEAT-046".into()
            })
        );
        assert_eq!(
            parse_subject("FEAT-046/R-2", None),
            Some(Subject::Requirement {
                code: "FEAT-046".into(),
                id: "R-2".into()
            })
        );
        // A bare requirement only means something when the branch says which item.
        assert_eq!(parse_subject("R-2", None), None);
        assert_eq!(
            parse_subject("R-2", Some("FEAT-046")),
            Some(Subject::Requirement {
                code: "FEAT-046".into(),
                id: "R-2".into()
            })
        );
    }

    #[test]
    fn tracing_downward_reports_the_chain_and_its_gaps() {
        let (store, dir) = fixture();
        let item = store
            .add_feature("demo", "Traceability", "", "M", None)
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(FeatureDefinition {
                    goals: vec!["G-1".into()],
                    requirements: vec![
                        Requirement {
                            id: "R-1".into(),
                            text: "THE SYSTEM SHALL print the chain.".into(),
                            tests: vec![TestRef {
                                name: "trace::prints".into(),
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                        Requirement {
                            id: "R-2".into(),
                            text: "THE SYSTEM SHALL answer in under a second.".into(),
                            kind: RequirementKind::Nfr,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
            )
            .unwrap();
        // A design note that says what it is about joins the chain; one that says nothing does not.
        store
            .write_doc(
                "demo",
                "design/trace.md",
                &format!("---\nrefs: [{}]\n---\n\n# How tracing works\n", item.code),
            )
            .unwrap();
        store
            .write_doc("demo", "design/unrelated.md", "# Something else\n")
            .unwrap();

        // No git repository in sight: the board-side chain is still complete.
        let t = run(&store, "demo", &Subject::Goal { id: "G-1".into() }).unwrap();
        assert_eq!(t.items.len(), 1);
        assert_eq!(t.items[0].requirements.len(), 2);
        assert_eq!(t.documents, vec!["design/trace.md".to_string()]);
        assert!(
            t.purpose
                .as_deref()
                .unwrap()
                .starts_with("Keep the reasoning")
        );

        // The gaps are the output: an unproven test, a requirement with none, an unbacked NFR.
        let gaps = t.gaps.join("\n");
        assert!(gaps.contains("R-1 has tests but none is green"), "{gaps}");
        assert!(gaps.contains("R-2 has no test"), "{gaps}");
        assert!(
            gaps.contains("R-2 is a quality requirement that no decision claims to serve"),
            "{gaps}"
        );

        // Green evidence closes the first gap; nothing else changes.
        store
            .set_test_state(
                "demo",
                &item.code,
                "R-1",
                "trace::prints",
                TestState::Green,
                None,
            )
            .unwrap();
        let t = run(
            &store,
            "demo",
            &Subject::Requirement {
                code: item.code.clone(),
                id: "R-1".into(),
            },
        )
        .unwrap();
        assert_eq!(
            t.items[0].requirements.len(),
            1,
            "narrowed to the one asked about"
        );
        assert_eq!(
            t.items[0].requirements[0].tests,
            vec![("trace::prints".to_string(), true)]
        );
        assert!(t.gaps.is_empty(), "{:?}", t.gaps);

        // A decision that rests on this item shows up, and a superseded one is called out.
        crate::adr::create(
            &store,
            "demo",
            "Derive, never store",
            crate::adr::Adr {
                status: "accepted".into(),
                affects: vec![item.code.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        let t = run(
            &store,
            "demo",
            &Subject::Item {
                code: item.code.clone(),
            },
        )
        .unwrap();
        assert_eq!(t.decisions, vec!["ADR-0001".to_string()]);
        crate::adr::create(
            &store,
            "demo",
            "Store it after all",
            crate::adr::Adr {
                status: "accepted".into(),
                affects: vec![item.code.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        crate::adr::supersede(&store, "demo", "ADR-0002", "ADR-0001").unwrap();
        let t = run(
            &store,
            "demo",
            &Subject::Item {
                code: item.code.clone(),
            },
        )
        .unwrap();
        assert!(
            t.gaps
                .iter()
                .any(|g| g.contains("ADR-0001 has been superseded")),
            "{:?}",
            t.gaps
        );

        // An unknown requirement is an error, not an empty answer.
        assert!(
            run(
                &store,
                "demo",
                &Subject::Requirement {
                    code: item.code.clone(),
                    id: "R-9".into()
                }
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_zachman_view_reports_the_columns_nothing_addresses() {
        let (store, dir) = fixture();
        let item = store
            .add_feature("demo", "Half-answered", "", "M", None)
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(FeatureDefinition {
                    goals: vec!["G-1".into()],
                    zachman: Zachman {
                        what: "the chain".into(),
                        why: "so the reason outlives the session".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            )
            .unwrap();
        crate::adr::create(
            &store,
            "demo",
            "How the chain is built",
            crate::adr::Adr {
                status: "accepted".into(),
                affects: vec![item.code.clone()],
                zachman: vec!["How".into()],
                ..Default::default()
            },
        )
        .unwrap();

        let view = zachman(&store, "demo", Some(&item.code)).unwrap();
        let answered: Vec<(&str, usize)> = view
            .cells
            .iter()
            .map(|c| (c.column.as_str(), c.answered_by))
            .collect();
        assert_eq!(
            answered,
            vec![
                ("What", 1),
                ("How", 0),
                ("Where", 0),
                ("When", 0),
                ("Who", 0),
                ("Why", 1)
            ]
        );
        // A decision tagged to a column is listed there even when no item fills it in.
        let how = view.cells.iter().find(|c| c.column == "How").unwrap();
        assert_eq!(how.decisions, vec!["ADR-0001".to_string()]);
        // And the blanks are named, because the gaps are the output.
        assert!(
            view.gaps.iter().any(|g| g.contains("about Where")),
            "{:?}",
            view.gaps
        );
        assert!(!view.gaps.iter().any(|g| g.contains("about What")));
        assert!(zachman(&store, "demo", Some("FEAT-404")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
