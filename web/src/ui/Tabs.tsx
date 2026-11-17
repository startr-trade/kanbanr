/**
 * Tabs with counts (FTR-046, R-1). An underlined tab list (R-12) whose selected key is kept in the
 * URL parameter `param`, written with `replace`. Only the selected panel is rendered.
 */
import { useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { pickTab, visibleTabs, type TabDef } from "./tabs.ts";
import { useUrlState } from "./urlState.ts";

export type { TabDef } from "./tabs.ts";

export function Tabs(p: {
  tabs: TabDef[];
  /** The URL parameter that holds the selected key; default `tab`. */
  param?: string;
  /** The tab to prefer when the URL names none (see `pickTab`). */
  defaultKey?: string;
  children: (key: string) => ReactNode;
  /** The tab list's accessible name. */
  label: string;
}): JSX.Element {
  const param = p.param ?? "tab";
  const [state, setState] = useUrlState({ [param]: "" } as Record<string, string>);
  const shown = visibleTabs(p.tabs);
  const selected = pickTab(p.tabs, state[param] || null, p.defaultKey);
  const base = useId();
  const tabId = (key: string) => `${base}-tab-${key}`;
  const panelId = (key: string) => `${base}-panel-${key}`;
  const buttons = useRef(new Map<string, HTMLButtonElement>());

  const choose = (key: string) => {
    if (key !== selected) setState({ [param]: key });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const enabled = shown.filter((t) => !t.disabled);
    if (enabled.length === 0) return;
    const at = enabled.findIndex((t) => t.key === selected);
    let next: number;
    switch (e.key) {
      case "ArrowRight":
        next = (at + 1) % enabled.length;
        break;
      case "ArrowLeft":
        next = (at - 1 + enabled.length) % enabled.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = enabled.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    const key = enabled[next].key;
    choose(key);
    buttons.current.get(key)?.focus();
  };

  return (
    <div className="ui-tabs-wrap">
      <div className="ui-tabs" role="tablist" aria-label={p.label} onKeyDown={onKeyDown}>
        {shown.map((t, i) => {
          const isSelected = t.key === selected;
          // With every tab disabled nothing is selected, so the first keeps the tab stop and the
          // list stays reachable by keyboard.
          const isStop = isSelected || (selected === "" && i === 0);
          return (
            <button
              key={t.key}
              ref={(el) => {
                if (el) buttons.current.set(t.key, el);
                else buttons.current.delete(t.key);
              }}
              type="button"
              role="tab"
              className="ui-tab"
              id={tabId(t.key)}
              aria-selected={isSelected}
              aria-controls={panelId(t.key)}
              aria-disabled={t.disabled ? true : undefined}
              title={t.disabled || undefined}
              aria-label={t.pending ? `${t.label}, counting` : undefined}
              tabIndex={isStop ? 0 : -1}
              onClick={() => {
                if (!t.disabled) choose(t.key);
              }}
            >
              {t.label}
              {t.pending ? (
                <>
                  {" "}
                  <span className="ui-count">…</span>
                </>
              ) : (
                t.count !== undefined && (
                  <>
                    {" "}
                    <span className="ui-count">{t.count}</span>
                  </>
                )
              )}
            </button>
          );
        })}
      </div>
      {selected && (
        <div className="ui-tabpanel" role="tabpanel" id={panelId(selected)} aria-labelledby={tabId(selected)} tabIndex={0}>
          {p.children(selected)}
        </div>
      )}
    </div>
  );
}
