import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const sedProps = vi.hoisted(() => ({ current: null as Record<string, unknown> | null }));
const cardProps = vi.hoisted(() => ({ current: null as Record<string, unknown> | null }));

vi.mock("../SedPanel", () => ({
  default: (props: Record<string, unknown>) => {
    sedProps.current = props;
    return "[sed]";
  },
}));
vi.mock("../ResolutionCard", () => ({
  default: (props: Record<string, unknown>) => {
    cardProps.current = props;
    return "[card]";
  },
}));

import { forcedSedPoints } from "../../../utils/forcedPhotometry";
import type {
  ForcedApertureCorrection,
  ForcedBand,
  ForcedCell,
  ForcedPhotometryResult,
} from "../../../shared/types/forcedPhotometry";

const { default: ForcedSourceDetail, ForcedSourceDetailInner } = await import("../ForcedSourceDetail");
const { sedNotes, sourceSizeBand, sourceTitle } = await import("../forcedSourceDetailModel");

const NO_WCS_ERROR = "x.fits has no celestial WCS: sky positions cannot be placed on it";
const APERTURE_NOTE =
  "Aperture fluxes: bands with different PSFs enclose different fractions of the source. Turn on the correction to total flux to compare them.";
const TOTAL_NOTE = "Each band is corrected to total flux with its own aperture correction.";

function cell(overrides: Partial<ForcedCell> = {}): ForcedCell {
  return {
    x: 10.5,
    y: 20.25,
    on_image: true,
    error: null,
    flux_native: 0.550813,
    flux_err_native: 0.0275406,
    flux_jy: 2e-6,
    flux_err_jy: 1e-7,
    flux_err_corr_jy: null,
    flux_total_jy: null,
    mag_ab: 23.147425,
    mag_ab_err: 0.054287,
    upper_limit_jy: null,
    upper_limit_ab: null,
    snr: 20,
    masked_fraction: 0,
    n_nonfinite: 0,
    n_masked: 0,
    flags_or: 0,
    saturated: false,
    fwhm_px: null,
    ...overrides,
  };
}

function band(index: number, label: string, pivot: number | null, overrides: Partial<ForcedBand> = {}): ForcedBand {
  return {
    index,
    path: `C:/data/${label}.fits`,
    file_name: `${label}.fits`,
    label,
    error: null,
    filter: pivot === null ? null : { dataset: "DECAM", filter: label, pivot_um: pivot, source: "header" },
    photcal: null,
    calibration_origin: "BUNIT nanomaggy, zero point 22.5",
    jy_per_native_unit: 3.631e-6,
    bunit: "nanomaggy",
    pixel_scale_arcsec: 0.262,
    aperture_radius_px: 7.6,
    annulus_px: null,
    aperture_correction: null,
    err_origin: null,
    dq_source: null,
    f_corr: null,
    n_measured: 1,
    n_failed: 0,
    warnings: [],
    ...overrides,
  };
}

function correction(factor: number): ForcedApertureCorrection {
  return { radius_px: 7.6, reference_radius_px: 13.4, factor, scatter: 0.01, n_stars: 12, n_candidates: 20 };
}

function failedBand(index: number, label: string, error: string): ForcedBand {
  return band(index, label, null, {
    error,
    calibration_origin: null,
    jy_per_native_unit: null,
    bunit: null,
    pixel_scale_arcsec: null,
    aperture_radius_px: null,
    n_measured: 0,
  });
}

function run(bands: ForcedBand[], cellsBySource: (ForcedCell | null)[][]): ForcedPhotometryResult {
  return {
    bands,
    sources: cellsBySource.map((cells, i) => ({ id: `s${i}`, ra_deg: 53.1 + i * 0.001, dec_deg: -28.1, cells })),
    aperture: {
      radius_arcsec: 2,
      radius_px: null,
      annulus_arcsec: null,
      annulus_px: null,
      correction: "none",
      reference_radius_arcsec: null,
    },
    limit_sigma: 3,
    elapsed_ms: 12,
    warnings: [],
  };
}

const gDetected = cell({ flux_jy: 1e-6, flux_err_jy: 1e-7, mag_ab: 23.9, mag_ab_err: 0.108574 });
const rDetected = cell({ flux_jy: 2e-6, flux_err_jy: 1e-7, mag_ab: 23.147425, mag_ab_err: 0.054287 });
const zDetected = cell({ flux_jy: 3e-6, flux_err_jy: 1e-7, mag_ab: 22.707197, mag_ab_err: 0.036191 });
const gLimit = cell({
  flux_jy: 5e-8,
  flux_err_jy: 1e-7,
  mag_ab: null,
  mag_ab_err: null,
  upper_limit_jy: 3e-7,
  upper_limit_ab: 25.207197,
  snr: 0.5,
});

const BANDS = [band(0, "r", 0.6418), band(1, "g", 0.4808), band(2, "z", 0.9168), failedBand(3, "x", NO_WCS_ERROR)];
const CELLS: (ForcedCell | null)[][] = [
  [{ ...rDetected, fwhm_px: 3.74 }, gDetected, zDetected, null],
  [{ ...rDetected, fwhm_px: 3.9 }, gLimit, zDetected, null],
  [rDetected, gDetected, zDetected, null],
];
const RESULT = run(BANDS, CELLS);
const NO_SIZE_TEXT = "No size for this source: its FWHM could not be measured at the forced position in any band.";
const NO_SIZE_WHY =
  "That width is a fit to the core of a compact source: it finds none on a source too faint for it or far wider than the PSF (a large galaxy, a blend), so this is not an unresolved verdict. To size an extended source, draw an ellipse on it and read R50 against the PSF in the Image tool, section Profiles.";

const markup = (result: ForcedPhotometryResult, source: number, currentPath: string | null = null) =>
  renderToStaticMarkup(createElement(ForcedSourceDetailInner, { result, source, currentPath }));

describe("ForcedSourceDetail", () => {
  beforeEach(() => {
    sedProps.current = null;
    cardProps.current = null;
  });

  it("the selected source gets the card of the band where its size was measured", () => {
    const html = markup(RESULT, 0, "C:/data/g.fits");
    expect(html).toContain('data-forced-size-card="0"');
    expect(html).toContain("Size measured in r: FWHM 0.98 arcsec at the forced position.");
    expect(html).toContain("[card]");
    expect(html).not.toContain("data-forced-no-size");
    expect(html.indexOf(APERTURE_NOTE)).toBeLessThan(html.indexOf("data-forced-size-card"));
    const { fwhmArcsec, ...rest } = cardProps.current!;
    expect(fwhmArcsec).toBeCloseTo(0.97988, 10);
    expect(rest).toEqual({ filePath: "C:/data/r.fits", snr: 20, context: "forced", sourceLabel: "s0 in r (r.fits)" });
    expect(Object.keys(cardProps.current!).sort()).toEqual(["context", "filePath", "fwhmArcsec", "snr", "sourceLabel"]);
  });

  it("the band on screen is preferred when the source has a size there", () => {
    const withG = run(
      BANDS,
      CELLS.map((cells, i) => (i === 0 ? cells.map((c, k) => (k === 1 ? { ...gDetected, fwhm_px: 3.1 } : c)) : cells)),
    );
    expect(sourceSizeBand(withG, 0, "C:/data/g.fits")).toBe(1);
    const html = markup(withG, 0, "C:/data/g.fits");
    expect(html).toContain('data-forced-size-card="1"');
    expect(html).toContain("Size measured in g: FWHM 0.812 arcsec at the forced position.");
    expect(cardProps.current!.filePath).toBe("C:/data/g.fits");
    expect(cardProps.current!.sourceLabel).toBe("s0 in g (g.fits)");
    expect(sourceSizeBand(withG, 0, null)).toBe(0);
    markup(withG, 0, null);
    expect(cardProps.current!.filePath).toBe("C:/data/r.fits");
    expect(sourceSizeBand(withG, 0, "C:/data/z.fits")).toBe(0);
  });

  it("a source without a measured size says so", () => {
    const html = markup(RESULT, 2, "C:/data/r.fits");
    expect(html).not.toContain("[card]");
    expect(html).not.toContain("data-forced-size-card");
    expect(html).toContain(`data-forced-no-size="true"`);
    expect(html).toContain(`${NO_SIZE_TEXT} ${NO_SIZE_WHY}`);
    expect(cardProps.current).toBeNull();
    expect(sourceSizeBand(RESULT, 2, null)).toBeNull();
  });

  it("the selected source is named with its position and its SED goes to the SED panel in uJy", () => {
    expect(RESULT.sources[0].ra_deg).toBe(53.1);
    expect(RESULT.sources[0].dec_deg).toBe(-28.1);
    expect(sourceTitle(RESULT, 0)).toBe("s0 at RA 53.100000 Dec -28.100000 deg");
    const html = markup(RESULT, 0);
    expect(html).toContain("s0 at RA 53.100000 Dec -28.100000 deg");
    expect(html).toContain("[sed]");
    expect(html).toContain('data-forced-source="0"');
    expect(sedProps.current).toEqual({
      points: forcedSedPoints(RESULT, 0).points,
      unit: "uJy",
      label: "s0, aperture fluxes",
      fileNames: ["r.fits", "g.fits", "z.fits"],
    });
    expect(html).toContain(APERTURE_NOTE);
    expect(renderToStaticMarkup(createElement(ForcedSourceDetail, { result: RESULT, source: 0, currentPath: "C:/data/r.fits" }))).toBe(html);
  });

  it("the notes say what the points are", () => {
    expect(sedNotes(RESULT, 0)).toEqual([APERTURE_NOTE]);

    const corrected = run(
      BANDS.map((b, i) => (i < 3 ? { ...b, aperture_correction: correction(1.2 + i * 0.1) } : b)),
      CELLS,
    );
    expect(sedNotes(corrected, 0)).toEqual([TOTAL_NOTE]);
    markup(corrected, 0);
    expect(sedProps.current?.label).toBe("s0, total fluxes");

    const limitNotes = sedNotes(RESULT, 1);
    expect(limitNotes).toEqual([APERTURE_NOTE, "Arrows are 3 sigma upper limits."]);
    expect(limitNotes[limitNotes.length - 1]).toBe("Arrows are 3 sigma upper limits.");
    expect(sedNotes({ ...RESULT, limit_sigma: 5 }, 1)).toEqual([APERTURE_NOTE, "Arrows are 5 sigma upper limits."]);

    const noPivotZ = run(
      BANDS.map((b, i) => (i === 2 ? { ...b, filter: null } : b)),
      CELLS,
    );
    expect(sedNotes(noPivotZ, 0)).toEqual([APERTURE_NOTE, "Not plotted, no pivot wavelength: z."]);
    expect(markup(noPivotZ, 0)).toContain("Not plotted, no pivot wavelength: z.");

    const single = run([band(0, "r", 0.6418)], [[rDetected]]);
    expect(sedNotes(single, 0)).toEqual([]);
  });

  it("nothing is drawn for a source that is not in the result", () => {
    expect(markup(RESULT, 9)).toBe("");
    expect(sedProps.current).toBeNull();
  });
});
