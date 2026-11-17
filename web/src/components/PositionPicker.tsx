/**
 * The position picker (FTR-004, R-10): which version of the repository's docs a page reads. It sits
 * in the Docs page header, on the Docs page and on the repository doc pages, and nowhere else: an
 * item page's links open at the item, so it carries none.
 *
 * A `<select>` in the style of FTR-046's tabs (its tokens, a control's height, the focus ring and an
 * accent underline on the current value) lists main, the working tree, the sprints and the releases;
 * a text field takes an item code, a commit or a position text. Choosing writes `?at=` with
 * `replace` and keeps the pick for the session.
 */
import { useEffect, useState, type FormEvent } from "react";
import { useLocation } from "react-router-dom";
import { useUrlState } from "../ui/urlState.ts";
import { atOf, currentPick, rememberPick } from "../position";
import type { PickView } from "../types";

/**
 * The pick the page reads at: the URL's `at`, else the one kept for the session, else main. A pick
 * the URL names is kept, so the next page opens at the same position.
 */
export function usePick(): string {
  const { search } = useLocation();
  const pick = currentPick(search);
  const fromUrl = atOf(search);
  useEffect(() => {
    if (fromUrl) rememberPick(fromUrl);
  }, [fromUrl]);
  return pick;
}

export function PositionPicker({
  picks,
  pick,
  onChoose,
}: {
  /** The positions the daemon offers; null while they are read or when the read failed. */
  picks: PickView[] | null;
  /** The pick the page reads at now. */
  pick: string;
  /** Called with the new pick; the page navigates when it is given (a test passes none). */
  onChoose?: (pick: string) => void;
}) {
  const [, setState] = useUrlState({ at: "" });
  const [typed, setTyped] = useState("");
  const choose = (next: string) => {
    const value = next.trim();
    if (!value) return;
    rememberPick(value);
    if (onChoose) onChoose(value);
    else setState({ at: value });
  };
  const listed = (picks ?? []).some((p) => p.pick === pick);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    choose(typed);
    setTyped("");
  };
  return (
    <form className="ui-position" onSubmit={submit} role="group" aria-label="Position the docs are read at">
      <label className="ui-position-label" htmlFor="position-select">
        Read at
      </label>
      <select
        id="position-select"
        className="ui-position-select"
        value={pick}
        onChange={(e) => choose(e.target.value)}
      >
        {!listed && <option value={pick}>{pick}</option>}
        {(picks ?? []).map((p) => (
          <option key={p.pick} value={p.pick} disabled={!p.available} title={p.available ? undefined : p.why}>
            {p.available ? p.label : `${p.label} (${p.why || "not available"})`}
          </option>
        ))}
      </select>
      <label className="ui-position-label" htmlFor="position-typed">
        or
      </label>
      <input
        id="position-typed"
        className="ui-position-typed"
        type="text"
        value={typed}
        onChange={(e) => setTyped(e.target.value)}
        placeholder="item code, commit or position"
        autoComplete="off"
        spellCheck={false}
      />
      <button type="submit" className="btn ui-position-go" disabled={!typed.trim()}>
        Go
      </button>
    </form>
  );
}
