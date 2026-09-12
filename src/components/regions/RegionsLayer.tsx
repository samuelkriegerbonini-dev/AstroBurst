import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import type { Region, RegionShape, RegionTool } from "../../shared/types";
import type { ViewerTransform } from "../../utils/pixelMapping";
import {
  isRegionMappingUsable,
  regionPointToScreen,
  regionToleranceImagePx,
  resolveRegionHost,
  screenToRegionPoint,
  type RegionMapping,
} from "../../utils/regionCoords";
import {
  createShape,
  shapeHandles,
  moveHandle,
  translateShape,
  hitTest,
  shapeBounds,
  shapeOutline,
  type Pt,
  type Handle,
} from "../../utils/regionGeometry";
import { regionStore } from "../../utils/regionStore";
import { DEFAULT_REGION_PROPS } from "../../utils/regionPersistence";
import { useRegionDoc, useRegionTool, useRegionDraft } from "../../hooks/useRegionStore";
import { useRegionKey } from "../../hooks/useRegionKey";
import { generateId } from "../../utils/format";

interface RegionsLayerProps {
  containerRef: RefObject<HTMLDivElement | null>;
  transform: ViewerTransform;
  renderW: number;
  renderH: number;
  fitsW: number;
  fitsH: number;
  enabled: boolean;
}

const DEFAULT_COLOR = "#7dd3fc";
const DEFAULT_WIDTH = 1.2;
const DRAFT_COLOR = "#fbbf24";
const SELECT_COLOR = "#ffffff";
const HANDLE_SIZE = 8;
const HANDLE_TOLERANCE_PX = 6;
const DRAG_THRESHOLD_PX = 2;
const DASH: number[] = [4, 3];

type DragState =
  | { kind: "draw"; tool: Exclude<RegionTool, "none" | "select" | "polygon">; start: Pt; startScreen: Pt; moved: boolean }
  | { kind: "move"; id: string; start: Pt; shapeStart: RegionShape }
  | { kind: "resize"; id: string; handleId: string; shapeStart: RegionShape };

function isEditableTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  const tag = t.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || t.isContentEditable;
}

function cursorForTool(tool: RegionTool): string {
  if (tool === "none") return "inherit";
  if (tool === "select") return "default";
  return "crosshair";
}

function RegionsLayer({ containerRef, transform, renderW, renderH, fitsW, fitsH, enabled }: RegionsLayerProps) {
  const regionKey = useRegionKey();
  const fileKey = enabled ? regionKey : null;
  const doc = useRegionDoc(fileKey);
  const tool = useRegionTool();
  const draft = useRegionDraft(fileKey);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const dragRef = useRef<DragState | null>(null);
  const hoverRef = useRef<Pt | null>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [hoverTick, setHoverTick] = useState(0);
  const [hostEl, setHostEl] = useState<HTMLElement | null>(null);

  const mapping: RegionMapping = { transform, renderW, renderH, fitsW, fitsH };
  const active = enabled && fileKey !== null && isRegionMappingUsable(mapping);
  const mappingRef = useRef(mapping);
  mappingRef.current = mapping;

  const attachCanvas = useCallback(
    (el: HTMLCanvasElement | null) => {
      canvasRef.current = el;
      setHostEl(resolveRegionHost(containerRef.current, el));
    },
    [containerRef],
  );

  useLayoutEffect(() => {
    if (!active || !hostEl) return;
    const measure = () => setSize({ w: hostEl.clientWidth, h: hostEl.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(hostEl);
    return () => ro.disconnect();
  }, [hostEl, active]);

  const toScreen = useCallback((p: Pt): Pt => regionPointToScreen(p, mappingRef.current), []);

  const toImage = useCallback(
    (clientX: number, clientY: number): Pt | null => {
      const rect = (containerRef.current ?? hostEl)?.getBoundingClientRect();
      if (!rect) return null;
      return screenToRegionPoint(clientX, clientY, rect, mappingRef.current);
    },
    [containerRef, hostEl],
  );

  const tolerance = useCallback((): number => regionToleranceImagePx(HANDLE_TOLERANCE_PX, mappingRef.current), []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !active) return;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(size.w * dpr));
    const h = Math.max(1, Math.round(size.h * dpr));
    if (canvas.width !== w) canvas.width = w;
    if (canvas.height !== h) canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.w, size.h);
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    ctx.font = "10px 'JetBrains Mono', monospace";
    ctx.textBaseline = "bottom";

    const strokePolylines = (lines: Pt[][]) => {
      for (const line of lines) {
        if (line.length === 0) continue;
        ctx.beginPath();
        const p0 = toScreen(line[0]);
        ctx.moveTo(p0.x, p0.y);
        for (let i = 1; i < line.length; i++) {
          const p = toScreen(line[i]);
          ctx.lineTo(p.x, p.y);
        }
        ctx.stroke();
      }
    };

    const paintRegion = (r: Region, selected: boolean) => {
      const color = r.props.color ?? DEFAULT_COLOR;
      const dashed = r.props.dash === true || !r.props.include;
      ctx.strokeStyle = color;
      ctx.lineWidth = r.props.width ?? DEFAULT_WIDTH;
      ctx.setLineDash(dashed ? DASH : []);
      strokePolylines(shapeOutline(r.shape));
      ctx.setLineDash([]);
      if (r.props.text) {
        const b = shapeBounds(r.shape);
        const p = toScreen({ x: b.x1, y: b.y0 });
        ctx.fillStyle = color;
        ctx.fillText(r.props.text, p.x + 4, p.y - 2);
      }
      if (selected) {
        ctx.strokeStyle = SELECT_COLOR;
        ctx.lineWidth = 1;
        ctx.setLineDash([2, 2]);
        strokePolylines(shapeOutline(r.shape));
        ctx.setLineDash([]);
        for (const hnd of shapeHandles(r.shape)) {
          const p = toScreen(hnd);
          ctx.fillStyle = hnd.id === "rot" ? DRAFT_COLOR : SELECT_COLOR;
          ctx.strokeStyle = "#111827";
          ctx.lineWidth = 1;
          ctx.fillRect(p.x - HANDLE_SIZE / 2, p.y - HANDLE_SIZE / 2, HANDLE_SIZE, HANDLE_SIZE);
          ctx.strokeRect(p.x - HANDLE_SIZE / 2, p.y - HANDLE_SIZE / 2, HANDLE_SIZE, HANDLE_SIZE);
        }
      }
    };

    for (const r of doc.regions) {
      if (r.id !== doc.selectedId) paintRegion(r, false);
    }
    const selected = doc.regions.find((r) => r.id === doc.selectedId);
    if (selected) paintRegion(selected, true);

    if (draft) {
      ctx.strokeStyle = DRAFT_COLOR;
      ctx.lineWidth = DEFAULT_WIDTH;
      ctx.setLineDash(DASH);
      if (draft.shape === "polygon") {
        const pts = draft.points.map(([x, y]) => ({ x, y }));
        const hover = hoverRef.current;
        strokePolylines([hover ? [...pts, hover] : pts]);
        ctx.setLineDash([]);
        ctx.fillStyle = DRAFT_COLOR;
        for (const p of pts) {
          const s = toScreen(p);
          ctx.fillRect(s.x - 2, s.y - 2, 4, 4);
        }
      } else {
        strokePolylines(shapeOutline(draft));
      }
      ctx.setLineDash([]);
    }
  }, [active, size, transform, renderW, renderH, fitsW, fitsH, doc, draft, toScreen, hoverTick]);

  useEffect(() => {
    if (!fileKey) return;
    return () => regionStore.clearDraft(fileKey);
  }, [fileKey]);

  const commitDraft = useCallback(
    (shape: RegionShape) => {
      if (!fileKey) return;
      const id = generateId();
      regionStore.add(fileKey, { id, shape, props: { ...DEFAULT_REGION_PROPS }, backgroundId: null });
      regionStore.select(fileKey, id);
      regionStore.clearDraft(fileKey);
      regionStore.setTool("select");
    },
    [fileKey],
  );

  const closePolygon = useCallback(() => {
    if (!fileKey) return;
    const d = regionStore.getDraftFor(fileKey);
    if (!d || d.shape !== "polygon") return;
    if (d.points.length >= 3) commitDraft({ shape: "polygon", points: d.points.slice() });
    else regionStore.clearDraft(fileKey);
  }, [fileKey, commitDraft]);

  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      if (e.button !== 0 || !fileKey || tool === "none") return;
      const pt = toImage(e.clientX, e.clientY);
      const d = regionStore.getDoc(fileKey);
      if (tool === "select") {
        if (!pt) {
          regionStore.select(fileKey, null);
          return;
        }
        const tol = tolerance();
        const selected = d.regions.find((r) => r.id === d.selectedId);
        if (selected) {
          let best: Handle | null = null;
          let bestD = Infinity;
          for (const h of shapeHandles(selected.shape)) {
            const dd = Math.hypot(h.x - pt.x, h.y - pt.y);
            if (dd <= tol && dd < bestD) {
              best = h;
              bestD = dd;
            }
          }
          if (best) {
            e.stopPropagation();
            dragRef.current = { kind: "resize", id: selected.id, handleId: best.id, shapeStart: selected.shape };
            e.currentTarget.setPointerCapture(e.pointerId);
            return;
          }
        }
        for (let i = d.regions.length - 1; i >= 0; i--) {
          const r = d.regions[i];
          if (hitTest(r.shape, pt, tol)) {
            e.stopPropagation();
            regionStore.select(fileKey, r.id);
            dragRef.current = { kind: "move", id: r.id, start: pt, shapeStart: r.shape };
            e.currentTarget.setPointerCapture(e.pointerId);
            return;
          }
        }
        regionStore.select(fileKey, null);
        return;
      }
      if (!pt) return;
      e.stopPropagation();
      if (tool === "polygon") {
        const cur = regionStore.getDraftFor(fileKey);
        if (cur && cur.shape === "polygon") {
          const last = cur.points[cur.points.length - 1];
          if (last && Math.hypot(last[0] - pt.x, last[1] - pt.y) <= tolerance()) return;
          regionStore.setDraft(fileKey, { shape: "polygon", points: [...cur.points, [pt.x, pt.y]] });
        } else {
          regionStore.setDraft(fileKey, { shape: "polygon", points: [[pt.x, pt.y]] });
        }
        return;
      }
      regionStore.setDraft(fileKey, createShape(tool, pt, pt));
      dragRef.current = { kind: "draw", tool, start: pt, startScreen: { x: e.clientX, y: e.clientY }, moved: false };
      e.currentTarget.setPointerCapture(e.pointerId);
    },
    [fileKey, tool, toImage, tolerance],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      if (!fileKey || tool === "none") return;
      const drag = dragRef.current;
      const pt = toImage(e.clientX, e.clientY);
      if (!drag) {
        if (tool === "polygon" && regionStore.getDraftFor(fileKey)) {
          hoverRef.current = pt;
          setHoverTick((t) => t + 1);
        }
        if (tool === "select" && pt) {
          const d = regionStore.getDoc(fileKey);
          const tol = tolerance();
          const selected = d.regions.find((r) => r.id === d.selectedId);
          let cursor = "default";
          const hnd = selected ? shapeHandles(selected.shape).find((h) => Math.hypot(h.x - pt.x, h.y - pt.y) <= tol) : undefined;
          if (hnd) cursor = hnd.cursor;
          else if (d.regions.some((r) => hitTest(r.shape, pt, tol))) cursor = "move";
          e.currentTarget.style.cursor = cursor;
        }
        return;
      }
      if (!pt) return;
      if (drag.kind === "draw") {
        if (!drag.moved && Math.hypot(e.clientX - drag.startScreen.x, e.clientY - drag.startScreen.y) >= DRAG_THRESHOLD_PX) {
          drag.moved = true;
        }
        regionStore.setDraft(fileKey, createShape(drag.tool, drag.start, pt));
      } else if (drag.kind === "move") {
        regionStore.update(fileKey, drag.id, { shape: translateShape(drag.shapeStart, pt.x - drag.start.x, pt.y - drag.start.y) });
      } else {
        regionStore.update(fileKey, drag.id, { shape: moveHandle(drag.shapeStart, drag.handleId, pt) });
      }
    },
    [fileKey, tool, toImage, tolerance],
  );

  const handlePointerUp = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      if (e.button !== 0) return;
      const drag = dragRef.current;
      dragRef.current = null;
      if (!drag || !fileKey) return;
      e.stopPropagation();
      if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
      if (drag.kind !== "draw") return;
      const d = regionStore.getDraftFor(fileKey);
      if (d && (drag.moved || drag.tool === "point")) commitDraft(d);
      else regionStore.clearDraft(fileKey);
    },
    [fileKey, commitDraft],
  );

  const handleDoubleClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      e.stopPropagation();
      if (tool === "polygon") closePolygon();
    },
    [tool, closePolygon],
  );

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return;
      const key = fileKey;
      if (!key) return;
      if (e.key === "Delete" || e.key === "Backspace") {
        const d = regionStore.getDoc(key);
        if (d.selectedId) {
          regionStore.remove(key, d.selectedId);
          e.preventDefault();
        }
        return;
      }
      if (e.key === "Enter") {
        if (regionStore.getDraftFor(key)?.shape === "polygon") {
          closePolygon();
          e.preventDefault();
        }
        return;
      }
      if (e.key === "Escape") {
        if (regionStore.getDraftFor(key)) {
          regionStore.clearDraft(key);
          dragRef.current = null;
        } else if (regionStore.getDoc(key).selectedId) {
          regionStore.select(key, null);
        } else if (regionStore.getTool() !== "none") {
          regionStore.setTool("none");
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, fileKey, closePolygon]);

  useEffect(() => {
    if (tool !== "polygon") hoverRef.current = null;
  }, [tool]);

  if (!active) return null;

  return (
    <canvas
      ref={attachCanvas}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onDoubleClick={handleDoubleClick}
      onClick={(e) => {
        if (e.button === 0 && tool !== "none") e.stopPropagation();
      }}
      style={{
        position: "absolute",
        inset: 0,
        width: "100%",
        height: "100%",
        pointerEvents: tool === "none" ? "none" : "auto",
        cursor: cursorForTool(tool),
        zIndex: 5,
      }}
    />
  );
}

export default memo(RegionsLayer);
