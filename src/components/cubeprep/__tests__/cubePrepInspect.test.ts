import { createElement, isValidElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { FitsExtension } from "../../../services/header";
import type { CubePrepStep } from "../../../shared/types/cubeprep";
import type { CompanionSuggestions } from "../../../shared/types/layers";
import type { ImagePsfFwhm, ResolutionCheck } from "../../../shared/types/resolution";
import { DEFAULT_RESOLUTION_SETTINGS } from "../../../utils/resolutionText";
import CubePrepStepEditor from "../CubePrepStepEditor";
import { createCubePrepDraft } from "../cubePrepModel";
import {
  COMMON_PSF_REFERENCE,
  commonPsfArcsec,
  FITS_INPUT,
  FITS_INPUT_EXTENSIONS,
  fwhmCell,
  layerName,
  layersText,
  psfFwhmRows,
  psfLine,
  REDSHIFT_HELP,
  redshiftNote,
  redshiftRequest,
  scaleLine,
  type RedshiftNote,
} from "../cubePrepInspect";

function ext(index: number, extname: string | null, more: Partial<FitsExtension> = {}): FitsExtension {
  return { index, extname, naxis: 2, naxis1: 10, naxis2: 10, naxis3: 0, bitpix: -32, has_data: index > 0, extver: null, ref: `/data/a.fits#hdu=${index}`, kind: "hdu", is_err: false, is_dq: false, ...more };
}
const MEF = [ext(0, null), ext(1, "SCI"), ext(2, "ERR", { is_err: true, role: "sigma" }), ext(3, "DQ", { is_dq: true, role: "mask" })];
const AFW = [ext(0, null), ext(1, "IMAGE"), ext(2, "MASK", { role: "mask" }), ext(3, "VARIANCE", { role: "variance" })];
function sug(err: CompanionSuggestions["err"], dq: CompanionSuggestions["dq"], more: Partial<CompanionSuggestions> = {}): CompanionSuggestions {
  return { role: { role: "science", basis: "t", assumed: false }, err, dq, applied: true, candidates: { err: [], dq: [] }, notes: [], ...more };
}
const STEPS = [
  { step: "psfs", report: { prepared: { psfs: [{ filter: "F200W", pixel_scale_arcsec: 0.03, after: { fwhm_px: 2.2, fwhm_arcsec: 0.066 } }, { filter: "F444W", pixel_scale_arcsec: 0.03, after: { fwhm_px: 4.8, fwhm_arcsec: 0.144 } }] } } },
  { step: "kernels", report: { reference: "f444w", pixel_scale_arcsec: 0.03, kernels: [{ filter: "F200W", diagnostics: { source_fwhm_px: 2.2, target_fwhm_px: 4.8, matched_fwhm_px: 4.83 } }] } },
];
const COSMOLOGY = "flat LambdaCDM, H0 67.66, Omega_m 0.30966, plus Planck 2018 neutrinos and radiation";
const CHECK: ResolutionCheck = {
  verdict: null, size_basis: null, psf_fwhm_arcsec: null, psf_reference: null, pixels_per_fwhm: null, nyquist_sampled: null, psf_area_px: null,
  intrinsic_size_arcsec: null, snr_floor_fraction: null, psf_floor_fraction: 0.24677925358506123, floor_fraction: null, size_floor_arcsec: null, resolution_elements: null,
  kpc_per_arcsec: 8.578275309405592, kpc_per_pixel: null, angular_diameter_distance_mpc: 1769.396294628795, psf_fwhm_kpc: null, intrinsic_size_kpc: null, size_floor_kpc: null,
  scale_of_interest_in_psf: null, snr: null, redshift: 2, psf_fwhm_rel_err: 0.03, resolved_elements: 3, min_pixels_per_fwhm: 2, scale_of_interest_kpc: 1, h0_km_s_mpc: 67.66, omega_m: 0.30966,
  omega_nu_massive: 0.0014147613306646316, omega_rad: 7.893312479128647e-5, cosmology: COSMOLOGY, notes: [], warnings: [],
};
const MASK_REASON = "floating-point plane (BITPIX -32): flag bits cannot be read exactly";

describe("Cube Prep input layers", () => {
  it("layers in the same file are named by EXTNAME and HDU", () => {
    const layers = sug({ source: "/data/a.fits#hdu=2", role: "sigma", basis: "EXTNAME ERR" }, { source: "/data/a.fits#hdu=3", basis: "EXTNAME DQ" });
    expect(layersText("/data/a.fits", "MJy/sr", MEF, layers)).toBe("MJy/sr · error: ERR, HDU 2 (sigma) · mask: DQ, HDU 3");
  });

  it("a VARIANCE and a MASK layer are an error and a mask", () => {
    const layers = sug({ source: "/data/a.fits#hdu=3", role: "variance", basis: "EXTNAME VARIANCE" }, { source: "/data/a.fits#hdu=2", basis: "EXTNAME MASK" });
    expect(layersText("/data/a.fits", "nJy", AFW, layers)).toBe("nJy · error: VARIANCE, HDU 3 (variance) · mask: MASK, HDU 2");
    expect(layersText("/data/a.fits", "nJy", AFW, null)).toBe("nJy · error: VARIANCE, HDU 3 (variance) · mask: MASK, HDU 2");
  });

  it("sibling files are named by file", () => {
    const err = { source: "/data/legacy/invvar-r.fits.fz#hdu=1", role: "inverse_variance" as const, basis: "file name -invvar-" };
    const dq = { source: "C:\\data\\legacy\\maskbits.fits.fz", basis: "file name -maskbits-" };
    const layers = sug(err, dq, { candidates: { err: [{ ...err, usable: true, reason: null }], dq: [{ ...dq, usable: true, reason: null }] } });
    expect(layersText("/data/legacy/image-r.fits.fz", "nanomaggy", [ext(0, null), ext(1, "IMAGE")], layers)).toBe("nanomaggy · error: invvar-r.fits.fz, HDU 1 (inverse variance) · mask: maskbits.fits.fz");
  });

  it("a layer that cannot be used says why", () => {
    const err = { source: "/data/ps1/wt.fits", role: "variance" as const, basis: "PS_COVAR card (PS1 variance layer)" };
    const dq = { source: "/data/ps1/mask.fits", basis: "MSKNAM cards (PS1 mask layer)" };
    const layers = (reason: string | null) => sug(err, dq, { applied: false, candidates: { err: [{ ...err, usable: true, reason: null }], dq: [{ ...dq, usable: false, reason }] } });
    expect(layersText("/data/ps1/stack.fits", "MJy/sr", [ext(0, null)], layers(MASK_REASON))).toBe(`MJy/sr · error: wt.fits (variance) · mask layer mask.fits not used: ${MASK_REASON}`);
    expect(layersText("/data/ps1/stack.fits", "MJy/sr", [ext(0, null)], layers(null))).toBe("MJy/sr · error: wt.fits (variance) · mask layer mask.fits not used");
    const unreadable = sug(err, null, { applied: false, candidates: { err: [{ ...err, usable: false, reason: "cannot read /data/ps1/wt.fits" }], dq: [] } });
    expect(layersText("/data/ps1/stack.fits", "MJy/sr", [ext(0, null)], unreadable)).toBe("MJy/sr · error layer wt.fits not used: cannot read /data/ps1/wt.fits · DQ absent");
  });

  it("a suggestion that is not applied is not called in use", () => {
    const err = { source: "/data/wgt.fits", role: "inverse_variance" as const, basis: "EXTNAME WGT" };
    const layers = sug(err, null, { applied: false, candidates: { err: [{ ...err, usable: true, reason: null }], dq: [] } });
    expect(layersText("/data/a.fits", "MJy/sr", [ext(0, null)], layers)).toBe("MJy/sr · error: wgt.fits (inverse variance) · DQ absent · not every layer listed is in use for this file");
  });

  it("no layer and no unit", () => {
    expect(layersText("/data/a.fits", undefined, [ext(0, null)], sug(null, null))).toBe("unit missing · ERR absent · DQ absent");
    expect(layersText("/data/a.fits", undefined, [ext(0, null)], null)).toBe("unit missing · ERR absent · DQ absent");
    expect(layersText("/data/a.fits", "", [ext(0, null)], null)).toBe("unit missing · ERR absent · DQ absent");
  });

  it("without suggestions the EXTNAME flags keep today's words", () => {
    expect(layersText("/data/a.fits", "MJy/sr", [{ is_err: true, is_dq: true }] as FitsExtension[], null)).toBe("MJy/sr · ERR present · DQ present");
  });

  it("layerName falls back to the path when no extension has the reference", () => {
    expect(layerName("C:\\d\\a.fits#hdu=4", "C:/d/A.fits", [])).toBe("HDU 4");
    expect(layerName("/d/b.fits#hdu=1&plane=2", "/d/a.fits", [])).toBe("b.fits, HDU 1");
    expect(layerName("/d/b.fits", "/d/a.fits", [])).toBe("b.fits");
    const other = sug({ source: "/data/a.fits#hdu=2", role: "other", basis: "t" }, null);
    expect(layersText("/data/a.fits", "MJy/sr", MEF, other)).toBe("MJy/sr · error: ERR, HDU 2 (other) · DQ absent");
  });
});

describe("Cube Prep input scale and PSF", () => {
  it("the pixel scale line", () => {
    expect(scaleLine(0.031)).toBe('Pixel scale 0.03100"/px');
    for (const scale of [null, undefined, 0, Number.NaN]) expect(scaleLine(scale)).toBe("No celestial WCS: pixel scale unknown");
  });

  it("the PSF line", () => {
    expect(psfLine({ fwhm_px: 3.506, fwhm_arcsec: 0.9184, fwhm_px_scatter: 0.13, n_stars: 108 } as ImagePsfFwhm)).toBe("PSF FWHM 3.51 px = 0.918 arcsec ± 0.13 px, 108 stars");
  });
});

describe("Cube Prep FWHM table", () => {
  it("FWHM rows come from the kernels and from the PSFs without a kernel", () => {
    expect(psfFwhmRows(STEPS)).toEqual([
      { filter: "F200W", sourcePx: 2.2, targetPx: 4.8, matchedPx: 4.83, scaleArcsec: 0.03, reference: false },
      { filter: "F444W", sourcePx: 4.8, targetPx: null, matchedPx: null, scaleArcsec: 0.03, reference: true },
    ]);
    expect(psfFwhmRows([STEPS[0]])).toEqual([
      { filter: "F200W", sourcePx: 2.2, targetPx: null, matchedPx: null, scaleArcsec: 0.03, reference: false },
      { filter: "F444W", sourcePx: 4.8, targetPx: null, matchedPx: null, scaleArcsec: 0.03, reference: false },
    ]);
    const later = { step: "kernels", report: { reference: "F200W", pixel_scale_arcsec: 0.06, kernels: [{ filter: "F444W", diagnostics: { source_fwhm_px: 2.4, target_fwhm_px: 2.5, matched_fwhm_px: 2.51 } }] } };
    expect(psfFwhmRows([...STEPS, later])).toEqual([
      { filter: "F444W", sourcePx: 2.4, targetPx: 2.5, matchedPx: 2.51, scaleArcsec: 0.06, reference: false },
      { filter: "F200W", sourcePx: 2.2, targetPx: null, matchedPx: null, scaleArcsec: 0.03, reference: true },
    ]);
  });

  it("unexpected reports give no row and never throw", () => {
    expect(psfFwhmRows([])).toEqual([]);
    expect(psfFwhmRows([{ step: "kernels", report: null }])).toEqual([]);
    expect(psfFwhmRows([{ step: "kernels", report: { kernels: "x" } }])).toEqual([]);
    expect(psfFwhmRows([{ step: "psfs", report: { prepared: 3 } }])).toEqual([]);
    const odd = { filter: "F200W", diagnostics: { source_fwhm_px: "2", target_fwhm_px: -1, matched_fwhm_px: Number.NaN } };
    expect(psfFwhmRows([{ step: "kernels", report: { kernels: [7, {}, { filter: " " }, odd, { filter: "F090W", diagnostics: 5 }] } }])).toEqual([
      { filter: "F200W", sourcePx: null, targetPx: null, matchedPx: null, scaleArcsec: null, reference: false },
      { filter: "F090W", sourcePx: null, targetPx: null, matchedPx: null, scaleArcsec: null, reference: false },
    ]);
    expect(psfFwhmRows([{ step: "psfs", report: { prepared: { psfs: [7, {}, { filter: " " }] } } }])).toEqual([]);
  });

  it("cells print pixels and arcsec", () => {
    expect(fwhmCell(2.2, 0.03)).toBe('2.20 px (0.066")');
    expect(fwhmCell(4.83, 0.03)).toBe('4.83 px (0.145")');
    expect(fwhmCell(2.2, null)).toBe("2.20 px");
    expect(fwhmCell(null, 0.03)).toBe("--");
  });

  it("the common PSF is the target of the kernels", () => {
    expect(commonPsfArcsec(STEPS)).toBeCloseTo(0.144, 12);
    expect(commonPsfArcsec([STEPS[0]])).toBeNull();
    expect(commonPsfArcsec([])).toBeNull();
  });
});

describe("Cube Prep redshift", () => {
  it("the redshift request", () => {
    expect(redshiftRequest(2, DEFAULT_RESOLUTION_SETTINGS, null)).toMatchObject({ redshift: 2, psf_fwhm_arcsec: null, psf_reference: null, pixel_scale_arcsec: null, h0_km_s_mpc: 67.66, omega_m: 0.30966 });
    expect(redshiftRequest(2, DEFAULT_RESOLUTION_SETTINGS, 0.144)).toMatchObject({ psf_fwhm_arcsec: 0.144, psf_reference: "target PSF of the last Cube Prep run" });
    expect(COMMON_PSF_REFERENCE).toBe("target PSF of the last Cube Prep run");
    for (const redshift of [null, undefined, "2", 0, -1, 25, Number.NaN]) expect(redshiftRequest(redshift, DEFAULT_RESOLUTION_SETTINGS, null)).toBeNull();
    expect(redshiftRequest(2, { ...DEFAULT_RESOLUTION_SETTINGS, redshift: 0.5 }, null)?.redshift).toBe(2);
  });

  it("the redshift note", () => {
    expect(redshiftNote(CHECK)).toEqual({ scale: "8.58 kpc/arcsec at z = 2", psf: null, cosmology: COSMOLOGY });
    expect(redshiftNote({ ...CHECK, psf_fwhm_arcsec: 0.144, psf_fwhm_kpc: 1.2352716445544052 }).psf).toBe("0.144 arcsec = 1.24 kpc");
  });

  it("file kinds", () => {
    for (const name of ["a.fits", "A.FIT", "a.fts.gz", "a.fits.bz2", "A.FITS.BZ2", "a.fits.fz", "B.FITS.FZ"]) expect(FITS_INPUT.test(name)).toBe(true);
    for (const name of ["a.png", "a.fits.zip", "a.bz2", "a.fz", "fits"]) expect(FITS_INPUT.test(name)).toBe(false);
    expect(FITS_INPUT_EXTENSIONS).toEqual(["fits", "fit", "fts", "gz", "bz2", "fz"]);
  });

  it("the editor says what the redshift is for", () => {
    const HEADER: CubePrepStep = { step: "edit", action: "header", redshift: 2, ra_center: null, dec_center: null, use_brightest_pixel: false };
    const NOTE: RedshiftNote = { scale: "8.58 kpc/arcsec at z = 2", psf: "0.144 arcsec = 1.24 kpc", cosmology: "C" };
    const recipe = createCubePrepDraft().recipe;
    const markup = (step: CubePrepStep, note?: RedshiftNote | null) => renderToStaticMarkup(createElement(CubePrepStepEditor, { step, recipe, onChange: vi.fn(), browse: vi.fn(), redshiftNote: note }));
    expect(REDSHIFT_HELP).toBe("Written to the cube header as REDSHIFT. The SAGUI check reads it back and gives the PSF in kpc; Cube Prep itself does not use it.");
    const plain = markup(HEADER);
    expect(plain).toContain(REDSHIFT_HELP);
    expect(plain).not.toContain("data-redshift-scale");
    const noted = markup(HEADER, NOTE);
    expect(noted).toContain('title="C"');
    expect(noted).toContain("8.58 kpc/arcsec at z = 2 · common PSF FWHM 0.144 arcsec = 1.24 kpc");
    expect(markup(HEADER, { ...NOTE, psf: null })).toContain(">8.58 kpc/arcsec at z = 2</p>");
    expect(markup(HEADER, { ...NOTE, scale: null })).not.toContain("data-redshift-scale");
    const other = markup({ step: "edit", action: "drop_invalid", threshold: 0, frac_valid: 0.01, use_dq: false }, NOTE);
    expect(other).not.toContain(REDSHIFT_HELP);
    expect(other).not.toContain("data-redshift-scale");
    expect(isValidElement(CubePrepStepEditor({ step: HEADER, recipe, onChange: vi.fn(), browse: vi.fn(), redshiftNote: NOTE }))).toBe(true);
  });
});
