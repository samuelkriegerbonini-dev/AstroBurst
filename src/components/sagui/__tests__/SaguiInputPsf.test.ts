import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { SaguiState } from "../saguiStore";
import type { SaguiInputInfo } from "../../../shared/types/sagui";
import type { ResolutionCheck } from "../../../shared/types/resolution";
import { DEFAULT_RESOLUTION_SETTINGS } from "../../../utils/resolutionText";

const state: SaguiState = { config: { input_path: "", output_dir: "", rscript_path: "", library_path: "", n_regions: 20, starlet: true, starlet_scales: [1, 2, 3], starlet_threshold: 3, dq_exclude_bits: 4294967295, psf_target: "", mode: "ward", knn_k: 40 }, input: null, runtime: null, status: "idle", operation: null, installation: null, installParent: "", progress: null, result: null, outputDir: null, unit: null, error: null };
vi.mock("../saguiStore", async (original) => ({ ...await original<typeof import("../saguiStore")>(), useSaguiState: () => state }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getSelectedFile: () => null } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
vi.mock("../../../hooks/useResolutionSettings", async () => {
  const { DEFAULT_RESOLUTION_SETTINGS: defaults } = await import("../../../utils/resolutionText");
  return { useResolutionSettings: () => defaults };
});
import SaguiPanel from "../SaguiPanel";
import { SaguiInputPsfView, type SaguiInputPsfViewProps } from "../SaguiInputPsf";
import { bandPsfLines, physicalRequest, regionBudgetText } from "../saguiInputPsfModel";

const BAND = { psf_fwhm_arcsec: 0.1388, pixscale_arcsec: 0.04, psf_area_px: 27.28684761586353 };
const INPUT: SaguiInputInfo = { width: 40, height: 30, bands: [{ filter: "F200W", pivot_um: 2 }, { filter: "F444W", pivot_um: 4.4 }], unit: "Jy/pixel", has_err: true, has_dq: true, has_noise_correlation: false, dq_flags: [], estimated_memory_bytes: 2 * 1024 ** 3, warnings: ["NOISCORR extension is absent: regional errors assume independent pixels and error_correlated stays empty; resampling and PSF matching create covariance that is not stored"] };
const INPUT_PSF: SaguiInputInfo = { ...INPUT, bands: INPUT.bands.map((band) => ({ ...band, ...BAND })) };
const CHECK: ResolutionCheck = {
  verdict: null, size_basis: null, psf_fwhm_arcsec: 0.1388, psf_reference: "PSF_FWHM of F444W in the cube", pixels_per_fwhm: 3.47, nyquist_sampled: true, psf_area_px: 27.28684761586353,
  intrinsic_size_arcsec: null, snr_floor_fraction: null, psf_floor_fraction: 0.24677925358506123, floor_fraction: null, size_floor_arcsec: null, resolution_elements: null,
  kpc_per_arcsec: 8.578275309405592, kpc_per_pixel: 0.3431310123762237, angular_diameter_distance_mpc: 1769.396294628795, psf_fwhm_kpc: 1.1906646129454963, intrinsic_size_kpc: null, size_floor_kpc: null,
  scale_of_interest_in_psf: 0.8398670701451139, snr: null, redshift: 2, psf_fwhm_rel_err: 0.03, resolved_elements: 3, min_pixels_per_fwhm: 2, scale_of_interest_kpc: 1, h0_km_s_mpc: 67.66, omega_m: 0.30966,
  omega_nu_massive: 0.0014147613306646316, omega_rad: 7.893312479128647e-5, cosmology: "flat LambdaCDM, H0 67.66, Omega_m 0.30966, plus Planck 2018 neutrinos and radiation", notes: [], warnings: ["w-check"],
};
const OK_TEXT = "20 regions over 40 × 30 pixels give 60.0 pixels per region, above the PSF noise-equivalent area of 27.3 pixels (F444W). Up to 43 regions keep one PSF area each on the full footprint; masks shrink the support, so the run can still warn.";
const WARN_TEXT = "60 regions over 40 × 30 pixels give 20.0 pixels per region, below the PSF noise-equivalent area of 27.3 pixels (F444W): regions smaller than the PSF are not resolved. Request at most 43 regions, fewer when masks shrink the support.";
const NO_CARD = "The cube has no REDSHIFT card: the PSF is given in arcsec only. Cube Prep writes the card in Edit, Set centre / redshift.";
const PSF_LINE = (filter: string, pivot: string) => `${filter} (${pivot} µm) · PSF FWHM 0.1388 arcsec · 0.04000"/px · PSF area 27.3 px`;
const markupText = (text: string) => text.replaceAll('"', "&quot;");

const view = (props: Partial<SaguiInputPsfViewProps>) => renderToStaticMarkup(createElement(SaguiInputPsfView, { input: INPUT, psfTarget: "F444W", nRegions: 20, check: null, error: null, ...props }));

function withState(change: Partial<SaguiState>, check: () => void) {
  const saved = { ...state };
  Object.assign(state, change);
  try { check(); } finally { Object.assign(state, saved); }
}

describe("SAGUI input PSF lines and region budget", () => {
  it("bands are listed as today when the cube states no PSF", () => {
    expect(bandPsfLines(INPUT)).toBeNull();
    const html = view({ input: INPUT, psfTarget: "F444W", nRegions: 20 });
    expect(html).toContain("F200W (2.000 µm) · F444W (4.400 µm)");
    expect(html).not.toContain('data-region-budget="ok"');
    expect(html).not.toContain("REDSHIFT");
  });

  it("each band shows its PSF, pixel scale and PSF area", () => {
    expect(bandPsfLines(INPUT_PSF)).toEqual([PSF_LINE("F200W", "2.000"), PSF_LINE("F444W", "4.400")]);
    const withoutFirst = { ...INPUT_PSF, bands: [INPUT.bands[0], INPUT_PSF.bands[1]] };
    expect(bandPsfLines(withoutFirst)?.[0]).toBe("F200W (2.000 µm) · PSF not stated");
    const withoutArea = { ...INPUT_PSF, bands: [{ ...INPUT_PSF.bands[0], psf_area_px: null }, INPUT_PSF.bands[1]] };
    expect(bandPsfLines(withoutArea)?.[0]).toBe('F200W (2.000 µm) · PSF FWHM 0.1388 arcsec · 0.04000"/px');
  });

  it("the region count is compared with the PSF area before the run", () => {
    expect(regionBudgetText(INPUT_PSF, "f444w ", 20)).toEqual({ tone: "ok", text: OK_TEXT });
    expect(regionBudgetText(INPUT_PSF, "f444w ", 60)).toEqual({ tone: "warn", text: WARN_TEXT });
    expect(regionBudgetText(INPUT_PSF, "F444W", 43)?.tone).toBe("ok");
    expect(regionBudgetText(INPUT_PSF, "F444W", 44)?.tone).toBe("warn");
  });

  it("a target without a stated PSF says that the check waits for the run", () => {
    expect(regionBudgetText(INPUT, "F444W", 20)).toEqual({ tone: "muted", text: "The cube does not state the PSF FWHM and pixel scale of F444W (FILTERS columns PSF_FWHM and PIXSCALE): the region count is compared with the PSF only after the run." });
    expect(regionBudgetText(INPUT_PSF, "", 20)).toBeNull();
    expect(regionBudgetText(INPUT_PSF, "F999X", 20)).toBeNull();
    expect(regionBudgetText(INPUT_PSF, "F444W", 2.5)).toBeNull();
  });

  it("the physical request uses the redshift of the cube and the cosmology of the settings", () => {
    const settings = { ...DEFAULT_RESOLUTION_SETTINGS, redshift: 0.5, h0: 70 };
    expect(physicalRequest(INPUT_PSF, "F444W", settings)).toBeNull();
    for (const redshift of [null, 0, -1, 25, Number.NaN]) expect(physicalRequest({ ...INPUT_PSF, redshift }, "F444W", settings)).toBeNull();
    const request = physicalRequest({ ...INPUT_PSF, redshift: 2 }, "F444W", settings);
    expect(request).toMatchObject({ redshift: 2, h0_km_s_mpc: 70, omega_m: 0.30966, psf_fwhm_arcsec: 0.1388, psf_reference: "PSF_FWHM of F444W in the cube", pixel_scale_arcsec: 0.04, scale_of_interest_kpc: 1, source_fwhm_arcsec: null, source_half_light_radius_arcsec: null, snr: null });
    for (const target of ["", "F999X"]) expect(physicalRequest({ ...INPUT_PSF, redshift: 2 }, target, settings)).toMatchObject({ psf_fwhm_arcsec: null, psf_reference: null, pixel_scale_arcsec: 0.04 });
    expect(physicalRequest({ ...INPUT, redshift: 2 }, "F444W", settings)?.pixel_scale_arcsec).toBeNull();
  });
});

describe("SAGUI input PSF view", () => {
  it("the view prints the budget with its tone and asks for a redshift only when a PSF is stated", () => {
    const warn = view({ input: INPUT_PSF, psfTarget: "F444W", nRegions: 60 });
    expect(warn).toContain('data-region-budget="warn"');
    expect(warn).toContain("text-amber-300");
    expect(warn).toContain(WARN_TEXT);
    expect(warn).toContain(NO_CARD);
    const ok = view({ input: INPUT_PSF, psfTarget: "F444W", nRegions: 20 });
    expect(ok).toContain('data-region-budget="ok"');
    expect(ok).not.toContain("text-amber-300");
  });

  it("with a redshift the view prints the PSF in kpc with the cosmology", () => {
    const html = view({ input: { ...INPUT_PSF, redshift: 2 }, psfTarget: "F444W", nRegions: 20, check: CHECK });
    for (const text of ["Redshift 2 from the REDSHIFT card of the cube.", "0.139 arcsec = 1.19 kpc", "PSF_FWHM of F444W in the cube", "3.47 px per FWHM", "8.58 kpc/arcsec at z = 2 (0.343 kpc/px)", "1 kpc is 0.84 x the PSF FWHM", CHECK.cosmology, "w-check"]) expect(html).toContain(text);
    expect(html).not.toContain(NO_CARD);
  });

  it("a failed check is said in one line and hides nothing else", () => {
    const html = view({ input: { ...INPUT_PSF, redshift: 2 }, psfTarget: "F444W", nRegions: 20, check: null, error: "boom" });
    expect(html).toContain("Physical scale not available: boom");
    expect(html).toContain(markupText(PSF_LINE("F200W", "2.000")));
    expect(html).toContain(markupText(PSF_LINE("F444W", "4.400")));
    expect(html).toContain(OK_TEXT);
  });
});

describe("SAGUI panel input summary", () => {
  it("the panel mounts the block inside the input summary", () => {
    withState({ input: INPUT_PSF, config: { ...state.config, psf_target: "F444W", n_regions: 60 } }, () => {
      const html = renderToStaticMarkup(createElement(SaguiPanel));
      expect(html).toContain("ERR present · DQ present · NOISCORR absent");
      expect(html).toContain("data-sagui-psf");
      expect(html).toContain(WARN_TEXT);
      expect(html).toContain(INPUT.warnings[0]);
    });
    withState({ input: INPUT }, () => {
      const html = renderToStaticMarkup(createElement(SaguiPanel));
      expect(html).toContain("F200W (2.000 µm) · F444W (4.400 µm)");
      expect(html).not.toContain("data-region-budget");
    });
  });
});
