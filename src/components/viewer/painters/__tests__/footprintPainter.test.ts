import { describe, expect, it } from "vitest";
import {
  FOOTPRINT_LAYER_ID,
  FOOTPRINT_LAYER_KIND,
  createFootprintPainter,
  footprintLayer,
  isFootprintLayer,
  pathOffCanvas,
  screenPath,
  type FootprintOutline,
} from "../footprintPainter";
import { overlayDataAttributes } from "../../overlayAttributes";
import type { OverlayLayer, OverlayPaintContext } from "../../../../utils/overlayStore";
import type { Pt } from "../../../../utils/regionGeometry";

const toScreen = (p: Pt): Pt => ({ x: p.x * 2 + 10, y: p.y * 2 + 20 });

const SQUARE: FootprintOutline = {
  datasetId: "jw-square",
  polygons: [
    [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ],
  ],
};
const FLC: FootprintOutline = {
  datasetId: "j9irw3fwq",
  polygons: [
    [
      [20, 20],
      [30, 20],
      [30, 30],
    ],
    [
      [40, 40],
      [50, 40],
      [50, 50],
    ],
  ],
};
const FAR: FootprintOutline = {
  datasetId: "far",
  polygons: [
    [
      [5000, 5000],
      [5010, 5000],
      [5010, 5010],
    ],
  ],
};
const COMET: FootprintOutline = { datasetId: "comet", polygons: [] };

interface Stroke {
  points: Pt[];
  strokeStyle: string;
  lineWidth: number;
}

function recordingContext(width = 200, height = 150) {
  const strokes: Stroke[] = [];
  const texts: string[] = [];
  let current: Pt[] = [];
  const ctx = {
    strokeStyle: "",
    fillStyle: "",
    lineWidth: 0,
    font: "",
    textAlign: "",
    textBaseline: "",
    save() {},
    restore() {},
    setLineDash() {},
    beginPath() {
      current = [];
    },
    moveTo(x: number, y: number) {
      current.push({ x, y });
    },
    lineTo(x: number, y: number) {
      current.push({ x, y });
    },
    closePath() {},
    stroke() {
      strokes.push({ points: current, strokeStyle: ctx.strokeStyle, lineWidth: ctx.lineWidth });
    },
    strokeText() {},
    fillText(text: string) {
      texts.push(text);
    },
  };
  const paint: OverlayPaintContext = {
    ctx: ctx as unknown as CanvasRenderingContext2D,
    toScreen,
    width,
    height,
    screenPxPerImagePx: 2,
  };
  return { paint, strokes, texts };
}

describe("footprint geometry", () => {
  it("maps every vertex through toScreen", () => {
    expect(screenPath(SQUARE.polygons[0], toScreen)).toEqual([
      { x: 10, y: 20 },
      { x: 30, y: 20 },
      { x: 30, y: 40 },
      { x: 10, y: 40 },
    ]);
  });

  it("culls a path whose bounding box misses the canvas and keeps one that crosses it", () => {
    expect(pathOffCanvas(screenPath(SQUARE.polygons[0], toScreen), 200, 150)).toBe(false);
    expect(pathOffCanvas(screenPath(FAR.polygons[0], toScreen), 200, 150)).toBe(true);
    expect(
      pathOffCanvas(
        [
          { x: -500, y: 70 },
          { x: 700, y: 70 },
          { x: 700, y: 80 },
        ],
        200,
        150,
      ),
    ).toBe(false);
    expect(
      pathOffCanvas(
        [
          { x: Number.NaN, y: 0 },
          { x: 1, y: 1 },
          { x: 2, y: 2 },
        ],
        200,
        150,
      ),
    ).toBe(true);
  });
});

describe("footprint painter", () => {
  it("strokes each visible polygon once and draws the highlighted footprint last, wider, with its label", () => {
    const { paint, strokes, texts } = recordingContext();
    createFootprintPainter({ footprints: [SQUARE, FLC, FAR], highlight: "jw-square" })(paint);
    expect(strokes).toHaveLength(3);
    expect(strokes[0].points[0]).toEqual({ x: 50, y: 60 });
    expect(strokes[2].points).toEqual(screenPath(SQUARE.polygons[0], toScreen));
    expect(strokes[2].lineWidth).toBeGreaterThan(strokes[0].lineWidth);
    expect(strokes[2].strokeStyle).not.toBe(strokes[0].strokeStyle);
    expect(texts).toEqual(["jw-square"]);
  });

  it("highlights every outline of a dataset listed once per aperture and labels it once", () => {
    const small: FootprintOutline = { datasetId: SQUARE.datasetId, polygons: FLC.polygons.slice(0, 1) };
    const { paint, strokes, texts } = recordingContext();
    createFootprintPainter({ footprints: [SQUARE, FLC, small], highlight: SQUARE.datasetId })(paint);
    expect(strokes).toHaveLength(4);
    const wide = strokes.filter((s) => s.lineWidth === strokes[3].lineWidth);
    expect(wide.map((s) => s.points)).toEqual([screenPath(SQUARE.polygons[0], toScreen), screenPath(FLC.polygons[0], toScreen)]);
    expect(strokes[0].lineWidth).toBeLessThan(strokes[3].lineWidth);
    expect(texts).toEqual([SQUARE.datasetId]);
  });

  it("draws nothing highlighted when the highlight is not a drawn footprint", () => {
    const { paint, strokes, texts } = recordingContext();
    createFootprintPainter({ footprints: [SQUARE, FLC], highlight: "other" })(paint);
    expect(strokes).toHaveLength(3);
    expect(new Set(strokes.map((s) => s.lineWidth)).size).toBe(1);
    expect(texts).toEqual([]);
  });
});

describe("footprint layer", () => {
  it("counts polygons, keeps only a highlight it draws and identifies itself", () => {
    const layer = footprintLayer([SQUARE, FLC, COMET], "j9irw3fwq");
    expect(layer.id).toBe(FOOTPRINT_LAYER_ID);
    expect(layer.kind).toBe(FOOTPRINT_LAYER_KIND);
    expect(FOOTPRINT_LAYER_ID).toBe("mast-footprints");
    expect(layer.visible).toBe(true);
    expect(layer.footprints).toBe(3);
    expect(layer.highlight).toBe("j9irw3fwq");
    expect(footprintLayer([SQUARE], "comet").highlight).toBeNull();
    expect(footprintLayer([SQUARE], null).highlight).toBeNull();
    expect(isFootprintLayer(layer)).toBe(true);
    const plain: OverlayLayer = { id: "gaia-catalog", kind: "catalog", visible: true, paint: () => {} };
    expect(isFootprintLayer(plain)).toBe(false);
  });
});

describe("viewer overlay attributes", () => {
  const grid: OverlayLayer = { id: "wcs-grid", kind: "grid", visible: true, paint: () => {} };
  const hidden: OverlayLayer = { id: "gaia-catalog", kind: "catalog", visible: false, paint: () => {} };

  it("name the visible layers and the footprint polygon count for drivers", () => {
    expect(overlayDataAttributes([grid, hidden, footprintLayer([SQUARE, FLC], "jw-square")])).toEqual({
      "data-testid": "viewer-overlay",
      "data-layers": "wcs-grid mast-footprints",
      "data-footprints": "3",
      "data-highlight": "jw-square",
    });
    expect(overlayDataAttributes([grid, footprintLayer([SQUARE, FLC], null)])).toEqual({
      "data-testid": "viewer-overlay",
      "data-layers": "wcs-grid mast-footprints",
      "data-footprints": "3",
    });
    expect(overlayDataAttributes([grid, hidden])).toEqual({ "data-testid": "viewer-overlay", "data-layers": "wcs-grid" });
    const off = { ...footprintLayer([SQUARE], null), visible: false };
    expect(overlayDataAttributes([off])).toEqual({ "data-testid": "viewer-overlay", "data-layers": "" });
  });
});
