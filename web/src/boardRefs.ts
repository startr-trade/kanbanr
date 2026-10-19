/**
 * References between board artifacts, found in markdown (FEAT-181).
 *
 * A port of `kanbanr-core/src/refs.rs`: the same rules, checked against the same fixture
 * (`web/scripts/refs-fixture.json`), so the monitor links exactly what doctor checks. A token is a
 * reference only when its shape belongs to this board — an item or milestone code with a prefix
 * the board uses, an ADR id, a goal id when the charter has goals, or a doc path — and code spans,
 * fences and HTML comments are skipped.
 */

export type Ref =
  | { kind: "item"; code: string; requirement?: string }
  | { kind: "milestone"; code: string }
  | { kind: "adr"; id: string }
  | { kind: "goal"; id: string }
  | { kind: "doc"; path: string };

export interface Found {
  reference: Ref;
  line: number;
  explicit: boolean;
}

/** What the board has, to resolve references against. */
export interface BoardIndex {
  project: string;
  itemPrefixes: Set<string>;
  milestonePrefixes: Set<string>;
  /** code -> requirement ids */
  items: Map<string, Set<string>>;
  milestones: Set<string>;
  /** id -> doc path */
  adrs: Map<string, string>;
  goals: Set<string>;
  docs: Set<string>;
}

/** The letters before the number in a code: `FEAT` of `FEAT-012`. */
export function prefixOf(code: string): string | null {
  const at = code.lastIndexOf("-");
  if (at <= 0 || at === code.length - 1) return null;
  return /^\d+$/.test(code.slice(at + 1)) ? code.slice(0, at) : null;
}

/** Build an index from what the monitor fetches. */
export function makeIndex(parts: {
  project: string;
  items: { code: string; requirements: string[] }[];
  milestones: string[];
  adrs: { id: string; path: string }[];
  goals: string[];
  docs: string[];
}): BoardIndex {
  const index: BoardIndex = {
    project: parts.project,
    itemPrefixes: new Set(),
    milestonePrefixes: new Set(),
    items: new Map(),
    milestones: new Set(),
    adrs: new Map(),
    goals: new Set(parts.goals),
    docs: new Set(parts.docs),
  };
  for (const item of parts.items) {
    const prefix = prefixOf(item.code);
    if (prefix) index.itemPrefixes.add(prefix);
    index.items.set(item.code, new Set(item.requirements));
  }
  for (const code of parts.milestones) {
    const prefix = prefixOf(code);
    if (prefix) index.milestonePrefixes.add(prefix);
    index.milestones.add(code);
  }
  for (const adr of parts.adrs) index.adrs.set(adr.id, adr.path);
  return index;
}

/** Is the reference something the board has? */
export function resolves(index: BoardIndex, r: Ref): boolean {
  switch (r.kind) {
    case "item": {
      const reqs = index.items.get(r.code);
      if (!reqs) return false;
      return r.requirement ? reqs.has(r.requirement) : true;
    }
    case "milestone":
      return index.milestones.has(r.code);
    case "adr":
      return index.adrs.has(r.id);
    case "goal":
      return index.goals.has(r.id);
    case "doc":
      return index.docs.has(r.path);
  }
}

/** The monitor path a reference links to, when the board has it. */
export function href(index: BoardIndex, r: Ref): string | null {
  if (!resolves(index, r)) return null;
  return hrefOf(index.project, r, index);
}

/** The path a reference would have, whether or not the board has it. */
function hrefOf(project: string, r: Ref, index: BoardIndex): string {
  const p = `/p/${encodeURIComponent(project)}`;
  switch (r.kind) {
    case "item":
      return `${p}/feature/${r.code}${r.requirement ? `#${r.requirement}` : ""}`;
    case "milestone":
      return `${p}/milestone/${r.code}`;
    case "goal":
      return `${p}/charter#${r.id}`;
    case "adr":
      return `${p}/docs/file/${index.adrs.get(r.id) ?? r.id}`;
    case "doc":
      return `${p}/docs/file/${r.path}`;
  }
}

/** A broken explicit link is always worth flagging; prose only when it has this board's shape. */
export function isBroken(index: BoardIndex, found: Found): boolean {
  return !resolves(index, found.reference) && (found.explicit || found.reference.kind !== "doc");
}

/** A reference as a person writes it: `FEAT-012/R-1`, `ADR-0005`, `architecture/data.md`. */
export function label(r: Ref): string {
  switch (r.kind) {
    case "item":
      return r.requirement ? `${r.code}/${r.requirement}` : r.code;
    case "milestone":
      return r.code;
    case "adr":
    case "goal":
      return r.id;
    case "doc":
      return r.path;
  }
}

/** A doc path written in prose or a relative link, as the board stores it. */
export function docKey(path: string): string {
  let p = path.trim();
  while (p.startsWith("./")) p = p.slice(2);
  while (p.startsWith("/")) p = p.slice(1);
  if (p.startsWith("docs/")) p = p.slice(5);
  return p.endsWith(".md") ? p : `${p}.md`;
}

const isAlnum = (c: string | undefined) => !!c && /[\p{L}\p{N}]/u.test(c);
const isUpper = (c: string | undefined) => !!c && c >= "A" && c <= "Z";
const isDigit = (c: string | undefined) => !!c && c >= "0" && c <= "9";

/** Every reference in `markdown`, in order. */
export function find(markdown: string, index: BoardIndex): Found[] {
  const out: Found[] = [];
  let fence: string | null = null;
  let inComment = false;
  const lines = markdown.split("\n");
  if (lines.length && lines[lines.length - 1] === "") lines.pop();
  lines.forEach((raw, n) => {
    const lineNo = n + 1;
    const trimmed = raw.trimStart();
    if (fence) {
      if (trimmed.startsWith(fence)) fence = null;
      return;
    }
    if (trimmed.startsWith("```") || trimmed.startsWith("~~~")) {
      fence = trimmed.slice(0, 3);
      return;
    }
    let line = raw.replace(/\r$/, "");
    if (inComment) {
      const end = line.indexOf("-->");
      if (end < 0) return;
      line = line.slice(end + 3);
      inComment = false;
    }
    for (;;) {
      const start = line.indexOf("<!--");
      if (start < 0) break;
      const end = line.indexOf("-->", start);
      if (end >= 0) {
        line = line.slice(0, start) + " " + line.slice(end + 3);
      } else {
        line = line.slice(0, start);
        inComment = true;
      }
    }
    scanLine(line, lineNo, index, out);
  });
  return out;
}

function scanLine(line: string, lineNo: number, index: BoardIndex, out: Found[]) {
  const chars = Array.from(line);
  let i = 0;
  let text = "";
  const flush = () => {
    scanProse(text, lineNo, index, out);
    text = "";
  };
  while (i < chars.length) {
    const c = chars[i];
    if (c === "`") {
      flush();
      let ticks = 0;
      while (chars[i + ticks] === "`") ticks++;
      const rest = chars.slice(i + ticks).join("");
      const end = rest.indexOf("`".repeat(ticks));
      i = end >= 0 ? i + ticks + Array.from(rest.slice(0, end)).length + ticks : chars.length;
    } else if (c === "[") {
      const link = linkAt(chars.slice(i).join(""));
      if (link) {
        flush();
        const r = targetRef(link.target, index);
        if (r) out.push({ reference: r, line: lineNo, explicit: true });
        i += link.length;
      } else {
        text += "[";
        i++;
      }
    } else {
      text += c;
      i++;
    }
  }
  flush();
}

/** `[text](target)` at the start of `s`: the target and the link's length in code points. */
function linkAt(s: string): { target: string; length: number } | null {
  const close = s.indexOf("](");
  if (close < 0) return null;
  const inner = s.slice(1, close);
  if (inner.includes("[") && !inner.includes("]")) return null;
  const after = s.slice(close + 2);
  const end = after.indexOf(")");
  if (end < 0) return null;
  const target = after.slice(0, end).trim().split(/\s+/)[0] ?? "";
  return { target, length: Array.from(s.slice(0, close + 2 + end + 1)).length };
}

/** What a link target points at on this board, if anything. */
export function targetRef(target: string, index: BoardIndex): Ref | null {
  if (target.includes("://") || target.startsWith("mailto:") || target.startsWith("#")) return null;
  const prefix = `/p/${index.project}/`;
  if (target.startsWith(prefix)) {
    const rest = target.slice(prefix.length);
    const hash = rest.indexOf("#");
    const path = hash >= 0 ? rest.slice(0, hash) : rest;
    const anchor = hash >= 0 ? rest.slice(hash + 1) : undefined;
    if (path.startsWith("feature/")) {
      const code = path.slice(8).replace(/\/+$/, "");
      return anchor !== undefined
        ? { kind: "item", code, requirement: anchor }
        : { kind: "item", code };
    }
    if (path.startsWith("milestone/")) {
      return { kind: "milestone", code: path.slice(10).replace(/\/+$/, "") };
    }
    if (path === "charter") return anchor !== undefined ? { kind: "goal", id: anchor } : null;
    if (path.startsWith("docs/file/")) return { kind: "doc", path: path.slice(10) };
    return null;
  }
  if (target.startsWith("/p/")) return null;
  const path = target.split("#")[0];
  return path.endsWith(".md") ? { kind: "doc", path: docKey(path) } : null;
}

/** A run of prose, cut into plain text and the references in it (what the renderer links). */
export type Piece = string | { ref: Ref; text: string };

export function splitProse(text: string, index: BoardIndex): Piece[] {
  const pieces: Piece[] = [];
  let last = 0;
  scanProseAt(text, index, (start, end, ref) => {
    if (start > last) pieces.push(text.slice(last, start));
    pieces.push({ ref, text: text.slice(start, end) });
    last = end;
  });
  if (last < text.length) pieces.push(text.slice(last));
  return pieces;
}

function scanProse(text: string, lineNo: number, index: BoardIndex, out: Found[]) {
  scanProseAt(text, index, (_s, _e, reference) => out.push({ reference, line: lineNo, explicit: false }));
}

/**
 * Board-shaped tokens in prose. `emit` gets UTF-16 offsets into `text`, so a caller can cut the
 * string; the scan itself walks code points, as the Rust one walks chars.
 */
function scanProseAt(text: string, index: BoardIndex, emit: (start: number, end: number, r: Ref) => void) {
  const chars = Array.from(text);
  // UTF-16 offset of each code point, plus the end.
  const at: number[] = [];
  let off = 0;
  for (const c of chars) {
    at.push(off);
    off += c.length;
  }
  at.push(off);
  const boundary = (i: number) => i === 0 || !(isAlnum(chars[i - 1]) || chars[i - 1] === "_");
  let i = 0;
  while (i < chars.length) {
    if (boundary(i) && (isAlnum(chars[i]) || chars[i] === "." || chars[i] === "/")) {
      let j = i;
      while (j < chars.length && (isAlnum(chars[j]) || "/-_.".includes(chars[j]))) j++;
      let word = chars.slice(i, j).join("");
      let wordEnd = j;
      while (word.endsWith(".")) {
        word = word.slice(0, -1);
        wordEnd--;
      }
      if (word.endsWith(".md") && word.length > 3) {
        const path = docKey(word);
        if (index.docs.has(path)) emit(at[i], at[wordEnd], { kind: "doc", path });
        i = j;
        continue;
      }
    }
    if (boundary(i) && isUpper(chars[i])) {
      let j = i;
      while (j < chars.length && (isUpper(chars[j]) || isDigit(chars[j]))) j++;
      if (chars[j] === "-") {
        let k = j + 1;
        while (k < chars.length && isDigit(chars[k])) k++;
        const ends = k === chars.length || !(isAlnum(chars[k]) || chars[k] === "_");
        if (k > j + 1 && ends) {
          const prefix = chars.slice(i, j).join("");
          const code = chars.slice(i, k).join("");
          let end = k;
          let requirement: string | undefined;
          const rest = chars.slice(k, k + 8).join("");
          if (rest.startsWith("/R-")) {
            const digits = /^\d+/.exec(rest.slice(3))?.[0] ?? "";
            if (digits) {
              requirement = `R-${digits}`;
              end = k + 3 + digits.length;
            }
          }
          let reference: Ref | null = null;
          if (prefix === "ADR") reference = { kind: "adr", id: code };
          else if (prefix === "G" && index.goals.size > 0) reference = { kind: "goal", id: code };
          else if (index.items.has(code) || index.itemPrefixes.has(prefix))
            reference = requirement ? { kind: "item", code, requirement } : { kind: "item", code };
          else if (index.milestones.has(code) || index.milestonePrefixes.has(prefix))
            reference = { kind: "milestone", code };
          // A requirement suffix only belongs to the linked text when it was read as an item.
          if (reference) emit(at[i], at[reference.kind === "item" ? end : k], reference);
          i = end;
          continue;
        }
      }
      i = Math.max(j, i + 1);
      continue;
    }
    i++;
  }
}

/** A run of raw (unescaped) prose as HTML, its references wrapped in links. */
export function linkProseHtml(text: string, index: BoardIndex, escape: (s: string) => string): string {
  return splitProse(text, index)
    .map((piece) => {
      if (typeof piece === "string") return escape(piece);
      const ok = resolves(index, piece.ref);
      if (!ok && piece.ref.kind === "doc") return escape(piece.text);
      const target = hrefOf(index.project, piece.ref, index);
      return ok
        ? `<a class="board-ref" href="${escape(target)}">${escape(piece.text)}</a>`
        : `<a class="board-ref broken" href="${escape(target)}" title="not on this board">${escape(piece.text)}</a>`;
    })
    .join("");
}
