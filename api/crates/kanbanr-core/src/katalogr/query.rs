//! The shared query-block grammar, format 1 (`fixtures/contract/query-block-v1.md`, copied from
//! katalogr's `int/contract` line). A doc holds a block whose info string is `katalogr-query`; its
//! body is one line: an operation, then its positionals and flags. FTR-016 R-1.

use serde::Serialize;
use std::collections::BTreeMap;

/// The info string of a query block's fence.
pub const FENCE_INFO: &str = "katalogr-query";

/// The operations of query-block-v1.md as copied (int/contract eaec943, 16 Nov 2026: nine).
pub const OPS: [&str; 9] = [
    "structure",
    "contracts",
    "callers",
    "flow",
    "impact",
    "lineage",
    "path",
    "process",
    "cycles",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryBlock {
    /// 1-based line of the opening fence.
    pub line: usize,
    pub source: String,
}

/// An open code fence: its character and its run length, and the body so far when it is a query.
struct Fence {
    ch: u8,
    run: usize,
    /// (line of the opening fence, body lines) for a `katalogr-query` fence.
    query: Option<(usize, Vec<String>)>,
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

/// Every `katalogr-query` fence of `markdown`, in order: three or more backticks or tildes
/// indented at most three spaces; a fence inside a longer or different fence is content; an
/// unclosed fence is not a block. Lines are file lines, as `mermaid_lint::blocks` counts them
/// (this walk is that one's, with the info string changed: mermaid-lint is standalone).
pub fn find_query_blocks(markdown: &str) -> Vec<QueryBlock> {
    let mut out = Vec::new();
    let mut open: Option<Fence> = None;
    for (i, raw) in markdown.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let n = i + 1;
        match open.as_mut() {
            None => {
                if let Some((ch, run, info)) = fence_of(line) {
                    let query = (info.split_whitespace().next() == Some(FENCE_INFO))
                        .then(|| (n, Vec::new()));
                    open = Some(Fence { ch, run, query });
                }
            }
            Some(fence) => {
                let closes = fence_of(line).is_some_and(|(ch, run, info)| {
                    ch == fence.ch && run >= fence.run && info.is_empty()
                });
                if closes {
                    if let Some((at, body)) = open.take().and_then(|f| f.query) {
                        out.push(QueryBlock {
                            line: at,
                            source: body.join("\n"),
                        });
                    }
                } else if let Some((_, body)) = fence.query.as_mut() {
                    body.push(line.to_string());
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuerySpec {
    pub op: String,
    /// Positionals by the op's names (`symbol`, or `from` and `to`), then the flags given.
    pub args: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryProblem {
    pub file: String,
    pub line: usize,
    pub source: String,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Integer,
    Enum(&'static [&'static str]),
    Boolean,
    Text,
}

/// One operation's own arguments, as `views-v1.json`'s `catalogue.views` lists them.
pub(crate) struct OpSpec {
    pub op: &'static str,
    pub positionals: &'static [&'static str],
    pub flags: &'static [(&'static str, Kind)],
}

const LEVELS: &[&str] = &["file", "package"];

/// The per-op tables: data, pinned against the shared catalogue by a test.
pub(crate) const SPECS: [OpSpec; 9] = [
    OpSpec {
        op: "structure",
        positionals: &[],
        flags: &[
            ("inferred", Kind::Boolean),
            ("level", Kind::Enum(LEVELS)),
            ("max-nodes", Kind::Integer),
        ],
    },
    OpSpec {
        op: "contracts",
        positionals: &[],
        flags: &[("kind", Kind::Text), ("max-nodes", Kind::Integer)],
    },
    OpSpec {
        op: "callers",
        positionals: &["symbol"],
        flags: &[("depth", Kind::Integer), ("max-nodes", Kind::Integer)],
    },
    OpSpec {
        op: "flow",
        positionals: &["symbol"],
        flags: &[
            ("depth", Kind::Integer),
            ("max-nodes", Kind::Integer),
            ("reverse", Kind::Boolean),
        ],
    },
    OpSpec {
        op: "impact",
        positionals: &["symbol"],
        flags: &[("depth", Kind::Integer), ("max-nodes", Kind::Integer)],
    },
    OpSpec {
        op: "lineage",
        positionals: &["symbol"],
        flags: &[("depth", Kind::Integer), ("max-nodes", Kind::Integer)],
    },
    OpSpec {
        op: "path",
        positionals: &["from", "to"],
        flags: &[("max-len", Kind::Integer)],
    },
    OpSpec {
        op: "process",
        positionals: &["symbol"],
        flags: &[("max-nodes", Kind::Integer), ("messages", Kind::Boolean)],
    },
    OpSpec {
        op: "cycles",
        positionals: &[],
        flags: &[("level", Kind::Enum(LEVELS)), ("limit", Kind::Integer)],
    },
];

/// Split one line into words: whitespace separates, a double quote opens and closes a stretch that
/// may hold spaces (and may sit inside a word, as in `--kind="a b"`). An open quote is an error.
fn words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err("a double quote is opened and never closed".to_string());
    }
    if started {
        out.push(cur);
    }
    Ok(out)
}

impl QuerySpec {
    /// One line: op, positionals in the op's own number, `--flag value` / `--flag=value`, a switch
    /// alone or with true|false, each flag once, numbers above 0, word flags from their list, a
    /// quoted word may hold spaces. Anything else is `Err(reason)`, worded for a reader. Keeps
    /// only what the block says: no defaults are filled in.
    pub fn parse(source: &str) -> Result<QuerySpec, String> {
        let body = source.trim_matches(|c| c == '\n' || c == '\r');
        if body.trim().is_empty() {
            return Err(
                "the block is empty: a query is one line, an operation and its arguments"
                    .to_string(),
            );
        }
        if body.contains('\n') {
            return Err("a query is one line; this block has more".to_string());
        }
        let words = words(body)?;
        let op = words[0].as_str();
        let Some(spec) = SPECS.iter().find(|s| s.op == op) else {
            return Err(format!(
                "`{op}` is not an operation; the operations are {}",
                OPS.join(", ")
            ));
        };
        let mut positionals: Vec<&str> = Vec::new();
        let mut args: BTreeMap<String, String> = BTreeMap::new();
        let mut i = 1;
        while i < words.len() {
            let w = words[i].as_str();
            i += 1;
            let Some(flag) = w.strip_prefix("--") else {
                positionals.push(w);
                continue;
            };
            let (name, inline) = match flag.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (flag, None),
            };
            let Some((_, kind)) = spec.flags.iter().find(|(n, _)| *n == name) else {
                let known: Vec<String> = spec.flags.iter().map(|(n, _)| format!("--{n}")).collect();
                return Err(if known.is_empty() {
                    format!("`--{name}` is not a flag of `{op}`, which takes none")
                } else {
                    format!(
                        "`--{name}` is not a flag of `{op}`; its flags are {}",
                        known.join(", ")
                    )
                });
            };
            if args.contains_key(name) {
                return Err(format!(
                    "`--{name}` is given more than once; give a flag once"
                ));
            }
            let value = match (kind, inline) {
                (_, Some(v)) => v.to_string(),
                (Kind::Boolean, None) => match words.get(i).map(String::as_str) {
                    Some(v @ ("true" | "false")) => {
                        i += 1;
                        v.to_string()
                    }
                    _ => "true".to_string(),
                },
                (_, None) => match words.get(i) {
                    Some(v) if !v.starts_with("--") => {
                        i += 1;
                        v.clone()
                    }
                    _ => return Err(format!("`--{name}` needs a value")),
                },
            };
            match kind {
                Kind::Integer => {
                    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(format!(
                            "`--{name}` takes a whole number above 0, not `{value}`"
                        ));
                    }
                    if value.trim_start_matches('0').is_empty() {
                        return Err(format!("`--{name}` takes a number above 0, not `{value}`"));
                    }
                }
                Kind::Enum(list) => {
                    if !list.contains(&value.as_str()) {
                        return Err(format!(
                            "`--{name}` is one of {}, not `{value}`",
                            list.join(", ")
                        ));
                    }
                }
                Kind::Boolean => {
                    if value != "true" && value != "false" {
                        return Err(format!("`--{name}` takes true or false, not `{value}`"));
                    }
                }
                Kind::Text => {
                    if value.is_empty() {
                        return Err(format!("`--{name}` needs a value"));
                    }
                }
            }
            args.insert(name.to_string(), value);
        }
        let want = spec.positionals;
        if positionals.len() != want.len() {
            return Err(match (want.len(), positionals.len()) {
                (0, _) => format!("`{op}` takes no argument besides its flags"),
                (1, 0) => format!("`{op}` needs a {}", want[0]),
                (_, n) if n < want.len() => format!(
                    "`{op}` needs {}; `{}` is missing",
                    want.join(" and "),
                    want[n]
                ),
                _ => format!("`{op}` takes {} and no more", want.join(" and ")),
            });
        }
        for (name, value) in want.iter().zip(positionals) {
            if value.is_empty() {
                return Err(format!("the {name} is empty"));
            }
            args.insert((*name).to_string(), value.to_string());
        }
        Ok(QuerySpec {
            op: op.to_string(),
            args,
        })
    }

    /// The query string for `GET /api/v1/views/<op>`: args sorted by name, percent-encoded. Empty
    /// when the query has no argument.
    pub fn query_string(&self) -> String {
        self.args
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    }
}

/// Percent-encode everything but RFC 3986's unreserved characters.
pub(crate) fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Every block of `markdown` that does not parse, named `file`.
pub fn problems(file: &str, markdown: &str) -> Vec<QueryProblem> {
    find_query_blocks(markdown)
        .into_iter()
        .filter_map(|b| {
            QuerySpec::parse(&b.source)
                .err()
                .map(|reason| QueryProblem {
                    file: file.to_string(),
                    line: b.line,
                    source: b.source,
                    reason,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn shared_file() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/contract/query-block-v1.md");
        std::fs::read_to_string(path).expect("read query-block-v1.md")
    }

    /// FTR-016 R-1: every block under a `### valid` heading of the shared file parses and every
    /// one under `### invalid` fails with a reason. Read by heading, so a case katalogr adds to
    /// its file is checked here.
    #[test]
    fn the_shared_query_block_file_is_accepted_and_rejected_as_marked() {
        let text = shared_file();
        let lines: Vec<&str> = text.lines().collect();
        let (mut valid, mut invalid) = (0, 0);
        for block in find_query_blocks(&text) {
            let heading = lines[..block.line - 1]
                .iter()
                .rev()
                .find(|l| l.starts_with("### "))
                .map(|l| l.trim_start_matches("### ").trim())
                .unwrap_or("");
            let parsed = QuerySpec::parse(&block.source);
            match heading {
                "valid" => {
                    valid += 1;
                    assert!(
                        parsed.is_ok(),
                        "line {}: {:?} should parse: {:?}",
                        block.line,
                        block.source,
                        parsed
                    );
                }
                "invalid" => {
                    invalid += 1;
                    let reason = parsed.expect_err(&format!(
                        "line {}: {:?} should not parse",
                        block.line, block.source
                    ));
                    assert!(
                        !reason.trim().is_empty(),
                        "line {}: empty reason",
                        block.line
                    );
                }
                other => panic!("line {}: block under heading {other:?}", block.line),
            }
        }
        assert!(
            valid >= 13 && invalid >= 14,
            "{valid} valid, {invalid} invalid"
        );
    }

    /// FTR-016 R-1: fence rules and 1-based file lines.
    #[test]
    fn query_blocks_are_found_with_their_lines_and_not_inside_other_fences() {
        let md = "# T\n\
\n\
```katalogr-query\n\
structure\n\
```\n\
text\n\
~~~katalogr-query\n\
cycles\n\
~~~\n\
\x20  ```katalogr-query\n\
contracts\n\
\x20  ```\n\
````markdown\n\
```katalogr-query\n\
callers x\n\
```\n\
````\n\
```mermaid\n\
flowchart LR\n\
```\n\
\x20   ```katalogr-query\n\
four-space indent is code\n\
\x20   ```\n\
```katalogr-query\n\
unclosed\n";
        let found = find_query_blocks(md);
        let got: Vec<(usize, &str)> = found.iter().map(|b| (b.line, b.source.as_str())).collect();
        assert_eq!(
            got,
            vec![(3, "structure"), (7, "cycles"), (10, "contracts")],
            "nested, indented-four, other-info and unclosed fences are not blocks"
        );
        // A mismatched closer (shorter run, other character) does not close.
        let md = "````katalogr-query\nstructure\n```\n~~~~\n````\n";
        let found = find_query_blocks(md);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "structure\n```\n~~~~");
        // Windows line endings count the same lines.
        let found = find_query_blocks("a\r\n```katalogr-query\r\nstructure\r\n```\r\n");
        assert_eq!((found[0].line, found[0].source.as_str()), (2, "structure"));
    }

    #[test]
    fn a_query_parses_to_its_op_and_the_arguments_it_names() {
        let q = QuerySpec::parse("callers \"Config load\" --depth=3 --max-nodes 10").unwrap();
        assert_eq!(q.op, "callers");
        assert_eq!(q.args["symbol"], "Config load");
        assert_eq!(q.args["depth"], "3");
        assert_eq!(q.args["max-nodes"], "10");
        assert_eq!(q.args.len(), 3, "no defaults are filled in");
        let q = QuerySpec::parse("path a b --max-len 4").unwrap();
        assert_eq!((q.args["from"].as_str(), q.args["to"].as_str()), ("a", "b"));
        let q = QuerySpec::parse("structure --inferred").unwrap();
        assert_eq!(q.args["inferred"], "true");
        let q = QuerySpec::parse("process order --messages false").unwrap();
        assert_eq!(q.args["messages"], "false");
        let q = QuerySpec::parse("flow f --reverse true --depth 2").unwrap();
        assert_eq!(q.args["reverse"], "true");
        assert_eq!(q.args["depth"], "2");
        // A switch may stand before a positional.
        assert!(QuerySpec::parse("flow --reverse f").is_ok());
    }

    #[test]
    fn a_bad_query_says_why() {
        let why = |s: &str| QuerySpec::parse(s).unwrap_err();
        assert!(why("").contains("empty"));
        assert!(why("structure\ncycles").contains("one line"));
        assert!(why("nope").contains("operation"));
        assert!(why("structure --level galaxy").contains("file"));
        assert!(why("structure --max-nodes 0").contains("above 0"));
        assert!(why("structure --max-nodes").contains("value"));
        assert!(why("structure --max-nodes abc").contains("whole number"));
        assert!(why("structure --level file --level package").contains("once"));
        assert!(why("structure --bogus 1").contains("--bogus"));
        assert!(why("callers").contains("symbol"));
        assert!(why("path only").contains("to"));
        assert!(why("cycles extra").contains("no argument"));
        assert!(why("callers \"open").contains("quote"));
        assert!(why("structure --inferred=maybe").contains("true or false"));
    }

    #[test]
    fn the_query_string_is_sorted_and_percent_encoded() {
        let q = QuerySpec::parse("callers \"a b&c\" --max-nodes 5 --depth 2").unwrap();
        assert_eq!(q.query_string(), "depth=2&max-nodes=5&symbol=a%20b%26c");
        let q = QuerySpec::parse("structure").unwrap();
        assert_eq!(q.query_string(), "");
    }

    #[test]
    fn problems_name_the_file_and_the_line_of_each_bad_block() {
        let md = "x\n```katalogr-query\nstructure\n```\n```katalogr-query\nwhat\n```\n";
        let p = problems("docs/a.md", md);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].file.as_str(), p[0].line), ("docs/a.md", 5));
        assert_eq!(p[0].source, "what");
        assert!(!p[0].reason.is_empty());
    }
}
