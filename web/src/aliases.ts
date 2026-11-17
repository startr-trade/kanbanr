/** An item's earlier codes (FTR-056): a rename keeps the old code as an alias, and a link or a
 *  bookmark made under it still reaches the item. Pure, so a check can run it without a browser. */

/** The part of a feature or an index item these lookups read. */
export interface Coded {
  code: string;
  aliases?: string[];
}

/** The current code of the item that answers to `code`, or null when none does. A current code
 *  wins over an alias, as the server's `aliases::find` has it. */
export function currentCode(items: readonly Coded[], code: string): string | null {
  const exact = items.find((f) => f.code === code);
  if (exact) return exact.code;
  const renamed = items.find((f) => (f.aliases ?? []).includes(code));
  return renamed ? renamed.code : null;
}
