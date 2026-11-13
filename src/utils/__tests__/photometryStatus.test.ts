import { describe, it, expect } from "vitest";
import { statusLineModel } from "../photometryStatus";
import { pixelScaleText } from "../coordFormat";
import type { PhotCal } from "../../services/analysis";
import type { PhotometryStatus } from "../../shared/types/photometryStatus";

const JWST_PHOTCAL: PhotCal = {
  convention: { kind: "jwst_mjy_sr", pixar_sr: 2.29249e-14, pixar_a2: 0.00097536, derived_pixar: false },
  bunit: "MJy/sr",
  notes: [],
  warnings: [],
  label: "JWST MJy/sr label",
};

const NO_ERROR_LAYER_TITLE =
  "No error layer was found for this image. The flux error comes from the scatter of the sky annulus, plus the photon term when a gain is given.";
const NO_WCS_TITLE = "No celestial WCS: radii in arcsec and sky positions are not available.";

function status(overrides: Partial<PhotometryStatus> = {}): PhotometryStatus {
  return {
    photcal: JWST_PHOTCAL,
    calibration_origin: "JWST MJy/sr, PIXAR_SR from header",
    missing_reason: null,
    bunit: "MJy/sr",
    summed_unit: "MJy/sr x pix",
    err: { origin: "ERR plane (HDU 2)" },
    dq: { source: "DQ plane (HDU 3), jwst flags", exclusion_available: true, unavailable_reason: null },
    pixel_scale_arcsec: 0.031,
    pixel_scale_axes_arcsec: [0.031, 0.031],
    filter: { dataset: "JWST", filter: "F200W", pivot_um: 1.988648, source: "header FILTER", warning: null },
    warnings: [],
    ...overrides,
  };
}

const chip = (s: PhotometryStatus, key: string) => statusLineModel(s).chips.find((c) => c.key === key);

describe("statusLineModel", () => {
  it("states the calibration sentence of a calibrated image", () => {
    const model = statusLineModel(status());
    expect(model.calibrated).toBe(true);
    expect(model.calibration).toBe("JWST MJy/sr, PIXAR_SR from header");
    expect(statusLineModel(status({ calibration_origin: null })).calibration).toBe("JWST MJy/sr label");
  });

  it("states why an image has no calibration", () => {
    const model = statusLineModel(
      status({ photcal: null, calibration_origin: null, missing_reason: "no photometric calibration in the header (BUNIT 'adu')" }),
    );
    expect(model.calibrated).toBe(false);
    expect(model.calibration).toBe("none: no photometric calibration in the header (BUNIT 'adu')");
    expect(statusLineModel(status({ photcal: null, calibration_origin: null, missing_reason: null })).calibration).toBe("none");
  });

  it("lists unit, errors, flags, scale and filter chips in that order with their texts", () => {
    expect(pixelScaleText(0.031)).toBe('0.03100"/px');
    const { chips } = statusLineModel(status());
    expect(chips.map((c) => c.key)).toEqual(["unit", "errors", "flags", "scale", "filter"]);
    expect(chips.map((c) => c.text)).toEqual([
      "unit MJy/sr",
      "errors: ERR plane (HDU 2)",
      "flags: DQ plane (HDU 3), jwst flags",
      pixelScaleText(0.031),
      "F200W 1.989 um",
    ]);
    expect(chips.map((c) => c.tone)).toEqual(["ok", "ok", "ok", "ok", "ok"]);
    expect(chips[0].title).toBe("An aperture sum is in MJy/sr x pix");
    expect(chips[4].title).toBe("JWST F200W, pivot 1.988648 um, from the header FILTER");
  });

  it("says what is missing in muted chips", () => {
    expect(chip(status({ bunit: null, summed_unit: null }), "unit")).toEqual({ key: "unit", text: "unit not stated", tone: "muted", title: null });
    expect(chip(status({ err: null }), "errors")).toEqual({ key: "errors", text: "errors: sky scatter only", tone: "muted", title: NO_ERROR_LAYER_TITLE });
    expect(chip(status({ dq: null }), "flags")).toMatchObject({ text: "flags: none", tone: "muted" });
    expect(chip(status({ pixel_scale_arcsec: null }), "scale")).toEqual({ key: "scale", text: "no pixel scale", tone: "muted", title: NO_WCS_TITLE });
    expect(statusLineModel(status({ filter: null })).chips.map((c) => c.key)).toEqual(["unit", "errors", "flags", "scale"]);
  });

  it("warns when the flag layer cannot exclude and gives the reason as the title", () => {
    const reason = "the DQ plane (HDU 2) could not be read as an integer mask of the image size";
    expect(chip(status({ dq: { source: "DQ plane (HDU 2)", exclusion_available: false, unavailable_reason: reason } }), "flags")).toEqual({
      key: "flags",
      text: "flags: DQ plane (HDU 2) (exclusion not available)",
      tone: "warn",
      title: reason,
    });
  });

  it("marks a filter with a registry warning", () => {
    const warning = "the file name names F444W but the header names F200W; the header wins";
    const filter = chip(status({ filter: { dataset: "JWST", filter: "F200W", pivot_um: 1.988648, source: "header FILTER", warning } }), "filter");
    expect(filter?.tone).toBe("warn");
    expect(filter?.title?.endsWith(`. ${warning}`)).toBe(true);
    expect(filter?.title).toBe(`JWST F200W, pivot 1.988648 um, from the header FILTER. ${warning}`);
  });

  it("passes the warnings through unchanged", () => {
    const warnings = ["pixel scales differ per axis (0.03100 x 0.03200 arcsec); radii in arcsec use the square root of the pixel area (0.03150 arcsec)", "second"];
    expect(statusLineModel(status({ warnings })).warnings).toEqual(warnings);
  });

  it("has no unit title when a sum has the pixel unit", () => {
    expect(chip(status({ bunit: "adu", summed_unit: "adu" }), "unit")).toEqual({ key: "unit", text: "unit adu", tone: "ok", title: null });
  });
});
