import { describe, it, expect } from "vitest";
import {
  LABEL_HEIGHT,
  LABEL_PAD,
  layoutGridLabels,
  viewportCrossing,
  viewportCrossings,
} from "../../components/viewer/painters/gridPainter";
import type { WcsGrid } from "../../shared/types/astrometry";
import type { Pt } from "../regionGeometry";
import { clampGridDensity } from "../gridSteps";

describe("gridSteps density", () => {
  it("clamps the density to the supported range and falls back to the default", () => {
    expect(clampGridDensity(0)).toBe(1);
    expect(clampGridDensity(9)).toBe(5);
    expect(clampGridDensity(2.6)).toBe(3);
    expect(clampGridDensity(Number.NaN)).toBe(3);
  });
});

const IDENTITY = (p: Pt): Pt => ({ x: p.x, y: p.y });
const MEASURE = (text: string): number => text.length * 6;

function northUpGrid(): WcsGrid {
  return {
    frame: "icrs",
    lon_step_deg: 2 / 240,
    lat_step_deg: 30 / 3600,
    notes: [],
    lines: [
      { kind: "lon", value_deg: 150, label: "10h00m00s", points: [[500, -0.5], [500, 1999.5]] },
      { kind: "lon", value_deg: 150.01, label: "10h00m02s", points: [[800, -0.5], [800, 1999.5]] },
      { kind: "lat", value_deg: 2, label: "+02°00'00\"", points: [[-0.5, 1000], [1999.5, 1000]] },
      { kind: "lat", value_deg: 2.01, label: "+02°00'30\"", points: [[-0.5, 1060], [1999.5, 1060]] },
    ],
    labels: [
      { kind: "lon", edge: "bottom", text: "10h00m00s", x: 500, y: 1999.5 },
      { kind: "lon", edge: "bottom", text: "10h00m02s", x: 800, y: 1999.5 },
      { kind: "lat", edge: "left", text: "+02°00'00\"", x: -0.5, y: 1000 },
      { kind: "lat", edge: "left", text: "+02°00'30\"", x: -0.5, y: 1060 },
    ],
  };
}

describe("gridPainter viewport crossings", () => {
  it("returns the entry point and edge of a polyline coming from outside the viewport", () => {
    const fromLeft = viewportCrossing([{ x: -20, y: 40 }, { x: 60, y: 40 }], 100, 100);
    expect(fromLeft).toEqual({ p: { x: 0, y: 40 }, edge: "left" });
    const leaving = viewportCrossing([{ x: 60, y: 40 }, { x: 60, y: 140 }], 100, 100);
    expect(leaving).toEqual({ p: { x: 60, y: 100 }, edge: "bottom" });
    const diagonal = viewportCrossing([{ x: -10, y: -30 }, { x: 30, y: 50 }], 100, 100);
    expect(diagonal?.edge).toBe("top");
    expect(diagonal?.p.y).toBeCloseTo(0, 9);
    expect(diagonal?.p.x).toBeCloseTo(5, 9);
  });

  it("finds both crossings of a segment that spans the whole viewport", () => {
    const both = viewportCrossings([{ x: 50, y: -300 }, { x: 50, y: 700 }], 100, 100);
    expect(both).toEqual([
      { p: { x: 50, y: 0 }, edge: "top" },
      { p: { x: 50, y: 100 }, edge: "bottom" },
    ]);
  });

  it("reports nothing for polylines entirely inside or entirely outside", () => {
    expect(viewportCrossing([{ x: 10, y: 10 }, { x: 90, y: 90 }], 100, 100)).toBeNull();
    expect(viewportCrossing([{ x: -50, y: 10 }, { x: -10, y: 90 }], 100, 100)).toBeNull();
    expect(viewportCrossing([{ x: 150, y: -10 }, { x: 150, y: 300 }], 100, 100)).toBeNull();
    expect(viewportCrossing([{ x: 10, y: 10 }], 100, 100)).toBeNull();
  });
});

describe("gridPainter label layout", () => {
  it("anchors visible edge labels on their edge with the preferred alignment", () => {
    const placed = layoutGridLabels(northUpGrid(), IDENTITY, 2400, 2400, MEASURE);
    expect(placed.map((l) => l.text)).toEqual(["10h00m00s", "10h00m02s", "+02°00'00\"", "+02°00'30\""]);
    const bottom = placed[0];
    expect(bottom.align).toBe("center");
    expect(bottom.baseline).toBe("bottom");
    expect(bottom.x).toBe(500);
    expect(bottom.y).toBe(1999.5 - LABEL_PAD);
    const left = placed[2];
    expect(left.align).toBe("left");
    expect(left.baseline).toBe("middle");
    expect(left.x).toBe(-0.5 + LABEL_PAD);
    expect(left.y).toBe(1000);
    expect(left.box.y1 - left.box.y0).toBe(LABEL_HEIGHT);
  });

  it("drops labels that would overlap an already placed one", () => {
    const zoomedOut = (p: Pt): Pt => ({ x: p.x * 0.1, y: p.y * 0.1 });
    const placed = layoutGridLabels(northUpGrid(), zoomedOut, 400, 400, MEASURE);
    const lonTexts = placed.filter((l) => l.align === "center").map((l) => l.text);
    expect(lonTexts).toEqual(["10h00m00s"]);
    const latTexts = placed.filter((l) => l.align === "left").map((l) => l.text);
    expect(latTexts).toEqual(["+02°00'00\""]);
  });

  it("moves labels of lines whose image edge is off screen to the crossing on the preferred viewport edge", () => {
    const zoomedIn = (p: Pt): Pt => ({ x: p.x * 4 - 1900, y: p.y * 4 - 3900 });
    const placed = layoutGridLabels(northUpGrid(), zoomedIn, 400, 400, MEASURE);
    const lat = placed.find((l) => l.text === "+02°00'00\"");
    expect(lat).toBeDefined();
    expect(lat?.align).toBe("left");
    expect(lat?.x).toBe(LABEL_PAD);
    expect(lat?.y).toBe(100);
    const lon = placed.find((l) => l.text === "10h00m00s");
    expect(lon).toBeDefined();
    expect(lon?.align).toBe("center");
    expect(lon?.baseline).toBe("bottom");
    expect(lon?.x).toBe(100);
    expect(lon?.y).toBe(400 - LABEL_PAD);
    expect(placed.some((l) => l.text === "10h00m02s")).toBe(false);
  });

  it("skips labels whose box would leave the viewport and lines that never cross it", () => {
    const placed = layoutGridLabels(northUpGrid(), IDENTITY, 520, 2400, MEASURE);
    expect(placed.map((l) => l.text)).toEqual(["+02°00'00\"", "+02°00'30\""]);
    expect(layoutGridLabels(northUpGrid(), IDENTITY, 60, 60, MEASURE)).toEqual([]);
  });
});
