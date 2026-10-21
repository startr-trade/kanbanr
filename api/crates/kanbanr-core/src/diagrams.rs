//! Mermaid diagrams on the board (FEAT-200): the lint run when a spec or a markdown doc is written,
//! by doctor, and at a gate that asks for `diagrams_valid`; strict mode; and the blocks the review
//! page parses with Pele before Approve.
//!
//! The lint itself is the standalone `mermaid-lint` crate, which has no kanbanr in it and follows
//! Pele 1.7.0: it flags only a diagram Pele's `parse` rejects, never one Pele accepts. This module
//! is what kanbanr adds — names for a board's files, the project's strict setting, an item's spec
//! and `affects_docs`, and the review route's response.

use serde::Serialize;

pub use mermaid_lint::{
    Finding as DiagramFinding, LintRule, MAX_FINDINGS_PER_FILE, is_markdown_path, lint_markdown,
    lint_source,
};

/// One fenced `mermaid` block of a board file, with where it is for a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DiagramBlock {
    /// What the block is in: `"FEAT-012 spec"` or `"docs/<key>"`.
    pub file: String,
    /// The doc key, for a link to it; `None` for a spec.
    pub doc: Option<String>,
    /// 1-based index among the file's mermaid blocks.
    pub block: usize,
    /// The file line (1-based) of the block's first source line.
    pub line: usize,
    /// The diagram's source, between the fences.
    pub source: String,
}

/// Every mermaid block of `markdown`, named `file`, with `doc` as its doc key when it is a doc.
pub fn blocks(file: &str, doc: Option<&str>, markdown: &str) -> Vec<DiagramBlock> {
    mermaid_lint::blocks(markdown)
        .into_iter()
        .map(|b| DiagramBlock {
            file: file.to_string(),
            doc: doc.map(str::to_string),
            block: b.block,
            line: b.line,
            source: b.source,
        })
        .collect()
}

/// Lint text about to be written. On a project that checks diagrams strictly, a problem refuses the
/// write, naming each one; otherwise the problems come back for the caller to report as warnings.
pub fn guard(
    config: &crate::config::DiagramsConfig,
    file: &str,
    markdown: &str,
) -> crate::Result<Vec<DiagramFinding>> {
    let findings = lint_markdown(file, markdown);
    if config.strict && !findings.is_empty() {
        let each: Vec<String> = findings.iter().map(DiagramFinding::describe).collect();
        return Err(crate::CoreError::InvalidDiagram(format!(
            "{file} has {} diagram problem(s), and this project checks diagrams strictly: {} — fix \
             them, or turn strict checking off with `kanbanr config diagrams --strict off`",
            findings.len(),
            each.join("; ")
        )));
    }
    Ok(findings)
}

/// The lint's findings for an item: its spec as `"{code} spec"`, then each doc it names in
/// `affects_docs` as `"docs/{key}"`, read from `docs_dir` on every call. A key that climbs out of
/// the folder or is absolute is skipped, and so is a doc that is missing or unreadable —
/// `docs_updated` and doctor report those.
pub fn item_findings(
    feature: &crate::FeatureItem,
    docs_dir: Option<&std::path::Path>,
) -> Vec<DiagramFinding> {
    let mut out = lint_markdown(&format!("{} spec", feature.code), &feature.specification);
    let Some(docs) = docs_dir else {
        return out;
    };
    for key in affects_doc_keys(feature) {
        if let Ok(text) = std::fs::read_to_string(docs.join(&key)) {
            out.extend(lint_markdown(&format!("docs/{key}"), &text));
        }
    }
    out
}

/// The board keys of the docs an item names in `affects_docs`, without any that climb out of the
/// docs folder or are absolute.
fn affects_doc_keys(feature: &crate::FeatureItem) -> Vec<String> {
    let declared = feature
        .definition
        .as_ref()
        .map(|d| d.affects_docs.as_slice())
        .unwrap_or(&[]);
    declared
        .iter()
        .map(|d| crate::readiness::affects_doc_key(d))
        .filter(|key| {
            let path = std::path::Path::new(key);
            !path.is_absolute()
                && !key.starts_with('/')
                && !path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
        })
        .collect()
}

/// The diagrams of one item — every block of its spec and of each doc it names in `affects_docs` —
/// with the lint's findings (FEAT-200). The review page parses each block with Pele and keeps
/// Approve disabled until all of them draw.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemDiagrams {
    pub code: String,
    pub blocks: Vec<DiagramBlock>,
    #[cfg_attr(feature = "schema", schemars(with = "Vec<schema::DiagramFinding>"))]
    pub findings: Vec<DiagramFinding>,
}

impl ItemDiagrams {
    /// Read the item's spec and its docs now. Docs are read through the store, which checks the
    /// path; one that is missing or unreadable is left out.
    pub fn load(store: &crate::Store, p: &str, code: &str) -> crate::Result<Self> {
        let project = store.load_meta(p)?;
        let mut feature = project.feature(code)?.clone();
        feature.specification = store.feature_spec(p, code)?;
        let spec_file = format!("{code} spec");
        let mut out = blocks(&spec_file, None, &feature.specification);
        for key in affects_doc_keys(&feature) {
            if let Ok(text) = store.read_doc(p, &key) {
                out.extend(blocks(&format!("docs/{key}"), Some(&key), &text));
            }
        }
        let docs = store.project_dir(p).join("docs");
        Ok(ItemDiagrams {
            code: feature.code.clone(),
            blocks: out,
            findings: item_findings(&feature, Some(&docs)),
        })
    }
}

/// How the lint's types are described in the published API. `mermaid-lint` stays free of schema
/// machinery, so these mirror its serialised shape: field for field, rule by its snake_case name.
#[cfg(feature = "schema")]
mod schema {
    #![allow(dead_code)]

    /// What the lint found wrong.
    #[derive(schemars::JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum LintRule {
        EmptyBlock,
        UnknownType,
        SemicolonInText,
        LabelPunctuation,
        UnbalancedLabel,
        EndAsId,
        UnbalancedBlock,
        UnknownStatement,
    }

    /// One problem in one diagram, located in its file.
    #[derive(schemars::JsonSchema)]
    pub struct DiagramFinding {
        pub file: String,
        /// 1-based index among the file's mermaid blocks.
        pub block: usize,
        /// The file line (1-based) the problem is on.
        pub line: usize,
        pub rule: LintRule,
        pub message: String,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every diagram in the book, the README and the skill draws in Pele (`check:docs`), so the lint
    /// must pass every one of them: a wide check that it flags nothing Pele accepts.
    #[test]
    fn the_books_diagrams_pass_the_lint() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut files = vec![root.join("README.md"), root.join("skill/kanbanr/SKILL.md")];
        let mut dirs = vec![root.join("docs/src")];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p.extension().is_some_and(|x| x == "md") {
                    files.push(p);
                }
            }
        }
        let mut seen = 0;
        let mut found = Vec::new();
        for p in files {
            let text = std::fs::read_to_string(&p).unwrap();
            seen += mermaid_lint::blocks(&text).len();
            found.extend(lint_markdown(&p.display().to_string(), &text));
        }
        assert!(
            seen > 0,
            "the book has diagrams; zero means the paths are wrong"
        );
        assert!(
            found.is_empty(),
            "{}",
            found
                .iter()
                .map(|f| f.describe())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// FEAT-200 R-14: the review page's route serves every block of the spec and of each doc in
    /// `affects_docs`, each doc block carrying its key for the link, with the lint's findings; a
    /// doc that is missing is left out.
    #[test]
    fn item_diagrams_carry_spec_and_doc_blocks() {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-item-diagrams-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = crate::Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        let spec = "```mermaid\nflowchart LR\n  A --> B\n```\n";
        let f = store.add_feature("demo", "T", spec, "M", None).unwrap();
        let definition = crate::models::FeatureDefinition {
            statement: "s".into(),
            affects_docs: vec!["arch/flow".into(), "arch/missing".into()],
            ..Default::default()
        };
        store
            .set_feature_definition("demo", &f.code, Some(definition))
            .unwrap();
        store
            .write_doc(
                "demo",
                "arch/flow.md",
                "# F\n\n```mermaid\nsequenceDiagram\n  junk\n```\n",
            )
            .unwrap();

        let d = ItemDiagrams::load(&store, "demo", &f.code).unwrap();
        assert_eq!(d.code, f.code);
        assert_eq!(d.blocks.len(), 2, "{:?}", d.blocks);
        assert_eq!(d.blocks[0].file, format!("{} spec", f.code));
        assert_eq!(d.blocks[0].doc, None);
        assert_eq!(d.blocks[1].file, "docs/arch/flow.md");
        assert_eq!(d.blocks[1].doc.as_deref(), Some("arch/flow.md"));
        assert_eq!((d.blocks[1].block, d.blocks[1].line), (1, 4));
        assert_eq!(d.findings.len(), 1, "{:?}", d.findings);
        assert_eq!(d.findings[0].file, "docs/arch/flow.md");
        assert_eq!(d.findings[0].rule, LintRule::UnknownStatement);
        assert!(ItemDiagrams::load(&store, "demo", "FEAT-999").is_err());

        // A key that climbs out of the docs folder is never read, even where the store's own
        // validation was bypassed.
        let mut item = store
            .load("demo")
            .unwrap()
            .feature(&f.code)
            .unwrap()
            .clone();
        if let Some(d) = item.definition.as_mut() {
            d.affects_docs = vec!["../outside".into()];
        }
        std::fs::write(
            store.project_dir("demo").join("outside.md"),
            "```mermaid\nx\n```\n",
        )
        .unwrap();
        let docs = store.project_dir("demo").join("docs");
        assert!(item_findings(&item, Some(&docs)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
