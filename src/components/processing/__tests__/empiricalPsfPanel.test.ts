import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import PsfPanel from "../PsfPanel";
import EmpiricalPsfResult from "../EmpiricalPsfResult";
import type { ProcessedFile } from "../../../shared/types";
import type { EmpiricalPsfReport, EmpiricalPsfStar } from "../../../shared/types/processing";

const FILE: ProcessedFile = {
  id: "f1",
  name: "jw_f444w_i2d.fits",
  path: "C:/data/jw_f444w_i2d.fits",
  sourcePath: "C:/data/jw_f444w_i2d.fits",
  imageRef: null,
  size: 1,
  status: "done",
  result: null,
  error: null,
  startedAt: null,
  finishedAt: null,
};

function renderPanel(compositeMode: boolean): string {
  const element = createElement(PsfPanel, {
    selectedFile: FILE,
    fileKey: FILE.path,
    compositeMode,
    compositeInput: compositeMode ? { input: "base", previewUrl: "asset://out/blend.png", label: "Composite" } : null,
    fileName: FILE.name,
  });
  const preview = createElement(PreviewProvider, { file: FILE, doneFiles: [], children: element });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function star(overrides: Partial<EmpiricalPsfStar>): EmpiricalPsfStar {
  return {
    name: "s1",
    x: 10,
    y: 20,
    used: true,
    rejected: null,
    centroid: "quadratic",
    centre_x: 10.25,
    centre_y: 19.75,
    background: 0.1,
    background_std: 0.01,
    cleaning_radius: null,
    aperture_flux: 12.5,
    profile: [1, 0.5, 0.1],
    ...overrides,
  };
}

const REPORT: EmpiricalPsfReport = {
  source: "regions",
  filter: "F444W",
  dataset: "JWST",
  pixel_scale_arcsec: 0.04,
  normalization_radius_px: 20,
  psf_size: 3,
  fwhm_px: 3.62,
  fwhm_arcsec: 0.1448,
  stars_used: 2,
  stars_rejected: 1,
  stars: [
    star({ name: "first" }),
    star({ name: "second", centroid: "peak", profile: [1, 0.4, 0.2] }),
    star({ name: "edge", used: false, rejected: "the cutout lies outside the image", centroid: null, centre_x: null, centre_y: null, profile: [] }),
  ],
  kernel: [[0, 0.1, 0], [0.1, 0.6, 0.1], [0, 0.1, 0]],
  profile: [0.6, 0.1],
  oversampling: 2,
  oversampled_size: 6,
  output_path: "C:/out/empirical_psf_jwst_f444w.fits",
  oversampled_path: "C:/out/empirical_psf_jwst_f444w_os2.fits",
  warnings: ["the file name names F277W but the header names F444W; the header wins"],
  elapsed_ms: 8,
};

describe("PsfPanel LIGHTSTACK recipe", () => {
  it("offers the star sources, the recipe options and the save toggle on a file", () => {
    const html = renderPanel(false);
    expect(html).toContain("Build empirical PSF");
    expect(html).toContain("Automatic finder");
    expect(html).toContain("DS9 region file");
    expect(html).toContain("RA/Dec list");
    expect(html).toContain("Cutout size");
    expect(html).toContain("Normalization radius");
    expect(html).toContain("Median");
    expect(html).toContain("Save PSF FITS");
  });

  it("stays file-only in composite mode", () => {
    const html = renderPanel(true);
    expect(html).toContain("Estimate PSF");
    expect(html).not.toContain("Build empirical PSF");
  });
});

describe("EmpiricalPsfResult", () => {
  const html = renderToStaticMarkup(createElement(EmpiricalPsfResult, { report: REPORT }));

  it("lists every star with its status and the rejection reason", () => {
    expect(html).toContain("first");
    expect(html).toContain("used (quadratic centroid)");
    expect(html).toContain("used (peak fallback)");
    expect(html).toContain("rejected: the cutout lies outside the image");
    expect(html).toContain("2 / 3");
  });

  it("plots one radial profile per used star beside the stacked PSF", () => {
    const lines = html.match(/<polyline/g) ?? [];
    expect(lines).toHaveLength(3);
    expect(html).toContain('data-star="first"');
    expect(html).toContain('data-star="second"');
    expect(html).not.toContain('data-star="edge"');
    expect(html).toContain('data-star="stack"');
  });

  it("shows the FWHM, the labels, the written files and the warnings", () => {
    expect(html).toContain("3.62 px");
    expect(html).toContain("0.145&quot;");
    expect(html).toContain("JWST F444W");
    expect(html).toContain("C:/out/empirical_psf_jwst_f444w.fits");
    expect(html).toContain("C:/out/empirical_psf_jwst_f444w_os2.fits");
    expect(html).toContain("the header wins");
  });
});
