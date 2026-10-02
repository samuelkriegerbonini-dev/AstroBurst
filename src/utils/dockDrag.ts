import {
  DOCK_ANCHORS,
  DOCK_SIZE_LIMITS,
  DOCK_TOOL_META,
  anchorOpenTool,
  anchorSide,
  columnMin,
  columnRenderWidth,
  dockReducer,
  type DockAnchor,
  type DockLayout,
  type DockSide,
  type DockToolId,
  type SideAnchor,
} from "./dockLayout";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface StripGroupGeometry {
  anchor: DockAnchor;
  rect: Rect;
  items: { tool: DockToolId; rect: Rect }[];
}

export interface DockGeometry {
  dock: Rect;
  leftStrip: Rect;
  rightStrip: Rect;
  groups: StripGroupGeometry[];
}

export interface DragSource {
  tool: DockToolId;
  from: DockAnchor;
  fromIndex: number;
}

export interface DropTarget {
  anchor: DockAnchor;
  index: number;
}

export const DRAG_THRESHOLD_PX = 5;
export const STRIP_HIT_SLOP_PX = 16;
export const GROUP_HIT_SLOP_PX = 16;
export const EMPTY_GROUP_SLOT_PX = 24;

const INSERT_MARKER_W = 30;
const INSERT_MARKER_H = 2;

export function dragStarted(start: Point, current: Point, threshold: number = DRAG_THRESHOLD_PX): boolean {
  return Math.hypot(current.x - start.x, current.y - start.y) >= threshold;
}

export function insertionIndex(pointerY: number, items: Rect[], draggedIndex: number | null): number {
  const above = items.filter((r) => r.y + r.h / 2 < pointerY).length;
  return draggedIndex !== null && draggedIndex < above ? above - 1 : above;
}

function contains(r: Rect, p: Point): boolean {
  return p.x >= r.x && p.x < r.x + r.w && p.y >= r.y && p.y < r.y + r.h;
}

function isTopGroup(anchor: DockAnchor): boolean {
  return anchor === "left-top" || anchor === "right-top";
}

function stripOf(side: DockSide, g: DockGeometry): Rect {
  return side === "left" ? g.leftStrip : g.rightStrip;
}

function stripHitRect(side: DockSide, g: DockGeometry): Rect {
  const s = stripOf(side, g);
  const w = s.w + STRIP_HIT_SLOP_PX;
  return side === "left" ? { x: s.x, y: s.y, w, h: s.h } : { x: s.x - STRIP_HIT_SLOP_PX, y: s.y, w, h: s.h };
}

function groupOf(anchor: DockAnchor, g: DockGeometry): StripGroupGeometry | undefined {
  return g.groups.find((group) => group.anchor === anchor);
}

export function emptyGroupSlotRect(anchor: DockAnchor, g: DockGeometry): Rect {
  const strip = stripOf(anchorSide(anchor), g);
  const box = groupOf(anchor, g)?.rect ?? strip;
  const y = isTopGroup(anchor) ? box.y : box.y + box.h - EMPTY_GROUP_SLOT_PX;
  return { x: strip.x, y, w: strip.w, h: EMPTY_GROUP_SLOT_PX };
}

function groupHitRect(anchor: DockAnchor, g: DockGeometry): Rect {
  const group = groupOf(anchor, g);
  return group && group.items.length > 0 ? group.rect : emptyGroupSlotRect(anchor, g);
}

function prefersTopGroup(y: number, top: Rect, bottom: Rect): boolean {
  if (y >= top.y && y < top.y + top.h) return true;
  if (y >= bottom.y && y < bottom.y + bottom.h) return false;
  const nearTop = y <= top.y + top.h + GROUP_HIT_SLOP_PX;
  const nearBottom = y >= bottom.y - GROUP_HIT_SLOP_PX;
  if (nearTop !== nearBottom) return nearTop;
  return Math.abs(y - (top.y + top.h)) <= Math.abs(bottom.y - y);
}

export function dropTargetAt(p: Point, g: DockGeometry, src: DragSource): DropTarget | null {
  const side: DockSide | null = contains(stripHitRect("left", g), p) ? "left" : contains(stripHitRect("right", g), p) ? "right" : null;
  if (side === null) return null;
  const top: DockAnchor = side === "left" ? "left-top" : "right-top";
  const bottom: DockAnchor = side === "left" ? "left-bottom" : "right-bottom";
  const anchor = prefersTopGroup(p.y, groupHitRect(top, g), groupHitRect(bottom, g)) ? top : bottom;
  const items = (groupOf(anchor, g)?.items ?? []).map((item) => item.rect);
  return { anchor, index: insertionIndex(p.y, items, src.from === anchor ? src.fromIndex : null) };
}

export function insertionMarkerRect(t: DropTarget, g: DockGeometry, src: DragSource): Rect {
  const strip = stripOf(anchorSide(t.anchor), g);
  const items = (groupOf(t.anchor, g)?.items ?? [])
    .filter((item) => !(t.anchor === src.from && item.tool === src.tool))
    .map((item) => item.rect);
  let y: number;
  if (items.length === 0) {
    const slot = emptyGroupSlotRect(t.anchor, g);
    y = slot.y + slot.h / 2;
  } else if (t.index <= 0) {
    y = items[0].y;
  } else if (t.index >= items.length) {
    const last = items[items.length - 1];
    y = last.y + last.h;
  } else {
    const prev = items[t.index - 1];
    y = (prev.y + prev.h + items[t.index].y) / 2;
  }
  return { x: strip.x + (strip.w - INSERT_MARKER_W) / 2, y: y - INSERT_MARKER_H / 2, w: INSERT_MARKER_W, h: INSERT_MARKER_H };
}

function sideColumnWidth(layout: DockLayout, anchor: SideAnchor, dockW: number): number {
  if (layout.active[anchor] === null) return 0;
  const key = anchor === "left-top" ? "leftW" : "rightW";
  const bothOpen = layout.active["left-top"] !== null && layout.active["right-top"] !== null;
  return columnRenderWidth({ size: layout.sizes[key], min: columnMin(layout, anchor), max: DOCK_SIZE_LIMITS[key].max, dockW, bothOpen });
}

function openWithFile(layout: DockLayout, hasFile: boolean): DockLayout {
  if (hasFile) return layout;
  const active = { ...layout.active };
  for (const anchor of DOCK_ANCHORS) active[anchor] = anchorOpenTool(layout, anchor, false);
  return { ...layout, active };
}

export function dropPreviewRect(t: DropTarget, g: DockGeometry, layout: DockLayout, src: DragSource, hasFile: boolean = true): Rect | null {
  if (t.anchor === src.from) return null;
  if (DOCK_TOOL_META[src.tool].needsFile && !hasFile) return null;
  const next = openWithFile(dockReducer(layout, { type: "move", tool: src.tool, anchor: t.anchor, index: t.index }), hasFile);
  const leftColW = sideColumnWidth(next, "left-top", g.dock.w);
  const rightColW = sideColumnWidth(next, "right-top", g.dock.w);
  if (t.anchor === "left-top") return { x: g.leftStrip.x + g.leftStrip.w, y: g.dock.y, w: leftColW, h: g.dock.h };
  if (t.anchor === "right-top") return { x: g.rightStrip.x - rightColW, y: g.dock.y, w: rightColW, h: g.dock.h };
  const leftTool = next.active["left-bottom"];
  const rightTool = next.active["right-bottom"];
  const split = bottomSplitLayout({
    areaW: g.dock.w - g.leftStrip.w - g.rightStrip.w - leftColW - rightColW,
    leftOpen: leftTool !== null,
    rightOpen: rightTool !== null,
    split: next.sizes.bottomSplit,
    bottomH: next.sizes.bottomH,
    leftMin: leftTool === null ? 0 : DOCK_TOOL_META[leftTool].minWidth,
    rightMin: rightTool === null ? 0 : DOCK_TOOL_META[rightTool].minWidth,
  });
  const half = t.anchor === "left-bottom" ? split.left : split.right;
  if (half === null) return null;
  const areaX = g.leftStrip.x + g.leftStrip.w + leftColW;
  return { x: areaX + half.x, y: g.dock.y + g.dock.h - split.total + half.y, w: half.w, h: half.h };
}

export interface BottomSplit {
  mode: "none" | "single" | "split";
  left: Rect | null;
  right: Rect | null;
  total: number;
}

export function bottomSplitLayout({ areaW, leftOpen, rightOpen, split, bottomH, leftMin, rightMin, handleW = 0 }: {
  areaW: number;
  leftOpen: boolean;
  rightOpen: boolean;
  split: number;
  bottomH: number;
  leftMin: number;
  rightMin: number;
  handleW?: number;
}): BottomSplit {
  if (!leftOpen && !rightOpen) return { mode: "none", left: null, right: null, total: 0 };
  if (leftOpen !== rightOpen) {
    const full: Rect = { x: 0, y: 0, w: Math.max(0, areaW), h: bottomH };
    return { mode: "single", left: leftOpen ? full : null, right: rightOpen ? full : null, total: bottomH };
  }
  const inner = Math.max(0, areaW - handleW);
  let leftW = Math.round(inner * split);
  if (inner >= leftMin + rightMin) leftW = Math.min(Math.max(leftW, leftMin), inner - rightMin);
  return {
    mode: "split",
    left: { x: 0, y: 0, w: leftW, h: bottomH },
    right: { x: leftW + handleW, y: 0, w: inner - leftW, h: bottomH },
    total: bottomH,
  };
}

function takeFrom(width: number, floor: number, amount: number): number {
  return Math.max(0, Math.min(amount, width - floor));
}

export function shrinkColumnsForDeficit({ leftW, rightW, leftOpen, rightOpen, leftMin, rightMin, deficit }: {
  leftW: number;
  rightW: number;
  leftOpen: boolean;
  rightOpen: boolean;
  leftMin: number;
  rightMin: number;
  deficit: number;
}): { leftW: number; rightW: number } {
  let left = leftW;
  let right = rightW;
  let rest = Math.ceil(Math.max(0, deficit));
  if (leftOpen && rightOpen) {
    if (left > right) {
      const t = takeFrom(left, Math.max(leftMin, right), rest);
      left -= t;
      rest -= t;
    } else if (right > left) {
      const t = takeFrom(right, Math.max(rightMin, left), rest);
      right -= t;
      rest -= t;
    }
    const fromLeft = takeFrom(left, leftMin, Math.ceil(rest / 2));
    const fromRight = takeFrom(right, rightMin, rest - fromLeft);
    left -= fromLeft + takeFrom(left - fromLeft, leftMin, rest - fromLeft - fromRight);
    right -= fromRight;
  } else if (leftOpen) {
    left -= takeFrom(left, leftMin, rest);
  } else if (rightOpen) {
    right -= takeFrom(right, rightMin, rest);
  }
  return { leftW: left, rightW: right };
}

export function clampColumnsToViewer({ rootW, stripsW, leftW, rightW, leftOpen, rightOpen, leftMin, rightMin, minCentre }: {
  rootW: number;
  stripsW: number;
  leftW: number;
  rightW: number;
  leftOpen: boolean;
  rightOpen: boolean;
  leftMin: number;
  rightMin: number;
  minCentre: number;
}): { leftW: number; rightW: number } | null {
  if (!leftOpen && !rightOpen) return null;
  const settledCentre = rootW - stripsW - (leftOpen ? leftW : 0) - (rightOpen ? rightW : 0);
  const deficit = minCentre - settledCentre;
  if (deficit <= 1) return null;
  const next = shrinkColumnsForDeficit({ leftW, rightW, leftOpen, rightOpen, leftMin, rightMin, deficit });
  return next.leftW === leftW && next.rightW === rightW ? null : next;
}

export function clampBottomToViewer({ viewportH, outerH, bottomH, open, minViewport, minBottom }: {
  viewportH: number;
  outerH: number;
  bottomH: number;
  open: boolean;
  minViewport: number;
  minBottom: number;
}): number | null {
  if (!open) return null;
  const deficit = minViewport - (viewportH + outerH - bottomH);
  if (deficit <= 1) return null;
  const next = Math.max(minBottom, bottomH - Math.ceil(deficit));
  return next === bottomH ? null : next;
}
