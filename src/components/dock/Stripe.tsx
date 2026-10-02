import {
  DOCK_TOOL_META,
  type DockAnchor,
  type DockLayout,
  type DockSide,
  type DockToolId,
} from "../../utils/dockLayout";
import { dockStore } from "../../hooks/useDockLayout";
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

function isMenuKey(e: React.KeyboardEvent<HTMLButtonElement>): boolean {
  return e.key === "ContextMenu" || (e.shiftKey && e.key === "F10");
}

export default function Stripe({ side, layout, hasFile, buttonProps, onContextMenu }: StripeProps) {
  const drag = useDockDragState();
  const targetAnchor = drag?.target?.anchor ?? null;

  const renderGroup = (anchor: DockAnchor) => (
    <div
      data-dock-group={anchor}
      className={`ab-dock-group${targetAnchor === anchor ? " ab-dock-group-target" : ""}`}
    >
      {layout.anchors[anchor].map((tool, index) => {
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
            title={meta.label}
            className={`ab-dock-strip-btn${isActive ? " ab-dock-strip-btn-active" : ""}`}
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
          </button>
        );
      })}
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
