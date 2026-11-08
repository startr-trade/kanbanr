//! DEF-025: every board write goes through `kanbanr_core::atomic::write`.
//!
//! `a_save_leaves_no_temporary_file_behind` runs one operation of each family and walks the board.
//! The scan below proves only that nothing else bypasses the function (ADR-0008: the property
//! itself, that a reader never sees a partial file, is proved by
//! `atomic::tests::a_reader_never_sees_a_partial_file`).

use std::path::{Path, PathBuf};

use kanbanr_core::Store;
use kanbanr_core::config::ProjectConfig;

/// Writes that are not converted. Each names the file (path suffix), the start of the trimmed line,
/// and why it stays: an exception is a decision.
const EXCEPTIONS: &[(&str, &str, &str)] = &[
    (
        "kanbanr-core/src/git.rs",
        "let _ = std::fs::write(dir.join(UNPUSHED_FILE)",
        "an empty machine-local marker; there is no content to half-write",
    ),
    (
        "kanbanr-core/src/write.rs",
        "let file = std::fs::OpenOptions::new()",
        "the board lock file holds no content and is opened, not written",
    ),
    (
        "kanbanr-core/src/atomic.rs",
        "let mut file = std::fs::OpenOptions::new()",
        "it is the one write",
    ),
    (
        "kanbanr-cli/src/self_update.rs",
        "std::fs::write(&staged, bytes)",
        "replaces the running binary by staging and renaming it, keeping the old one aside; the executable bit is set before the rename",
    ),
    (
        "kanbanr-cli/src/skill.rs",
        "std::fs::write(&path, bytes)",
        "writes a whole folder into a staging folder, then renames the folder",
    ),
    (
        "kanbanr-cli/src/skill.rs",
        "std::fs::write(staging.join(STAMP)",
        "part of the same staging folder",
    ),
    (
        "kanbanr-cli/src/scm.rs",
        "std::fs::write(&path, script(command, name))",
        "writes a git hook script, then sets its mode; the old hook was set aside first (.pre-kanbanr)",
    ),
    (
        "kanbanr-cli/src/editor.rs",
        "std::fs::write(&path, &bytes)",
        "a scratch copy under the system temporary folder",
    ),
    (
        "kanbanr-cli/src/main.rs",
        "std::fs::write(&copy, text)",
        "a scratch copy under the system temporary folder",
    ),
    (
        "kanbanr-cli/build.rs",
        "std::fs::write(dest, out)",
        "build-time tool, not the product",
    ),
    (
        "kanbanr-server/build.rs",
        "std::fs::write(out.join(",
        "build-time tool, not the product",
    ),
    (
        "kanbanr-core/examples/openapi.rs",
        "} else if let Err(e) = std::fs::write(path, text)",
        "developer tool, not the product",
    ),
];

const PATTERNS: &[&str] = &[
    "fs::write(",
    "File::create(",
    "OpenOptions::new(",
    "fs::copy(",
];

fn crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Remove every item that follows a `#[cfg(test)]` line, counting braces from the item's first
/// `{` (or to the first `;` when it has no body). Braces inside strings, raw strings, characters
/// and comments are not counted. Panics, naming the file, if the item does not close.
fn strip_test_items(name: &str, src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut line_start = true;
    while i < b.len() {
        if line_start {
            let end = b[i..]
                .iter()
                .position(|c| *c == '\n')
                .map_or(b.len(), |p| i + p);
            let line: String = b[i..end].iter().collect();
            if matches!(line.trim(), "#[cfg(test)]" | "#[cfg(all(test, unix))]") {
                i = skip_item(name, &b, end);
                continue;
            }
        }
        out.push(b[i]);
        line_start = b[i] == '\n';
        i += 1;
    }
    out
}

/// Index just after the item that starts at `i`.
fn skip_item(name: &str, b: &[char], mut i: usize) -> usize {
    let (mut depth, mut opened) = (0i32, false);
    while i < b.len() {
        match b[i] {
            '/' if b.get(i + 1) == Some(&'/') => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            'r' if matches!(b.get(i + 1), Some('"') | Some('#'))
                && (i == 0 || !b[i - 1].is_alphanumeric() && b[i - 1] != '_') =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while b.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if b.get(j) == Some(&'"') {
                    j += 1;
                    loop {
                        assert!(j < b.len(), "{name}: a raw string does not close");
                        if b[j] == '"' && (0..hashes).all(|k| b.get(j + 1 + k) == Some(&'#')) {
                            j += 1 + hashes;
                            break;
                        }
                        j += 1;
                    }
                    i = j;
                    continue;
                }
            }
            '"' => {
                i += 1;
                while i < b.len() && b[i] != '"' {
                    i += if b[i] == '\\' { 2 } else { 1 };
                }
                i += 1;
                continue;
            }
            '\'' => {
                // A character literal ('{', '\n'), not a lifetime ('a).
                if b.get(i + 1) == Some(&'\\') {
                    i += 2;
                    while i < b.len() && b[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                    continue;
                } else if b.get(i + 2) == Some(&'\'') {
                    i += 3;
                    continue;
                }
            }
            '{' => {
                depth += 1;
                opened = true;
            }
            '}' => {
                depth -= 1;
                if opened && depth <= 0 {
                    return i + 1;
                }
            }
            ';' if !opened => return i + 1,
            _ => {}
        }
        i += 1;
    }
    panic!("{name}: braces after #[cfg(test)] do not close before the end of the file");
}

/// Every hit: (path relative to crates, 1-based line, trimmed line), tests removed.
fn hits() -> Vec<(String, usize, String)> {
    let root = crates();
    let mut files = Vec::new();
    for d in [
        "kanbanr-core/src",
        "kanbanr-cli/src",
        "kanbanr-server/src",
        "kanbanr-core/examples",
    ] {
        rust_files(&root.join(d), &mut files);
    }
    files.push(root.join("kanbanr-cli/build.rs"));
    files.push(root.join("kanbanr-server/build.rs"));
    let mut found = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let src = std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("{rel}: {e}"));
        let stripped = strip_test_items(&rel, &src);
        assert!(
            !stripped.contains("#[test]"),
            "{rel}: a test item survived the strip"
        );
        for (i, line) in stripped.lines().enumerate() {
            if PATTERNS.iter().any(|p| line.contains(p)) {
                found.push((rel.clone(), i + 1, line.trim().to_string()));
            }
        }
    }
    found
}

fn is_listed(rel: &str, line: &str, (suffix, starts, _): &(&str, &str, &str)) -> bool {
    rel.ends_with(suffix) && line.contains(starts)
}

#[test]
fn no_other_write_path_remains() {
    let bad: Vec<String> = hits()
        .into_iter()
        .filter(|(rel, _, line)| !EXCEPTIONS.iter().any(|e| is_listed(rel, line, e)))
        .map(|(rel, n, line)| {
            format!("{rel}:{n}: use kanbanr_core::atomic::write, or add this line to EXCEPTIONS with the reason: {line}")
        })
        .collect();
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

#[test]
fn every_listed_exception_still_exists() {
    let all = hits();
    let stale: Vec<&str> = EXCEPTIONS
        .iter()
        .filter(|e| !all.iter().any(|(rel, _, line)| is_listed(rel, line, e)))
        .map(|e| e.1)
        .collect();
    assert!(stale.is_empty(), "exceptions that match nothing: {stale:?}");
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn a_save_leaves_no_temporary_file_behind() {
    let dir = std::env::temp_dir().join(format!("kanbanr-atomic-board-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::new(dir.clone());
    let scrum = kanbanr_core::config::preset("scrum").unwrap();
    let mut config = ProjectConfig::default_for("shop");
    config.statuses = scrum.statuses.clone();
    config.default_state = scrum.default_state.clone();
    config.displayed_states = scrum.displayed_states.clone();
    config.terminal_states = scrum.terminal_states.clone();
    config.no_op_states = scrum.no_op_states.clone();
    config.transitions = scrum.transitions.clone();
    store.init_project("shop", config).unwrap();
    kanbanr_core::dispatch::dispatch(
        &store,
        "PUT",
        "/projects/shop/config/cadence",
        Some(&serde_json::json!({"sprints": true, "estimate_unit": "points"})),
    )
    .unwrap();
    store
        .add_milestone("shop", "M", "", vec![], Some("M".into()))
        .unwrap();
    let item = store.add_feature("shop", "A", "spec", "M", None).unwrap();
    store.write_doc("shop", "notes/a.md", "a note").unwrap();
    kanbanr_core::sprints::add(&store, "shop", "2026-10-05", Some(5), "", None, "").unwrap();
    kanbanr_core::dispatch::dispatch(
        &store,
        "PUT",
        "/projects/shop/config/cadence",
        Some(&serde_json::json!({"sprints": true, "releases": true, "estimate_unit": "points"})),
    )
    .unwrap();
    kanbanr_core::releases::add(&store, "shop", "0.1.0", "", "").unwrap();
    kanbanr_core::lessons::add(
        &store,
        "shop",
        kanbanr_core::lessons::Lesson {
            lesson: "write files whole".into(),
            from_item: item.code.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    let charter = kanbanr_core::Charter {
        purpose: "p".into(),
        ..Default::default()
    };
    kanbanr_core::charter::save(&store, "shop", &charter).unwrap();
    kanbanr_core::activity::append(&dir, "shop", "me", "did a thing", Some(&item.code));

    let mut files = Vec::new();
    walk(&dir, &mut files);
    assert!(files.len() > 5, "the board has files: {files:?}");
    let temp: Vec<_> = files
        .iter()
        .filter(|p| {
            p.file_name().is_some_and(|n| {
                n.to_string_lossy()
                    .ends_with(kanbanr_core::atomic::TEMP_SUFFIX)
            })
        })
        .collect();
    assert!(temp.is_empty(), "temporary files left: {temp:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
