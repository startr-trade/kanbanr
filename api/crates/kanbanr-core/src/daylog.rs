//! A raw log kept as one file per day (FEAT-066).
//!
//! Both the activity changelog and the event log used to be a single file truncated to its most
//! recent 200 entries. That is data loss dressed as housekeeping: it deleted raw history the
//! charter says is never discarded, and it silently defeated the retrospective's own fallback —
//! `retro` asked for a thousand entries to reconstruct pre-history waves and could never receive
//! more than two hundred, so an old wave reported "no recorded moves" when its moves had simply
//! been trimmed away.
//!
//! So: **a folder per log, a file per day.** An append touches only today's file, which stays small
//! whatever the project's age, so a write no longer rewrites history and a commit's diff is one
//! day's entries. Reads take a limit and walk the days newest-first: the **view** is bounded, the
//! data never is.
//!
//! Boards written by an older version keep a single `<name>.yaml`. It is read as the oldest source
//! and split into the days its entries name the first time the log is appended to — the entries
//! carry their own timestamps, so they can be put where they belong rather than lumped together.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// Anything kept in a day log. `at` is the entry's own RFC3339 timestamp, which decides its file.
pub trait Dated {
    fn at(&self) -> &str;
}

/// `projects/<id>/<name>/` — the folder holding the day files.
fn dir(data_dir: &Path, project: &str, name: &str) -> PathBuf {
    data_dir.join("projects").join(project).join(name)
}

/// The pre-folder file, e.g. `projects/<id>/activity.yaml`.
fn legacy_file(data_dir: &Path, project: &str, name: &str) -> PathBuf {
    data_dir
        .join("projects")
        .join(project)
        .join(format!("{name}.yaml"))
}

/// `YYYY-MM-DD` from an RFC3339 timestamp, or `undated` for anything unparseable — an entry with a
/// broken timestamp is still an entry, and dropping it would be the very loss this avoids.
fn day_of(at: &str) -> String {
    let day = at.get(..10).unwrap_or("");
    let plausible = day.len() == 10
        && day.chars().enumerate().all(|(i, c)| match i {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        });
    if plausible {
        day.to_string()
    } else {
        "undated".to_string()
    }
}

/// The day files, newest first by name (ISO dates sort lexically, which is why they are named so).
fn day_files(data_dir: &Path, project: &str, name: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir(data_dir, project, name)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    files.sort();
    files.reverse();
    files
}

fn read_file<T: DeserializeOwned>(file: &Path) -> Vec<T> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok())
        .unwrap_or_default()
}

/// The most recent `limit` entries, newest first. Walks day files newest-first and stops as soon as
/// it has enough, so a board with years of history costs the same to read as a new one.
pub fn read<T: DeserializeOwned + Dated>(
    data_dir: &Path,
    project: &str,
    name: &str,
    limit: usize,
) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for file in day_files(data_dir, project, name) {
        if out.len() >= limit {
            // `break`, not `return`: the truncate below is what honours the limit, and returning
            // early skipped it — a bounded read could hand back a whole extra day.
            break;
        }
        let mut day: Vec<T> = read_file(&file);
        // Within a file, newest first — the same order a reader expects across files.
        day.sort_by(|a, b| b.at().cmp(a.at()));
        out.extend(day);
    }
    // A board that has not been migrated yet still has everything in the old single file.
    if out.len() < limit {
        let mut legacy: Vec<T> = read_file(&legacy_file(data_dir, project, name));
        legacy.sort_by(|a, b| b.at().cmp(a.at()));
        out.extend(legacy);
    }
    out.truncate(limit);
    out
}

/// Every entry a log holds, oldest first. Used by migration and by anything that genuinely needs
/// the whole history rather than a window.
pub fn read_all<T: DeserializeOwned + Dated>(data_dir: &Path, project: &str, name: &str) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for file in day_files(data_dir, project, name) {
        out.extend(read_file::<T>(&file));
    }
    out.extend(read_file::<T>(&legacy_file(data_dir, project, name)));
    out.sort_by(|a, b| a.at().cmp(b.at()));
    out
}

/// Append one entry to its day's file, nothing else touched. Best-effort like the logs it replaces:
/// a failure here must never fail the write that produced the entry.
pub fn append<T: Serialize + DeserializeOwned + Dated>(
    data_dir: &Path,
    project: &str,
    name: &str,
    entry: T,
) {
    migrate::<T>(data_dir, project, name);
    let folder = dir(data_dir, project, name);
    if std::fs::create_dir_all(&folder).is_err() {
        return;
    }
    let file = folder.join(format!("{}.yaml", day_of(entry.at())));
    let mut day: Vec<T> = read_file(&file);
    day.push(entry);
    // Oldest first within a day: a day file then reads like a diary, and an append is a diff at the
    // end rather than a rewrite of the whole file.
    day.sort_by(|a, b| a.at().cmp(b.at()));
    if let Ok(yaml) = serde_yaml::to_string(&day) {
        let _ = std::fs::write(&file, yaml);
    }
}

/// Split a pre-folder single file into the days its entries name, then remove it. Runs once, on the
/// first append; a board that never writes again keeps working because `read` still consults the old
/// file. Nothing is dropped: an entry whose timestamp cannot be parsed lands in `undated.yaml`.
pub fn migrate<T: Serialize + DeserializeOwned + Dated>(
    data_dir: &Path,
    project: &str,
    name: &str,
) {
    let legacy = legacy_file(data_dir, project, name);
    if !legacy.is_file() {
        return;
    }
    let entries: Vec<T> = read_file(&legacy);
    let folder = dir(data_dir, project, name);
    if std::fs::create_dir_all(&folder).is_err() {
        return;
    }
    let mut by_day: std::collections::BTreeMap<String, Vec<T>> = std::collections::BTreeMap::new();
    for entry in entries {
        by_day.entry(day_of(entry.at())).or_default().push(entry);
    }
    for (day, mut day_entries) in by_day {
        let file = folder.join(format!("{day}.yaml"));
        let mut existing: Vec<T> = read_file(&file);
        existing.append(&mut day_entries);
        existing.sort_by(|a, b| a.at().cmp(b.at()));
        let Ok(yaml) = serde_yaml::to_string(&existing) else {
            return; // leave the old file in place rather than half-migrating
        };
        if std::fs::write(&file, yaml).is_err() {
            return;
        }
    }
    let _ = std::fs::remove_file(&legacy);
}

/// The oldest day the log reaches, as `YYYY-MM-DD`. A reader asking about an earlier period is told
/// the period predates the log, instead of being shown silence and left to read it as absence.
pub fn oldest_day(data_dir: &Path, project: &str, name: &str) -> Option<String> {
    let mut days: Vec<String> = day_files(data_dir, project, name)
        .into_iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .filter(|d| d != "undated")
        .collect();
    days.sort();
    days.into_iter().next()
}
