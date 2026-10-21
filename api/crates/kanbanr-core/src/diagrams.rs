//! The Mermaid diagram lint (FEAT-200): a line scanner over the fenced `mermaid` blocks of a spec or
//! a markdown doc, run when the text is written, by doctor, and at a gate that asks for
//! `diagrams_valid`.
//!
//! It follows Pele 1.7.0, which is what the monitor and the book draw with: **it flags only a
//! diagram Pele's `parse` rejects, never one Pele accepts.** It is not a Mermaid grammar — it catches
//! the common mistakes, and the review page's real parse is the authoritative check. The fixture
//! `testdata/diagram_lint_cases.json` holds that line from both sides: `lint_cases_fixture` checks
//! the lint against it, and `web/scripts/check-diagram-lint.mjs` checks it against the real Pele.
//!
//! The lint is a total function over `&str`: no regex, no recursion, one pass per line, and at most
//! [`MAX_FINDINGS_PER_FILE`] findings per file, so untrusted text costs linear time.

use serde::{Deserialize, Serialize};

/// The most findings one file reports. A file with more is broken everywhere; fifty say so.
pub const MAX_FINDINGS_PER_FILE: usize = 50;

/// One fenced `mermaid` block of a markdown file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DiagramBlock {
    /// What the block is in: `"FEAT-012 spec"` or `"docs/<key>"`.
    pub file: String,
    /// The doc key, for a link to it; `None` for a spec.
    pub doc: Option<String>,
    /// 1-based index among the file's mermaid blocks.
    pub block: usize,
    /// The file line (1-based) of the block's first source line.
    pub line: usize,
    /// The diagram's source, between the fences.
    pub source: String,
}

/// What the lint found wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LintRule {
    EmptyBlock,
    UnknownType,
    SemicolonInText,
    LabelPunctuation,
    UnbalancedLabel,
    EndAsId,
    UnbalancedBlock,
    UnknownStatement,
}

impl LintRule {
    /// The snake_case name, as it is serialised.
    pub fn name(self) -> &'static str {
        match self {
            LintRule::EmptyBlock => "empty_block",
            LintRule::UnknownType => "unknown_type",
            LintRule::SemicolonInText => "semicolon_in_text",
            LintRule::LabelPunctuation => "label_punctuation",
            LintRule::UnbalancedLabel => "unbalanced_label",
            LintRule::EndAsId => "end_as_id",
            LintRule::UnbalancedBlock => "unbalanced_block",
            LintRule::UnknownStatement => "unknown_statement",
        }
    }
}

/// One problem in one diagram, located in its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DiagramFinding {
    pub file: String,
    /// 1-based index among the file's mermaid blocks.
    pub block: usize,
    /// The file line (1-based) the problem is on.
    pub line: usize,
    pub rule: LintRule,
    pub message: String,
}

impl DiagramFinding {
    /// `"{file}, diagram #{block}, line {line}: {message} ({rule})"`.
    pub fn describe(&self) -> String {
        format!(
            "{}, diagram #{}, line {}: {} ({})",
            self.file,
            self.block,
            self.line,
            self.message,
            self.rule.name()
        )
    }
}

/// Whether a doc path is markdown: `.md` or `.markdown`, in any case.
pub fn is_markdown_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// An open code fence: its character, its run length, and whether it is a mermaid block.
struct Fence {
    ch: u8,
    run: usize,
    mermaid: Option<(usize, String)>,
}

/// The fence a line opens or closes, if it is one: (char, run length, info string).
fn fence_of(line: &str) -> Option<(u8, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = *rest.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let run = rest.bytes().take_while(|&b| b == ch).count();
    if run < 3 {
        return None;
    }
    Some((ch, run, rest[run..].trim()))
}

/// Every fenced `mermaid` block in `markdown`, in order. A mermaid fence inside another open fence
/// (a four-backtick `markdown` example, say) is that fence's content, not a block. An unclosed
/// fence runs to the end of the file.
pub fn blocks(file: &str, doc: Option<&str>, markdown: &str) -> Vec<DiagramBlock> {
    let mut out = Vec::new();
    let mut open: Option<Fence> = None;
    let mut push = |start: usize, source: String| {
        out.push(DiagramBlock {
            file: file.to_string(),
            doc: doc.map(str::to_string),
            block: 0,
            line: start,
            source,
        })
    };
    for (i, raw) in markdown.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let n = i + 1;
        match open.as_mut() {
            None => {
                if let Some((ch, run, info)) = fence_of(line) {
                    let mermaid = (info.split_whitespace().next() == Some("mermaid"))
                        .then(|| (n + 1, String::new()));
                    open = Some(Fence { ch, run, mermaid });
                }
            }
            Some(fence) => {
                let closes = fence_of(line).is_some_and(|(ch, run, info)| {
                    ch == fence.ch && run >= fence.run && info.is_empty()
                });
                if closes {
                    if let Some((start, source)) = open.take().and_then(|f| f.mermaid) {
                        push(start, source);
                    }
                } else if let Some((_, source)) = fence.mermaid.as_mut() {
                    source.push_str(line);
                    source.push('\n');
                }
            }
        }
    }
    if let Some((start, source)) = open.and_then(|f| f.mermaid) {
        push(start, source);
    }
    for (i, b) in out.iter_mut().enumerate() {
        b.block = i + 1;
    }
    out
}

/// The type prefixes Pele 1.7.0 knows, in the order they are tried: `flowchart-elk` before
/// `flowchart`. Matched case-sensitively, as Pele does.
const TYPES: &[&str] = &[
    "sequenceDiagram",
    "flowchart-elk",
    "flowchart",
    "graph",
    "classDiagram",
    "stateDiagram",
    "erDiagram",
    "gantt",
    "pie",
    "journey",
    "gitGraph",
    "mindmap",
    "timeline",
    "quadrantChart",
    "requirement",
    "C4Context",
    "C4Container",
    "C4Component",
    "C4Dynamic",
    "C4Deployment",
    "sankey",
    "xychart",
    "block",
    "packet",
    "kanban",
    "architecture",
    "radar-beta",
    "treemap",
    "info",
    "venn-beta",
    "agentflow-beta",
    "usecase-beta",
    "railroad-",
    "treeView-beta",
    "cynefin-beta",
    "wardley-beta",
    "swimlane-beta",
    "ishikawa",
    "eventmodeling",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Flowchart,
    Sequence,
    Class,
    State,
    Other,
}

/// A finding inside one block: (line within the block, rule, message).
type Local = (usize, LintRule, String);

/// Lint one diagram's source. Each finding is (line within the block, 1-based, rule, message),
/// ordered by line.
pub fn lint_source(source: &str) -> Vec<(usize, LintRule, String)> {
    let lines: Vec<&str> = source
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    // The preamble: blank lines, `%%` comments and directives, and a leading `---` front matter.
    let skip_comments = |mut i: usize| {
        while i < lines.len() && {
            let t = lines[i].trim();
            t.is_empty() || t.starts_with("%%")
        } {
            i += 1;
        }
        i
    };
    let mut i = skip_comments(0);
    if i < lines.len() && lines[i].trim() == "---" {
        i += 1;
        while i < lines.len() && lines[i].trim() != "---" {
            i += 1;
        }
        i = skip_comments(i + 1);
    }
    if i >= lines.len() {
        return vec![(1, LintRule::EmptyBlock, "the diagram is empty".into())];
    }
    let header = lines[i].trim_start();
    let Some(ty) = TYPES.iter().find(|t| header.starts_with(**t)) else {
        let word = header.split_whitespace().next().unwrap_or(header);
        return vec![(
            i + 1,
            LintRule::UnknownType,
            format!(
                "`{word}` is not a diagram type Pele knows — the first line names the type, \
                 such as `flowchart LR` or `sequenceDiagram`"
            ),
        )];
    };
    let kind = match *ty {
        "flowchart" | "flowchart-elk" | "graph" => Kind::Flowchart,
        "sequenceDiagram" => Kind::Sequence,
        "classDiagram" => Kind::Class,
        "stateDiagram" => Kind::State,
        _ => Kind::Other,
    };
    // The body: (line within the block, text), comments left out.
    let body = lines
        .iter()
        .enumerate()
        .skip(i + 1)
        .filter(|(_, l)| !l.trim_start().starts_with("%%"))
        .map(|(n, l)| (n + 1, *l));
    let mut out = Vec::new();
    match kind {
        Kind::Flowchart => {
            let rest = flowchart_header_rest(&header[ty.len()..]);
            let header_line = std::iter::once((i + 1, rest));
            flowchart(header_line.chain(body), &mut out);
        }
        Kind::Sequence => sequence(body, &mut out),
        Kind::Class => braces(body, true, &mut out),
        Kind::State => braces(body, false, &mut out),
        Kind::Other => {}
    }
    out.sort_by_key(|f| f.0);
    out
}

/// Lint every mermaid block of `markdown`, each finding located by its file line. Capped at
/// [`MAX_FINDINGS_PER_FILE`].
pub fn lint_markdown(file: &str, markdown: &str) -> Vec<DiagramFinding> {
    let mut out = Vec::new();
    for b in blocks(file, None, markdown) {
        for (line, rule, message) in lint_source(&b.source) {
            if out.len() >= MAX_FINDINGS_PER_FILE {
                return out;
            }
            out.push(DiagramFinding {
                file: file.to_string(),
                block: b.block,
                line: b.line + line - 1,
                rule,
                message,
            });
        }
    }
    out
}

/// Lint text about to be written. On a project that checks diagrams strictly, a problem refuses the
/// write, naming each one; otherwise the problems come back for the caller to report as warnings.
pub fn guard(
    config: &crate::config::DiagramsConfig,
    file: &str,
    markdown: &str,
) -> crate::Result<Vec<DiagramFinding>> {
    let findings = lint_markdown(file, markdown);
    if config.strict && !findings.is_empty() {
        let each: Vec<String> = findings.iter().map(DiagramFinding::describe).collect();
        return Err(crate::CoreError::InvalidDiagram(format!(
            "{file} has {} diagram problem(s), and this project checks diagrams strictly: {} — fix \
             them, or turn strict checking off with `kanbanr config diagrams --strict off`",
            findings.len(),
            each.join("; ")
        )));
    }
    Ok(findings)
}

/// The lint's findings for an item: its spec as `"{code} spec"`, then each doc it names in
/// `affects_docs` as `"docs/{key}"`, read from `docs_dir` on every call. A key that climbs out of
/// the folder or is absolute is skipped, and so is a doc that is missing or unreadable —
/// `docs_updated` and doctor report those.
pub fn item_findings(
    feature: &crate::FeatureItem,
    docs_dir: Option<&std::path::Path>,
) -> Vec<DiagramFinding> {
    let mut out = lint_markdown(&format!("{} spec", feature.code), &feature.specification);
    let Some(docs) = docs_dir else {
        return out;
    };
    for key in affects_doc_keys(feature) {
        if let Ok(text) = std::fs::read_to_string(docs.join(&key)) {
            out.extend(lint_markdown(&format!("docs/{key}"), &text));
        }
    }
    out
}

/// The board keys of the docs an item names in `affects_docs`, without any that climb out of the
/// docs folder or are absolute.
fn affects_doc_keys(feature: &crate::FeatureItem) -> Vec<String> {
    let declared = feature
        .definition
        .as_ref()
        .map(|d| d.affects_docs.as_slice())
        .unwrap_or(&[]);
    declared
        .iter()
        .map(|d| crate::readiness::affects_doc_key(d))
        .filter(|key| {
            let path = std::path::Path::new(key);
            !path.is_absolute()
                && !key.starts_with('/')
                && !path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
        })
        .collect()
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// ---------------------------------------------------------------------------------------------
// Flowchart

/// What follows the type word on a flowchart's first line, its direction left out: statements, as
/// in `graph LR; A --> B`.
fn flowchart_header_rest(after_type: &str) -> &str {
    // `flowchart-elk` was matched as `flowchart`'s longer sibling; anything glued on is the word.
    let rest = after_type.trim_start_matches(|c: char| !c.is_whitespace() && c != ';');
    let t = rest.trim_start();
    for dir in ["TB", "TD", "BT", "RL", "LR"] {
        if let Some(after) = t.strip_prefix(dir)
            && !after.bytes().next().is_some_and(is_word)
        {
            return after;
        }
    }
    rest
}

/// A line's statements: split at `;` only at bracket depth 0, outside quotes and outside an edge
/// label (`|…|`).
fn statements(line: &str) -> Vec<&str> {
    let b = line.as_bytes();
    let (mut depth, mut quote, mut pipe) = (0usize, false, false);
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'"' => quote = !quote,
            _ if quote => {}
            b'[' | b'(' | b'{' => depth += 1,
            b'>' if i > 0 && is_word(b[i - 1]) => depth += 1,
            b']' | b')' | b'}' => depth = depth.saturating_sub(1),
            b'|' if depth == 0 => pipe = !pipe,
            b';' if depth == 0 && !pipe => {
                out.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    out
}

/// Statements a flowchart holds that carry no labels or ids the lint reads.
const FLOW_SKIP: &[&str] = &[
    "classDef",
    "class ",
    "style ",
    "linkStyle",
    "click ",
    "direction ",
    "%%",
];

fn flowchart<'a>(lines: impl Iterator<Item = (usize, &'a str)>, out: &mut Vec<Local>) {
    let mut subgraphs: Vec<usize> = Vec::new();
    for (n, line) in lines {
        for stmt in statements(line) {
            let s = stmt.trim();
            if s.is_empty() || FLOW_SKIP.iter().any(|p| s.starts_with(p)) {
                continue;
            }
            if s == "end" {
                if subgraphs.pop().is_none() {
                    out.push((n, LintRule::UnbalancedBlock, "`end` closes nothing".into()));
                }
                continue;
            }
            if let Some(rest) = s.strip_prefix("subgraph")
                && (rest.is_empty() || rest.starts_with(char::is_whitespace))
            {
                subgraphs.push(n);
                let rest = rest.trim_start();
                let id_len = rest
                    .find(|c: char| c.is_whitespace() || c == '[')
                    .unwrap_or(rest.len());
                if &rest[..id_len] == "end" {
                    out.push((n, LintRule::EndAsId, end_message("a subgraph")));
                }
                let title = rest[id_len..].trim_start();
                if title.starts_with('[')
                    && let Err(f) = label(title.as_bytes(), 1, &[b"]"], NODE_FORBIDDEN, true)
                {
                    out.push((n, f.0, f.1));
                }
                continue;
            }
            if let Some(f) = scan_statement(s) {
                out.push((n, f.0, f.1));
            }
        }
    }
    for n in subgraphs {
        out.push((
            n,
            LintRule::UnbalancedBlock,
            "this `subgraph` is never closed with `end`".into(),
        ));
    }
}

fn end_message(what: &str) -> String {
    format!(
        "`end` closes a block, so it cannot name {what} — call it something else, such as `done`"
    )
}

/// Characters an unquoted node label may not hold, in Pele.
const NODE_FORBIDDEN: &[u8] = b"\"|{}()[]";
/// Characters an unquoted edge label may not hold.
const EDGE_FORBIDDEN: &[u8] = b"\"{}()[]";

/// Node shape openers, longest first, each with the closers that end it.
const SHAPES: &[(&[u8], &[&[u8]])] = &[
    (b"(((", &[b")))"]),
    (b"((", &[b"))"]),
    (b"([", &[b"])"]),
    (b"(", &[b")"]),
    (b"[[", &[b"]]"]),
    (b"[(", &[b")]"]),
    (b"[/", &[b"/]", b"\\]"]),
    (b"[\\", &[b"\\]", b"/]"]),
    (b"[", &[b"]"]),
    (b"{{", &[b"}}"]),
    (b"{", &[b"}"]),
    (b">", &[b"]"]),
];

type Problem = (LintRule, String);

/// The first occurrence of any of `closers` in `b` at or after `from`: (start, length).
fn find_closer(b: &[u8], from: usize, closers: &[&[u8]]) -> Option<(usize, usize)> {
    (from..b.len()).find_map(|i| {
        closers
            .iter()
            .find(|c| b[i..].starts_with(c))
            .map(|c| (i, c.len()))
    })
}

/// Check one label that starts at `from` (just after its opener) and ends at one of `closers`.
/// Returns the index just past the closer. `strict_quote`: text between a quoted label's closing
/// quote and its closer is an error (true for node labels; Pele allows it in edge labels).
fn label(
    b: &[u8],
    from: usize,
    closers: &[&[u8]],
    forbidden: &[u8],
    strict_quote: bool,
) -> Result<usize, Problem> {
    let mut j = from;
    while j < b.len() && b[j] == b' ' {
        j += 1;
    }
    if j < b.len() && b[j] == b'"' {
        let Some(q) = (j + 1..b.len()).find(|&k| b[k] == b'"') else {
            return Err((
                LintRule::UnbalancedLabel,
                "a quote in this label is never closed".into(),
            ));
        };
        let Some((c, len)) = find_closer(b, q + 1, closers) else {
            return Err((
                LintRule::UnbalancedLabel,
                "this label is never closed".into(),
            ));
        };
        if strict_quote && b[q + 1..c].iter().any(|&x| x != b' ') {
            return Err((
                LintRule::LabelPunctuation,
                "text after the label's closing quote — a quote inside a quoted label ends it; \
                 keep the whole label in one pair of quotes, without quotes inside"
                    .into(),
            ));
        }
        return Ok(c + len);
    }
    let Some((c, len)) = find_closer(b, from, closers) else {
        return Err((
            LintRule::UnbalancedLabel,
            "this label is never closed".into(),
        ));
    };
    if let Some(&bad) = b[from..c].iter().find(|x| forbidden.contains(x)) {
        return Err((
            LintRule::LabelPunctuation,
            format!(
                "`{}` in an unquoted label — put the label in double quotes, such as A[\"f(x)\"]",
                bad as char
            ),
        ));
    }
    Ok(c + len)
}

/// Whether the `end` at `i` starts an id: what is before it ends a token rather than continuing
/// one (`A-end` and `x.end` are ids that merely contain it).
fn end_starts_id(b: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    match b[i - 1] {
        b' ' | b'\t' | b'>' | b'&' | b'|' | b';' | b'=' => true,
        b'-' => i >= 2 && b[i - 2] == b'-',
        _ => false,
    }
}

/// Scan one flowchart statement for its labels and for `end` used as an id; the first problem.
fn scan_statement(s: &str) -> Option<Problem> {
    let b = s.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        let c = b[i];
        // Edge text, ` -- text -->` or ` == text ==>`: skipped up to the arrow that ends it.
        if c == b' '
            && i + 3 < n
            && (&b[i + 1..i + 3] == b"--" || &b[i + 1..i + 3] == b"==")
            && b[i + 3] == b' '
        {
            let dash = b[i + 1];
            let pair = [dash, dash];
            match (i + 4..n.saturating_sub(1)).find(|&k| b[k..k + 2] == pair) {
                Some(k) => {
                    let mut k = k;
                    while k < n && b[k] == dash {
                        k += 1;
                    }
                    if k < n && matches!(b[k], b'>' | b'x' | b'o') {
                        k += 1;
                    }
                    i = k;
                }
                None => i = n,
            }
            continue;
        }
        if c == b'"' {
            // A quoted string outside a label (`@{ label: "…" }`): not ids.
            i = (i + 1..n).find(|&k| b[k] == b'"').map_or(n, |k| k + 1);
            continue;
        }
        if c == b'|' {
            match label(b, i + 1, &[b"|"], EDGE_FORBIDDEN, false) {
                Ok(next) => i = next,
                Err(p) => return Some(p),
            }
            continue;
        }
        if matches!(c, b'[' | b'(' | b'{' | b'>') && i > 0 && is_word(b[i - 1]) {
            let (open, closers) = SHAPES
                .iter()
                .find(|(o, _)| b[i..].starts_with(o))
                .expect("every opener char is a shape");
            match label(b, i + open.len(), closers, NODE_FORBIDDEN, true) {
                Ok(next) => i = next,
                Err(p) => return Some(p),
            }
            continue;
        }
        if b[i..].starts_with(b"end")
            && end_starts_id(b, i)
            && !b.get(i + 3).copied().is_some_and(is_word)
        {
            return Some((LintRule::EndAsId, end_message("a node")));
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Sequence

/// Sequence arrows, longest first.
const ARROWS: &[&str] = &[
    "<<-->>", "<<->>", "-->>", "->>", "-->", "--x", "--)", "->", "-x", "-)",
];

/// Whether `text` has a `;` with more than blanks after it: a statement separator mid-text.
fn semicolon_mid_text(text: &str) -> bool {
    text.match_indices(';')
        .any(|(k, _)| !text[k + 1..].trim().is_empty())
}

fn semicolon_problem(where_: &str) -> String {
    format!("`;` ends the statement early, so the rest of {where_} is lost — use a comma or words")
}

fn sequence<'a>(lines: impl Iterator<Item = (usize, &'a str)>, out: &mut Vec<Local>) {
    let mut stack: Vec<(String, usize)> = Vec::new();
    for (n, line) in lines {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let first = t.split_whitespace().next().unwrap_or("");
        let word = first.trim_end_matches(':').to_ascii_lowercase();
        match word.as_str() {
            "alt" | "loop" | "opt" | "par" | "critical" | "break" | "rect" | "box" => {
                stack.push((word, n));
            }
            "else" | "and" | "option" => {
                let opener = match word.as_str() {
                    "else" => "alt",
                    "and" => "par",
                    _ => "critical",
                };
                if stack.last().map(|(k, _)| k.as_str()) != Some(opener) {
                    out.push((
                        n,
                        LintRule::UnbalancedBlock,
                        format!("`{word}` belongs inside an `{opener}` block"),
                    ));
                }
            }
            "end" => {
                if stack.pop().is_none() {
                    out.push((n, LintRule::UnbalancedBlock, "`end` closes nothing".into()));
                }
            }
            "participant" | "actor" | "create" => {
                if semicolon_mid_text(t) {
                    out.push((n, LintRule::SemicolonInText, semicolon_problem("the line")));
                }
            }
            "destroy" | "activate" | "deactivate" | "autonumber" | "title" | "acctitle"
            | "accdescr" | "link" | "links" | "properties" | "details" => {}
            "note" => {
                if let Some(k) = t.find(':')
                    && semicolon_mid_text(&t[k + 1..])
                {
                    out.push((n, LintRule::SemicolonInText, semicolon_problem("the note")));
                }
            }
            _ => {
                if let Some(p) = message(t) {
                    out.push((n, p.0, p.1));
                }
            }
        }
    }
    for (kw, n) in stack {
        out.push((
            n,
            LintRule::UnbalancedBlock,
            format!("this `{kw}` is never closed with `end`"),
        ));
    }
}

/// Check a line that should be a message, `A->>B: text`.
fn message(t: &str) -> Option<Problem> {
    let not_a_statement = || {
        (
            LintRule::UnknownStatement,
            "not a sequence statement — a message is `A->>B: text`, with the colon".to_string(),
        )
    };
    let found = (0..t.len()).find_map(|i| {
        ARROWS
            .iter()
            .find(|a| t.as_bytes()[i..].starts_with(a.as_bytes()))
            .map(|a| (i, a.len()))
    });
    let Some((at, len)) = found else {
        return Some(not_a_statement());
    };
    let from = t[..at].trim();
    let rest = t[at + len..].trim_start();
    let rest = rest
        .strip_prefix('+')
        .or_else(|| rest.strip_prefix('-'))
        .unwrap_or(rest);
    let Some(colon) = rest.find(':') else {
        return Some(not_a_statement());
    };
    let to = rest[..colon].trim();
    if from == "end" || to == "end" {
        return Some((LintRule::EndAsId, end_message("a participant")));
    }
    if semicolon_mid_text(&rest[colon + 1..]) {
        return Some((LintRule::SemicolonInText, semicolon_problem("the message")));
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Class and state

/// `{`/`}` balance in a class or state diagram, outside quotes and before a label's `:`; in a class
/// diagram, also a `;` mid-way through a relation label. Pele allows a stray `}` in a state diagram.
fn braces<'a>(lines: impl Iterator<Item = (usize, &'a str)>, class: bool, out: &mut Vec<Local>) {
    let mut open: Vec<usize> = Vec::new();
    let mut in_note = false;
    for (n, line) in lines {
        let t = line.trim();
        // A state diagram's multi-line note runs to `end note`; its text is not structure.
        if !class {
            if in_note {
                in_note = t != "end note";
                continue;
            }
            if t.to_ascii_lowercase().starts_with("note ") && !t.contains(':') {
                in_note = true;
                continue;
            }
        }
        if class
            && open.is_empty()
            && (t.contains("--") || t.contains(".."))
            && let Some(k) = t.find(" : ")
            && semicolon_mid_text(&t[k + 3..])
        {
            out.push((n, LintRule::SemicolonInText, semicolon_problem("the label")));
        }
        let mut quote = false;
        for c in t.bytes() {
            match c {
                b'"' => quote = !quote,
                _ if quote => {}
                b':' if open.is_empty() => break,
                b'{' => open.push(n),
                b'}' => match open.pop() {
                    None if class => {
                        out.push((n, LintRule::UnbalancedBlock, "`}` closes nothing".into()))
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    for n in open {
        out.push((
            n,
            LintRule::UnbalancedBlock,
            "this `{` is never closed".into(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(source: &str) -> Vec<(usize, &'static str)> {
        lint_source(source)
            .into_iter()
            .map(|(l, r, _)| (l, r.name()))
            .collect()
    }

    fn first(source: &str) -> Option<(usize, &'static str)> {
        rules(source).into_iter().next()
    }

    #[test]
    fn extracts_mermaid_blocks_with_file_lines() {
        let md = "# Title\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\ntext\n\n~~~ mermaid\n\
                  sequenceDiagram\n  A->>B: a; b\n~~~\n\n```rust\nfn x() {}\n```\n";
        let b = blocks("FEAT-1 spec", None, md);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].block, b[0].line), (1, 4));
        assert_eq!(b[0].source, "flowchart LR\n  A --> B\n");
        assert_eq!((b[1].block, b[1].line), (2, 11));
        assert_eq!(b[1].file, "FEAT-1 spec");
        assert_eq!(b[1].doc, None);

        let f = lint_markdown("FEAT-1 spec", md);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!((f[0].block, f[0].line), (2, 12));
        assert_eq!(f[0].rule, LintRule::SemicolonInText);
        assert!(
            f[0].describe()
                .starts_with("FEAT-1 spec, diagram #2, line 12: "),
            "{}",
            f[0].describe()
        );
        assert!(f[0].describe().ends_with("(semicolon_in_text)"));

        // A doc key rides along for the link; an unclosed fence runs to the end of the file.
        let b = blocks("docs/a.md", Some("a.md"), "```mermaid\npie\n");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].doc.as_deref(), Some("a.md"));
        assert_eq!(b[0].line, 2);
        // Windows line endings.
        let b = blocks("x", None, "```mermaid\r\nflowchart LR\r\n```\r\n");
        assert_eq!(b[0].source, "flowchart LR\n");
    }

    #[test]
    fn ignores_mermaid_inside_other_fences() {
        let md =
            "````markdown\n```mermaid\nflowchrt\n```\n````\n\n   ```mermaid\nflowchart LR\n```\n";
        let b = blocks("f", None, md);
        assert_eq!(b.len(), 1, "{b:?}");
        assert_eq!(b[0].line, 8);
        assert!(lint_markdown("f", md).is_empty());
        // Indented four spaces is a code block, not a fence.
        assert!(blocks("f", None, "    ```mermaid\n    flowchrt\n    ```\n").is_empty());
    }

    #[test]
    fn empty_and_unknown_type() {
        assert_eq!(first(""), Some((1, "empty_block")));
        assert_eq!(first("\n\n%% only a comment\n"), Some((1, "empty_block")));
        assert_eq!(first("flowchrt LR\n  A --> B\n"), Some((1, "unknown_type")));
        assert_eq!(first("\n\nFLOWCHART LR\n"), Some((3, "unknown_type")));
        assert_eq!(first("title x\nflowchart LR\n"), Some((1, "unknown_type")));
        // The preamble is skipped; the type then counts from its own line.
        assert_eq!(first("---\ntitle: t\n---\nflowchart LR\n  A --> B\n"), None);
        assert_eq!(
            first("%%{init: {}}%%\nsequenceDiagram\n  A->>B: hi\n"),
            None
        );
        assert_eq!(first("flowchart-elk LR\n  A --> B\n"), None);
        assert_eq!(first("erDiagram\n  A ||--o{ B : has\n"), None);
        // An unknown type is reported once; nothing after it is checked.
        assert_eq!(rules("flowchrt\n  end --> B\n").len(), 1);
    }

    #[test]
    fn semicolon_in_text() {
        assert_eq!(
            first("sequenceDiagram\n  A->>B: a; b\n"),
            Some((2, "semicolon_in_text"))
        );
        assert_eq!(
            first("sequenceDiagram\n  A->>B: a ;b\n"),
            Some((2, "semicolon_in_text"))
        );
        assert_eq!(first("sequenceDiagram\n  A->>B: x;\n"), None);
        assert_eq!(first("sequenceDiagram\n  A->>B: x;  \n"), None);
        assert_eq!(
            first("sequenceDiagram\n  note left of A: a; b\n"),
            Some((2, "semicolon_in_text"))
        );
        assert_eq!(
            first("sequenceDiagram\n  actor A as x; y\n"),
            Some((2, "semicolon_in_text"))
        );
        assert_eq!(first("sequenceDiagram\n  participant A;\n"), None);
        assert_eq!(
            first("classDiagram\n  A <|-- B : x; y\n"),
            Some((2, "semicolon_in_text"))
        );
        // Pele accepts these, so the lint does too.
        assert_eq!(first("sequenceDiagram\n  A->>B: {\"a\": \"b\"}\n"), None);
        assert_eq!(first("flowchart LR\n  A[x; y] --> B\n"), None);
        assert_eq!(first("flowchart LR\n  A -->|a; b| B\n"), None);
        assert_eq!(first("stateDiagram-v2\n  A --> B : x; y\n"), None);
    }

    #[test]
    fn flowchart_label_punctuation() {
        for bad in [
            "A[f(x)] --> B",
            "A(f(x)) --> B",
            "A{a[0]} --> B",
            "A -->|f(x)| B",
            "A[a | b] --> B",
            "A[[a(b)]] --> B",
            "A>a (b)] --> B",
            "A[\"a \"b\" c\"] --> B",
        ] {
            assert_eq!(
                first(&format!("flowchart LR\n  {bad}\n")),
                Some((2, "label_punctuation")),
                "{bad}"
            );
        }
        assert_eq!(
            first("flowchart LR\n  subgraph s [a (b)]\n  A\n  end\n"),
            Some((2, "label_punctuation"))
        );
        for good in [
            "A[\"f(x) {y} | z; w\"] --> B",
            "A -->|\"f(x)\"| B",
            "A -->|\"a\" b| B",
            "A([stadium]) --> B[[sub]] --> C[(db)] --> D((c)) --> E{{h}} --> F>flag]",
            "A[/para/] --> B[\\x\\] --> C[/trap\\] --> D(((dbl)))",
            "A -- f(x) --> B",
            "A == f(x) ==> B",
            "A[a > b] --> B",
            "A>x; y] --> B",
            "A@{ shape: rect, label: \"the end\" } --> B",
            "A[\"`**md**`\"]:::hot --> B",
            "A --> B & C",
        ] {
            assert_eq!(first(&format!("flowchart LR\n  {good}\n")), None, "{good}");
        }
        assert_eq!(
            first("flowchart LR\n  A[abc --> B\n"),
            Some((2, "unbalanced_label"))
        );
        assert_eq!(
            first("flowchart LR\n  A[\"abc] --> B\n"),
            Some((2, "unbalanced_label"))
        );
        assert_eq!(
            first("flowchart LR\n  A -->|abc B\n"),
            Some((2, "unbalanced_label"))
        );
    }

    #[test]
    fn end_is_never_an_id() {
        for bad in [
            "end --> B",
            "A --> end",
            "A-->end",
            "A --- end",
            "A --> end.x",
            "A --> end-x",
            "A --> B & end",
            "end[x] --> B",
        ] {
            assert_eq!(
                first(&format!("flowchart LR\n  {bad}\n")),
                Some((2, "end_as_id")),
                "{bad}"
            );
        }
        assert_eq!(
            first("flowchart LR\n  subgraph end\n  A\n  end\n"),
            Some((2, "end_as_id"))
        );
        for good in [
            "A --> End",
            "A --> endpoint",
            "A[the end] --> B",
            "A-end --> B",
            "x.end --> B",
            "A_end --> end_x",
            "A -- end --> B",
            "A -->|the end| B",
        ] {
            assert_eq!(first(&format!("flowchart LR\n  {good}\n")), None, "{good}");
        }
        assert_eq!(
            first("sequenceDiagram\n  A->>end: hi\n"),
            Some((2, "end_as_id"))
        );
        assert_eq!(
            first("sequenceDiagram\n  end->>A: hi\n"),
            Some((2, "end_as_id"))
        );
        assert_eq!(
            first("sequenceDiagram\n  end ->> A: hi\n"),
            Some((2, "unbalanced_block"))
        );
        // Pele accepts a participant declared as `end`, and an id that merely contains it.
        assert_eq!(first("sequenceDiagram\n  participant end\n"), None);
        assert_eq!(first("sequenceDiagram\n  Alice->>Bob end: hi\n"), None);
        assert_eq!(first("sequenceDiagram\n  A->>endpoint: hi\n"), None);
    }

    #[test]
    fn blocks_are_balanced() {
        assert_eq!(
            first("flowchart TB\n  subgraph one\n    A --> B\n"),
            Some((2, "unbalanced_block"))
        );
        assert_eq!(
            first("flowchart TB\n  A --> B\n  end\n"),
            Some((3, "unbalanced_block"))
        );
        assert_eq!(
            first("flowchart TB\n  subgraph a [x]\n  subgraph b\n  C\n  end\n  end\n"),
            None
        );
        for kw in [
            "alt", "loop", "opt", "par", "critical", "break", "rect", "box",
        ] {
            assert_eq!(
                first(&format!("sequenceDiagram\n  {kw} x\n  A->>B: q\n")),
                Some((2, "unbalanced_block")),
                "{kw}"
            );
            assert_eq!(
                first(&format!("sequenceDiagram\n  {kw} x\n  A->>B: q\n  end\n")),
                None,
                "{kw}"
            );
        }
        // Pele reads block keywords in any case.
        assert_eq!(first("sequenceDiagram\n  ALT x\n  A->>B: q\n  END\n"), None);
        assert_eq!(
            first("sequenceDiagram\n  loop x\n  A->>B: q\n  else y\n  end\n"),
            Some((4, "unbalanced_block"))
        );
        assert_eq!(
            first("sequenceDiagram\n  alt a\n  A->>B: q\n  and b\n  end\n"),
            Some((4, "unbalanced_block"))
        );
        assert_eq!(
            first("sequenceDiagram\n  par a\n  A->>B: q\n  option b\n  end\n"),
            Some((4, "unbalanced_block"))
        );
        assert_eq!(
            first(
                "sequenceDiagram\n  critical a\n  A->>B: q\n  option b\n  A->>B: r\n  end\n  \
                 par x\n  A->>B: s\n  and y\n  A->>C: t\n  end\n"
            ),
            None
        );
        assert_eq!(
            first("sequenceDiagram\n  A->>B: q\n  end\n"),
            Some((3, "unbalanced_block"))
        );
        assert_eq!(
            first("classDiagram\n  class A {\n    +x int\n"),
            Some((2, "unbalanced_block"))
        );
        assert_eq!(
            first("classDiagram\n  A --> B\n  }\n"),
            Some((3, "unbalanced_block"))
        );
        assert_eq!(
            first("stateDiagram-v2\n  state S {\n    A --> B\n"),
            Some((2, "unbalanced_block"))
        );
        // Pele allows a stray `}` in a state diagram, and braces in its labels and quotes.
        assert_eq!(first("stateDiagram-v2\n  A --> B\n  }\n"), None);
        assert_eq!(first("stateDiagram-v2\n  A --> B : x {\n"), None);
        assert_eq!(first("stateDiagram-v2\n  state \"a {\" as S\n"), None);
        assert_eq!(first("classDiagram\n  note \"a {b\"\n"), None);
    }

    #[test]
    fn sequence_lines_are_statements() {
        assert_eq!(
            first("sequenceDiagram\n  A->>B: hi\n  this is junk\n"),
            Some((3, "unknown_statement"))
        );
        assert_eq!(
            first("sequenceDiagram\n  A->>B\n"),
            Some((2, "unknown_statement"))
        );
        let good = "sequenceDiagram\n  title: Hello\n  autonumber\n  participant A as Alice (admin)\n  \
                    create participant X\n  actor U as User\n  U->>+S: go\n  S-->>-U: done\n  \
                    A-xB: x\n  A-)B: x\n  A--)B: y\n  A--xB: z\n  A<<->>B: q\n  A->>B:\n  \
                    A ->> B : spaced\n  activate U\n  deactivate U\n  destroy X\n  \
                    Note right of A: text\n  NOTE over A,B: x\n  accTitle: t\n  accDescr: d\n  \
                    rect rgb(0, 0, 0)\n  A->>B: x\n  end\n  %% a comment\n";
        assert_eq!(rules(good), vec![]);
    }

    #[test]
    fn lint_cases_fixture() {
        #[derive(Deserialize)]
        struct Case {
            name: String,
            source: String,
            rule: Option<String>,
            line: Option<usize>,
        }
        #[derive(Deserialize)]
        struct Fixture {
            cases: Vec<Case>,
        }
        let f: Fixture =
            serde_json::from_str(include_str!("testdata/diagram_lint_cases.json")).unwrap();
        assert!(f.cases.len() >= 40, "the fixture holds the probe table");
        let mut failures = Vec::new();
        for c in &f.cases {
            let got = first(&c.source);
            let want = c.rule.as_deref().map(|r| (c.line.unwrap(), r));
            if got != want {
                failures.push(format!("{}: want {want:?}, got {got:?}", c.name));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Every diagram in the book, the README and the skill draws in Pele (`check:docs`), so the lint
    /// must pass every one of them: a wide check that it flags nothing Pele accepts.
    #[test]
    fn the_books_diagrams_pass_the_lint() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut files = vec![root.join("README.md"), root.join("skill/kanbanr/SKILL.md")];
        let mut dirs = vec![root.join("docs/src")];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p.extension().is_some_and(|x| x == "md") {
                    files.push(p);
                }
            }
        }
        let mut seen = 0;
        let mut found = Vec::new();
        for p in files {
            let text = std::fs::read_to_string(&p).unwrap();
            seen += blocks("", None, &text).len();
            found.extend(lint_markdown(&p.display().to_string(), &text));
        }
        assert!(
            seen > 0,
            "the book has diagrams; zero means the paths are wrong"
        );
        assert!(
            found.is_empty(),
            "{}",
            found
                .iter()
                .map(|f| f.describe())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn lint_is_linear() {
        // 1,000 blocks of about 2 KiB each: 2 MiB of markdown, every block a busy flowchart or a
        // sequence diagram, with findings in some of them.
        let mut md = String::new();
        let mut k = 0;
        while md.len() < 2 * 1024 * 1024 {
            md.push_str("Some prose between the diagrams.\n\n```mermaid\n");
            if k % 2 == 0 {
                md.push_str("flowchart LR\n");
                for e in 0..40 {
                    md.push_str(&format!(
                        "  N{e}[\"node {e}\"] -->|edge {e}| M{e}(step {e})\n"
                    ));
                }
            } else {
                md.push_str("sequenceDiagram\n  loop poll\n");
                for e in 0..40 {
                    md.push_str(&format!("    A{e}->>B{e}: message number {e}, plain\n"));
                }
                md.push_str("  end\n");
            }
            if k % 100 == 0 {
                md.push_str("  A->>B: a; b\n");
            }
            md.push_str("```\n\n");
            k += 1;
        }
        assert!(k >= 1000, "{k} blocks");
        // A pathological line: many openers, none closed.
        md.push_str("```mermaid\nflowchart LR\n  ");
        md.push_str(&"A[".repeat(20_000));
        md.push_str("\n```\n");
        let start = std::time::Instant::now();
        let found = lint_markdown("big.md", &md);
        let took = start.elapsed();
        assert!(!found.is_empty());
        assert!(found.len() <= MAX_FINDINGS_PER_FILE);
        assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
    }
}
