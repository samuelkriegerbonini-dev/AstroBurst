import type { ReactNode } from "react";
import {
  DOCK_TOOL_META,
  type DockAnchor,
  type DockLayout,
  type DockSide,
  type DockToolId,
} from "../../utils/dockLayout";
import { dockStore } from "../../hooks/useDockLayout";
import { useMeasurementLog } from "../../hooks/useMeasurementLog";
import { DOCK_TOOLS } from "./toolRegistry";
import { useDockDragState, type StripButtonDragProps } from "./useDockDrag";

export interface StripeProps {
  side: DockSide;
  layout: DockLayout;
  hasFile: boolean;
  buttonProps(tool: DockToolId, anchor: DockAnchor, index: number): StripButtonDragProps;
  onContextMenu(tool: DockToolId, button: HTMLButtonElement): void;
}

const GROUPS: Record<DockSide, [DockAnchor, DockAnchor]> = {
  left: ["left-top", "left-bottom"],
  right: ["right-top", "right-bottom"],
};

const LOG_BADGE_CAP = 99;

function isMenuKey(e: React.KeyboardEvent<HTMLButtonElement>): boolean {
  return e.key === "ContextMenu" || (e.shiftKey && e.key === "F10");
}

function logStripLabel(count: number): string {
  const label = DOCK_TOOL_META.log.label;
  if (count <= 0) return label;
  return `${label} (${count} ${count === 1 ? "entry" : "entries"})`;
}

function logStripBadge(count: number): string | null {
  if (count <= 0) return null;
  return count > LOG_BADGE_CAP ? `${LOG_BADGE_CAP}+` : String(count);
}

function LogStripCount({ children }: { children(label: string, badge: ReactNode): ReactNode }) {
  const count = useMeasurementLog().length;
  const badge = logStripBadge(count);
  return children(
    logStripLabel(count),
    badge === null ? null : (
      <span
        data-log-count=""
        aria-hidden="true"
        className="absolute -top-0.5 right-0 min-w-[14px] px-0.5 rounded-full text-[7px] leading-[11px] text-center tabular-nums text-zinc-100"
        style={{ background: "rgba(63,63,70,0.95)" }}
      >
        {badge}
      </span>
    ),
  );
}

export default function Stripe({ side, layout, hasFile, buttonProps, onContextMenu }: StripeProps) {
  const drag = useDockDragState();
  const targetAnchor = drag?.target?.anchor ?? null;

  const renderButton = (tool: DockToolId, anchor: DockAnchor, index: number, label: string, badge: ReactNode) => {
    const meta = DOCK_TOOL_META[tool];
    const Icon = DOCK_TOOLS[tool].icon;
    const isActive = layout.active[anchor] === tool;
    const disabled = meta.needsFile && !hasFile;
    return (
      <button
        key={tool}
        type="button"
        data-tool-id={tool}
        data-anchor={anchor}
        data-dragging={drag?.tool === tool ? "true" : undefined}
        aria-pressed={isActive}
        aria-haspopup="menu"
        aria-disabled={disabled ? true : undefined}
        title={label}
        aria-label={label}
        className={`ab-dock-strip-btn${isActive ? " ab-dock-strip-btn-active" : ""}${badge ? " relative" : ""}`}
        style={isActive ? ({ "--strip-accent": meta.accent } as React.CSSProperties) : undefined}
        {...buttonProps(tool, anchor, index)}
        onClick={() => {
          if (disabled) return;
          dockStore.dispatch({ type: "toggle", tool });
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          onContextMenu(tool, e.currentTarget);
        }}
        onKeyDown={(e) => {
          if (!isMenuKey(e)) return;
          e.preventDefault();
          onContextMenu(tool, e.currentTarget);
        }}
      >
        <Icon size={14} />
        <span>{meta.shortLabel}</span>
        {badge}
      </button>
    );
  };

  const renderGroup = (anchor: DockAnchor) => (
    <div
      data-dock-group={anchor}
      className={`ab-dock-group${targetAnchor === anchor ? " ab-dock-group-target" : ""}`}
    >
      {layout.anchors[anchor].map((tool, index) =>
        tool === "log" ? (
          <LogStripCount key={tool}>{(label, badge) => renderButton(tool, anchor, index, label, badge)}</LogStripCount>
        ) : (
          renderButton(tool, anchor, index, DOCK_TOOL_META[tool].label, null)
        ),
      )}
    </div>
  );

  const [top, bottom] = GROUPS[side];
  return (
    <div
      className="ab-dock-strip"
      data-side={side}
      data-dock-strip={side}
      role="group"
      aria-label={side === "left" ? "Left tools" : "Right tools"}
    >
      {renderGroup(top)}
      <div className="flex-1" />
      {renderGroup(bottom)}
    </div>
  );
}
