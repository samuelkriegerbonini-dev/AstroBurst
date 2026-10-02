import { useCallback } from "react";
import { DOCK_SIZE_LIMITS, type DockSizes } from "../../utils/dockLayout";
import { dockStore } from "../../hooks/useDockLayout";
import { startResizeDrag } from "./dockGeometry";

export interface ResizeSession {
  start: number;
  min: number;
  max: number;
  direction: 1 | -1;
  apply(value: number): void;
  commit(value: number): number;
  frozen: (HTMLElement | null)[];
}

export interface ResizeHandleProps {
  sizeKey: keyof DockSizes;
  axis: "x" | "y";
  open: boolean;
  begin(): ResizeSession | null;
  onActiveChange(active: boolean): void;
}

export default function ResizeHandle({ sizeKey, axis, open, begin, onActiveChange }: ResizeHandleProps) {
  const handleMouseDown = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    if (!open || e.button !== 0) return;
    const session = begin();
    if (!session) return;
    e.preventDefault();
    const origin = axis === "x" ? e.clientX : e.clientY;
    startResizeDrag({ sizeKey, axis, origin, handle: e.currentTarget, session, onActiveChange });
  }, [open, begin, axis, sizeKey, onActiveChange]);

  const handleDoubleClick = useCallback(() => {
    if (!open) return;
    dockStore.dispatch({ type: "resize", key: sizeKey, value: DOCK_SIZE_LIMITS[sizeKey].default });
  }, [open, sizeKey]);

  return (
    <div
      className={axis === "x" ? "ab-resize-handle" : "ab-resize-handle-h"}
      data-dock-handle={sizeKey}
      data-open={open ? "true" : "false"}
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
      title="Drag to resize — double-click to reset"
    />
  );
}
