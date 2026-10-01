import { marked } from "marked";
import DOMPurify from "dompurify";

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
 */
export function markdownToSafeHtml(
  source: string,
  resolveImage?: (src: string) => string,
  purifier: typeof DOMPurify = DOMPurify,
): string {
  let out = marked.parse(source, { async: false }) as string;
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
