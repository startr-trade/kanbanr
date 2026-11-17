//! Practice profiles (FTR-033): packs of engineering questions a project answers once, at setup
//! or at the first design that needs each.
//!
//! A pack is one YAML file listing **areas**: a question, its options with their reasoning, and
//! what each option records in the project's engineering baseline ([`crate::baseline`]). Packs
//! live in the same three libraries as saved processes (FEAT-169), looked up board, personal,
//! built-in: `<board>/profiles/<name>.yaml`, `~/.kanbanr/profiles/` (or `KANBANR_PROFILES_DIR`),
//! and the ten built-in packs under `profiles/starter/`. A project keeps the applied copy of each
//! pack it follows at `projects/<id>/packs/<name>.yaml`.

use crate::config::Library;
use crate::models::Extra;
use crate::{CoreError, Result, Store};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The board's pack library: `<board>/profiles/<name>.yaml`.
pub const FOLDER: &str = "profiles";
/// A project's applied copies: `projects/<id>/packs/<name>.yaml`.
pub const APPLIED_DIR: &str = "packs";
/// The built-in packs, in the order they are listed.
pub const BUILTIN: [&str; 10] = [
    "core",
    "observability",
    "pipeline",
    "devsecops",
    "web",
    "api",
    "data",
    "containers",
    "kubernetes",
    "infra",
];
/// R-1: at most four questions a dialog.
pub const DIALOG_SIZE: usize = 4;

const STARTER: [(&str, &str); 10] = [
    ("core", include_str!("../profiles/starter/core.yaml")),
    (
        "observability",
        include_str!("../profiles/starter/observability.yaml"),
    ),
    (
        "pipeline",
        include_str!("../profiles/starter/pipeline.yaml"),
    ),
    (
        "devsecops",
        include_str!("../profiles/starter/devsecops.yaml"),
    ),
    ("web", include_str!("../profiles/starter/web.yaml")),
    ("api", include_str!("../profiles/starter/api.yaml")),
    ("data", include_str!("../profiles/starter/data.yaml")),
    (
        "containers",
        include_str!("../profiles/starter/containers.yaml"),
    ),
    (
        "kubernetes",
        include_str!("../profiles/starter/kubernetes.yaml"),
    ),
    ("infra", include_str!("../profiles/starter/infra.yaml")),
];

/// Answers no option may take: they are the dialog's own.
const RESERVED_OPTIONS: [&str; 2] = ["later", "not-applicable"];

fn is_false(b: &bool) -> bool {
    !*b
}

/// A pack's header. Optional in a draft; `kanbanr profile save` writes it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProfileHeader {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Bumped by `kanbanr profile save` when the content changes.
    #[serde(default)]
    pub version: u32,
    /// The packs this one builds on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// One pack file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProfileFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileHeader>,
    #[serde(default)]
    pub areas: Vec<Area>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// When an area is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Asks {
    Setup,
    FirstNeed,
}

/// Whether an area takes one option or several in one answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Select {
    #[default]
    One,
    Many,
}

impl Select {
    fn is_one(&self) -> bool {
        *self == Select::One
    }
}

/// An area is asked only when another area's answer set meets one of `options`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct When {
    pub area: String,
    pub options: Vec<String>,
}

/// One question of a pack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Area {
    pub id: String,
    pub title: String,
    pub asks: Asks,
    #[serde(default, skip_serializing_if = "Select::is_one")]
    pub select: Select,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<When>,
    /// What makes a design touch this area: words, or file tokens (with `.`, `/` or `*`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touches: Vec<String>,
    pub question: String,
    #[serde(default)]
    pub options: Vec<AreaOption>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

impl Area {
    /// The option asked first when nothing is detected: the one marked, else the first.
    pub fn recommended(&self) -> Option<&AreaOption> {
        self.options
            .iter()
            .find(|o| o.recommended)
            .or_else(|| self.options.first())
    }

    pub fn option(&self, id: &str) -> Option<&AreaOption> {
        self.options.iter().find(|o| o.id == id)
    }
}

/// One answer an area offers, with its reasoning and what it records.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AreaOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub recommended: bool,
    /// One sentence: what choosing it means.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// When it is the right choice.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fits_when: String,
    /// What it gives up or needs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub costs: String,
    /// Words that mean this option, for the contradiction flag.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    /// What in the folder, or in the facts record, implies this option.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detect: Vec<Detect>,
    /// A free value the answer carries, templated as `{value}`; what to ask for.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub asks_value: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<Records>,
    #[serde(default, skip_serializing_if = "Adds::is_empty")]
    pub adds: Adds,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// What in the project implies an option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Detect {
    /// An exact relative path, a folder (trailing `/`), or `*.ext`.
    Path(String),
    /// A file (exact relative path) holding a substring.
    Text { file: String, contains: String },
    /// The project's facts record (FTR-005) lists one of `any` under `fact`.
    Fact { fact: String, any: Vec<String> },
}

/// The four kinds an answer records in the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Nfr,
    Constraint,
    NonGoal,
    Practice,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Nfr => "nfr",
            Kind::Constraint => "constraint",
            Kind::NonGoal => "non_goal",
            Kind::Practice => "practice",
        }
    }
}

/// One baseline entry an option records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Records {
    pub kind: Kind,
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// ISO 25010 characteristics, for an nfr.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub iso25010: Vec<String>,
    /// How an nfr is measured.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub measure: String,
    /// For a non-goal: the option of the same area it rules out. When that option is chosen too,
    /// the non-goal gives way to it (FTR-033 P4 Q1); without it the words of the other chosen
    /// options are matched against the text.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub excludes: String,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

/// What an option adds to the work.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Adds {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<String>,
    /// A readiness check (`kanbanr process checks`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    /// Gates a release stage requires once the answer is accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub release_gates: Vec<ReleaseGateAdd>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    #[cfg_attr(feature = "schema", schemars(with = "crate::models::ExtraSchema"))]
    pub extra: Extra,
}

impl Adds {
    pub fn is_empty(&self) -> bool {
        self == &Adds::default()
    }
}

/// A gate a release stage must then require.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReleaseGateAdd {
    pub stage: String,
    pub gate: String,
}

/// Which pack a project follows, from which library, at which version and content rev.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProfileSource {
    pub name: String,
    pub library: Library,
    #[serde(default)]
    pub version: u32,
    /// [`ProfileFile::content_rev`] of what was applied.
    pub rev: String,
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn problem(msg: String) -> CoreError {
    CoreError::Unsupported(msg)
}

/// A path a pack may name: relative, without `..`.
fn safe_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && !path.is_absolute()
        && !p.starts_with('/')
        && !p.starts_with('\\')
        && !p.contains(':')
        && path
            .components()
            .all(|c| !matches!(c, std::path::Component::ParentDir))
        && !p.split(['/', '\\']).any(|s| s == "..")
}

impl ProfileFile {
    /// A hash of the pack itself — everything but its header — from canonical JSON.
    pub fn content_rev(&self) -> String {
        let mut bare = self.clone();
        bare.profile = None;
        crate::hash::stable_hash(&crate::hash::canonical_json(&bare))
    }

    /// The header's version; 0 without a header.
    pub fn version(&self) -> u32 {
        self.profile.as_ref().map_or(0, |h| h.version)
    }

    /// The packs this one builds on.
    pub fn requires(&self) -> &[String] {
        self.profile.as_ref().map_or(&[], |h| h.requires.as_slice())
    }

    /// What a project records when it follows this pack.
    pub fn source(&self, name: &str, library: Library) -> ProfileSource {
        ProfileSource {
            name: name.to_string(),
            library,
            version: self.version(),
            rev: self.content_rev(),
        }
    }

    /// A pack from its YAML (or JSON) text; the error is the parser's own.
    pub fn from_yaml(text: &str) -> std::result::Result<ProfileFile, crate::yaml::Error> {
        crate::yaml::parse(text)
    }

    pub fn to_yaml(&self) -> Result<String> {
        crate::yaml::to_string(self)
    }

    pub fn area(&self, id: &str) -> Option<&Area> {
        self.areas.iter().find(|a| a.id == id)
    }

    fn is_builtin_pack(&self) -> bool {
        self.profile.as_ref().is_some_and(|h| is_builtin(&h.name))
    }

    /// What is wrong with this pack, each naming the area and the fix. Required packs are looked
    /// up among the built-in ones only; [`ProfileFile::problems_in`] looks in every library.
    pub fn problems(&self) -> Vec<CoreError> {
        self.problems_with(&|name| builtin(name), false)
    }

    /// [`ProfileFile::problems`], with the required packs resolved board, personal, built-in, so
    /// a `requires` naming a pack no library holds is a problem too.
    pub fn problems_in(
        &self,
        store: Option<&Store>,
        personal_dir: Option<&Path>,
    ) -> Vec<CoreError> {
        self.problems_with(
            &|name| resolve(store, name, personal_dir).ok().map(|(f, _)| f),
            true,
        )
    }

    fn problems_with(
        &self,
        lookup: &dyn Fn(&str) -> Option<ProfileFile>,
        require_found: bool,
    ) -> Vec<CoreError> {
        let mut out = Vec::new();
        let builtin_pack = self.is_builtin_pack();
        let own = self
            .profile
            .as_ref()
            .map(|h| h.name.clone())
            .unwrap_or_default();
        // Areas a `when` may name: this pack's, the ones it requires, and the built-in ones.
        let mut known: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let add_known = |f: &ProfileFile, known: &mut BTreeMap<String, Vec<String>>| {
            for a in &f.areas {
                known
                    .entry(a.id.clone())
                    .or_insert_with(|| a.options.iter().map(|o| o.id.clone()).collect());
            }
        };
        add_known(self, &mut known);
        for r in self.requires() {
            if !is_slug(r) {
                out.push(problem(format!(
                    "requires '{r}': a pack name is lower-case letters, digits and hyphens"
                )));
                continue;
            }
            if *r == own {
                out.push(problem(format!(
                    "pack '{own}' requires itself: remove it from requires"
                )));
                continue;
            }
            match lookup(r) {
                Some(f) => add_known(&f, &mut known),
                None if require_found => out.push(problem(format!(
                    "requires '{r}', which no library holds: `kanbanr profile list` shows the packs"
                ))),
                None => {}
            }
        }
        for name in BUILTIN {
            if name != own
                && let Some(f) = builtin(name)
            {
                add_known(&f, &mut known);
            }
        }
        let gateable: Vec<String> = crate::readiness::Check::GATEABLE
            .iter()
            .map(|c| c.name())
            .collect();
        let mut seen_areas = BTreeSet::new();
        for area in &self.areas {
            let a = &area.id;
            if !is_slug(a) {
                out.push(problem(format!(
                    "area '{a}': an id is lower-case letters, digits and hyphens"
                )));
            }
            if !seen_areas.insert(a.clone()) {
                out.push(problem(format!(
                    "area '{a}' is defined twice: rename or merge one"
                )));
            }
            if let Some(w) = &area.when {
                if w.area == *a {
                    out.push(problem(format!(
                        "area '{a}': `when` names the area itself: gate it on another area"
                    )));
                } else {
                    match known.get(&w.area) {
                        None => out.push(problem(format!(
                            "area '{a}': `when` names area '{}', which no pack defines",
                            w.area
                        ))),
                        Some(opts) => {
                            for o in &w.options {
                                if !opts.contains(o) {
                                    out.push(problem(format!(
                                        "area '{a}': `when` names option '{o}', which area '{}' does \
                                         not offer",
                                        w.area
                                    )));
                                }
                            }
                        }
                    }
                }
            }
            if area.options.is_empty() {
                out.push(problem(format!(
                    "area '{a}' has no options: add at least one"
                )));
            }
            if area.options.iter().filter(|o| o.recommended).count() > 1 {
                out.push(problem(format!(
                    "area '{a}' has more than one recommended option: mark one, or none (the first \
                     is then recommended)"
                )));
            }
            if area.asks == Asks::FirstNeed && area.touches.is_empty() {
                out.push(problem(format!(
                    "area '{a}' is asked at first need but touches nothing, so it could never be \
                     asked: add `touches`"
                )));
            }
            let mut seen_options = BTreeSet::new();
            for opt in &area.options {
                let o = &opt.id;
                if !is_slug(o) {
                    out.push(problem(format!(
                        "area '{a}': option '{o}': an id is lower-case letters, digits and hyphens"
                    )));
                }
                if RESERVED_OPTIONS.contains(&o.as_str()) {
                    out.push(problem(format!(
                        "area '{a}': option '{o}' is a reserved answer: rename it"
                    )));
                }
                if !seen_options.insert(o.clone()) {
                    out.push(problem(format!("area '{a}': option '{o}' appears twice")));
                }
                if builtin_pack
                    && (opt.why.trim().is_empty()
                        || opt.fits_when.trim().is_empty()
                        || opt.costs.trim().is_empty())
                {
                    out.push(problem(format!(
                        "area '{a}': option '{o}' lacks its reasoning: a built-in pack gives every \
                         option `why`, `fits_when` and `costs`"
                    )));
                }
                for r in &opt.records {
                    if r.text.trim().is_empty() {
                        out.push(problem(format!(
                            "area '{a}': option '{o}' records an empty text: write the entry"
                        )));
                    }
                    let excl = r.excludes.trim();
                    if !excl.is_empty() && r.kind != Kind::NonGoal {
                        out.push(problem(format!(
                            "area '{a}': option '{o}': `excludes` is for a non_goal, not a {}",
                            r.kind.as_str()
                        )));
                    } else if !excl.is_empty()
                        && (excl == o.as_str() || !area.options.iter().any(|x| x.id == excl))
                    {
                        out.push(problem(format!(
                            "area '{a}': option '{o}': `excludes` names '{excl}', which is not \
                             another option of the area"
                        )));
                    }
                    if !r.iso25010.is_empty() && r.kind != Kind::Nfr {
                        out.push(problem(format!(
                            "area '{a}': option '{o}': `iso25010` is for an nfr, not a {}",
                            r.kind.as_str()
                        )));
                    }
                    for q in &r.iso25010 {
                        if ears_classifier::normalize_iso(q).is_none() {
                            out.push(problem(format!(
                                "area '{a}': option '{o}': '{q}' is not an ISO 25010 \
                                 characteristic (known: {})",
                                ears_classifier::ISO25010.join(", ")
                            )));
                        }
                    }
                }
                if let Some(g) = &opt.adds.gate
                    && !gateable.contains(g)
                {
                    out.push(problem(format!(
                        "area '{a}': option '{o}': adds.gate '{g}' is not a readiness check \
                         (`kanbanr process checks`); a release stage's gate goes in \
                         adds.release_gates"
                    )));
                }
                for rg in &opt.adds.release_gates {
                    if let Err(e) = crate::gates::check_name(&rg.gate) {
                        out.push(problem(format!(
                            "area '{a}': option '{o}': adds.release_gates: {e}"
                        )));
                    }
                    if rg.stage.trim().is_empty() {
                        out.push(problem(format!(
                            "area '{a}': option '{o}': adds.release_gates names no stage"
                        )));
                    }
                }
                for d in &opt.detect {
                    match d {
                        Detect::Path(p) | Detect::Text { file: p, .. } if !safe_relative(p) => {
                            out.push(problem(format!(
                                "area '{a}': option '{o}': detect '{p}' must be a relative path \
                                 inside the project, without '..'"
                            )));
                        }
                        Detect::Fact { fact, any } => {
                            match crate::facts::VOCABULARY.iter().find(|v| v.key == fact) {
                                None => out.push(problem(format!(
                                    "area '{a}': option '{o}': detect names fact '{fact}', which \
                                     the facts record does not have (known: {})",
                                    crate::facts::VOCABULARY
                                        .iter()
                                        .map(|v| v.key)
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ))),
                                Some(v) => {
                                    for value in any {
                                        if !v.values.contains(&value.as_str())
                                            && !value.starts_with("other:")
                                        {
                                            out.push(problem(format!(
                                                "area '{a}': option '{o}': '{value}' is not a \
                                                 value of fact '{fact}' (known: {})",
                                                v.values.join(", ")
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        out
    }

    /// What `kanbanr profile check` prints without refusing.
    pub fn warnings(&self) -> Vec<String> {
        let builtin_pack = self.is_builtin_pack();
        let mut out = Vec::new();
        for area in &self.areas {
            for opt in &area.options {
                if !builtin_pack
                    && (opt.why.trim().is_empty()
                        || opt.fits_when.trim().is_empty()
                        || opt.costs.trim().is_empty())
                {
                    out.push(format!(
                        "area '{}': option '{}' lacks `why`, `fits_when` or `costs`: the person \
                         is asked without the reasoning",
                        area.id, opt.id
                    ));
                }
                for r in &opt.records {
                    if r.kind == Kind::Nfr && r.measure.trim().is_empty() {
                        out.push(format!(
                            "area '{}': option '{}': the nfr \"{}\" has no `measure`",
                            area.id, opt.id, r.text
                        ));
                    }
                }
            }
        }
        out
    }
}

/// A chosen option's value problems in an answer: a chosen option that asks for a value and has
/// none, or a value on an option that asks for none (in `one` and `many` areas alike).
pub fn value_problems(area: &Area, chosen: &[(String, String)]) -> Vec<CoreError> {
    let mut out = Vec::new();
    for (id, value) in chosen {
        let Some(opt) = area.option(id) else { continue };
        if !opt.asks_value.is_empty() && value.trim().is_empty() {
            out.push(problem(format!(
                "area '{}': option '{id}' needs a value ({})",
                area.id, opt.asks_value
            )));
        }
        if opt.asks_value.is_empty() && !value.trim().is_empty() {
            out.push(problem(format!(
                "area '{}': option '{id}' takes no value, but '{value}' was given",
                area.id
            )));
        }
    }
    out
}

/// A pack the project follows can be joined by `file` (named `name`): no area id is defined by
/// both, and every pack it requires is followed. `followed` excludes a removed pack; a pack of
/// the same name (an update) is not compared with itself.
pub fn check_composition(
    followed: &[(&str, &ProfileFile)],
    name: &str,
    file: &ProfileFile,
) -> Result<()> {
    for r in file.requires() {
        if !followed.iter().any(|(n, _)| n == r) && r != name {
            return Err(problem(format!(
                "pack '{name}' requires pack '{r}', which this project does not follow: add '{r}' \
                 first (`kanbanr profile add {r}`)"
            )));
        }
    }
    for (other, f) in followed {
        if *other == name {
            continue;
        }
        for area in &file.areas {
            if f.area(&area.id).is_some() {
                return Err(problem(format!(
                    "area '{}' is defined by pack '{other}' and by pack '{name}': an area id is \
                     unique across the packs a project follows",
                    area.id
                )));
            }
        }
    }
    Ok(())
}

/// A built-in pack by name.
pub fn builtin(name: &str) -> Option<ProfileFile> {
    let key = name.trim().to_ascii_lowercase();
    STARTER
        .iter()
        .find(|(n, _)| *n == key)
        .and_then(|(_, text)| ProfileFile::from_yaml(text).ok())
}

/// Whether `name` is a built-in pack's.
pub fn is_builtin(name: &str) -> bool {
    let key = name.trim().to_ascii_lowercase();
    BUILTIN.contains(&key.as_str())
}

/// A name a pack can be saved under: lower-case letters, digits and hyphens, and not a built-in
/// pack's.
pub fn check_name(name: &str) -> Result<()> {
    if !is_slug(name) {
        return Err(problem(format!(
            "'{name}' cannot name a pack: use lower-case letters, digits and hyphens, such as \
             our-web"
        )));
    }
    if is_builtin(name) {
        return Err(problem(format!(
            "'{name}' is a built-in pack; save yours under another name"
        )));
    }
    Ok(())
}

/// The file to save as pack `name`, given what is saved under that name now (as
/// [`crate::config::versioned`]): it must check clean; the version goes up by one when the content
/// changed; the description is the one given, else the new file's, else the old one. Returns the
/// file and whether anything changed.
pub fn versioned(
    name: &str,
    mut new: ProfileFile,
    existing: Option<&ProfileFile>,
    description: Option<String>,
) -> Result<(ProfileFile, bool)> {
    check_name(name)?;
    if let Some(p) = new.problems().into_iter().next() {
        return Err(p);
    }
    let old_header = existing.and_then(|e| e.profile.clone());
    let same = existing.is_some_and(|e| e.content_rev() == new.content_rev());
    let version = match (&old_header, same) {
        (Some(h), true) => h.version.max(1),
        (Some(h), false) => h.version + 1,
        (None, true) => 1,
        (None, false) => existing.map_or(1, |_| 2),
    };
    let new_header = new.profile.take();
    let description = description
        .or_else(|| new_header.as_ref().map(|h| h.description.clone()))
        .filter(|d| !d.trim().is_empty())
        .or_else(|| old_header.as_ref().map(|h| h.description.clone()))
        .unwrap_or_default();
    let requires = new_header
        .as_ref()
        .map(|h| h.requires.clone())
        .or_else(|| old_header.as_ref().map(|h| h.requires.clone()))
        .unwrap_or_default();
    let extra = new_header.map(|h| h.extra).unwrap_or_default();
    new.profile = Some(ProfileHeader {
        name: name.to_string(),
        description,
        version,
        requires,
        extra,
    });
    let changed = existing != Some(&new);
    Ok((new, changed))
}

// ---- the three libraries ------------------------------------------------------------------

/// The personal library: `$KANBANR_PROFILES_DIR` when set, else `<home>/.kanbanr/profiles`.
pub fn personal_dir(home: Option<&Path>) -> Option<PathBuf> {
    std::env::var_os("KANBANR_PROFILES_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".kanbanr").join("profiles")))
}

fn read_pack(path: &Path) -> Result<ProfileFile> {
    let text = std::fs::read_to_string(path)?;
    ProfileFile::from_yaml(&text)
        .map_err(|e| problem(format!("{} does not parse as a pack: {e}", path.display())))
}

/// The packs in a folder of `<name>.yaml` files, by name. A missing folder holds none.
pub(crate) fn list_dir(dir: &Path) -> Result<Vec<(String, ProfileFile)>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".yaml"))
        else {
            continue;
        };
        out.push((name.to_string(), read_pack(&path)?));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The packs in a personal library, by name.
pub fn list_personal(dir: &Path) -> Result<Vec<(String, ProfileFile)>> {
    list_dir(dir)
}

/// One pack of a folder, if there is one by that name.
pub(crate) fn get_in(dir: &Path, name: &str) -> Result<Option<ProfileFile>> {
    if check_name(name).is_err() {
        return Ok(None);
    }
    let path = dir.join(format!("{name}.yaml"));
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(read_pack(&path)?))
}

/// One personal pack, if there is one by that name.
pub fn get(dir: &Path, name: &str) -> Result<Option<ProfileFile>> {
    get_in(dir, name)
}

/// Save a pack into a folder, versioned by [`versioned`], after [`ProfileFile::problems_in`].
pub(crate) fn save_in(
    dir: &Path,
    name: &str,
    file: ProfileFile,
    description: Option<String>,
    store: Option<&Store>,
    personal: Option<&Path>,
) -> Result<ProfileFile> {
    check_name(name)?;
    if let Some(p) = file.problems_in(store, personal).into_iter().next() {
        return Err(p);
    }
    let existing = get_in(dir, name)?;
    let (file, changed) = versioned(name, file, existing.as_ref(), description)?;
    if changed {
        std::fs::create_dir_all(dir)?;
        crate::atomic::write(dir.join(format!("{name}.yaml")), file.to_yaml()?)?;
    }
    Ok(file)
}

/// Save a personal pack.
pub fn save(
    dir: &Path,
    name: &str,
    file: ProfileFile,
    description: Option<String>,
) -> Result<ProfileFile> {
    save_in(dir, name, file, description, None, Some(dir))
}

/// A pack by name: the board's (when there is a board), then the personal one, then the built-in
/// one.
pub fn resolve(
    store: Option<&Store>,
    name: &str,
    personal_dir: Option<&Path>,
) -> Result<(ProfileFile, Library)> {
    if let Some(store) = store
        && let Ok(Some(f)) = store.board_profile(name)
    {
        return Ok((f, Library::Board));
    }
    if let Some(dir) = personal_dir
        && let Some(f) = get(dir, name)?
    {
        return Ok((f, Library::Personal));
    }
    if let Some(f) = builtin(name) {
        return Ok((f, Library::Builtin));
    }
    Err(CoreError::Refused(format!(
        "no pack named '{name}' on this board, in your personal library or built in — \
         `kanbanr profile list` shows them"
    )))
}

/// One pack the board can apply: `GET /profiles`. A personal pack is known here only by name and
/// the projects that follow it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProfileListing {
    pub name: String,
    pub library: Library,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    pub used_by: Vec<String>,
}

/// One pack as `kanbanr profile list` shows it: the board's listing with the personal library
/// filled in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListedProfile {
    pub name: String,
    pub library: Library,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    pub used_by: Vec<String>,
    /// A personal pack the board has one of the same name of: the board's copy is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_by_board: Option<bool>,
    /// Someone's personal pack, followed on the board but not in this user's library.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_here: Option<bool>,
}

impl From<ProfileListing> for ListedProfile {
    fn from(l: ProfileListing) -> Self {
        ListedProfile {
            name: l.name,
            library: l.library,
            version: l.version,
            description: l.description,
            rev: l.rev,
            used_by: l.used_by,
            shadowed_by_board: None,
            not_here: None,
        }
    }
}

fn description_of(file: &ProfileFile) -> String {
    file.profile
        .as_ref()
        .map(|h| h.description.clone())
        .unwrap_or_default()
}

/// Which projects follow which pack, from each project's `baseline.yaml` (a removed pack does not
/// count). A project whose baseline cannot be read is skipped.
fn used_by(store: &Store) -> Result<BTreeMap<(String, Library), Vec<String>>> {
    let mut used: BTreeMap<(String, Library), Vec<String>> = BTreeMap::new();
    for id in store.list_projects()? {
        let Ok(b) = crate::baseline::load(store, &id) else {
            continue;
        };
        for p in b.packs.iter().filter(|p| p.removed.is_none()) {
            used.entry((p.source.name.clone(), p.source.library))
                .or_default()
                .push(id.clone());
        }
    }
    Ok(used)
}

/// Every pack this board can apply: its own library, then the built-in ones, each with the
/// projects that follow it; then personal packs projects follow.
pub fn listing(store: &Store) -> Result<Vec<ProfileListing>> {
    let used = used_by(store)?;
    let row = |name: &str, library: Library, file: &ProfileFile| ProfileListing {
        name: name.to_string(),
        library,
        version: Some(file.version()),
        description: Some(description_of(file)),
        rev: Some(file.content_rev()),
        used_by: used
            .get(&(name.to_string(), library))
            .cloned()
            .unwrap_or_default(),
    };
    let mut out = Vec::new();
    for (name, file) in store.list_profiles()? {
        out.push(row(&name, Library::Board, &file));
    }
    for name in BUILTIN {
        if let Some(file) = builtin(name) {
            out.push(row(name, Library::Builtin, &file));
        }
    }
    for ((name, library), projects) in &used {
        if *library == Library::Personal {
            out.push(ProfileListing {
                name: name.clone(),
                library: *library,
                version: None,
                description: None,
                rev: None,
                used_by: projects.clone(),
            });
        }
    }
    Ok(out)
}

/// Every pack a user can apply here: the board's, then their personal library's, then the
/// built-in ones. Without a board, the built-in ones stand in for the board's listing.
pub fn list(store: Option<&Store>, personal_dir: Option<&Path>) -> Result<Vec<ListedProfile>> {
    let mut rows: Vec<ListedProfile> = match store {
        Some(store) => listing(store)?.into_iter().map(Into::into).collect(),
        None => BUILTIN
            .iter()
            .filter_map(|name| builtin(name).map(|f| (name, f)))
            .map(|(name, f)| ListedProfile {
                name: name.to_string(),
                library: Library::Builtin,
                version: Some(f.version()),
                description: Some(description_of(&f)),
                rev: Some(f.content_rev()),
                used_by: Vec::new(),
                shadowed_by_board: None,
                not_here: None,
            })
            .collect(),
    };
    let board_names: Vec<String> = rows
        .iter()
        .filter(|r| r.library == Library::Board)
        .map(|r| r.name.clone())
        .collect();
    let (mut used_personal, rest): (Vec<ListedProfile>, Vec<ListedProfile>) = rows
        .into_iter()
        .partition(|r| r.library == Library::Personal);
    rows = rest;
    let mut mine = Vec::new();
    if let Some(dir) = personal_dir {
        for (name, file) in list_personal(dir)? {
            let used = used_personal
                .iter()
                .position(|r| r.name == name)
                .map(|i| used_personal.remove(i).used_by)
                .unwrap_or_default();
            mine.push(ListedProfile {
                library: Library::Personal,
                version: Some(file.version()),
                description: Some(description_of(&file)),
                rev: Some(file.content_rev()),
                used_by: used,
                shadowed_by_board: Some(board_names.contains(&name)),
                not_here: None,
                name,
            });
        }
    }
    for r in used_personal {
        mine.push(ListedProfile {
            name: r.name,
            library: Library::Personal,
            version: Some(0),
            description: Some("someone's personal pack — not in your library".into()),
            rev: None,
            used_by: r.used_by,
            shadowed_by_board: None,
            not_here: Some(true),
        });
    }
    let at = rows
        .iter()
        .position(|r| r.library == Library::Builtin)
        .unwrap_or(rows.len());
    rows.splice(at..at, mine);
    Ok(rows)
}

/// The copy of `source` in the library it was taken from — not by lookup order. A personal pack
/// with no personal library to look in reads as gone.
pub(crate) fn copy_in(
    store: &Store,
    source: &ProfileSource,
    personal_dir: Option<&Path>,
) -> Result<Option<ProfileFile>> {
    Ok(match source.library {
        Library::Board => store.board_profile(&source.name)?,
        Library::Personal => match personal_dir {
            Some(dir) => get(dir, &source.name)?,
            None => None,
        },
        Library::Builtin => builtin(&source.name),
    })
}

// ---- changes between versions -------------------------------------------------------------

/// What changed between the applied copy of a pack and its current one, a line each.
pub fn diff(applied: &ProfileFile, current: &ProfileFile) -> Vec<String> {
    let mut out = Vec::new();
    for a in &current.areas {
        if applied.area(&a.id).is_none() {
            out.push(format!("+ area '{}'", a.id));
        }
    }
    for a in &applied.areas {
        if current.area(&a.id).is_none() {
            out.push(format!("- area '{}'", a.id));
        }
    }
    for old in &applied.areas {
        let Some(new) = current.area(&old.id) else {
            continue;
        };
        let id = &old.id;
        let (ro, rn) = (
            old.recommended().map(|o| o.id.as_str()).unwrap_or(""),
            new.recommended().map(|o| o.id.as_str()).unwrap_or(""),
        );
        if ro != rn {
            out.push(format!("area '{id}': recommended '{ro}' → '{rn}'"));
        }
        if old.question != new.question {
            out.push(format!("area '{id}': question changed"));
        }
        for o in &new.options {
            match old.option(&o.id) {
                None => out.push(format!("area '{id}': option '{}' added", o.id)),
                Some(was) if was != o => {
                    out.push(format!("area '{id}': option '{}' changed", o.id))
                }
                Some(_) => {}
            }
        }
        for o in &old.options {
            if new.option(&o.id).is_none() {
                out.push(format!("area '{id}': option '{}' removed", o.id));
            }
        }
        if old.touches != new.touches {
            out.push(format!("area '{id}': touches changed"));
        }
    }
    out
}

/// How a pack a project follows stands against its library's current copy.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Drift {
    /// What the project follows.
    pub source: ProfileSource,
    /// The library's copy has changed since: its current version.
    pub newer: Option<u32>,
    /// The pack is no longer in its library.
    pub gone: bool,
    /// Areas the current copy has and the applied one does not.
    pub new_areas: Vec<String>,
    /// Areas whose recommended option changed.
    pub changed_recommendations: Vec<String>,
}

impl Drift {
    pub fn is_current(&self) -> bool {
        self.newer.is_none() && !self.gone
    }

    /// One sentence per thing to report, each saying what to run.
    pub fn messages(&self) -> Vec<String> {
        let s = &self.source;
        let label = match s.version {
            0 => format!("'{}' ({})", s.name, s.library.as_str()),
            v => format!("'{}' v{v} ({})", s.name, s.library.as_str()),
        };
        let mut out = Vec::new();
        if let Some(v) = self.newer {
            let now = if v == 0 {
                "it has changed since".to_string()
            } else {
                format!("v{v} is saved now")
            };
            let mut parts = Vec::new();
            if !self.new_areas.is_empty() {
                parts.push(format!(
                    "{} new area{} to answer ({})",
                    self.new_areas.len(),
                    if self.new_areas.len() == 1 { "" } else { "s" },
                    self.new_areas.join(", ")
                ));
            }
            if !self.changed_recommendations.is_empty() {
                parts.push(format!(
                    "{} recommendation{} changed ({})",
                    self.changed_recommendations.len(),
                    if self.changed_recommendations.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    self.changed_recommendations.join(", ")
                ));
            }
            let what = if parts.is_empty() {
                String::new()
            } else {
                format!(": {}", parts.join(", "))
            };
            out.push(format!(
                "this project follows pack {label}; {now}{what}: see `kanbanr profile diff {n}`, \
                 adopt it with `kanbanr profile update {n}`",
                n = s.name
            ));
        }
        if self.gone {
            out.push(format!(
                "this project follows pack {label}, which is no longer saved there: save it again \
                 with `kanbanr profile save {}`",
                s.name
            ));
        }
        out
    }
}

/// Compare the applied copy of a pack with its current copy (`None`: gone from its library).
pub fn drift(
    source: &ProfileSource,
    applied: &ProfileFile,
    current: Option<&ProfileFile>,
) -> Drift {
    let changed = current.filter(|c| c.content_rev() != source.rev);
    let (new_areas, changed_recommendations) = match changed {
        Some(c) => {
            let new_areas = c
                .areas
                .iter()
                .filter(|a| applied.area(&a.id).is_none())
                .map(|a| a.id.clone())
                .collect();
            let changed = applied
                .areas
                .iter()
                .filter_map(|old| {
                    let new = c.area(&old.id)?;
                    let (ro, rn) = (old.recommended()?, new.recommended()?);
                    (ro.id != rn.id).then(|| old.id.clone())
                })
                .collect();
            (new_areas, changed)
        }
        None => (Vec::new(), Vec::new()),
    };
    Drift {
        source: source.clone(),
        newer: changed.map(|c| c.version()),
        gone: current.is_none(),
        new_areas,
        changed_recommendations,
    }
}

// ---- detection ------------------------------------------------------------------------------

/// How deep the folder walk of [`detect`] goes.
const WALK_DEPTH: usize = 4;
/// The most entries the walk visits before it stops.
const WALK_LIMIT: usize = 20_000;
/// Folders the walk never enters.
const WALK_SKIP: [&str; 7] = [
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".venv",
];
/// The most of a file a [`Detect::Text`] rule reads.
const TEXT_LIMIT: u64 = 256 * 1024;

/// An option the project's folder or facts record implies, and what implied it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Detected {
    pub option: String,
    /// The matching path, `file: contains`, or `facts: <fact>=<value>`.
    pub evidence: String,
}

/// The entries of a project folder, walked once: relative paths with `/` separators.
struct Walk {
    root: PathBuf,
    files: Vec<String>,
    dirs: Vec<String>,
}

impl Walk {
    /// Walks `root` to [`WALK_DEPTH`], skipping [`WALK_SKIP`] and never following a link, and
    /// stops after [`WALK_LIMIT`] entries. Sorted, so detection is the same on every machine.
    fn of(root: &Path) -> Walk {
        let mut w = Walk {
            root: root.to_path_buf(),
            files: Vec::new(),
            dirs: Vec::new(),
        };
        let mut seen = 0usize;
        let mut queue: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 1)];
        while let Some((dir, rel, depth)) = queue.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut entries: Vec<_> = read.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                if seen >= WALK_LIMIT {
                    break;
                }
                seen += 1;
                let Some(name) = e.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                let path = if rel.is_empty() {
                    name.clone()
                } else {
                    format!("{rel}/{name}")
                };
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    if WALK_SKIP.contains(&name.as_str()) {
                        continue;
                    }
                    w.dirs.push(path.clone());
                    if depth < WALK_DEPTH {
                        queue.push((e.path(), path, depth + 1));
                    }
                } else if ft.is_file() {
                    w.files.push(path);
                }
            }
        }
        w.files.sort();
        w.dirs.sort();
        w
    }

    /// The first entry a [`Detect::Path`] pattern names: `*.ext` any file with that ending, `x/`
    /// a folder, else a file; an exact path matches at the root or under any walked folder.
    fn find(&self, pattern: &str) -> Option<String> {
        if !safe_relative(pattern) {
            return None;
        }
        let ends = |entry: &str, p: &str| entry == p || entry.ends_with(&format!("/{p}"));
        if let Some(ext) = pattern.strip_prefix('*') {
            let ext = ext.to_string();
            return self
                .files
                .iter()
                .find(|f| {
                    let base = f.rsplit('/').next().unwrap_or(f);
                    base.len() > ext.len() && base.ends_with(&ext)
                })
                .cloned();
        }
        if let Some(folder) = pattern.strip_suffix('/') {
            let folder = folder.trim_end_matches('/');
            return self
                .dirs
                .iter()
                .find(|d| ends(d, folder))
                .map(|d| format!("{d}/"));
        }
        self.files.iter().find(|f| ends(f, pattern)).cloned()
    }

    /// Whether `file` (an exact relative path) exists and holds `contains`, reading at most
    /// [`TEXT_LIMIT`] of it.
    fn holds(&self, file: &str, contains: &str) -> bool {
        use std::io::Read;
        if !safe_relative(file) || contains.is_empty() {
            return false;
        }
        let Ok(f) = std::fs::File::open(self.root.join(file)) else {
            return false;
        };
        let mut buf = Vec::new();
        if f.take(TEXT_LIMIT).read_to_end(&mut buf).is_err() {
            return false;
        }
        String::from_utf8_lossy(&buf).contains(contains)
    }
}

/// What `d` finds, as evidence, or `None`.
fn detect_one(
    d: &Detect,
    walk: Option<&Walk>,
    facts: Option<&crate::facts::Facts>,
) -> Option<String> {
    match d {
        Detect::Path(p) => walk.and_then(|w| w.find(p)),
        Detect::Text { file, contains } => walk
            .filter(|w| w.holds(file, contains))
            .map(|_| format!("{file}: {contains}")),
        Detect::Fact { fact, any } => facts.and_then(|f| {
            any.iter()
                .find(|v| f.has(fact, v))
                .map(|v| format!("facts: {fact}={}", v.trim().to_lowercase()))
        }),
    }
}

fn detect_with(
    area: &Area,
    walk: Option<&Walk>,
    facts: Option<&crate::facts::Facts>,
) -> Vec<Detected> {
    let mut out = Vec::new();
    for opt in &area.options {
        if let Some(evidence) = opt.detect.iter().find_map(|d| detect_one(d, walk, facts)) {
            out.push(Detected {
                option: opt.id.clone(),
                evidence,
            });
            if area.select == Select::One {
                break;
            }
        }
    }
    out
}

/// The options of `area` the project's folder (`root`) or facts record implies, in pack order:
/// every match for a `many` area, the first for a `one` area. Pure over the file system and the
/// record; never writes.
pub fn detect(root: &Path, area: &Area, facts: Option<&crate::facts::Facts>) -> Vec<Detected> {
    let walk = Walk::of(root);
    detect_with(area, Some(&walk), facts)
}

// ---- touches --------------------------------------------------------------------------------

/// A character a path holds: the rule of `docdrift`'s file names.
fn path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')
}

/// Whether one line holds a file token: `*.ext` as the end of a path word, a folder (`x/`) as a
/// path's leading part, else the whole token; each bounded by the path-character rule.
fn file_token_on_line(token: &str, line: &str) -> bool {
    if let Some(ext) = token.strip_prefix('*') {
        // `*.tsx`: a path word ending in `.tsx`, so `src/App.tsx`, never `App.tsx.bak`.
        return line.match_indices(ext).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            let after = line[at + ext.len()..].chars().next();
            before.is_some_and(|c| path_char(c) && c != '/' && c != '.')
                && !after.is_some_and(path_char)
        });
    }
    if token.ends_with('/') {
        // A folder: bounded before by the path rule; the slash itself ends the folder's name, so
        // `migrations/0003_users.sql` names `migrations/` and `premigrations/` does not.
        return line.match_indices(token).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            !before.is_some_and(|c| path_char(c) && c != '/')
        });
    }
    let name = crate::docdrift::Name {
        kind: crate::docdrift::NameKind::File,
        text: token.to_string(),
    };
    crate::docdrift::name_on_line(&name, line, &[], "")
}

/// Whether `text` holds `word`, case-insensitive, with no ASCII letter or digit on either side.
fn word_in(text: &str, word: &str) -> bool {
    let hay = text.to_ascii_lowercase();
    let needle = word.to_ascii_lowercase();
    if needle.trim().is_empty() {
        return false;
    }
    hay.match_indices(&needle).any(|(at, _)| {
        let before = hay[..at].chars().next_back();
        let after = hay[at + needle.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// Whether `token` is a file token (it holds `.`, `/` or `*`) rather than a word.
fn is_file_token(token: &str) -> bool {
    token.contains(['.', '/', '*'])
}

/// The tokens of `tokens` the text holds (L-23: boundaries stated here, never a substring).
pub(crate) fn tokens_in(tokens: &[String], text: &str) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for t in tokens {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        let hit = if is_file_token(t) {
            text.lines().any(|line| file_token_on_line(t, line))
        } else {
            word_in(text, t)
        };
        if hit {
            out.insert(t.to_string());
        }
    }
    out.into_iter().collect()
}

/// The tokens of `area.touches` a design holds, sorted. A token with `.`, `/` or `*` is a file
/// token: `*.ext` matches a path word ending in `.ext`, a folder (`x/`) a path that starts with
/// it, any other the whole token, each bounded by `docdrift`'s path-character rule; any other
/// token is a word, matched case-insensitively with ASCII-alphanumeric boundaries. Fenced blocks
/// and code spans are scanned like prose. Pure.
pub fn touches(area: &Area, design: &str) -> Vec<String> {
    tokens_in(&area.touches, design)
}

// ---- ask and explain ------------------------------------------------------------------------

/// Where an answer goes, as the dialog tells the person.
pub const RECORDS_TO: &str = "the baseline (`projects/<id>/baseline.yaml`), as a proposed \
                              revision for you to accept on the Baseline page";

/// One option as the dialog offers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AskedOption {
    pub id: String,
    pub label: String,
    pub recommended: bool,
    pub detected: bool,
    pub why: String,
    pub fits_when: String,
    pub costs: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub asks_value: String,
    /// What choosing it records: "constraint, with an ADR: PostgreSQL is the database".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<String>,
}

/// One area the dialog asks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AskedArea {
    pub id: String,
    pub pack: String,
    pub title: String,
    pub question: String,
    /// Which dialog of at most [`DIALOG_SIZE`] questions it falls in, from 1.
    pub dialog: usize,
    /// A `select: many` area: asked as a multi-select.
    pub multi: bool,
    /// Detected first, then the recommended one, then the rest in pack order.
    pub options: Vec<AskedOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detected: Vec<Detected>,
    /// Deferred before ("decide later"); the skill always offers deferring again.
    pub deferred: bool,
    /// The design's tokens that touch it (at first need).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touched: Vec<String>,
    pub records_to: String,
}

/// What to ask now, and where the rest stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Ask {
    pub packs: Vec<ProfileSource>,
    pub at: Asks,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub item: String,
    pub ask: Vec<AskedArea>,
    /// Areas of this moment already answered, or answered by a proposed revision or `given`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answered: Vec<String>,
    /// Areas of this moment that do not apply: (area, reason).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_applicable: Vec<(String, String)>,
    /// Areas of this moment deferred before; they are asked again, marked `deferred`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deferred: Vec<String>,
    /// At setup: built-in packs the facts fit and the project does not follow, each a yes/no.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub propose_packs: Vec<String>,
}

/// One option with its reasoning, for "Explain the options".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExplainedOption {
    pub id: String,
    pub label: String,
    pub why: String,
    pub fits_when: String,
    pub costs: String,
}

/// An area explained: every option's three texts and what in the project bears on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Explained {
    pub area: String,
    pub title: String,
    pub options: Vec<ExplainedOption>,
    /// The detected facts and files, each `<option>: <evidence>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bearing: Vec<String>,
}

/// How one `records` row reads in the dialog.
fn record_line(r: &Records, label: &str) -> String {
    let kind = match r.kind {
        Kind::Constraint => "constraint, with an ADR",
        Kind::Nfr => "nfr",
        Kind::NonGoal => "non-goal",
        Kind::Practice => "practice",
    };
    format!("{kind}: {}", r.text.replace("{label}", label))
}

/// The packs to ask from: those in the baseline in its order, a removed pack skipped, then any
/// the baseline does not list yet (setup, before the packs are recorded) in the order given.
pub(crate) fn followed<'a>(
    packs: &'a [(ProfileSource, ProfileFile)],
    baseline: &crate::baseline::Baseline,
) -> Vec<&'a (ProfileSource, ProfileFile)> {
    let mut out: Vec<&(ProfileSource, ProfileFile)> = Vec::new();
    for rec in &baseline.packs {
        if rec.removed.is_some() {
            continue;
        }
        if let Some(p) = packs.iter().find(|(s, _)| s.name == rec.source.name)
            && !out.iter().any(|(s, _)| s.name == p.0.name)
        {
            out.push(p);
        }
    }
    for p in packs {
        let listed = baseline.packs.iter().any(|r| r.source.name == p.0.name);
        if !listed && !out.iter().any(|(s, _)| s.name == p.0.name) {
            out.push(p);
        }
    }
    out
}

/// Every area's chosen options across the baseline's standing answers, its proposed revisions
/// and the answers given so far.
pub(crate) fn answer_set(
    baseline: &crate::baseline::Baseline,
    given: &[crate::baseline::Answer],
) -> BTreeMap<String, BTreeSet<String>> {
    use crate::baseline::{AreaState, RevisionState};
    let mut set: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut latest: BTreeMap<&str, &crate::baseline::AreaRecord> = BTreeMap::new();
    for rec in &baseline.areas {
        latest.insert(rec.id.as_str(), rec);
    }
    for rec in latest.values() {
        if rec.state == AreaState::Answered {
            set.entry(rec.id.clone())
                .or_default()
                .extend(rec.options.iter().map(|c| c.option.clone()));
        }
    }
    let proposed = baseline
        .revisions
        .iter()
        .filter(|r| r.state() == RevisionState::Proposed)
        .flat_map(|r| r.answers.iter());
    for a in proposed.chain(given.iter()) {
        if !a.options.is_empty() {
            set.entry(a.area.clone())
                .or_default()
                .extend(a.options.iter().map(|c| c.option.clone()));
        }
    }
    set
}

/// Whether an area's `when` holds against the answer set; an unanswered gating area never does.
pub(crate) fn when_holds(area: &Area, set: &BTreeMap<String, BTreeSet<String>>) -> bool {
    match &area.when {
        None => true,
        Some(w) => set
            .get(&w.area)
            .is_some_and(|chosen| w.options.iter().any(|o| chosen.contains(o))),
    }
}

/// The built-in packs the facts (and a detected CI service) fit that the project does not
/// follow: `parts` web-ui → web, api → api; `data` other than none → data; `production`
/// container → containers, kubernetes → kubernetes; a CI service detected → pipeline.
fn packs_to_propose(
    followed: &[&(ProfileSource, ProfileFile)],
    facts: Option<&crate::facts::Facts>,
    walk: Option<&Walk>,
) -> Vec<String> {
    let mut want: Vec<&str> = Vec::new();
    if let Some(f) = facts {
        if f.has("parts", "web-ui") {
            want.push("web");
        }
        if f.has("parts", "api") {
            want.push("api");
        }
        if f.list("data")
            .iter()
            .any(|v| v.trim().to_lowercase() != "none")
        {
            want.push("data");
        }
        if f.has("production", "container") {
            want.push("containers");
        }
        if f.has("production", "kubernetes") {
            want.push("kubernetes");
        }
    }
    // A CI service: an option of the `delivery` area, from a followed pack or else core's.
    let delivery = followed
        .iter()
        .find_map(|(_, f)| f.area("delivery").cloned())
        .or_else(|| builtin("core").and_then(|c| c.area("delivery").cloned()));
    if let Some(area) = delivery
        && detect_with(&area, walk, facts)
            .iter()
            .any(|d| d.option != "none-yet")
    {
        want.push("pipeline");
    }
    let mut out = Vec::new();
    for name in BUILTIN {
        if want.contains(&name) && !followed.iter().any(|(s, _)| s.name == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// What to ask the person now (R-1, R-2). `packs` are the packs the project follows; they are
/// taken in the baseline's order with a removed pack skipped, then area order. An area is listed
/// when its state (with `given` counted as answers) is open or deferred, its `when` holds against
/// every answer so far, and either `at` is setup and the area is asked at setup, or `at` is first
/// need and `design` touches it (a setup area still open included). Options are ordered detected
/// first, then the recommended one, then the rest; areas fall in dialogs of [`DIALOG_SIZE`].
/// With `root` None nothing is detected from the folder; with `facts` None nothing from the
/// facts record. Changes nothing.
#[allow(clippy::too_many_arguments)]
pub fn ask(
    packs: &[(ProfileSource, ProfileFile)],
    baseline: &crate::baseline::Baseline,
    given: &[crate::baseline::Answer],
    at: Asks,
    root: Option<&Path>,
    facts: Option<&crate::facts::Facts>,
    design: Option<&str>,
    item: &str,
) -> Ask {
    use crate::baseline::{AreaStatus, area_status};
    let followed = followed(packs, baseline);
    let walk = root.map(Walk::of);
    let set = answer_set(baseline, given);
    let mut out = Ask {
        packs: followed.iter().map(|(s, _)| s.clone()).collect(),
        at,
        item: item.to_string(),
        ask: Vec::new(),
        answered: Vec::new(),
        not_applicable: Vec::new(),
        deferred: Vec::new(),
        propose_packs: Vec::new(),
    };
    for (source, file) in &followed {
        for area in &file.areas {
            let touched = match (at, design) {
                (Asks::FirstNeed, Some(d)) => touches(area, d),
                _ => Vec::new(),
            };
            let in_scope = match at {
                Asks::Setup => area.asks == Asks::Setup,
                Asks::FirstNeed => !touched.is_empty(),
            };
            if !in_scope || !when_holds(area, &set) {
                continue;
            }
            if let Some(g) = given.iter().find(|g| g.area == area.id) {
                if let Some(reason) = &g.not_applicable {
                    out.not_applicable.push((area.id.clone(), reason.clone()));
                } else if g.later && g.options.is_empty() {
                    out.deferred.push(area.id.clone());
                } else {
                    out.answered.push(area.id.clone());
                }
                continue;
            }
            let deferred = match area_status(baseline, &area.id) {
                AreaStatus::Open => false,
                AreaStatus::Later => true,
                AreaStatus::Proposed(_) | AreaStatus::Answered(_) => {
                    out.answered.push(area.id.clone());
                    continue;
                }
                AreaStatus::NotApplicable(reason) => {
                    out.not_applicable.push((area.id.clone(), reason));
                    continue;
                }
            };
            if deferred {
                out.deferred.push(area.id.clone());
            }
            let detected = detect_with(area, walk.as_ref(), facts);
            let recommended = area.recommended().map(|o| o.id.clone());
            let is_detected = |id: &str| detected.iter().any(|d| d.option == id);
            let mut order: Vec<&AreaOption> = Vec::new();
            for d in &detected {
                if let Some(o) = area.option(&d.option) {
                    order.push(o);
                }
            }
            if let Some(r) = &recommended
                && !is_detected(r)
                && let Some(o) = area.option(r)
            {
                order.push(o);
            }
            for o in &area.options {
                if !order.iter().any(|x| x.id == o.id) {
                    order.push(o);
                }
            }
            let options = order
                .into_iter()
                .map(|o| AskedOption {
                    id: o.id.clone(),
                    label: o.label.clone(),
                    recommended: recommended.as_deref() == Some(o.id.as_str()),
                    detected: is_detected(&o.id),
                    why: o.why.clone(),
                    fits_when: o.fits_when.clone(),
                    costs: o.costs.clone(),
                    asks_value: o.asks_value.clone(),
                    records: o.records.iter().map(|r| record_line(r, &o.label)).collect(),
                })
                .collect();
            let index = out.ask.len();
            out.ask.push(AskedArea {
                id: area.id.clone(),
                pack: source.name.clone(),
                title: area.title.clone(),
                question: area.question.clone(),
                dialog: index / DIALOG_SIZE + 1,
                multi: area.select == Select::Many,
                options,
                detected,
                deferred,
                touched,
                records_to: RECORDS_TO.to_string(),
            });
        }
    }
    if at == Asks::Setup {
        out.propose_packs = packs_to_propose(&followed, facts, walk.as_ref());
    }
    out
}

/// Every option of `area` with its `why`, `fits_when` and `costs`, and what in the folder or the
/// facts record bears on it. Changes nothing.
pub fn explain(
    packs: &[(ProfileSource, ProfileFile)],
    area: &str,
    facts: Option<&crate::facts::Facts>,
    root: Option<&Path>,
) -> Result<Explained> {
    let Some(a) = packs.iter().find_map(|(_, f)| f.area(area)) else {
        return Err(problem(format!(
            "area '{area}' is in none of the packs this project follows: `kanbanr profile ask` \
             lists the areas"
        )));
    };
    let walk = root.map(Walk::of);
    let bearing = detect_with(a, walk.as_ref(), facts)
        .into_iter()
        .map(|d| format!("{}: {}", d.option, d.evidence))
        .collect();
    Ok(Explained {
        area: a.id.clone(),
        title: a.title.clone(),
        options: a
            .options
            .iter()
            .map(|o| ExplainedOption {
                id: o.id.clone(),
                label: o.label.clone(),
                why: o.why.clone(),
                fits_when: o.fits_when.clone(),
                costs: o.costs.clone(),
            })
            .collect(),
        bearing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;

    fn temp_store() -> (Store, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-profile-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        // L-41: the board is this test's own, under the system temp folder.
        assert!(store.data_dir().starts_with(std::env::temp_dir()));
        store
            .init_project("demo", ProjectConfig::default_for("demo"))
            .unwrap();
        (store, dir)
    }

    fn pack(yaml: &str) -> ProfileFile {
        ProfileFile::from_yaml(yaml).unwrap()
    }

    const TEAM: &str = r#"
profile: {name: team-web, description: Our web stack, version: 1}
areas:
  - id: house-style
    title: The house style
    asks: first_need
    touches: [screen]
    question: Which house style?
    options:
      - {id: ours, label: Ours, why: w, fits_when: f, costs: c}
"#;

    /// R-5: board, then personal, then built-in; `check_name` refuses a built-in pack's name;
    /// `versioned` bumps on a content change only and keeps the description.
    #[test]
    fn a_profile_name_resolves_board_then_personal_then_builtin_and_save_versions_a_change() {
        let (store, dir) = temp_store();
        let personal = dir.join("home-profiles");

        // Built in only.
        let (f, lib) = resolve(Some(&store), "core", Some(&personal)).unwrap();
        assert_eq!(lib, Library::Builtin);
        assert!(f.area("stack").is_some());
        assert!(resolve(Some(&store), "nothing-here", Some(&personal)).is_err());

        // A built-in name cannot be saved over.
        assert!(check_name("core").is_err());
        assert!(check_name("Bad Name").is_err());
        assert!(check_name("team-web").is_ok());
        assert!(store.save_profile("core", pack(TEAM), None).is_err());

        // Personal, then board, shadow in that order.
        let saved = save(&personal, "team-web", pack(TEAM), None).unwrap();
        assert_eq!(saved.version(), 1);
        let (_, lib) = resolve(Some(&store), "team-web", Some(&personal)).unwrap();
        assert_eq!(lib, Library::Personal);
        store.save_profile("team-web", pack(TEAM), None).unwrap();
        let (_, lib) = resolve(Some(&store), "team-web", Some(&personal)).unwrap();
        assert_eq!(lib, Library::Board);
        let (_, lib) = store.resolve_profile("team-web", Some(&personal)).unwrap();
        assert_eq!(lib, Library::Board);
        assert_eq!(
            store
                .list_profiles()
                .unwrap()
                .into_iter()
                .map(|(n, _)| n)
                .collect::<Vec<_>>(),
            vec!["team-web".to_string()]
        );

        // Saving the same content keeps the version; a change bumps it; the description stays.
        let again = store.save_profile("team-web", pack(TEAM), None).unwrap();
        assert_eq!(again.version(), 1);
        let mut changed = pack(TEAM);
        changed.areas[0].question = "Which house style, now?".into();
        changed.profile.as_mut().unwrap().description = String::new();
        let v2 = store
            .save_profile("team-web", changed.clone(), None)
            .unwrap();
        assert_eq!(v2.version(), 2);
        assert_eq!(v2.profile.as_ref().unwrap().description, "Our web stack");
        let (f, changed_flag) = versioned("team-web", changed, Some(&v2), None).unwrap();
        assert!(!changed_flag);
        assert_eq!(f.version(), 2);

        // A pack with a problem is not saved.
        let mut bad = pack(TEAM);
        bad.areas[0].options.clear();
        assert!(store.save_profile("team-web", bad, None).is_err());

        // Listing shows board and built-in packs.
        let rows = list(Some(&store), Some(&personal)).unwrap();
        assert!(
            rows.iter()
                .any(|r| r.name == "team-web" && r.library == Library::Board)
        );
        assert!(rows.iter().any(|r| r.name == "team-web"
            && r.library == Library::Personal
            && r.shadowed_by_board == Some(true)));
        assert!(
            rows.iter()
                .any(|r| r.name == "data" && r.library == Library::Builtin)
        );

        // Drift: applied v1, saved v2 changed the question only.
        let v1 = pack(TEAM);
        let src = v1.source("team-web", Library::Board);
        let d = drift(&src, &v1, Some(&v2));
        assert_eq!(d.newer, Some(2));
        assert!(!d.is_current());
        assert!(d.messages()[0].contains("kanbanr profile update team-web"));
        assert!(diff(&v1, &v2).contains(&"area 'house-style': question changed".to_string()));
        assert!(drift(&src, &v1, Some(&v1)).is_current());
        assert!(drift(&src, &v1, None).gone);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn kinds_named(problems: &[CoreError], needle: &str) -> usize {
        problems
            .iter()
            .filter(|p| p.to_string().contains(needle))
            .count()
    }

    /// R-5: validation names each problem kind once; then the ten built-in packs check clean.
    #[test]
    fn the_starter_profile_checks_clean_and_validation_names_each_problem() {
        let bad = pack(
            r#"
profile: {name: broken, version: 1, requires: [no-such-pack]}
areas:
  - id: Bad_Id
    title: t
    asks: setup
    question: q
    options: [{id: a, label: A}]
  - id: twice
    title: t
    asks: setup
    question: q
    options: [{id: a, label: A}]
  - id: twice
    title: t
    asks: setup
    when: {area: twice, options: [a]}
    question: q
    options: [{id: a, label: A}]
  - id: gated
    title: t
    asks: setup
    when: {area: nowhere, options: [a]}
    question: q
    options: [{id: a, label: A}]
  - id: gated-option
    title: t
    asks: setup
    when: {area: twice, options: [zzz]}
    question: q
    options: [{id: a, label: A}]
  - id: empty
    title: t
    asks: setup
    question: q
    options: []
  - id: two-recommended
    title: t
    asks: setup
    question: q
    options: [{id: a, label: A, recommended: true}, {id: b, label: B, recommended: true}]
  - id: reserved
    title: t
    asks: setup
    question: q
    options: [{id: later, label: Later}, {id: dup, label: D}, {id: dup, label: D}]
  - id: records
    title: t
    asks: setup
    question: q
    options:
      - id: a
        label: A
        records:
          - {kind: constraint, text: ""}
          - {kind: practice, text: x, iso25010: [Security]}
          - {kind: nfr, text: y, iso25010: [Usability]}
        adds:
          gate: sast
          release_gates: [{stage: "", gate: "Bad Gate"}]
        detect: ["../outside", {fact: colour, any: [red]}, {fact: parts, any: [spaceship]}]
  - id: never-asked
    title: t
    asks: first_need
    question: q
    options: [{id: a, label: A}]
"#,
        );
        let problems = bad.problems_in(None, None);
        for needle in [
            "an id is lower-case",
            "is defined twice",
            "names the area itself",
            "which no pack defines",
            "does not offer",
            "has no options",
            "more than one recommended",
            "is a reserved answer",
            "appears twice",
            "records an empty text",
            "`iso25010` is for an nfr",
            "is not an ISO 25010",
            "is not a readiness check",
            "is not a gate name",
            "names no stage",
            "without '..'",
            "which the facts record does not have",
            "is not a value of fact",
            "touches nothing",
            "which no library holds",
        ] {
            assert_eq!(
                kinds_named(&problems, needle),
                1,
                "{needle}: {:?}",
                problems.iter().map(|p| p.to_string()).collect::<Vec<_>>()
            );
        }
        // The context-free check cannot see the libraries, so it does not name the unknown pack.
        assert_eq!(kinds_named(&bad.problems(), "which no library holds"), 0);

        // A many area may carry asks_value options (coordinator, 17 Nov 2026); each chosen
        // option's value is checked on its own.
        let many = pack(
            r#"
areas:
  - id: ui
    title: t
    asks: setup
    select: many
    question: q
    options: [{id: lib, label: L, asks_value: the library}, {id: sys, label: S, asks_value: where}, {id: css, label: C}]
"#,
        );
        assert!(many.problems().is_empty());
        let ui = many.area("ui").unwrap();
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert!(
            value_problems(ui, &[s("lib", "MUI"), s("sys", "docs/ds"), s("css", "")]).is_empty()
        );
        assert_eq!(value_problems(ui, &[s("lib", ""), s("css", "x")]).len(), 2);

        // The ten built-in packs check clean, every one with its header and `core` required.
        for name in BUILTIN {
            let f = builtin(name).unwrap_or_else(|| panic!("built-in pack {name} does not parse"));
            let p = f.problems_in(None, None);
            assert!(
                p.is_empty(),
                "{name}: {:?}",
                p.iter().map(|p| p.to_string()).collect::<Vec<_>>()
            );
            assert_eq!(f.profile.as_ref().unwrap().name, name);
            if name != "core" {
                assert_eq!(f.requires(), ["core".to_string()]);
            }
        }
        // Shared rows (YAML anchors) read on every option that names them.
        let core = builtin("core").unwrap();
        let delivery = core.area("delivery").unwrap();
        for o in &delivery.options {
            let gates: Vec<&str> = o
                .adds
                .release_gates
                .iter()
                .map(|g| g.gate.as_str())
                .collect();
            assert_eq!(
                gates,
                if o.id == "none-yet" {
                    vec![]
                } else {
                    vec!["ci"]
                },
                "{}",
                o.id
            );
        }
    }

    /// R-5: a pack that defines an area another followed pack defines is refused, naming both;
    /// `requires` is honoured.
    #[test]
    fn packs_compose_with_unique_area_names() {
        let core = builtin("core").unwrap();
        let data = builtin("data").unwrap();
        // `data` requires `core`.
        let err = check_composition(&[], "data", &data)
            .unwrap_err()
            .to_string();
        assert!(err.contains("requires pack 'core'"), "{err}");
        check_composition(&[("core", &core)], "data", &data).unwrap();
        // Updating a pack is not a clash with itself.
        check_composition(&[("core", &core), ("data", &data)], "data", &data).unwrap();

        let clash = pack(
            r#"
profile: {name: our-data, version: 1, requires: [core]}
areas:
  - id: database
    title: Our database
    asks: first_need
    touches: [table]
    question: Which?
    options: [{id: ours, label: Ours}]
"#,
        );
        let err = check_composition(&[("core", &core), ("data", &data)], "our-data", &clash)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("'database'") && err.contains("'data'") && err.contains("'our-data'"),
            "{err}"
        );
        // Every built-in area id is unique across the ten packs.
        let mut seen = BTreeMap::new();
        for name in BUILTIN {
            for a in builtin(name).unwrap().areas {
                assert!(
                    seen.insert(a.id.clone(), name).is_none(),
                    "area {} in two built-in packs",
                    a.id
                );
            }
        }
    }

    /// A project folder of this test's own under the system temp folder (L-41), holding `files`
    /// (a trailing `/` makes a folder).
    fn temp_folder(files: &[&str]) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-profile-folder-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(dir.starts_with(std::env::temp_dir()));
        for f in files {
            let path = dir.join(f);
            if f.ends_with('/') {
                std::fs::create_dir_all(&path).unwrap();
            } else {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, "x\n").unwrap();
            }
        }
        dir
    }

    fn core_packs() -> Vec<(ProfileSource, ProfileFile)> {
        let core = builtin("core").unwrap();
        vec![(core.source("core", Library::Builtin), core)]
    }

    fn ids(ask: &Ask) -> Vec<&str> {
        ask.ask.iter().map(|a| a.id.as_str()).collect()
    }

    fn listed<'a>(ask: &'a Ask, id: &str) -> &'a AskedArea {
        ask.ask.iter().find(|a| a.id == id).unwrap()
    }

    /// R-1: at setup over `core`, a folder with `Cargo.toml` and a `Dockerfile` lists the setup
    /// areas whose `when` holds, `stack` detected as rust and `runs-where` as containers, each
    /// detected option first and marked, the recommended one first otherwise, in dialogs of four;
    /// answers given so far open the gated areas and drop the answered ones; `root: None` detects
    /// nothing.
    #[test]
    fn ask_lists_the_setup_areas_the_folder_does_not_answer_recommended_first_in_dialogs_of_four() {
        use crate::baseline::{Answer, AreaRecord, AreaState, Baseline, Chosen};
        let root = temp_folder(&["Cargo.toml", "Dockerfile"]);
        let packs = core_packs();
        let none = Baseline::default();

        let a = ask(&packs, &none, &[], Asks::Setup, Some(&root), None, None, "");
        assert_eq!(a.at, Asks::Setup);
        assert_eq!(a.packs.len(), 1);
        assert_eq!(a.packs[0].name, "core");
        // `iac` waits for `runs-where`, which nothing has answered yet.
        assert_eq!(
            ids(&a),
            [
                "project-kind",
                "stack",
                "runs-where",
                "delivery",
                "practice-baseline"
            ]
        );
        let dialogs: Vec<usize> = a.ask.iter().map(|x| x.dialog).collect();
        assert_eq!(dialogs, [1, 1, 1, 1, 2]);
        for area in &a.ask {
            assert!(area.records_to.contains("baseline"), "{}", area.records_to);
            assert_eq!(area.pack, "core");
            assert!(!area.deferred);
        }

        let stack = listed(&a, "stack");
        assert!(!stack.multi);
        assert_eq!(stack.detected.len(), 1);
        assert_eq!(stack.detected[0].option, "rust");
        assert_eq!(stack.detected[0].evidence, "Cargo.toml");
        assert_eq!(stack.options[0].id, "rust");
        assert!(stack.options[0].detected);
        assert!(stack.options[1..].iter().all(|o| !o.detected));
        assert!(
            stack.options[0]
                .records
                .iter()
                .any(|r| r
                    == "constraint, with an ADR: Rust is the implementation language and runtime"),
            "{:?}",
            stack.options[0].records
        );

        let runs = listed(&a, "runs-where");
        assert!(runs.multi, "a many area is asked as a multi-select");
        assert_eq!(runs.options[0].id, "containers");
        assert!(runs.options[0].detected);
        assert_eq!(runs.detected[0].evidence, "Dockerfile");

        // Nothing detected: the recommended option comes first, marked.
        let kind = listed(&a, "project-kind");
        assert!(kind.detected.is_empty());
        let core = builtin("core").unwrap();
        let rec = core.area("project-kind").unwrap().recommended().unwrap();
        assert_eq!(kind.options[0].id, rec.id);
        assert!(kind.options[0].recommended);
        assert_eq!(kind.options.iter().filter(|o| o.recommended).count(), 1);
        assert_eq!(
            kind.options.len(),
            core.area("project-kind").unwrap().options.len()
        );

        // Answers given so far: the answered areas drop out and the gated `iac` appears.
        let given = vec![
            Answer {
                area: "project-kind".into(),
                options: vec![Chosen::new("service", "")],
                ..Answer::default()
            },
            Answer {
                area: "runs-where".into(),
                options: vec![Chosen::new("containers", "")],
                ..Answer::default()
            },
        ];
        let b = ask(
            &packs,
            &none,
            &given,
            Asks::Setup,
            Some(&root),
            None,
            None,
            "",
        );
        assert_eq!(ids(&b), ["stack", "iac", "delivery", "practice-baseline"]);
        assert_eq!(b.answered, ["project-kind", "runs-where"]);
        assert!(b.ask.iter().all(|x| x.dialog == 1));

        // An area the baseline has answered is not asked; one deferred is asked again, marked.
        let mut answered = Baseline::default();
        for (id, state) in [
            ("stack", AreaState::Answered),
            ("delivery", AreaState::Later),
        ] {
            answered.areas.push(AreaRecord {
                id: id.into(),
                state,
                options: if state == AreaState::Answered {
                    vec![Chosen::new("rust", "")]
                } else {
                    vec![]
                },
                value: String::new(),
                reason: String::new(),
                item: String::new(),
                revision: "REV-001".into(),
                extra: Extra::default(),
            });
        }
        let c = ask(
            &packs,
            &answered,
            &[],
            Asks::Setup,
            Some(&root),
            None,
            None,
            "",
        );
        assert!(!ids(&c).contains(&"stack"));
        assert!(c.answered.contains(&"stack".to_string()));
        assert!(listed(&c, "delivery").deferred);
        assert_eq!(c.deferred, ["delivery"]);

        // No folder: nothing is detected, and the recommended option leads.
        let d = ask(&packs, &none, &[], Asks::Setup, None, None, None, "");
        assert!(d.ask.iter().all(|x| x.detected.is_empty()));
        assert!(d.ask.iter().all(|x| x.options.iter().all(|o| !o.detected)));
        assert!(listed(&d, "stack").options[0].recommended);

        // At first need with no design, nothing is asked.
        let e = ask(
            &packs,
            &none,
            &[],
            Asks::FirstNeed,
            Some(&root),
            None,
            None,
            "FTR-1",
        );
        assert!(e.ask.is_empty());
        assert_eq!(e.item, "FTR-1");

        // At setup, the packs the facts fit are proposed; a detected CI service proposes the
        // pipeline pack.
        let ci = temp_folder(&["Cargo.toml", ".github/workflows/"]);
        let facts = crate::facts::Facts {
            parts: vec!["web-ui".into(), "api".into()],
            data: vec!["none".into()],
            production: vec!["container".into()],
            ..crate::facts::Facts::default()
        };
        let f = ask(
            &packs,
            &none,
            &[],
            Asks::Setup,
            Some(&ci),
            Some(&facts),
            None,
            "",
        );
        assert_eq!(f.propose_packs, ["pipeline", "web", "api", "containers"]);
        assert_eq!(
            listed(&f, "delivery").detected[0].evidence,
            ".github/workflows/"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&ci);
    }

    /// R-1: `explain` gives every option's three texts and the facts and files that bear on the
    /// area, and changes nothing.
    #[test]
    fn ask_explains_an_area_with_why_fits_when_and_costs() {
        let root = temp_folder(&["Cargo.toml"]);
        let before: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        let packs = core_packs();
        let facts = crate::facts::Facts {
            languages: vec!["rust".into()],
            ..crate::facts::Facts::default()
        };
        let e = explain(&packs, "stack", Some(&facts), Some(&root)).unwrap();
        assert_eq!(e.area, "stack");
        assert_eq!(e.title, "The stack");
        let area = packs[0].1.area("stack").unwrap();
        assert_eq!(e.options.len(), area.options.len());
        for (x, o) in e.options.iter().zip(&area.options) {
            assert_eq!(x.id, o.id);
            assert_eq!(x.label, o.label);
            assert_eq!(x.why, o.why);
            assert_eq!(x.fits_when, o.fits_when);
            assert_eq!(x.costs, o.costs);
            assert!(!x.why.is_empty() && !x.fits_when.is_empty() && !x.costs.is_empty());
        }
        // The facts row is first in `detect`, so the facts are the evidence.
        assert_eq!(e.bearing, ["rust: facts: languages=rust"]);
        let files_only = explain(&packs, "stack", None, Some(&root)).unwrap();
        assert_eq!(files_only.bearing, ["rust: Cargo.toml"]);
        assert!(
            explain(&packs, "stack", None, None)
                .unwrap()
                .bearing
                .is_empty()
        );
        assert!(explain(&packs, "no-such-area", None, None).is_err());
        let after: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(before, after, "explain changes nothing");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// R-4: file tokens match by the path-character boundary, words whole and case-insensitive.
    #[test]
    fn touches_matches_file_tokens_by_path_boundary_and_words_whole() {
        let file = pack(
            r#"
areas:
  - id: database
    title: The database
    asks: first_need
    touches: ["*.tsx", migrations/, SQL, Postgres, docker-compose.yaml, load test]
    question: Which?
    options:
      - {id: a, label: A, why: w, fits_when: f, costs: c}
"#,
        );
        let area = file.area("database").unwrap();
        let t = |text: &str| touches(area, text);
        assert_eq!(t("The page is `src/App.tsx`."), ["*.tsx"]);
        assert!(t("Keep App.tsx.bak for now").is_empty());
        assert!(t("tsx files").is_empty());
        assert_eq!(t("Adds `migrations/0003_users/up.rs`"), ["migrations/"]);
        assert_eq!(t("under db/migrations/ as usual"), ["migrations/"]);
        assert_eq!(t("the migrations/ folder"), ["migrations/"]);
        assert!(t("premigrations/ is someone else's").is_empty());
        assert_eq!(t("one SQL query"), ["SQL"]);
        assert_eq!(t("plain sql, lower case"), ["SQL"]);
        assert!(
            t("PostgreSQL only").is_empty(),
            "SQL inside PostgreSQL is no match"
        );
        assert_eq!(t("POSTGRES 16"), ["Postgres"]);
        assert!(t("postgresql").is_empty());
        assert_eq!(
            t("```\nservices in docker-compose.yaml\n```"),
            ["docker-compose.yaml"]
        );
        assert!(t("my-docker-compose.yaml.orig").is_empty());
        assert_eq!(t("a Load Test before release"), ["load test"]);
        assert_eq!(
            t("SQL in `migrations/1.sql` and `src/Db.tsx`"),
            ["*.tsx", "SQL", "migrations/"]
        );
        assert!(t("").is_empty());
    }

    /// R-5: removing a pack keeps its entries and ADRs, stops asking and checking its areas, and
    /// adding it again restores it unchanged.
    #[test]
    fn a_removed_pack_keeps_its_entries_and_stops_asking() {
        use crate::baseline::{self, tests::*};
        let (store, dir, _) = board(true);
        for name in ["core", "data"] {
            let f = builtin(name).unwrap();
            baseline::add_pack(&store, "demo", name, &f, Library::Builtin, stamp()).unwrap();
        }
        let v = propose_answers(&store, "", vec![answer("database", &["postgresql"])]);
        accept_it(&store, &v);
        let design = "## Design\n\nAdds `migrations/0002.sql` and a query on the database.\n";
        let all: Vec<(ProfileSource, ProfileFile)> = ["core", "data"]
            .iter()
            .map(|n| {
                let f = builtin(n).unwrap();
                (f.source(n, Library::Builtin), f)
            })
            .collect();
        let before = baseline::load(&store, "demo").unwrap();
        assert!(!baseline::touched(&before, design).is_empty());
        let open: Vec<String> = baseline::open_areas(&all, &before, design)
            .into_iter()
            .map(|o| o.area)
            .collect();
        assert!(open.contains(&"migrations".to_string()), "{open:?}");
        let adrs = crate::adr::list(&store, "demo").unwrap().len();
        assert_eq!(adrs, 1);

        let held = baseline::remove_pack(&store, "demo", "core", person())
            .unwrap_err()
            .to_string();
        assert!(held.contains("requires pack 'core'"), "{held}");
        baseline::remove_pack(&store, "demo", "data", person()).unwrap();
        let b = baseline::load(&store, "demo").unwrap();
        assert_eq!(b.entries, before.entries, "entries stay");
        assert_eq!(
            baseline::in_force(&b).len(),
            baseline::in_force(&before).len()
        );
        assert_eq!(crate::adr::list(&store, "demo").unwrap().len(), adrs);
        assert!(baseline::touched(&b, design).is_empty());
        assert!(baseline::contradictions(&b, "## Design\n\nthe database is SQLite\n").is_empty());
        let data_areas: Vec<String> = builtin("data")
            .unwrap()
            .areas
            .iter()
            .map(|a| a.id.clone())
            .collect();
        assert!(
            baseline::open_areas(&all, &b, design)
                .iter()
                .all(|o| !data_areas.contains(&o.area))
        );
        let asked = ask(&all, &b, &[], Asks::FirstNeed, None, None, Some(design), "");
        assert!(
            asked.ask.iter().all(|a| !data_areas.contains(&a.id)),
            "{:?}",
            asked.ask
        );
        assert!(asked.answered.iter().all(|a| !data_areas.contains(a)));

        let f = builtin("data").unwrap();
        baseline::add_pack(&store, "demo", "data", &f, Library::Builtin, stamp()).unwrap();
        let again = baseline::load(&store, "demo").unwrap();
        assert_eq!(again.entries, before.entries);
        assert_eq!(again.areas, before.areas);
        assert_eq!(again.revisions, before.revisions);
        assert!(again.packs.iter().all(|p| p.removed.is_none()));
        assert!(!baseline::touched(&again, design).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
