//! Project documentation tree: nested folders (each with a name + short description) holding
//! markdown files. Folder metadata is stored in a `_folder.yaml` file within each folder.

use serde::{Deserialize, Serialize};

/// The metadata file name placed inside a documentation folder.
pub const FOLDER_META: &str = "_folder.yaml";

/// Stored folder metadata (`_folder.yaml`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// A documentation file leaf.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocFile {
    /// Path relative to the project docs root (forward-slashed), e.g. `design/customer/overview.md`.
    pub path: String,
    /// Display title (derived from the file name).
    pub title: String,
}

/// A documentation folder node (renders as a tile with sub-folders + documents as links).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocFolder {
    /// Path relative to the docs root (empty string for the root), forward-slashed.
    pub path: String,
    pub name: String,
    pub description: String,
    pub folders: Vec<DocFolder>,
    pub docs: Vec<DocFile>,
}

/// Split a document's YAML front-matter from its body (FEAT-057). Front-matter is how a document
/// joins the graph — an ADR's `affects`, a design note's `refs` — without a second index to keep
/// in step with the prose.
///
/// A document with no front-matter is the normal case, not an error: most documents are prose.
pub fn split_front_matter(text: &str) -> (Option<&str>, &str) {
    let rest = match text.strip_prefix("---\n") {
        Some(rest) => rest,
        None => return (None, text),
    };
    match rest.split_once("\n---\n") {
        Some((front, body)) => (Some(front), body.trim_start_matches('\n')),
        // An opening fence with no close is malformed; treat the whole thing as prose rather than
        // swallowing the document.
        None => (None, text),
    }
}

/// Re-attach front-matter to a body.
pub fn with_front_matter(front: &str, body: &str) -> String {
    format!("---\n{}\n---\n\n{}", front.trim_end(), body.trim_start())
}

/// What a document says it is about: `refs: [FEAT-046, FEAT-046/R-2, G-2]` in its front-matter.
pub fn refs_of(text: &str) -> Vec<String> {
    let Some(front) = split_front_matter(text).0 else {
        return Vec::new();
    };
    #[derive(serde::Deserialize, Default)]
    struct Refs {
        #[serde(default)]
        refs: Vec<String>,
    }
    serde_yaml::from_str::<Refs>(front)
        .unwrap_or_default()
        .refs
        .into_iter()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect()
}

/// Turn a file stem into a friendly title: `getting-started` -> `Getting Started`.
pub fn title_from_stem(stem: &str) -> String {
    stem.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
