import { useCallback, useSyncExternalStore } from "react";
import { DOCK_SIZE_LIMITS, type DockSizes } from "../../utils/dockLayout";
import { dockStore } from "../../hooks/useDockLayout";
import { startResizeDrag } from "./dockGeometry";
import { RESIZE_HANDLE_LABELS, resizeKeyMove, resizeTarget } from "./resizeHandleModel";

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

export const RESIZE_HANDLE_TITLE = "Drag or use the arrow keys to resize; double-click or press Enter to reset";

export default function ResizeHandle({ sizeKey, axis, open, begin, onActiveChange }: ResizeHandleProps) {
  const readSize = () => dockStore.get().sizes[sizeKey];
  const size = useSyncExternalStore(dockStore.subscribe, readSize, readSize);
  const limits = DOCK_SIZE_LIMITS[sizeKey];
  const percent = sizeKey === "bottomSplit";
  const shown = (value: number) => Math.round(percent ? value * 100 : value);
  const valueNow = shown(size);

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

  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!open) return;
    if (e.key === "Enter") {
      e.preventDefault();
      dockStore.dispatch({ type: "resize", key: sizeKey, value: DOCK_SIZE_LIMITS[sizeKey].default });
      return;
    }
    const move = resizeKeyMove(e.key, e.shiftKey, axis);
    if (!move) return;
    const session = begin();
    if (!session) return;
    e.preventDefault();
    const next = resizeTarget(move, session);
    if (next !== session.start) dockStore.dispatch({ type: "resize", key: sizeKey, value: session.commit(next) });
  }, [open, begin, axis, sizeKey]);

  return (
    <div
      role="separator"
      tabIndex={open ? 0 : -1}
      aria-orientation={axis === "x" ? "vertical" : "horizontal"}
      aria-label={RESIZE_HANDLE_LABELS[sizeKey]}
      aria-valuenow={valueNow}
      aria-valuemin={shown(limits.min)}
      aria-valuemax={shown(limits.max)}
      aria-valuetext={`${valueNow} ${percent ? "%" : "px"}`}
      className={axis === "x" ? "ab-resize-handle" : "ab-resize-handle-h"}
      data-dock-handle={sizeKey}
      data-open={open ? "true" : "false"}
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
      onKeyDown={handleKeyDown}
      title={RESIZE_HANDLE_TITLE}
    />
  );
}
