import type { RegionShape, RegionShapeKind } from "../shared/types";
import { REGION_SHAPE_KINDS } from "../shared/types";

export interface Pt {
  x: number;
  y: number;
}

export interface Handle {
  id: string;
  x: number;
  y: number;
  cursor: string;
}

export const MIN_RADIUS = 0.5;
export const MIN_SIZE = 1;
export const MIN_ANNULUS_GAP = 0.5;

const OUTLINE_SEGMENTS = 64;
const POINT_CROSS_HALF = 3;
const ROT_HANDLE_GAP = 4;
const SNAP_EPS = 1e-12;

function snapTrig(v: number): number {
  if (Math.abs(v) < SNAP_EPS) return 0;
  if (Math.abs(v - 1) < SNAP_EPS) return 1;
  if (Math.abs(v + 1) < SNAP_EPS) return -1;
  return v;
}

interface Basis {
  cos: number;
  sin: number;
}

function basis(angleDeg: number): Basis {
  const a = (angleDeg * Math.PI) / 180;
  return { cos: snapTrig(Math.cos(a)), sin: snapTrig(Math.sin(a)) };
}

function localToIndex(u: number, v: number, b: Basis): Pt {
  return { x: u * b.cos - v * b.sin, y: u * b.sin + v * b.cos };
}

function indexToLocal(dx: number, dy: number, b: Basis): { u: number; v: number } {
  return { u: dx * b.cos + dy * b.sin, v: -dx * b.sin + dy * b.cos };
}

export function normalizeAngle(deg: number): number {
  const n = ((deg % 360) + 360) % 360;
  return n === 0 ? 0 : n;
}

function dist(a: Pt, b: Pt): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

function segmentDistance(p: Pt, a: Pt, b: Pt): number {
  const abx = b.x - a.x;
  const aby = b.y - a.y;
  const len2 = abx * abx + aby * aby;
  if (len2 === 0) return dist(p, a);
  const t = Math.max(0, Math.min(1, ((p.x - a.x) * abx + (p.y - a.y) * aby) / len2));
  return dist(p, { x: a.x + t * abx, y: a.y + t * aby });
}

function polylineDistance(p: Pt, pts: Pt[]): number {
  let best = Infinity;
  for (let i = 0; i + 1 < pts.length; i++) {
    best = Math.min(best, segmentDistance(p, pts[i], pts[i + 1]));
  }
  return best;
}

export function createShape(kind: Exclude<RegionShapeKind, "polygon">, start: Pt, end: Pt): RegionShape {
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const d = Math.hypot(dx, dy);
  switch (kind) {
    case "circle":
      return { shape: "circle", x: start.x, y: start.y, r: Math.max(MIN_RADIUS, d) };
    case "ellipse":
      return {
        shape: "ellipse",
        x: start.x,
        y: start.y,
        rx: Math.max(MIN_RADIUS, Math.abs(dx)),
        ry: Math.max(MIN_RADIUS, Math.abs(dy)),
        angle: 0,
      };
    case "box":
      return {
        shape: "box",
        x: (start.x + end.x) / 2,
        y: (start.y + end.y) / 2,
        width: Math.max(MIN_SIZE, Math.abs(dx)),
        height: Math.max(MIN_SIZE, Math.abs(dy)),
        angle: 0,
      };
    case "annulus": {
      const outer = Math.max(2 * MIN_ANNULUS_GAP, d);
      return { shape: "annulus", x: start.x, y: start.y, r_inner: outer / 2, r_outer: outer };
    }
    case "line":
      return { shape: "line", x1: start.x, y1: start.y, x2: end.x, y2: end.y };
    case "point":
      return { shape: "point", x: start.x, y: start.y };
  }
}

function centreOf(shape: RegionShape): Pt {
  switch (shape.shape) {
    case "polygon": {
      const n = shape.points.length;
      let sx = 0;
      let sy = 0;
      for (const [px, py] of shape.points) {
        sx += px;
        sy += py;
      }
      return n === 0 ? { x: 0, y: 0 } : { x: sx / n, y: sy / n };
    }
    case "line":
      return { x: (shape.x1 + shape.x2) / 2, y: (shape.y1 + shape.y2) / 2 };
    default:
      return { x: shape.x, y: shape.y };
  }
}

export function shapeHandles(shape: RegionShape): Handle[] {
  switch (shape.shape) {
    case "circle":
      return [{ id: "r", x: shape.x + shape.r, y: shape.y, cursor: "ew-resize" }];
    case "ellipse": {
      const b = basis(shape.angle);
      const rx = localToIndex(shape.rx, 0, b);
      const ry = localToIndex(0, shape.ry, b);
      const rot = localToIndex(shape.rx + ROT_HANDLE_GAP, 0, b);
      return [
        { id: "rx", x: shape.x + rx.x, y: shape.y + rx.y, cursor: "ew-resize" },
        { id: "ry", x: shape.x + ry.x, y: shape.y + ry.y, cursor: "ns-resize" },
        { id: "rot", x: shape.x + rot.x, y: shape.y + rot.y, cursor: "grab" },
      ];
    }
    case "box": {
      const b = basis(shape.angle);
      const hw = shape.width / 2;
      const hh = shape.height / 2;
      const mk = (id: string, u: number, v: number, cursor: string): Handle => {
        const p = localToIndex(u, v, b);
        return { id, x: shape.x + p.x, y: shape.y + p.y, cursor };
      };
      return [
        mk("n", 0, -hh, "ns-resize"),
        mk("s", 0, hh, "ns-resize"),
        mk("e", hw, 0, "ew-resize"),
        mk("w", -hw, 0, "ew-resize"),
        mk("ne", hw, -hh, "nesw-resize"),
        mk("nw", -hw, -hh, "nwse-resize"),
        mk("se", hw, hh, "nwse-resize"),
        mk("sw", -hw, hh, "nesw-resize"),
        mk("rot", hw + ROT_HANDLE_GAP, 0, "grab"),
      ];
    }
    case "annulus":
      return [
        { id: "inner", x: shape.x + shape.r_inner, y: shape.y, cursor: "ew-resize" },
        { id: "outer", x: shape.x + shape.r_outer, y: shape.y, cursor: "ew-resize" },
      ];
    case "polygon":
      return shape.points.map(([px, py], i) => ({ id: `v${i}`, x: px, y: py, cursor: "move" }));
    case "line":
      return [
        { id: "p1", x: shape.x1, y: shape.y1, cursor: "move" },
        { id: "p2", x: shape.x2, y: shape.y2, cursor: "move" },
      ];
    case "point":
      return [];
  }
}

function rotationAngle(centre: Pt, pt: Pt): number {
  return normalizeAngle((Math.atan2(pt.y - centre.y, pt.x - centre.x) * 180) / Math.PI);
}

export function moveHandle(shape: RegionShape, handleId: string, pt: Pt): RegionShape {
  switch (shape.shape) {
    case "circle": {
      if (handleId !== "r") return shape;
      return { ...shape, r: Math.max(MIN_RADIUS, dist(pt, shape)) };
    }
    case "ellipse": {
      if (handleId === "rot") return { ...shape, angle: rotationAngle(shape, pt) };
      const b = basis(shape.angle);
      const { u, v } = indexToLocal(pt.x - shape.x, pt.y - shape.y, b);
      if (handleId === "rx") return { ...shape, rx: Math.max(MIN_RADIUS, Math.abs(u)) };
      if (handleId === "ry") return { ...shape, ry: Math.max(MIN_RADIUS, Math.abs(v)) };
      return shape;
    }
    case "box": {
      if (handleId === "rot") return { ...shape, angle: rotationAngle(shape, pt) };
      const b = basis(shape.angle);
      const { u, v } = indexToLocal(pt.x - shape.x, pt.y - shape.y, b);
      let left = -shape.width / 2;
      let right = shape.width / 2;
      let top = -shape.height / 2;
      let bottom = shape.height / 2;
      if (handleId.includes("e")) right = Math.max(u, left + MIN_SIZE);
      if (handleId.includes("w")) left = Math.min(u, right - MIN_SIZE);
      if (handleId.includes("n")) top = Math.min(v, bottom - MIN_SIZE);
      if (handleId.includes("s")) bottom = Math.max(v, top + MIN_SIZE);
      const known = ["n", "s", "e", "w", "ne", "nw", "se", "sw"];
      if (!known.includes(handleId)) return shape;
      const off = localToIndex((left + right) / 2, (top + bottom) / 2, b);
      return {
        ...shape,
        x: shape.x + off.x,
        y: shape.y + off.y,
        width: right - left,
        height: bottom - top,
      };
    }
    case "annulus": {
      const d = dist(pt, shape);
      if (handleId === "inner") {
        return { ...shape, r_inner: Math.max(0, Math.min(d, shape.r_outer - MIN_ANNULUS_GAP)) };
      }
      if (handleId === "outer") {
        return { ...shape, r_outer: Math.max(d, shape.r_inner + MIN_ANNULUS_GAP) };
      }
      return shape;
    }
    case "polygon": {
      if (!handleId.startsWith("v")) return shape;
      const i = Number(handleId.slice(1));
      if (!Number.isInteger(i) || i < 0 || i >= shape.points.length) return shape;
      const points = shape.points.map((p, k) => (k === i ? ([pt.x, pt.y] as [number, number]) : p));
      return { ...shape, points };
    }
    case "line": {
      if (handleId === "p1") return { ...shape, x1: pt.x, y1: pt.y };
      if (handleId === "p2") return { ...shape, x2: pt.x, y2: pt.y };
      return shape;
    }
    case "point":
      return shape;
  }
}

export function translateShape(shape: RegionShape, dx: number, dy: number): RegionShape {
  switch (shape.shape) {
    case "polygon":
      return { ...shape, points: shape.points.map(([px, py]) => [px + dx, py + dy] as [number, number]) };
    case "line":
      return { ...shape, x1: shape.x1 + dx, y1: shape.y1 + dy, x2: shape.x2 + dx, y2: shape.y2 + dy };
    default:
      return { ...shape, x: shape.x + dx, y: shape.y + dy };
  }
}

export function shapeContains(shape: RegionShape, pt: Pt): boolean {
  switch (shape.shape) {
    case "circle": {
      const dx = pt.x - shape.x;
      const dy = pt.y - shape.y;
      return dx * dx + dy * dy <= shape.r * shape.r;
    }
    case "ellipse": {
      const { u, v } = indexToLocal(pt.x - shape.x, pt.y - shape.y, basis(shape.angle));
      const a = u / shape.rx;
      const b = v / shape.ry;
      return a * a + b * b <= 1;
    }
    case "box": {
      const { u, v } = indexToLocal(pt.x - shape.x, pt.y - shape.y, basis(shape.angle));
      return Math.abs(u) <= shape.width / 2 && Math.abs(v) <= shape.height / 2;
    }
    case "annulus": {
      const dx = pt.x - shape.x;
      const dy = pt.y - shape.y;
      const d2 = dx * dx + dy * dy;
      return shape.r_inner * shape.r_inner < d2 && d2 <= shape.r_outer * shape.r_outer;
    }
    case "polygon": {
      const pts = shape.points;
      const n = pts.length;
      let inside = false;
      for (let i = 0, j = n - 1; i < n; j = i++) {
        const [xi, yi] = pts[i];
        const [xj, yj] = pts[j];
        if (yi > pt.y !== yj > pt.y && pt.x < ((xj - xi) * (pt.y - yi)) / (yj - yi) + xi) inside = !inside;
      }
      return inside;
    }
    case "line":
    case "point":
      return false;
  }
}

function edgeDistance(shape: RegionShape, pt: Pt): number {
  switch (shape.shape) {
    case "circle":
      return Math.abs(dist(pt, shape) - shape.r);
    case "ellipse": {
      const { u, v } = indexToLocal(pt.x - shape.x, pt.y - shape.y, basis(shape.angle));
      const rho = Math.hypot(u / shape.rx, v / shape.ry);
      return Math.abs(rho - 1) * Math.min(shape.rx, shape.ry);
    }
    case "box":
      return polylineDistance(pt, shapeOutline(shape)[0]);
    case "annulus": {
      const d = dist(pt, shape);
      return Math.min(Math.abs(d - shape.r_inner), Math.abs(d - shape.r_outer));
    }
    case "polygon":
      return polylineDistance(pt, shapeOutline(shape)[0]);
    case "line":
      return segmentDistance(pt, { x: shape.x1, y: shape.y1 }, { x: shape.x2, y: shape.y2 });
    case "point":
      return dist(pt, shape);
  }
}

export function hitTest(shape: RegionShape, pt: Pt, tolerance: number): "edge" | "inside" | null {
  if (edgeDistance(shape, pt) <= tolerance) return "edge";
  return shapeContains(shape, pt) ? "inside" : null;
}

export function shapeBounds(shape: RegionShape): { x0: number; y0: number; x1: number; y1: number } {
  switch (shape.shape) {
    case "circle":
      return { x0: shape.x - shape.r, y0: shape.y - shape.r, x1: shape.x + shape.r, y1: shape.y + shape.r };
    case "ellipse": {
      const b = basis(shape.angle);
      const hx = Math.hypot(shape.rx * b.cos, shape.ry * b.sin);
      const hy = Math.hypot(shape.rx * b.sin, shape.ry * b.cos);
      return { x0: shape.x - hx, y0: shape.y - hy, x1: shape.x + hx, y1: shape.y + hy };
    }
    case "box":
    case "polygon":
    case "line": {
      const pts = shapeOutline(shape).flat();
      let x0 = Infinity;
      let y0 = Infinity;
      let x1 = -Infinity;
      let y1 = -Infinity;
      for (const p of pts) {
        x0 = Math.min(x0, p.x);
        y0 = Math.min(y0, p.y);
        x1 = Math.max(x1, p.x);
        y1 = Math.max(y1, p.y);
      }
      return { x0, y0, x1, y1 };
    }
    case "annulus":
      return {
        x0: shape.x - shape.r_outer,
        y0: shape.y - shape.r_outer,
        x1: shape.x + shape.r_outer,
        y1: shape.y + shape.r_outer,
      };
    case "point":
      return { x0: shape.x, y0: shape.y, x1: shape.x, y1: shape.y };
  }
}

function ring(cx: number, cy: number, rx: number, ry: number, b: Basis): Pt[] {
  const pts: Pt[] = [];
  for (let i = 0; i <= OUTLINE_SEGMENTS; i++) {
    const t = (i / OUTLINE_SEGMENTS) * 2 * Math.PI;
    const p = localToIndex(rx * Math.cos(t), ry * Math.sin(t), b);
    pts.push({ x: cx + p.x, y: cy + p.y });
  }
  return pts;
}

const IDENTITY_BASIS: Basis = { cos: 1, sin: 0 };

export function shapeOutline(shape: RegionShape): Pt[][] {
  switch (shape.shape) {
    case "circle":
      return [ring(shape.x, shape.y, shape.r, shape.r, IDENTITY_BASIS)];
    case "ellipse":
      return [ring(shape.x, shape.y, shape.rx, shape.ry, basis(shape.angle))];
    case "annulus":
      return [
        ring(shape.x, shape.y, shape.r_inner, shape.r_inner, IDENTITY_BASIS),
        ring(shape.x, shape.y, shape.r_outer, shape.r_outer, IDENTITY_BASIS),
      ];
    case "box": {
      const b = basis(shape.angle);
      const hw = shape.width / 2;
      const hh = shape.height / 2;
      const corners = [
        [-hw, -hh],
        [hw, -hh],
        [hw, hh],
        [-hw, hh],
        [-hw, -hh],
      ].map(([u, v]) => {
        const p = localToIndex(u, v, b);
        return { x: shape.x + p.x, y: shape.y + p.y };
      });
      return [corners];
    }
    case "polygon": {
      const pts = shape.points.map(([px, py]) => ({ x: px, y: py }));
      if (pts.length > 0) pts.push({ ...pts[0] });
      return [pts];
    }
    case "line":
      return [[{ x: shape.x1, y: shape.y1 }, { x: shape.x2, y: shape.y2 }]];
    case "point":
      return [
        [{ x: shape.x - POINT_CROSS_HALF, y: shape.y }, { x: shape.x + POINT_CROSS_HALF, y: shape.y }],
        [{ x: shape.x, y: shape.y - POINT_CROSS_HALF }, { x: shape.x, y: shape.y + POINT_CROSS_HALF }],
      ];
  }
}

function f1(v: number): string {
  return v.toFixed(1);
}

export function shapeSummary(shape: RegionShape): string {
  switch (shape.shape) {
    case "circle":
      return `circle (${f1(shape.x)}, ${f1(shape.y)}) r=${f1(shape.r)}`;
    case "ellipse":
      return `ellipse (${f1(shape.x)}, ${f1(shape.y)}) rx=${f1(shape.rx)} ry=${f1(shape.ry)} θ=${f1(shape.angle)}°`;
    case "box":
      return `box (${f1(shape.x)}, ${f1(shape.y)}) ${f1(shape.width)}×${f1(shape.height)} θ=${f1(shape.angle)}°`;
    case "annulus":
      return `annulus (${f1(shape.x)}, ${f1(shape.y)}) r=${f1(shape.r_inner)}–${f1(shape.r_outer)}`;
    case "polygon": {
      const c = centreOf(shape);
      return `polygon n=${shape.points.length} (${f1(c.x)}, ${f1(c.y)})`;
    }
    case "line":
      return `line (${f1(shape.x1)}, ${f1(shape.y1)}) → (${f1(shape.x2)}, ${f1(shape.y2)})`;
    case "point":
      return `point (${f1(shape.x)}, ${f1(shape.y)})`;
  }
}

function fin(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

export function isRegionShape(v: unknown): v is RegionShape {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  if (typeof o.shape !== "string" || !REGION_SHAPE_KINDS.includes(o.shape as RegionShapeKind)) return false;
  switch (o.shape as RegionShapeKind) {
    case "circle":
      return fin(o.x) && fin(o.y) && fin(o.r) && o.r > 0;
    case "ellipse":
      return fin(o.x) && fin(o.y) && fin(o.rx) && fin(o.ry) && fin(o.angle) && o.rx > 0 && o.ry > 0;
    case "box":
      return fin(o.x) && fin(o.y) && fin(o.width) && fin(o.height) && fin(o.angle) && o.width > 0 && o.height > 0;
    case "annulus":
      return fin(o.x) && fin(o.y) && fin(o.r_inner) && fin(o.r_outer) && o.r_inner >= 0 && o.r_inner < o.r_outer;
    case "polygon":
      return (
        Array.isArray(o.points) &&
        o.points.length >= 3 &&
        o.points.every((p) => Array.isArray(p) && p.length === 2 && fin(p[0]) && fin(p[1]))
      );
    case "line":
      return fin(o.x1) && fin(o.y1) && fin(o.x2) && fin(o.y2);
    case "point":
      return fin(o.x) && fin(o.y);
  }
}
