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

if (failures.length > 0) {
  console.error(`check:ui — ${failures.length} element(s) that act but do not look like it:\n`);
  for (const f of failures) console.error(f + "\n");
  process.exit(1);
}
console.log(`check:ui — every action in ${root.split("/").slice(-2).join("/")} uses the control style.`);
