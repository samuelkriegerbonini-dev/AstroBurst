import type { ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../../test/hookRuntime";
import type { SaguiInputInfo } from "../../../shared/types/sagui";
import type { ResolutionCheck, ResolutionSettings } from "../../../shared/types/resolution";
import { DEFAULT_RESOLUTION_SETTINGS } from "../../../utils/resolutionText";

const mocks = vi.hoisted(() => ({ check: vi.fn() }));
const live = vi.hoisted(() => ({ settings: {} as ResolutionSettings }));
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/resolution", () => ({ resolutionCheck: mocks.check }));
vi.mock("../../../hooks/useResolutionSettings", () => ({ useResolutionSettings: () => live.settings }));

import SaguiInputPsf, { SaguiInputPsfView, type SaguiInputPsfProps, type SaguiInputPsfViewProps } from "../SaguiInputPsf";
import { physicalRequest } from "../saguiInputPsfModel";

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
const AT_Z2 = { ...INPUT_PSF, redshift: 2 };

function render(props: SaguiInputPsfProps): SaguiInputPsfViewProps {
  const element = hooks.render(SaguiInputPsf, props) as ReactElement<SaguiInputPsfViewProps>;
  expect(element.type).toBe(SaguiInputPsfView);
  return element.props;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  hooks.mount();
  vi.resetAllMocks();
  live.settings = { ...DEFAULT_RESOLUTION_SETTINGS };
});

afterEach(() => {
  hooks.mount();
});

describe("SAGUI input PSF check command", () => {
  it("without a redshift no command is called", () => {
    const props = { input: INPUT_PSF, psfTarget: "F444W", nRegions: 20 };
    render(props);
    const view = render(props);
    expect(mocks.check).not.toHaveBeenCalled();
    expect(view.check).toBeNull();
    expect(view.error).toBeNull();
  });

  it("with a redshift the command is called once per request and its answer reaches the view", async () => {
    const answer = deferred<ResolutionCheck>();
    mocks.check.mockReturnValue(answer.promise);
    const props = { input: AT_Z2, psfTarget: "F444W", nRegions: 20 };
    render(props);
    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(mocks.check).toHaveBeenCalledWith(physicalRequest(AT_Z2, "F444W", live.settings));
    expect(render(props).check).toBeNull();
    answer.resolve(CHECK);
    await settle();
    const view = render(props);
    expect(view.check).toBe(CHECK);
    expect(view.error).toBeNull();
    for (let index = 0; index < 3; index += 1) render(props);
    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(render({ ...props, psfTarget: "F200W" }).check).toBeNull();
  });

  it("an answer for an older request is never shown", async () => {
    const first = deferred<ResolutionCheck>();
    const second = deferred<ResolutionCheck>();
    mocks.check.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise).mockResolvedValue(CHECK);
    render({ input: AT_Z2, psfTarget: "F444W", nRegions: 20 });
    const changed = { input: AT_Z2, psfTarget: "F200W", nRegions: 20 };
    expect(render(changed).check).toBeNull();
    first.resolve(CHECK);
    await settle();
    expect(render(changed).check).toBeNull();
    const f200w = { ...CHECK, psf_reference: "PSF_FWHM of F200W in the cube" };
    second.resolve(f200w);
    await settle();
    expect(render(changed).check).toBe(f200w);
    expect(mocks.check).toHaveBeenCalledTimes(2);
    live.settings = { ...live.settings, h0: 70 };
    render(changed);
    expect(mocks.check).toHaveBeenCalledTimes(3);
    expect(mocks.check).toHaveBeenLastCalledWith(expect.objectContaining({ h0_km_s_mpc: 70 }));
  });

  it("an older answer that arrives last does not replace the newer one", async () => {
    const first = deferred<ResolutionCheck>();
    const second = deferred<ResolutionCheck>();
    mocks.check.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    render({ input: AT_Z2, psfTarget: "F444W", nRegions: 20 });
    const changed = { input: AT_Z2, psfTarget: "F200W", nRegions: 20 };
    render(changed);
    const f200w = { ...CHECK, psf_reference: "PSF_FWHM of F200W in the cube" };
    second.resolve(f200w);
    await settle();
    first.resolve(CHECK);
    await settle();
    expect(render(changed).check).toBe(f200w);
  });

  it("a rejection reaches the view as text", async () => {
    const props = { input: AT_Z2, psfTarget: "F444W", nRegions: 20 };
    mocks.check.mockRejectedValueOnce(new Error("z out of range"));
    render(props);
    await settle();
    expect(render(props)).toMatchObject({ check: null, error: "z out of range" });
    hooks.mount();
    mocks.check.mockRejectedValueOnce("plain");
    render(props);
    await settle();
    expect(render(props)).toMatchObject({ check: null, error: "plain" });
  });
});
