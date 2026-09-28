//! Per-project activity log — a changelog the writer appends to, stored as plain data in the data
//! folder (`projects/<id>/activity.yaml`): a capped, newest-first list of `{time, actor, message}`.
//! The view (web monitor, or any future viewer) just reads this file; no git plumbing required.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The folder holding one file per day (FEAT-066). Nothing is ever trimmed: the raw log is kept in
/// full and only *reads* are bounded. See [`crate::daylog`] for why.
const LOG: &str = "activity";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    /// RFC3339 timestamp.
    pub time: String,
    /// Who made the change (the commit identity name).
    pub actor: String,
    /// A short human description of the change.
    pub message: String,
    /// The work item (feature code) this change touched, when applicable — enables per-item and
    /// per-kind activity streams. Serialized as `item`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

impl crate::daylog::Dated for Activity {
    fn at(&self) -> &str {
        &self.time
    }
}

/// The most recent `limit` entries, newest first.
pub fn read(data_dir: &Path, project: &str, limit: usize) -> Vec<Activity> {
    crate::daylog::read(data_dir, project, LOG, limit)
}

/// Every entry, oldest first. For callers that need the whole history rather than a window — the
/// retrospective's fallback, which used to ask for a thousand entries and silently get two hundred.
pub fn read_all(data_dir: &Path, project: &str) -> Vec<Activity> {
    crate::daylog::read_all(data_dir, project, LOG)
}

/// The oldest day this log reaches, for telling a reader that a period predates it.
pub fn oldest_day(data_dir: &Path, project: &str) -> Option<String> {
    crate::daylog::oldest_day(data_dir, project, LOG)
}

/// Stand in a log from an older board — a single `activity.yaml`, the shape written before the day
/// folders existed. Test-only, and deliberately writing the OLD shape: that is what a test needs to
/// prove migration works.
#[cfg(test)]
pub fn write_legacy_for_test(data_dir: &Path, project: &str, entries: &[Activity]) {
    let dir = data_dir.join("projects").join(project);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(yaml) = serde_yaml::to_string(entries) {
        let _ = std::fs::write(dir.join("activity.yaml"), yaml);
    }
}

/// Append an entry to today's file. Nothing is trimmed — `item` is the work item the change
/// touched, when applicable.
pub fn append(data_dir: &Path, project: &str, actor: &str, message: &str, item: Option<&str>) {
    crate::daylog::append(
        data_dir,
        project,
        LOG,
        Activity {
            time: crate::now_rfc3339(),
            actor: actor.to_string(),
            message: message.to_string(),
            item: item.map(String::from),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-log-{}-{name}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("projects").join("demo")).unwrap();
        dir
    }

    fn entry(time: &str, message: &str) -> Activity {
        Activity {
            time: time.to_string(),
            actor: "t".into(),
            message: message.to_string(),
            item: None,
        }
    }

    /// FEAT-066: the log was a ring buffer capped at 200 entries. That is data loss dressed as
    /// housekeeping — the charter says raw data is never discarded.
    #[test]
    fn a_day_file_holds_its_own_entries_and_nothing_is_trimmed() {
        let dir = scratch("append");
        // Far more than the old cap of 200.
        for i in 0..250 {
            append(&dir, "demo", "t", &format!("write {i}"), None);
        }
        assert_eq!(
            read_all(&dir, "demo").len(),
            250,
            "nothing is trimmed, however many there are"
        );
        // All of today's, in one file named for today.
        let today = crate::now_rfc3339()[..10].to_string();
        let folder = dir.join("projects/demo/activity");
        let files: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(files, vec![format!("{today}.yaml")], "one file, today's");
        // And the old single file is not recreated.
        assert!(!dir.join("projects/demo/activity.yaml").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_are_bounded_while_the_data_is_not() {
        let dir = scratch("bounded");
        // Three days of history, written as day files directly.
        let folder = dir.join("projects/demo/activity");
        std::fs::create_dir_all(&folder).unwrap();
        for day in ["2026-09-01", "2026-09-02", "2026-09-03"] {
            let entries: Vec<Activity> = (0..5)
                .map(|i| entry(&format!("{day}T0{i}:00:00Z"), &format!("{day} #{i}")))
                .collect();
            std::fs::write(
                folder.join(format!("{day}.yaml")),
                serde_yaml::to_string(&entries).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(read_all(&dir, "demo").len(), 15, "the data is all there");

        // A window takes the newest, across day boundaries, newest first.
        let recent = read(&dir, "demo", 7);
        assert_eq!(recent.len(), 7);
        assert!(
            recent[0].message.starts_with("2026-09-03"),
            "{:?}",
            recent[0]
        );
        assert!(
            recent.last().unwrap().message.starts_with("2026-09-02"),
            "the window spans into the previous day: {:?}",
            recent.last()
        );
        assert_eq!(oldest_day(&dir, "demo").as_deref(), Some("2026-09-01"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_existing_single_file_log_is_split_by_the_dates_it_holds() {
        let dir = scratch("migrate");
        // A board written by an older version: one file, several days, newest first.
        write_legacy_for_test(
            &dir,
            "demo",
            &[
                entry("2026-09-03T10:00:00Z", "newest"),
                entry("2026-09-01T10:00:00Z", "oldest"),
                entry("2026-09-03T09:00:00Z", "same day as newest"),
                entry("not-a-timestamp", "broken but still an entry"),
            ],
        );
        // Readable before migration: an old board keeps working untouched.
        assert_eq!(read_all(&dir, "demo").len(), 4);

        // The first append migrates it — by the dates the entries themselves carry.
        append(&dir, "demo", "t", "a new write", None);
        let folder = dir.join("projects/demo/activity");
        let mut files: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        files.sort();
        assert!(files.contains(&"2026-09-01.yaml".to_string()), "{files:?}");
        assert!(files.contains(&"2026-09-03.yaml".to_string()), "{files:?}");
        assert!(
            files.contains(&"undated.yaml".to_string()),
            "an unparseable timestamp is still an entry and must not be dropped: {files:?}"
        );
        // Nothing lost, nothing duplicated, and the old file is gone.
        assert_eq!(read_all(&dir, "demo").len(), 5);
        assert!(!dir.join("projects/demo/activity.yaml").exists());
        // The two same-day entries share one file.
        let same_day: Vec<Activity> =
            serde_yaml::from_str(&std::fs::read_to_string(folder.join("2026-09-03.yaml")).unwrap())
                .unwrap();
        assert_eq!(same_day.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
