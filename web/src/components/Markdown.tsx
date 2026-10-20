import { useEffect, useMemo, useRef, type MouseEvent } from "react";
import { useNavigate, useParams } from "react-router-dom";
import mermaid from "mermaid";
import DOMPurify from "dompurify";
import { markdownToSafeHtml, safeSvg } from "../markdownHtml";
import { appHref } from "../runtimeConfig";
import { useBoardIndex } from "../useBoardIndex";

// Unique-per-render id source for mermaid (ids must be stable strings, unique across renders).
let mermaidSeq = 0;

/**
 * Render a board's markdown to HTML. Not trusted: a shared board's documents are written by
 * whoever can push to it, so the HTML is sanitised before it reaches the page (FEAT-140).
 *
 * - `resolveImage`, when given, rewrites relative `<img>` sources (e.g. `diagram.png`) to an
 *   absolute URL. `DocPage` passes a resolver scoped to **each document's own folder**, so a
 *   relative image name is meaningful in *any* docs folder (it resolves to `<that-folder>/<name>`),
 *   served by the view daemon's `/docs/raw` endpoint.
 * - Fenced ```mermaid blocks are rendered to SVG diagrams (flowcharts, sequence, etc.).
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

    // Render any ```mermaid fenced blocks into inline SVG diagrams.
    const mmdBlocks = Array.from(root.querySelectorAll<HTMLElement>("code.language-mermaid"));
    if (mmdBlocks.length) {
      const dark = document.documentElement.dataset.theme !== "light";
      mermaid.initialize({ startOnLoad: false, theme: dark ? "dark" : "default", securityLevel: "strict" });
      mmdBlocks.forEach(async (code) => {
        const host = code.closest("pre") ?? code;
        const src = code.textContent ?? "";
        try {
          const { svg } = await mermaid.render(`mmd-${mermaidSeq++}`, src);
          const wrap = document.createElement("div");
          wrap.className = "mermaid-diagram";
          wrap.innerHTML = safeSvg(svg);
          host.replaceWith(wrap);
        } catch (e) {
          const note = document.createElement("div");
          note.className = "mermaid-diagram";
          note.textContent = "mermaid error: " + (e instanceof Error ? e.message : String(e));
          host.replaceWith(note);
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
