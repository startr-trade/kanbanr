//! The published read API (FEAT-177): the daemon's **public** read routes, and the OpenAPI 3.1
//! description of them, version 1.
//!
//! The public set is the routes the monitor uses, plus the obvious reads. Every other read route
//! the daemon answers is **internal**: served, but not covered, and free to change. Write routes
//! are never part of it. A breaking change to a public route means a v2, under `/api/v2/`, with v1
//! kept for a deprecation period (`docs/src/project/stability.md`).
//!
//! The response schemas are derived from the Rust types the routes serialize (feature `schema`),
//! so a field cannot change without the description changing with it. [`document`] is
//! deterministic: `cargo run -p kanbanr-core --features schema --example openapi` writes it to
//! `docs/src/reference/openapi.json`, the daemon serves those bytes, and the contract test fails
//! when the committed copy and the code disagree.

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Map, Value, json};

/// The version of the read API this document describes.
pub const API_VERSION: &str = "1";

/// Where a parameter goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum In {
    Path,
    Query,
}

/// What a parameter holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    String,
    Integer,
    Boolean,
    /// One of these strings.
    OneOf(&'static [&'static str]),
}

/// One parameter of a route.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub location: In,
    pub required: bool,
    pub kind: Kind,
    pub description: &'static str,
}

/// What a route answers with when it succeeds.
#[derive(Clone, Copy)]
pub enum Response {
    /// `application/json`, with this schema.
    Json(fn(&mut SchemaGenerator) -> Schema),
    /// Text of this media type.
    Text(&'static str),
    /// A file's bytes, with the media type its name implies.
    Bytes,
    /// `text/event-stream`: event `changed`, data the project id that changed, or `*`.
    Sse,
    /// Text of this media type, or `application/json` with `?format=json`.
    TextOrJson(&'static str, fn(&mut SchemaGenerator) -> Schema),
}

/// One public route.
#[derive(Clone, Copy)]
pub struct RouteDoc {
    /// The path as served, with `{name}` for path parameters.
    pub path: &'static str,
    pub summary: &'static str,
    pub params: &'static [Param],
    pub response: Response,
}

impl RouteDoc {
    /// Whether the route answers about a project, and so can fail: an unknown project or item is
    /// a 404, a board newer than the daemon a 409, a malformed request a 400.
    pub fn can_fail(&self) -> bool {
        self.path.starts_with("/api/projects/") || self.path.starts_with("/api/portfolio")
    }
}

/// The status codes an error can carry, and what each means. Errors are `text/plain`.
pub const ERRORS: &[(u16, &str)] = &[
    (
        400,
        "The request is malformed: a missing or invalid parameter, or an unsupported format.",
    ),
    (404, "No such project, item, sprint or document."),
    (
        409,
        "The board was written by a newer kanbanr than this daemon; upgrade it.",
    ),
    (500, "The board could not be read."),
];

fn schema_of<T: JsonSchema>(g: &mut SchemaGenerator) -> Schema {
    g.subschema_for::<T>()
}

fn any_object(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "object", "description": "An OpenAPI 3.1 document: this one." })
}

const PROJECT: Param = Param {
    name: "project",
    location: In::Path,
    required: true,
    kind: Kind::String,
    description: "The project's id.",
};
const CODE: Param = Param {
    name: "code",
    location: In::Path,
    required: true,
    kind: Kind::String,
    description: "The item's code, such as FEAT-001.",
};
const DOC_PATH: Param = Param {
    name: "path",
    location: In::Query,
    required: true,
    kind: Kind::String,
    description: "The document's path under the project's docs, forward-slashed.",
};

/// The public v1 routes, in the order the document lists them.
pub fn public_routes() -> Vec<RouteDoc> {
    use crate::server::{Health, Meta};
    vec![
        // ---- meta and health ----
        RouteDoc {
            path: "/healthz",
            summary: "Liveness probe",
            params: &[],
            response: Response::Json(schema_of::<Health>),
        },
        RouteDoc {
            path: "/api/meta",
            summary: "What this daemon can do, and who an approval here would be attributed to",
            params: &[],
            response: Response::Json(schema_of::<Meta>),
        },
        RouteDoc {
            path: "/api/openapi.json",
            summary: "This description of the public read API",
            params: &[],
            response: Response::Json(any_object),
        },
        // ---- projects ----
        RouteDoc {
            path: "/api/projects",
            summary: "The projects on the board",
            params: &[],
            response: Response::Json(schema_of::<Vec<crate::dispatch::ProjectSummary>>),
        },
        RouteDoc {
            path: "/api/projects/{project}",
            summary: "A project: its configuration, milestones and items",
            params: &[PROJECT],
            response: Response::Json(schema_of::<crate::store::Project>),
        },
        // ---- project views ----
        RouteDoc {
            path: "/api/projects/{project}/charter",
            summary: "The project's charter: purpose, goals, scope and constraints",
            params: &[PROJECT],
            response: Response::Json(schema_of::<crate::charter::Charter>),
        },
        RouteDoc {
            path: "/api/projects/{project}/readiness",
            summary: "What each live item is missing, and its next stages, keyed by code",
            params: &[PROJECT],
            response: Response::Json(schema_of::<crate::readiness::ProjectReadiness>),
        },
        RouteDoc {
            path: "/api/projects/{project}/features/{code}/readiness",
            summary: "What one item is missing, and what its next stages ask",
            params: &[PROJECT, CODE],
            response: Response::Json(schema_of::<crate::readiness::ReadinessView>),
        },
        RouteDoc {
            path: "/api/projects/{project}/features/{code}/diagrams",
            summary: "The Mermaid blocks of an item's spec and affects_docs, with the lint's findings",
            params: &[PROJECT, CODE],
            response: Response::Json(schema_of::<crate::diagrams::ItemDiagrams>),
        },
        RouteDoc {
            path: "/api/projects/{project}/review",
            summary: "What is waiting for a person's agreement, in the order to ask it",
            params: &[PROJECT],
            response: Response::Json(schema_of::<Vec<crate::review::ReviewEntry>>),
        },
        RouteDoc {
            path: "/api/projects/{project}/releases",
            summary: "The project's releases, each with what it ships",
            params: &[PROJECT],
            response: Response::Json(schema_of::<Vec<crate::releases::ReleaseReport>>),
        },
        RouteDoc {
            path: "/api/projects/{project}/sprints",
            summary: "The project's sprints",
            params: &[PROJECT],
            response: Response::Json(schema_of::<Vec<crate::sprints::Sprint>>),
        },
        RouteDoc {
            path: "/api/projects/{project}/sprints/{sprint}",
            summary: "One sprint's report",
            params: &[
                PROJECT,
                Param {
                    name: "sprint",
                    location: In::Path,
                    required: true,
                    kind: Kind::String,
                    description: "The sprint's code, or `active` for the sprint in progress.",
                },
            ],
            response: Response::Json(schema_of::<crate::sprints::SprintReport>),
        },
        RouteDoc {
            path: "/api/projects/{project}/adrs",
            summary: "The project's architecture decision records",
            params: &[PROJECT],
            response: Response::Json(schema_of::<Vec<crate::adr::Adr>>),
        },
        RouteDoc {
            path: "/api/projects/{project}/lessons",
            summary: "The lessons worth showing, most believed first",
            params: &[
                PROJECT,
                Param {
                    name: "for",
                    location: In::Query,
                    required: false,
                    kind: Kind::String,
                    description: "Only the lessons that bear on this item.",
                },
                Param {
                    name: "all",
                    location: In::Query,
                    required: false,
                    kind: Kind::String,
                    description: "Present: every lesson, retired ones included.",
                },
            ],
            response: Response::Json(schema_of::<Vec<crate::lessons::Lesson>>),
        },
        RouteDoc {
            path: "/api/projects/{project}/process",
            summary: "How the project's workflow stands against the process it was applied from",
            params: &[PROJECT],
            response: Response::Json(schema_of::<crate::library::ProcessStatus>),
        },
        RouteDoc {
            path: "/api/projects/{project}/gates",
            summary: "The gate each status's moves are judged by, keyed by status: role-addressed \
                      gates resolved, the built-in rule included",
            params: &[PROJECT],
            response: Response::Json(
                schema_of::<std::collections::BTreeMap<String, crate::config::Gate>>,
            ),
        },
        RouteDoc {
            path: "/api/projects/{project}/releases/{version}/cut-preview",
            summary: "What cutting a release now would ship and carry over, with nothing written",
            params: &[
                PROJECT,
                Param {
                    name: "version",
                    location: In::Path,
                    required: true,
                    kind: Kind::String,
                    description: "The release's version, such as 1.0.",
                },
            ],
            response: Response::Json(schema_of::<Vec<crate::releases::CutPreview>>),
        },
        // ---- diagrams ----
        RouteDoc {
            path: "/api/projects/{project}/workflow",
            summary: "The workflow as a Mermaid state diagram",
            params: &[
                PROJECT,
                Param {
                    name: "format",
                    location: In::Query,
                    required: true,
                    kind: Kind::OneOf(&["mermaid"]),
                    description: "The diagram language; only `mermaid`.",
                },
            ],
            response: Response::Text("text/plain; charset=utf-8"),
        },
        RouteDoc {
            path: "/api/projects/{project}/gantt",
            summary: "The schedule as a Mermaid gantt diagram, by milestone",
            params: &[PROJECT],
            response: Response::Text("text/plain; charset=utf-8"),
        },
        // ---- docs ----
        RouteDoc {
            path: "/api/projects/{project}/docs",
            summary: "The project's documentation tree",
            params: &[PROJECT],
            response: Response::Json(schema_of::<crate::docs::DocFolder>),
        },
        RouteDoc {
            path: "/api/projects/{project}/docs/content",
            summary: "A document's text",
            params: &[PROJECT, DOC_PATH],
            response: Response::Text("text/markdown; charset=utf-8"),
        },
        RouteDoc {
            path: "/api/projects/{project}/docs/raw",
            summary: "A documentation file's bytes, such as an image",
            params: &[PROJECT, DOC_PATH],
            response: Response::Bytes,
        },
        // ---- activity ----
        RouteDoc {
            path: "/api/projects/{project}/activity",
            summary: "Recent changes to the project, newest first",
            params: &[
                PROJECT,
                Param {
                    name: "ref",
                    location: In::Query,
                    required: false,
                    kind: Kind::String,
                    description: "Only the changes to this item.",
                },
                Param {
                    name: "ongoing",
                    location: In::Query,
                    required: false,
                    kind: Kind::Boolean,
                    description: "Only changes to items in a status that is not a board column.",
                },
                Param {
                    name: "limit",
                    location: In::Query,
                    required: false,
                    kind: Kind::Integer,
                    description: "At most this many entries; 25 when absent.",
                },
            ],
            response: Response::Json(schema_of::<Vec<crate::activity::Activity>>),
        },
        // ---- export ----
        RouteDoc {
            path: "/api/projects/{project}/features/{code}/export",
            summary: "One item, as Markdown, or as JSON with `?format=json`",
            params: &[
                PROJECT,
                CODE,
                Param {
                    name: "format",
                    location: In::Query,
                    required: false,
                    kind: Kind::OneOf(&["md", "json"]),
                    description: "`json` for JSON; `md`, or none, for Markdown.",
                },
            ],
            response: Response::TextOrJson(
                "text/markdown; charset=utf-8",
                schema_of::<crate::export::FeatureExport<'static>>,
            ),
        },
        // ---- portfolio ----
        RouteDoc {
            path: "/api/portfolio",
            summary: "The workspace: its programs and their projects",
            params: &[],
            response: Response::Json(schema_of::<crate::portfolio::PortfolioView>),
        },
        RouteDoc {
            path: "/api/portfolio/rollups",
            summary: "Progress rolled up by program and project",
            params: &[],
            response: Response::Json(schema_of::<crate::portfolio::RollupReport>),
        },
        RouteDoc {
            path: "/api/portfolio/board",
            summary: "Every project's items on one board, in normalized lanes",
            params: &[],
            response: Response::Json(schema_of::<crate::portfolio::BoardReport>),
        },
        // ---- live updates ----
        RouteDoc {
            path: "/api/projects/{project}/events",
            summary: "Server-sent events: `changed` whenever this project changes",
            params: &[PROJECT],
            response: Response::Sse,
        },
        RouteDoc {
            path: "/api/events",
            summary: "Server-sent events: `changed` whenever anything on the board changes",
            params: &[],
            response: Response::Sse,
        },
    ]
}

/// The settings every schema here is generated with: JSON Schema 2020-12 (what OpenAPI 3.1
/// uses), definitions under `components/schemas`, and the shape a response is *serialized* in.
pub fn settings() -> SchemaSettings {
    SchemaSettings::draft2020_12()
        .with(|s| {
            s.definitions_path = "/components/schemas".into();
            s.meta_schema = None;
        })
        .for_serialize()
        .with_transform(absent_is_not_null as fn(&mut Schema))
}

/// A field a response may leave out is left out when it has nothing to say, never sent as null:
/// every optional field the responses can omit is an `Option` skipped when `None`
/// (`skip_serializing_if = "Option::is_none"`). The derived schema would otherwise also allow
/// null for it, and a client would have to handle a value it never receives. The contract test
/// validates real bodies against the result, so a field that did send null would fail there.
fn absent_is_not_null(schema: &mut Schema) {
    if let Some(obj) = schema.as_object_mut() {
        let required: Vec<String> = obj
            .get("required")
            .and_then(Value::as_array)
            .map(|r| {
                r.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(Value::Object(props)) = obj.get_mut("properties") {
            for (name, prop) in props.iter_mut() {
                if !required.contains(name) {
                    strip_null(prop);
                }
            }
        }
    }
    schemars::transform::transform_subschemas(&mut (absent_is_not_null as fn(&mut Schema)), schema);
}

/// `schema` without its null alternative.
fn strip_null(schema: &mut Value) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };
    let mut stripped = false;
    if let Some(Value::Array(types)) = obj.get_mut("type") {
        let before = types.len();
        types.retain(|t| t != "null");
        stripped = types.len() != before;
        if types.len() == 1 {
            let only = types[0].clone();
            obj.insert("type".into(), only);
        }
    }
    if let Some(Value::Array(alts)) = obj.get_mut("anyOf") {
        let before = alts.len();
        alts.retain(|a| a.get("type").and_then(Value::as_str) != Some("null"));
        stripped |= alts.len() != before;
        if alts.len() == 1 {
            let only = alts.remove(0);
            obj.remove("anyOf");
            if let Value::Object(fields) = only {
                for (k, v) in fields {
                    obj.entry(k).or_insert(v);
                }
            }
        }
    }
    if stripped && obj.get("default") == Some(&Value::Null) {
        obj.remove("default");
    }
}

fn param_schema(kind: Kind) -> Value {
    match kind {
        Kind::String => json!({ "type": "string" }),
        Kind::Integer => json!({ "type": "integer", "minimum": 0 }),
        Kind::Boolean => json!({ "type": "boolean" }),
        Kind::OneOf(values) => json!({ "type": "string", "enum": values }),
    }
}

fn sse() -> Value {
    json!({
        "description": "A stream of server-sent events. Each is `event: changed`, with `data:` the \
            id of the project that changed, or `*` for a change to the board as a whole.",
        "content": { "text/event-stream": { "schema": { "type": "string" } } },
    })
}

fn operation(route: &RouteDoc, g: &mut SchemaGenerator) -> Value {
    let params: Vec<Value> = route
        .params
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "in": match p.location { In::Path => "path", In::Query => "query" },
                "required": p.required,
                "description": p.description,
                "schema": param_schema(p.kind),
            })
        })
        .collect();
    let ok = match route.response {
        Response::Json(schema) => json!({
            "description": "OK",
            "content": { "application/json": { "schema": schema(g) } },
        }),
        Response::Text(mime) => json!({
            "description": "OK",
            "content": { mime: { "schema": { "type": "string" } } },
        }),
        Response::Bytes => json!({
            "description": "The file's bytes; the media type follows its extension (png, jpg, \
                gif, svg, webp, pdf; md and txt as text/plain), application/octet-stream \
                otherwise.",
            "content": { "*/*": { "schema": { "type": "string", "contentMediaType": "application/octet-stream" } } },
        }),
        Response::Sse => sse(),
        Response::TextOrJson(mime, schema) => json!({
            "description": "OK: Markdown, or JSON with `?format=json`.",
            "content": {
                mime: { "schema": { "type": "string" } },
                "application/json": { "schema": schema(g) },
            },
        }),
    };
    let mut responses = Map::new();
    responses.insert("200".into(), ok);
    if route.can_fail() {
        for (code, _) in ERRORS {
            responses.insert(
                code.to_string(),
                json!({ "$ref": format!("#/components/responses/E{code}") }),
            );
        }
    }
    let mut op = json!({
        "summary": route.summary,
        "operationId": operation_id(route.path),
        "responses": responses,
    });
    if !params.is_empty() {
        op["parameters"] = Value::Array(params);
    }
    json!({ "get": op })
}

/// A stable operation id from a path: `/api/projects/{project}/docs/raw` → `getProjectDocsRaw`.
fn operation_id(path: &str) -> String {
    let mut id = String::from("get");
    for seg in path.split('/').filter(|s| !s.is_empty() && *s != "api") {
        if seg.starts_with('{') {
            continue;
        }
        for word in seg.split(['-', '.', '_']) {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                id.extend(first.to_uppercase());
                id.push_str(chars.as_str());
            }
        }
    }
    // Two routes differing only in a parameter would collide; the one with the parameter says so.
    if path.ends_with('}') {
        id.push_str("One");
    }
    id
}

/// The OpenAPI 3.1 document for v1. Deterministic: every object's keys are sorted, so it can be
/// committed and diffed.
pub fn document() -> Value {
    let mut g = settings().into_generator();
    let mut paths = Map::new();
    for route in public_routes() {
        paths.insert(route.path.to_string(), operation(&route, &mut g));
    }
    let mut errors = Map::new();
    for (code, why) in ERRORS {
        errors.insert(
            format!("E{code}"),
            json!({
                "description": why,
                "content": { "text/plain; charset=utf-8": { "schema": { "type": "string" } } },
            }),
        );
    }
    let doc = json!({
        "openapi": "3.1.0",
        "jsonSchemaDialect": "https://json-schema.org/draft/2020-12/schema",
        "info": {
            "title": "kanbanr read API",
            "version": API_VERSION,
            "summary": "The view daemon's public read routes.",
            "description": "The read-only routes `kanbanr serve` answers for the monitor and for \
                tools built on the board. Version 1 stays at `/api/...`; a breaking change would \
                be published as v2 under `/api/v2/...`, with v1 kept for a deprecation period. \
                Routes the daemon serves that are not listed here are internal and may change. \
                Write routes (`serve --allow-writes`) are not part of this contract. Errors are \
                `text/plain`. CORS is permissive; there is no authentication.",
            "license": { "name": "MIT OR Apache-2.0", "identifier": "MIT OR Apache-2.0" },
        },
        "paths": paths,
        "components": {
            "schemas": g.take_definitions(true),
            "responses": errors,
        },
    });
    sorted(doc)
}

/// `value` with every object's keys in sorted order, whatever order the map type keeps.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// The document as committed and served: pretty JSON with a final newline.
pub fn document_text() -> String {
    let mut text = serde_json::to_string_pretty(&document()).expect("a JSON value serializes");
    text.push('\n');
    text
}

/// Where the route table goes in `docs/src/reference/http-api.md`: between these two lines.
pub const TABLE_START: &str = "<!-- generated from openapi.json: start -->";
pub const TABLE_END: &str = "<!-- generated from openapi.json: end -->";

/// The public routes as a Markdown table, generated from [`document`]: route, parameters, what
/// a success answers with, and what the route is for.
pub fn route_table() -> String {
    let doc = document();
    let mut out =
        String::from("| Route | Parameters | Answers | What it is |\n|---|---|---|---|\n");
    for route in public_routes() {
        let op = &doc["paths"][route.path]["get"];
        let params: Vec<String> = route
            .params
            .iter()
            .filter(|p| p.location == In::Query)
            .map(|p| {
                let values = match p.kind {
                    Kind::OneOf(v) => format!("={}", v.join("\\|")),
                    _ => String::new(),
                };
                let required = if p.required { "" } else { " (optional)" };
                format!("`{}{values}`{required}", p.name)
            })
            .collect();
        let content = op["responses"]["200"]["content"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let answers: Vec<String> = content
            .iter()
            .map(|(mime, c)| match mime.as_str() {
                "application/json" => format!("JSON `{}`", schema_label(&c["schema"])),
                "*/*" => "the file's bytes".to_string(),
                other => format!("`{}`", other.split(';').next().unwrap_or(other)),
            })
            .collect();
        out.push_str(&format!(
            "| `GET {}` | {} | {} | {} |\n",
            route.path,
            if params.is_empty() {
                "—".to_string()
            } else {
                params.join(", ")
            },
            answers.join(", or "),
            route.summary.replace('|', "\\|"),
        ));
    }
    out
}

/// A short name for a response schema: `ReviewEntry[]`, `Project`, `object`.
fn schema_label(schema: &Value) -> String {
    if let Some(r) = schema["$ref"].as_str() {
        return r.trim_start_matches("#/components/schemas/").to_string();
    }
    if schema["type"] == "array" {
        return format!("{}[]", schema_label(&schema["items"]));
    }
    schema["type"].as_str().unwrap_or("any").to_string()
}

/// `page` with the text between [`TABLE_START`] and [`TABLE_END`] replaced by [`route_table`];
/// `None` when the page lacks either marker.
pub fn with_route_table(page: &str) -> Option<String> {
    let start = page.find(TABLE_START)? + TABLE_START.len();
    let end = page[start..].find(TABLE_END)? + start;
    Some(format!(
        "{}\n\n{}\n{}",
        &page[..start],
        route_table(),
        &page[end..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed copy is compared byte for byte, so two builds must produce the same bytes.
    #[test]
    fn document_is_deterministic() {
        let first = document_text();
        assert_eq!(first, document_text());
        // Sorted at every level, whatever order the generator inserted things in.
        fn check(v: &Value, at: &str) {
            match v {
                Value::Object(m) => {
                    let keys: Vec<&String> = m.keys().collect();
                    let mut want = keys.clone();
                    want.sort();
                    assert_eq!(keys, want, "keys out of order at {at}");
                    for (k, v) in m {
                        check(v, &format!("{at}/{k}"));
                    }
                }
                Value::Array(items) => items.iter().for_each(|v| check(v, at)),
                _ => {}
            }
        }
        let doc: Value = serde_json::from_str(&first).unwrap();
        check(&doc, "");
        // Every public route is there, and every schema a route names is defined.
        let paths = doc["paths"].as_object().unwrap();
        assert_eq!(paths.len(), public_routes().len());
        let defined = doc["components"]["schemas"].as_object().unwrap();
        for r in first.match_indices("#/components/schemas/") {
            let name: String = first[r.0 + r.1.len()..]
                .chars()
                .take_while(|c| *c != '"')
                .collect();
            assert!(
                defined.contains_key(&name),
                "{name} is referenced, not defined"
            );
        }
    }
}
