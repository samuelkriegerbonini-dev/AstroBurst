import { describe, it, expect } from "vitest";
import {
  CHAIN_ORDER,
  EMPTY_CHAIN,
  withStep,
  inputFor,
  lastStep,
  hasAnyStep,
  dropPaths,
  samePath,
  withVersionParam,
  putCapped,
  pruneRecord,
} from "../processingChain";
import type { ChainEntry, ChainStep, FileRenderState, ProcessingChain } from "../../shared/types/preview";

function entry(fitsPath: string): ChainEntry {
  return { fitsPath, previewUrl: null, dimensions: null };
}

function chainOf(...steps: ChainStep[]): ProcessingChain {
  let chain = EMPTY_CHAIN;
  for (const step of steps) chain = { ...chain, steps: { ...chain.steps, [step]: entry(`/out/${step}.fits`) } };
  return chain;
}

function presentSteps(chain: ProcessingChain): ChainStep[] {
  return CHAIN_ORDER.filter((s) => chain.steps[s] !== undefined);
}

describe("CHAIN_ORDER", () => {
  it("lists the pipeline order", () => {
    expect(CHAIN_ORDER).toEqual(["background", "denoise", "deconv", "stretch", "maskedStretch", "localContrast", "pixelMath"]);
  });
});

describe("withStep", () => {
  it("re-running background clears every downstream step, including stretch, LHE and PixelMath", () => {
    const full = chainOf("background", "denoise", "deconv", "stretch", "localContrast", "pixelMath");
    const next = withStep(full, "background", entry("/out/bg2.fits"));
    expect(presentSteps(next)).toEqual(["background"]);
    expect(next.steps.background?.fitsPath).toBe("/out/bg2.fits");
  });

  it("keeps upstream steps", () => {
    const next = withStep(chainOf("background", "denoise", "deconv"), "deconv", entry("/out/deconv2.fits"));
    expect(presentSteps(next)).toEqual(["background", "denoise", "deconv"]);
    expect(next.steps.deconv?.fitsPath).toBe("/out/deconv2.fits");
    expect(next.steps.background?.fitsPath).toBe("/out/background.fits");
  });

  it("stretch and masked stretch are alternatives", () => {
    const afterMasked = withStep(chainOf("background", "stretch", "localContrast"), "maskedStretch", entry("/out/m.fits"));
    expect(presentSteps(afterMasked)).toEqual(["background", "maskedStretch"]);
    const afterStretch = withStep(afterMasked, "stretch", entry("/out/s.fits"));
    expect(presentSteps(afterStretch)).toEqual(["background", "stretch"]);
  });

  it("re-running stretch clears PixelMath", () => {
    const next = withStep(chainOf("stretch", "pixelMath"), "stretch", entry("/out/s2.fits"));
    expect(presentSteps(next)).toEqual(["stretch"]);
  });

  it("does not mutate its input and keeps the PSF kernel", () => {
    const base = { ...chainOf("background", "denoise"), psfKernel: [[1]] };
    const snapshot = JSON.stringify(base);
    const next = withStep(base, "background", entry("/out/x.fits"));
    expect(JSON.stringify(base)).toBe(snapshot);
    expect(next.psfKernel).toEqual([[1]]);
    expect(EMPTY_CHAIN.steps).toEqual({});
  });
});

describe("inputFor", () => {
  it("uses the original when nothing upstream exists", () => {
    expect(inputFor(EMPTY_CHAIN, "background", "/raw/a.fits")).toBe("/raw/a.fits");
    expect(inputFor(chainOf("stretch"), "deconv", "/raw/a.fits")).toBe("/raw/a.fits");
  });

  it("uses the latest upstream entry", () => {
    const chain = chainOf("background", "denoise", "deconv", "stretch", "localContrast");
    expect(inputFor(chain, "denoise", "/raw/a.fits")).toBe("/out/background.fits");
    expect(inputFor(chain, "stretch", "/raw/a.fits")).toBe("/out/deconv.fits");
    expect(inputFor(chain, "maskedStretch", "/raw/a.fits")).toBe("/out/deconv.fits");
    expect(inputFor(chain, "pixelMath", "/raw/a.fits")).toBe("/out/localContrast.fits");
  });

  it("local contrast falls through to the linear chain when no stretch ran", () => {
    expect(inputFor(chainOf("background", "denoise"), "localContrast", "/raw/a.fits")).toBe("/out/denoise.fits");
  });

  it("stretch never consumes masked stretch output", () => {
    expect(inputFor(chainOf("background", "maskedStretch"), "stretch", "/raw/a.fits")).toBe("/out/background.fits");
  });
});

describe("lastStep and hasAnyStep", () => {
  it("reports the most downstream step", () => {
    expect(lastStep(EMPTY_CHAIN)).toBeNull();
    expect(lastStep(chainOf("background", "deconv"))).toBe("deconv");
    expect(lastStep(chainOf("maskedStretch", "background"))).toBe("maskedStretch");
  });

  it("ignores the PSF kernel", () => {
    expect(hasAnyStep(EMPTY_CHAIN)).toBe(false);
    expect(hasAnyStep({ ...EMPTY_CHAIN, psfKernel: [[1]] })).toBe(false);
    expect(hasAnyStep(chainOf("pixelMath"))).toBe(true);
  });
});

describe("dropPaths", () => {
  it("removes entries case- and slash-insensitively", () => {
    const chain: ProcessingChain = {
      steps: { background: entry("C:\\Out\\BG.fits"), denoise: entry("C:/out/dn.fits") },
      psfKernel: null,
    };
    const next = dropPaths(chain, ["c:/out/bg.FITS"]);
    expect(presentSteps(next)).toEqual(["denoise"]);
  });

  it("returns the same object when nothing matches", () => {
    const chain = chainOf("background");
    expect(dropPaths(chain, ["/elsewhere.fits"])).toBe(chain);
    expect(dropPaths(chain, [])).toBe(chain);
  });
});

describe("samePath", () => {
  it("compares case- and slash-insensitively", () => {
    expect(samePath("C:\\A\\b.fits", "c:/a/B.FITS")).toBe(true);
    expect(samePath("/a/b.fits", "/a/c.fits")).toBe(false);
  });
});

describe("withVersionParam", () => {
  it("appends a version", () => {
    expect(withVersionParam("http://asset.localhost/a.png", 3)).toBe("http://asset.localhost/a.png?v=3");
    expect(withVersionParam("http://asset.localhost/a.png?t=9", 3)).toBe("http://asset.localhost/a.png?t=9&v=3");
  });

  it("replaces a previous version instead of stacking it", () => {
    expect(withVersionParam("http://x/a.png?v=1&t=2", 5)).toBe("http://x/a.png?t=2&v=5");
    expect(withVersionParam("http://x/a.png?v=1", 2)).toBe("http://x/a.png?v=2");
  });
});

describe("pruneRecord", () => {
  const record: FileRenderState = {
    processed: {
      fitsPath: "C:\\out\\a_denoise.fits",
      previewUrl: "http://x/a.png?v=2",
      dimensions: null,
      label: "Denoise",
      kind: "processing",
      inputPath: "C:/raw/a.fits",
    },
    chain: {
      steps: { background: entry("C:/out/a_bg.fits"), denoise: entry("C:/out/a_denoise.fits") },
      psfKernel: null,
    },
    version: 2,
  };

  it("drops the whole record when the displayed output was deleted", () => {
    expect(pruneRecord(record, ["c:/OUT/a_denoise.fits"])).toBeNull();
  });

  it("drops only chain entries when an upstream output was deleted", () => {
    const next = pruneRecord(record, ["C:/out/a_bg.fits"]);
    expect(next).not.toBeNull();
    expect(next?.processed).toBe(record.processed);
    expect(presentSteps(next!.chain)).toEqual(["denoise"]);
  });

  it("returns the same record when nothing matches", () => {
    expect(pruneRecord(record, ["C:/out/other.fits"])).toBe(record);
  });
});

describe("putCapped", () => {
  it("evicts the oldest entry when a new key arrives at the cap", () => {
    const map = new Map<string, number>([["a", 1], ["b", 2], ["c", 3]]);
    putCapped(map, "d", 4, 3);
    expect([...map.keys()]).toEqual(["b", "c", "d"]);
  });

  it("updating an existing key in a full map does not evict another key", () => {
    const map = new Map<string, number>([["a", 1], ["b", 2], ["c", 3]]);
    putCapped(map, "b", 20, 3);
    expect(map.size).toBe(3);
    expect(map.get("a")).toBe(1);
    expect(map.get("b")).toBe(20);
  });

  it("moves a rewritten key to the newest position", () => {
    const map = new Map<string, number>([["a", 1], ["b", 2], ["c", 3]]);
    putCapped(map, "a", 10, 3);
    putCapped(map, "d", 4, 3);
    expect([...map.keys()]).toEqual(["c", "a", "d"]);
  });
});
