import { describe, it, expect } from "vitest";
import {
  createShape,
  shapeHandles,
  moveHandle,
  translateShape,
  hitTest,
  shapeContains,
  shapeBounds,
  shapeOutline,
  shapeSummary,
  normalizeAngle,
  isRegionShape,
  MIN_RADIUS,
  MIN_SIZE,
  MIN_ANNULUS_GAP,
} from "../regionGeometry";
import type { RegionShape } from "../../shared/types/regions";

const circle: RegionShape = { shape: "circle", x: 100, y: 200, r: 5 };
const box: RegionShape = { shape: "box", x: 50, y: 50, width: 20, height: 10, angle: 0 };
const annulus: RegionShape = { shape: "annulus", x: 0, y: 0, r_inner: 3, r_outer: 6 };
const line: RegionShape = { shape: "line", x1: 0, y1: 0, x2: 10, y2: 0 };
const concave: RegionShape = {
  shape: "polygon",
  points: [
    [0, 0],
    [10, 0],
    [10, 10],
    [5, 5],
    [0, 10],
  ],
};

describe("createShape", () => {
  it("builds each kind from a start/end drag", () => {
    expect(createShape("circle", { x: 1, y: 1 }, { x: 4, y: 5 })).toEqual({ shape: "circle", x: 1, y: 1, r: 5 });
    expect(createShape("ellipse", { x: 1, y: 1 }, { x: 4, y: -3 })).toEqual({
      shape: "ellipse", x: 1, y: 1, rx: 3, ry: 4, angle: 0,
    });
    expect(createShape("box", { x: 10, y: 20 }, { x: 0, y: 24 })).toEqual({
      shape: "box", x: 5, y: 22, width: 10, height: 4, angle: 0,
    });
    expect(createShape("annulus", { x: 0, y: 0 }, { x: 8, y: 0 })).toEqual({
      shape: "annulus", x: 0, y: 0, r_inner: 4, r_outer: 8,
    });
    expect(createShape("line", { x: 1, y: 2 }, { x: 3, y: 4 })).toEqual({ shape: "line", x1: 1, y1: 2, x2: 3, y2: 4 });
    expect(createShape("point", { x: 7, y: 8 }, { x: 9, y: 9 })).toEqual({ shape: "point", x: 7, y: 8 });
  });

  it("enforces minimum sizes on a zero-length drag", () => {
    const p = { x: 3, y: 3 };
    expect(createShape("circle", p, p)).toMatchObject({ r: MIN_RADIUS });
    expect(createShape("ellipse", p, p)).toMatchObject({ rx: MIN_RADIUS, ry: MIN_RADIUS });
    expect(createShape("box", p, p)).toMatchObject({ width: MIN_SIZE, height: MIN_SIZE });
    const a = createShape("annulus", p, p);
    expect(a.shape === "annulus" && a.r_outer - a.r_inner >= MIN_ANNULUS_GAP).toBe(true);
  });
});

describe("shapeHandles", () => {
  it("returns the documented handle counts", () => {
    expect(shapeHandles(circle)).toHaveLength(1);
    expect(shapeHandles({ shape: "ellipse", x: 0, y: 0, rx: 4, ry: 2, angle: 0 })).toHaveLength(3);
    expect(shapeHandles(box)).toHaveLength(9);
    expect(shapeHandles(annulus)).toHaveLength(2);
    expect(shapeHandles(concave)).toHaveLength(5);
    expect(shapeHandles(line)).toHaveLength(2);
    expect(shapeHandles({ shape: "point", x: 0, y: 0 })).toHaveLength(0);
  });

  it("places handles at the geometric positions", () => {
    expect(shapeHandles(circle)[0]).toMatchObject({ id: "r", x: 105, y: 200 });
    const h = Object.fromEntries(shapeHandles(box).map((x) => [x.id, x]));
    expect(h.e).toMatchObject({ x: 60, y: 50 });
    expect(h.w).toMatchObject({ x: 40, y: 50 });
    expect(h.n).toMatchObject({ x: 50, y: 45 });
    expect(h.s).toMatchObject({ x: 50, y: 55 });
    expect(h.ne).toMatchObject({ x: 60, y: 45 });
    expect(h.sw).toMatchObject({ x: 40, y: 55 });
    const ah = shapeHandles(annulus);
    expect(ah[0]).toMatchObject({ id: "inner", x: 3, y: 0 });
    expect(ah[1]).toMatchObject({ id: "outer", x: 6, y: 0 });
    expect(shapeHandles(concave)[3]).toMatchObject({ id: "v3", x: 5, y: 5 });
    expect(shapeHandles(line)[1]).toMatchObject({ id: "p2", x: 10, y: 0 });
  });

  it("rotates box handles with the angle", () => {
    const rotated: RegionShape = { ...box, angle: 90 };
    const h = Object.fromEntries(shapeHandles(rotated).map((x) => [x.id, x]));
    expect(h.e.x).toBeCloseTo(50, 9);
    expect(h.e.y).toBeCloseTo(60, 9);
    expect(h.n.x).toBeCloseTo(55, 9);
    expect(h.n.y).toBeCloseTo(50, 9);
  });
});

describe("moveHandle", () => {
  it("resizes the circle radius with a minimum", () => {
    expect(moveHandle(circle, "r", { x: 110, y: 200 })).toMatchObject({ r: 10 });
    expect(moveHandle(circle, "r", { x: 100, y: 200 })).toMatchObject({ r: MIN_RADIUS });
  });

  it("keeps r_inner < r_outer for the annulus", () => {
    const inner = moveHandle(annulus, "inner", { x: 100, y: 0 });
    expect(inner).toMatchObject({ r_inner: 6 - MIN_ANNULUS_GAP, r_outer: 6 });
    const outer = moveHandle(annulus, "outer", { x: 1, y: 0 });
    expect(outer).toMatchObject({ r_inner: 3, r_outer: 3 + MIN_ANNULUS_GAP });
    expect(moveHandle(annulus, "inner", { x: 0, y: 0 })).toMatchObject({ r_inner: 0 });
  });

  it("moves box edges keeping the opposite edge fixed and width >= MIN_SIZE", () => {
    const e = moveHandle(box, "e", { x: 70, y: 50 });
    expect(e).toMatchObject({ x: 55, y: 50, width: 30, height: 10 });
    const collapsed = moveHandle(box, "e", { x: 0, y: 50 });
    expect(collapsed).toMatchObject({ width: MIN_SIZE });
    expect(collapsed.shape === "box" && collapsed.x - collapsed.width / 2).toBeCloseTo(40, 9);
    const ne = moveHandle(box, "ne", { x: 64, y: 41 });
    expect(ne).toMatchObject({ x: 52, y: 48, width: 24, height: 14 });
  });

  it("rotates the box via the rot handle with an angle in [0, 360)", () => {
    expect(moveHandle(box, "rot", { x: 50, y: 60 })).toMatchObject({ angle: 90 });
    expect(moveHandle(box, "rot", { x: 50, y: 40 })).toMatchObject({ angle: 270 });
    const rotEllipse = moveHandle({ shape: "ellipse", x: 0, y: 0, rx: 4, ry: 2, angle: 0 }, "rot", { x: -5, y: 0 });
    expect(rotEllipse).toMatchObject({ angle: 180 });
  });

  it("moves polygon vertices and line ends", () => {
    const p = moveHandle(concave, "v3", { x: 6, y: 6 });
    expect(p.shape === "polygon" && p.points[3]).toEqual([6, 6]);
    expect(moveHandle(line, "p1", { x: -1, y: -1 })).toMatchObject({ x1: -1, y1: -1, x2: 10, y2: 0 });
    expect(moveHandle(concave, "v9", { x: 6, y: 6 })).toBe(concave);
  });
});

describe("translateShape", () => {
  it("moves every point of every kind", () => {
    expect(translateShape(circle, 1, -1)).toMatchObject({ x: 101, y: 199, r: 5 });
    expect(translateShape(line, 1, 2)).toEqual({ shape: "line", x1: 1, y1: 2, x2: 11, y2: 2 });
    const p = translateShape(concave, 1, 1);
    expect(p.shape === "polygon" && p.points).toEqual([
      [1, 1],
      [11, 1],
      [11, 11],
      [6, 6],
      [1, 11],
    ]);
  });
});

describe("hitTest", () => {
  it("classifies circle centre, ring and outside", () => {
    expect(hitTest(circle, { x: 100, y: 200 }, 0.5)).toBe("inside");
    expect(hitTest(circle, { x: 104.8, y: 200 }, 0.5)).toBe("edge");
    expect(hitTest(circle, { x: 105.3, y: 200 }, 0.5)).toBe("edge");
    expect(hitTest(circle, { x: 110, y: 200 }, 0.5)).toBeNull();
  });

  it("treats a box rotated 90 degrees like the transposed box", () => {
    const rotated: RegionShape = { ...box, angle: 90 };
    const transposed: RegionShape = { ...box, width: 10, height: 20, angle: 0 };
    const probes = [
      { x: 50, y: 58 },
      { x: 58, y: 50 },
      { x: 54, y: 59 },
      { x: 55, y: 50 },
      { x: 50, y: 60 },
      { x: 30, y: 30 },
    ];
    for (const p of probes) {
      expect(hitTest(rotated, p, 0.3)).toBe(hitTest(transposed, p, 0.3));
      expect(shapeContains(rotated, p)).toBe(shapeContains(transposed, p));
    }
    expect(shapeContains(rotated, { x: 50, y: 58 })).toBe(true);
    expect(shapeContains(rotated, { x: 58, y: 50 })).toBe(false);
  });

  it("uses the half-open annulus rule", () => {
    expect(shapeContains(annulus, { x: 3, y: 0 })).toBe(false);
    expect(shapeContains(annulus, { x: 6, y: 0 })).toBe(true);
    expect(shapeContains(annulus, { x: 4.5, y: 0 })).toBe(true);
    expect(hitTest(annulus, { x: 4.5, y: 0 }, 0.2)).toBe("inside");
    expect(hitTest(annulus, { x: 3.1, y: 0 }, 0.2)).toBe("edge");
    expect(hitTest(annulus, { x: 0, y: 0 }, 0.2)).toBeNull();
  });

  it("applies the even-odd rule on a concave polygon", () => {
    expect(hitTest(concave, { x: 2, y: 5 }, 0.1)).toBe("inside");
    expect(hitTest(concave, { x: 5, y: 8 }, 0.1)).toBeNull();
    expect(hitTest(concave, { x: 5, y: 2 }, 0.1)).toBe("inside");
    expect(hitTest(concave, { x: 20, y: 5 }, 0.1)).toBeNull();
    expect(hitTest(concave, { x: 5, y: 0.05 }, 0.1)).toBe("edge");
  });

  it("uses the segment distance for lines and points", () => {
    expect(hitTest(line, { x: 5, y: 0.4 }, 0.5)).toBe("edge");
    expect(hitTest(line, { x: 5, y: 0.6 }, 0.5)).toBeNull();
    expect(hitTest(line, { x: 11, y: 0 }, 0.5)).toBeNull();
    expect(hitTest({ shape: "point", x: 3, y: 3 }, { x: 3.3, y: 3 }, 0.5)).toBe("edge");
    expect(hitTest({ shape: "point", x: 3, y: 3 }, { x: 4, y: 3 }, 0.5)).toBeNull();
  });
});

describe("shapeBounds", () => {
  it("returns the float bbox, accounting for rotation", () => {
    expect(shapeBounds(circle)).toEqual({ x0: 95, y0: 195, x1: 105, y1: 205 });
    const b = shapeBounds({ ...box, angle: 90 } as RegionShape);
    expect(b.x0).toBeCloseTo(45, 9);
    expect(b.x1).toBeCloseTo(55, 9);
    expect(b.y0).toBeCloseTo(40, 9);
    expect(b.y1).toBeCloseTo(60, 9);
    expect(shapeBounds(concave)).toEqual({ x0: 0, y0: 0, x1: 10, y1: 10 });
    expect(shapeBounds(line)).toEqual({ x0: 0, y0: 0, x1: 10, y1: 0 });
  });
});

describe("shapeOutline", () => {
  it("returns closed polylines and a 64-segment circle", () => {
    const c = shapeOutline(circle);
    expect(c).toHaveLength(1);
    expect(c[0]).toHaveLength(65);
    expect(c[0][0].x).toBeCloseTo(c[0][64].x, 9);
    expect(c[0][0].y).toBeCloseTo(c[0][64].y, 9);
    expect(c[0][0]).toEqual({ x: 105, y: 200 });
    const b = shapeOutline(box)[0];
    expect(b).toHaveLength(5);
    expect(b[0]).toEqual(b[4]);
    const p = shapeOutline(concave)[0];
    expect(p).toHaveLength(6);
    expect(p[0]).toEqual(p[5]);
    expect(shapeOutline(annulus)).toHaveLength(2);
    expect(shapeOutline(line)[0]).toHaveLength(2);
    expect(shapeOutline({ shape: "point", x: 1, y: 1 })).toHaveLength(2);
  });
});

describe("shapeSummary", () => {
  it("formats the circle summary", () => {
    expect(shapeSummary(circle)).toBe("circle (100.0, 200.0) r=5.0");
    expect(shapeSummary(concave)).toBe("polygon n=5 (5.0, 5.0)");
  });
});

describe("normalizeAngle", () => {
  it("wraps into [0, 360)", () => {
    expect(normalizeAngle(-30)).toBe(330);
    expect(normalizeAngle(360)).toBe(0);
    expect(normalizeAngle(725)).toBe(5);
    expect(Object.is(normalizeAngle(-360), 0)).toBe(true);
  });
});

describe("isRegionShape", () => {
  it("accepts every valid kind", () => {
    for (const s of [circle, box, annulus, line, concave]) expect(isRegionShape(s)).toBe(true);
    expect(isRegionShape({ shape: "ellipse", x: 0, y: 0, rx: 1, ry: 2, angle: 30 })).toBe(true);
    expect(isRegionShape({ shape: "point", x: 0, y: 0 })).toBe(true);
  });

  it("rejects missing fields, wrong tags and non-finite numbers", () => {
    expect(isRegionShape({ shape: "circle", x: 0, y: 0 })).toBe(false);
    expect(isRegionShape({ shape: "square", x: 0, y: 0, r: 1 })).toBe(false);
    expect(isRegionShape({ shape: "circle", x: NaN, y: 0, r: 1 })).toBe(false);
    expect(isRegionShape({ shape: "circle", x: 0, y: 0, r: Infinity })).toBe(false);
    expect(isRegionShape({ shape: "circle", x: 0, y: 0, r: 0 })).toBe(false);
    expect(isRegionShape({ shape: "annulus", x: 0, y: 0, r_inner: 5, r_outer: 3 })).toBe(false);
    expect(isRegionShape({ shape: "polygon", points: [[0, 0], [1, 1]] })).toBe(false);
    expect(isRegionShape({ shape: "polygon", points: [[0, 0], [1, 1], [1, "2"]] })).toBe(false);
    expect(isRegionShape({ shape: "box", x: 0, y: 0, width: 1, height: 1 })).toBe(false);
    expect(isRegionShape(null)).toBe(false);
    expect(isRegionShape("circle")).toBe(false);
  });
});
