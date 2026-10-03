import { useRef } from "react";
import type { KeyboardEvent } from "react";
import {
  ANALYSIS_TABS,
  CUBE_TAB_UNAVAILABLE_TITLE,
  isTabNavKey,
  nextAnalysisTab,
  type AnalysisTabAvailability,
  type AnalysisTabId,
} from "../../utils/analysisSections";

interface AnalysisTabBarProps {
  active: AnalysisTabId;
  available: AnalysisTabAvailability;
  onSelect: (tab: AnalysisTabId) => void;
}

function AnalysisTabBar({ active, available, onSelect }: AnalysisTabBarProps) {
  const buttons = useRef<Partial<Record<AnalysisTabId, HTMLButtonElement | null>>>({});

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>, from: AnalysisTabId) => {
    if (!isTabNavKey(e.key)) return;
    e.preventDefault();
    const next = nextAnalysisTab(from, e.key, available);
    onSelect(next);
    buttons.current[next]?.focus();
  };

  return (
    <div role="tablist" aria-label="Analysis groups" className="flex gap-1">
      {ANALYSIS_TABS.map((t) => {
        const selected = t.id === active;
        const enabled = available[t.id];
        return (
          <button
            key={t.id}
            ref={(el) => {
              buttons.current[t.id] = el;
            }}
            type="button"
            role="tab"
            id={`analysis-tab-${t.id}`}
            data-analysis-tab={t.id}
            aria-selected={selected}
            aria-controls={`analysis-panel-${t.id}`}
            aria-disabled={enabled ? undefined : true}
            title={!enabled && t.id === "cube" ? CUBE_TAB_UNAVAILABLE_TITLE : undefined}
            tabIndex={selected ? 0 : -1}
            onClick={() => {
              if (enabled) onSelect(t.id);
            }}
            onKeyDown={(e) => onKeyDown(e, t.id)}
            className={`flex-1 text-[10px] px-2 py-1 rounded transition-colors ${
              selected
                ? "text-zinc-100 bg-zinc-800/80"
                : enabled
                  ? "text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800/50"
                  : "text-zinc-600 cursor-not-allowed"
            }`}
            style={{ border: selected ? "1px solid rgba(113,113,122,0.7)" : "1px solid rgba(63,63,70,0.5)" }}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

export default AnalysisTabBar;
