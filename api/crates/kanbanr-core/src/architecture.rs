//! The front ends stay thin (FEAT-176): behaviour lives in kanbanr-core, and the CLI and the daemon
//! parse, render and collect facts.
//!
//! A ratchet rather than a wall. Today's offenders are listed in [`BASELINE`] with a count per file
//! and rule; the test fails when a count goes **up** (a new offence) and when it goes **down** (the
//! baseline must be lowered to match, so a fixed offence cannot quietly come back). The refactor
//! empties the list. An exception that is meant to stay says so on its own line:
//! `// architecture: allowed because …`.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// One rule: what it looks for, and why that belongs in core.
    struct Rule {
        name: &'static str,
        why: &'static str,
        /// Whether a production line of a front-end file breaks it.
        hits: fn(file: &str, line: &str) -> bool,
    }

    fn rules() -> Vec<Rule> {
        vec![
            Rule {
                name: "yaml",
                why: "board YAML is read and written by core, so every board file has one reader",
                hits: |_, l| l.contains("serde_yaml::") || l.contains("serde_saphyr::"),
            },
            Rule {
                name: "model-literal",
                why: "a domain record (transition, verdict, sign-off, process source) is made by core",
                hits: |_, l| {
                    ["Transition", "ApprovalEvent", "Signoff", "ProcessSource"]
                        .iter()
                        .any(|t| {
                            l.contains(&format!("models::{t} {{"))
                                || l.contains(&format!("config::{t} {{"))
                        })
                },
            },
            Rule {
                name: "board-commit",
                why: "a front end commits, pushes or changes the board's identity only in a function \
                      gated by agentscope::outside_core_refusal, marked `// architecture: allowed \
                      because gated by outside_core_refusal` (FEAT-214)",
                hits: |_, l| BOARD_COMMIT_CALLS.iter().any(|c| l.contains(c)),
            },
            Rule {
                name: "dispatch",
                why: "each front end reaches core's operations through one entry point",
                hits: |file, l| {
                    l.contains("dispatch::dispatch(")
                        && !file.ends_with("kanbanr-cli/src/backend.rs")
                        && !file.ends_with("kanbanr-server/src/routes.rs")
                },
            },
        ]
    }

    /// The calls that write the board outside `write::apply` (FEAT-214's `board-commit` rule).
    const BOARD_COMMIT_CALLS: &[&str] = &["commit_local(", "push_pending(", "set_identity("];
    /// The mark a gated board commit carries.
    const BOARD_COMMIT_MARK: &str =
        "// architecture: allowed because gated by outside_core_refusal";

    /// The `board-commit` offences of one file's production code: a call that is not marked, or is
    /// marked in a function that never asks `outside_core_refusal` before it.
    fn board_commit_offences(text: &str) -> usize {
        let mut gated = false;
        let mut n = 0;
        // Up to the test module: a `#[cfg(test)]` helper above it does not end production code.
        let code = text.split("#[cfg(test)]\nmod tests").next().unwrap_or(text);
        for line in code.lines() {
            let t = line.trim_start();
            if t.starts_with("fn ")
                || t.starts_with("pub fn ")
                || t.starts_with("pub(crate) fn ")
                || t.starts_with("async fn ")
                || t.starts_with("pub async fn ")
            {
                gated = false;
            }
            if line
                .replace(BOARD_COMMIT_MARK, "")
                .contains("outside_core_refusal")
            {
                gated = true;
            }
            if BOARD_COMMIT_CALLS.iter().any(|c| line.contains(c))
                && !(line.contains(BOARD_COMMIT_MARK) && gated)
            {
                n += 1;
            }
        }
        n
    }

    /// Today's offences, file → rule → count. Lower these as the refactor moves code into core;
    /// the refactor is finished when this is empty.
    const BASELINE: &[(&str, &str, usize)] = &[];

    /// Front-end crates may not depend directly on these: they are core's business.
    const CORE_ONLY_DEPS: &[&str] = &["git2", "serde_yaml", "serde-saphyr", "ears-classifier"];
    /// Direct dependencies still to be removed (the dependency rule's baseline).
    const DEP_BASELINE: &[(&str, &str)] = &[];

    fn crates_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    /// A file's production code: everything before its first `#[cfg(test)]`.
    fn production(text: &str) -> &str {
        text.split("#[cfg(test)]").next().unwrap_or(text)
    }

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                rust_files(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }

    fn offences() -> BTreeMap<(String, String), usize> {
        let mut found = BTreeMap::new();
        for krate in ["kanbanr-cli", "kanbanr-server"] {
            let mut files = Vec::new();
            rust_files(&crates_dir().join(krate).join("src"), &mut files);
            for path in files {
                let rel = format!(
                    "{krate}/{}",
                    path.strip_prefix(crates_dir().join(krate))
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                );
                let text = std::fs::read_to_string(&path).unwrap();
                let gated = board_commit_offences(&text);
                if gated > 0 {
                    found.insert((rel.clone(), "board-commit".to_string()), gated);
                }
                for line in production(&text).lines() {
                    if line.contains("// architecture: allowed because") {
                        continue;
                    }
                    for rule in rules().into_iter().filter(|r| r.name != "board-commit") {
                        if (rule.hits)(&rel, line) {
                            *found
                                .entry((rel.clone(), rule.name.to_string()))
                                .or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        found
    }

    /// The `[dependencies]` table of a front-end crate's manifest, by name.
    fn direct_deps(krate: &str) -> Vec<String> {
        let manifest =
            std::fs::read_to_string(crates_dir().join(krate).join("Cargo.toml")).unwrap();
        manifest
            .split("\n[")
            .find(|section| section.starts_with("dependencies]"))
            .map(|section| {
                section
                    .lines()
                    .skip(1)
                    .filter_map(|l| l.split(['=', '.', ' ']).next())
                    .map(str::trim)
                    .filter(|n| !n.is_empty() && !n.starts_with('#'))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// FEAT-176 R-2: a front-end crate that implements domain behaviour outside kanbanr-core's
    /// public API fails here, by name, with the rule it broke and why.
    #[test]
    fn front_ends_are_thin() {
        let found = offences();
        let baseline: BTreeMap<(String, String), usize> = BASELINE
            .iter()
            .map(|(f, r, n)| ((f.to_string(), r.to_string()), *n))
            .collect();
        let why: BTreeMap<&str, &str> = rules().iter().map(|r| (r.name, r.why)).collect();
        let mut problems = Vec::new();
        for (key, n) in &found {
            let allowed = baseline.get(key).copied().unwrap_or(0);
            if *n > allowed {
                problems.push(format!(
                    "{} breaks '{}' {n} time(s), {allowed} allowed: {}",
                    key.0,
                    key.1,
                    why[key.1.as_str()]
                ));
            }
        }
        for (key, allowed) in &baseline {
            let n = found.get(key).copied().unwrap_or(0);
            if n < *allowed {
                problems.push(format!(
                    "{} now breaks '{}' {n} time(s): lower its BASELINE entry from {allowed} to {n}",
                    key.0, key.1
                ));
            }
        }
        for krate in ["kanbanr-cli", "kanbanr-server"] {
            let deps = direct_deps(krate);
            for dep in CORE_ONLY_DEPS {
                let listed = deps.iter().any(|d| d == dep);
                let allowed = DEP_BASELINE.contains(&(krate, dep));
                if listed && !allowed {
                    problems.push(format!(
                        "{krate} depends directly on {dep}, which is core's"
                    ));
                }
                if !listed && allowed {
                    problems.push(format!(
                        "{krate} no longer depends on {dep}: remove it from DEP_BASELINE"
                    ));
                }
            }
        }
        assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
    }

    /// The guardrail seen failing (ADR-0008): each rule catches the shape it is for.
    #[test]
    fn each_rule_catches_its_shape() {
        let rules = rules();
        let hit = |name: &str, file: &str, line: &str| {
            (rules.iter().find(|r| r.name == name).unwrap().hits)(file, line)
        };
        assert!(hit(
            "yaml",
            "kanbanr-cli/src/main.rs",
            "let d = serde_yaml::from_str(&raw)?;"
        ));
        assert!(hit(
            "yaml",
            "kanbanr-server/src/routes.rs",
            "let d: Value = serde_saphyr::from_str(&raw)?;"
        ));
        assert!(hit(
            "model-literal",
            "kanbanr-cli/src/main.rs",
            "let t = kanbanr_core::models::Transition {"
        ));
        assert!(hit(
            "dispatch",
            "kanbanr-server/src/write.rs",
            "dispatch::dispatch(store, m, p, b)"
        ));
        assert!(!hit(
            "dispatch",
            "kanbanr-cli/src/backend.rs",
            "dispatch::dispatch(&self.store, m, p, b)"
        ));
        assert!(hit(
            "board-commit",
            "kanbanr-cli/src/main.rs",
            "kanbanr_core::git::set_identity(&dir, name, email)?;"
        ));
        // FEAT-214: only a marked call in a function that asks outside_core_refusal first passes.
        let mark = BOARD_COMMIT_MARK;
        assert_eq!(
            board_commit_offences("fn a() {\n git::commit_local(d, m);\n}"),
            1
        );
        assert_eq!(
            board_commit_offences(&format!("fn a() {{\n git::commit_local(d, m); {mark}\n}}")),
            1,
            "marked, but never gated"
        );
        assert_eq!(
            board_commit_offences(&format!(
                "fn a() {{\n refuse(outside_core_refusal(c, w))?;\n git::push_pending(d); {mark}\n}}\n\
                 fn b() {{\n git::push_pending(d); {mark}\n}}"
            )),
            1,
            "the gate in one function does not cover the next"
        );
        assert!(
            production("fn a() {}\n#[cfg(test)]\nmod t { serde_yaml:: }")
                .find("serde_yaml")
                .is_none()
        );
        assert!(direct_deps("kanbanr-cli").contains(&"kanbanr-core".to_string()));
    }
}
