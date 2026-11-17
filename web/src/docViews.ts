// A doc's `katalogr-query` blocks, drawn from what the daemon asked of katalogr (FTR-016). Pure:
// no React and no network, so `scripts/check-katalogr.mjs` runs it in node with jsdom, the way
// `check-markdown.mjs` runs the markdown renderer. Everything shown is text or a Pele diagram that
// went through `safeSvg`; nothing here sets markup from a string except that sanitised SVG.
import type { DocViewBlock, DocViews, PositionRef } from "./types";

/** The query's operation: the first word of its one line. */
function opOf(block: DocViewBlock): string {
  return block.source.trim().split(/\s+/)[0] ?? "";
}

/** One position as a person reads it: the short commit and whether the checkout was clean. */
function positionText(p: PositionRef, withRepo: boolean): string {
  const at = p.commit ? p.commit.slice(0, 7) : "no commit";
  const shown = `${at} (${p.dirty ? "dirty" : "clean"})`;
  return withRepo ? `${p.repo} ${shown}` : shown;
}

/** A view ID shortened to its start and end: `v1-0123456789abcdef…` reads `v1-0123…cdef`. */
export function shortViewId(id: string): string {
  return id.length > 12 ? `${id.slice(0, 7)}…${id.slice(-4)}` : id;
}

/**
 * The position line under a drawn block: "katalogr view structure · at 0123456 (clean) · exact ·
 * view v1-0123…cdef", or "… · not exact" when katalogr is still catching up and gives no ID.
 */
export function caption(block: DocViewBlock, _doc?: DocViews): string {
  const view = block.view;
  const head = `katalogr view ${opOf(block)}`;
  if (!view) return head;
  const many = view.positions.length > 1;
  const at = view.positions.length ? `at ${view.positions.map((p) => positionText(p, many)).join(", ")}` : "";
  const state = view.exact ? "exact" : "not exact";
  const id = view.exact && view.view_id ? `view ${shortViewId(view.view_id)}` : "";
  return [head, at, state, id].filter(Boolean).join(" · ");
}

/** The one plain sentence under a block that has no view. Never empty for such a block. */
export function saidFor(views: DocViews, block: DocViewBlock): string {
  if (block.said) return block.said;
  if (block.problem) return `this block is not a query: ${block.problem}`;
  const k = views.katalogr;
  if (k.state === "absent") return `${k.said} — the block shows its source`;
  if (k.state === "too_old") return k.said;
  return "katalogr gave no view for this block";
}

export interface Pair<N> {
  node: N;
  block: DocViewBlock | undefined;
}

/** The page's query blocks, in document order, each with the daemon's block of the same index. */
export function pairBlocks<N>(nodes: N[], views: DocViews): Pair<N>[] {
  return nodes.map((node, i) => ({ node, block: views.blocks.find((b) => b.index === i) }));
}

/** The text a block shows while the daemon is being asked; the source is never hidden. */
export const ASKING = "asking katalogr…";

/** A line of plain text under a block's source (`.katalogr-said`); text only, never markup. */
export function saidElement(doc: Document, text: string): HTMLElement {
  const el = doc.createElement("div");
  el.className = "katalogr-said";
  el.textContent = text;
  return el;
}

/** `open` is katalogr's own page; only an http(s) address is ever made a link. */
function safeOpen(open: string | undefined): string | null {
  return open && /^https?:\/\//i.test(open) ? open : null;
}

/**
 * The drawn block as one figure (R-13): a `<section>` of the UI kit's Section class with an
 * `aria-label` and no heading, holding in order the diagram, the position line and the one-line
 * source. `svg` is Pele's output after `safeSvg`.
 */
export function figureElement(doc: Document, block: DocViewBlock, views: DocViews, svg: string): HTMLElement {
  const section = doc.createElement("section");
  section.className = "ui-section katalogr-view";
  section.setAttribute("aria-label", `katalogr view ${opOf(block)}`);

  const drawing = doc.createElement("div");
  drawing.className = "katalogr-drawing";
  drawing.innerHTML = svg;

  const position = doc.createElement("div");
  position.className = "katalogr-position";
  const line = doc.createElement("span");
  line.textContent = caption(block, views);
  position.append(line);
  const open = safeOpen(block.view?.open);
  if (open) {
    const a = doc.createElement("a");
    a.href = open;
    a.target = "_blank";
    a.rel = "noopener noreferrer";
    a.textContent = "open in katalogr";
    position.append(" · ", a);
  }
  for (const w of block.view?.warnings ?? []) {
    const warn = doc.createElement("div");
    warn.className = "katalogr-warning";
    warn.textContent = w;
    position.append(warn);
  }

  const source = doc.createElement("div");
  source.className = "katalogr-source";
  const code = doc.createElement("code");
  code.textContent = block.source.trim();
  source.append(code);

  section.append(drawing, position, source);
  return section;
}

/** The daemon could not be asked at all: the same sentence for every block. */
export const UNREACHED = "the monitor's daemon could not ask katalogr — the block shows its source";

/** What `webViewFor` finds for one block: its figure inputs, or the sentence that stands in. */
export type WebView = { kind: "draw"; block: DocViewBlock } | { kind: "say"; text: string };

/** Decide, for one pair, whether to draw the diagram or say why not (R-5, R-6). */
export function webViewFor(views: DocViews | null, pair: Pair<unknown>): WebView {
  if (!views) return { kind: "say", text: UNREACHED };
  if (!pair.block) return { kind: "say", text: "the daemon had no answer for this block" };
  if (pair.block.view) return { kind: "draw", block: pair.block };
  return { kind: "say", text: saidFor(views, pair.block) };
}
