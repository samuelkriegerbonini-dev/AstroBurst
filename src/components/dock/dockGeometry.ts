import {
  DOCK_ANCHORS,
  isDockToolId,
  otherBottom,
  type BottomAnchor,
  type DockAnchor,
  type DockSide,
  type DockSizes,
  type DockToolId,
  type SideAnchor,
} from "../../utils/dockLayout";
import type { DockGeometry, Rect, StripGroupGeometry } from "../../utils/dockDrag";
import { dockStore } from "../../hooks/useDockLayout";
import type { ResizeSession } from "./ResizeHandle";

function toRect(r: DOMRect): Rect {
  return { x: r.left, y: r.top, w: r.width, h: r.height };
}

function isDockAnchor(value: string | undefined): value is DockAnchor {
  return value !== undefined && (DOCK_ANCHORS as readonly string[]).includes(value);
}

function stripRect(root: HTMLElement, side: DockSide, dock: Rect): Rect {
  const strip = root.querySelector<HTMLElement>(`[data-dock-strip="${side}"]`);
  if (strip) return toRect(strip.getBoundingClientRect());
  return { x: side === "left" ? dock.x : dock.x + dock.w, y: dock.y, w: 0, h: dock.h };
}

function groupGeometry(el: HTMLElement, anchor: DockAnchor): StripGroupGeometry {
  const items: { tool: DockToolId; rect: Rect }[] = [];
  for (const button of el.querySelectorAll<HTMLElement>("[data-tool-id]")) {
    const tool = button.dataset.toolId;
    if (isDockToolId(tool)) items.push({ tool, rect: toRect(button.getBoundingClientRect()) });
  }
  return { anchor, rect: toRect(el.getBoundingClientRect()), items };
}

export function measureDockGeometry(root: HTMLElement): DockGeometry {
  const dock = toRect(root.getBoundingClientRect());
  const groups: StripGroupGeometry[] = [];
  for (const el of root.querySelectorAll<HTMLElement>("[data-dock-group]")) {
    const anchor = el.dataset.dockGroup;
    if (isDockAnchor(anchor) && !groups.some((g) => g.anchor === anchor)) groups.push(groupGeometry(el, anchor));
  }
  return { dock, leftStrip: stripRect(root, "left", dock), rightStrip: stripRect(root, "right", dock), groups };
}

export function stripShowsFile(root: HTMLElement): boolean {
  return root.querySelector('[data-tool-id][aria-disabled="true"]') === null;
}

export type AnchorTools = Record<DockAnchor, DockToolId | null>;

function isBottom(anchor: DockAnchor): anchor is BottomAnchor {
  return anchor === "left-bottom" || anchor === "right-bottom";
}

export function nextShown(shown: AnchorTools, open: AnchorTools): AnchorTools {
  let next = shown;
  for (const anchor of DOCK_ANCHORS) {
    const closedBesideOpenHalf = isBottom(anchor) && open[otherBottom(anchor)] !== null;
    const value = open[anchor] ?? (closedBesideOpenHalf ? null : shown[anchor]);
    if (value !== next[anchor]) next = { ...next, [anchor]: value };
  }
  return next;
}

export function settleSide(shown: AnchorTools, open: AnchorTools, anchor: SideAnchor): AnchorTools {
  return open[anchor] !== null || shown[anchor] === null ? shown : { ...shown, [anchor]: null };
}

export function settleBottom(shown: AnchorTools, open: AnchorTools): AnchorTools {
  if (open["left-bottom"] !== null || open["right-bottom"] !== null) return shown;
  if (shown["left-bottom"] === null && shown["right-bottom"] === null) return shown;
  return { ...shown, "left-bottom": null, "right-bottom": null };
}

export interface ResizeDrag {
  sizeKey: keyof DockSizes;
  axis: "x" | "y";
  origin: number;
  handle: HTMLElement;
  session: ResizeSession;
  onActiveChange(active: boolean): void;
}

export function startResizeDrag({ sizeKey, axis, origin, handle, session, onActiveChange }: ResizeDrag): void {
  const max = Math.max(session.min, session.max);
  let value = session.start;
  let moved = false;
  document.body.style.cursor = axis === "x" ? "col-resize" : "row-resize";
  document.body.style.userSelect = "none";
  handle.dataset.dragging = "true";
  for (const el of session.frozen) if (el) el.style.transition = "none";
  onActiveChange(true);
  const onMove = (ev: MouseEvent) => {
    const pos = axis === "x" ? ev.clientX : ev.clientY;
    const next = Math.round(Math.max(session.min, Math.min(max, session.start + session.direction * (pos - origin))));
    if (!moved && (pos === origin || next === value)) return;
    moved = true;
    value = next;
    session.apply(value);
  };
  const onUp = () => {
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    delete handle.dataset.dragging;
    for (const el of session.frozen) if (el) el.style.transition = "";
    onActiveChange(false);
    if (moved) dockStore.dispatch({ type: "resize", key: sizeKey, value: session.commit(value) });
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}
