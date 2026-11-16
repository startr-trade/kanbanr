/**
 * WCAG 2.1 contrast for the monitor's design tokens (DEF-082).
 *
 * Pure: no DOM, no React. `parseTokens` reads `tokens.css` as text, so the check script and the
 * UI kit's palette check measure exactly the values the page is styled with.
 */

/** Expand `#rgb` to `#rrggbb` and lower-case it; anything else is returned trimmed. */
function normalizeHex(hex: string): string {
  const h = hex.trim().toLowerCase();
  const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/.exec(h);
  return short ? `#${short[1]}${short[1]}${short[2]}${short[2]}${short[3]}${short[3]}` : h;
}

function channels(hex: string): [number, number, number] {
  const h = normalizeHex(hex);
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/.exec(h);
  if (!m) throw new Error(`not a #rgb or #rrggbb colour: ${hex}`);
  return [parseInt(m[1], 16), parseInt(m[2], 16), parseInt(m[3], 16)];
}

const toHex = (rgb: number[]) => `#${rgb.map((c) => Math.round(c).toString(16).padStart(2, "0")).join("")}`;

/** WCAG 2.1 relative luminance of `#rgb` or `#rrggbb`. */
export function luminance(hex: string): number {
  const [r, g, b] = channels(hex).map((c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** `(L1 + 0.05) / (L2 + 0.05)`, L1 the lighter of the two. */
export function ratio(fg: string, bg: string): number {
  const a = luminance(fg);
  const b = luminance(bg);
  const [hi, lo] = a >= b ? [a, b] : [b, a];
  return (hi + 0.05) / (lo + 0.05);
}

/** `fg` at `weight` (0–1) over `bg`, per sRGB channel: a pill background composited on its surface. */
export function mix(fg: string, bg: string, weight: number): string {
  const f = channels(fg);
  const b = channels(bg);
  return toHex(f.map((c, i) => c * weight + b[i] * (1 - weight)));
}

/**
 * Every token pair the monitor draws text or a boundary with: fg on bg, and the bar it must meet.
 * PAIRS lists only pairs the CSS draws; a pair nothing draws is not held to a bar.
 */
export interface Pair {
  fg: string;
  bg: string;
  min: 4.5 | 3;
  where: string;
}

const TEXT_SURFACES = ["--bg", "--surface", "--surface-2", "--control"];
const TONES = ["ok", "warn", "error", "info"];

export const PAIRS: Pair[] = [
  ...["--text", "--text-muted", "--text-subtle"].flatMap((fg) =>
    TEXT_SURFACES.map((bg): Pair => ({ fg, bg, min: 4.5, where: `text in ${fg} on ${bg}` })),
  ),
  ...["--bg", "--surface", "--surface-2"].map((bg): Pair => ({ fg: "--accent", bg, min: 4.5, where: `a link on ${bg}` })),
  { fg: "--on-accent", bg: "--accent", min: 4.5, where: "text on an accent-filled control" },
  { fg: "--on-accent-strong", bg: "--accent-strong", min: 4.5, where: "the active shell tab" },
  ...TONES.flatMap((tone) =>
    ["--surface", "--surface-2", `--${tone}-bg`].map(
      (bg): Pair => ({ fg: `--${tone}-fg`, bg, min: 4.5, where: `${tone} text on ${bg}` }),
    ),
  ),
  // `--border` is a decorative separator; a control's edge, which a person needs to find it, is
  // `--border-strong` (WCAG 1.4.11).
  { fg: "--border-strong", bg: "--surface", min: 3, where: "a control's edge on a surface" },
  { fg: "--border-strong", bg: "--bg", min: 3, where: "a control's edge on the page" },
  { fg: "--accent", bg: "--bg", min: 3, where: "the focus ring on the page" },
  { fg: "--accent", bg: "--surface", min: 3, where: "the focus ring on a surface" },
];

type Tokens = Record<string, string>;

/** Custom properties per selector block, comments stripped. */
function blocks(css: string): { selectors: string[]; props: [string, string][] }[] {
  const text = css.replace(/\/\*[\s\S]*?\*\//g, "");
  return [...text.matchAll(/([^{}]+)\{([^}]*)\}/g)].map((m) => ({
    selectors: m[1].split(",").map((s) => s.trim()),
    props: [...m[2].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)].map((p): [string, string] => [p[1], p[2].trim()]),
  }));
}

/** Resolve `var()` aliases and `color-mix(in srgb, A p%, B)`; colours come out as `#rrggbb`. */
function resolveTheme(raw: Map<string, string>): Tokens {
  const out: Tokens = {};
  const resolve = (value: string, depth: number): string => {
    if (depth > 20) return value;
    let v = value.replace(/var\((--[a-z0-9-]+)\)/g, (all, k: string) =>
      raw.has(k) ? resolve(raw.get(k)!, depth + 1) : all,
    );
    v = v.replace(
      /color-mix\(\s*in\s+srgb\s*,\s*(#[0-9a-fA-F]{3,6})\s+([0-9.]+)%\s*,\s*(#[0-9a-fA-F]{3,6})\s*\)/g,
      (_all, a: string, p: string, b: string) => mix(a, b, Number(p) / 100),
    );
    return /^#[0-9a-fA-F]{3}$|^#[0-9a-fA-F]{6}$/.test(v.trim()) ? normalizeHex(v) : v.trim();
  };
  for (const [k, v] of raw) out[k] = resolve(v, 0);
  return out;
}

/** Parse tokens.css into `{ dark, light }`, each name to its value, `var()` aliases resolved. */
export function parseTokens(css: string): { dark: Tokens; light: Tokens } {
  const all = blocks(css);
  const theme = (name: "dark" | "light"): Tokens => {
    const applies = [":root", "[data-theme]", `[data-theme="${name}"]`];
    const raw = new Map<string, string>();
    for (const b of all) {
      // Blocks apply in source order, as the cascade does: the light block overrides `:root`'s.
      if (b.selectors.some((s) => applies.includes(s))) for (const [k, v] of b.props) raw.set(k, v);
    }
    return resolveTheme(raw);
  };
  return { dark: theme("dark"), light: theme("light") };
}

const isHex = (v: string | undefined) => !!v && /^#[0-9a-f]{6}$/.test(v);

/** Each pair below its bar, with the theme and the ratio. A token that is missing counts as 0. */
export function failures(
  css: string,
  pairs: Pair[] = PAIRS,
): { theme: "dark" | "light"; pair: Pair; ratio: number }[] {
  const themes = parseTokens(css);
  const out: { theme: "dark" | "light"; pair: Pair; ratio: number }[] = [];
  for (const theme of ["dark", "light"] as const) {
    const t = themes[theme];
    for (const pair of pairs) {
      const fg = t[pair.fg];
      const bg = t[pair.bg];
      const r = isHex(fg) && isHex(bg) ? ratio(fg, bg) : 0;
      if (r < pair.min) out.push({ theme, pair, ratio: r });
    }
  }
  return out;
}
