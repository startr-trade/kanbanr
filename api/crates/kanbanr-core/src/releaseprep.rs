//! The text a release prepares itself with (FEAT-212): the version in each version file, the
//! dated changelog heading, the notes listing. Pure functions over file contents; reading the
//! files from the release tip and committing them is `releases` and `coderepo`'s.
//!
//! A version file is matched with a **template**, not a regular expression: literal text with
//! exactly one `{version}` (which matches `[0-9A-Za-z.+-]+`), where every run of whitespace matches
//! any run of whitespace, newlines included, and a leading `^` anchors to the start of a line
//! (leading spaces allowed).

/// What a template or a path may name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholders {
    /// The release version verbatim, e.g. `v0.2.1`.
    pub version: String,
    /// The version with one leading `v` removed when a digit follows, e.g. `0.2.1`.
    pub number: String,
    /// The release's name.
    pub name: String,
    /// `YYYY-MM-DD`.
    pub date: String,
}

impl Placeholders {
    pub fn new(version: &str, name: &str, date: &str) -> Result<Placeholders, String> {
        Ok(Placeholders {
            version: version.trim().to_string(),
            number: number_of(version)?,
            name: name.trim().to_string(),
            date: date.to_string(),
        })
    }
}

fn is_version_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'+' || b == b'-'
}

/// The version as files carry it: one leading `v` dropped when a digit follows (`v0.2.1` →
/// `0.2.1`). Only `[0-9A-Za-z.+-]` may appear, so a version can never reach a file or a ref as
/// anything else.
pub fn number_of(version: &str) -> Result<String, String> {
    let v = version.trim();
    let refuse = || {
        Err(format!(
            "'{version}' cannot be a release version to prepare: use only letters, digits and . + -"
        ))
    };
    if v.is_empty() || !v.bytes().all(is_version_byte) {
        return refuse();
    }
    let number = match v.strip_prefix('v') {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => v,
    };
    Ok(number.to_string())
}

/// `{version}`, `{number}`, `{name}` and `{date}` filled in.
pub fn expand(template: &str, p: &Placeholders) -> String {
    template
        .replace("{version}", &p.version)
        .replace("{number}", &p.number)
        .replace("{name}", &p.name)
        .replace("{date}", &p.date)
}

/// A path `prepare` may write: relative to the repository root, never outside it, never in `.git`.
pub fn check_repo_path(path: &str) -> Result<(), String> {
    let p = path.trim();
    if p.is_empty() {
        return Err("a path is needed".into());
    }
    let bytes = p.as_bytes();
    if p.starts_with('/') || p.starts_with('\\') || (bytes.len() > 1 && bytes[1] == b':') {
        return Err(format!(
            "'{p}' is absolute: give it relative to the repository root"
        ));
    }
    if p.split(['/', '\\']).any(|s| s == "..") {
        return Err(format!("'{p}' reaches outside the repository (`..`)"));
    }
    if p == ".git" || p.starts_with(".git/") || p.starts_with(".git\\") {
        return Err(format!("'{p}' is inside .git"));
    }
    Ok(())
}

enum Tok {
    Lit(u8),
    Ws,
    Version,
}

struct Template {
    anchored: bool,
    toks: Vec<Tok>,
}

fn parse(template: &str) -> Template {
    let (anchored, rest) = match template.strip_prefix('^') {
        Some(rest) => (true, rest),
        None => (false, template),
    };
    let bytes = rest.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if rest[i..].starts_with("{version}") {
            toks.push(Tok::Version);
            i += "{version}".len();
        } else if bytes[i].is_ascii_whitespace() {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            toks.push(Tok::Ws);
        } else {
            toks.push(Tok::Lit(bytes[i]));
            i += 1;
        }
    }
    Template { anchored, toks }
}

/// Match `toks` at `pos`: the end of the match and where `{version}` sat in it.
fn match_at(text: &[u8], toks: &[Tok], pos: usize) -> Option<(usize, Option<(usize, usize)>)> {
    let Some(first) = toks.first() else {
        return Some((pos, None));
    };
    match first {
        Tok::Lit(b) => (text.get(pos) == Some(b))
            .then(|| match_at(text, &toks[1..], pos + 1))
            .flatten(),
        Tok::Ws => {
            let mut end = pos;
            while end < text.len() && text[end].is_ascii_whitespace() {
                end += 1;
            }
            match_at(text, &toks[1..], end)
        }
        Tok::Version => {
            let mut end = pos;
            while end < text.len() && is_version_byte(text[end]) {
                end += 1;
            }
            (pos + 1..=end).rev().find_map(|e| {
                match_at(text, &toks[1..], e).map(|(m, inner)| (m, inner.or(Some((pos, e)))))
            })
        }
    }
}

/// Every non-overlapping match of `template` in `text` (or the first), as the span `{version}`
/// matched.
fn find(text: &str, template: &str, first_only: bool) -> Vec<(usize, usize)> {
    let t = parse(template);
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let start = if t.anchored {
            let line_start = pos == 0 || bytes[pos - 1] == b'\n';
            if !line_start {
                pos += 1;
                continue;
            }
            let mut s = pos;
            while s < bytes.len() && (bytes[s] == b' ' || bytes[s] == b'\t') {
                s += 1;
            }
            s
        } else {
            pos
        };
        match match_at(bytes, &t.toks, start) {
            Some((end, Some(span))) if end > start => {
                out.push(span);
                if first_only {
                    break;
                }
                pos = end;
            }
            _ => pos += 1,
        }
    }
    out
}

/// The template a version file is matched with: its own, else the built-in one for its kind.
fn template_for(path: &str, pattern: Option<&str>) -> Result<(String, bool), String> {
    if let Some(p) = pattern {
        return Ok((p.to_string(), false));
    }
    if path.ends_with(".toml") {
        Ok(("^version = \"{version}\"".into(), true))
    } else if path.ends_with(".json") {
        Ok(("\"version\": \"{version}\"".into(), true))
    } else {
        Err(format!(
            "{path}: give a match (only .toml and .json files have a default), e.g. \
             `--version-file '{path}=VERSION = \"{{version}}\"'`"
        ))
    }
}

/// `text` with its version set to `number`: every match of an explicit `pattern`, or the first of
/// the built-in one. No match is a refusal naming the file.
pub fn set_version(
    text: &str,
    path: &str,
    pattern: Option<&str>,
    number: &str,
) -> Result<String, String> {
    let (template, first_only) = template_for(path, pattern)?;
    let spans = find(text, &template, first_only);
    if spans.is_empty() {
        return Err(format!(
            "{path}: '{}' matches nothing, so its version cannot be set",
            template.replace('\n', "\\n")
        ));
    }
    let mut out = text.to_string();
    for (s, e) in spans.into_iter().rev() {
        out.replace_range(s..e, number);
    }
    Ok(out)
}

/// Whether `text` already names `number` where `set_version` would put it (the `prepared` check).
pub fn names_version(text: &str, path: &str, pattern: Option<&str>, number: &str) -> bool {
    let Ok((template, first_only)) = template_for(path, pattern) else {
        return false;
    };
    let spans = find(text, &template, first_only);
    !spans.is_empty() && spans.iter().all(|&(s, e)| &text[s..e] == number)
}

/// Whether the changelog already has the heading `## [{number}]`.
pub fn has_heading(text: &str, number: &str) -> bool {
    let want = format!("## [{number}]");
    text.lines().any(|l| l.trim_start().starts_with(&want))
}

/// The changelog with `## [{number}] - {date}` below its first `## [Unreleased]` line, so what was
/// unreleased belongs to the release, and its compare links moved on. `None` when the heading is
/// already there or there is no Unreleased heading to date.
pub fn date_changelog(text: &str, p: &Placeholders) -> Result<Option<String>, String> {
    if has_heading(text, &p.number) {
        return Ok(None);
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(at) = lines
        .iter()
        .position(|l| l.trim().to_ascii_lowercase().starts_with("## [unreleased]"))
    else {
        return Ok(None);
    };
    let mut out = String::with_capacity(text.len() + 64);
    for (i, line) in lines.iter().enumerate() {
        if let Some((label, url)) = unreleased_link(line) {
            let (base, prev) = url;
            out.push_str(&format!("{label}: {base}{}...HEAD\n", p.version));
            out.push_str(&format!("[{}]: {base}{prev}...{}\n", p.number, p.version));
            continue;
        }
        out.push_str(line);
        if i == at {
            if !line.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("\n## [{}] - {}\n", p.number, p.date));
        }
    }
    Ok(Some(out))
}

/// `[Unreleased]: <base>compare/<prev>...HEAD` as its label, and the base (through `compare/`)
/// and the previous version.
fn unreleased_link(line: &str) -> Option<(&str, (&str, &str))> {
    let t = line.trim_end();
    let (label, url) = t.split_once(':')?;
    if !label.trim().eq_ignore_ascii_case("[unreleased]") {
        return None;
    }
    let url = url.trim();
    let at = url.find("compare/")? + "compare/".len();
    let prev = url[at..].strip_suffix("...HEAD")?;
    Some((label.trim(), (&url[..at], prev)))
}

/// `text` with `entry` directly below the first line equal (trimmed) to `after`. `None` when the
/// entry is already there; a refusal when `after` is not.
pub fn add_listing(text: &str, after: &str, entry: &str) -> Result<Option<String>, String> {
    if text.lines().any(|l| l.trim() == entry.trim()) {
        return Ok(None);
    }
    let mut out = String::with_capacity(text.len() + entry.len() + 1);
    let mut placed = false;
    for line in text.split_inclusive('\n') {
        out.push_str(line);
        if !placed && line.trim() == after.trim() {
            if !line.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(entry);
            out.push('\n');
            placed = true;
        }
    }
    if !placed {
        return Err(format!(
            "has no line '{}' to list the notes below",
            after.trim()
        ));
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(version: &str) -> Placeholders {
        Placeholders::new(version, "Gates", "2026-10-21").unwrap()
    }

    /// FEAT-212 R-8: the built-in template sets the first match; an explicit one sets every match,
    /// across lines; a template that matches nothing is a refusal naming the file.
    #[test]
    fn version_template_sets_first_or_every_match() {
        assert_eq!(number_of("v0.2.1").unwrap(), "0.2.1");
        assert_eq!(number_of("0.2.1").unwrap(), "0.2.1");
        assert_eq!(number_of("vNext").unwrap(), "vNext", "no digit follows");
        assert!(number_of("v1;rm").is_err());
        assert!(number_of("v1 2").is_err());

        let toml = "[workspace.package]\nversion = \"0.2.0\"\n\n[dep]\n  version = \"1.0\"\n";
        assert_eq!(
            set_version(toml, "api/Cargo.toml", None, "0.3.0").unwrap(),
            "[workspace.package]\nversion = \"0.3.0\"\n\n[dep]\n  version = \"1.0\"\n",
            "first only"
        );
        assert!(names_version(
            &set_version(toml, "Cargo.toml", None, "0.3.0").unwrap(),
            "Cargo.toml",
            None,
            "0.3.0"
        ));
        assert!(!names_version(toml, "Cargo.toml", None, "0.3.0"));

        let json = "{\n  \"name\": \"x\",\n  \"version\":   \"0.2.0\",\n  \"dep\": {\"version\": \"9\"}\n}\n";
        let set = set_version(json, "package.json", None, "0.3.0").unwrap();
        assert!(set.contains("\"version\":   \"0.3.0\""), "{set}");
        assert!(set.contains("{\"version\": \"9\"}"), "{set}");

        // Explicit: every match, whitespace in the template matching newlines too.
        let lock = "[[package]]\nname = \"kanbanr-core\"\nversion = \"0.2.0\"\n\n\
                    [[package]]\nname = \"other\"\nversion = \"0.2.0\"\n\n\
                    [[package]]\nname = \"kanbanr-core\"\r\nversion = \"0.2.0\"\n";
        let pattern = "name = \"kanbanr-core\"\nversion = \"{version}\"";
        let set = set_version(lock, "Cargo.lock", Some(pattern), "0.3.0").unwrap();
        assert_eq!(set.matches("version = \"0.3.0\"").count(), 2, "{set}");
        assert!(
            set.contains("name = \"other\"\nversion = \"0.2.0\""),
            "{set}"
        );
        assert!(names_version(&set, "Cargo.lock", Some(pattern), "0.3.0"));

        // A version followed by a character of its own class still matches (backtracking).
        assert_eq!(
            set_version(
                "tag: 0.2.0.final\n",
                "x.yml",
                Some("tag: {version}.final"),
                "0.3.0"
            )
            .unwrap(),
            "tag: 0.3.0.final\n"
        );
        let err = set_version("nothing here\n", "Cargo.toml", None, "0.3.0").unwrap_err();
        assert!(
            err.contains("Cargo.toml") && err.contains("matches nothing"),
            "{err}"
        );
        let err = set_version("v=1\n", "VERSION", None, "0.3.0").unwrap_err();
        assert!(err.contains("give a match"), "{err}");

        assert!(check_repo_path("api/Cargo.toml").is_ok());
        assert!(check_repo_path("../x").is_err());
        assert!(check_repo_path("/etc/x").is_err());
        assert!(check_repo_path("C:/x").is_err());
        assert!(check_repo_path(".git/hooks/x").is_err());
        assert_eq!(
            expand("docs/{version}-{number}-{name}-{date}.md", &p("v1.2.0")),
            "docs/v1.2.0-1.2.0-Gates-2026-10-21.md"
        );
    }

    /// FEAT-212 R-8: the changelog gets the dated heading below Unreleased, and its compare links
    /// move on; done twice, the second time changes nothing.
    #[test]
    fn changelog_heading_dated_with_compare_links() {
        let text = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- Gates.\n\n\
                    ## [0.2.1] - 2026-10-20\n\n- Fix.\n\n\
                    [Unreleased]: https://github.com/o/r/compare/v0.2.1...HEAD\n\
                    [0.2.1]: https://github.com/o/r/compare/v0.2.0...v0.2.1\n";
        let dated = date_changelog(text, &p("v0.3.0")).unwrap().unwrap();
        assert_eq!(
            dated,
            "# Changelog\n\n## [Unreleased]\n\n## [0.3.0] - 2026-10-21\n\n### Added\n\n- Gates.\n\n\
             ## [0.2.1] - 2026-10-20\n\n- Fix.\n\n\
             [Unreleased]: https://github.com/o/r/compare/v0.3.0...HEAD\n\
             [0.3.0]: https://github.com/o/r/compare/v0.2.1...v0.3.0\n\
             [0.2.1]: https://github.com/o/r/compare/v0.2.0...v0.2.1\n"
        );
        assert!(has_heading(&dated, "0.3.0"));
        assert_eq!(date_changelog(&dated, &p("v0.3.0")).unwrap(), None);
        // No Unreleased heading: nothing to date.
        assert_eq!(
            date_changelog("# Changelog\n\n## [0.1.0]\n", &p("v0.3.0")).unwrap(),
            None
        );
        // Without links, only the heading; a lowercase heading is found too.
        assert_eq!(
            date_changelog("## [unreleased]\n- x\n", &p("1.0.0"))
                .unwrap()
                .unwrap(),
            "## [unreleased]\n\n## [1.0.0] - 2026-10-21\n- x\n"
        );

        let summary = "- [Release notes](releases/index.md)\n  - [0.2.1](releases/v0.2.1.md)\n";
        let entry = "  - [0.3.0](releases/v0.3.0.md)";
        let listed = add_listing(summary, "- [Release notes](releases/index.md)", entry)
            .unwrap()
            .unwrap();
        assert_eq!(
            listed,
            "- [Release notes](releases/index.md)\n  - [0.3.0](releases/v0.3.0.md)\n  - [0.2.1](releases/v0.2.1.md)\n"
        );
        assert_eq!(
            add_listing(&listed, "- [Release notes](releases/index.md)", entry).unwrap(),
            None
        );
        assert!(add_listing(summary, "- [Elsewhere]", entry).is_err());
    }
}
