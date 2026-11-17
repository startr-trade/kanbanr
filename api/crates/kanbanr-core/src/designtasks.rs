//! A Design's implementation-task list, read from an item's spec (FTR-060): the source the
//! todo-list mirrors. `kanbanr todo from-design` compares what the Design says with what the
//! board's list holds and turns the difference into one batch of `todo.add`, `task.add` and
//! `task.edit` operations. It never changes a task's state: that is the agent's record.

use crate::models::TodoList;
use crate::store::Project;

/// One task as a Design's implementation-task list writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignTask {
    pub key: String,
    pub text: String,
    /// `T<n>` in this item; `<CODE>/?/T<n>` for another item until [`resolve`] names its list.
    pub depends_on: Vec<String>,
    pub writes: Vec<String>,
    pub reads: Vec<String>,
    pub estimate_h: Option<f64>,
}

/// How a Design task compares with the board's list.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    New,
    /// The fields that differ: `text`, `depends_on`, `writes`, `reads`, `estimate_h`.
    Changed(Vec<&'static str>),
    Unchanged,
}

/// The tasks of the first `Implementation tasks` heading of `spec`.
pub fn parse(spec: &str) -> Vec<DesignTask> {
    let lines: Vec<&str> = spec.lines().collect();
    let Some((start, level)) = lines.iter().enumerate().find_map(|(i, l)| {
        heading(l).and_then(|(lv, text)| {
            text.eq_ignore_ascii_case("Implementation tasks")
                .then_some((i, lv))
        })
    }) else {
        return Vec::new();
    };
    // Each task: its key, its first line's rest, and its nested lines.
    let mut raw: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut open = false;
    for line in &lines[start + 1..] {
        if let Some((lv, _)) = heading(line) {
            if lv <= level {
                break;
            }
            open = false;
            continue;
        }
        if let Some((key, rest)) = task_bullet(line) {
            raw.push((key, rest.to_string(), Vec::new()));
            open = true;
        } else if line.trim().is_empty() {
            continue;
        } else if open && line.starts_with([' ', '\t']) {
            if let Some(t) = raw.last_mut() {
                t.2.push(line.trim().to_string());
            }
        } else {
            open = false;
        }
    }
    raw.into_iter()
        .map(|(key, first, nested)| {
            let mut all: Vec<&str> = vec![first.as_str()];
            all.extend(nested.iter().map(String::as_str));
            let writes = labelled(&all, "Writes:").unwrap_or_else(|| first_line_paths(&first));
            let reads = labelled(&all, "Reads:").unwrap_or_default();
            let depends_on = all.iter().find_map(|l| depends(l)).unwrap_or_default();
            let estimate_h = all.iter().find_map(|l| estimate(l));
            DesignTask {
                key,
                text: cut(&first, 160),
                depends_on,
                writes,
                reads,
                estimate_h,
            }
        })
        .collect()
}

/// `(level, text)` of a Markdown ATX heading.
fn heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_end();
    let level = t.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &t[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((level, rest.trim().trim_end_matches('#').trim()))
}

/// A top-level `- **T<n>** …` bullet: its key and the rest of the line.
fn task_bullet(line: &str) -> Option<(String, &str)> {
    let rest = line
        .strip_prefix("- **")
        .or_else(|| line.strip_prefix("* **"))?;
    let (key, rest) = rest.split_once("**")?;
    let n = key.strip_prefix('T')?;
    (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).then(|| (key.to_string(), rest.trim()))
}

/// The text of `s` cut at the last word boundary before `max` characters.
fn cut(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max + 1).collect();
    match head.rfind(char::is_whitespace) {
        Some(i) => head[..i].trim_end().to_string(),
        None => s.chars().take(max).collect(),
    }
}

/// The sentence of `line` that starts at `at`: up to a `.` followed by whitespace or the end, or a
/// trailing `[…]` tag.
fn sentence(line: &str, at: usize) -> &str {
    let rest = &line[at..];
    let bytes = rest.as_bytes();
    let mut end = rest.len();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'.' && bytes.get(i + 1).is_none_or(|n| n.is_ascii_whitespace()) {
            end = i;
            break;
        }
        if *b == b'[' {
            end = i;
            break;
        }
    }
    &rest[..end]
}

/// The list after `label` (`Writes:`, `Reads:`) on any of `lines`, backticks and `(new)` /
/// `(deleted)` dropped.
fn labelled(lines: &[&str], label: &str) -> Option<Vec<String>> {
    lines.iter().find_map(|l| {
        let at = l.find(label)?;
        let list = sentence(l, at + label.len());
        Some(
            list.split(',')
                .flat_map(|p| p.split(" and "))
                .map(clean_path)
                .filter(|p| !p.is_empty())
                .collect(),
        )
    })
}

fn clean_path(p: &str) -> String {
    p.replace("(new)", "")
        .replace("(deleted)", "")
        .replace('`', "")
        .trim()
        .to_string()
}

const PATH_EXTENSIONS: &[&str] = &[
    ".rs", ".ts", ".tsx", ".mjs", ".md", ".yml", ".yaml", ".toml", ".py", ".sh", ".json", ".css",
];

/// Every backticked token on `line` that looks like a path.
fn first_line_paths(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, tok) in line.split('`').enumerate() {
        if i % 2 == 0 {
            continue;
        }
        let t = tok.trim();
        let pathlike = !t.is_empty()
            && !t.contains(char::is_whitespace)
            && (t.contains('/') || PATH_EXTENSIONS.iter().any(|e| t.ends_with(e)));
        if pathlike && !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

fn is_item_code(s: &str) -> bool {
    match s.split_once('-') {
        Some((a, n)) => {
            !a.is_empty()
                && a.chars().all(|c| c.is_ascii_uppercase())
                && !n.is_empty()
                && n.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

fn is_task_key(s: &str) -> bool {
    s.strip_prefix('T')
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// The refs after `Depends on` on `line`.
fn depends(line: &str) -> Option<Vec<String>> {
    let at = line.find("Depends on")?;
    let list = sentence(line, at + "Depends on".len());
    let mut out: Vec<String> = Vec::new();
    for part in list.split([',', ';']).flat_map(|p| p.split(" and ")) {
        let p = part.trim().trim_matches(['`', '(', ')', ':']).trim();
        let words: Vec<&str> = p.split_whitespace().collect();
        let r = match words.as_slice() {
            [k] if is_task_key(k) => Some(k.to_string()),
            [c, k] if is_item_code(c) && is_task_key(k) => Some(format!("{c}/?/{k}")),
            [one] => {
                let parts: Vec<&str> = one.split('/').collect();
                match parts.as_slice() {
                    [c, k] if is_item_code(c) && is_task_key(k) => Some(format!("{c}/?/{k}")),
                    [l, k] if l.starts_with("TL-") && is_task_key(k) => Some(one.to_string()),
                    [c, l, k] if is_item_code(c) && l.starts_with("TL-") && is_task_key(k) => {
                        Some(one.to_string())
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(r) = r
            && !out.contains(&r)
        {
            out.push(r);
        }
    }
    Some(out)
}

/// `Estimate: 2 h`, or `2 h` / `2h` inside the trailing `[…]` tag.
fn estimate(line: &str) -> Option<f64> {
    if let Some(at) = line.find("Estimate:") {
        return hours(sentence(line, at + "Estimate:".len()));
    }
    let t = line.trim_end();
    let inner = t.strip_suffix(']')?;
    let open = inner.rfind('[')?;
    inner[open + 1..].split(',').find_map(hours)
}

/// `2 h`, `2h`, `1.5 h`: hours more than 0.
fn hours(s: &str) -> Option<f64> {
    let s = s.trim();
    let n = s.strip_suffix('h')?.trim();
    n.parse::<f64>().ok().filter(|h| *h > 0.0 && h.is_finite())
}

/// Name the list of each `<CODE>/?/T<n>` dependency: the one list of `<CODE>` in `project`
/// holding key `T<n>`. A dependency that names no list or two leaves the task unresolved: it is
/// returned as `(key, "unresolved: …")` and must not be applied.
pub fn resolve(tasks: &mut [DesignTask], project: &Project) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for t in tasks.iter_mut() {
        let mut problems: Vec<String> = Vec::new();
        for d in t.depends_on.iter_mut() {
            let Some((code, key)) = d
                .split_once("/?/")
                .map(|(c, k)| (c.to_string(), k.to_string()))
            else {
                continue;
            };
            let lists: Vec<&str> = project
                .features
                .iter()
                .find(|f| f.code == code)
                .map(|f| {
                    f.todo_lists
                        .iter()
                        .filter(|l| l.tasks.iter().any(|x| x.key == key))
                        .map(|l| l.code.as_str())
                        .collect()
                })
                .unwrap_or_default();
            match lists.as_slice() {
                [one] => *d = format!("{code}/{one}/{key}"),
                [] => problems.push(format!("{code} has no task {key}")),
                many => problems.push(format!(
                    "{code} {key} is in {}; name the list ({code}/TL-…/{key})",
                    many.join(" and ")
                )),
            }
        }
        if !problems.is_empty() {
            out.push((
                t.key.clone(),
                format!("unresolved: {}", problems.join("; ")),
            ));
        }
    }
    out
}

/// Each Design task with how it compares with `list`, and the keys of the board's tasks the
/// Design does not name.
pub fn compare(
    tasks: &[DesignTask],
    list: Option<&TodoList>,
) -> (Vec<(DesignTask, Change)>, Vec<String>) {
    let board: &[crate::models::Task] = list.map(|l| l.tasks.as_slice()).unwrap_or(&[]);
    let plan = tasks
        .iter()
        .map(|d| {
            let change = match board.iter().find(|b| b.key == d.key) {
                None => Change::New,
                Some(b) => {
                    let mut fields = Vec::new();
                    if b.text != d.text {
                        fields.push("text");
                    }
                    if b.depends_on != d.depends_on {
                        fields.push("depends_on");
                    }
                    if b.writes != d.writes {
                        fields.push("writes");
                    }
                    if b.reads != d.reads {
                        fields.push("reads");
                    }
                    if b.estimate_h != d.estimate_h {
                        fields.push("estimate_h");
                    }
                    if fields.is_empty() {
                        Change::Unchanged
                    } else {
                        Change::Changed(fields)
                    }
                }
            };
            (d.clone(), change)
        })
        .collect();
    let others = board
        .iter()
        .filter(|b| !tasks.iter().any(|d| d.key == b.key))
        .map(|b| b.key.clone())
        .collect();
    (plan, others)
}

/// The description of a list `todo from-design` makes.
pub const LIST_DESCRIPTION: &str = "Build (from the Design's implementation tasks)";

/// The batch that makes list `todo` of `code` match the plan: `todo.add` when the list is
/// missing, `task.add` for each new task, `task.edit` with the changed fields for each changed
/// one. Never `task.state`.
pub fn batch_ops(
    code: &str,
    todo: &str,
    list_exists: bool,
    plan: &[(DesignTask, Change)],
) -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut ops = Vec::new();
    if !list_exists {
        ops.push(json!({
            "op": "todo.add", "feature": code, "code": todo, "description": LIST_DESCRIPTION,
        }));
    }
    for (t, change) in plan {
        match change {
            Change::New => {
                let mut op = json!({
                    "op": "task.add", "feature": code, "todo": todo, "key": t.key, "text": t.text,
                });
                if !t.depends_on.is_empty() {
                    op["depends_on"] = json!(t.depends_on);
                }
                if !t.writes.is_empty() {
                    op["writes"] = json!(t.writes);
                }
                if !t.reads.is_empty() {
                    op["reads"] = json!(t.reads);
                }
                if let Some(h) = t.estimate_h {
                    op["estimate_h"] = json!(h);
                }
                ops.push(op);
            }
            Change::Changed(fields) => {
                let mut op =
                    json!({"op": "task.edit", "feature": code, "todo": todo, "key": t.key});
                for f in fields {
                    op[*f] = match *f {
                        "text" => json!(t.text),
                        "depends_on" => json!(t.depends_on),
                        "writes" => json!(t.writes),
                        "reads" => json!(t.reads),
                        _ => json!(t.estimate_h),
                    };
                }
                ops.push(op);
            }
            Change::Unchanged => {}
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Task, TaskState};

    const SPEC: &str = "\
# Title

## Design

Some prose that says Depends on T9. and `ignored/path.rs`.

### Implementation tasks
- **T1** `api/src/models.rs`, `config.rs`: the four fields. Depends on —. Writes: `api/src/models.rs`, `api/src/config.rs`. [haiku, 1 h]
- **T2** `api/src/taskgraph.rs` (new), `lib.rs`: the graph. Depends on T1, FTR-041 T1. [sonnet, 2.5 h]
  - Reads: `api/src/graph.rs`, `docs/`
  - Writes: `api/src/taskgraph.rs` (new), `api/src/lib.rs`
- **T3** The skill. Depends on T1 and FTR-099/T2. Estimate: 1.5 h
- **T4** WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD WORD
1. **T5** a numbered list is not the task list's form

### Affects docs
- **T6** after the list's section ends
";

    #[test]
    fn a_designs_task_list_is_read_with_dependencies_files_and_estimates() {
        let tasks = parse(SPEC);
        let keys: Vec<&str> = tasks.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, ["T1", "T2", "T3", "T4"], "{tasks:#?}");

        // The house form: text is the rest of the first line; `Writes:` names the files; the
        // trailing tag carries the estimate; `Depends on —` is none.
        let t1 = &tasks[0];
        assert!(
            t1.text
                .starts_with("`api/src/models.rs`, `config.rs`: the four fields."),
            "{t1:?}"
        );
        assert!(t1.text.ends_with("[haiku, 1 h]"), "{t1:?}");
        assert_eq!(t1.writes, ["api/src/models.rs", "api/src/config.rs"]);
        assert!(t1.reads.is_empty());
        assert!(t1.depends_on.is_empty());
        assert_eq!(t1.estimate_h, Some(1.0));

        // Nested lines belong to the task; `(new)` is dropped; another item's task waits for its
        // list to be named.
        let t2 = &tasks[1];
        assert_eq!(t2.writes, ["api/src/taskgraph.rs", "api/src/lib.rs"]);
        assert_eq!(t2.reads, ["api/src/graph.rs", "docs/"]);
        assert_eq!(t2.depends_on, ["T1", "FTR-041/?/T1"]);
        assert_eq!(t2.estimate_h, Some(2.5));

        // No `Writes:`: the backticked paths of the first line (none here); `Estimate:`; `and`.
        let t3 = &tasks[2];
        assert!(t3.writes.is_empty(), "{t3:?}");
        assert_eq!(t3.depends_on, ["T1", "FTR-099/?/T2"]);
        assert_eq!(t3.estimate_h, Some(1.5));

        // Text is cut at the last word boundary before 160 characters.
        let t4 = &tasks[3];
        assert!(t4.text.chars().count() <= 160, "{}", t4.text.len());
        assert!(t4.text.ends_with("WORD"), "{:?}", t4.text);
        assert!(t4.estimate_h.is_none());

        // Without a `Writes:` line, every backticked path of the first line.
        let fallback = parse(
            "## Implementation tasks\n- **T1** `web/src/a.tsx` (new), `README.md`, `not a path`: do it\n",
        );
        assert_eq!(fallback[0].writes, ["web/src/a.tsx", "README.md"]);

        // No such heading: no tasks.
        assert!(parse("# Design\n- **T1** something\n").is_empty());
    }

    fn task(key: &str, text: &str, writes: &[&str]) -> Task {
        Task {
            key: key.into(),
            text: text.into(),
            state: TaskState::Completed,
            changed_at: None,
            depends_on: Vec::new(),
            writes: writes.iter().map(|s| s.to_string()).collect(),
            reads: Vec::new(),
            estimate_h: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn a_design_is_compared_with_the_board_list_and_never_sets_state() {
        let design = vec![
            DesignTask {
                key: "T1".into(),
                text: "one".into(),
                depends_on: vec![],
                writes: vec!["a.rs".into()],
                reads: vec![],
                estimate_h: None,
            },
            DesignTask {
                key: "T2".into(),
                text: "two".into(),
                depends_on: vec!["T1".into()],
                writes: vec![],
                reads: vec![],
                estimate_h: Some(2.0),
            },
            DesignTask {
                key: "T3".into(),
                text: "three".into(),
                depends_on: vec![],
                writes: vec![],
                reads: vec![],
                estimate_h: None,
            },
        ];
        let list = TodoList {
            code: "TL-001".into(),
            description: String::new(),
            tasks: vec![
                task("T1", "one", &["a.rs"]),
                task("T2", "two, old", &["b.rs"]),
                task("T9", "gone", &[]),
            ],
            created_at: String::new(),
            extra: Default::default(),
        };
        let (plan, extra) = compare(&design, Some(&list));
        assert_eq!(plan[0].1, Change::Unchanged);
        assert_eq!(
            plan[1].1,
            Change::Changed(vec!["text", "depends_on", "writes", "estimate_h"])
        );
        assert_eq!(plan[2].1, Change::New);
        assert_eq!(extra, ["T9"]);

        let ops = batch_ops("FEAT-001", "TL-001", true, &plan);
        assert_eq!(ops.len(), 2, "{ops:#?}");
        assert_eq!(ops[0]["op"], "task.edit");
        assert_eq!(ops[0]["key"], "T2");
        assert_eq!(ops[0]["writes"], serde_json::json!([]));
        assert_eq!(ops[0]["estimate_h"], serde_json::json!(2.0));
        assert!(ops[0].get("reads").is_none(), "{:?}", ops[0]);
        assert_eq!(ops[1]["op"], "task.add");
        assert_eq!(ops[1]["key"], "T3");
        assert!(ops.iter().all(|o| o["op"] != "task.state"));

        // A missing list is added first.
        let (plan, _) = compare(&design, None);
        assert!(plan.iter().all(|(_, c)| *c == Change::New));
        let ops = batch_ops("FEAT-001", "TL-002", false, &plan);
        assert_eq!(ops[0]["op"], "todo.add");
        assert_eq!(ops[0]["code"], "TL-002");
        assert_eq!(
            ops[0]["description"],
            "Build (from the Design's implementation tasks)"
        );
        assert_eq!(ops.len(), 4);
    }
}
