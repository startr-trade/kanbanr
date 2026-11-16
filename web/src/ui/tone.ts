/**
 * A severity tone (FTR-046). Tone is never carried by colour alone: where a pill's or tile's own
 * text does not say it, the component adds `TONE_WORD[tone]` — visibly or for a screen reader.
 */
export type Tone = "ok" | "info" | "warn" | "error" | "neutral";

/** The word a screen reader hears when a pill's own text does not say its tone. */
export const TONE_WORD: Record<Tone, string> = {
  ok: "",
  info: "note",
  warn: "warning",
  error: "error",
  neutral: "",
};
