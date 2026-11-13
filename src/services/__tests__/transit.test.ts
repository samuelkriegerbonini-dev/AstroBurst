import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { fitTransit } from "../transit";
import type { TransitFitRequest, TransitFitResult } from "../../shared/types/transit";

const REQ: TransitFitRequest = {
  time: [2460310.4, 2460310.6],
  flux: [1, 0.99],
  err: null,
  period: 3,
  t0: { value: 2460310.5, free: true },
  rp: { value: 0.1, free: true },
  a_rs: { value: 9, free: false },
  b: { value: 0.3, free: false },
  u1: { value: 0.4, free: false },
  u2: { value: 0.25, free: false },
  baseline_order: 1,
  scale_errors: false,
};

const RESULT: TransitFitResult = {
  model: "Mandel and Agol 2002, quadratic limb darkening, circular orbit",
  period: 3,
  parameters: [
    { name: "t0", value: 2460310.5, err: 0.0001, free: true, at_bound: false },
    { name: "rp", value: 0.1, err: 0.001, free: true, at_bound: false },
    { name: "a_rs", value: 9, err: null, free: false, at_bound: false },
    { name: "b", value: 0.3, err: null, free: false, at_bound: false },
    { name: "u1", value: 0.4, err: null, free: false, at_bound: false },
    { name: "u2", value: 0.25, err: null, free: false, at_bound: false },
  ],
  baseline: { order: 1, reference_time: 2460310.5, coefficients: [1, 0], errors: [0.001, null] },
  depth: 0.01,
  depth_err: 0.0002,
  chi2: 0,
  dof: 0,
  reduced_chi2: 0,
  weighted: false,
  error_scale: 1,
  rms: 0,
  rms_ppm: 0,
  n_points: 2,
  n_in_transit: 1,
  iterations: 3,
  converged: true,
  free_names: ["t0", "rp", "c0", "c1"],
  time: [],
  model_flux: [],
  baseline_flux: [],
  residual: [],
  notes: [],
  warnings: [],
  correlation: null,
  derived: {
    inclination_deg: 88.1, inclination_err: null,
    duration_t14_days: 0.11, duration_t14_err: null,
    duration_t23_days: 0.09, duration_t23_err: null,
    stellar_density_cgs: 1.5, stellar_density_err: null,
  },
  time_averaging: { cadence_days: 0.2, bin_points: [1], n_bins: [2], rms_ppm: [0], expected_ppm: [0], beta: 1 },
  curve_time: [],
  curve_flux: [],
  curve_baseline: [],
  kind: "transit",
  elapsed_ms: 1,
};

describe("fitTransit", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins light_curve_transit_fit_cmd with the request as given and the series by default", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    const res = await fitTransit(REQ);
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("light_curve_transit_fit_cmd", { request: REQ, includeSeries: true });
    expect(typedInvokeMock.mock.calls[0][1].request).toBe(REQ);
    expect(res).toBe(RESULT);
  });

  it("forwards includeSeries false for a fit per bin", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    await fitTransit(REQ, { includeSeries: false });
    expect(typedInvokeMock.mock.calls[0][1]).toEqual({ request: REQ, includeSeries: false });
  });

  it("the types allow an eclipse request", async () => {
    typedInvokeMock.mockResolvedValue({ ...RESULT, kind: "eclipse" });
    const eclipse: TransitFitRequest = { ...REQ, kind: "eclipse", flux_ratio: { value: 0.001, free: true } };
    const res = await fitTransit(eclipse);
    expect(typedInvokeMock.mock.calls[0][1].request).toBe(eclipse);
    expect(typedInvokeMock.mock.calls[0][1].request).toEqual({ ...REQ, kind: "eclipse", flux_ratio: { value: 0.001, free: true } });
    expect(res.kind).toBe("eclipse");
  });
});
