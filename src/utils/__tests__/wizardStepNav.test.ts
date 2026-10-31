import { describe, expect, it } from "vitest";
import { INITIAL_STATE, type WizardState } from "../wizard";
import { stepNavLabel, wizardStepNav } from "../wizardStepNav";

const M16 = { r: ["/m16/673nmos.fits"], g: ["/m16/656nmos.fits"], b: ["/m16/502nmos.fits"] };
const M16_ALIGNED = { r: "__wizard_ch_tk1_r_aligned", g: "__wizard_ch_tk1_g_aligned", b: "__wizard_ch_tk1_b_aligned" };

function stateWith(files: Record<string, string[]>, extra: Partial<WizardState> = {}): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: files[b.id] ?? [] })),
    ...extra,
  };
}

describe("wizardStepNav", () => {
  it("Next skips a disabled Stack step and lands on Align for single-file channels", () => {
    expect(wizardStepNav("channels", stateWith(M16))).toEqual({
      previous: null,
      next: { id: "align", label: "Align", reason: null },
    });
  });

  it("after Align, Previous skips Stack back to Channels and Next lands on Crop", () => {
    const aligned = stateWith(M16, { alignedPaths: M16_ALIGNED, completedSteps: { align: true } });
    expect(wizardStepNav("align", aligned)).toEqual({
      previous: { id: "channels", label: "Channels", reason: null },
      next: { id: "crop", label: "Crop", reason: null },
    });
  });

  it("before Align, Next skips Crop, which waits for Align, and lands on BG", () => {
    expect(wizardStepNav("align", stateWith(M16)).next).toEqual({ id: "background", label: "BG", reason: null });
  });

  it("from BG, Previous skips Crop back to Align until Align has run", () => {
    expect(wizardStepNav("background", stateWith(M16)).previous).toEqual({ id: "align", label: "Align", reason: null });
    expect(wizardStepNav("background", stateWith(M16, { alignedPaths: M16_ALIGNED })).previous).toEqual({ id: "crop", label: "Crop", reason: null });
  });

  it("names the next step and why it is not ready when no later step is ready", () => {
    const unstacked = stateWith({ r: ["/r1.fits", "/r2.fits"], g: ["/g1.fits", "/g2.fits"], b: ["/b1.fits", "/b2.fits"] });
    expect(wizardStepNav("stack", unstacked)).toEqual({
      previous: { id: "channels", label: "Channels", reason: null },
      next: { id: "align", label: "Align", reason: "stack R, G and B first (2+ frames each), or keep one file per channel" },
    });
  });

  it("points one unstacked channel at Crop, since Align needs a second channel", () => {
    const mono = stateWith({ ha: ["/ha1.fits", "/ha2.fits", "/ha3.fits"] });
    expect(wizardStepNav("stack", mono).next).toEqual({
      id: "crop",
      label: "Crop",
      reason: "stack Hα first (2+ frames), or keep one file per channel",
    });
  });

  it("asks for frames on an empty wizard", () => {
    expect(wizardStepNav("channels", INITIAL_STATE)).toEqual({
      previous: null,
      next: { id: "stack", label: "Stack", reason: "assign frames in step 1" },
    });
  });

  it("goes from Stretch to Export, skipping Adjust while no composite exists", () => {
    expect(wizardStepNav("stretch", stateWith(M16)).next).toEqual({ id: "export", label: "Export", reason: null });
    expect(wizardStepNav("export", stateWith(M16))).toEqual({
      previous: { id: "stretch", label: "Stretch", reason: null },
      next: null,
    });
  });

  it("keeps Adjust in the walk once Blend made the composite", () => {
    const blended = stateWith(M16, { compositeReady: true, completedSteps: { blend: true } });
    expect(wizardStepNav("stretch", blended).next).toEqual({ id: "adjust", label: "Adjust", reason: null });
    expect(wizardStepNav("export", blended).previous).toEqual({ id: "adjust", label: "Adjust", reason: null });
  });
});

describe("stepNavLabel", () => {
  it("names the step after Previous or Next, and the reason after a comma", () => {
    expect(stepNavLabel("Previous", { id: "channels", label: "Channels", reason: null })).toBe("Previous: Channels");
    expect(stepNavLabel("Next", { id: "align", label: "Align", reason: null })).toBe("Next: Align");
    expect(stepNavLabel("Next", { id: "crop", label: "Crop", reason: "stack Hα first (2+ frames), or keep one file per channel" }))
      .toBe("Next: Crop, stack Hα first (2+ frames), or keep one file per channel");
  });
});
