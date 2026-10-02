import { createPortal } from "react-dom";
import { DOCK_TOOL_META } from "../../utils/dockLayout";
import type { Rect } from "../../utils/dockDrag";
import { DOCK_TOOLS } from "./toolRegistry";
import { useDockDragState } from "./useDockDrag";
import "./dock.css";

const GHOST_OFFSET_PX = 14;

function rectStyle(r: Rect): React.CSSProperties {
  return { left: r.x, top: r.y, width: r.w, height: r.h };
}

export default function DragOverlay(): React.ReactNode {
  const drag = useDockDragState();
  if (drag === null) return null;
  const Icon = DOCK_TOOLS[drag.tool].icon;
  const follow = `translate(${drag.pointer.x + GHOST_OFFSET_PX}px, ${drag.pointer.y + GHOST_OFFSET_PX}px)`;
  return createPortal(
    <>
      {drag.preview && <div data-dock-drop-preview="" className="ab-dock-drop-preview" style={rectStyle(drag.preview)} />}
      {drag.slot && <div data-dock-drop-slot="" className="ab-dock-drop-slot" style={rectStyle(drag.slot)} />}
      {drag.marker && <div data-dock-insert-marker="" className="ab-dock-insert-marker" style={rectStyle(drag.marker)} />}
      <div className="ab-dock-drag-follow" style={{ transform: follow }}>
        <div data-dock-drag-ghost="" className="ab-dock-drag-ghost">
          <Icon size={14} aria-hidden="true" />
          <span>{DOCK_TOOL_META[drag.tool].shortLabel}</span>
        </div>
        {drag.label && <div data-dock-drag-label="" className="ab-dock-drag-label">{drag.label}</div>}
      </div>
    </>,
    document.body,
  );
}
