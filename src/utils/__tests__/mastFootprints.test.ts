import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { MastFootprint, MastFootprintInput, MastRow } from "../../shared/types/mast";
import {
  CIRCLE_VERTICES,
  MAX_FOOTPRINT_VERTICES,
  MOVING_TEXT,
  NO_FOOTPRINT_TEXT,
  estimateVertices,
  footprintBlock,
  footprintChunks,
  footprintInput,
  footprintRegions,
  footprintSummary,
  isMovingTarget,
  polygonCount,
} from "../mastFootprints";

const rustSource = (rel: string) => readFileSync(fileURLToPath(new URL(`../../../src-tauri/src/${rel}`, import.meta.url)), "utf8");

function row(over: Partial<MastRow> = {}): MastRow {
  return {
    datasetId: "jw02739-o001_t001_nircam",
    levelClass: "combined",
    level: "3",
    instrument: "NIRCAM",
    filter: "F187N",
    filters: ["F187N"],
    expType: "NRC_IMAGE",
    target: "M 16",
    program: "2739",
    observation: "1",
    dateObs: null,
    expTimeS: null,
    sepArcmin: null,
    sepVerified: false,
    access: "PUBLIC",
    releaseDate: null,
    raDeg: 274.688,
    decDeg: -13.792,
    sRegion: "POLYGON ICRS  274.69 -13.89 274.76 -13.91 274.77 -13.79",
    values: { targtype: "FIXED" },
    ...over,
  };
}

function input(over: Partial<MastFootprintInput> = {}): MastFootprintInput {
  return { datasetId: "a", sRegion: null, polygons: null, targtype: null, ...over };
}

function footprint(over: Partial<MastFootprint> = {}): MastFootprint {
  return {
    datasetId: "jw02739-o001_t001_nircam",
    polygons: [
      [
        [10, 10],
        [80, 10],
        [80, 70],
      ],
    ],
    onImage: true,
    vertices: 3,
    cone: { ra: 274.7347, dec: -13.8672, radiusDeg: 0.0104 },
    note: null,
    ...over,
  };
}

describe("footprint items from archive rows", () => {
  it("carry the row s_region and its target type", () => {
    expect(footprintInput(row())).toEqual({
      datasetId: "jw02739-o001_t001_nircam",
      sRegion: "POLYGON ICRS  274.69 -13.89 274.76 -13.91 274.77 -13.79",
      polygons: null,
      targtype: "FIXED",
    });
    expect(footprintInput(row({ sRegion: "  ", values: {} }))).toEqual({ datasetId: "jw02739-o001_t001_nircam", sRegion: null, polygons: null, targtype: null });
    expect(footprintInput(row({ values: { targtype: 7 } })).targtype).toBeNull();
  });

  it("block moving targets and rows without a footprint, like core/mast/footprints.rs", () => {
    for (const [targtype, moving] of [
      ["MOVING", true],
      [" moving ", true],
      ["FIXED", false],
      ["UNKNOWN", false],
      ["GENERIC", false],
      [null, false],
    ] as const) {
      expect(isMovingTarget(targtype)).toBe(moving);
    }
    expect(footprintBlock(footprintInput(row()))).toBeNull();
    expect(footprintBlock(footprintInput(row({ values: { targtype: "MOVING" } })))).toBe(MOVING_TEXT);
    expect(footprintBlock(footprintInput(row({ sRegion: null })))).toBe(NO_FOOTPRINT_TEXT);
    expect(footprintBlock(input({ polygons: [[[1, 2], [3, 4], [5, 6]]] }))).toBeNull();
    const rust = rustSource("core/mast/footprints.rs");
    expect(rust).toContain(`pub const MAX_FOOTPRINT_VERTICES: usize = ${MAX_FOOTPRINT_VERTICES.toLocaleString("en-US").replace(",", "_")};`);
    expect(rust).toContain('const MOVING_TARGTYPE: &str = "MOVING";');
    expect(rustSource("core/astrometry/stcs.rs")).toContain(`pub const CIRCLE_VERTICES: usize = ${CIRCLE_VERTICES};`);
  });
});

describe("vertex estimates and chunks", () => {
  it("never estimate fewer vertices than the backend counts", () => {
    expect(estimateVertices(input({ sRegion: "POLYGON ICRS 1 2 3 4 5 6" }))).toBe(3);
    expect(estimateVertices(input({ sRegion: "CIRCLE ICRS 150 2 0.001" }))).toBeGreaterThanOrEqual(CIRCLE_VERTICES);
    expect(estimateVertices(input({ sRegion: "BOX ICRS 150 2 0.1 0.1" }))).toBeGreaterThanOrEqual(4);
    expect(estimateVertices(input({ sRegion: "union icrs ( polygon 1 2 3 4 5 6 circle 1 2 3 box 1 2 3 4 )" }))).toBeGreaterThanOrEqual(3 + CIRCLE_VERTICES + 4);
    expect(estimateVertices(input({ sRegion: "POLYGON ICRS 1 2 3 4 5 6", polygons: [[[0, 0], [1, 0], [1, 1], [0, 1]]] }))).toBe(4);
    expect(estimateVertices(input())).toBe(0);
  });

  it("split a long list so no request exceeds the backend cap", () => {
    const circle = input({ sRegion: "CIRCLE ICRS 150 2 0.001" });
    const per = estimateVertices(circle);
    const many = Array.from({ length: 700 }, (_, i) => ({ ...circle, datasetId: `c${i}` }));
    const chunks = footprintChunks(many);
    expect(chunks.flat().map((i) => i.datasetId)).toEqual(many.map((i) => i.datasetId));
    expect(chunks.length).toBeGreaterThan(1);
    for (const chunk of chunks) expect(chunk.length * per).toBeLessThanOrEqual(MAX_FOOTPRINT_VERTICES);
    expect(footprintChunks([])).toEqual([]);
    const blocked = Array.from({ length: 500 }, (_, i) => input({ datasetId: `m${i}`, sRegion: "CIRCLE ICRS 1 2 3", targtype: "MOVING" }));
    expect(footprintChunks(blocked)).toHaveLength(1);
    expect(footprintChunks([circle, circle], 70)).toEqual([[circle], [circle]]);
  });
});

describe("footprint regions and summaries", () => {
  it("turn every drawn polygon into an image-pixel region labelled with its dataset", () => {
    const two = footprint({
      datasetId: "j9irw3fwq",
      polygons: [
        [
          [1, 2],
          [3, 4],
          [5, 6],
        ],
        [
          [7, 8],
          [9, 10],
          [11, 12],
        ],
      ],
    });
    const regions = footprintRegions([footprint(), two, footprint({ datasetId: "comet", polygons: [], note: "moving" })]);
    expect(regions).toHaveLength(3);
    expect(regions[0]).toEqual({
      shape: {
        shape: "polygon",
        points: [
          [10, 10],
          [80, 10],
          [80, 70],
        ],
      },
      props: { color: null, width: null, text: "jw02739-o001_t001_nircam", dash: null, include: true },
    });
    expect(regions.map((r) => r.props.text)).toEqual(["jw02739-o001_t001_nircam", "j9irw3fwq", "j9irw3fwq"]);
    expect(polygonCount([footprint(), two])).toBe(3);
  });

  it("summarise vertices, cone and overlap for the metadata tab", () => {
    expect(footprintSummary(footprint(), "nrca1_cal.fits")).toBe("1 polygon, 3 vertices, cone 274.7347 -13.8672 r 0.62', overlaps nrca1_cal.fits");
    expect(footprintSummary(footprint({ onImage: false, vertices: 78 }), "a.fits")).toBe("1 polygon, 78 vertices, cone 274.7347 -13.8672 r 0.62', off a.fits");
    expect(footprintSummary(footprint({ note: "1 of 2 polygons do not project onto this image" }), "a.fits")).toBe(
      "1 polygon, 3 vertices, cone 274.7347 -13.8672 r 0.62', overlaps a.fits; 1 of 2 polygons do not project onto this image",
    );
    expect(footprintSummary(footprint({ polygons: [], cone: null, vertices: 0, onImage: false, note: "moving target: the archive footprint is not sidereal" }), "a.fits")).toBe(
      "moving target: the archive footprint is not sidereal",
    );
  });
});
