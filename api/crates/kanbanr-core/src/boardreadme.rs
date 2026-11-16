//! The README every board carries (FEAT-201).
//!
//! A board is a git repository of YAML and markdown; cloned, or opened on GitHub, it shows folders
//! and nothing that says what it is or how to see it as a board. This writes `README.md` at the
//! board's root: what the repository is, its projects, and the steps from nothing to the visual
//! board — install kanbanr, clone, `kanbanr serve`, open the page.
//!
//! The generated text sits between markers; anything the board's owner writes outside them is
//! theirs and is never touched. It is written when projects are created, renamed or deleted, and
//! on request (`kanbanr readme`).

use crate::{Result, Store};

pub const START: &str = "<!-- kanbanr:readme:start -->";
pub const END: &str = "<!-- kanbanr:readme:end -->";
pub const FILE: &str = "README.md";

const INSTALL_UNIX: &str =
    "curl -fsSL https://github.com/startr-trade/kanbanr/releases/latest/download/install.sh | sh";
const INSTALL_WINDOWS: &str =
    "irm https://github.com/startr-trade/kanbanr/releases/latest/download/install.ps1 | iex";
const INSTALL_CARGO: &str =
    "cargo install --locked --git https://github.com/startr-trade/kanbanr kanbanr-cli";
const DOCS: &str = "https://startr-trade.github.io/kanbanr/";

const KANBANR: &str = "https://github.com/startr-trade/kanbanr";

/// One project as the README shows it.
struct Entry {
    id: String,
    title: String,
    description: String,
    repository: Option<crate::config::CodeRepository>,
}

impl Entry {
    /// The repository as a link, or its name alone when private; `None` with no repository.
    fn code(&self) -> Option<String> {
        let repo = self.repository.as_ref()?;
        let name = repo.name();
        Some(match repo.web() {
            Some(web) => format!("[{name}]({})", web.split_whitespace().collect::<String>()),
            None => name,
        })
    }
}

/// The generated block, markers included, for the board in `store`.
pub fn render(store: &Store) -> Result<String> {
    let mut projects: Vec<Entry> = Vec::new();
    for id in store.list_projects()? {
        if let Ok(p) = store.load_meta(&id) {
            let name = p.config.name.trim();
            let title = if name.is_empty() || name == id {
                id.clone()
            } else {
                name.to_string()
            };
            projects.push(Entry {
                id,
                title,
                description: p.config.description.trim().to_string(),
                repository: p.config.repository.clone(),
            });
        }
    }
    let first = projects
        .first()
        .map(|e| e.id.clone())
        .unwrap_or_else(|| "<project>".to_string());
    let board = store
        .data_dir()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the board".into());
    let mut out = format!("{START}\n");
    let lead = if let [only] = projects.as_slice() {
        out.push_str(&format!("# {}: plan and evidence\n\n", only.title));
        let about = match (&only.repository, only.code()) {
            (Some(repo), Some(link)) if repo.web().is_some() => link,
            (Some(_), Some(name)) => format!("**{}** (`{name}`)", only.title),
            _ => format!("**{}**", only.title),
        };
        format!(
            "This repository is the board of {about}: its plan, its reasoning and the evidence for \
             its work, kept as plain YAML and markdown."
        )
    } else {
        out.push_str(&format!("# {board}: plan and evidence\n\n"));
        "This repository is a board: the plan, the reasoning and the evidence for the projects \
         below, kept as plain YAML and markdown."
            .to_string()
    };
    out.push_str(&lead);
    out.push_str(&format!(
        "\n\nIt is kept with [kanbanr]({KANBANR}), which reads it and shows it as a board; you can \
         read the files too.\n\n"
    ));
    out.push_str("## Projects\n\n");
    if projects.is_empty() {
        out.push_str("No projects yet.\n\n");
    }
    for e in &projects {
        let mut line = format!("- **{}**", e.title);
        if e.title != e.id {
            line.push_str(&format!(" (`{}`)", e.id));
        }
        if let Some(code) = e.code() {
            line.push_str(&format!(" — code: {code}"));
        }
        if !e.description.is_empty() {
            line.push_str(&format!(": {}", e.description));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&format!(
        "\n## See it as a board\n\n\
         1. Install kanbanr:\n\n   \
         ```sh\n   # macOS and Linux\n   {INSTALL_UNIX}\n   ```\n\n   \
         ```powershell\n   # Windows\n   {INSTALL_WINDOWS}\n   ```\n\n   \
         Or, with a Rust toolchain: `{INSTALL_CARGO}`\n\n\
         2. Clone this repository, and start the monitor over it:\n\n   \
         ```sh\n   git clone <this repository's URL> board\n   kanbanr serve --data-dir board\n   ```\n\n\
         3. Open <http://127.0.0.1:8080>. The monitor is read-only unless it was started with \
         `--allow-writes`.\n\n\
         ## Or in the terminal\n\n\
         ```sh\nkanbanr board --data-dir board --project {first}\n\
         kanbanr feature show <CODE> --data-dir board --project {first}\n```\n\n\
         ## Changing it\n\n\
         kanbanr is the one writer: change the board with the `kanbanr` command (or the monitor \
         started with `--allow-writes`), never by editing the YAML by hand. Changes reach others \
         through this repository's git remote.\n\n\
         More: the [kanbanr guide]({DOCS}).\n"
    ));
    out.push_str(&format!(
        "\n_This section is generated by kanbanr; text outside it is yours._\n{END}\n"
    ));
    Ok(out)
}

/// `existing` with its generated block replaced by `block` — or, with no block yet, `block`
/// appended once (or on its own, when there is no README). Text outside the markers is kept.
pub fn apply(existing: Option<&str>, block: &str) -> String {
    let Some(text) = existing.filter(|t| !t.trim().is_empty()) else {
        return block.to_string();
    };
    match (text.find(START), text.find(END)) {
        (Some(s), Some(e)) if e > s => {
            let after = &text[e + END.len()..];
            let after = after.strip_prefix('\n').unwrap_or(after);
            format!("{}{block}{after}", &text[..s])
        }
        _ => {
            let sep = if text.ends_with("\n\n") {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{text}{sep}{block}")
        }
    }
}

/// Write the board's README, keeping its owner's text. Returns whether it changed.
pub fn write(store: &Store) -> Result<bool> {
    let path = store.data_dir().join(FILE);
    let existing = std::fs::read_to_string(&path).ok();
    let next = apply(existing.as_deref(), &render(store)?);
    if existing.as_deref() == Some(next.as_str()) {
        return Ok(false);
    }
    crate::atomic::write(&path, next)?;
    Ok(true)
}

/// Is the generated block out of date: the markers are there, and the text between them is not
/// what [`render`] would write now? (doctor asks.)
pub fn stale(store: &Store) -> bool {
    let Ok(text) = std::fs::read_to_string(store.data_dir().join(FILE)) else {
        return false;
    };
    let (Some(s), Some(e)) = (text.find(START), text.find(END)) else {
        return false;
    };
    if e <= s {
        return false;
    }
    let Ok(now) = render(store) else {
        return false;
    };
    let have = &text[s..e + END.len()];
    have.trim_end() != now.trim_end()
}

/// Does the board carry the generated README? (doctor asks.)
pub fn present(store: &Store) -> bool {
    std::fs::read_to_string(store.data_dir().join(FILE)).is_ok_and(|t| t.contains(START))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("kanbanr-readme-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Store::new(dir.clone()), dir)
    }

    /// FEAT-201 R-1: the README names every project and takes a reader from nothing to the board:
    /// the installers, `kanbanr serve --data-dir`, the address, and that it is read-only.
    #[test]
    fn readme_walks_a_reader_to_the_board() {
        let (store, dir) = store("walk");
        let mut shop = crate::ProjectConfig::for_new_project("shop");
        shop.description = "Carts that survive a closed tab".into();
        store.init_project("shop", shop).unwrap();
        store
            .init_project("ops", crate::ProjectConfig::for_new_project("ops"))
            .unwrap();
        let text = std::fs::read_to_string(dir.join(FILE)).expect("written with the projects");
        for want in [
            "**shop**: Carts that survive a closed tab",
            "**ops**",
            INSTALL_UNIX,
            INSTALL_WINDOWS,
            INSTALL_CARGO,
            "kanbanr serve --data-dir board",
            "http://127.0.0.1:8080",
            "read-only unless it was started with `--allow-writes`",
            "never by editing the YAML by hand",
        ] {
            assert!(text.contains(want), "missing {want:?}:\n{text}");
        }
        store.delete_project("ops").unwrap();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(
            !text.contains("**ops**"),
            "a deleted project leaves the list:\n{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-201 R-2: only the text between the markers is the generator's.
    #[test]
    fn owner_text_is_kept() {
        let block = format!("{START}\ngenerated v1\n{END}\n");
        let newer = format!("{START}\ngenerated v2\n{END}\n");
        let owned = format!("# Our team board\n\nAsk Ada for access.\n\n{block}\nFooter by us.\n");
        let refreshed = apply(Some(&owned), &newer);
        assert_eq!(
            refreshed,
            format!("# Our team board\n\nAsk Ada for access.\n\n{newer}\nFooter by us.\n")
        );
        // A README without markers gets the block once, and a refresh does not add it again.
        let theirs = "# Notes\n\nOurs.\n";
        let once = apply(Some(theirs), &block);
        assert_eq!(once, format!("# Notes\n\nOurs.\n\n{block}"));
        assert_eq!(apply(Some(&once), &block), once);
        assert_eq!(apply(None, &block), block);
    }

    /// DEF-053 R-1: the README leads with the project and links its code repository; kanbanr is
    /// named after it.
    #[test]
    fn readme_leads_with_the_project_and_links_its_repository() {
        let (shop_store, dir) = store("lead");
        let mut shop = crate::ProjectConfig::for_new_project("shop");
        shop.name = "Shop".into();
        shop.description = "Carts".into();
        shop.repository = Some(crate::config::CodeRepository {
            url: "git@git.example.invalid:acme/shop.git".into(),
            ..Default::default()
        });
        shop_store.init_project("shop", shop).unwrap();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some(START));
        assert_eq!(lines.next(), Some("# Shop: plan and evidence"));
        let link = "[acme/shop](https://git.example.invalid/acme/shop)";
        let at = text.find(link).expect("the repository is linked");
        assert!(
            text.find("[kanbanr](").unwrap() > at,
            "kanbanr is named after the project:\n{text}"
        );

        // Private: named, never linked.
        let (priv_store, priv_dir) = store("lead-private");
        let mut hidden = crate::ProjectConfig::for_new_project("shop");
        hidden.repository = Some(crate::config::CodeRepository {
            url: "git@git.example.invalid:acme/shop.git".into(),
            private: true,
            ..Default::default()
        });
        priv_store.init_project("shop", hidden).unwrap();
        let text = std::fs::read_to_string(priv_dir.join(FILE)).unwrap();
        assert!(text.contains("acme/shop"));
        assert!(!text.contains("https://git.example.invalid/acme/shop"));
        let _ = std::fs::remove_dir_all(&priv_dir);

        // A second project with no repository: the board's folder names the heading.
        shop_store
            .init_project("ops", crate::ProjectConfig::for_new_project("ops"))
            .unwrap();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let folder = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            text.contains(&format!("# {folder}: plan and evidence")),
            "{text}"
        );
        let ops = text.lines().find(|l| l.contains("**ops**")).unwrap();
        assert!(!ops.contains("code:"));
        assert!(
            text.lines()
                .any(|l| l.contains("**Shop**") && l.contains("code:"))
        );
        let _ = std::fs::remove_dir_all(&dir);

        // An empty board.
        let (empty_store, dir) = store("lead-empty");
        let text = render(&empty_store).unwrap();
        let folder = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(text.contains(&format!("# {folder}: plan and evidence")));
        assert!(text.contains("No projects yet."));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
