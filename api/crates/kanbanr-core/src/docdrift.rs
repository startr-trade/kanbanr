//! Docs and items that fell behind the code (FTR-013).
//!
//! When an item changes a command, a flag, a route, a setting, a stage or a file, the docs and
//! items linked to it may still describe the old thing. This module finds the texts that *name*
//! something an item changed and have not changed since, so a person can read them. It flags and
//! never judges: a flag says which text names which changed name, not that the text is wrong.
//!
//! Four steps, each deterministic:
//! 1. [`item_changes`]: the names in the diff of the commits that reference the item;
//! 2. [`linked`]: the docs and items linked to it, from the board alone;
//! 3. [`scan`]: which of the names a text holds, as whole tokens, with their lines;
//! 4. [`flag_for`] / [`flags_for_item`]: a text is flagged for the names changed after it last did.
//!
//! Nothing is stored. Everything is derived from the board's files and the repository's history
//! on each read (ADR-0005).

use crate::error::{CoreError, Result};
use crate::refs::{self, BoardIndex, Ref};
use crate::{Store, coderepo};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

// ---- names ----------------------------------------------------------------------------------

/// What kind of thing a [`Name`] is; it decides how a text must hold it to match (see [`scan`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum NameKind {
    Flag,
    Command,
    Env,
    Route,
    ConfigKey,
    Name,
    File,
}

/// Something a change put in the code that a text may describe.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Name {
    pub kind: NameKind,
    pub text: String,
}

impl Name {
    fn new(kind: NameKind, text: impl Into<String>) -> Self {
        Name {
            kind,
            text: text.into(),
        }
    }
}

/// Words too common to mean anything as a config key or a stage/status/check name. Dropped from
/// [`NameKind::ConfigKey`] and [`NameKind::Name`].
pub const GENERIC: &[&str] = &[
    "id", "code", "name", "text", "kind", "state", "status", "title", "items", "rev", "note",
    "path", "type", "data", "true", "false", "none", "some", "file", "value", "error", "result",
    "config", "project", "release", "sprint", "stage",
];

/// File base names that say nothing about which file is meant: matched by their full path only.
pub const COMMON_FILES: &[&str] = &[
    "main.rs",
    "lib.rs",
    "mod.rs",
    "index.md",
    "README.md",
    "Cargo.toml",
    "package.json",
    "openapi.json",
];

/// The published API description, read for routes only: it is where a route change surfaces.
const OPENAPI: &str = "docs/src/reference/openapi.json";

/// Is this repository path documentation (so it contributes no names)? Anything under `docs/`,
/// Markdown, a changelog, an image, a lockfile.
pub fn is_documentation(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    lower.starts_with("docs/")
        || base.ends_with(".md")
        || base.starts_with("changelog")
        || base.ends_with(".lock")
        || base == "package-lock.json"
        || [
            ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".ico", ".pdf",
        ]
        .iter()
        .any(|e| base.ends_with(e))
}

fn is_word_byte(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn kept(word: &str) -> bool {
    word.chars().count() >= 4 && !GENERIC.contains(&word.to_ascii_lowercase().as_str())
}

/// `DocsUpdated` -> `docs_updated`, `DocsUpdated` -> `docs-updated` with `sep` `-`.
fn split_camel(word: &str, sep: char) -> String {
    let mut out = String::new();
    for (i, c) in word.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push(sep);
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `{x}` and `<x>` become `{}`; a query or fragment is dropped.
fn normalize_route(route: &str) -> String {
    let route = route.split(['?', '#']).next().unwrap_or(route);
    let mut out = String::new();
    let mut close: Option<char> = None;
    for c in route.chars() {
        match (close, c) {
            (Some(end), c) if c == end => {
                close = None;
                out.push_str("{}");
            }
            (Some(_), _) => {}
            (None, '{') => close = Some('}'),
            (None, '<') => close = Some('>'),
            (None, c) => out.push(c),
        }
    }
    out
}

/// Long flags in a line: `--` + a letter + letters, digits or `-`, not preceded by a letter, a
/// digit or `-`, at least 4 characters with the dashes.
fn long_flags(line: &str, out: &mut BTreeSet<Name>) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i + 2 < chars.len() {
        let starts = chars[i] == '-'
            && chars[i + 1] == '-'
            && (i == 0 || !(is_word_byte(chars[i - 1]) || chars[i - 1] == '-'))
            && chars[i + 2].is_ascii_alphabetic();
        if !starts {
            i += 1;
            continue;
        }
        let mut end = i + 2;
        while end < chars.len() && (is_word_byte(chars[end]) || chars[end] == '-') {
            end += 1;
        }
        let token: String = chars[i..end].iter().collect();
        let token = token.trim_end_matches('-');
        if token.len() >= 4 {
            out.insert(Name::new(NameKind::Flag, token));
        }
        i = end;
    }
}

fn env_names(line: &str, out: &mut BTreeSet<Name>) {
    const PREFIX: &str = "KANBANR_";
    let mut from = 0;
    while let Some(at) = line[from..].find(PREFIX) {
        let start = from + at;
        let before_ok = line[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'));
        let mut end = start + PREFIX.len();
        while line[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            end += 1;
        }
        if before_ok && end > start + PREFIX.len() {
            out.insert(Name::new(NameKind::Env, &line[start..end]));
        }
        from = end.max(start + 1);
    }
}

/// `"/projects/…"` strings.
fn route_strings(line: &str, out: &mut BTreeSet<Name>) {
    let mut from = 0;
    while let Some(at) = line[from..].find("\"/projects/") {
        let start = from + at + 1;
        let Some(len) = line[start..].find('"') else {
            break;
        };
        let route = normalize_route(&line[start..start + len]);
        if route.len() > "/projects/".len() {
            out.insert(Name::new(NameKind::Route, route));
        }
        from = start + len + 1;
    }
}

/// The word of an enum variant line: `indent` spaces exactly, an UpperCamelCase word, then one of
/// `ends`. Returns the word.
fn variant_word<'a>(line: &'a str, indents: &[usize], ends: &[&str]) -> Option<&'a str> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if !indents.contains(&indent) {
        return None;
    }
    let rest = &line[indent..];
    let word_len = rest
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    let (word, after) = rest.split_at(word_len);
    let mut chars = word.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_uppercase()) {
        return None;
    }
    ends.iter()
        .any(|e| {
            after.trim_end() == *e
                || (e.len() > 1 && after.starts_with(e))
                || (*e == "(" && after.starts_with('('))
        })
        .then_some(word)
}

/// State carried between the lines of one file in a patch.
#[derive(Default)]
struct FileState {
    /// Inside a `#[arg(` attribute that has not closed yet.
    in_arg: bool,
    /// A bare `long` was seen: the next field line gives the flag.
    long_pending: bool,
}

fn attr_long(content: &str, state: &mut FileState, out: &mut BTreeSet<Name>) {
    let t = content.trim();
    if t.contains("#[arg(") {
        state.in_arg = true;
    }
    if state.in_arg {
        let mut rest = t;
        while let Some(at) = rest.find("long") {
            let before_ok = rest[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
            let after = rest[at + 4..].trim_start();
            if before_ok {
                if let Some(value) = after.strip_prefix('=') {
                    if let Some(name) = value.trim_start().strip_prefix('"')
                        && let Some(name) = name.split('"').next()
                    {
                        out.insert(Name::new(NameKind::Flag, format!("--{name}")));
                    }
                } else if after.is_empty() || after.starts_with([',', ')']) {
                    state.long_pending = true;
                }
            }
            rest = &rest[at + 4..];
        }
        if t.contains(")]") {
            state.in_arg = false;
        }
        return;
    }
    if state.long_pending && !t.is_empty() && !t.starts_with("#[") && !t.starts_with("//") {
        let t = t.strip_prefix("pub ").unwrap_or(t);
        if let Some((field, _)) = t.split_once(':') {
            let field = field.trim();
            if !field.is_empty() && field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.insert(Name::new(
                    NameKind::Flag,
                    format!("--{}", field.replace('_', "-")),
                ));
            }
        }
        state.long_pending = false;
    }
}

/// `const NAME: &str = "value"`: the value.
fn const_value(content: &str) -> Option<&str> {
    let t = content.trim_start();
    let t = t
        .strip_prefix("pub(crate) ")
        .or_else(|| t.strip_prefix("pub "))
        .unwrap_or(t);
    let rest = t.strip_prefix("const ")?;
    let (_, value) = rest.split_once(": &str = \"")?;
    value.split('"').next()
}

fn base_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The names one added or removed line holds, for the file it is in.
fn names_in_line(path: &str, content: &str, state: &mut FileState, out: &mut BTreeSet<Name>) {
    let base = base_of(path);
    let rust = path.ends_with(".rs");
    if path == OPENAPI {
        // Only a route key: `"/projects/{p}/x": {`.
        let t = content.trim();
        if let Some(rest) = t.strip_prefix("\"/")
            && let Some((key, after)) = rest.split_once('"')
            && after.trim_start().starts_with(':')
            && after.contains('{')
        {
            out.insert(Name::new(
                NameKind::Route,
                normalize_route(&format!("/{key}")),
            ));
        }
        return;
    }
    long_flags(content, out);
    env_names(content, out);
    if rust || path.ends_with(".ts") || path.ends_with(".tsx") {
        route_strings(content, out);
    }
    if rust {
        attr_long(content, state, out);
        if let Some(value) = const_value(content)
            && kept(value)
        {
            out.insert(Name::new(NameKind::Name, value));
        }
    }
    if path.ends_with("kanbanr-cli/src/main.rs") {
        if let Some(word) = variant_word(content, &[4], &[" {", "(", ","]) {
            out.insert(Name::new(NameKind::Command, split_camel(word, '-')));
        }
        if let Some((_, rest)) = content.split_once("#[command(name = \"")
            && let Some(name) = rest.split('"').next()
        {
            out.insert(Name::new(NameKind::Command, name));
        }
    }
    if matches!(base, "config.rs" | "models.rs") && content.starts_with("    pub ") {
        let rest = &content["    pub ".len()..];
        if !rest.starts_with(' ')
            && let Some((field, _)) = rest.split_once(':')
            && field
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            && kept(field)
        {
            out.insert(Name::new(NameKind::ConfigKey, field));
        }
    }
    if base == "readiness.rs"
        && let Some(word) = variant_word(content, &[4, 8], &[","])
    {
        let name = split_camel(word, '_');
        if kept(&name) {
            out.insert(Name::new(NameKind::Name, name));
        }
    }
    if path.contains("/presets/") && path.ends_with(".yaml") {
        let t = content.trim();
        let item = if let Some(item) = t.strip_prefix("- ") {
            (!item.contains(':')).then_some(item)
        } else {
            t.strip_suffix(':')
                .or_else(|| t.split_once(": ").map(|(k, _)| k))
        };
        if let Some(item) = item {
            let item = item.trim().trim_matches(['"', '\'']);
            if kept(item) && !item.contains('#') {
                out.insert(Name::new(NameKind::Name, item));
            }
        }
    }
}

/// The names a patch put in the code, by pattern. The input is the text of `git log -p -U0` for
/// one commit (or any unified diff); lines of documentation files are ignored.
pub fn names_in_diff(patch: &str) -> BTreeSet<Name> {
    let mut out = BTreeSet::new();
    let mut file: Option<String> = None;
    let mut header = false;
    let mut state = FileState::default();
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            header = true;
            state = FileState::default();
            file = rest.rsplit_once(" b/").map(|(_, b)| b.to_string());
            if let Some(path) = file.as_deref()
                && !is_documentation(path)
            {
                out.insert(Name::new(NameKind::File, path));
            }
            continue;
        }
        if line.starts_with("@@") {
            header = false;
            state = FileState::default();
            continue;
        }
        if header {
            continue;
        }
        let Some(path) = file.as_deref() else {
            continue;
        };
        let Some(content) = line.strip_prefix(['+', '-']) else {
            continue;
        };
        if path != OPENAPI && is_documentation(path) {
            continue;
        }
        names_in_line(path, content, &mut state, &mut out);
    }
    out
}

// ---- what an item changed -------------------------------------------------------------------

/// The names one commit of an item changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub sha: String,
    /// The commit's committer time, RFC 3339.
    pub at: String,
    /// The item whose `Refs:` trailer names the commit.
    pub item: String,
    pub names: BTreeSet<Name>,
}

/// An item code is letters, `-`, digits: nothing else reaches a git argument.
fn valid_code(code: &str) -> bool {
    code.split_once('-').is_some_and(|(p, n)| {
        !p.is_empty()
            && p.chars().all(|c| c.is_ascii_alphabetic())
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
    })
}

/// A `--since` value: a date or a date and time. One git argument, never interpolated into a shell.
pub fn valid_since(since: &str) -> bool {
    !since.is_empty()
        && since.len() <= 40
        && since
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | '+' | '.' | 'T' | 'Z' | ' '))
        && since.chars().next().is_some_and(|c| c.is_ascii_digit())
}

fn parse_log(out: &str, item: Option<&str>) -> Vec<Change> {
    let mut changes = Vec::new();
    for part in out
        .split("\n@@@")
        .flat_map(|p| p.strip_prefix("@@@").or(Some(p)))
    {
        let (header, patch) = part.split_once('\n').unwrap_or((part, ""));
        let Some((sha, at)) = header.trim().split_once(' ') else {
            continue;
        };
        let names = names_in_diff(patch);
        if names.is_empty() {
            continue;
        }
        changes.push(Change {
            sha: sha.to_string(),
            at: at.trim().to_string(),
            item: item.unwrap_or("").to_string(),
            names,
        });
    }
    changes
}

/// The names in the diff of every non-merge commit whose `Refs:` trailer names `code`, newest
/// first. A commit that changed no recognised name is left out.
pub fn item_changes(repo: &Path, code: &str, since: Option<&str>) -> Result<Vec<Change>> {
    if !valid_code(code) {
        return Err(CoreError::Refused(format!(
            "{code:?} is not an item code (letters, a dash, digits)"
        )));
    }
    let grep = format!("--grep=^Refs:.*kanbanr:{code}([^0-9A-Za-z]|$)");
    let mut args = vec![
        "log",
        "--no-merges",
        "--all",
        "-E",
        &grep,
        "--format=@@@%H %cI",
        "-p",
        "-U0",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
    ];
    let since_arg;
    if let Some(since) = since {
        if !valid_since(since) {
            return Err(CoreError::Refused(format!(
                "{since:?} is not a date (try 2026-11-01)"
            )));
        }
        since_arg = format!("--since={since}");
        args.push(&since_arg);
    }
    let out = coderepo::git(repo, &args)?;
    Ok(parse_log(&out, Some(code)))
}

// ---- scanning a text ------------------------------------------------------------------------

/// A name a text holds, with the lines it is on (1-based).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Hit {
    pub name: Name,
    pub lines: Vec<usize>,
}

/// Is `needle` in `hay` with a character neither `before` nor `after` rejects on either side?
fn bounded(
    hay: &str,
    needle: &str,
    bad_before: impl Fn(char) -> bool,
    bad_after: impl Fn(char) -> bool,
) -> bool {
    if needle.is_empty() {
        return false;
    }
    hay.match_indices(needle).any(|(at, _)| {
        let before = hay[..at].chars().next_back();
        let after = hay[at + needle.len()..].chars().next();
        !before.is_some_and(&bad_before) && !after.is_some_and(&bad_after)
    })
}

/// The inline code spans of a line: text between a pair of single backticks.
fn code_spans(line: &str) -> Vec<&str> {
    let parts: Vec<&str> = line.split('`').collect();
    parts
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 1 && *i + 1 < parts.len())
        .map(|(_, p)| *p)
        .collect()
}

fn name_on_line(name: &Name, line: &str, spans: &[&str], route_line: &str) -> bool {
    match name.kind {
        NameKind::Flag => bounded(
            line,
            &name.text,
            |c| c.is_ascii_alphanumeric() || c == '-',
            |c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
        ),
        NameKind::Env => bounded(
            line,
            &name.text,
            |c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_',
            |c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_',
        ),
        NameKind::Route => bounded(
            route_line,
            &name.text,
            |_| false,
            |c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '{' | '-'),
        ),
        NameKind::Command => {
            line.contains("kanbanr ")
                && bounded(
                    line,
                    &name.text,
                    |c| c.is_ascii_alphanumeric() || c == '-',
                    |c| c.is_ascii_alphanumeric() || c == '-',
                )
        }
        NameKind::ConfigKey | NameKind::Name => spans.iter().any(|span| {
            bounded(
                span,
                &name.text,
                |c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'),
                |c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'),
            )
        }),
        NameKind::File => {
            let path_char =
                |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-');
            if bounded(line, &name.text, path_char, path_char) {
                return true;
            }
            let base = base_of(&name.text);
            base != name.text
                && base.len() >= 8
                && !COMMON_FILES.contains(&base)
                && bounded(line, base, path_char, path_char)
        }
    }
}

/// Which of `names` a text holds, by the rule of each kind (a config key, check, stage or status
/// name only inside a code span or fenced block; a subcommand only on a line that says `kanbanr`;
/// a flag, variable, route or file as a whole token anywhere). Pure; sorted by name.
pub fn scan(text: &str, names: &BTreeSet<Name>) -> Vec<Hit> {
    let mut found: BTreeMap<&Name, Vec<usize>> = BTreeMap::new();
    let mut in_fence = false;
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        let spans = if in_fence {
            vec![line]
        } else {
            code_spans(line)
        };
        let route_line = normalize_route_line(line);
        for name in names {
            if name_on_line(name, line, &spans, &route_line) {
                found.entry(name).or_default().push(n + 1);
            }
        }
    }
    found
        .into_iter()
        .map(|(name, lines)| Hit {
            name: name.clone(),
            lines,
        })
        .collect()
}

/// A line with each `{…}` and `<…>` replaced by `{}`, to meet a normalised route.
fn normalize_route_line(line: &str) -> String {
    if !line.contains('/') {
        return String::new();
    }
    let mut out = String::new();
    let mut close: Option<char> = None;
    for c in line.chars() {
        match (close, c) {
            (Some(end), c) if c == end => {
                close = None;
                out.push_str("{}");
            }
            (Some(_), _) => {}
            (None, '{') => close = Some('}'),
            (None, '<') => close = Some('>'),
            (None, c) => out.push(c),
        }
    }
    // An unclosed bracket swallowed the rest of the line: fall back to the line as written.
    if close.is_some() {
        line.to_string()
    } else {
        out
    }
}

// ---- what is linked -------------------------------------------------------------------------

/// What kind of text a flag points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Doc,
    Adr,
    Item,
}

/// A text linked to an item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Target {
    pub kind: TargetKind,
    /// The doc path, the ADR id or the item code.
    pub id: String,
    pub title: String,
    /// The doc path under the board's `docs/`, for a doc or an ADR; empty for an item.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

impl Target {
    /// How a sign-off names this text: `doc:<path>`, `adr:<id>` or `item:<CODE>`.
    pub fn key(&self) -> String {
        let kind = match self.kind {
            TargetKind::Doc => "doc",
            TargetKind::Adr => "adr",
            TargetKind::Item => "item",
        };
        format!("{kind}:{}", self.id)
    }
}

/// A linked text and why it is linked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Linked {
    pub target: Target,
    /// Sorted, without repeats.
    pub via: Vec<String>,
}

fn doc_title(path: &str, text: &str) -> String {
    crate::docs::split_front_matter(text)
        .1
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .map(|t| t.trim().to_string())
        .unwrap_or_else(|| crate::docs::title_from_stem(base_of(path).trim_end_matches(".md")))
}

/// The docs and items linked to `code`, from the board alone: its `affects_docs`; the docs whose
/// front matter lists it (or one of its requirements) under `items` or `refs`; the ADRs that
/// affect it; the docs and items whose text links it; and the items its own spec links. The item
/// itself is never in the list.
pub fn linked(store: &Store, project: &str, code: &str) -> Result<Vec<Linked>> {
    let p = store.load(project)?;
    let feature = p.feature(code)?;
    let index = BoardIndex::of(store, &p)?;
    let mut out: BTreeMap<(TargetKind, String), (Target, BTreeSet<String>)> = BTreeMap::new();
    let mut add = |target: Target, via: &str| {
        out.entry((target.kind, target.id.clone()))
            .or_insert_with(|| (target, BTreeSet::new()))
            .1
            .insert(via.to_string());
    };

    let adrs = crate::adr::list(store, project)?;
    let adr_at = |path: &str| adrs.iter().find(|a| a.path == path);
    let mut docs: Vec<(String, String)> = Vec::new();
    for path in store.list_docs(project)? {
        if let Ok(text) = store.read_doc(project, &path) {
            docs.push((path, text));
        }
    }
    let doc_target = |path: &str, text: &str| -> Target {
        match adr_at(path) {
            Some(adr) => Target {
                kind: TargetKind::Adr,
                id: adr.id.clone(),
                title: if adr.title.is_empty() {
                    adr.id.clone()
                } else {
                    adr.title.clone()
                },
                path: path.to_string(),
            },
            None => Target {
                kind: TargetKind::Doc,
                id: path.to_string(),
                title: doc_title(path, text),
                path: path.to_string(),
            },
        }
    };

    let mentions = |entry: &str| entry == code || entry.split('/').next() == Some(code);
    let is_this = |r: &Ref| matches!(r, Ref::Item { code: c, .. } if c == code);

    // 1. affects_docs.
    if let Some(def) = &feature.definition {
        for declared in &def.affects_docs {
            let key = refs::doc_key(declared);
            if let Some((path, text)) = docs.iter().find(|(p, _)| *p == key) {
                add(doc_target(path, text), "affects_docs");
            }
        }
    }
    for (path, text) in &docs {
        // 2. Front matter.
        let listed = ["items", "refs"]
            .iter()
            .any(|k| crate::docs::front_list(text, k).iter().any(|e| mentions(e)));
        if listed {
            add(doc_target(path, text), "front matter items");
        }
        // 4. A link in the doc.
        if refs::find(crate::docs::split_front_matter(text).1, &index)
            .iter()
            .any(|f| is_this(&f.reference))
        {
            add(doc_target(path, text), "link in the doc");
        }
    }
    // 3. ADRs.
    for adr in adrs.iter().filter(|a| a.concerns(code)) {
        if let Some((path, text)) = docs.iter().find(|(p, _)| *p == adr.path) {
            add(doc_target(path, text), "ADR affects");
        }
    }
    // 5. Items.
    let item_target = |f: &crate::FeatureItem| Target {
        kind: TargetKind::Item,
        id: f.code.clone(),
        title: f.title.clone(),
        path: String::new(),
    };
    for other in &p.features {
        if other.code == code || p.config.is_no_op(&other.status) {
            continue;
        }
        if refs::find(&other.specification, &index)
            .iter()
            .any(|f| is_this(&f.reference))
        {
            add(item_target(other), "link in the spec of that item");
        }
    }
    for found in refs::find(&feature.specification, &index) {
        if let Ref::Item { code: c, .. } = &found.reference {
            if c == code {
                continue;
            }
            if let Some(other) = p.features.iter().find(|f| f.code == *c)
                && !p.config.is_no_op(&other.status)
            {
                add(item_target(other), "linked from the spec");
            }
        }
    }

    Ok(out
        .into_values()
        .map(|(target, via)| Linked {
            target,
            via: via.into_iter().collect(),
        })
        .collect())
}

// ---- flags ----------------------------------------------------------------------------------

/// A commit that changed a flagged name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CommitRef {
    pub sha: String,
    pub at: String,
    pub item: String,
    /// The names this commit changed that the text holds.
    pub names: Vec<Name>,
}

/// A linked text that names something changed after it last changed. A flag says which text names
/// which changed name, never that the text is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Flag {
    pub target: Target,
    pub via: Vec<String>,
    pub hits: Vec<Hit>,
    pub commits: Vec<CommitRef>,
    /// When the text last changed, RFC 3339.
    pub target_changed_at: String,
}

fn instant(t: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(t.trim(), &Rfc3339).ok()
}

/// Flag `linked` if its `text` (last changed at `changed_at`) holds a name that a change after
/// that moment put in the code. Pure.
pub fn flag_for(linked: &Linked, text: &str, changed_at: &str, changes: &[Change]) -> Option<Flag> {
    let since = instant(changed_at)?;
    let later: Vec<&Change> = changes
        .iter()
        .filter(|c| instant(&c.at).is_some_and(|at| at > since))
        .collect();
    if later.is_empty() {
        return None;
    }
    // One scan over every name changed after the text; then attribute each hit to its commits.
    let names: BTreeSet<Name> = later.iter().flat_map(|c| c.names.iter().cloned()).collect();
    let hits = scan(text, &names);
    if hits.is_empty() {
        return None;
    }
    let mut commits: Vec<CommitRef> = later
        .iter()
        .filter(|c| hits.iter().any(|h| c.names.contains(&h.name)))
        .map(|c| CommitRef {
            sha: c.sha.clone(),
            at: c.at.clone(),
            item: c.item.clone(),
            names: hits
                .iter()
                .filter(|h| c.names.contains(&h.name))
                .map(|h| h.name.clone())
                .collect(),
        })
        .collect();
    commits.sort_by(|a, b| (&a.at, &a.sha).cmp(&(&b.at, &b.sha)));
    commits.dedup();
    Some(Flag {
        target: linked.target.clone(),
        via: linked.via.clone(),
        hits,
        commits,
        target_changed_at: changed_at.to_string(),
    })
}

impl Flag {
    /// What to call the flagged text on a terminal.
    pub fn heading(&self) -> String {
        match self.target.kind {
            TargetKind::Doc => self.target.path.clone(),
            TargetKind::Adr => format!("{} ({})", self.target.id, self.target.path),
            TargetKind::Item => format!("{}: {}", self.target.id, self.target.title),
        }
    }

    /// One line per name the text holds, for a terminal: the name, the commit that changed it and
    /// when the text last changed. It says what to read, never that the text is wrong.
    pub fn lines(&self) -> Vec<String> {
        let day = |t: &str| t.get(..10).unwrap_or(t).to_string();
        let what = match self.target.kind {
            TargetKind::Doc | TargetKind::Adr => "doc",
            TargetKind::Item => "item",
        };
        self.hits
            .iter()
            .map(|h| {
                let name = match h.name.kind {
                    NameKind::ConfigKey | NameKind::Name => format!("`{}`", h.name.text),
                    _ => h.name.text.clone(),
                };
                let by = self
                    .commits
                    .iter()
                    .find(|c| c.names.contains(&h.name))
                    .map(|c| {
                        format!(
                            "changed by {} ({}, {})",
                            c.item,
                            c.sha.get(..7).unwrap_or(&c.sha),
                            day(&c.at)
                        )
                    })
                    .unwrap_or_default();
                format!(
                    "names {name}  {by}; this {what} last changed {}",
                    day(&self.target_changed_at)
                )
            })
            .collect()
    }
}

/// When the files last changed in the board's repository: the committer time of the last commit
/// that touched any of them, or now when one is modified and not yet committed (a change written
/// but not committed counts, as for `docs_updated`). `None` for files with no history, which are
/// never flagged.
pub fn changed_at(board: &Path, files: &[&Path]) -> Option<String> {
    let paths: Vec<String> = files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let tail = |head: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = head.iter().map(|s| s.to_string()).collect();
        v.push("--".into());
        v.extend(paths.iter().cloned());
        v
    };
    let run = |args: Vec<String>| {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        coderepo::git(board, &refs).ok()
    };
    if run(tail(&["status", "--porcelain"])).is_some_and(|s| !s.is_empty()) {
        return OffsetDateTime::now_utc().format(&Rfc3339).ok();
    }
    run(tail(&["log", "-1", "--format=%cI"])).filter(|s| !s.is_empty())
}

/// The flags for one item: every linked text that names something the item changed after the text
/// last changed. Needs the code repository (`repo`). Sorted by target kind and id.
pub fn flags_for_item(store: &Store, project: &str, code: &str, repo: &Path) -> Result<Vec<Flag>> {
    let changes = item_changes(repo, code, None)?;
    flags_for_changes(store, project, code, &changes)
}

/// [`flags_for_item`] over changes already read (the board-wide report reads the log once).
pub fn flags_for_changes(
    store: &Store,
    project: &str,
    code: &str,
    changes: &[Change],
) -> Result<Vec<Flag>> {
    if changes.is_empty() {
        return Ok(Vec::new());
    }
    let p = store.load(project)?;
    let board = store.data_dir();
    let mut flags = Vec::new();
    for l in linked(store, project, code)? {
        let (text, files) = match l.target.kind {
            TargetKind::Item => {
                let Ok(f) = p.feature(&l.target.id) else {
                    continue;
                };
                let yaml = store.feature_path(project, &f.status, &f.code);
                let spec = yaml
                    .parent()
                    .map(|d| d.join("features-spec").join(format!("{}.md", f.code)));
                let mut files = vec![yaml];
                files.extend(spec);
                (f.specification.clone(), files)
            }
            _ => {
                let Ok(text) = store.read_doc(project, &l.target.path) else {
                    continue;
                };
                let file = store.project_dir(project).join("docs").join(&l.target.path);
                (text, vec![file])
            }
        };
        let refs: Vec<&Path> = files.iter().map(|f| f.as_path()).collect();
        let Some(at) = changed_at(board, &refs) else {
            continue;
        };
        if let Some(flag) = flag_for(&l, &text, &at, changes) {
            flags.push(flag);
        }
    }
    flags.sort_by(|a, b| (a.target.kind, &a.target.id).cmp(&(b.target.kind, &b.target.id)));
    Ok(flags)
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    // ---- repository fixtures ----

    pub(crate) fn scratch(tag: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kanbanr-docdrift-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn git_env(dir: &Path, args: &[&str], date: Option<&str>) -> String {
        let mut cmd = Command::new("git");
        cmd.args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
        if let Some(d) = date {
            cmd.env("GIT_COMMITTER_DATE", d).env("GIT_AUTHOR_DATE", d);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    pub(crate) fn code_commit(dir: &Path, file: &str, body: &str, msg: &str, date: &str) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        git_env(dir, &["add", "-A"], None);
        git_env(dir, &["commit", "-q", "-m", msg], Some(date));
    }

    pub(crate) fn board(tag: &str) -> (Store, PathBuf) {
        let dir = scratch(tag);
        crate::git::ensure_repo_as(&dir, Some(("Ada", "ada@example.com")));
        let store = Store::new(dir.clone());
        store
            .init_project("demo", crate::ProjectConfig::default_for("demo"))
            .unwrap();
        store
            .add_milestone("demo", "M", "", vec![], Some("M".into()))
            .unwrap();
        (store, dir)
    }

    pub(crate) fn commit_board(dir: &Path) {
        assert!(crate::git::commit(dir, "Ada", "ada@example.com", "board"));
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use std::path::PathBuf;

    fn name(kind: NameKind, text: &str) -> Name {
        Name::new(kind, text)
    }

    fn has(names: &BTreeSet<Name>, kind: NameKind, text: &str) -> bool {
        names.contains(&name(kind, text))
    }

    fn file_patch(path: &str, lines: &[&str]) -> String {
        let mut s = format!(
            "diff --git a/{path} b/{path}\nindex 111..222 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,0 +1,1 @@\n"
        );
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }

    /// FTR-013 R-1: one of each row of the table, and the near-misses that are not names.
    #[test]
    fn names_are_read_from_a_diff_by_pattern() {
        let cli = file_patch(
            "api/crates/kanbanr-cli/src/main.rs",
            &[
                "+    /// Look back.",
                "+    #[arg(long = \"since\")]",
                "+    since: Option<String>,",
                "+    #[arg(",
                "+        long,",
                "+        help = \"x\"",
                "+    )]",
                "+    allow_writes: bool,",
                "+    Drift {",
                "+    DocsFolded(Args),",
                "+    #[command(name = \"sign-off\")]",
                "+        let h = \"--other-flag\"; // KANBANR_BIND and `--` and -x and --a and a--b",
                "-    Old,",
            ],
        );
        let names = names_in_diff(&cli);
        for (k, t) in [
            (NameKind::Flag, "--since"),
            (NameKind::Flag, "--allow-writes"),
            (NameKind::Flag, "--other-flag"),
            (NameKind::Command, "drift"),
            (NameKind::Command, "docs-folded"),
            (NameKind::Command, "old"),
            (NameKind::Command, "sign-off"),
            (NameKind::Env, "KANBANR_BIND"),
            (NameKind::File, "api/crates/kanbanr-cli/src/main.rs"),
        ] {
            assert!(has(&names, k, t), "{k:?} {t}: {names:?}");
        }
        for t in ["--", "-x", "--a", "a--b"] {
            assert!(
                !names
                    .iter()
                    .any(|n| n.kind == NameKind::Flag && n.text == t),
                "{t} is not a flag: {names:?}"
            );
        }

        let routes = file_patch(
            "api/crates/kanbanr-server/src/routes.rs",
            &[
                "+    .route(\"/projects/{p}/review\", get(review))",
                "+let word = \"projects\";",
            ],
        );
        let ts = file_patch(
            "web/src/api.ts",
            &["+  `fetch(\"/projects/<id>/drift?since=x\")`"],
        );
        let r = names_in_diff(&format!("{routes}{ts}"));
        assert!(has(&r, NameKind::Route, "/projects/{}/review"), "{r:?}");
        assert!(has(&r, NameKind::Route, "/projects/{}/drift"), "{r:?}");

        let cfg = file_patch(
            "api/crates/kanbanr-core/src/config.rs",
            &[
                "+    pub release_choices: Vec<String>,",
                "+    pub status: String,",
                "+    pub id: String,",
                "+        pub nested_field: u8,",
                "+const GATE: &str = \"docs-folded\";",
            ],
        );
        let c = names_in_diff(&cfg);
        assert!(has(&c, NameKind::ConfigKey, "release_choices"), "{c:?}");
        assert!(has(&c, NameKind::Name, "docs-folded"), "{c:?}");
        for t in ["status", "id", "nested_field"] {
            assert!(
                !c.iter()
                    .any(|n| n.kind == NameKind::ConfigKey && n.text == t),
                "{t}: {c:?}"
            );
        }

        let ready = file_patch(
            "api/crates/kanbanr-core/src/readiness.rs",
            &["+    DocsUpdated,", "+        Approved,", "+  DocsBad,"],
        );
        let preset = file_patch(
            "api/crates/kanbanr-core/presets/togaf.yaml",
            &["+  - In Progress", "+  Review:"],
        );
        let n = names_in_diff(&format!("{ready}{preset}"));
        assert!(has(&n, NameKind::Name, "docs_updated"), "{n:?}");
        assert!(has(&n, NameKind::Name, "approved"), "{n:?}");
        assert!(has(&n, NameKind::Name, "In Progress"), "{n:?}");
        assert!(has(&n, NameKind::Name, "Review"), "{n:?}");
        assert!(!has(&n, NameKind::Name, "docs_bad"), "wrong indent: {n:?}");
    }

    /// FTR-013 R-1: documentation contributes no names, except routes from the published API.
    #[test]
    fn documentation_files_contribute_no_names() {
        let patch = format!(
            "{}{}{}{}",
            file_patch("docs/src/a.md", &["+Use --allow-writes and KANBANR_BIND."]),
            file_patch("CHANGELOG.md", &["+- `--since` added"]),
            file_patch("Cargo.lock", &["+name = \"--oddly-named\""]),
            file_patch(
                "docs/src/reference/openapi.json",
                &[
                    "+    \"/projects/{project}/drift\": {",
                    "+      \"description\": \"--not-a-route KANBANR_X\",",
                    "+      \"$ref\": \"#/components/schemas/Thing\"",
                ],
            ),
        );
        let names = names_in_diff(&patch);
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            vec![name(NameKind::Route, "/projects/{}/drift")]
        );
        assert!(is_documentation("docs/x/y.png"));
        assert!(!is_documentation("api/src/lib.rs"));
    }

    #[test]
    fn generic_words_are_lowercase_and_dropped() {
        for w in GENERIC {
            assert_eq!(*w, w.to_ascii_lowercase());
            assert!(!kept(w), "{w}");
        }
        assert!(kept("release_choices"));
        assert!(!kept("abc"));
    }

    fn set(names: &[Name]) -> BTreeSet<Name> {
        names.iter().cloned().collect()
    }

    /// FTR-013 R-3: whole tokens, with their lines; each kind by its own rule.
    #[test]
    fn a_text_naming_a_changed_name_is_flagged_with_its_lines() {
        let text = "# T\n\nRun `kanbanr drift --since 2026-01-01` now.\nNothing here.\nSet KANBANR_BIND=0.0.0.0 and\nsee api/crates/kanbanr-core/src/review.rs and review.rs again.\nGET /projects/acme/review works; /projects/{p}/review too.\n";
        let hits = scan(
            text,
            &set(&[
                name(NameKind::Flag, "--since"),
                name(NameKind::Command, "drift"),
                name(NameKind::Env, "KANBANR_BIND"),
                name(NameKind::File, "api/crates/kanbanr-core/src/review.rs"),
                name(NameKind::Route, "/projects/{}/review"),
                name(NameKind::Flag, "--sinc"),
                name(NameKind::Env, "KANBANR_BIN"),
            ]),
        );
        let lines = |k: NameKind, t: &str| {
            hits.iter()
                .find(|h| h.name.kind == k && h.name.text == t)
                .map(|h| h.lines.clone())
        };
        assert_eq!(lines(NameKind::Flag, "--since"), Some(vec![3]));
        assert_eq!(lines(NameKind::Command, "drift"), Some(vec![3]));
        assert_eq!(lines(NameKind::Env, "KANBANR_BIND"), Some(vec![5]));
        assert_eq!(
            lines(NameKind::File, "api/crates/kanbanr-core/src/review.rs"),
            Some(vec![6])
        );
        assert_eq!(lines(NameKind::Route, "/projects/{}/review"), Some(vec![7]));
        assert_eq!(
            lines(NameKind::Flag, "--sinc"),
            None,
            "a prefix is not the token"
        );
        assert_eq!(lines(NameKind::Env, "KANBANR_BIN"), None);
    }

    /// FTR-013 R-3: a text the item updated after the change was folded: not flagged.
    #[test]
    fn a_text_changed_after_the_commit_is_not_flagged() {
        let l = Linked {
            target: Target {
                kind: TargetKind::Doc,
                id: "a.md".into(),
                title: "A".into(),
                path: "a.md".into(),
            },
            via: vec!["affects_docs".into()],
        };
        let change = Change {
            sha: "abc".into(),
            at: "2026-11-07T10:00:00+00:00".into(),
            item: "FTR-1".into(),
            names: set(&[name(NameKind::Flag, "--since")]),
        };
        let text = "uses --since here\n";
        let before = flag_for(
            &l,
            text,
            "2026-11-02T09:00:00+00:00",
            std::slice::from_ref(&change),
        )
        .unwrap();
        assert_eq!(before.hits[0].lines, vec![1]);
        assert_eq!(before.commits[0].sha, "abc");
        assert_eq!(before.target_changed_at, "2026-11-02T09:00:00+00:00");
        // The same instant in another zone, after the commit.
        assert!(
            flag_for(
                &l,
                text,
                "2026-11-07T16:00:00+05:30",
                std::slice::from_ref(&change)
            )
            .is_none()
        );
        assert!(
            flag_for(
                &l,
                text,
                "2026-11-08T00:00:00+00:00",
                std::slice::from_ref(&change)
            )
            .is_none()
        );
        // An unreadable time is never flagged.
        assert!(flag_for(&l, text, "yesterday", &[change]).is_none());
    }

    /// FTR-013 R-3: a config key, check or status name in prose is quiet; in a code span or a
    /// fenced block it matches; a subcommand needs a line that says `kanbanr`.
    #[test]
    fn code_span_only_names_are_not_matched_in_prose() {
        let text = "The release_choices setting is described here.\nSet `release_choices` to a list.\n```yaml\ndocs-folded: yes\n```\nSigned off docs-folded in prose.\nThe drift of a boat.\nRun kanbanr drift.\nA `release_choices_x` span is another word.\n";
        let hits = scan(
            text,
            &set(&[
                name(NameKind::ConfigKey, "release_choices"),
                name(NameKind::Name, "docs-folded"),
                name(NameKind::Command, "drift"),
            ]),
        );
        let lines = |t: &str| {
            hits.iter()
                .find(|h| h.name.text == t)
                .unwrap()
                .lines
                .clone()
        };
        assert_eq!(lines("release_choices"), vec![2]);
        assert_eq!(lines("docs-folded"), vec![4]);
        assert_eq!(lines("drift"), vec![8]);
    }

    /// FTR-013 R-1: only the non-merge commits whose Refs trailer names the item.
    #[test]
    fn item_changes_come_from_commits_that_reference_the_item() {
        let dir = scratch("changes");
        git_env(&dir, &["init", "-q", "-b", "main"], None);
        code_commit(
            &dir,
            "src/a.rs",
            "let a = \"--first-flag\";\n",
            "one\n\nRefs: kanbanr:FTR-1",
            "2026-11-01T10:00:00+00:00",
        );
        code_commit(
            &dir,
            "src/b.rs",
            "let b = \"--other-flag\";\n",
            "two\n\nRefs: kanbanr:FTR-12",
            "2026-11-02T10:00:00+00:00",
        );
        code_commit(
            &dir,
            "src/c.rs",
            "let c = \"--bare-flag\";\n",
            "three, no trailer",
            "2026-11-03T10:00:00+00:00",
        );
        code_commit(
            &dir,
            "src/d.rs",
            "let d = 1;\n",
            "four\n\nRefs: kanbanr:FTR-1/R-2",
            "2026-11-04T10:00:00+00:00",
        );
        git_env(&dir, &["checkout", "-q", "-b", "side"], None);
        code_commit(
            &dir,
            "src/e.rs",
            "let e = \"--side-flag\";\n",
            "five\n\nRefs: kanbanr:FTR-1",
            "2026-11-05T10:00:00+00:00",
        );
        git_env(&dir, &["checkout", "-q", "main"], None);
        code_commit(
            &dir,
            "src/f.rs",
            "let f = \"--main-flag\";\n",
            "six\n\nRefs: kanbanr:FTR-9",
            "2026-11-05T11:00:00+00:00",
        );
        git_env(
            &dir,
            &[
                "merge",
                "--no-ff",
                "-q",
                "-m",
                "merge\n\nRefs: kanbanr:FTR-1",
                "side",
            ],
            Some("2026-11-06T10:00:00+00:00"),
        );

        let changes = item_changes(&dir, "FTR-1", None).unwrap();
        let found: BTreeSet<&str> = changes
            .iter()
            .flat_map(|c| c.names.iter())
            .filter(|n| n.kind == NameKind::Flag)
            .map(|n| n.text.as_str())
            .collect();
        assert_eq!(
            found,
            BTreeSet::from(["--first-flag", "--side-flag"]),
            "{changes:?}"
        );
        assert_eq!(
            changes.len(),
            3,
            "FTR-1's three commits, FTR-12's and the merge left out"
        );
        assert!(
            changes
                .iter()
                .all(|c| c.item == "FTR-1" && c.at.starts_with("2026-11-0"))
        );

        let recent = item_changes(&dir, "FTR-1", Some("2026-11-04")).unwrap();
        assert_eq!(recent.len(), 1);
        assert!(item_changes(&dir, "FTR-1; rm", None).is_err());
        assert!(item_changes(&dir, "FTR-1", Some("--output=x")).is_err());
        assert!(valid_since("2026-11-01"));
        assert!(!valid_since("--x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FTR-013 R-2: the five sources, each target once with its reasons.
    #[test]
    fn linked_texts_come_from_affects_front_matter_adr_and_links() {
        let (store, dir) = board("linked");
        let item = store
            .add_feature(
                "demo",
                "Drift",
                "Links [Other](/p/demo/feature/FEAT-2).",
                "M",
                Some("FEAT-1".into()),
            )
            .unwrap();
        store
            .add_feature(
                "demo",
                "Other",
                "See [Drift](/p/demo/feature/FEAT-1).",
                "M",
                Some("FEAT-2".into()),
            )
            .unwrap();
        store
            .add_feature("demo", "Third", "Unrelated.", "M", Some("FEAT-3".into()))
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(crate::models::FeatureDefinition {
                    statement: "Find drift".into(),
                    affects_docs: vec!["design/data.md".into(), "design/missing".into()],
                    ..Default::default()
                }),
            )
            .unwrap();
        store
            .write_doc("demo", "design/data.md", "# Data\n\nbody\n")
            .unwrap();
        store
            .write_doc(
                "demo",
                "design/front.md",
                "---\nitems: [FEAT-1/R-1]\n---\n\n# Front\n",
            )
            .unwrap();
        store
            .write_doc(
                "demo",
                "design/listed.md",
                "---\nrefs:\n  - FEAT-1\n  - G-1\n---\n\n# Listed\n",
            )
            .unwrap();
        store
            .write_doc(
                "demo",
                "design/other-item.md",
                "---\nitems: [FEAT-12]\n---\n\n# Not this one\n",
            )
            .unwrap();
        store
            .write_doc(
                "demo",
                "design/linked.md",
                "# Linked\n\nSee [FEAT-1](/p/demo/feature/FEAT-1) and `design/data.md`.\n",
            )
            .unwrap();
        let mut adr =
            crate::adr::create(&store, "demo", "Keep it", crate::adr::Adr::default()).unwrap();
        adr.affects = vec!["FEAT-1".into()];
        crate::adr::update(&store, "demo", &adr).unwrap();
        commit_board(&dir);

        let found = linked(&store, "demo", "FEAT-1").unwrap();
        let by_id: BTreeMap<String, Vec<String>> = found
            .iter()
            .map(|l| (l.target.id.clone(), l.via.clone()))
            .collect();
        assert_eq!(by_id["design/data.md"], vec!["affects_docs"]);
        assert_eq!(by_id["design/front.md"], vec!["front matter items"]);
        assert_eq!(by_id["design/listed.md"], vec!["front matter items"]);
        assert_eq!(by_id["design/linked.md"], vec!["link in the doc"]);
        assert_eq!(by_id[&adr.id], vec!["ADR affects"]);
        assert_eq!(
            by_id["FEAT-2"],
            vec!["link in the spec of that item", "linked from the spec"]
        );
        for absent in ["design/other-item.md", "design/missing", "FEAT-1", "FEAT-3"] {
            assert!(!by_id.contains_key(absent), "{absent}: {by_id:?}");
        }
        assert_eq!(found.len(), by_id.len(), "each target once");
        let adr_target = found.iter().find(|l| l.target.id == adr.id).unwrap();
        assert_eq!(adr_target.target.kind, TargetKind::Adr);
        assert_eq!(adr_target.target.title, "Keep it");
        assert!(found.windows(2).all(|w| (w[0].target.kind, &w[0].target.id) <= (w[1].target.kind, &w[1].target.id)));
        assert_eq!(
            crate::docs::front_list("---\nitems: FEAT-1\n---\nx", "items"),
            vec!["FEAT-1"]
        );
        assert!(crate::docs::front_list("no front matter", "items").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A board with FEAT-1 and a doc naming `--since`, and a code repository where FEAT-1 changed
    /// `--since` at `when`.
    fn drift_fixture(tag: &str, when: &str) -> (Store, PathBuf, PathBuf) {
        let (store, dir) = board(tag);
        let item = store
            .add_feature("demo", "Drift", "Spec.", "M", Some("FEAT-1".into()))
            .unwrap();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(crate::models::FeatureDefinition {
                    statement: "Find drift".into(),
                    affects_docs: vec!["design/data.md".into(), "design/quiet.md".into()],
                    ..Default::default()
                }),
            )
            .unwrap();
        store
            .write_doc(
                "demo",
                "design/data.md",
                "# Data\n\nRun with --since to look back.\n",
            )
            .unwrap();
        store
            .write_doc("demo", "design/quiet.md", "# Quiet\n\nNothing named.\n")
            .unwrap();
        commit_board(&dir);
        let repo = scratch(&format!("{tag}-code"));
        git_env(&repo, &["init", "-q", "-b", "main"], None);
        code_commit(
            &repo,
            "src/a.rs",
            "let a = \"--since\";\n",
            "add\n\nRefs: kanbanr:FEAT-1",
            when,
        );
        (store, dir, repo)
    }

    /// FTR-013 R-3, R-4: the board, the history and the rule give the same flags every time.
    #[test]
    fn the_same_board_and_history_give_the_same_flags() {
        let (store, dir, repo) = drift_fixture("same", "2099-01-01T00:00:00+00:00");
        let first = flags_for_item(&store, "demo", "FEAT-1", &repo).unwrap();
        let second = flags_for_item(&store, "demo", "FEAT-1", &repo).unwrap();
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(first[0].target.id, "design/data.md");
        assert_eq!(first[0].hits[0].name.text, "--since");
        assert_eq!(first[0].hits[0].lines, vec![3]);
        assert_eq!(first[0].commits[0].item, "FEAT-1");
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );

        // Written but not committed counts as changed now; the change of 2099 is still later.
        store
            .write_doc(
                "demo",
                "design/data.md",
                "# Data\n\nRun with --since, folded.\n",
            )
            .unwrap();
        assert_eq!(
            flags_for_item(&store, "demo", "FEAT-1", &repo)
                .unwrap()
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);

        // A change made before the doc's last commit is already folded.
        let (store, dir, repo) = drift_fixture("old", "2000-01-01T00:00:00+00:00");
        assert!(
            flags_for_item(&store, "demo", "FEAT-1", &repo)
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }
}

#[cfg(test)]
mod perf {
    use super::testkit::{board, commit_board, git_env, scratch};
    use super::*;
    use std::process::Command;

    /// FTR-013 R-10: one item on a history of 2,500 commits with 40 linked texts, under 2 seconds.
    /// Run in release: `cargo test -p kanbanr-core --release docdrift::perf -- --ignored`.
    #[test]
    #[ignore]
    fn one_item_on_a_long_history() {
        let (store, dir) = board("perf");
        let item = store
            .add_feature("demo", "Drift", "Spec.", "M", Some("FEAT-1".into()))
            .unwrap();
        let docs: Vec<String> = (0..40).map(|i| format!("design/d{i}.md")).collect();
        store
            .set_feature_definition(
                "demo",
                &item.code,
                Some(crate::models::FeatureDefinition {
                    statement: "Find drift".into(),
                    affects_docs: docs.clone(),
                    ..Default::default()
                }),
            )
            .unwrap();
        for (i, d) in docs.iter().enumerate() {
            let body: String = (0..200)
                .map(|n| format!("Line {n} of {i}: run `kanbanr drift --since` or --flag-{n}.\n"))
                .collect();
            store
                .write_doc("demo", d, &format!("# D{i}\n\n{body}"))
                .unwrap();
        }
        commit_board(&dir);

        let repo = scratch("perf-code");
        git_env(&repo, &["init", "-q", "-b", "main"], None);
        let mut stream = String::new();
        for i in 0..2500u32 {
            let msg = if i % 50 == 0 {
                format!("c{i}\n\nRefs: kanbanr:FEAT-1\n")
            } else {
                format!("c{i}\n\nRefs: kanbanr:FEAT-{}\n", 100 + i)
            };
            let body = format!("let f = \"--flag-{i}\";\nlet n = {i};\n");
            stream.push_str(&format!(
                "commit refs/heads/main\ncommitter T <t@example.com> {} +0000\ndata {}\n{msg}\nM 100644 inline src/f{}.rs\ndata {}\n{body}\n",
                4_000_000_000u64 + u64::from(i), msg.len(), i % 40, body.len()
            ));
        }
        use std::io::Write;
        let mut child = Command::new("git")
            .args(["fast-import", "--quiet"])
            .current_dir(&repo)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stream.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());

        let started = std::time::Instant::now();
        let flags = flags_for_item(&store, "demo", "FEAT-1", &repo).unwrap();
        let took = started.elapsed();
        assert_eq!(flags.len(), 40, "every doc names --since or a changed flag");
        assert!(took < std::time::Duration::from_secs(2), "took {took:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }
}
