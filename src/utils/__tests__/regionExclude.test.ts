import { describe, expect, it } from "vitest";
import {
  EXCLUDE_TITLE,
  backgroundCandidates,
  excludeShapes,
  excludedCellLabel,
  exclusionsFor,
  isExcludeRegion,
  toggledInclude,
} from "../regionExclude";
import type { Region, RegionShape } from "../../shared/types/regions";

const CIRCLE: RegionShape = { shape: "circle", x: 30, y: 30, r: 5 };
const BOX: RegionShape = { shape: "box", x: 10, y: 10, width: 4, height: 2, angle: 0 };
const ANNULUS: RegionShape = { shape: "annulus", x: 5, y: 5, r_inner: 2, r_outer: 4 };
const POLYGON: RegionShape = { shape: "polygon", points: [[0, 0], [4, 0], [0, 4]] };
const ELLIPSE: RegionShape = { shape: "ellipse", x: 8, y: 8, rx: 3, ry: 2, angle: 15 };
const LINE: RegionShape = { shape: "line", x1: 0, y1: 0, x2: 9, y2: 9 };
const POINT: RegionShape = { shape: "point", x: 3, y: 3 };

function region(id: string, shape: RegionShape, include: boolean): Region {
  return { id, shape, props: { color: "#f00", width: 2, text: id, dash: null, include }, backgroundId: null };
}

describe("excludeShapes", () => {
  it("keeps the area shapes marked exclude, in order", () => {
    const regions = [
      region("a", CIRCLE, false),
      region("b", BOX, true),
      region("c", ANNULUS, false),
      region("d", POLYGON, false),
      region("e", ELLIPSE, false),
    ];
    expect(excludeShapes(regions)).toEqual([CIRCLE, ANNULUS, POLYGON, ELLIPSE]);
  });

  it("ignores lines and points even when they carry the DS9 '-' prefix, and include regions", () => {
    expect(excludeShapes([region("l", LINE, false), region("p", POINT, false), region("c", CIRCLE, true)])).toEqual([]);
    expect(excludeShapes([])).toEqual([]);
  });
});

describe("isExcludeRegion", () => {
  it("is true only for an area shape with include off", () => {
    expect(isExcludeRegion(region("a", CIRCLE, false))).toBe(true);
    expect(isExcludeRegion(region("a", CIRCLE, true))).toBe(false);
    expect(isExcludeRegion(region("l", LINE, false))).toBe(false);
    expect(isExcludeRegion(region("p", POINT, false))).toBe(false);
  });
});

describe("exclusionsFor", () => {
  const inner = region("inner", CIRCLE, false);
  const outer = region("outer", { shape: "circle", x: 32, y: 32, r: 20 }, true);
  const cut = region("cut", LINE, true);
  const regions = [outer, inner, cut];

  it("masks every exclude region for an include region, a line and no selection", () => {
    expect(exclusionsFor(outer, regions)).toEqual([CIRCLE]);
    expect(exclusionsFor(cut, regions)).toEqual([CIRCLE]);
    expect(exclusionsFor(undefined, regions)).toEqual([CIRCLE]);
  });

  it("measures an exclude region itself without any exclusion, so it reports what it removes", () => {
    expect(exclusionsFor(inner, regions)).toEqual([]);
  });
});

describe("backgroundCandidates", () => {
  const target = region("c", CIRCLE, true);
  const ring = region("ring", ANNULUS, true);
  const hole = region("hole", { shape: "annulus", x: 9, y: 9, r_inner: 1, r_outer: 3 }, false);
  const regions = [target, ring, hole, region("b", BOX, true)];

  it("offers the include annuli, never the region itself, a non-annulus or an exclude annulus", () => {
    expect(backgroundCandidates(regions, target).map((r) => r.id)).toEqual(["ring"]);
    expect(backgroundCandidates(regions, ring).map((r) => r.id)).toEqual([]);
  });

  it("keeps an exclude annulus that is already the chosen background, so the select shows what is measured", () => {
    expect(backgroundCandidates(regions, { ...target, backgroundId: "hole" }).map((r) => r.id)).toEqual(["ring", "hole"]);
  });
});

describe("excludedCellLabel", () => {
  it("is null when nothing was excluded and names the count with a minus sign otherwise", () => {
    expect(excludedCellLabel({ n_excluded: 0 })).toBeNull();
    expect(excludedCellLabel({ n_excluded: 81 })).toBe("(−81 excl)");
  });
});

describe("toggledInclude", () => {
  it("flips include and keeps every other property", () => {
    const r = region("a", CIRCLE, true);
    expect(toggledInclude(r)).toEqual({ props: { color: "#f00", width: 2, text: "a", dash: null, include: false } });
    expect(toggledInclude(region("a", CIRCLE, false))).toEqual({ props: { color: "#f00", width: 2, text: "a", dash: null, include: true } });
    expect(r.props.include).toBe(true);
  });
});

describe("EXCLUDE_TITLE", () => {
  it("says what an exclude region does and that it is the DS9 '-' prefix", () => {
    expect(EXCLUDE_TITLE).toContain("left out of every other region's statistics and profiles");
    expect(EXCLUDE_TITLE).toContain("DS9 '-' prefix");
  });
});
