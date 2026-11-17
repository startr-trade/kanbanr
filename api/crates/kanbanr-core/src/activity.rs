//! Per-project activity log — a changelog the writer appends to, stored as plain data in the data
//! folder (`projects/<id>/activity.yaml`): a capped, newest-first list of `{time, actor, message}`.
//! The view (web monitor, or any future viewer) just reads this file; no git plumbing required.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The folder holding one file per day (FEAT-066). Nothing is ever trimmed: the raw log is kept in
/// full and only *reads* are bounded. See [`crate::daylog`] for why.
const LOG: &str = "activity";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// Made under an agent scope (FEAT-214): the person, by the agent for its item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_agent: Option<crate::agentscope::ByAgent>,
    /// Where a verdict was taken (DEF-028); empty for any other change.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
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

/// Which activity a reader asked for (FEAT-176): `GET /projects/{p}/activity?ref=&ongoing=&limit=`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Only changes to this work item.
    pub item: Option<String>,
    /// The maintenance stream: only changes to items now in a status the board does not display.
    pub ongoing: bool,
    /// How many entries at most; 25 when not given.
    pub limit: Option<usize>,
}

/// The most recent activity a [`Filter`] selects, newest first, from the last 200 entries. When
/// the project cannot be read, the ongoing stream is empty.
pub fn read_filtered(store: &crate::Store, id: &str, filter: &Filter) -> Vec<Activity> {
    let mut entries = read(store.data_dir(), id, 200);
    if let Some(item) = &filter.item {
        // FTR-056: entries written under an earlier code of the item are the item's too.
        let codes: Vec<String> = store
            .load_meta(id)
            .ok()
            .and_then(|p| {
                crate::aliases::find(&p, item).map(|f| f.codes().map(str::to_string).collect())
            })
            .unwrap_or_else(|| vec![item.clone()]);
        entries.retain(|e| {
            e.item
                .as_deref()
                .is_some_and(|c| codes.iter().any(|k| k == c))
        });
    }
    if filter.ongoing {
        match store.load(id).ok() {
            Some(project) => {
                let displayed = project.config.board_states();
                // Feature codes whose current status is NOT displayed (ongoing/maintenance).
                let ongoing_codes: std::collections::HashSet<&str> = project
                    .features
                    .iter()
                    .filter(|f| !displayed.contains(&f.status))
                    .flat_map(|f| f.codes())
                    .collect();
                entries.retain(|e| e.item.as_deref().is_some_and(|c| ongoing_codes.contains(c)));
            }
            None => entries.clear(),
        }
    }
    entries.truncate(filter.limit.unwrap_or(25));
    entries
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
    if let Ok(yaml) = crate::yaml::to_string(entries) {
        let _ = std::fs::write(dir.join("activity.yaml"), yaml);
    }
}

/// Append an entry to today's file. Nothing is trimmed — `item` is the work item the change
/// touched, when applicable.
pub fn append(data_dir: &Path, project: &str, actor: &str, message: &str, item: Option<&str>) {
    append_by(data_dir, project, actor, message, item, None);
}

/// [`append`], recording the agent the change was made by, when one made it (FEAT-214).
pub fn append_by(
    data_dir: &Path,
    project: &str,
    actor: &str,
    message: &str,
    item: Option<&str>,
    by_agent: Option<&crate::agentscope::ByAgent>,
) {
    append_with(data_dir, project, actor, message, item, by_agent, "");
}

/// [`append_by`], recording where a verdict was taken (DEF-028).
pub fn append_with(
    data_dir: &Path,
    project: &str,
    actor: &str,
    message: &str,
    item: Option<&str>,
    by_agent: Option<&crate::agentscope::ByAgent>,
    origin: &str,
) {
    crate::daylog::append(
        data_dir,
        project,
        LOG,
        Activity {
            time: crate::now_rfc3339(),
            actor: actor.to_string(),
            message: message.to_string(),
            item: item.map(String::from),
            by_agent: by_agent.cloned(),
            origin: origin.to_string(),
        },
    );
}

/// Activity verbs and their past tense (FTR-048), as the Overview's Activity lines read them. A
/// verb the board writes as two words (`sign off`) is listed in that form too.
const PAST: &[(&str, &str)] = &[
    ("add", "added"),
    ("edit", "edited"),
    ("move", "moved"),
    ("approve", "approved"),
    ("unapprove", "withdrew approval of"),
    ("signoff", "signed off"),
    ("sign off", "signed off"),
    ("ratify", "ratified"),
    ("define", "defined"),
    ("plan", "planned"),
    ("cut", "cut"),
    ("promote", "promoted"),
    ("ship", "shipped"),
    ("record", "recorded"),
    ("prepare", "prepared"),
    ("confirm", "confirmed"),
    ("reject", "rejected"),
    ("task", "updated tasks of"),
    ("test", "updated tests of"),
    ("delete", "deleted"),
    ("remove", "removed"),
    ("close", "closed"),
    ("start", "started"),
];

/// Object nouns dropped when an item code follows them ("edit feature FTR-043").
const NOUNS: &[&str] = &[
    "feature",
    "item",
    "milestone",
    "sprint",
    "release",
    "todo-list",
    "task",
    "test",
    "doc",
    "adr",
];

/// An activity message in plain words (FTR-048): `(item code, words)`.
///
/// The first word (or the two words of `sign off`) is the verb, put in the past tense by [`PAST`].
/// The item code is recognised by its boundary (L-23): a whole whitespace-separated token, with
/// trailing `.,;:` trimmed, that `title_of` knows. A part of a longer token (`FTR-0431`, or the
/// `FTR-037` of `FTR-037/R-1`) is never the code, and a code-shaped token the board does not
/// know (`SP-003`) is not an item. With a code, the words are `"<CODE> <title> <past> <rest>"`,
/// the object noun right before the code dropped; without one, `"<Past> <rest>"` capitalised.
/// An unknown verb keeps the message as it is, with the title inserted after the code.
pub fn humanise(
    message: &str,
    title_of: impl Fn(&str) -> Option<String>,
) -> (Option<String>, String) {
    let tokens: Vec<&str> = message.split_whitespace().collect();
    let code_at = tokens.iter().position(|t| {
        let bare = t.trim_end_matches(['.', ',', ';', ':']);
        !bare.is_empty() && title_of(bare).is_some()
    });
    let code = code_at.map(|i| tokens[i].trim_end_matches(['.', ',', ';', ':']).to_string());
    let title = code.as_deref().and_then(&title_of);

    let lower: Vec<String> = tokens.iter().take(2).map(|t| t.to_lowercase()).collect();
    let verb = PAST.iter().find_map(|(v, past)| {
        let words = v.split(' ').count();
        (lower.len() >= words && lower[..words].join(" ") == *v).then_some((words, *past))
    });
    // A code inside the verb's own words is not the verb.
    let verb = verb.filter(|(n, _)| code_at.is_none_or(|i| i >= *n));

    let Some((verb_len, past)) = verb else {
        // Unknown verb: the message as it is, the title after the code.
        let words = match (code_at, &title) {
            (Some(i), Some(t)) => {
                let mut out: Vec<String> = tokens.iter().map(|s| s.to_string()).collect();
                out.insert(i + 1, t.clone());
                out.join(" ")
            }
            _ => message.to_string(),
        };
        return (code, words);
    };

    let mut rest: Vec<&str> = Vec::new();
    for (i, t) in tokens.iter().enumerate().skip(verb_len) {
        if Some(i) == code_at {
            continue;
        }
        let noun_before_code = code_at == Some(i + 1) && NOUNS.contains(&t.to_lowercase().as_str());
        if !noun_before_code {
            rest.push(t);
        }
    }
    let rest = rest.join(" ");
    let words = match (&code, &title) {
        (Some(c), Some(t)) => [c.as_str(), t.as_str(), past, rest.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
        _ => {
            let line = if rest.is_empty() {
                past.to_string()
            } else {
                format!("{past} {rest}")
            };
            let mut chars = line.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => line,
            }
        }
    };
    (code, words)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R-8: a message reads as the item and what happened to it.
    #[test]
    fn a_message_reads_as_the_item_and_what_happened() {
        let title_of = |c: &str| match c {
            "FTR-043" => Some("One Plan page".to_string()),
            "DEF-056" => Some("A broken link".to_string()),
            _ => None,
        };
        // A known verb with a code: the code, its title, the past tense; the noun dropped.
        assert_eq!(
            humanise("edit feature FTR-043", title_of),
            (
                Some("FTR-043".to_string()),
                "FTR-043 One Plan page edited".to_string()
            )
        );
        // No code: the past tense first, capitalised (coordinator, 17 Nov).
        assert_eq!(
            humanise("add milestone", title_of),
            (None, "Added milestone".to_string())
        );
        // A code-shaped token the board does not know is not an item, and its noun stays.
        assert_eq!(
            humanise("plan sprint SP-003", title_of),
            (None, "Planned sprint SP-003".to_string())
        );
        // Sign-off, in one word and in the two words the board writes.
        assert_eq!(
            humanise("signoff design-review on FTR-043", title_of).1,
            "FTR-043 One Plan page signed off design-review on"
        );
        assert_eq!(
            humanise("sign off docs-folded on DEF-056", title_of),
            (
                Some("DEF-056".to_string()),
                "DEF-056 A broken link signed off docs-folded on".to_string()
            )
        );
        // A verb phrase that is not a plain past tense.
        assert_eq!(
            humanise("unapprove feature FTR-043", title_of).1,
            "FTR-043 One Plan page withdrew approval of"
        );
        // An unknown verb keeps its text, with the title after the code.
        assert_eq!(
            humanise("frobnicate FTR-043 twice", title_of),
            (
                Some("FTR-043".to_string()),
                "frobnicate FTR-043 One Plan page twice".to_string()
            )
        );
        assert_eq!(
            humanise("frobnicate the board", title_of),
            (None, "frobnicate the board".to_string())
        );
        // L-23: a code is a whole token, never a part of one.
        let (code, words) = humanise("edit feature FTR-0431", title_of);
        assert_eq!(code, None);
        assert_eq!(words, "Edited feature FTR-0431");
        let (code, _) = humanise("test a::b of FTR-043/R-1 is now green", title_of);
        assert_eq!(code, None);
        // Trailing punctuation is not part of the code.
        assert_eq!(
            humanise("move FTR-043.", title_of).0.as_deref(),
            Some("FTR-043")
        );
    }

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
            by_agent: None,
            origin: String::new(),
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
                crate::yaml::to_string(&entries).unwrap(),
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
        let same_day: Vec<Activity> = crate::yaml::from_str(
            &std::fs::read_to_string(folder.join("2026-09-03.yaml")).unwrap(),
        )
        .unwrap();
        assert_eq!(same_day.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
