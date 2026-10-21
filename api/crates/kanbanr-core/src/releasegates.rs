//! Release stages that act (FEAT-212): each stage a release moves through may declare conditions
//! (`requires`) and entry actions (`on_enter`), as an item's status declares a gate.
//!
//! The rules sit beside the stage names, in `cadence.release_gates`, keyed by stage name, so
//! `cadence.release_stages` stays a list of names: an older binary keeps the new keys in
//! `Cadence.extra` (FEAT-151) rather than failing to read the board. A stage that declares no gate
//! acts as every stage did before: the first makes the release branch, the last fast-forwards main
//! and tags the release (see [`Cadence::effective_release_gates`]).
//!
//! `cadence.release_prepare` says what the `prepare` action writes onto `release/<V>`: the version
//! files, the dated changelog heading and its copies, a notes draft and where it is listed.

use crate::config::Cadence;
use crate::models::Extra;
use serde::{Deserialize, Serialize};

/// What entering a release stage does. The order of the variants is the order a promotion applies
/// them in, whichever stages hold them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReleaseAction {
    /// Make `release/<V>` from the line of integration, without checking it out.
    Branch,
    /// Commit the version files, the changelog heading, the notes draft and its listings onto
    /// `release/<V>`, without checking anything out.
    Prepare,
    /// Fast-forward main to the release tip, refused if main has commits the release lacks.
    MoveMain,
    /// Tag the release tip with the version (an annotated tag, `Release <V>`).
    Tag,
}

impl ReleaseAction {
    pub const ALL: [ReleaseAction; 4] = [
        ReleaseAction::Branch,
        ReleaseAction::Prepare,
        ReleaseAction::MoveMain,
        ReleaseAction::Tag,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ReleaseAction::Branch => "branch",
            ReleaseAction::Prepare => "prepare",
            ReleaseAction::MoveMain => "move_main",
            ReleaseAction::Tag => "tag",
        }
    }

    /// What the action does, in a sentence.
    pub fn does(self) -> &'static str {
        match self {
            ReleaseAction::Branch => "makes release/<V> from the line of integration",
            ReleaseAction::Prepare => {
                "commits the version files, the dated changelog heading and draft notes to \
                 release/<V>"
            }
            ReleaseAction::MoveMain => "fast-forwards main to the release tip",
            ReleaseAction::Tag => "tags the release tip with the version",
        }
    }
}

/// Read by hand so a misspelt action is named with the ones there are.
impl<'de> Deserialize<'de> for ReleaseAction {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        ReleaseAction::ALL
            .into_iter()
            .find(|a| a.name() == name.trim())
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "'{name}' is not a release stage action — the actions are: {}",
                    ReleaseAction::ALL.map(ReleaseAction::name).join(", ")
                ))
            })
    }
}

/// A condition a release stage can require before a release enters it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReleaseCheck {
    /// Nothing planned into the release would be carried over by the cut.
    ItemsFinished,
    /// The release tip carries the version, the dated changelog heading and the notes.
    Prepared,
    /// Main is at the release tip.
    MainAtRelease,
    /// The release tip is on main's remote-tracking branch.
    Pushed,
}

impl ReleaseCheck {
    pub const ALL: [ReleaseCheck; 4] = [
        ReleaseCheck::ItemsFinished,
        ReleaseCheck::Prepared,
        ReleaseCheck::MainAtRelease,
        ReleaseCheck::Pushed,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ReleaseCheck::ItemsFinished => "items_finished",
            ReleaseCheck::Prepared => "prepared",
            ReleaseCheck::MainAtRelease => "main_at_release",
            ReleaseCheck::Pushed => "pushed",
        }
    }

    /// When the condition holds, in a sentence.
    pub fn passes_when(self) -> &'static str {
        match self {
            ReleaseCheck::ItemsFinished => "the cut would carry nothing over",
            ReleaseCheck::Prepared => {
                "at the release tip every version file names the version, the changelog has its \
                 heading and the notes file exists (always, when nothing is configured to prepare)"
            }
            ReleaseCheck::MainAtRelease => {
                "main is at the release tip (or, for a patch, main has moved on)"
            }
            ReleaseCheck::Pushed => {
                "the release tip is on main's remote-tracking branch (for a patch left behind, on \
                 any remote-tracking branch)"
            }
        }
    }
}

/// Read by hand so a misspelt check is named with the ones there are (as `Condition`, FEAT-168).
impl<'de> Deserialize<'de> for ReleaseCheck {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        ReleaseCheck::ALL
            .into_iter()
            .find(|c| c.name() == name.trim())
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "'{name}' is not a check a release stage can require — the checks are: {}",
                    ReleaseCheck::ALL.map(ReleaseCheck::name).join(", ")
                ))
            })
    }
}

/// One release stage's rules: what it is for, what must hold to enter it, and what entering it
/// does.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleaseGate {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<ReleaseCheck>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_enter: Vec<ReleaseAction>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// A file whose version `prepare` sets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VersionFile {
    /// Relative to the repository root.
    pub path: String,
    /// A template with exactly one `{version}`. Absent: `^version = "{version}"` for `*.toml`,
    /// `"version": "{version}"` for `*.json`, first match only. Given: every match.
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// Where the notes draft is listed: `entry` goes directly below the first line equal to `after`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct NotesListing {
    pub path: String,
    pub after: String,
    pub entry: String,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// What the `prepare` action writes onto `release/<V>`. Empty by default: `prepare` is then a
/// no-op the plan names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleasePrepare {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub version_files: Vec<VersionFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changelog_copies: Vec<String>,
    /// A path template naming `{version}` or `{number}`, e.g. `docs/src/releases/{version}.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes_listings: Vec<NotesListing>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

impl ReleasePrepare {
    /// Nothing configured, nothing kept: what an absent key means, and why it stays absent.
    pub fn is_empty(&self) -> bool {
        *self == ReleasePrepare::default()
    }

    /// Nothing configured that `prepare` would write.
    pub fn writes_nothing(&self) -> bool {
        self.version_files.is_empty()
            && self.changelog.is_none()
            && self.notes.is_none()
            && self.notes_listings.is_empty()
    }
}

impl Cadence {
    /// Every release stage with the gate in force: its declared one, or the default that reproduces
    /// what stages did before they could act (FEAT-183):
    ///
    /// - the first stage `[branch]`, unless a declared gate holds `branch`;
    /// - the last stage `[move_main, tag]`, without `move_main` if a declared gate holds it;
    /// - a single stage `[branch, move_main, tag]`;
    /// - a middle stage nothing.
    pub fn effective_release_gates(&self) -> Vec<(String, ReleaseGate)> {
        let stages = self.stages();
        let n = stages.len();
        let declared_holds = |a: ReleaseAction| {
            self.release_gates
                .iter()
                .any(|(k, g)| stages.contains(k) && g.on_enter.contains(&a))
        };
        stages
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if let Some(g) = self.release_gates.get(s) {
                    return (s.clone(), g.clone());
                }
                let on_enter = if n == 1 {
                    vec![
                        ReleaseAction::Branch,
                        ReleaseAction::MoveMain,
                        ReleaseAction::Tag,
                    ]
                } else if i == 0 {
                    if declared_holds(ReleaseAction::Branch) {
                        vec![]
                    } else {
                        vec![ReleaseAction::Branch]
                    }
                } else if i == n - 1 {
                    let mut v = vec![];
                    if !declared_holds(ReleaseAction::MoveMain) {
                        v.push(ReleaseAction::MoveMain);
                    }
                    v.push(ReleaseAction::Tag);
                    v
                } else {
                    vec![]
                };
                (
                    s.clone(),
                    ReleaseGate {
                        on_enter,
                        ..ReleaseGate::default()
                    },
                )
            })
            .collect()
    }

    /// Whether `stage` declares its own gate (rather than taking the default).
    pub fn declares_release_gate(&self, stage: &str) -> bool {
        self.release_gates.contains_key(stage)
    }

    /// The release gates and preparation must fit the stages: every gate names a stage, each
    /// action is placed once, `tag` only at the last stage, `move_main` not after `tag`, `prepare`
    /// not after `move_main`, and every path and template is one `prepare` can safely use.
    pub fn validate_release_gates(&self) -> Result<(), String> {
        let stages = self.stages();
        for key in self.release_gates.keys() {
            if !stages.contains(key) {
                return Err(format!(
                    "release stage '{key}' has a gate but is not one of this project's release \
                     stages ({}): clear it first with `kanbanr config release-stage {key} --clear`",
                    stages.join(", ")
                ));
            }
        }
        let effective = self.effective_release_gates();
        let last = effective.len().saturating_sub(1);
        let mut at: std::collections::BTreeMap<ReleaseAction, usize> = Default::default();
        for (i, (stage, gate)) in effective.iter().enumerate() {
            for a in &gate.on_enter {
                if *a == ReleaseAction::Tag && i != last {
                    return Err(format!(
                        "release stage '{stage}' tags, but only the last stage ({}) can: the tag \
                         is the cut",
                        effective[last].0
                    ));
                }
                if let Some(prev) = at.insert(*a, i) {
                    return Err(if prev == i {
                        format!("release stage '{stage}' lists '{}' twice", a.name())
                    } else {
                        format!(
                            "'{}' is placed at both '{}' and '{stage}': each action happens at \
                             one stage",
                            a.name(),
                            effective[prev].0
                        )
                    });
                }
            }
        }
        let place = |a: ReleaseAction| at.get(&a).copied();
        if let (Some(m), Some(t)) = (place(ReleaseAction::MoveMain), place(ReleaseAction::Tag))
            && m > t
        {
            return Err(format!(
                "'move_main' at '{}' comes after 'tag' at '{}': main moves before it is tagged",
                effective[m].0, effective[t].0
            ));
        }
        if let (Some(p), Some(m)) = (
            place(ReleaseAction::Prepare),
            place(ReleaseAction::MoveMain),
        ) && p > m
        {
            return Err(format!(
                "'prepare' at '{}' comes after 'move_main' at '{}': a release is prepared before \
                 main moves to it",
                effective[p].0, effective[m].0
            ));
        }
        let prep = &self.release_prepare;
        let path_ok = |what: &str, path: &str| {
            crate::releaseprep::check_repo_path(path).map_err(|e| format!("{what}: {e}"))
        };
        for f in &prep.version_files {
            path_ok("version file", &f.path)?;
            if let Some(pattern) = &f.pattern
                && pattern.matches("{version}").count() != 1
            {
                return Err(format!(
                    "the match for {} must name {{version}} exactly once: '{pattern}'",
                    f.path
                ));
            }
        }
        if let Some(c) = &prep.changelog {
            path_ok("changelog", c)?;
        }
        for c in &prep.changelog_copies {
            path_ok("changelog copy", c)?;
        }
        if let Some(notes) = &prep.notes {
            path_ok("notes", notes)?;
            if !notes.contains("{version}") && !notes.contains("{number}") {
                return Err(format!(
                    "the notes path must name {{version}} or {{number}}, so each release has its \
                     own page: '{notes}'"
                ));
            }
        }
        for l in &prep.notes_listings {
            path_ok("notes listing", &l.path)?;
            if l.after.trim().is_empty() || l.entry.trim().is_empty() {
                return Err(format!(
                    "the notes listing in {} needs both `after` (the line it goes below) and \
                     `entry`",
                    l.path
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cadence(yaml: &str) -> Result<Cadence, String> {
        crate::yaml::from_str::<Cadence>(yaml).map_err(|e| e.to_string())
    }

    fn actions(c: &Cadence) -> Vec<(String, Vec<&'static str>)> {
        c.effective_release_gates()
            .into_iter()
            .map(|(s, g)| (s, g.on_enter.iter().map(|a| a.name()).collect()))
            .collect()
    }

    /// FEAT-212 R-1: the gates are read from `release_gates`, keyed by stage name, while
    /// `release_stages` stays a list of names; both survive a write unchanged.
    #[test]
    fn release_gates_read_beside_stage_names() {
        let c = cadence(
            "releases: true\nrelease_stages: [candidate, main, production]\n\
             release_gates:\n  candidate: { on_enter: [branch, prepare] }\n  \
             main: { purpose: 'main holds the release', requires: [items_finished], on_enter: [move_main] }\n  \
             production: { requires: [main_at_release, pushed], on_enter: [tag] }\n\
             release_prepare:\n  version_files:\n    - path: api/Cargo.toml\n    \
             - { path: api/Cargo.lock, match: \"name = \\\"x\\\"\\nversion = \\\"{version}\\\"\" }\n  \
             changelog: CHANGELOG.md\n  notes: docs/src/releases/{version}.md\n",
        )
        .unwrap();
        assert_eq!(c.release_stages, ["candidate", "main", "production"]);
        assert_eq!(
            c.release_gates["main"].requires,
            [ReleaseCheck::ItemsFinished]
        );
        assert_eq!(c.release_gates["main"].purpose, "main holds the release");
        assert_eq!(
            actions(&c),
            [
                ("candidate".to_string(), vec!["branch", "prepare"]),
                ("main".to_string(), vec!["move_main"]),
                ("production".to_string(), vec!["tag"]),
            ]
        );
        assert_eq!(
            c.release_prepare.version_files[1].pattern.as_deref(),
            Some("name = \"x\"\nversion = \"{version}\"")
        );
        c.validate_release_gates().unwrap();
        let text = crate::yaml::to_string(&c).unwrap();
        assert_eq!(cadence(&text).unwrap(), c, "round trip\n{text}");
        // A cadence that does not use them writes neither key.
        let plain = cadence("releases: true\nrelease_stages: [a, b]\n").unwrap();
        let text = crate::yaml::to_string(&plain).unwrap();
        assert!(
            !text.contains("release_gates") && !text.contains("release_prepare"),
            "{text}"
        );
        // An unknown key inside a gate is kept, as the cadence keeps its own (FEAT-151).
        let newer =
            cadence("release_stages: [a]\nrelease_gates:\n  a: { on_enter: [branch], later: 1 }\n")
                .unwrap();
        assert!(crate::yaml::to_string(&newer).unwrap().contains("later"));
    }

    /// FEAT-212 R-2: a gate on no stage, an unknown check or action, or actions out of order are
    /// refused, saying why.
    #[test]
    fn release_gates_validated() {
        let refused = |yaml: &str, says: &str| {
            let err = match cadence(yaml) {
                Err(e) => e,
                Ok(c) => c.validate_release_gates().unwrap_err(),
            };
            assert!(err.contains(says), "{yaml}\n→ {err}");
        };
        refused(
            "release_stages: [a, b]\nrelease_gates: { c: { on_enter: [branch] } }\n",
            "`kanbanr config release-stage c --clear`",
        );
        refused(
            "release_stages: [a, b]\nrelease_gates: { a: { requires: [pushd] } }\n",
            "'pushd' is not a check a release stage can require — the checks are: \
             items_finished, prepared, main_at_release, pushed",
        );
        refused(
            "release_stages: [a, b]\nrelease_gates: { a: { on_enter: [merge] } }\n",
            "'merge' is not a release stage action — the actions are: branch, prepare, \
             move_main, tag",
        );
        refused(
            "release_stages: [a, b]\nrelease_gates: { a: { on_enter: [branch, tag] } }\n",
            "only the last stage (b) can",
        );
        refused(
            "release_stages: [a, b, c]\nrelease_gates: { a: { on_enter: [branch] }, b: { on_enter: [branch] } }\n",
            "'branch' is placed at both 'a' and 'b'",
        );
        refused(
            "release_stages: [a, b, c]\nrelease_gates: { b: { on_enter: [move_main] }, c: { on_enter: [prepare, tag] } }\n",
            "'prepare' at 'c' comes after 'move_main' at 'b'",
        );
        refused(
            "release_stages: [a, b]\nrelease_gates: { a: { on_enter: [prepare, prepare] } }\n",
            "lists 'prepare' twice",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { version_files: [{ path: ../x.toml }] }\n",
            "version file",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { version_files: [{ path: /etc/x.toml }] }\n",
            "absolute",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { changelog: .git/config }\n",
            ".git",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { version_files: [{ path: a.txt, match: 'v = {version} {version}' }] }\n",
            "exactly once",
        );
        refused(
            "release_stages: [a]\nrelease_prepare: { notes: docs/notes.md }\n",
            "{version} or {number}",
        );
        // Moving the cut's main step to an earlier stage is allowed: the last stage then only tags.
        let ok =
            cadence("release_stages: [a, b, c]\nrelease_gates: { b: { on_enter: [move_main] } }\n")
                .unwrap();
        ok.validate_release_gates().unwrap();
        assert_eq!(actions(&ok)[2].1, ["tag"]);
    }

    /// FEAT-212 R-3: with no gate declared, the first stage makes the branch and the last moves
    /// main and tags, as before; a single stage does all three.
    #[test]
    fn default_release_gates_reproduce_today() {
        let plain = Cadence::default();
        assert_eq!(
            actions(&plain),
            [
                ("candidate".to_string(), vec!["branch"]),
                ("test".to_string(), vec![]),
                ("staging".to_string(), vec![]),
                ("production".to_string(), vec!["move_main", "tag"]),
            ]
        );
        assert!(
            plain
                .effective_release_gates()
                .iter()
                .all(|(_, g)| g.requires.is_empty()),
            "no synthesised conditions"
        );
        let one = cadence("release_stages: [ship]\n").unwrap();
        assert_eq!(actions(&one)[0].1, ["branch", "move_main", "tag"]);
        plain.validate_release_gates().unwrap();
        one.validate_release_gates().unwrap();
        // A declared gate taking the branch leaves the first stage without it.
        let moved =
            cadence("release_stages: [a, b, c]\nrelease_gates: { b: { on_enter: [branch] } }\n")
                .unwrap();
        assert_eq!(
            actions(&moved),
            [
                ("a".to_string(), vec![]),
                ("b".to_string(), vec!["branch"]),
                ("c".to_string(), vec!["move_main", "tag"]),
            ]
        );
    }
}
