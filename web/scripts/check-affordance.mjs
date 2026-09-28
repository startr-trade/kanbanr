/**
 * A control must look like a control (FEAT-076 R-2).
 *
 * The monitor's stylesheet has two vocabularies: `.chip` is a label — no pointer cursor, no hover
 * — and `.btn` is a control. The approve action shipped as a `.chip`, so the person being asked to
 * agree could not tell the action from a tag: the cursor never changed, and nothing separated it
 * from the status chips beside it. That is not a style preference; it made the approval gate's one
 * required human step unfindable, which is the failure the gate exists to prevent.
 *
 * Greppable rules are worth a script rather than a review habit, because this decays silently: the
 * next `className="chip"` on a clickable element reintroduces it and looks perfectly ordinary in a
 * diff. Run by `npm run check:ui`.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const root = new URL("../src", import.meta.url).pathname;

function walk(dir) {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    return statSync(path).isDirectory() ? walk(path) : /\.tsx?$/.test(path) ? [path] : [];
  });
}

const failures = [];
for (const file of walk(root)) {
  const src = readFileSync(file, "utf8");
  src.split("\n").forEach((line, i) => {
    const at = `${file.slice(root.length + 1)}:${i + 1}`;
    // A <button> is a control by role, so it must not borrow the label style.
    if (/<button[^>]*className="[^"]*\bchip\b/.test(line)) {
      failures.push(`${at}  a <button> styled as a chip — use "btn" (FEAT-076 R-2)\n    ${line.trim()}`);
    }
    // And a chip must not be given an action, which is the same error from the other direction.
    if (/className="[^"]*\bchip\b[^"]*"[^>]*onClick/.test(line)) {
      failures.push(`${at}  a chip with an onClick — labels are not controls (FEAT-076 R-2)\n    ${line.trim()}`);
    }
  });
}

/**
 * The class name was never the requirement (FEAT-081).
 *
 * The check above passed while "Withdraw approval" still looked exactly like a tag, because `.btn`
 * and `.chip` were declared with the *same* background and border — a button was a chip with
 * squarer corners. Verifying the class and not the appearance is verifying the letter of the rule,
 * so this asserts the two vocabularies do not share a surface. It is the one visual property that
 * is mechanically checkable from the stylesheet, and it is the one that failed.
 */
const css = readFileSync(new URL("../src/styles.css", import.meta.url).pathname, "utf8");
const ruleFor = (selector) => {
  const m = css.match(new RegExp(`\\n\\${selector}\\s*\\{([^}]*)\\}`));
  return m ? m[1] : null;
};
const declared = (block, prop) => {
  const m = block?.match(new RegExp(`(?:^|;|\\s)${prop}\\s*:\\s*([^;]+)`));
  return m ? m[1].trim() : null;
};
const chip = ruleFor(".chip");
const btn = ruleFor(".btn");
if (chip == null || btn == null) {
  failures.push("styles.css: cannot find the .chip and .btn rules — this check has gone stale");
} else {
  const chipBg = declared(chip, "background");
  const btnBg = declared(btn, "background");
  if (chipBg && btnBg && chipBg === btnBg) {
    failures.push(
      `styles.css: .btn and .chip share a background (${btnBg}), so a control looks like a label\n` +
        `    A button must be distinguishable from a tag beside it without hovering (FEAT-081).`,
    );
  }
}

/**
 * Each destination is offered once (FEAT-083).
 *
 * The header grew two navigations at different times — a global nav and a breadcrumb — and both
 * rendered a `Projects` link to `/`. Each was correct alone; nothing looked at the shell as one
 * thing. Two identical labels pointing at one place ask the reader to find a difference that is not
 * there.
 */
const shell = readFileSync(new URL("../src/App.tsx", import.meta.url).pathname, "utf8");

/**
 * Pull `{to, label}` out of every <Link>/<NavLink> in the shell.
 *
 * Deliberately a scanner rather than a regex: a JSX attribute routinely contains `>` inside a brace
 * expression (`className={({ isActive }) => …}`), so the obvious `<Link[^>]*>` stops in the middle
 * of the tag and matches nothing. The first version of this check did exactly that and passed
 * happily against the very duplicate it was written for — caught only because ADR-0008 requires a
 * check to be observed failing before it counts.
 */
function shellLinks(src) {
  const found = [];
  const tag = /<(Link|NavLink)\b/g;
  let m;
  while ((m = tag.exec(src)) !== null) {
    // Walk to this tag's closing `>`, ignoring any inside braces, brackets or strings.
    let i = m.index + m[0].length;
    let depth = 0;
    let quote = null;
    for (; i < src.length; i++) {
      const c = src[i];
      if (quote) {
        if (c === quote && src[i - 1] !== "\\") quote = null;
        continue;
      }
      if (c === '"' || c === "'" || c === "`") quote = c;
      else if (c === "{" || c === "[") depth++;
      else if (c === "}" || c === "]") depth--;
      else if (c === ">" && depth === 0) break;
    }
    const attrs = src.slice(m.index, i);
    if (/\/\s*$/.test(attrs)) continue; // self-closing: no label
    const to = attrs.match(/\bto=(?:"([^"]*)"|\{`([^`]*)`\}|\{"([^"]*)"\})/);
    if (!to) continue;
    const label = src.slice(i + 1, src.indexOf("<", i + 1)).replace(/\s+/g, " ").trim();
    if (label) found.push({ to: (to[1] ?? to[2] ?? to[3]).trim(), label });
  }
  return found;
}

const seen = new Map();
for (const { to, label } of shellLinks(shell)) {
  const key = `${to}\u0000${label}`;
  seen.set(key, (seen.get(key) ?? 0) + 1);
}
for (const [key, count] of seen) {
  if (count > 1) {
    const [to, label] = key.split("\u0000");
    failures.push(
      `App.tsx: "${label}" -> ${to} appears ${count} times in the app shell\n` +
        `    Offer each destination once; two identical links invite a choice that does not exist (FEAT-083).`,
    );
  }
}

if (failures.length > 0) {
  console.error(`check:ui — ${failures.length} problem(s) with how the monitor presents itself:\n`);
  for (const f of failures) console.error(f + "\n");
  process.exit(1);
}
console.log(`check:ui — every action looks like a control, and every destination is offered once.`);
