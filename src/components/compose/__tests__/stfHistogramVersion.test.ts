import { beforeEach, describe, expect, it, vi } from "vitest";

const { computeHistogram } = vi.hoisted(() => ({ computeHistogram: vi.fn() }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/analysis", () => ({ computeHistogram }));

import { hooks } from "../../../test/hookRuntime";
import StfHistogram from "../StfHistogram";

const CHANNELS = [
  { key: "__composite_r", color: "#ef4444" },
  { key: "__composite_g", color: "#22c55e" },
  { key: "__composite_b", color: "#3b82f6" },
];

beforeEach(() => {
  hooks.mount();
  computeHistogram.mockReset().mockResolvedValue({ bins: [1, 2, 3], data_min: 0, data_max: 1 });
});

describe("StfHistogram on the composite", () => {
  it("reads the histogram again when the composite under the same cache keys changes", () => {
    const render = (version: number) => hooks.render(StfHistogram, { channels: CHANNELS, symmetryPoint: 0.01, version });
    render(1);
    expect(computeHistogram).toHaveBeenCalledTimes(3);
    render(1);
    expect(computeHistogram).toHaveBeenCalledTimes(3);
    render(2);
    expect(computeHistogram).toHaveBeenCalledTimes(6);
  });
});
