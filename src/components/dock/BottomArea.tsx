import { useCallback, useLayoutEffect, useState } from "react";
import { DOCK_TOOL_META, type DockToolId } from "../../utils/dockLayout";
import { bottomSplitLayout } from "../../utils/dockDrag";
import ResizeHandle, { type ResizeSession } from "./ResizeHandle";

export interface BottomAreaProps {
  open: boolean;
  height: number;
  leftTool: DockToolId | null;
  rightTool: DockToolId | null;
  split: number;
  outerRef: React.Ref<HTMLDivElement>;
  innerRef: React.RefObject<HTMLDivElement | null>;
  leftRef: React.Ref<HTMLDivElement>;
  rightRef: React.Ref<HTMLDivElement>;
  beginSplit(): ResizeSession | null;
  onResizeActive(active: boolean): void;
  onSettled(): void;
}

const COLLAPSED: React.CSSProperties = { width: 0, flex: "none" };
const FILL: React.CSSProperties = { flex: "1 1 0%" };

export default function BottomArea({
  open, height, leftTool, rightTool, split, outerRef, innerRef, leftRef, rightRef, beginSplit, onResizeActive, onSettled,
}: BottomAreaProps) {
  const [areaW, setAreaW] = useState(0);

  useLayoutEffect(() => {
    const inner = innerRef.current;
    if (!inner) return;
    setAreaW(inner.clientWidth);
    const ro = new ResizeObserver(() => setAreaW(inner.clientWidth));
    ro.observe(inner);
    return () => ro.disconnect();
  }, [innerRef]);

  const handleTransitionEnd = useCallback((e: React.TransitionEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget || e.propertyName !== "height") return;
    onSettled();
  }, [onSettled]);

  const layout = bottomSplitLayout({
    areaW,
    leftOpen: leftTool !== null,
    rightOpen: rightTool !== null,
    split,
    bottomH: height,
    leftMin: leftTool === null ? 0 : DOCK_TOOL_META[leftTool].minWidth,
    rightMin: rightTool === null ? 0 : DOCK_TOOL_META[rightTool].minWidth,
  });

  let leftStyle = COLLAPSED;
  let rightStyle = COLLAPSED;
  if (layout.mode === "single") {
    if (layout.left) leftStyle = FILL;
    else rightStyle = FILL;
  } else if (layout.mode === "split" && layout.left) {
    leftStyle = { width: layout.left.w, flex: "none" };
    rightStyle = { ...FILL, borderLeft: "1px solid var(--ab-border)" };
  }

  return (
    <div
      ref={outerRef}
      className="shrink-0 relative overflow-hidden ab-panel-anim-h"
      style={{ height: open ? height : 0 }}
      aria-hidden={!open}
      onTransitionEnd={handleTransitionEnd}
    >
      <div
        ref={innerRef}
        inert={!open}
        data-bottom-mode={layout.mode}
        className="absolute inset-x-0 bottom-0 flex"
        style={{ height }}
      >
        <div ref={leftRef} data-dock-anchor="left-bottom" className="relative h-full min-w-0 overflow-hidden" style={leftStyle} />
        <ResizeHandle
          sizeKey="bottomSplit"
          axis="x"
          open={layout.mode === "split"}
          begin={beginSplit}
          onActiveChange={onResizeActive}
        />
        <div ref={rightRef} data-dock-anchor="right-bottom" className="relative h-full min-w-0 overflow-hidden" style={rightStyle} />
      </div>
    </div>
  );
}
