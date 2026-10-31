import { describe, expect, it } from "vitest";
import type { PhotometricPlane, RegionSpectrum } from "../../shared/types/cube";
import { sedPoints, sedSeries, sedWavelengthAxis, sedX } from "../sedAxis";

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
    });
    expect(sedSeries({ spectrum: [1, 2, 3], err: [0.1, 0.2, 0.3], coord: { x: 4, y: 5 }, region: null })).toEqual({
      label: "pixel (4, 5)",
      values: [1, 2, 3],
      errors: [0.1, 0.2, 0.3],
      unit: null,
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
    });
  });
});
