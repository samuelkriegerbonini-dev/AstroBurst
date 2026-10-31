import { describe, expect, it } from "vitest";
import {
  compositeInputDisplayed,
  curvesAutoStfNote,
  hdrLinearInputHint,
  nonLinearInputNote,
  upstreamSteps,
  type NonLinearInput,
  type NonLinearSection,
} from "../nonLinearInput";
import { EMPTY_CHAIN, withStep } from "../processingChain";
import type { ChainStep, ProcessingChain } from "../../shared/types/preview";
import type { CompositeChain, CompositeChainEntry, CompositeDisplayed } from "../../shared/types/compositeChain";

const FILE_NOTES: Record<NonLinearSection, string> = {
  local_contrast: "No stretch step in this chain: LHE on linear data equalizes the noise floor; Stretch or Masked first gives the usual result.",
  hdr: "No stretch step in this chain: HDRMT refuses images whose median is under 1 % of the maximum; Stretch or Masked first.",
  curves: "No stretch step in this chain: if the input is linear, Curves applies an automatic STF first, then your levels and curve.",
};

const COMPOSITE_NOTES: Record<NonLinearSection, string | null> = {
  local_contrast: "The composite input is linear: LHE runs only on a stretched composite; Stretch or Masked first.",
  hdr: "The composite input is linear: HDRMT runs only on a stretched composite; Stretch or Masked first.",
  curves: null,
};

const SECTIONS: NonLinearSection[] = ["local_contrast", "hdr", "curves"];
const LINEAR_STEPS: ChainStep[] = ["background", "denoise", "deconv"];
const STRETCH_STEPS: ChainStep[] = ["stretch", "maskedStretch"];

function fileInput(from: ChainStep | null): NonLinearInput {
  return { kind: "file", upstream: from === null ? [] : [from] };
}

function compositeInput(displayed: CompositeDisplayed | null): NonLinearInput {
  return { kind: "composite", displayed };
}

function chainOf(...steps: ChainStep[]): ProcessingChain {
  return steps.reduce(
    (chain, step) => withStep(chain, step, { fitsPath: `C:/app/history/${step}.fits`, previewUrl: `asset://out/${step}.png`, dimensions: [8, 8] }),
    EMPTY_CHAIN,
  );
}

function entry(displayed: CompositeDisplayed): CompositeChainEntry {
  return { previewUrl: `asset://out/${displayed}.png`, label: displayed, displayed };
}

function compositeChainOf(steps: Partial<Record<ChainStep, CompositeChainEntry>>): CompositeChain {
  const stf = { shadow: 0, midtone: 0.5, highlight: 1 };
  return { generation: 1, base: { previewUrl: "asset://out/blend.png", stf: { r: stf, g: stf, b: stf, linked: true } }, steps, psfKernel: null };
}

describe.each(SECTIONS)("nonLinearInputNote(%s, input)", (section) => {
  it.each([null, ...LINEAR_STEPS])("on a file whose input comes from %s, says that no stretch step precedes the run", (from) => {
    expect(nonLinearInputNote(section, fileInput(from))).toBe(FILE_NOTES[section]);
  });

  it.each(STRETCH_STEPS)("on a file after %s, says nothing", (from) => {
    expect(nonLinearInputNote(section, fileInput(from))).toBeNull();
  });

  it.each([...LINEAR_STEPS, ...STRETCH_STEPS])("on the composite, a linear input read from %s gets the definite composite note", (from) => {
    const chain = compositeChainOf({ [from]: entry("linear") });
    expect(nonLinearInputNote(section, compositeInput(compositeInputDisplayed(chain, from)))).toBe(COMPOSITE_NOTES[section]);
  });

  it.each([...LINEAR_STEPS, ...STRETCH_STEPS])("on the composite, a stretched input read from %s says nothing", (from) => {
    const chain = compositeChainOf({ [from]: entry("stretched") });
    expect(nonLinearInputNote(section, compositeInput(compositeInputDisplayed(chain, from)))).toBeNull();
  });

  it("on the composite base, whose tier the frontend does not hold, says nothing", () => {
    expect(nonLinearInputNote(section, compositeInput(compositeInputDisplayed(compositeChainOf({}), "base")))).toBeNull();
  });

  it("on a composite base the backend reports linear, gets the definite composite note", () => {
    const chain = compositeChainOf({});
    const linearBase = { ...chain, base: chain.base ? { ...chain.base, displayed: "linear" as const } : null };
    expect(compositeInputDisplayed(linearBase, "base")).toBe("linear");
    expect(nonLinearInputNote(section, compositeInput(compositeInputDisplayed(linearBase, "base")))).toBe(COMPOSITE_NOTES[section]);
  });

  it("on a composite base the backend reports stretched, says nothing", () => {
    const chain = compositeChainOf({});
    const stretchedBase = { ...chain, base: chain.base ? { ...chain.base, displayed: "stretched" as const } : null };
    expect(nonLinearInputNote(section, compositeInput(compositeInputDisplayed(stretchedBase, "base")))).toBeNull();
  });

  it("on a toned composite input, says nothing", () => {
    expect(nonLinearInputNote(section, compositeInput("toned"))).toBeNull();
  });
});

describe("nonLinearInputNote wording", () => {
  it("drops the hedge only on the composite, where the input tier is known", () => {
    for (const section of SECTIONS) expect(FILE_NOTES[section].startsWith("No stretch step in this chain: ")).toBe(true);
    expect(nonLinearInputNote("curves", fileInput(null))).toContain("if the input is linear");
    expect(nonLinearInputNote("local_contrast", compositeInput("linear"))).not.toContain("if ");
    expect(nonLinearInputNote("hdr", compositeInput("linear"))).not.toContain("if ");
  });

  it("Curves after LHE says so only when no stretch step precedes the LHE step", () => {
    expect(nonLinearInputNote("curves", { kind: "file", upstream: upstreamSteps(chainOf("localContrast"), "tone") })).toBe(FILE_NOTES.curves);
    expect(nonLinearInputNote("curves", { kind: "file", upstream: upstreamSteps(chainOf("background", "stretch", "localContrast"), "tone") })).toBeNull();
    expect(nonLinearInputNote("curves", { kind: "file", upstream: upstreamSteps(chainOf("maskedStretch", "localContrast"), "tone") })).toBeNull();
  });
});

describe("upstreamSteps", () => {
  it("lists the chain steps at earlier stages than the step, in chain order", () => {
    const chain = chainOf("background", "denoise", "stretch", "localContrast", "tone");
    expect(upstreamSteps(chain, "localContrast")).toEqual(["background", "denoise", "stretch"]);
    expect(upstreamSteps(chain, "tone")).toEqual(["background", "denoise", "stretch", "localContrast"]);
    expect(upstreamSteps(chain, "background")).toEqual([]);
  });

  it("is empty on the original", () => {
    expect(upstreamSteps(EMPTY_CHAIN, "localContrast")).toEqual([]);
  });
});

describe("compositeInputDisplayed", () => {
  it("reads the tier of the chain entry the step reads from, and nothing for the base", () => {
    const chain = compositeChainOf({ background: entry("linear"), stretch: entry("stretched") });
    expect(compositeInputDisplayed(chain, "background")).toBe("linear");
    expect(compositeInputDisplayed(chain, "stretch")).toBe("stretched");
    expect(compositeInputDisplayed(chain, "deconv")).toBeNull();
    expect(compositeInputDisplayed(chain, "base")).toBeNull();
  });
});

describe("hdrLinearInputHint", () => {
  it("names this tab's Stretch and Masked on a file, with GHS inside Stretch", () => {
    expect(hdrLinearInputHint(false)).toBe("HDRMT works on stretched data. Run Stretch (Arcsinh or GHS) or Masked in this tab, then run HDRMT on its output.");
  });

  it("adds GHS in Compose › Stretch on the composite", () => {
    expect(hdrLinearInputHint(true)).toBe(
      "HDRMT works on stretched data. Run Stretch or Masked in this tab, or GHS in Compose › Stretch, then run HDRMT on its output.",
    );
  });

  it("says where GHS runs: inside Stretch in this tab on a file, in Compose › Stretch on the composite", () => {
    expect(hdrLinearInputHint(false)).toContain("Stretch (Arcsinh or GHS) or Masked in this tab");
    expect(hdrLinearInputHint(false)).not.toContain("Compose");
    expect(hdrLinearInputHint(true)).toContain("or GHS in Compose › Stretch");
    expect(hdrLinearInputHint(true)).not.toContain("(Arcsinh or GHS)");
    for (const hint of [hdrLinearInputHint(false), hdrLinearInputHint(true)]) expect(hint).not.toContain("Arcsinh, GHS or Masked");
  });
});

describe("curvesAutoStfNote", () => {
  it("says the auto-STF ran before the curve when the backend normalized the input", () => {
    expect(curvesAutoStfNote({ input_normalized: { min: 0, max: 812 }, curves_applied: true })).toBe("Auto-STF applied before the curve");
  });

  it("says before the levels when only the levels were applied", () => {
    expect(curvesAutoStfNote({ input_normalized: { min: 0, max: 812 }, curves_applied: false })).toBe("Auto-STF applied before the levels");
  });

  it("says nothing on a display-referred input", () => {
    expect(curvesAutoStfNote({ input_normalized: null, curves_applied: true })).toBeNull();
  });
});
