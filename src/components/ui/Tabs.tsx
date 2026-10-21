import React, { useRef } from "react";

export interface TabItem<T extends string> {
  id: T;
  label: string;
  disabled?: boolean;
  /** Shown on hover, for example to say why a tab is disabled. */
  hint?: string;
}

interface TabsProps<T extends string> {
  tabs: TabItem<T>[];
  selected: T;
  onSelect: (id: T) => void;
  label: string;
  /** Prefix of the ids that tie each tab to its panel. */
  idPrefix: string;
}

export const tabId = (prefix: string, id: string) => `${prefix}-tab-${id}`;
export const panelId = (prefix: string, id: string) => `${prefix}-panel-${id}`;

/** An accessible tab list: arrow keys move between the enabled tabs. */
export const Tabs = <T extends string>({
  tabs,
  selected,
  onSelect,
  label,
  idPrefix,
}: TabsProps<T>) => {
  const refs = useRef(new Map<T, HTMLButtonElement>());

  const move = (from: T, step: number) => {
    const enabled = tabs.filter((tab) => !tab.disabled);
    const index = enabled.findIndex((tab) => tab.id === from);
    const next = enabled[(index + step + enabled.length) % enabled.length];
    if (!next) return;
    onSelect(next.id);
    refs.current.get(next.id)?.focus();
  };

  return (
    <div
      role="tablist"
      aria-label={label}
      className="flex gap-6 border-b border-mid-gray/20"
    >
      {tabs.map((tab) => {
        const active = tab.id === selected;
        return (
          <button
            key={tab.id}
            ref={(element) => {
              if (element) refs.current.set(tab.id, element);
              else refs.current.delete(tab.id);
            }}
            type="button"
            role="tab"
            id={tabId(idPrefix, tab.id)}
            aria-controls={panelId(idPrefix, tab.id)}
            aria-selected={active}
            tabIndex={active ? 0 : -1}
            disabled={tab.disabled}
            title={tab.hint}
            onClick={() => onSelect(tab.id)}
            onKeyDown={(event) => {
              if (event.key === "ArrowRight") move(tab.id, 1);
              if (event.key === "ArrowLeft") move(tab.id, -1);
            }}
            className={`-mb-px cursor-pointer border-b-2 px-0.5 pb-2 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-text disabled:cursor-not-allowed disabled:opacity-50 ${
              active
                ? "border-accent-text text-text"
                : "border-transparent text-mid-gray hover:text-text"
            }`}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
};
