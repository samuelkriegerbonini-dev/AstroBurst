import { describe, expect, it } from "vitest";
import { correctedCopySwap, cosmeticCopiesOf, listedCosmeticCopies, withCosmeticCopies, type CosmeticCopy } from "../cosmeticCopies";

const LIGHTS = [1, 2, 3, 4, 5].map((n) => `C:/sx11/frame_000${n}_3.fits`);
const COPIES: CosmeticCopy[] = LIGHTS.map((path) => ({
  original: path,
  corrected: path.replace("C:/sx11/", "C:\\run\\output/").replace(".fits", "_cosmetic.fits"),
}));
const CORRECTED = COPIES.map((c) => c.corrected);

describe("cosmeticCopiesOf", () => {
  it("pairs each corrected light with its _cosmetic.fits and skips the frames that failed", () => {
    expect(cosmeticCopiesOf([
      { path: LIGHTS[0], fits_path: CORRECTED[0] },
      { path: LIGHTS[1], error: "IMAGETYP refused" },
      { path: LIGHTS[2], fits_path: null },
      { path: LIGHTS[3], fits_path: CORRECTED[3] },
    ])).toEqual([COPIES[0], COPIES[3]]);
  });
});

describe("withCosmeticCopies", () => {
  it("adds the copies of a new run after the earlier ones", () => {
    expect(withCosmeticCopies([COPIES[0]], [COPIES[1], COPIES[2]])).toEqual([COPIES[0], COPIES[1], COPIES[2]]);
  });

  it("keeps one entry per corrected file, the latest run naming its original", () => {
    const rerun = { original: "C:/other/frame_0001_3.fits", corrected: CORRECTED[0].replace(/\\/g, "/").toUpperCase() };
    expect(withCosmeticCopies(COPIES.slice(0, 2), [rerun])).toEqual([rerun, COPIES[1]]);
  });

  it("returns the same list when nothing new arrives", () => {
    const current = COPIES.slice(0, 2);
    expect(withCosmeticCopies(current, [])).toBe(current);
  });
});

describe("listedCosmeticCopies", () => {
  it("leaves out a copy that is also loaded in Files, whatever its separators or case", () => {
    const loaded = [...LIGHTS, CORRECTED[0].replace(/\//g, "\\")];
    expect(listedCosmeticCopies(COPIES, loaded)).toEqual(COPIES.slice(1));
  });
});

describe("correctedCopySwap", () => {
  it("swaps each selected original for its copy in place", () => {
    expect(correctedCopySwap([LIGHTS[0], "C:/sx11/other.fits", LIGHTS[2]], COPIES, [])).toEqual({
      count: 2,
      paths: [CORRECTED[0], "C:/sx11/other.fits", CORRECTED[2]],
    });
  });

  it("selects every copy when no original with a copy is selected", () => {
    expect(correctedCopySwap([], COPIES, [])).toEqual({ count: 5, paths: CORRECTED });
  });

  it("leaves out the copies of frames the Subframe Selector rejected", () => {
    expect(correctedCopySwap([], COPIES, [LIGHTS[1]])).toEqual({
      count: 4,
      paths: [CORRECTED[0], CORRECTED[2], CORRECTED[3], CORRECTED[4]],
    });
  });

  it("never selects a copy twice and has nothing to do once the copies are in use", () => {
    expect(correctedCopySwap([LIGHTS[0], CORRECTED[0]], COPIES, [])).toEqual({ count: 1, paths: [CORRECTED[0]] });
    expect(correctedCopySwap(CORRECTED, COPIES, [])).toEqual({ count: 0, paths: CORRECTED });
  });

  it("uses the path of a copy loaded in Files so its row shows selected", () => {
    const loadedCopy = CORRECTED[0].replace(/\//g, "\\");
    expect(correctedCopySwap([LIGHTS[0]], COPIES, [], [loadedCopy])).toEqual({ count: 1, paths: [loadedCopy] });
  });
});
