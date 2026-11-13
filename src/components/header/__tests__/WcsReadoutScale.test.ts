import { isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../../test/hookRuntime";
import type { WcsInfo } from "../../../shared/types/astrometry";
import type { ResolutionCheck, ResolutionSettings } from "../../../shared/types/resolution";
import { DEFAULT_RESOLUTION_SETTINGS, resolutionRequest, type PsfChoice } from "../../../utils/resolutionText";

const mocks = vi.hoisted(() => ({ info: vi.fn(), world: vi.fn(), check: vi.fn() }));
const live = vi.hoisted(() => ({ settings: null as unknown as ResolutionSettings }));
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/astrometry", () => ({ getWcsInfo: mocks.info, pixelToWorld: mocks.world }));
vi.mock("../../../services/resolution", () => ({ resolutionCheck: mocks.check }));
vi.mock("../../../hooks/useResolutionSettings", () => ({ useResolutionSettings: () => live.settings }));

import WcsReadout from "../WcsReadout";

type ReadoutProps = { filePath: string; mouseX: number | null; mouseY: number | null; imageWidth: number; imageHeight: number };

const readoutProps: ReadoutProps = { filePath: "a.fits", mouseX: 4, mouseY: 5, imageWidth: 200, imageHeight: 100 };
const component = (WcsReadout as unknown as { type: (props: ReadoutProps) => ReactNode }).type;
const tree = (props = readoutProps) => hooks.render(component, props) as ReactNode;
const markup = (props = readoutProps) => renderToStaticMarkup(tree(props));

const NO_PSF: PsfChoice = { fwhmArcsec: null, halfLightRadiusArcsec: null, reference: null, missing: null };
const COSMOLOGY = "flat LambdaCDM, H0 67.66, Omega_m 0.30966, plus Planck 2018 neutrinos and radiation";

const CHECK2: ResolutionCheck = {
  verdict: null,
  size_basis: null,
  psf_fwhm_arcsec: null,
  psf_reference: null,
  pixels_per_fwhm: null,
  nyquist_sampled: null,
  psf_area_px: null,
  intrinsic_size_arcsec: null,
  snr_floor_fraction: null,
  psf_floor_fraction: 0,
  floor_fraction: null,
  size_floor_arcsec: null,
  resolution_elements: null,
  kpc_per_arcsec: 8.578275309405592,
  kpc_per_pixel: 0.2659265345915733,
  angular_diameter_distance_mpc: 1769.4,
  psf_fwhm_kpc: null,
  intrinsic_size_kpc: null,
  size_floor_kpc: null,
  scale_of_interest_in_psf: null,
  snr: null,
  redshift: 2,
  psf_fwhm_rel_err: 0.03,
  resolved_elements: 3,
  min_pixels_per_fwhm: 2,
  scale_of_interest_kpc: 1,
  h0_km_s_mpc: 67.66,
  omega_m: 0.30966,
  omega_nu_massive: 0.0014147613306646316,
  omega_rad: 7.893312479128647e-5,
  cosmology: COSMOLOGY,
  notes: [],
  warnings: [],
};

function info(scale: number, centre: [number, number] = [30, 10]): WcsInfo {
  return { center_ra: centre[0], center_dec: centre[1], pixel_scale_arcsec: scale, fov_arcmin: [1, 2], field_of_view_w_arcmin: 1, field_of_view_h_arcmin: 2, naxis1: 200, naxis2: 100 };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

type SelectProps = { children?: ReactNode; "aria-label"?: string; onChange?: (event: unknown) => void };

function selects(node: ReactNode): SelectProps[] {
  if (Array.isArray(node)) return node.flatMap(selects);
  if (!isValidElement<SelectProps>(node)) return [];
  return node.type === "select" ? [node.props] : selects(node.props.children);
}

function choose(node: ReactNode, label: string, value: string): void {
  for (const select of selects(node)) if (select["aria-label"] === label) select.onChange?.({ target: { value } });
}

async function settle(props = readoutProps): Promise<string> {
  markup(props);
  await vi.advanceTimersByTimeAsync(0);
  markup(props);
  await vi.advanceTimersByTimeAsync(40);
  markup(props);
  await vi.advanceTimersByTimeAsync(0);
  return markup(props);
}

function stubReadoutPreference(text: string): void {
  vi.stubGlobal("window", { localStorage: { getItem: () => text, setItem: vi.fn() } });
}

beforeEach(() => {
  hooks.mount();
  vi.useFakeTimers();
  vi.resetAllMocks();
  stubReadoutPreference('{"frame":"icrs","format":"decimal"}');
  live.settings = { ...DEFAULT_RESOLUTION_SETTINGS };
  mocks.info.mockResolvedValue(info(0.031));
  mocks.world.mockResolvedValue({ points: [[11.125, 22.25]], frame: "icrs" });
  mocks.check.mockResolvedValue(CHECK2);
});

afterEach(() => {
  hooks.mount();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("WCS readout physical scale", () => {
  it("without a redshift the readout has no physical scale and asks for none", async () => {
    const html = await settle();
    expect(html).toContain("0.03100&quot;/px");
    expect(html).not.toContain("data-readout-scale");
    expect(mocks.check).not.toHaveBeenCalled();
  });

  it("with a redshift it prints kpc per arcsec and per pixel", async () => {
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, redshift: 2 };
    const html = await settle();
    expect(html).toContain("8.58 kpc/arcsec at z = 2 (0.266 kpc/px)");
    expect(mocks.check).toHaveBeenCalledTimes(1);
    const expected = resolutionRequest(live.settings, NO_PSF, 0.031, {});
    expect(mocks.check).toHaveBeenCalledWith(expected);
    expect(expected.psf_fwhm_arcsec).toBeNull();
    expect(expected.redshift).toBe(2);
    expect(expected.h0_km_s_mpc).toBe(67.66);
    expect(expected.omega_m).toBe(0.30966);
  });

  it("the cosmology is in the title", async () => {
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, redshift: 2 };
    const html = await settle();
    const element = html.match(/<div[^>]*data-readout-scale[^>]*>/)?.[0] ?? "";
    expect(element).toContain(
      `title="${COSMOLOGY}. Redshift from the Resolution section of the Photometry tool."`,
    );
  });

  it("a change of settings asks again and hides the old line meanwhile", async () => {
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, redshift: 2 };
    expect(await settle()).toContain("8.58 kpc/arcsec at z = 2 (0.266 kpc/px)");
    const next = deferred<ResolutionCheck>();
    mocks.check.mockReturnValue(next.promise);
    live.settings = { ...live.settings, h0: 70 };
    expect(markup()).not.toContain("data-readout-scale");
    await vi.advanceTimersByTimeAsync(0);
    expect(markup()).not.toContain("data-readout-scale");
    next.resolve({ ...CHECK2, h0_km_s_mpc: 70, kpc_per_arcsec: 8.29, kpc_per_pixel: 0.25699 });
    await vi.advanceTimersByTimeAsync(0);
    const html = markup();
    expect(html).toContain("8.29 kpc/arcsec at z = 2 (0.257 kpc/px)");
    expect(html).not.toContain("8.58 kpc/arcsec");
    expect(mocks.check).toHaveBeenCalledTimes(2);
    expect(mocks.check).toHaveBeenLastCalledWith(resolutionRequest(live.settings, NO_PSF, 0.031, {}));
  });

  it("a failed check shows nothing", async () => {
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, redshift: 2 };
    mocks.check.mockRejectedValue(new Error("cosmology failed"));
    const html = await settle();
    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(html).not.toContain("data-readout-scale");
    expect(html).toContain("0.03100&quot;/px");
  });

  it("clearing the redshift removes the line", async () => {
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, redshift: 2 };
    expect(await settle()).toContain("data-readout-scale");
    live.settings = { ...live.settings, redshift: null };
    markup();
    await vi.advanceTimersByTimeAsync(0);
    expect(markup()).not.toContain("data-readout-scale");
    expect(mocks.check).toHaveBeenCalledTimes(1);
  });
});

describe("sexagesimal digits", () => {
  const centred = { ...readoutProps, mouseX: null, mouseY: null };

  beforeEach(() => {
    stubReadoutPreference('{"frame":"icrs","format":"sexagesimal"}');
    mocks.info.mockResolvedValue(info(0.031, [274.72992, -13.85181]));
    mocks.world.mockResolvedValue({ points: [[17.25, -13.85181]], frame: "galactic" });
  });

  it("prints the centre to 0.001 s and 0.01 arcsec", async () => {
    const html = await settle(centred);
    expect(html).toContain(">centre<");
    expect(html).toContain("18h18m55.181s");
    expect(html).toContain("-13°51&#x27;06.52&quot;");
  });

  it("a longitude in degrees gets 0.01 arcsec too", async () => {
    await settle();
    choose(tree(), "Coordinate frame", "galactic");
    const html = await settle();
    expect(mocks.world).toHaveBeenLastCalledWith("a.fits", [[4, 5]], "galactic");
    expect(html).toContain("17°15&#x27;00.00&quot;");
    expect(html).toContain("-13°51&#x27;06.52&quot;");
  });

  it("the decimal format keeps six decimals", async () => {
    stubReadoutPreference('{"frame":"icrs","format":"decimal"}');
    const html = await settle(centred);
    expect(html).toContain("274.729920");
    expect(html).toContain("-13.851810");
  });
});
