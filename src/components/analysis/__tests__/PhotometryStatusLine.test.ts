import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { PhotometryStatus } from "../../../shared/types/photometryStatus";

const hook = vi.hoisted(() => ({
  doc: { loading: false, status: null, error: null } as Record<string, unknown>,
  calls: [] as unknown[][],
}));

vi.mock("../../../hooks/usePhotometryStatus", () => ({
  usePhotometryStatus: (...args: unknown[]) => {
    hook.calls.push(args);
    return hook.doc;
  },
}));

import PhotometryStatusLine from "../PhotometryStatusLine";
import { pixelScaleText } from "../../../utils/coordFormat";

const PATH = "C:/d/a.fits";
const WARNINGS = ["pixel scales differ per axis", "photometry on processed data (stack)"];

const STATUS: PhotometryStatus = {
  photcal: {
    convention: { kind: "jwst_mjy_sr", pixar_sr: 2.29249e-14, pixar_a2: 0.00097536, derived_pixar: false },
    bunit: "MJy/sr",
    notes: [],
    warnings: [],
    label: "JWST MJy/sr, PIXAR_SR from header",
  },
  calibration_origin: "JWST MJy/sr, PIXAR_SR from header",
  missing_reason: null,
  bunit: "MJy/sr",
  summed_unit: "MJy/sr x pix",
  err: { origin: "ERR plane (HDU 2)" },
  dq: { source: "DQ plane (HDU 3), jwst flags", exclusion_available: true, unavailable_reason: null },
  pixel_scale_arcsec: 0.031,
  pixel_scale_axes_arcsec: [0.031, 0.031],
  filter: { dataset: "JWST", filter: "F200W", pivot_um: 1.988648, source: "header FILTER", warning: null },
  warnings: WARNINGS,
};

const render = (props: Record<string, unknown>) => renderToStaticMarkup(createElement(PhotometryStatusLine, props as never));
const escaped = (text: string) => text.replace(/"/g, "&quot;");

describe("PhotometryStatusLine", () => {
  afterEach(() => {
    hook.doc = { loading: false, status: null, error: null };
    hook.calls = [];
  });

  it("renders nothing without a file", () => {
    expect(render({ filePath: null, excludeDq: false })).toBe("");
  });

  it("says that the status is being read", () => {
    hook.doc = { loading: true, status: null, error: null };
    const loading = render({ filePath: PATH, excludeDq: false });
    expect(loading).toContain("Reading calibration and layers...");
    expect(loading).toContain("data-photometry-status");
    hook.doc = { loading: false, status: null, error: null };
    const empty = render({ filePath: PATH, excludeDq: false });
    expect(empty).toContain("Reading calibration and layers...");
    expect(empty).toContain("data-photometry-status");
  });

  it("shows the failure of the status read", () => {
    hook.doc = { loading: false, status: null, error: "no such file" };
    expect(render({ filePath: PATH, excludeDq: false })).toContain("Status not available: no such file");
  });

  it("shows the calibration, the five chips and the warnings of a status", () => {
    hook.doc = { loading: false, status: STATUS, error: null };
    const markup = render({ filePath: PATH, excludeDq: false });
    expect(markup).toContain(">Calibration<");
    expect(markup).toContain("JWST MJy/sr, PIXAR_SR from header");
    for (const text of ["unit MJy/sr", "errors: ERR plane (HDU 2)", "flags: DQ plane (HDU 3), jwst flags", escaped(pixelScaleText(0.031)!), "F200W 1.989 um"]) {
      expect(markup, text).toContain(`>${text}<`);
    }
    expect(markup.match(/data-status-chip="/g)).toHaveLength(5);
    expect(markup.match(/data-status-warning/g)).toHaveLength(WARNINGS.length);
    expect(markup).not.toContain("Reading calibration and layers...");
  });

  it("asks for the status of the file with the DQ flag and the measure key of the panel", () => {
    render({ filePath: PATH, measureKey: "C:/d/a.fits@4", excludeDq: true });
    expect(hook.calls).toEqual([[PATH, true, "C:/d/a.fits@4", null]]);
    hook.calls = [];
    const zeroPoint = { zp: 25, system: "ab", per_second: false };
    render({ filePath: PATH, measureKey: "C:/d/a.fits@4", excludeDq: true, zeroPoint });
    expect(hook.calls).toEqual([[PATH, true, "C:/d/a.fits@4", zeroPoint]]);
  });

  it("shows the refusal of a per-second zero point as its error line", () => {
    hook.doc = {
      loading: false,
      status: null,
      error: "a zero point per second needs EXPTIME to turn the pixel values into a rate, and this header has none (BUNIT 'adu')",
    };
    const markup = render({ filePath: PATH, excludeDq: false, zeroPoint: { zp: 25, system: "ab", per_second: true } });
    expect(markup).toContain("Status not available: a zero point per second needs EXPTIME");
    expect(hook.calls[0][3]).toEqual({ zp: 25, system: "ab", per_second: true });
  });
});
