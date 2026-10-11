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
  psfUseOf,
  showsPsfCrumb,
  pixelMathTarget,
  pixelMathCompareBase,
  chainForPublish,
  chainBasePath,
  pngOnlyInputNotice,
  debayerPreviewRecord,
  backgroundInputFor,
  pixelMathInputPreview,
  sectionInputName,
  gridSourceOf,
} from "../processingChain";
import { displaysFileGrid } from "../stackingOutputs";
import type { ChainBase, ChainEntry, ChainStep, FileRenderState, ProcessedResult, ProcessingChain } from "../../shared/types/preview";
import type { ProcessedFile } from "../../shared/types";

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
    expect(CHAIN_ORDER).toEqual(["background", "denoise", "deconv", "stretch", "maskedStretch", "localContrast", "tone", "pixelMath"]);
  });

  it("places tone after local contrast and before PixelMath", () => {
    expect(CHAIN_ORDER.indexOf("tone")).toBeGreaterThan(CHAIN_ORDER.indexOf("localContrast"));
    expect(CHAIN_ORDER.indexOf("tone")).toBe(CHAIN_ORDER.indexOf("pixelMath") - 1);
  });
});

describe("tone step", () => {
  it("withStep tone keeps stretch and local contrast and drops PixelMath", () => {
    const next = withStep(chainOf("background", "stretch", "localContrast", "pixelMath"), "tone", entry("/out/tone.fits"));
    expect(presentSteps(next)).toEqual(["background", "stretch", "localContrast", "tone"]);
    expect(next.steps.tone?.fitsPath).toBe("/out/tone.fits");
  });

  it("re-running an upstream step drops tone", () => {
    const next = withStep(chainOf("stretch", "tone"), "stretch", entry("/out/s2.fits"));
    expect(presentSteps(next)).toEqual(["stretch"]);
  });

  it("PixelMath consumes the tone output when present", () => {
    expect(inputFor(chainOf("background", "stretch", "localContrast", "tone"), "pixelMath", "/raw/a.fits")).toBe("/out/tone.fits");
  });

  it("tone consumes the latest step before it, or the original", () => {
    expect(inputFor(chainOf("background", "maskedStretch", "localContrast", "tone", "pixelMath"), "tone", "/raw/a.fits")).toBe("/out/localContrast.fits");
    expect(inputFor(chainOf("background", "deconv"), "tone", "/raw/a.fits")).toBe("/out/deconv.fits");
    expect(inputFor(chainOf("pixelMath"), "tone", "/raw/a.fits")).toBe("/raw/a.fits");
  });

  it("lastStep reports tone ahead of local contrast", () => {
    expect(lastStep(chainOf("localContrast", "tone"))).toBe("tone");
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
      base: null,
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
      base: null,
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

describe("PSF crumb before Deconv", () => {
  const KERNEL = [[0, 1, 0], [1, 4, 1], [0, 1, 0]];

  it("records whether Deconv used the PSF-tab kernel, and only on the Deconv entry", () => {
    expect(psfUseOf("deconv", "provided")).toEqual({ psfUsed: true });
    expect(psfUseOf("deconv", "estimated")).toEqual({ psfUsed: false });
    expect(psfUseOf("deconv", "gaussian")).toEqual({ psfUsed: false });
    expect(psfUseOf("deconv", undefined)).toEqual({ psfUsed: false });
    expect(psfUseOf("denoise", "provided")).toEqual({});
  });

  it("shows the crumb while a kernel exists and Deconv has not run yet", () => {
    expect(showsPsfCrumb(KERNEL, undefined)).toBe(true);
  });

  it("shows the crumb when Deconv ran with the PSF-tab kernel", () => {
    expect(showsPsfCrumb(KERNEL, { ...entry("/out/deconv.fits"), ...psfUseOf("deconv", "provided") })).toBe(true);
  });

  it("hides the crumb when Deconv ran with a Gaussian or re-estimated PSF", () => {
    expect(showsPsfCrumb(KERNEL, { ...entry("/out/deconv.fits"), ...psfUseOf("deconv", "gaussian") })).toBe(false);
    expect(showsPsfCrumb(KERNEL, { ...entry("/out/deconv.fits"), ...psfUseOf("deconv", "estimated") })).toBe(false);
    expect(showsPsfCrumb(KERNEL, entry("/out/deconv.fits"))).toBe(false);
  });

  it("never shows the crumb without a kernel", () => {
    expect(showsPsfCrumb(null, undefined)).toBe(false);
    expect(showsPsfCrumb(null, { ...entry("/out/deconv.fits"), psfUsed: true })).toBe(false);
  });

  it("keeps the flag when a later step is added to the chain", () => {
    const deconv = { ...entry("/out/deconv.fits"), ...psfUseOf("deconv", "provided") };
    const chain = withStep(withStep({ ...EMPTY_CHAIN, psfKernel: KERNEL }, "deconv", deconv), "stretch", entry("/out/stretch.fits"));
    expect(chain.steps.deconv?.psfUsed).toBe(true);
    expect(showsPsfCrumb(chain.psfKernel, chain.steps.deconv)).toBe(true);
  });
});

describe("pixelMathTarget", () => {
  const RAW = "C:/raw/f444w_i2d.fits";
  const BG = "C:/out/f444w_i2d_bg.fits";
  const PM = "C:/out/f444w_i2d_bg_pixelmath.fits";

  function shown(kind: ProcessedResult["kind"], fitsPath: string | null, inputPath: string): Pick<ProcessedResult, "kind" | "fitsPath" | "inputPath"> {
    return { kind, fitsPath, inputPath };
  }

  it("a PixelMath result on screen targets the image it was computed from, not itself", () => {
    expect(pixelMathTarget(shown("pixelmath", PM, BG), RAW)).toBe(BG);
    expect(pixelMathTarget(shown("pixelmath", "C:/out/f444w_i2d_pixelmath.fits", RAW), RAW)).toBe(RAW);
  });

  it("any other displayed image is the target, and the loaded file when nothing is displayed", () => {
    expect(pixelMathTarget(shown("processing", BG, RAW), RAW)).toBe(BG);
    expect(pixelMathTarget(shown("cube", "C:/out/f444w_moment0.fits", RAW), RAW)).toBe("C:/out/f444w_moment0.fits");
    expect(pixelMathTarget(shown("debayer", null, RAW), RAW)).toBe(RAW);
    expect(pixelMathTarget(null, RAW)).toBe(RAW);
  });
});

describe("pixelMathCompareBase", () => {
  const RAW = "C:/raw/f444w_i2d.fits";
  const BG = "C:/out/f444w_i2d_bg.fits";
  const PM = "C:/out/f444w_i2d_bg_pixelmath.fits";
  const ORIGINAL = { path: RAW, previewUrl: "asset://raw.png" };
  const LABELS: Record<ChainStep, string> = {
    background: "Background",
    denoise: "Denoise",
    deconv: "Deconvolution",
    stretch: "Stretch",
    maskedStretch: "Masked stretch",
    localContrast: "LHE / HDRMT",
    tone: "Curves",
    pixelMath: "PixelMath",
  };
  const bgEntry: ChainEntry = { fitsPath: BG, previewUrl: "asset://bg.png?v=3", dimensions: null };
  const pmEntry: ChainEntry = { fitsPath: PM, previewUrl: "asset://pm.png?v=4", dimensions: null };
  const chain: ProcessingChain = { steps: { background: bgEntry, pixelMath: pmEntry }, psfKernel: null, base: null };

  function pixelMathShown(inputPath: string) {
    return { kind: "pixelmath" as const, previewUrl: "asset://pm.png?v=5", label: "PixelMath", inputPath };
  }

  it("compares a PixelMath result with the chain output it was computed from, never with itself", () => {
    expect(pixelMathCompareBase(chain, pixelMathShown(BG), ORIGINAL, LABELS)).toEqual({ previewUrl: "asset://bg.png?v=3", label: "Background" });
  });

  it("compares a PixelMath result computed from the loaded file with the original", () => {
    const onlyPm: ProcessingChain = { steps: { pixelMath: pmEntry }, psfKernel: null, base: null };
    expect(pixelMathCompareBase(onlyPm, pixelMathShown("c:\\raw\\F444W_i2d.fits"), ORIGINAL, LABELS)).toEqual({ previewUrl: "asset://raw.png", label: "Original" });
  });

  it("has no base preview when the PixelMath input is no longer on screen anywhere", () => {
    expect(pixelMathCompareBase(chain, pixelMathShown("C:/out/f444w_moment0.fits"), ORIGINAL, LABELS)).toEqual({ previewUrl: null, label: "f444w_moment0.fits" });
  });

  it("any other displayed image is its own base, and the original when nothing is displayed", () => {
    const cube = { kind: "cube" as const, previewUrl: "asset://m0.png", label: "Moment 0", inputPath: RAW };
    expect(pixelMathCompareBase(chain, cube, ORIGINAL, LABELS)).toEqual({ previewUrl: "asset://m0.png", label: "Moment 0" });
    expect(pixelMathCompareBase(EMPTY_CHAIN, null, ORIGINAL, LABELS)).toEqual({ previewUrl: "asset://raw.png", label: "Original" });
  });
});

describe("chain base from a stack or cube result on screen", () => {
  const LIGHT = "C:/data/light_001.fits";
  const STACK = "C:/out/light_001_stack3_20261010-120000-000.fits";
  const FRAME = "C:/out/g235h_frame_0120.fits";
  const PV = "C:/out/g235h_pv.fits";

  function shown(kind: ProcessedResult["kind"], fitsPath: string | null, label: string, previewUrl: string | null = "asset://out/shown.png?v=7"): ProcessedResult {
    return { fitsPath, previewUrl, dimensions: [64, 48], label, kind, inputPath: LIGHT };
  }

  const fileChain = withStep(withStep({ ...EMPTY_CHAIN, psfKernel: [[1]] }, "background", entry("C:/out/light_001_bg.fits")), "denoise", entry("C:/out/light_001_dn.fits"));

  it("a stack on screen becomes the base and drops the steps and PSF of the file", () => {
    const next = chainForPublish(fileChain, shown("stacking", STACK, "Stack - 3 frames"));
    expect(next.steps).toEqual({});
    expect(next.psfKernel).toBeNull();
    expect(next.base).toEqual({
      path: STACK,
      label: "Stack - 3 frames",
      previewUrl: "asset://out/shown.png?v=7",
      dimensions: [64, 48],
      kind: "stacking",
      inputPath: LIGHT,
    });
  });

  it("a cube frame FITS on screen becomes the base", () => {
    const next = chainForPublish(fileChain, shown("cube", FRAME, "Frame 120"));
    expect(next.base?.path).toBe(FRAME);
    expect(hasAnyStep(next)).toBe(false);
  });

  it("a PNG-only stack restarts the chain without a base", () => {
    const next = chainForPublish(fileChain, shown("stacking", null, "Drizzle RGB - 6 frames"));
    expect(next).toEqual({ steps: {}, psfKernel: null, base: null });
  });

  it("a processing publish keeps the base, with or without a chain update", () => {
    const rebased = chainForPublish(EMPTY_CHAIN, shown("stacking", STACK, "Stack - 3 frames"));
    expect(chainForPublish(rebased, shown("processing", "C:/out/wcs.fits", "Solved WCS"))).toBe(rebased);
    const bg = entry("C:/out/stack_bg.fits");
    const next = chainForPublish(rebased, shown("processing", bg.fitsPath, "Background"), (c) => withStep(c, "background", bg));
    expect(next.base).toBe(rebased.base);
    expect(next.steps.background).toBe(bg);
  });

  it("wizard and debayer records keep the chain as it is", () => {
    expect(chainForPublish(fileChain, shown("wizard", null, "Stretch"))).toBe(fileChain);
    expect(chainForPublish(fileChain, shown("debayer", null, "Debayer"))).toBe(fileChain);
  });

  it("withStep and dropPaths keep the base", () => {
    const rebased = chainForPublish(EMPTY_CHAIN, shown("cube", PV, "PV"));
    const withBg = withStep(rebased, "background", entry("C:/out/pv_bg.fits"));
    expect(withBg.base).toBe(rebased.base);
    expect(dropPaths(withBg, ["C:/out/pv_bg.fits"]).base).toBe(rebased.base);
  });

  it("the first step reads the base, later steps read the chain", () => {
    const rebased = chainForPublish(fileChain, shown("stacking", STACK, "Stack - 3 frames"));
    expect(chainBasePath(rebased, LIGHT)).toBe(STACK);
    expect(chainBasePath(EMPTY_CHAIN, LIGHT)).toBe(LIGHT);
    expect(inputFor(rebased, "denoise", chainBasePath(rebased, LIGHT))).toBe(STACK);
    expect(inputFor(withStep(rebased, "background", entry("C:/out/stack_bg.fits")), "denoise", chainBasePath(rebased, LIGHT))).toBe("C:/out/stack_bg.fits");
  });

  it("a rebase is visible to the run guard, which reads inputs without a file path", () => {
    const rebased = chainForPublish(EMPTY_CHAIN, shown("stacking", STACK, "Stack - 3 frames"));
    expect(inputFor(rebased, "denoise", "")).toBe(STACK);
    expect(inputFor(EMPTY_CHAIN, "denoise", "")).toBe("");
  });
});

describe("sectionInputName", () => {
  const BASE: ChainBase = { path: "C:/out/stack.fits", label: "Stack - 3 frames", previewUrl: null, dimensions: null, kind: "stacking", inputPath: "C:/data/light_001.fits" };

  it("names the loaded file for the sections that never read the chain base", () => {
    for (const section of ["debayer", "pixelmath", "geometry"]) {
      expect(sectionInputName(section, BASE, "light_001.fits")).toBe("light_001.fits");
    }
  });

  it("names the chain base for the chain steps, and the file when there is no base", () => {
    for (const section of ["background", "denoise", "psf", "deconvolution", "stretch", "masked_stretch", "local_contrast", "hdr", "curves"]) {
      expect(sectionInputName(section, BASE, "light_001.fits")).toBe("Stack - 3 frames");
      expect(sectionInputName(section, null, "light_001.fits")).toBe("light_001.fits");
    }
  });
});

describe("gridSourceOf", () => {
  const LIGHT_1 = "C:/data/light_001.fits";
  const LIGHT_3 = "C:/data/light_003.fits";
  const STACK = "C:/out/light_003_stack3.fits";
  const BG = "C:/out/light_003_stack3_bg.fits";
  const stack: ProcessedResult = { fitsPath: STACK, previewUrl: "asset://out/stack.png", dimensions: [64, 48], label: "Stack - 3 frames", kind: "stacking", inputPath: LIGHT_3 };
  const rebased = chainForPublish(EMPTY_CHAIN, stack);
  const withBg = withStep(rebased, "background", entry(BG));
  const bgShown: ProcessedResult = { fitsPath: BG, previewUrl: "asset://out/bg.png", dimensions: [64, 48], label: "Background", kind: "processing", inputPath: STACK };

  it("puts a step run on a stack built on another frame on that frame's grid", () => {
    expect(displaysFileGrid(gridSourceOf(bgShown, withBg), LIGHT_1)).toBe(false);
    expect(displaysFileGrid(gridSourceOf(bgShown, withBg), LIGHT_3)).toBe(true);
    const pmShown: ProcessedResult = { ...bgShown, fitsPath: "C:/out/pm.fits", label: "PixelMath", kind: "pixelmath", inputPath: BG };
    expect(displaysFileGrid(gridSourceOf(pmShown, withBg), LIGHT_1)).toBe(false);
    const previewOnly: ProcessedResult = { ...bgShown, fitsPath: null };
    expect(displaysFileGrid(gridSourceOf(previewOnly, rebased), LIGHT_1)).toBe(false);
  });

  it("keeps the record on screen when it did not come from the chain base", () => {
    expect(gridSourceOf(stack, rebased)).toBe(stack);
    expect(gridSourceOf(bgShown, EMPTY_CHAIN)).toBe(bgShown);
    const onFile: ProcessedResult = { ...bgShown, fitsPath: "C:/out/light_001_wcs.fits", label: "Solved WCS", inputPath: LIGHT_1 };
    expect(gridSourceOf(onFile, withBg)).toBe(onFile);
    const debayer: ProcessedResult = { fitsPath: null, previewUrl: "asset://out/d.png", dimensions: [64, 48], label: "Debayer", kind: "debayer", inputPath: LIGHT_1 };
    expect(gridSourceOf(debayer, withBg)).toBe(debayer);
    expect(gridSourceOf(null, withBg)).toBeNull();
  });
});

describe("pngOnlyInputNotice", () => {
  function shown(kind: ProcessedResult["kind"], fitsPath: string | null, label: string): ProcessedResult {
    return { fitsPath, previewUrl: "asset://out/shown.png", dimensions: null, label, kind, inputPath: "C:/data/r_1.fits" };
  }

  it("names the input and the PNG-only result on screen", () => {
    expect(pngOnlyInputNotice(shown("stacking", null, "Drizzle RGB - 6 frames"), "r_1.fits")).toBe(
      "Input: r_1.fits, not the image on screen (Drizzle RGB - 6 frames is a PNG-only result).",
    );
    expect(pngOnlyInputNotice(shown("debayer", null, "Debayer"), "cfa.fits")).toBe(
      "Input: cfa.fits, not the image on screen (Debayer is a PNG-only result).",
    );
    expect(pngOnlyInputNotice(shown("cube", null, "Frame 3"), "cube.fits")).toBe(
      "Input: cube.fits, not the image on screen (Frame 3 is a PNG-only result).",
    );
  });

  it("is silent when the image on screen has a FITS, is a processing preview, or nothing is shown", () => {
    expect(pngOnlyInputNotice(shown("stacking", "C:/out/stack.fits", "Stack"), "r_1.fits")).toBeNull();
    expect(pngOnlyInputNotice(shown("processing", null, "Background"), "r_1.fits")).toBeNull();
    expect(pngOnlyInputNotice(null, "r_1.fits")).toBeNull();
    expect(pngOnlyInputNotice({ ...shown("stacking", null, "Stack"), previewUrl: null }, "r_1.fits")).toBeNull();
  });
});

describe("debayerPreviewRecord", () => {
  it("publishes the Debayer output dimensions", () => {
    expect(debayerPreviewRecord("C:/data/cfa.fits", "asset://out/cfa_debayer.png", [512, 384])).toEqual({
      fitsPath: null,
      previewUrl: "asset://out/cfa_debayer.png",
      dimensions: [512, 384],
      label: "Debayer",
      kind: "debayer",
      inputPath: "C:/data/cfa.fits",
    });
  });

  it("publishes no dimensions when the response has none", () => {
    expect(debayerPreviewRecord("C:/data/cfa.fits", "asset://out/cfa_debayer.png", undefined).dimensions).toBeNull();
    expect(debayerPreviewRecord("C:/data/cfa.fits", "asset://out/cfa_debayer.png", [0, 384]).dimensions).toBeNull();
  });
});

describe("backgroundInputFor", () => {
  const FILE: ProcessedFile = {
    id: "f",
    name: "light_001.fits",
    path: "C:/data/light_001.fits",
    sourcePath: "C:/data/light_001.fits",
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/light_001.png", previewUrl: "asset://out/light_001.png", dimensions: [64, 48], elapsed_ms: 1 },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
  const BASE: ChainBase = {
    path: "C:/out/stack.fits",
    label: "Stack - 3 frames",
    previewUrl: "asset://out/stack.png?v=4",
    dimensions: [66, 50],
    kind: "stacking",
    inputPath: "C:/data/light_001.fits",
  };

  it("is the loaded file without a base", () => {
    expect(backgroundInputFor(FILE, null)).toBe(FILE);
    expect(backgroundInputFor(null, BASE)).toBeNull();
  });

  it("reads the base and compares with the base preview", () => {
    const input = backgroundInputFor(FILE, BASE);
    expect(input?.path).toBe("C:/out/stack.fits");
    expect(input?.result?.previewUrl).toBe("asset://out/stack.png?v=4");
    expect(input?.result?.dimensions).toEqual([66, 50]);
    expect(input?.name).toBe("light_001.fits");
  });

  it("never compares with the file preview when the base has none", () => {
    const input = backgroundInputFor(FILE, { ...BASE, previewUrl: null, dimensions: null });
    expect(input?.result?.previewUrl).toBe("");
    expect(input?.result?.dimensions).toEqual([64, 48]);
  });
});

describe("PixelMath compare base on a chain base", () => {
  const RAW = "C:/data/g235h_s3d.fits";
  const PV = "C:/out/g235h_pv.fits";
  const WCS = "C:/out/g235h_wcs.fits";
  const PM = "C:/out/g235h_pv_pixelmath.fits";
  const ORIGINAL = { path: RAW, previewUrl: "asset://raw.png" };
  const LABELS: Record<ChainStep, string> = {
    background: "Background",
    denoise: "Denoise",
    deconv: "Deconvolution",
    stretch: "Stretch",
    maskedStretch: "Masked stretch",
    localContrast: "LHE / HDRMT",
    tone: "Curves",
    pixelMath: "PixelMath",
  };
  const pvShown: ProcessedResult = { fitsPath: PV, previewUrl: "asset://out/pv.png?v=3", dimensions: [40, 900], label: "PV slice", kind: "cube", inputPath: RAW };
  const pvChain = chainForPublish(EMPTY_CHAIN, pvShown);

  function pixelMathShown(inputPath: string, inputPreviewUrl?: string | null) {
    return { kind: "pixelmath" as const, previewUrl: "asset://out/pm.png?v=5", label: "PixelMath", inputPath, inputPreviewUrl };
  }

  it("compares every PixelMath run on a PV slice with the PV slice", () => {
    const withPm = withStep(pvChain, "pixelMath", entry(PM));
    expect(pixelMathCompareBase(withPm, pixelMathShown(PV), ORIGINAL, LABELS)).toEqual({ previewUrl: "asset://out/pv.png?v=3", label: "PV slice" });
  });

  it("falls back to the preview stored on the record when the input is neither the base nor a chain step", () => {
    expect(pixelMathCompareBase(pvChain, pixelMathShown(WCS, "asset://out/wcs.png?v=9"), ORIGINAL, LABELS)).toEqual({
      previewUrl: "asset://out/wcs.png?v=9",
      label: "g235h_wcs.fits",
    });
  });

  it("still compares a run on the loaded file with the original", () => {
    expect(pixelMathCompareBase(pvChain, pixelMathShown(RAW, "asset://other.png"), ORIGINAL, LABELS)).toEqual({ previewUrl: "asset://raw.png", label: "Original" });
  });

  it("stores the preview of the image PixelMath read", () => {
    expect(pixelMathInputPreview(pvShown, "asset://raw.png")).toBe("asset://out/pv.png?v=3");
    expect(pixelMathInputPreview({ ...pixelMathShown(PV, "asset://out/pv.png?v=3"), fitsPath: PM }, "asset://raw.png")).toBe("asset://out/pv.png?v=3");
    expect(pixelMathInputPreview({ ...pvShown, kind: "debayer", fitsPath: null }, "asset://raw.png")).toBe("asset://raw.png");
    expect(pixelMathInputPreview(null, "asset://raw.png")).toBe("asset://raw.png");
  });
});
