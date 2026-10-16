import type { CSSProperties } from "react";
import type { AppMode } from "../hooks/useAppMode";

interface ModeSwitchProps {
  mode: AppMode;
  onChange: (mode: AppMode) => void;
}

const SEGMENTS: { mode: AppMode; label: string; digit: string }[] = [
  { mode: "workspace", label: "Workspace", digit: "1" },
  { mode: "archive", label: "Archive", digit: "2" },
];

const SEGMENT_STYLE: CSSProperties = { padding: "1px 8px", fontSize: 10 };
const ACTIVE_STYLE: CSSProperties = { ...SEGMENT_STYLE, background: "var(--ab-accent-bg-strong)", color: "var(--ab-accent)" };

export default function ModeSwitch({ mode, onChange }: ModeSwitchProps) {
  const modifier = navigator.platform?.includes("Mac") ? "⌘" : "Ctrl+";
  return (
    <div
      data-testid="mode-switch"
      role="group"
      aria-label="Mode"
      className="flex items-center gap-0.5 p-0.5 rounded"
      style={{ border: "1px solid var(--ab-border)" }}
    >
      {SEGMENTS.map((segment) => {
        const active = segment.mode === mode;
        return (
          <button
            key={segment.mode}
            type="button"
            className="ab-pill"
            data-mode={segment.mode}
            data-active={active ? "true" : "false"}
            aria-pressed={active}
            title={`${segment.label} (${modifier}${segment.digit})`}
            style={active ? ACTIVE_STYLE : SEGMENT_STYLE}
            onClick={() => {
              if (!active) onChange(segment.mode);
            }}
          >
            {segment.label}
          </button>
        );
      })}
    </div>
  );
}
