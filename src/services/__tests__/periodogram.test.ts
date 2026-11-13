import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { lightCurvePeriodogram } from "../periodogram";
import type { PeriodogramResult } from "../../shared/types/periodogram";

const RESULT: PeriodogramResult = {
  frequency: [0.25, 0.5],
  power: [0.1, 0.9],
  best_frequency: 0.5,
  best_period: 2,
  best_power: 0.9,
  n_points: 4,
  n_dropped: 0,
  n_frequencies: 2,
  baseline_days: 3,
  frequency_step: 0.25,
  normalization: "standard",
  weighted: false,
  false_alarm_probability: 0.4,
  fap_method: "baluev",
  notes: ["equal weights"],
  elapsed_ms: 1,
};

describe("lightCurvePeriodogram", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins light_curve_periodogram_cmd with nulls for absent options", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    const res = await lightCurvePeriodogram([0, 1, 2, 3], [1, 2, 1, 2], null);
    expect(typedInvokeMock).toHaveBeenCalledWith("light_curve_periodogram_cmd", {
      time: [0, 1, 2, 3],
      value: [1, 2, 1, 2],
      err: null,
      minFrequency: null,
      maxFrequency: null,
      samplesPerPeak: null,
    });
    expect(res).toBe(RESULT);
  });

  it("forwards errors and the grid options", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    await lightCurvePeriodogram([0, 1, 2, 3], [1, 2, 1, 2], [0.1, 0.1, 0.1, 0.1], {
      minFrequency: 0.2,
      maxFrequency: 0.5,
      samplesPerPeak: 10,
    });
    expect(typedInvokeMock).toHaveBeenCalledWith("light_curve_periodogram_cmd", {
      time: [0, 1, 2, 3],
      value: [1, 2, 1, 2],
      err: [0.1, 0.1, 0.1, 0.1],
      minFrequency: 0.2,
      maxFrequency: 0.5,
      samplesPerPeak: 10,
    });
  });
});
