import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { tsoTransitSpectrum } from "../tsoTransit";
import type { TsoTransitBin, TsoTransitFitRequest, TsoTransitSpectrum } from "../../shared/types/tsoTransit";

describe("tso transit service", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("tsoTransitSpectrum pins tso_transit_spectrum_cmd with the series, the curve request and the fit", async () => {
    const fitted: TsoTransitBin = {
      band_um: [0.6, 1.0],
      n_points: 240,
      n_in_transit: 18,
      weighted: true,
      rp: 7.6999,
      rp_err: 0.0005,
      depth: 59.289,
      depth_err: 0.007,
      u1: 0.2,
      u1_err: null,
      u2: 0.25,
      u2_err: null,
      chi2: 468409.87,
      dof: 237,
      rms: 0.04,
      grazing: true,
      npixels_ratio: 0.6396,
      pixel_loss: true,
      error: null,
      notes: [],
      warnings: ["reduced chi2 = 1976.4: the model or the errors do not describe the scatter"],
    };
    const unfitted: TsoTransitBin = {
      ...fitted,
      band_um: [5.3, 6.0],
      n_points: 0,
      n_in_transit: 0,
      rp: null,
      rp_err: null,
      depth: null,
      depth_err: null,
      u1: null,
      u2: null,
      chi2: null,
      dof: -3,
      rms: null,
      grazing: false,
      npixels_ratio: null,
      pixel_loss: false,
      error: "no integration has a flux in this band",
      warnings: [],
    };
    const spectrum: TsoTransitSpectrum = {
      set_id: "4b385da81d5c70b8",
      label: "NRS1",
      series: { detector: "NRS1", sporder: 0 },
      mode: "pipeline",
      time_scale: "bmjd_tdb",
      fit: {
        period_days: 1.4079405,
        t0: 60061.507668,
        a_rs: 336,
        b: 7.16,
        rp_start: 7.73,
        u1: 0.2,
        u2: 0.25,
        limb_darkening: "fixed",
        baseline_order: 1,
        exclude_rows: [3],
        scale_errors: false,
      },
      model: "Mandel and Agol 2002, quadratic limb darkening, circular orbit",
      n_integrations: 240,
      n_excluded: 1,
      n_in_transit: 18,
      bins: [fitted, unfitted],
      notes: ["1 integration(s) left out of every bin"],
      warnings: ["1 of 2 bins could not be fitted: each says why"],
    };
    typedInvokeMock.mockResolvedValue(spectrum);
    const res = await tsoTransitSpectrum(
      ["C:/d/a_x1dints.fits"],
      { detector: "NRS1", sporder: 0 },
      { mode: "pipeline", bins: [[0.6, 1.0]] },
      {
        period_days: 1.4079405,
        t0: 60061.507668,
        a_rs: 336,
        b: 7.16,
        rp_start: 7.73,
        u1: 0.2,
        u2: 0.25,
        limb_darkening: "fixed",
        baseline_order: 1,
        exclude_rows: [3],
        scale_errors: false,
      },
    );
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("tso_transit_spectrum_cmd", {
      paths: ["C:/d/a_x1dints.fits"],
      series: { detector: "NRS1", sporder: 0 },
      request: { mode: "pipeline", bins: [[0.6, 1.0]] },
      fit: {
        period_days: 1.4079405,
        t0: 60061.507668,
        a_rs: 336,
        b: 7.16,
        rp_start: 7.73,
        u1: 0.2,
        u2: 0.25,
        limb_darkening: "fixed",
        baseline_order: 1,
        exclude_rows: [3],
        scale_errors: false,
      },
    });
    expect(res).toBe(spectrum);
    expect(res.bins[1].depth).toBeNull();
    expect(res.bins[0].u1_err).toBeNull();
    expect(Object.keys(spectrum).sort()).toEqual([
      "bins",
      "fit",
      "label",
      "mode",
      "model",
      "n_excluded",
      "n_in_transit",
      "n_integrations",
      "notes",
      "series",
      "set_id",
      "time_scale",
      "warnings",
    ]);
    expect(Object.keys(fitted).sort()).toEqual([
      "band_um",
      "chi2",
      "depth",
      "depth_err",
      "dof",
      "error",
      "grazing",
      "n_in_transit",
      "n_points",
      "notes",
      "npixels_ratio",
      "pixel_loss",
      "rms",
      "rp",
      "rp_err",
      "u1",
      "u1_err",
      "u2",
      "u2_err",
      "warnings",
      "weighted",
    ]);
  });

  it("the fitted-spectrum types keep the shapes the backend serialises", () => {
    expectTypeOf<TsoTransitBin["depth"]>().toEqualTypeOf<number | null>();
    expectTypeOf<TsoTransitBin["error"]>().toEqualTypeOf<string | null>();
    expectTypeOf<TsoTransitBin["warnings"]>().toEqualTypeOf<string[]>();
    expectTypeOf<TsoTransitSpectrum["fit"]["limb_darkening"]>().toEqualTypeOf<"fixed" | "free">();
    expectTypeOf<TsoTransitSpectrum["fit"]["scale_errors"]>().toEqualTypeOf<boolean>();
    expectTypeOf<TsoTransitFitRequest["exclude_rows"]>().toEqualTypeOf<number[] | undefined>();
    expectTypeOf<TsoTransitSpectrum["bins"]>().toEqualTypeOf<TsoTransitBin[]>();
  });
});
