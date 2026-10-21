import type { ItemDiagrams } from "./types";

/**
 * The review page's diagram check (FEAT-200), kept pure so `check:diagram-review` runs it in node:
 * every block of an item's spec and of each doc in its `affects_docs` is parsed with Pele, the
 * library the monitor and the book draw with, and Approve stays disabled until all of them parse.
 * The core's lint follows Pele, so its findings are a subset of Pele's errors; one Pele did not
 * raise is listed anyway, so the page fails closed rather than open.
 */

/** One diagram that does not parse, located in its file. */
export type DiagramProblem = {
  file: string;
  /** The doc key, for a link to it; null for a spec. */
  doc: string | null;
  block: number;
  /** The file line the error is on. */
  line: number;
  message: string;
};

/**
 * Where a Pele error is, as a file line, and what it says in one line. Pele numbers lines from 1
 * within the block and uses 0 when it found no diagram type at all; `firstLine` is the file line of
 * the block's first source line.
 */
export function locateError(e: unknown, firstLine: number): { line: number; message: string } {
  const err = (typeof e === "object" && e !== null ? e : {}) as {
    line?: unknown;
    message?: unknown;
    code?: unknown;
  };
  const within = typeof err.line === "number" && err.line > 0 ? err.line : 0;
  const line = within > 0 ? firstLine + within - 1 : firstLine;
  if (err.code === "unsupported-diagram") return { line, message: "no diagram type on the first line" };
  const text = typeof err.message === "string" ? err.message : String(e);
  // Pele's syntax errors open with "Parse error on line N:" and the source snippet; the line that
  // says what went wrong ("Expecting …, got …") is the last. The line is already given above.
  const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
  const said = lines.filter((l) => !/^Parse error on line \d+:?$/.test(l) && !/^-*\^$/.test(l));
  const message = (err.code === "syntax" ? said[said.length - 1] : said[0]) ?? "";
  return { line, message: message || "the diagram does not parse" };
}

/**
 * Parse every block with `parse` (Pele's), and list each that throws; then add any finding of the
 * core lint on a block Pele accepted. One problem per (file, block).
 */
export function checkBlocks(d: ItemDiagrams, parse: (source: string) => unknown): DiagramProblem[] {
  const problems: DiagramProblem[] = [];
  const seen = new Set<string>();
  const key = (file: string, block: number) => `${file}\u0000${block}`;
  for (const b of d.blocks) {
    try {
      parse(b.source);
    } catch (e) {
      seen.add(key(b.file, b.block));
      // Pele says where; the core lint, when it caught the same block, says why in plain words.
      const plain = d.findings.find((f) => f.file === b.file && f.block === b.block);
      const at = locateError(e, b.line);
      problems.push({
        file: b.file,
        doc: b.doc,
        block: b.block,
        line: at.line,
        message: plain ? `${plain.message} (${plain.rule})` : at.message,
      });
    }
  }
  for (const f of d.findings) {
    const k = key(f.file, f.block);
    if (seen.has(k)) continue;
    seen.add(k);
    const doc = d.blocks.find((b) => b.file === f.file && b.block === f.block)?.doc ?? null;
    problems.push({ file: f.file, doc, block: f.block, line: f.line, message: `${f.message} (${f.rule})` });
  }
  return problems;
}

/**
 * Why Approve (or Ratify) is disabled, or null when it may be offered. Fails closed: while the
 * check is loading, or when it could not run, the button stays disabled and says why.
 */
export function approveBlocker(state: {
  loading: boolean;
  failed: string | null;
  problems: DiagramProblem[];
}): string | null {
  if (state.failed) return `The diagram check could not run (${state.failed}) — reload the page`;
  if (state.loading) return "Checking the diagrams…";
  const n = state.problems.length;
  if (n > 0) return `${n} diagram${n === 1 ? " does" : "s do"} not parse — fix ${n === 1 ? "it" : "them"} first`;
  return null;
}
