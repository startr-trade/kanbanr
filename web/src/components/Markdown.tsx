import { useEffect, useMemo, useRef } from "react";
import { marked } from "marked";
import mermaid from "mermaid";

// Unique-per-render id source for mermaid (ids must be stable strings, unique across renders).
let mermaidSeq = 0;

/**
 * Render trusted (locally-authored) markdown to HTML.
 *
 * - `resolveImage`, when given, rewrites relative `<img>` sources (e.g. `diagram.png`) to an
 *   absolute URL. `DocPage` passes a resolver scoped to **each document's own folder**, so a
 *   relative image name is meaningful in *any* docs folder (it resolves to `<that-folder>/<name>`),
 *   served by the view daemon's `/docs/raw` endpoint.
 * - Fenced ```mermaid blocks are rendered to SVG diagrams (flowcharts, sequence, etc.).
 */
export default function Markdown({
  source,
  resolveImage,
}: {
  source: string;
  resolveImage?: (src: string) => string;
}) {
  const html = useMemo(() => {
    let out = marked.parse(source, { async: false }) as string;
    if (resolveImage) {
      out = out.replace(/(<img\b[^>]*?\bsrc=")([^"]+)(")/g, (m, pre, src, post) =>
        /^(https?:|data:|\/)/i.test(src) ? m : pre + resolveImage(src) + post
      );
    }
    return out;
  }, [source, resolveImage]);

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
          wrap.innerHTML = svg;
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

  return <div className="markdown" ref={ref} dangerouslySetInnerHTML={{ __html: html }} />;
}
