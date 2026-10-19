import { Marked, Renderer, marked, type Token } from "marked";
import DOMPurify from "dompurify";
import { type BoardIndex, linkProseHtml, resolves, href as refHref, targetRef } from "./boardRefs.ts";

const escapeHtml = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");

/** A Marked instance per board index, so linking never touches the global `marked` (FEAT-181). */
const linkers = new WeakMap<BoardIndex, Marked>();

function linkerFor(index: BoardIndex): Marked {
  const cached = linkers.get(index);
  if (cached) return cached;
  const inLink = new WeakSet<object>();
  const linker = new Marked({
    // Prose only: text tokens outside links. Code spans, fences and HTML are other token types,
    // so they are never touched.
    walkTokens(token: Token) {
      if (token.type === "link") {
        for (const child of token.tokens ?? []) markInLink(child, inLink);
        return;
      }
      if (token.type !== "text" || inLink.has(token) || ("tokens" in token && token.tokens?.length)) return;
      token.text = linkProseHtml(token.raw, index, escapeHtml);
    },
    renderer: {
      // An explicit link to the board is classed, and a relative doc link goes to the doc's page.
      link(this: Renderer, href: string, title: string | null | undefined, text: string) {
        const ref = targetRef(href, index);
        if (!ref) return Renderer.prototype.link.call(this, href, title, text);
        const ok = resolves(index, ref);
        const html = Renderer.prototype.link.call(this, ok ? (refHref(index, ref) ?? href) : href, title, text);
        return html.replace(
          /^<a /,
          ok ? '<a class="board-ref" ' : '<a class="board-ref broken" title="not on this board" ',
        );
      },
    },
  });
  linkers.set(index, linker);
  return linker;
}

function markInLink(token: Token, seen: WeakSet<object>) {
  seen.add(token);
  for (const child of ("tokens" in token && token.tokens) || []) markInLink(child, seen);
}

/**
 * Markdown to HTML that is safe to put in the page (FEAT-140).
 *
 * A board is shared through a git remote, so its documents and specs are written by everyone who
 * can push to it — not only by whoever is looking. Rendered markdown used to go into the page as
 * it came out of `marked`, so a `<script>`, an `onerror=` or a `javascript:` link in a pushed
 * document ran in the viewer's browser, on the monitor's own origin, where `--allow-writes` lets
 * the page approve, ratify and sign off. Everything that leaves here has been through DOMPurify.
 *
 * `purifier` is the browser's DOMPurify by default; the check script passes one bound to a
 * jsdom window, so the same function is what gets tested.
 *
 * With a board `index`, references to the board — `FEAT-012`, `FEAT-012/R-2`, `ADR-0005`, `G-1`,
 * `MS-004`, a doc path — become links to their pages, and one that points at nothing is classed
 * `broken` (FEAT-181). They are linked before sanitising, so they pass through DOMPurify too.
 */
export function markdownToSafeHtml(
  source: string,
  resolveImage?: (src: string) => string,
  purifier: typeof DOMPurify = DOMPurify,
  index?: BoardIndex | null,
): string {
  let out = (index ? linkerFor(index) : marked).parse(source, { async: false }) as string;
  if (resolveImage) {
    out = out.replace(/(<img\b[^>]*?\bsrc=")([^"]+)(")/g, (m, pre, src, post) =>
      /^(https?:|data:|\/)/i.test(src) ? m : pre + resolveImage(src) + post,
    );
  }
  return purifier.sanitize(out, { USE_PROFILES: { html: true } });
}

/**
 * A rendered Mermaid diagram, made safe to insert. Mermaid sanitises its own labels in strict
 * mode; this does not rely on that. Flowchart labels are HTML inside `<foreignObject>`, which
 * DOMPurify drops unless it is allowed as an integration point — every label would vanish.
 */
export function safeSvg(svg: string, purifier: typeof DOMPurify = DOMPurify): string {
  return purifier.sanitize(svg, {
    ADD_TAGS: ["foreignObject"],
    HTML_INTEGRATION_POINTS: { foreignobject: true },
  });
}
