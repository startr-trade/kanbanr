import { useEffect, useMemo, useRef, type MouseEvent } from "react";
import { useNavigate, useParams } from "react-router-dom";
import DOMPurify from "dompurify";
import { markdownToSafeHtml, safeSvg } from "../markdownHtml";
import { locateError } from "../diagramCheck";
import { appHref } from "../runtimeConfig";
import { useBoardIndex } from "../useBoardIndex";

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
 */
export default function Markdown({
  source,
  resolveImage,
}: {
  source: string;
  resolveImage?: (src: string) => string;
}) {
  const { project } = useParams();
  const index = useBoardIndex(project);
  const navigate = useNavigate();
  // A board link written as a path (`/p/x/feature/Y`) is an in-app route. Under hash routing — the
  // standalone bundle on a plain static host (FEAT-178) — it must become `#/p/…`, or it would ask
  // the host for a file that is not there; the hash router then follows it.
  const html = useMemo(
    () =>
      markdownToSafeHtml(source, resolveImage, DOMPurify, index).replace(
        /(<a\b[^>]*?\bhref=")(\/(?:p\/|portfolio\b)[^"]*)(")/g,
        (_m, pre, path, post) => pre + appHref(path) + post,
      ),
    [source, resolveImage, index],
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
  }, [html]);

  return <div className="markdown" ref={ref} onClick={follow} dangerouslySetInnerHTML={{ __html: html }} />;
}
