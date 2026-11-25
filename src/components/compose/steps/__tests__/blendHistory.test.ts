import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { mocks, clicks } = vi.hoisted(() => ({
  mocks: { blendChannels: vi.fn() },
  clicks: {} as Record<string, () => Promise<unknown>>,
}));

vi.mock("../../../../services/compose", () => ({
  blendChannels: mocks.blendChannels,
  lrgbCombineComposite: vi.fn(),
  renderLinearCompositePreview: vi.fn(),
}));

vi.mock("../../../../services/channelLevels", () => ({ measureChannelLevels: vi.fn() }));

vi.mock("../../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn(async () => "/out") }));

vi.mock("../../../../hooks/usePointingOverlap", () => ({ usePointingOverlap: () => ({ disjointPairs: [] }) }));

vi.mock("../../../../context/CompositeContext", () => {
  const stf = { shadow: 0, midtone: 0.5, highlight: 1 };
  return { useCompositeStf: () => ({ compositeStfR: stf, compositeStfG: stf, compositeStfB: stf, compositeStfLinked: true }) };
});

vi.mock("../../../ui", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../ui")>()),
  RunButton: (props: { label: string; onClick: () => Promise<unknown> }) => {
    clicks[props.label] = props.onClick;
    return null;
  },
}));

import BlendStep from "../BlendStep";
import { applyCompositeOp, EMPTY_COMPOSITE_HISTORY, INITIAL_STATE, type BlendWeight, type CompositeOp, type WizardState } from "../../../../utils/wizard";
import { BLEND_WEIGHTS_CHANGED, blendWeightsSignature } from "../../../../utils/blendWeights";

const WEIGHTS: BlendWeight[] = [
  { channelId: "ha", r: 0.3, g: 0.8, b: 0.2 },
  { channelId: "oiii", r: 0, g: 0.15, b: 0.85 },
  { channelId: "sii", r: 0.7, g: 0.3, b: 0 },
];

const FRAMES: Record<string, string[]> = { ha: ["/d/frame_1.fits"], oiii: ["/d/frame_2.fits"], sii: ["/d/frame_3.fits"] };

function state(extra: Partial<WizardState> = {}): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: FRAMES[b.id] ?? [] })),
    blendWeights: WEIGHTS,
    blendPreset: "custom",
    levelMatch: false,
    ...extra,
  };
}

function render(s: WizardState, onCompositeOp: (op: CompositeOp) => void = vi.fn()): string {
  return renderToStaticMarkup(createElement(BlendStep, {
    state: s,
    files: [],
    onLevelMatchChange: vi.fn(),
    onLevelScales: vi.fn(),
    onWeightsChange: vi.fn(),
    onCompositeReady: vi.fn(),
    onCompositeOp,
  }));
}

beforeEach(() => {
  mocks.blendChannels.mockReset();
  for (const key of Object.keys(clicks)) delete clicks[key];
});

describe("Blend weights and the composite they made", () => {
  it("records the weight matrix it sent, so the exported HISTORY can reproduce the colours", async () => {
    mocks.blendChannels.mockResolvedValue({ previewUrl: "asset://out/blend.png", dimensions: [10, 10], channel_count: 3, elapsed_ms: 1 });
    const onCompositeOp = vi.fn();
    render(state(), onCompositeOp);
    await clicks["Run Blend"]();
    const op = onCompositeOp.mock.calls[0][0] as CompositeOp;
    expect(op).toMatchObject({ kind: "blend", preset: "custom", signature: blendWeightsSignature(WEIGHTS, false) });
    expect(applyCompositeOp(EMPTY_COMPOSITE_HISTORY, op).weights).toEqual([
      "Blend ha: R=0.300 G=0.800 B=0.200",
      "Blend oiii: R=0.000 G=0.150 B=0.850",
      "Blend sii: R=0.700 G=0.300 B=0.000",
    ]);
  });

  it("says when the weights on screen are not the ones the composite was blended with", () => {
    const blended = applyCompositeOp(EMPTY_COMPOSITE_HISTORY, { kind: "blend", preset: "custom", signature: blendWeightsSignature(WEIGHTS, false) });
    expect(render(state({ compositeReady: true, compositeHistory: blended }))).not.toContain(BLEND_WEIGHTS_CHANGED);
    const edited = WEIGHTS.map((w) => (w.channelId === "ha" ? { ...w, r: 0.4 } : w));
    expect(render(state({ compositeReady: true, compositeHistory: blended, blendWeights: edited }))).toContain(BLEND_WEIGHTS_CHANGED);
    expect(render(state({ compositeReady: true, compositeHistory: blended, levelMatch: true }))).toContain(BLEND_WEIGHTS_CHANGED);
    expect(BLEND_WEIGHTS_CHANGED).toBe("Weights changed since the last Blend: Run Blend to apply.");
  });
});
