import { describe, expect, it } from "vitest";
import type { PhotometricPlane, RegionSpectrum } from "../../shared/types/cube";
import { CSV_LINE_END } from "../catalogCsv";
import { provenanceLines, unitLines } from "../csvProvenance";
import {
  psfFwhmSummary,
  sedAbView,
  sedCsv,
  sedCsvFileName,
  sedLogAllowed,
  sedLogYAxis,
  sedPoints,
  sedSeries,
  sedUnitKind,
  sedWavelengthAxis,
  sedX,
  sedYDomain,
  sedYLabel,
  type SedPoint,
} from "../sedAxis";

function plane(filter: string, dataset: string, pivot_um: number | null): PhotometricPlane {
  return {
    filter,
    dataset,
    instrument: dataset === "HST" ? "ACS" : "NIRCam",
    pivot_um,
    pixscale_arcsec: 0.04,
    bunit_in: "MJy/sr",
    fluxconv: null,
    psf_fwhm_arcsec: null,
    kernel: "",
  };
}

const GIVEN_ORDER = [plane("F444W", "JWST", 4.404315), plane("F150W", "JWST", 1.500744), plane("F814W", "HST", 0.804553)];

describe("SED axis", () => {
  it("puts each plane at its pivot wavelength in microns, in wavelength order, with its error", () => {
    const points = sedPoints(GIVEN_ORDER, [3, 1, 0.5], [0.3, 0.1, Number.NaN]);
    expect(points.map((p) => p.filter)).toEqual(["F814W", "F150W", "F444W"]);
    expect(points.map((p) => p.pivotUm)).toEqual([0.804553, 1.500744, 4.404315]);
    expect(points.map((p) => p.value)).toEqual([0.5, 1, 3]);
    expect(points.map((p) => p.err)).toEqual([null, 0.1, 0.3]);
    expect(points.map((p) => p.index)).toEqual([2, 1, 0]);
  });

  it("leaves out a plane without a pivot or a finite value instead of placing it at its plane index", () => {
    const planes = [plane("F444W", "JWST", 4.404315), plane("CUSTOM", "", null), plane("F150W", "JWST", 1.500744)];
    const points = sedPoints(planes, [3, 7, Number.NaN], null);
    expect(points).toEqual([{ index: 0, filter: "F444W", pivotUm: 4.404315, value: 3, err: null }]);
    expect(sedPoints(planes, [], null)).toEqual([]);
  });

  it("spans the pivots with padding on a log axis whose ticks are wavelengths in microns", () => {
    const axis = sedWavelengthAxis(sedPoints(GIVEN_ORDER, [3, 1, 0.5], null));
    expect(axis).not.toBeNull();
    expect(axis!.log).toBe(true);
    expect(axis!.min).toBeLessThan(0.804553);
    expect(axis!.max).toBeGreaterThan(4.404315);
    expect(axis!.ticks).toEqual([1, 2, 5]);
    expect(sedX(1, axis!, 200)).toBeCloseTo((200 * Math.log10(1 / axis!.min)) / Math.log10(axis!.max / axis!.min), 9);
    expect(sedX(axis!.min, axis!, 200)).toBeCloseTo(0, 9);
    expect(sedX(axis!.max, axis!, 200)).toBeCloseTo(200, 9);
  });

  it("uses a linear axis when the pivots span less than a factor of two", () => {
    const close = [plane("F356W", "JWST", 3.563), plane("F444W", "JWST", 4.404315)];
    const axis = sedWavelengthAxis(sedPoints(close, [1, 2], null))!;
    expect(axis.log).toBe(false);
    expect(axis.min).toBeLessThan(3.563);
    expect(axis.max).toBeGreaterThan(4.404315);
    expect(axis.ticks.every((t) => t >= axis.min && t <= axis.max)).toBe(true);
    expect(sedX((axis.min + axis.max) / 2, axis, 100)).toBeCloseTo(50, 9);
    const single = sedWavelengthAxis(sedPoints([close[0]], [1], null))!;
    expect(single.min).toBeLessThan(3.563);
    expect(single.max).toBeGreaterThan(3.563);
    expect(sedWavelengthAxis([])).toBeNull();
  });

  it("plots the region sum and its error when a region spectrum is shown, else the picked pixel", () => {
    const region = { sum: [5, 6, 7], err: [0.5, null, 0.7], npix: 12.5, flux_jy: null, flux_jy_err: null } as unknown as RegionSpectrum;
    expect(sedSeries({ spectrum: [1, 2, 3], err: [0.1, 0.2, 0.3], coord: { x: 4, y: 5 }, region })).toEqual({
      label: "region sum over 12.5 px",
      values: [5, 6, 7],
      errors: [0.5, null, 0.7],
      unit: null,
      kind: "region_sum",
      npix: 12.5,
    });
    expect(sedSeries({ spectrum: [1, 2, 3], err: [0.1, 0.2, 0.3], coord: { x: 4, y: 5 }, region: null })).toEqual({
      label: "pixel (4, 5)",
      values: [1, 2, 3],
      errors: [0.1, 0.2, 0.3],
      unit: null,
      kind: "pixel",
      npix: null,
    });
    expect(sedSeries({ spectrum: [], err: null, coord: null, region: null })).toBeNull();
  });

  it("plots the region flux in Jy of a surface-brightness cube instead of its summed MJy/sr", () => {
    const region = { sum: [5, 6], err: [0.5, 0.6], npix: 3, flux_jy: [1e-6, 2e-6], flux_jy_err: [1e-7, null] } as unknown as RegionSpectrum;
    expect(sedSeries({ spectrum: [], err: null, coord: null, region })).toEqual({
      label: "region flux over 3.0 px",
      values: [1e-6, 2e-6],
      errors: [1e-7, null],
      unit: "Jy",
      kind: "region_flux",
      npix: 3,
    });
  });
});

const LIMIT_POINTS: SedPoint[] = [
  { index: 0, filter: "g", pivotUm: 0.4808, value: 2, err: 0.1 },
  { index: 1, filter: "r", pivotUm: 0.6418, value: 0.3, err: null, limit: true },
];

describe("SED units, titles and exports", () => {
  it("tells flux densities from surface brightnesses by the unit text", () => {
    for (const unit of ["Jy", "mJy", "uJy", "nJy", "MJy", "Jy/pixel", " u Jy "]) expect(sedUnitKind(unit)).toBe("flux_density");
    for (const unit of ["MJy/sr", "Jy/arcsec2", "uJy/arcsec^2", "mjy/SR", "Jy sr-1"]) expect(sedUnitKind(unit)).toBe("surface_brightness");
    for (const unit of ["DN/s", "Jy/beam", "counts", "", null]) expect(sedUnitKind(unit)).toBe("other");
  });

  it("titles the y axis by series kind and unit", () => {
    const pixel = { unit: null, kind: "pixel" as const, npix: null };
    const regionSum = { unit: null, kind: "region_sum" as const, npix: 50 };
    expect(sedYLabel(pixel, "MJy/sr")).toBe("Surface brightness (MJy/sr)");
    expect(sedYLabel(pixel, "Jy")).toBe("Flux (Jy)");
    expect(sedYLabel(pixel, "DN/s")).toBe("Value (DN/s)");
    expect(sedYLabel(regionSum, "Jy")).toBe("Flux (Jy)");
    expect(sedYLabel({ unit: "Jy", kind: "region_flux", npix: 50 }, "MJy/sr")).toBe("Flux (Jy)");
    expect(sedYLabel(pixel, null)).toBe("Value");
    expect(sedYLabel(regionSum, "MJy/sr")).toBe("Sum of MJy/sr over 50.0 px");
    expect(sedYLabel(null, "Jy")).toBe("Flux");
    expect(sedYLabel({ unit: null, kind: "region_sum", npix: 12.5 }, null)).toBe("Sum over 12.5 px");
  });

  it("the y domain spans values, errors and limits", () => {
    const domain = sedYDomain(LIMIT_POINTS)!;
    expect(domain[0]).toBeCloseTo(0.3 - 0.18, 12);
    expect(domain[1]).toBeCloseTo(2.1 + 0.18, 12);
    expect(sedYDomain([])).toBeNull();
    expect(sedYDomain([{ index: 0, filter: "g", pivotUm: 0.48, value: 5, err: null }])).toEqual([4, 6]);
    expect(sedYDomain([{ index: 0, filter: "g", pivotUm: 0.48, value: 20, err: null }])).toEqual([18, 22]);
  });

  it("writes the points as csv with provenance and units", () => {
    const meta = { unit: "uJy", source: "source 7", file: "r.fits; g.fits", exportedUtc: "2026-11-10T00:00:00Z", appVersion: "0.8.0" };
    const lines = sedCsv(LIMIT_POINTS, meta).split(CSV_LINE_END);
    const header = lines.indexOf("filter,pivot_um,flux,flux_err,limit");
    expect(lines.slice(0, header)).toEqual([
      ...provenanceLines([
        ["software", "AstroBurst 0.8.0"],
        ["exported_utc", "2026-11-10T00:00:00Z"],
        ["file", "r.fits; g.fits"],
        ["source", "source 7"],
        ["flux_unit", "uJy"],
      ]),
      ...unitLines({ pivot_um: "um", flux: "uJy", flux_err: "uJy" }),
    ]);
    expect(lines.slice(header + 1, header + 3)).toEqual(["g,0.4808,2,0.1,false", "r,0.6418,0.3,,true"]);
    const raw = sedCsv(LIMIT_POINTS, { ...meta, unit: null }).split(CSV_LINE_END);
    expect(raw).toContain("# flux_unit: not calibrated");
    expect(raw).toContain("# unit flux: image unit");
    expect(raw).toContain("# unit flux_err: image unit");
  });

  it("names the file after the cube", () => {
    expect(sedCsvFileName("cube.fits")).toBe("cube_sed.csv");
    expect(sedCsvFileName("a.b.fits.fz")).toBe("a.b.fits_sed.csv");
    expect(sedCsvFileName(null)).toBe("sed.csv");
  });

  it("summarises the plane PSFs", () => {
    const withPsf = (psf: number | null) => ({ ...plane("F150W", "JWST", 1.5), psf_fwhm_arcsec: psf });
    expect(psfFwhmSummary([withPsf(0.16), withPsf(0.16), withPsf(0.16)])).toBe("PSF FWHM 0.160 arcsec on every plane");
    expect(psfFwhmSummary([withPsf(0.064), withPsf(0.11), withPsf(null)])).toBe("PSF FWHM 0.064 to 0.110 arcsec across planes (2 of 3 planes)");
    expect(psfFwhmSummary([withPsf(null), withPsf(null), withPsf(null)])).toBeNull();
  });
});

const G: SedPoint = { index: 0, filter: "g", pivotUm: 0.4808, value: 2, err: 0.1 };
const R: SedPoint = { index: 1, filter: "r", pivotUm: 0.6418, value: 0.3, err: null, limit: true };

const relativeGap = (actual: number, expected: number) => Math.abs(actual - expected) / Math.abs(expected);

describe("SED log y and AB magnitudes", () => {
  it("a log scale needs every value minus its error above zero", () => {
    expect(sedLogAllowed([G, R])).toBe(true);
    expect(sedLogAllowed([G, { ...G, index: 2, value: 0.05, err: 0.1 }])).toBe(false);
    expect(sedLogAllowed([{ ...R, value: 0 }])).toBe(false);
    expect(sedLogAllowed([])).toBe(false);
  });

  it("the log y axis spans values and errors with 1-2-5 ticks", () => {
    const axis = sedLogYAxis([G, R])!;
    expect(axis.log).toBe(true);
    expect(relativeGap(axis.min, 0.25675174948071844)).toBeLessThan(1e-12);
    expect(relativeGap(axis.max, 2.4537320632641366)).toBeLessThan(1e-12);
    expect(axis.ticks).toEqual([0.5, 1, 2]);
    const single = sedLogYAxis([{ ...G, value: 5, err: null }])!;
    expect(relativeGap(single.min, 4.77496293010718)).toBeLessThan(1e-12);
    expect(relativeGap(single.max, 5.235642740254499)).toBeLessThan(1e-12);
    expect(single.ticks).toEqual([5]);
    expect(sedLogYAxis([{ ...G, value: 0.05 }])).toBeNull();
  });

  it("AB magnitudes come from the flux density in Jy", () => {
    const view = sedAbView([G, R], "uJy")!;
    expect(view.dropped).toBe(0);
    expect(view.points).toHaveLength(2);
    expect(Math.abs(view.points[0].value - 23.147425010840045)).toBeLessThan(1e-9);
    expect(Math.abs(view.points[0].err! - 0.05428681023790645)).toBeLessThan(1e-12);
    expect(Math.abs(view.points[1].value - 25.207196863200842)).toBeLessThan(1e-9);
    expect(view.points[1].err).toBeNull();
    expect(view.points[1].limit).toBe(true);
    expect(view.points.map((p) => [p.index, p.filter, p.pivotUm])).toEqual([
      [0, "g", 0.4808],
      [1, "r", 0.6418],
    ]);
    expect(Math.abs(sedAbView([G], "mJy")!.points[0].value - 15.647425010840047)).toBeLessThan(1e-9);
    for (const unit of ["MJy/sr", "Jy/beam", "DN/s", null]) expect(sedAbView([G], unit)).toBeNull();
    const dropping = sedAbView([G, { ...G, index: 2, filter: "z", value: -0.4, err: 0.2 }, { ...R, value: 0 }], "uJy")!;
    expect(dropping.points).toHaveLength(1);
    expect(dropping.dropped).toBe(2);
  });

  it("the magnitude domain of F1's rule spans the magnitudes and their errors", () => {
    const domain = sedYDomain(sedAbView([G, R], "uJy")!.points)!;
    expect(Math.abs(domain[0] - 22.881732334342267)).toBeLessThan(1e-9);
    expect(Math.abs(domain[1] - 25.418602729460712)).toBeLessThan(1e-9);
  });
});
