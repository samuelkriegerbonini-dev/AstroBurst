import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { PhotometricPlane, RegionSpectrum } from "../../../shared/types/cube";
import type { SedPoint, SedSource } from "../../../utils/sedAxis";
import { formatWithUncertainty } from "../../../utils/formatUncertainty";
import SedPanel from "../SedPanel";

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: { id: "f1", path: "C:/data/cube.fits", name: "cube.fits" } }),
}));

function plane(filter: string, pivot_um: number, psf_fwhm_arcsec: number | null = null): PhotometricPlane {
  return {
    filter,
    dataset: "JWST",
    instrument: "NIRCam",
    pivot_um,
    pixscale_arcsec: 0.04,
    bunit_in: "Jy",
    fluxconv: 1,
    psf_fwhm_arcsec,
    kernel: "",
  };
}

function planes(psf: readonly (number | null)[] = [null, null, null]): PhotometricPlane[] {
  return [plane("F150W", 1.500744, psf[0]), plane("F277W", 2.761, psf[1]), plane("F444W", 4.404315, psf[2])];
}

function region(npix: number, flux_jy: number[] | null): SedSource["region"] {
  return { sum: [5, 6, 7], err: [0.5, 0.6, 0.7], npix, flux_jy, flux_jy_err: flux_jy ? [0.1, 0.1, 0.1] : null } as Pick<
    RegionSpectrum,
    "sum" | "err" | "npix" | "flux_jy" | "flux_jy_err"
  >;
}

const PIXEL: SedSource = { spectrum: [3, 1, 2], err: [0.3, 0.1, 0.2], coord: { x: 2, y: 3 }, region: null };
const EMPTY: SedSource = { spectrum: [], err: null, coord: null, region: null };

function cube(bunit: string | null, source: SedSource, planeList: PhotometricPlane[] = planes()): string {
  return renderToStaticMarkup(createElement(SedPanel, { planes: planeList, bunit, source }));
}

function pointsForm(points: SedPoint[], unit = "uJy", label = "source 7"): string {
  return renderToStaticMarkup(createElement(SedPanel, { points, unit, label }));
}

const count = (html: string, attribute: string) => html.split(attribute).length - 1;

function rows(html: string): string[] {
  return [...html.matchAll(/<tr[^>]*data-sed-row[^>]*>(.*?)<\/tr>/g)].map((m) => m[1]);
}

const LIMIT_POINTS: SedPoint[] = [
  { index: 0, filter: "g", pivotUm: 0.4808, value: 2, err: 0.1 },
  { index: 1, filter: "r", pivotUm: 0.6418, value: 0.3, err: null, limit: true },
];

describe("SedPanel", () => {
  it("a region sum of a surface-brightness cube is not titled as a flux", () => {
    const html = cube("MJy/sr", { ...EMPTY, region: region(50, null) });
    expect(html).not.toContain("Flux (MJy/sr)");
    expect(html).toContain("Sum of MJy/sr over 50.0 px");
  });

  it("the y title says what the value is", () => {
    expect(cube("MJy/sr", PIXEL)).toContain("Surface brightness (MJy/sr)");
    expect(cube("Jy", PIXEL)).toContain("Flux (Jy)");
    expect(cube("DN/s", PIXEL)).toContain("Value (DN/s)");
    expect(cube("Jy", { ...EMPTY, region: region(50, null) })).toContain("Flux (Jy)");
    const flux = cube("MJy/sr", { ...EMPTY, region: region(50, [1e-6, 2e-6, 3e-6]) });
    expect(flux).toContain("Flux (Jy)");
    expect(flux).toContain("region flux over 50.0 px");
    expect(cube(null, PIXEL)).toContain(">Value<");
  });

  it("the values are in a table that the chart points to", () => {
    const html = cube("Jy", PIXEL);
    expect(count(html, "data-sed-row")).toBe(3);
    expect(count(html, "data-sed-point")).toBe(3);
    expect(count(html, "data-sed-err")).toBe(3);
    const texts = rows(html);
    expect(texts[0]).toContain("F150W");
    expect(texts[0]).toContain("1.501");
    expect(texts[0]).toContain(formatWithUncertainty(3, 0.3));
    const describedBy = /<svg[^>]*aria-describedby="([^"]+)"/.exec(html)?.[1];
    const tableId = /<table[^>]*\sid="([^"]+)"/.exec(html)?.[1];
    expect(describedBy).toBeDefined();
    expect(describedBy).toBe(tableId);
  });

  it("a matched cube states its PSF", () => {
    const matched = cube("Jy", PIXEL, planes([0.16, 0.16, 0.16]));
    expect(matched).toContain("PSF FWHM 0.160 arcsec on every plane");
    expect(matched).toContain("PSF FWHM (arcsec)");
    expect(cube("Jy", PIXEL, planes([0.064, 0.11, null]))).toContain("PSF FWHM 0.064 to 0.110 arcsec across planes (2 of 3 planes)");
    const none = cube("Jy", PIXEL);
    expect(none).not.toContain("data-sed-psf");
    expect(none).not.toContain("PSF FWHM");
  });

  it("points with an upper limit draw an arrow and no error bar", () => {
    const html = pointsForm(LIMIT_POINTS);
    expect(count(html, "data-sed-limit")).toBe(1);
    expect(count(html, "data-sed-err")).toBe(1);
    expect(count(html, "data-sed-point")).toBe(2);
    expect(html).toContain("Flux (uJy)");
    const limitRow = rows(html).find((row) => row.includes(">r<"));
    expect(limitRow).toContain(`&lt; ${formatWithUncertainty(0.3, null)}`);
    expect(html).toContain("source 7");
    expect(html).toContain(`r at 0.642 um: &lt; ${formatWithUncertainty(0.3, null)} (upper limit)`);
  });

  it("the export buttons exist only when there is something to export", () => {
    const html = cube("Jy", PIXEL);
    expect(html).toContain("Copy CSV");
    expect(html).toContain("Save CSV");
    const empty = cube("Jy", EMPTY);
    expect(empty).not.toContain("Copy CSV");
    expect(empty).not.toContain("Save CSV");
    const none = pointsForm([]);
    expect(none).not.toContain("Copy CSV");
    expect(none).not.toContain("Save CSV");
    expect(none).toContain("No band of this source has both a pivot wavelength and a flux.");
  });

  it("the cube form keeps the strings CubeTool pins", () => {
    const html = cube("Jy", PIXEL);
    expect(html).toContain("data-sed-panel");
    expect(html).toContain("Pivot wavelength (um)");
    expect(html).toContain("pixel (2, 3)");
    expect(cube("Jy", EMPTY)).toContain("Click a pixel or extract a region spectrum to plot the SED of the 3 filter planes.");
  });
});
