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
