import { describe, it, expect } from "vitest";
import {
  COMPOSITE_RUN_KEY,
  EMPTY_COMPOSITE_CHAIN,
  compositeChainHolds,
  compositeInputFor,
  compositeInputLabel,
  compositeInputPreview,
  compositeResultTarget,
  compositeStepInvalidatesWizard,
  compositeStepOutcome,
  hasCompositeReset,
  isCompositeChainStale,
  lastCompositeStep,
  samePreview,
  withCompositePsfKernel,
  withCompositeStep,
} from "../compositeChain";
import { CHAIN_ORDER } from "../processingChain";
import type { ChainStep } from "../../shared/types/preview";
import type { CompositeChain, CompositeChainEntry, CompositeStepMeta, DisplayStf } from "../../shared/types/compositeChain";

const STF = { shadow: 0, midtone: 0.5, highlight: 1 };
const CALL_STF: DisplayStf = { r: STF, g: STF, b: STF, linked: true };
const OTHER_STF: DisplayStf = { r: { shadow: 0.1, midtone: 0.3, highlight: 0.9 }, g: STF, b: STF, linked: false };

function entry(step: ChainStep, displayed: CompositeChainEntry["displayed"] = "linear"): CompositeChainEntry {
  return { previewUrl: `http://asset.localhost/out/composite_chain_${step}_1.png`, label: step, displayed };
}

function meta(overrides: Partial<CompositeStepMeta> = {}): CompositeStepMeta {
  return {
    png_path: "C:/out/composite_chain_x_1.png",
    base_png_path: null,
    dimensions: [10, 10],
    elapsed_ms: 1,
    chain_generation: 7,
    chain_restarted: false,
    chain_input: "base",
    displayed: "linear",
    stf: null,
    previewUrl: "http://asset.localhost/out/composite_chain_x_1.png",
    ...overrides,
  };
}

function chainOf(...steps: ChainStep[]): CompositeChain {
  const built: Partial<Record<ChainStep, CompositeChainEntry>> = {};
  for (const step of steps) built[step] = entry(step);
  return { generation: 7, base: { previewUrl: "http://asset.localhost/out/composite_chain_base_1.png", stf: CALL_STF }, steps: built, psfKernel: null };
}

function presentSteps(chain: CompositeChain): ChainStep[] {
  return CHAIN_ORDER.filter((s) => chain.steps[s] !== undefined);
}

describe("constants", () => {
  it("exposes the shared run lock key and a frozen empty chain", () => {
    expect(COMPOSITE_RUN_KEY).toBe("__composite__");
    expect(EMPTY_COMPOSITE_CHAIN).toEqual({ generation: null, base: null, steps: {}, psfKernel: null });
    expect(Object.isFrozen(EMPTY_COMPOSITE_CHAIN)).toBe(true);
  });
});

describe("compositeInputFor", () => {
  it("is base when nothing upstream exists", () => {
    expect(compositeInputFor(EMPTY_COMPOSITE_CHAIN, "background")).toBe("base");
    expect(compositeInputFor(chainOf("stretch"), "deconv")).toBe("base");
    expect(compositeInputFor(EMPTY_COMPOSITE_CHAIN, "pixelMath")).toBe("base");
  });

  it("is the newest step of a lower stage", () => {
    const chain = chainOf("background", "denoise", "deconv", "stretch", "localContrast");
    expect(compositeInputFor(chain, "denoise")).toBe("background");
    expect(compositeInputFor(chain, "stretch")).toBe("deconv");
    expect(compositeInputFor(chain, "maskedStretch")).toBe("deconv");
    expect(compositeInputFor(chain, "localContrast")).toBe("stretch");
  });

  it("treats stretch and masked stretch as siblings that never feed each other", () => {
    expect(compositeInputFor(chainOf("background", "maskedStretch"), "stretch")).toBe("background");
    expect(compositeInputFor(chainOf("background", "stretch"), "maskedStretch")).toBe("background");
  });

  it("local contrast falls through to the linear chain when no stretch ran", () => {
    expect(compositeInputFor(chainOf("background", "denoise"), "localContrast")).toBe("denoise");
  });

  it("pixelMath compounds on the newest step of all, itself included", () => {
    expect(compositeInputFor(chainOf("background", "localContrast"), "pixelMath")).toBe("localContrast");
    expect(compositeInputFor(chainOf("background", "pixelMath"), "pixelMath")).toBe("pixelMath");
  });
});

describe("compositeInputPreview and compositeInputLabel", () => {
  it("resolves the base and step previews", () => {
    const chain = chainOf("background");
    expect(compositeInputPreview(chain, "base")).toBe(chain.base?.previewUrl);
    expect(compositeInputPreview(chain, "background")).toBe(chain.steps.background?.previewUrl);
    expect(compositeInputPreview(chain, "denoise")).toBeNull();
    expect(compositeInputPreview(EMPTY_COMPOSITE_CHAIN, "base")).toBeNull();
  });

  it("labels the base as the composite and steps by name", () => {
    expect(compositeInputLabel("base")).toBe("Composite");
    expect(compositeInputLabel("maskedStretch")).toBe("Masked stretch");
    expect(compositeInputLabel("localContrast")).toBe("LHE / HDRMT");
  });
});

describe("withCompositeStep", () => {
  it("captures the base from the first call and records the generation", () => {
    const next = withCompositeStep(
      EMPTY_COMPOSITE_CHAIN,
      "background",
      entry("background"),
      meta({ base_png_path: "C:/out/composite_chain_base_1.png", basePreviewUrl: "http://asset.localhost/out/composite_chain_base_1.png", chain_generation: 3 }),
      CALL_STF,
    );
    expect(next.base).toEqual({ previewUrl: "http://asset.localhost/out/composite_chain_base_1.png", stf: CALL_STF });
    expect(next.generation).toBe(3);
    expect(presentSteps(next)).toEqual(["background"]);
  });

  it("drops downstream steps when an upstream step re-runs and keeps the base", () => {
    const full = chainOf("background", "denoise", "deconv", "stretch", "localContrast", "pixelMath");
    const next = withCompositeStep(full, "denoise", entry("denoise"), meta({ chain_generation: 9 }), OTHER_STF);
    expect(presentSteps(next)).toEqual(["background", "denoise"]);
    expect(next.base).toBe(full.base);
    expect(next.generation).toBe(9);
  });

  it("stretch and masked stretch replace each other", () => {
    const afterMasked = withCompositeStep(chainOf("background", "stretch", "localContrast"), "maskedStretch", entry("maskedStretch", "stretched"), meta(), CALL_STF);
    expect(presentSteps(afterMasked)).toEqual(["background", "maskedStretch"]);
  });

  it("pixelMath keeps every earlier step and replaces itself", () => {
    const next = withCompositeStep(chainOf("background", "localContrast", "pixelMath"), "pixelMath", entry("pixelMath", "toned"), meta(), CALL_STF);
    expect(presentSteps(next)).toEqual(["background", "localContrast", "pixelMath"]);
  });

  it("a restart replaces the base with the new capture and drops every step", () => {
    const full = chainOf("background", "denoise", "stretch");
    const next = withCompositeStep(
      full,
      "stretch",
      entry("stretch", "stretched"),
      meta({ chain_restarted: true, chain_input: "base", basePreviewUrl: "http://asset.localhost/out/composite_chain_base_2.png", chain_generation: 12 }),
      OTHER_STF,
    );
    expect(presentSteps(next)).toEqual(["stretch"]);
    expect(next.base).toEqual({ previewUrl: "http://asset.localhost/out/composite_chain_base_2.png", stf: OTHER_STF });
    expect(next.generation).toBe(12);
  });

  it("does not mutate its input and keeps the PSF kernel", () => {
    const chain = { ...chainOf("background"), psfKernel: [[1]] };
    const snapshot = JSON.stringify(chain);
    const next = withCompositeStep(chain, "deconv", entry("deconv"), meta(), CALL_STF);
    expect(JSON.stringify(chain)).toBe(snapshot);
    expect(next.psfKernel).toEqual([[1]]);
  });

  it("records nothing on an empty chain when the result carries no base", () => {
    const next = withCompositeStep(EMPTY_COMPOSITE_CHAIN, "deconv", entry("deconv"), meta({ chain_generation: 5 }), CALL_STF);
    expect(next).toBe(EMPTY_COMPOSITE_CHAIN);
  });

  it("records nothing on a restart that carries no base", () => {
    const full = chainOf("background", "denoise");
    const next = withCompositeStep(full, "stretch", entry("stretch", "stretched"), meta({ chain_restarted: true, chain_input: "base", chain_generation: 12 }), OTHER_STF);
    expect(next).toBe(full);
  });
});

describe("withCompositePsfKernel", () => {
  it("pins a kernel measured without a base to the live generation it was measured on", () => {
    const next = withCompositePsfKernel(EMPTY_COMPOSITE_CHAIN, [[1]], 4);
    expect(next).toEqual({ generation: 4, base: null, steps: {}, psfKernel: [[1]] });
  });

  it("keeps the chain generation, base and steps when a base exists", () => {
    const chain = chainOf("background");
    const next = withCompositePsfKernel(chain, [[2]], 9);
    expect(next.generation).toBe(7);
    expect(next.base).toBe(chain.base);
    expect(next.steps).toBe(chain.steps);
    expect(next.psfKernel).toEqual([[2]]);
  });
});

describe("compositeChainHolds", () => {
  it("is true only while the chain still holds that step's output, ignoring cache-busting queries", () => {
    const chain = chainOf("background", "denoise");
    expect(compositeChainHolds(chain, "denoise", `${chain.steps.denoise?.previewUrl}?v=5`)).toBe(true);
    expect(compositeChainHolds(chain, "denoise", "http://asset.localhost/out/composite_chain_denoise_2.png")).toBe(false);
    expect(compositeChainHolds(chain, "deconv", "http://asset.localhost/out/composite_chain_deconv_1.png")).toBe(false);
    expect(compositeChainHolds(chain, "deconv", undefined)).toBe(true);
  });
});

describe("samePreview", () => {
  it("compares case- and slash-insensitively without the query or hash", () => {
    expect(samePreview("http://x/A.png?v=1", "http://x/a.png#f")).toBe(true);
    expect(samePreview("http://x/a.png", "http://x/b.png")).toBe(false);
  });
});

describe("isCompositeChainStale", () => {
  const chain = chainOf("background");

  it("is stale when the backend has no chain, the live composite moved on, or the generations disagree", () => {
    expect(isCompositeChainStale(chain, { chain_generation: null, live_generation: 7 })).toBe(true);
    expect(isCompositeChainStale(chain, { chain_generation: 7, live_generation: 8 })).toBe(true);
    expect(isCompositeChainStale(chain, { chain_generation: 6, live_generation: 6 })).toBe(true);
  });

  it("is fresh when every generation agrees", () => {
    expect(isCompositeChainStale(chain, { chain_generation: 7, live_generation: 7 })).toBe(false);
  });

  it("a kernel-only chain goes stale once the live composite moves past the generation it was measured on", () => {
    const kernelOnly: CompositeChain = { ...EMPTY_COMPOSITE_CHAIN, psfKernel: [[1]], generation: 4 };
    expect(isCompositeChainStale(kernelOnly, { chain_generation: null, live_generation: 4 })).toBe(false);
    expect(isCompositeChainStale(kernelOnly, { chain_generation: null, live_generation: 5 })).toBe(true);
    expect(isCompositeChainStale({ ...EMPTY_COMPOSITE_CHAIN, psfKernel: [[1]] }, { chain_generation: null, live_generation: 0 })).toBe(true);
  });
});

describe("lastCompositeStep and hasCompositeReset", () => {
  it("reports the most downstream step", () => {
    expect(lastCompositeStep(EMPTY_COMPOSITE_CHAIN)).toBeNull();
    expect(lastCompositeStep(chainOf("maskedStretch", "background"))).toBe("maskedStretch");
  });

  it("offers a reset for any step or a PSF kernel", () => {
    expect(hasCompositeReset(EMPTY_COMPOSITE_CHAIN)).toBe(false);
    expect(hasCompositeReset({ ...EMPTY_COMPOSITE_CHAIN, psfKernel: [[1]] })).toBe(true);
    expect(hasCompositeReset(chainOf("pixelMath"))).toBe(true);
  });
});

describe("compositeStepInvalidatesWizard", () => {
  it("invalidates after linear outputs and after either stretch, never after toned outputs", () => {
    expect(compositeStepInvalidatesWizard("background", "linear")).toBe(true);
    expect(compositeStepInvalidatesWizard("stretch", "stretched")).toBe(true);
    expect(compositeStepInvalidatesWizard("maskedStretch", "stretched")).toBe(true);
    expect(compositeStepInvalidatesWizard("pixelMath", "stretched")).toBe(false);
    expect(compositeStepInvalidatesWizard("localContrast", "toned")).toBe(false);
  });
});

describe("compositeStepOutcome", () => {
  it("a linear step that lands after the composite was parked updates the parked slot and still invalidates the wizard", () => {
    expect(compositeStepOutcome({ step: "deconv", displayed: "linear", accepted: true, compositeMode: false })).toEqual({
      target: "parked",
      invalidatesWizard: true,
    });
  });

  it("a step that lands while the composite is on screen shows its output", () => {
    expect(compositeStepOutcome({ step: "deconv", displayed: "linear", accepted: true, compositeMode: true })).toEqual({
      target: "screen",
      invalidatesWizard: true,
    });
  });

  it("the backend write stales the wizard even when the chain did not record the entry", () => {
    expect(compositeStepOutcome({ step: "background", displayed: "linear", accepted: false, compositeMode: true })).toEqual({
      target: "none",
      invalidatesWizard: true,
    });
    expect(compositeStepOutcome({ step: "background", displayed: "linear", accepted: false, compositeMode: false })).toEqual({
      target: "none",
      invalidatesWizard: true,
    });
  });

  it("a toned step never invalidates the wizard, parked or not", () => {
    expect(compositeStepOutcome({ step: "localContrast", displayed: "toned", accepted: true, compositeMode: false })).toEqual({
      target: "parked",
      invalidatesWizard: false,
    });
  });
});

describe("compositeResultTarget", () => {
  it("sends an accepted result to the screen while the composite is still shown, to the parked slot otherwise", () => {
    expect(compositeResultTarget(true, true)).toBe("screen");
    expect(compositeResultTarget(true, false)).toBe("parked");
  });

  it("drops a result the backend did not apply", () => {
    expect(compositeResultTarget(false, true)).toBe("none");
    expect(compositeResultTarget(false, false)).toBe("none");
  });
});
