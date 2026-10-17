import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OpSpec } from "../../shared/types/ops";
import { historyRerunStore, sectionForOp } from "../historyRerunStore";

const SCOPE = { imageKey: "C:/data/a.fits", basePath: null };
const DENOISE: OpSpec = { op: "wavelet_denoise", v: 1, numScales: 5, thresholds: [3, 2.5, 2, 1.5, 1], linearDenoise: true, layerBias: null };
const TONE: OpSpec = { op: "tone", v: 1, levels: { black: 0.1, gamma: 1, white: 0.9 } };

beforeEach(() => {
  historyRerunStore.clear();
});

describe("historyRerunStore", () => {
  it("hands a pending request to the panel of its section once", () => {
    const listener = vi.fn();
    const stop = historyRerunStore.subscribe(listener);
    const pending = historyRerunStore.request({ section: "curves", op: TONE, node: "n0004", downstream: false, scope: SCOPE });
    expect(listener).toHaveBeenCalledTimes(1);
    expect(historyRerunStore.peek("curves")).toBe(pending);
    expect(historyRerunStore.peek("denoise")).toBeNull();
    historyRerunStore.consume(pending.seq - 1);
    expect(historyRerunStore.peek("curves")).toBe(pending);
    historyRerunStore.consume(pending.seq);
    expect(historyRerunStore.peek("curves")).toBeNull();
    expect(historyRerunStore.armed()).toBeNull();
    stop();
  });

  it("arms a downstream re-run for the next run of the same step on the same image", () => {
    const pending = historyRerunStore.request({ section: "denoise", op: DENOISE, node: "n0002", downstream: true, scope: SCOPE });
    historyRerunStore.consume(pending.seq);
    expect(historyRerunStore.armed()?.node).toBe("n0002");
    expect(historyRerunStore.takeArmed({ imageKey: "C:/data/b.fits", basePath: null }, "wavelet_denoise")).toBeNull();
    expect(historyRerunStore.takeArmed(SCOPE, "tone")).toBeNull();
    expect(historyRerunStore.takeArmed(SCOPE, "wavelet_denoise")?.node).toBe("n0002");
    expect(historyRerunStore.takeArmed(SCOPE, "wavelet_denoise")).toBeNull();
  });

  it("a new request replaces an armed one and disarm cancels it", () => {
    historyRerunStore.request({ section: "denoise", op: DENOISE, node: "n0002", downstream: true, scope: SCOPE });
    historyRerunStore.request({ section: "curves", op: TONE, node: "n0004", downstream: false, scope: SCOPE });
    expect(historyRerunStore.armed()).toBeNull();
    historyRerunStore.request({ section: "denoise", op: DENOISE, node: "n0002", downstream: true, scope: SCOPE });
    historyRerunStore.disarm();
    expect(historyRerunStore.armed()).toBeNull();
  });

  it("maps every recorded op to the Processing section that edits it", () => {
    expect(sectionForOp(DENOISE)).toBe("denoise");
    expect(sectionForOp(TONE)).toBe("curves");
    expect(sectionForOp({ op: "arcsinh", v: 1, factor: 10 })).toBe("stretch");
    expect(sectionForOp({ op: "ghs", v: 1, d: 5, b: 2, sp: 0.05, lp: 0, hp: 1 })).toBeNull();
    expect(sectionForOp({ op: "lhe", v: 1, kernelRadius: 64, contrastLimit: 2, amount: 1, histBits: 8, circular: true })).toBe("local_contrast");
    expect(sectionForOp({ op: "pixelmath", v: 1, expression: "$T", slots: [], truncate: true, rescale: false })).toBe("pixelmath");
  });
});
