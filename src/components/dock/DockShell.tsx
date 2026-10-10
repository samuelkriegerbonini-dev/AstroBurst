import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ANCHOR_LABELS,
  COLUMN_VW_CAP_DOUBLE,
  COLUMN_VW_CAP_SINGLE,
  DOCK_ANCHORS,
  DOCK_SIZE_LIMITS,
  DOCK_TOOL_META,
  MIN_PREVIEW_H,
  MIN_PREVIEW_W,
  anchorOpenTool,
  type DockAnchor,
  type DockToolId,
  type SideAnchor,
} from "../../utils/dockLayout";
import { clampBottomToViewer, clampColumnsToViewer } from "../../utils/dockDrag";
import { dockStore, useDockLayout } from "../../hooks/useDockLayout";
import { fileKeyOf, useFileContext } from "../../context/PreviewContext";
import Stripe from "./Stripe";
import SideColumn from "./SideColumn";
import BottomArea from "./BottomArea";
import ResizeHandle, { type ResizeSession } from "./ResizeHandle";
import ToolHostRoot from "./ToolHostRoot";
import DragOverlay from "./DragOverlay";
import DockOnboarding from "./DockOnboarding";
import DockContextMenu, { type DockMenuRequest } from "./DockContextMenu";
import LiveRegion from "./LiveRegion";
import { useDockDrag } from "./useDockDrag";
import { useAnalysisRouting } from "./useAnalysisRouting";
import { nextShown, settleBottom, settleSide, type AnchorTools } from "./dockGeometry";

export interface DockShellProps {
  tools: Partial<Record<DockToolId, React.ReactNode>>;
  children: React.ReactNode;
}

const SIDE_SIZE_KEY: Record<SideAnchor, "leftW" | "rightW"> = { "left-top": "leftW", "right-top": "rightW" };

function sideMin(anchor: SideAnchor, tool: DockToolId | null): number {
  const limit = DOCK_SIZE_LIMITS[SIDE_SIZE_KEY[anchor]].min;
  return tool === null ? limit : Math.max(limit, DOCK_TOOL_META[tool].minWidth);
}

function columnWidthCss(min: number, size: number, bothOpen: boolean): string {
  return `clamp(${min}px, ${size}px, ${bothOpen ? COLUMN_VW_CAP_DOUBLE : COLUMN_VW_CAP_SINGLE}vw)`;
}

function panelLabel(anchor: DockAnchor, tool: DockToolId | null): string {
  return tool === null ? ANCHOR_LABELS[anchor] : DOCK_TOOL_META[tool].label;
}

function observeWithFrame(el: Element, run: () => void): () => void {
  let raf: number | null = null;
  const ro = new ResizeObserver(() => {
    if (raf === null) {
      raf = requestAnimationFrame(() => {
        raf = null;
        run();
      });
    }
  });
  ro.observe(el);
  return () => {
    ro.disconnect();
    if (raf !== null) cancelAnimationFrame(raf);
  };
}

export default function DockShell({ tools, children }: DockShellProps) {
  const layout = useDockLayout();
  const { file } = useFileContext();
  const hasFile = file !== null;
  const fileKey = fileKeyOf(file);
  useAnalysisRouting(layout.active, fileKey);

  const open = useMemo(() => {
    const result = {} as AnchorTools;
    for (const anchor of DOCK_ANCHORS) result[anchor] = anchorOpenTool(layout, anchor, hasFile);
    return result;
  }, [layout, hasFile]);

  const [shownState, setShown] = useState<AnchorTools>(open);
  const shown = nextShown(shownState, open);
  if (shown !== shownState) setShown(shown);

  const rootRef = useRef<HTMLDivElement>(null);
  const centreRef = useRef<HTMLDivElement>(null);
  const leftOuterRef = useRef<HTMLDivElement>(null);
  const leftInnerRef = useRef<HTMLDivElement>(null);
  const rightOuterRef = useRef<HTMLDivElement>(null);
  const rightInnerRef = useRef<HTMLDivElement>(null);
  const bottomOuterRef = useRef<HTMLDivElement>(null);
  const bottomInnerRef = useRef<HTMLDivElement>(null);
  const containers = useRef<Record<DockAnchor, HTMLElement | null>>({
    "left-top": null,
    "left-bottom": null,
    "right-top": null,
    "right-bottom": null,
  });
  const containerRefs = useMemo(() => {
    const refs = {} as Record<DockAnchor, (el: HTMLDivElement | null) => void>;
    for (const anchor of DOCK_ANCHORS) {
      refs[anchor] = (el) => {
        containers.current[anchor] = el;
      };
    }
    return refs;
  }, []);

  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  const openRef = useRef(open);
  openRef.current = open;
  const resizingRef = useRef(false);
  const handleResizeActive = useCallback((active: boolean) => {
    resizingRef.current = active;
  }, []);

  const handleSideSettled = useCallback((anchor: SideAnchor) => {
    const o = openRef.current;
    setShown((s) => settleSide(s, o, anchor));
  }, []);

  const handleBottomSettled = useCallback(() => {
    const o = openRef.current;
    setShown((s) => settleBottom(s, o));
  }, []);

  const beginColumn = useCallback((anchor: SideAnchor): ResizeSession | null => {
    const left = anchor === "left-top";
    const outer = (left ? leftOuterRef : rightOuterRef).current;
    const inner = (left ? leftInnerRef : rightInnerRef).current;
    const centre = centreRef.current;
    const o = openRef.current;
    const tool = o[anchor];
    if (!outer || !inner || !centre || tool === null) return null;
    const min = sideMin(anchor, tool);
    const start = inner.offsetWidth;
    const max = Math.min(DOCK_SIZE_LIMITS[SIDE_SIZE_KEY[anchor]].max, Math.max(min, start + centre.clientWidth - MIN_PREVIEW_W));
    const both = o["left-top"] !== null && o["right-top"] !== null;
    return {
      start,
      min,
      max,
      direction: left ? 1 : -1,
      apply: (value) => {
        const css = columnWidthCss(min, value, both);
        outer.style.width = css;
        inner.style.width = css;
      },
      commit: (value) => value,
      frozen: [outer],
    };
  }, []);
  const beginLeft = useCallback(() => beginColumn("left-top"), [beginColumn]);
  const beginRight = useCallback(() => beginColumn("right-top"), [beginColumn]);

  const beginBottom = useCallback((): ResizeSession | null => {
    const outer = bottomOuterRef.current;
    const inner = bottomInnerRef.current;
    if (!outer || !inner) return null;
    const { min, max: limitMax } = DOCK_SIZE_LIMITS.bottomH;
    const start = layoutRef.current.sizes.bottomH;
    const viewport = rootRef.current?.querySelector<HTMLElement>("[data-dock-viewport]");
    const max = viewport ? Math.min(limitMax, Math.max(min, start + viewport.clientHeight - MIN_PREVIEW_H)) : limitMax;
    return {
      start,
      min,
      max,
      direction: -1,
      apply: (value) => {
        outer.style.height = `${value}px`;
        inner.style.height = `${value}px`;
      },
      commit: (value) => value,
      frozen: [outer],
    };
  }, []);

  const beginSplit = useCallback((): ResizeSession | null => {
    const inner = bottomInnerRef.current;
    const leftHalf = containers.current["left-bottom"];
    const o = openRef.current;
    const leftTool = o["left-bottom"];
    const rightTool = o["right-bottom"];
    if (!inner || !leftHalf || leftTool === null || rightTool === null) return null;
    const areaW = inner.clientWidth;
    if (areaW <= 0) return null;
    const leftMin = DOCK_TOOL_META[leftTool].minWidth;
    const rightMin = DOCK_TOOL_META[rightTool].minWidth;
    const fits = areaW >= leftMin + rightMin;
    const limit = DOCK_SIZE_LIMITS.bottomSplit;
    return {
      start: leftHalf.offsetWidth,
      min: Math.max(Math.ceil(areaW * limit.min), fits ? leftMin : 0),
      max: Math.min(Math.floor(areaW * limit.max), fits ? areaW - rightMin : areaW),
      direction: 1,
      apply: (value) => {
        leftHalf.style.width = `${value}px`;
      },
      commit: (value) => value / areaW,
      frozen: [],
    };
  }, []);

  useEffect(() => {
    const root = rootRef.current;
    const centre = centreRef.current;
    if (!root || !centre) return;
    return observeWithFrame(centre, () => {
      if (resizingRef.current) return;
      const o = openRef.current;
      const leftTool = o["left-top"];
      const rightTool = o["right-top"];
      const leftW = leftInnerRef.current?.offsetWidth ?? 0;
      const rightW = rightInnerRef.current?.offsetWidth ?? 0;
      let stripsW = 0;
      for (const strip of root.querySelectorAll<HTMLElement>("[data-dock-strip]")) stripsW += strip.offsetWidth;
      const next = clampColumnsToViewer({
        rootW: root.clientWidth,
        stripsW,
        leftW,
        rightW,
        leftOpen: leftTool !== null,
        rightOpen: rightTool !== null,
        leftMin: sideMin("left-top", leftTool),
        rightMin: sideMin("right-top", rightTool),
        minCentre: MIN_PREVIEW_W,
      });
      if (next === null) return;
      if (leftTool !== null && next.leftW !== leftW) dockStore.dispatch({ type: "resize", key: "leftW", value: next.leftW, persist: false });
      if (rightTool !== null && next.rightW !== rightW) dockStore.dispatch({ type: "resize", key: "rightW", value: next.rightW, persist: false });
    });
  }, []);

  useEffect(() => {
    const viewport = rootRef.current?.querySelector<HTMLElement>("[data-dock-viewport]");
    if (!viewport) return;
    return observeWithFrame(viewport, () => {
      if (resizingRef.current) return;
      const o = openRef.current;
      const height = layoutRef.current.sizes.bottomH;
      const next = clampBottomToViewer({
        viewportH: viewport.clientHeight,
        outerH: bottomOuterRef.current?.offsetHeight ?? height,
        bottomH: height,
        open: o["left-bottom"] !== null || o["right-bottom"] !== null,
        minViewport: MIN_PREVIEW_H,
        minBottom: DOCK_SIZE_LIMITS.bottomH.min,
      });
      if (next !== null) dockStore.dispatch({ type: "resize", key: "bottomH", value: next, persist: false });
    });
  }, []);

  const [menu, setMenu] = useState<DockMenuRequest | null>(null);
  const openMenu = useCallback((tool: DockToolId, trigger: HTMLButtonElement) => setMenu({ tool, trigger }), []);
  const closeMenu = useCallback(() => setMenu(null), []);

  const { buttonProps } = useDockDrag(rootRef, hasFile);

  const leftOpen = open["left-top"] !== null;
  const rightOpen = open["right-top"] !== null;
  const bothOpen = leftOpen && rightOpen;
  const areaOpen = open["left-bottom"] !== null || open["right-bottom"] !== null;
  const leftWidth = columnWidthCss(sideMin("left-top", shown["left-top"]), layout.sizes.leftW, bothOpen);
  const rightWidth = columnWidthCss(sideMin("right-top", shown["right-top"]), layout.sizes.rightW, bothOpen);

  return (
    <div ref={rootRef} data-dock-root="" className="flex-1 flex min-h-0 overflow-hidden">
      <Stripe side="left" layout={layout} hasFile={hasFile} buttonProps={buttonProps} onContextMenu={openMenu} />
      <SideColumn
        anchor="left-top"
        open={leftOpen}
        width={leftWidth}
        label={panelLabel("left-top", shown["left-top"])}
        outerRef={leftOuterRef}
        innerRef={leftInnerRef}
        containerRef={containerRefs["left-top"]}
        onSettled={handleSideSettled}
      />
      <ResizeHandle sizeKey="leftW" axis="x" open={leftOpen} begin={beginLeft} onActiveChange={handleResizeActive} />
      <div ref={centreRef} data-dock-centre="" className="flex-1 min-w-0 flex flex-col overflow-hidden">
        <div data-dock-viewer="" className="flex-1 min-h-0 overflow-hidden">
          {children}
        </div>
        <ResizeHandle sizeKey="bottomH" axis="y" open={areaOpen} begin={beginBottom} onActiveChange={handleResizeActive} />
        <BottomArea
          open={areaOpen}
          height={layout.sizes.bottomH}
          leftTool={shown["left-bottom"]}
          rightTool={shown["right-bottom"]}
          split={layout.sizes.bottomSplit}
          outerRef={bottomOuterRef}
          innerRef={bottomInnerRef}
          leftRef={containerRefs["left-bottom"]}
          rightRef={containerRefs["right-bottom"]}
          beginSplit={beginSplit}
          onResizeActive={handleResizeActive}
          onSettled={handleBottomSettled}
        />
      </div>
      <ResizeHandle sizeKey="rightW" axis="x" open={rightOpen} begin={beginRight} onActiveChange={handleResizeActive} />
      <SideColumn
        anchor="right-top"
        open={rightOpen}
        width={rightWidth}
        label={panelLabel("right-top", shown["right-top"])}
        outerRef={rightOuterRef}
        innerRef={rightInnerRef}
        containerRef={containerRefs["right-top"]}
        onSettled={handleSideSettled}
      />
      <Stripe side="right" layout={layout} hasFile={hasFile} buttonProps={buttonProps} onContextMenu={openMenu} />
      <ToolHostRoot tools={tools} containers={containers} layout={layout} open={open} shown={shown} hasFile={hasFile} fileKey={fileKey} />
      <DragOverlay />
      <DockOnboarding rootRef={rootRef} />
      <DockContextMenu request={menu} onClose={closeMenu} />
      <LiveRegion />
    </div>
  );
}
