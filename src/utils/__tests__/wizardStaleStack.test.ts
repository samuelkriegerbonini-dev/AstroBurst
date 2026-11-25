import { describe, expect, it } from "vitest";
import { wizardReducer, type WizardAction } from "../../context/ComposeWizardContext";
import { INITIAL_STATE, resolveChannelPath, STEPS, type WizardState } from "../wizard";

const HA = ["/ha/h1.fits", "/ha/h2.fits", "/ha/h3.fits"];
const STACK = "/out/stacked_ha_1.fits";

function run(state: WizardState, ...actions: WizardAction[]): WizardState {
  return actions.reduce(wizardReducer, state);
}

function stackedAndBlended(): WizardState {
  const bins = INITIAL_STATE.bins.map((b) => ({ ...b, files: b.id === "ha" ? HA : b.id === "oiii" ? ["/o.fits"] : [] }));
  return run(
    INITIAL_STATE,
    { type: "SET_BINS", bins },
    { type: "SET_STACKED", channelId: "ha", path: STACK, files: HA },
    { type: "SET_ALIGNED", paths: { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" } },
    { type: "SET_COMPOSITE_READY", ready: true },
    { type: "COMPLETE_STEP", stepId: "blend" },
  );
}

const reason = (s: WizardState, id: string) => STEPS.find((step) => step.id === id)!.blockedReason(s);

describe("a stack built before frames were excluded", () => {
  it("is out of date: Align, Blend and Export stop using it and say why", () => {
    const s = run(stackedAndBlended(), { type: "SET_EXCLUDED_FILES", binId: "ha", files: ["/ha/h3.fits"] });
    expect(s.stackedPaths.ha).toBe(STACK);
    expect(resolveChannelPath(s, "ha", "stacked")).not.toBe(STACK);
    expect(s.compositeReady).toBe(false);
    expect(s.alignedPaths).toEqual({});
    expect(s.completedSteps.blend).toBeUndefined();
    for (const id of ["align", "blend", "export"]) {
      expect(reason(s, id), id).toBe("the Hα stack is out of date (built from 3 frames, 2 selected now); stack Hα again");
    }
  });

  it("is current again once the same frames are selected, or once the channel is stacked again", () => {
    const excluded = run(stackedAndBlended(), { type: "SET_EXCLUDED_FILES", binId: "ha", files: ["/ha/h3.fits"] });
    const restored = run(excluded, { type: "SET_EXCLUDED_FILES", binId: "ha", files: [] });
    expect(resolveChannelPath(restored, "ha", "stacked")).toBe(STACK);
    expect(reason(restored, "align")).toBeNull();
    const restacked = run(excluded, { type: "SET_STACKED", channelId: "ha", path: "/out/stacked_ha_2.fits", files: ["/ha/h1.fits", "/ha/h2.fits"] });
    expect(resolveChannelPath(restacked, "ha", "stacked")).toBe("/out/stacked_ha_2.fits");
    expect(reason(restacked, "align")).toBeNull();
  });

  it("is left aside when one frame remains, which the channel then uses directly", () => {
    const s = run(stackedAndBlended(), { type: "SET_EXCLUDED_FILES", binId: "ha", files: ["/ha/h1.fits", "/ha/h2.fits"] });
    expect(resolveChannelPath(s, "ha", "stacked")).toBe("/ha/h3.fits");
    expect(reason(s, "align")).toBeNull();
  });
});
