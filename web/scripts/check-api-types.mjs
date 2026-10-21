#!/usr/bin/env node
//
// FEAT-177: the monitor's idea of the read API is the published one.
//
// Reads the OpenAPI document the daemon serves (docs/src/reference/openapi.json, generated from the
// Rust response types), finds every JSON read in src/api.ts — `getJson<T>(apiUrl(`...`))` — and
// compares the TypeScript type T, field by field and through every type it names, with the schema
// of the route it calls:
//   - every field the monitor reads must be one the route sends;
//   - a field the monitor treats as always there must be one the route always sends;
//   - a field the monitor treats as never null must be one the route never sends as null;
//   - strings, numbers, booleans, arrays and maps must be what the schema says, and a string the
//     monitor narrows to a set of values must only expect values the schema allows.
// A route the monitor calls that the document does not list is internal: named, not checked.
//
// Usage: npm run check:api-types
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const doc = JSON.parse(readFileSync(here("../../docs/src/reference/openapi.json"), "utf8"));
const apiFile = here("../src/api.ts");

const program = ts.createProgram([apiFile], {
  strict: true,
  target: ts.ScriptTarget.ES2020,
  module: ts.ModuleKind.ESNext,
  moduleResolution: ts.ModuleResolutionKind.Bundler,
  allowImportingTsExtensions: true,
  noEmit: true,
  skipLibCheck: true,
  lib: ["lib.es2020.d.ts", "lib.dom.d.ts"],
});
const checker = program.getTypeChecker();
const source = program.getSourceFile(apiFile);

let problems = [];
const internal = [];
let checked = 0;

/** The documented path a template literal in api.ts names, e.g. `/projects/${…}/sprints/${…}`. */
function docPath(template) {
  const name = { projects: "{project}", features: "{code}", sprints: "{sprint}", releases: "{version}" };
  // A substitution glued to a segment (`activity${params}`) is a query string, not a segment.
  const segs = template
    .split("?")[0]
    .split("/")
    .filter(Boolean)
    .map((s) => (s !== "${}" && s.endsWith("${}") ? s.slice(0, -3) : s));
  const out = segs.map((s, i) => (s === "${}" ? name[segs[i - 1]] ?? "{param}" : s));
  return "/api/" + out.join("/");
}

/** The text of an `apiUrl(...)` argument with every substitution written `${}`. */
function templateText(node) {
  if (ts.isNoSubstitutionTemplateLiteral(node) || ts.isStringLiteral(node)) return node.text;
  if (ts.isTemplateExpression(node)) {
    return node.head.text + node.templateSpans.map((s) => "${}" + s.literal.text).join("");
  }
  return null;
}

function resolve(schema) {
  let s = schema;
  for (let i = 0; s && s.$ref && i < 20; i++) {
    s = doc.components.schemas[s.$ref.replace("#/components/schemas/", "")];
  }
  return s ?? {};
}

/** A schema's alternatives (anyOf/oneOf), each resolved, and whether null is one of them. */
function alternatives(schema) {
  const s = resolve(schema);
  const alts = s.anyOf ?? s.oneOf;
  if (alts) {
    const out = alts.flatMap((a) => alternatives(a).alts);
    return { alts: out.filter((a) => !isNullSchema(a)), nullable: alts.some((a) => alternatives(a).nullable) || out.some(isNullSchema) };
  }
  const types = Array.isArray(s.type) ? s.type : s.type ? [s.type] : [];
  if (types.length > 1) {
    return {
      alts: types.filter((t) => t !== "null").map((t) => ({ ...s, type: t })),
      nullable: types.includes("null"),
    };
  }
  if (isNullSchema(s)) return { alts: [], nullable: true };
  return { alts: [s], nullable: false };
}

function isNullSchema(s) {
  return s.type === "null" || (s.const === null && "const" in s);
}

function kindOf(s) {
  if (s.type) return s.type;
  if (s.enum) return typeof s.enum[0] === "string" ? "string" : "unknown";
  if ("const" in s) return typeof s.const === "string" ? "string" : typeof s.const;
  if (s.properties || s.additionalProperties) return "object";
  return "any";
}

function stringValues(s) {
  if (s.enum) return s.enum;
  if ("const" in s) return [s.const];
  return null; // any string
}

const seen = new Set();

/**
 * Compare a TS type with a schema; `where` names the place for a problem. For an optional
 * property, `undefined` is its absence, not a null.
 */
function compare(type, schema, where, optional = false) {
  // A type already being compared with this schema further up is a cycle: nothing new below.
  const key = `${checker.typeToString(type)}@${JSON.stringify(schema).slice(0, 200)}`;
  if (seen.has(key)) return;
  seen.add(key);
  try {
    compareOnce(type, schema, where, optional);
  } finally {
    seen.delete(key);
  }
}

function compareOnce(type, schema, where, optional) {
  const members = type.isUnion() ? type.types : [type];
  const tsNullable = members.some(
    (m) => m.flags & ts.TypeFlags.Null || (!optional && m.flags & ts.TypeFlags.Undefined),
  );
  const nonNull = members.filter((m) => !(m.flags & (ts.TypeFlags.Null | ts.TypeFlags.Undefined)));
  const { alts, nullable } = alternatives(schema);
  if (nullable && !tsNullable) {
    problems.push(`${where}: the API can send null; the monitor's type does not allow it`);
  }
  if (alts.some((a) => kindOf(a) === "any")) return; // any JSON value

  // A union of string literals: every value the monitor expects must be one the API can send.
  const literals = nonNull.filter((m) => m.isStringLiteral());
  if (literals.length && literals.length === nonNull.length) {
    const allowed = alts.flatMap((a) => (kindOf(a) === "string" ? stringValues(a) ?? ["*"] : []));
    if (!allowed.length) problems.push(`${where}: the monitor expects a string; the API sends ${alts.map(kindOf).join(" | ")}`);
    else if (!allowed.includes("*")) {
      for (const l of literals) {
        if (!allowed.includes(l.value)) problems.push(`${where}: the monitor expects "${l.value}", which the API never sends (it sends ${allowed.join(", ")})`);
      }
    }
    return;
  }
  for (const m of nonNull) {
    const ok = alts.filter((a) => matches(m, a));
    if (!ok.length) {
      problems.push(`${where}: the monitor reads ${checker.typeToString(m)}; the API sends ${alts.map(kindOf).join(" | ") || "nothing"}`);
      continue;
    }
    // Objects and arrays: the alternative of that shape the type fits best — one an untagged
    // union's member fits entirely, else the one with the fewest problems.
    let best = null;
    for (const a of ok) {
      const outer = problems;
      problems = [];
      if (checker.isArrayType(m) || checker.isTupleType(m)) {
        const [item] = checker.getTypeArguments(m);
        if (item && a.items) compare(item, a.items, `${where}[]`);
      } else if (m.flags & ts.TypeFlags.Object) {
        compareObject(m, a, where);
      }
      const found = problems;
      problems = outer;
      if (!best || found.length < best.length) best = found;
      if (!found.length) break;
    }
    problems.push(...best);
  }
}

function matches(m, a) {
  const k = kindOf(a);
  if (m.flags & (ts.TypeFlags.String | ts.TypeFlags.StringLiteral)) return k === "string";
  if (m.flags & (ts.TypeFlags.Number | ts.TypeFlags.NumberLiteral)) return k === "integer" || k === "number";
  if (m.flags & (ts.TypeFlags.Boolean | ts.TypeFlags.BooleanLiteral)) return k === "boolean";
  if (checker.isArrayType(m) || checker.isTupleType(m)) return k === "array";
  if (m.flags & ts.TypeFlags.Object) return k === "object";
  if (m.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown)) return true;
  return false;
}

function compareObject(type, schema, where) {
  const index = checker.getIndexTypeOfType(type, ts.IndexKind.String);
  if (index) {
    const values = schema.additionalProperties;
    if (values === undefined || values === false) {
      problems.push(`${where}: the monitor reads a map; the API sends an object with named fields`);
    } else if (values !== true) {
      compare(index, values, `${where}{}`);
    }
  }
  const props = schema.properties ?? {};
  const required = new Set(schema.required ?? []);
  for (const p of checker.getPropertiesOfType(type)) {
    const name = p.getName();
    const at = `${where}.${name}`;
    if (!(name in props)) {
      problems.push(`${at}: the monitor reads it; the API does not send it`);
      continue;
    }
    const optional = (p.flags & ts.SymbolFlags.Optional) !== 0;
    if (!optional && !required.has(name)) {
      problems.push(`${at}: the monitor treats it as always there; the API may leave it out`);
    }
    const decl = p.valueDeclaration ?? p.declarations?.[0];
    const t = checker.getTypeOfSymbolAtLocation(p, decl ?? source);
    compare(t, props[name], at, optional);
  }
}

function visit(node) {
  if (
    ts.isCallExpression(node) &&
    ts.isIdentifier(node.expression) &&
    node.expression.text === "getJson" &&
    node.typeArguments?.length === 1
  ) {
    const arg = node.arguments[0];
    const inner = arg && ts.isCallExpression(arg) && arg.arguments[0];
    const text = inner && templateText(inner);
    if (text) {
      const path = docPath(text);
      const route = doc.paths[path];
      if (!route) {
        internal.push(path);
      } else {
        const content = route.get.responses["200"].content ?? {};
        const schema = content["application/json"]?.schema;
        if (!schema) {
          problems.push(`${path}: the monitor reads JSON; the API sends ${Object.keys(content).join(", ")}`);
        } else {
          const type = checker.getTypeFromTypeNode(node.typeArguments[0]);
          compare(type, schema, `${path} ${node.typeArguments[0].getText(source)}`);
          checked++;
        }
      }
    }
  }
  ts.forEachChild(node, visit);
}
visit(source);

if (checked === 0) problems.push("found no getJson<T>(apiUrl(...)) call in src/api.ts to check");
if (internal.length) {
  console.log(`not in the published API (internal, not checked): ${[...new Set(internal)].join(", ")}`);
}
if (problems.length) {
  console.error(`check:api-types — ${problems.length} problem(s):\n  ${problems.join("\n  ")}`);
  process.exit(1);
}
console.log(`check:api-types: ${checked} monitor reads match the published API`);
