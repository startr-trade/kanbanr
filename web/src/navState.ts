// Which navigation menu is open (FTR-047): a pure reducer, so the open/close rules are checked by
// node without React (web/scripts/check-nav.mjs). The shell applies `returnFocus` with a ref in an
// effect, then clears it.
import type { AreaKey } from "./nav.ts";

export type NavEvent =
  | { type: "toggle"; area: AreaKey }
  | { type: "close" }
  | { type: "escape" }
  | { type: "navigate" }
  | { type: "outside" }
  | { type: "toggle-mobile" };

export interface NavState {
  /** The area whose menu is open on a wide screen. */
  open: AreaKey | null;
  /** Whether the narrow-screen menu panel is open. */
  mobile: boolean;
  /** Where focus goes back to after Escape: an area's button, or the mobile menu button. */
  returnFocus: AreaKey | "mobile" | null;
}

export function navReducer(s: NavState, e: NavEvent): NavState {
  switch (e.type) {
    case "toggle":
      // One menu at a time: opening one closes any other; toggling the open one closes it.
      return { ...s, open: s.open === e.area ? null : e.area, returnFocus: null };
    case "escape": {
      if (s.open === null && !s.mobile) return s;
      const returnFocus = s.open ?? "mobile";
      return { open: null, mobile: false, returnFocus };
    }
    case "close":
    case "navigate":
    case "outside":
      return { open: null, mobile: false, returnFocus: null };
    case "toggle-mobile":
      return { open: null, mobile: !s.mobile, returnFocus: null };
  }
}
