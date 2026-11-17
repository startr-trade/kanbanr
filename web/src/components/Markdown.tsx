import { useEffect, useMemo, useRef, type MouseEvent } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import DOMPurify from "dompurify";
import { markdownToSafeHtml, safeSvg } from "../markdownHtml";
import { locateError } from "../diagramCheck";
import { appHref } from "../runtimeConfig";
import { useBoardIndex } from "../useBoardIndex";
import { currentPick, rewriteRepoLinks } from "../position";
import type { DocViews } from "../types";
import { ASKING, figureElement, pairBlocks, saidElement, webViewFor } from "../docViews";

// Unique-per-diagram id prefix, so two diagrams on one page never share an SVG id.
let diagramSeq = 0;

/**
 * Render a board's markdown to HTML. Not trusted: a shared board's documents are written by
 * whoever can push to it, so the HTML is sanitised before it reaches the page (FEAT-140).
 *
 * - `resolveImage`, when given, rewrites relative `<img>` sources (e.g. `diagram.png`) to an
 *   absolute URL. `DocPage` passes a resolver scoped to **each document's own folder**, so a
 *   relative image name is meaningful in *any* docs folder (it resolves to `<that-folder>/<name>`),
 *   served by the view daemon's `/docs/raw` endpoint.
 * - Fenced ```mermaid blocks are drawn as SVG by Pele (FEAT-203), loaded only when a page has one,
 *   and sanitised like the rest; their colours come from the monitor's theme (`styles.css`).
 * - References to the board (`FEAT-012`, `ADR-0005`, a doc path…) link to their pages, and a link
 *   into the monitor navigates in place rather than reloading it (FEAT-181).
 * - A relative link under `docs/` (`docs/guide/using/x.md`) is a repository doc: it opens the doc's
 *   page at the position, which is `at` (an item page passes its own code) or else the pick in the
 *   URL or kept for the session (FTR-004). A link to a board doc (`/p/…/docs/file/…`) is absolute and
 *   untouched.
 * - Fenced ```katalogr-query blocks are asked of katalogr through `resolveViews` (FTR-016): each is
 *   drawn as one figure when the daemon has a view, and otherwise keeps its source with one plain
 *   sentence under it. The page never blanks a block, and nothing here needs katalogr.
 * - Headings are shifted down by `headingOffset` (default 1), so the page's own title stays its
 *   only h1 (DEF-082). `DocPage` passes 0 when the doc starts with an h1 and the page draws none.
 */
export default function Markdown({
  source,
  resolveImage,
  headingOffset = 1,
  at,
  resolveViews,
}: {
  source: string;
  resolveImage?: (src: string) => string;
  headingOffset?: 0 | 1 | 2;
  /** The position repository doc links open at; default is the current pick (FTR-004). */
  at?: string;
  /** Asks the daemon for this doc's query blocks (`api.getDocViews`); without it blocks stay source. */
  resolveViews?: () => Promise<DocViews>;
}) {
  const { project } = useParams();
  const { search } = useLocation();
  const pick = at ?? currentPick(search);
  const index = useBoardIndex(project);
  const navigate = useNavigate();
  // A board link written as a path (`/p/x/feature/Y`) is an in-app route. Under hash routing — the
  // standalone bundle on a plain static host (FEAT-178) — it must become `#/p/…`, or it would ask
  // the host for a file that is not there; the hash router then follows it.
  const html = useMemo(
    () =>
      rewriteRepoLinks(
        markdownToSafeHtml(source, resolveImage, DOMPurify, index, headingOffset),
        project ?? "",
        pick,
      ).replace(/(<a\b[^>]*?\bhref=")(\/(?:p\/|portfolio\b)[^"]*)(")/g, (_m, pre, path, post) => pre + appHref(path) + post),
    [source, resolveImage, index, headingOffset, project, pick],
  );
  const follow = (e: MouseEvent<HTMLDivElement>) => {
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const a = (e.target as HTMLElement).closest?.("a");
    const target = a?.getAttribute("href");
    if (!target || !target.startsWith("/p/")) return;
    e.preventDefault();
    navigate(target);
  };

  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const root = ref.current;
    if (!root) return;

    // Draw any ```mermaid fenced blocks as inline SVG diagrams.
    const mmdBlocks = Array.from(root.querySelectorAll<HTMLElement>("code.language-mermaid"));
    if (mmdBlocks.length) {
      import("pele").then(({ render }) => {
        for (const code of mmdBlocks) {
          const host = code.closest("pre") ?? code;
          if (!host.isConnected) continue;
          const src = code.textContent ?? "";
          const wrap = document.createElement("div");
          wrap.className = "mermaid-diagram";
          try {
            const { svg } = render(src, { idPrefix: `d${diagramSeq++}-` });
            wrap.innerHTML = safeSvg(svg);
            host.replaceWith(wrap);
          } catch (e) {
            // A diagram that does not draw keeps its source, with the error and its line under it
            // (FEAT-200): "Syntax error" alone said neither what nor where. Text only, never HTML.
            const { line, message } = locateError(e, 1);
            const source = document.createElement("pre");
            source.className = "diagram-source";
            source.textContent = src;
            const error = document.createElement("div");
            error.className = "diagram-error";
            error.textContent = `Diagram error, line ${line}: ${message}`;
            const failed = document.createElement("div");
            failed.className = "diagram-failed";
            failed.append(source, error);
            host.replaceWith(failed);
          }
        }
      });
    }

    // Draw any ```katalogr-query blocks from what the daemon asked of katalogr (FTR-016).
    const queryBlocks = Array.from(root.querySelectorAll<HTMLElement>("code.language-katalogr-query"));
    let cancelled = false;
    let waiting: HTMLElement[] = [];
    if (queryBlocks.length && resolveViews) {
      const hosts = queryBlocks.map((code) => code.closest("pre") ?? code);
      waiting = hosts.map((host) => {
        const note = saidElement(document, ASKING);
        host.after(note);
        return note;
      });
      Promise.all([resolveViews().then((v) => v, () => null), import("pele")]).then(([views, { render }]) => {
        if (cancelled) return;
        waiting.forEach((n) => n.remove());
        pairBlocks(hosts, views ?? { doc: "", position: "", position_from: "", katalogr: { state: "absent", url: "", said: "" }, blocks: [] }).forEach((pair) => {
          const host = pair.node;
          if (!host.isConnected) return;
          const what = webViewFor(views, pair);
          if (what.kind === "say") {
            host.after(saidElement(document, what.text));
            return;
          }
          try {
            const { svg } = render(what.block.view!.mermaid, { idPrefix: `k${diagramSeq++}-` });
            host.replaceWith(figureElement(document, what.block, views!, safeSvg(svg)));
          } catch (e) {
            const { line, message } = locateError(e, 1);
            host.after(saidElement(document, `Diagram error, line ${line}: ${message}`));
          }
        });
      });
    }

    // Add a small "Copy" button to ordinary code blocks (markdown nicety; skip mermaid blocks).
    root.querySelectorAll<HTMLPreElement>("pre").forEach((pre) => {
      if (pre.querySelector("code.language-mermaid")) return;
      if (pre.querySelector(".code-copy")) return;
      const btn = document.createElement("button");
      btn.className = "code-copy";
      btn.type = "button";
      btn.textContent = "Copy";
      btn.onclick = () => {
        const code = pre.querySelector("code")?.textContent ?? pre.textContent ?? "";
        navigator.clipboard?.writeText(code).then(() => {
          btn.textContent = "Copied";
          setTimeout(() => (btn.textContent = "Copy"), 1200);
        });
      };
      pre.appendChild(btn);
    });
    return () => {
      cancelled = true;
      waiting.forEach((n) => n.remove());
    };
  }, [html, resolveViews]);

  return <div className="markdown" ref={ref} onClick={follow} dangerouslySetInnerHTML={{ __html: html }} />;
}
