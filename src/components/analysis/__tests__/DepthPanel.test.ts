import { createElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import type { ImageDepth } from "../../../shared/types/depth";
import type { ResolutionSettings } from "../../../shared/types/resolution";
import type { DepthState } from "../../../hooks/useDepthStore";
import type { ImagePsfState } from "../../../hooks/useResolutionSettings";

const mocks = vi.hoisted(() => ({
  add: vi.fn(),
  remove: vi.fn(),
  append: vi.fn(),
  save: vi.fn(async (): Promise<string | null> => "C:/out/a_depth.csv"),
}));

const live = vi.hoisted(() => ({
  state: { depth: null, loading: false, error: null } as DepthState,
  settings: null as ResolutionSettings | null,
  imagePsf: { psf: null, loading: false, error: null } as ImagePsfState,
}));

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

vi.mock("../../../hooks/useDepthStore", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  useDepth: () => live.state,
}));

vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => "rk" }));

vi.mock("../../../hooks/useResolutionSettings", () => ({
  useResolutionSettings: () => live.settings,
  useImagePsf: () => live.imagePsf,
  useHeaderSeeing: () => [],
}));

vi.mock("../../../utils/overlayStore", () => ({ overlayStore: { add: mocks.add, remove: mocks.remove } }));

vi.mock("../../../hooks/useMeasurementLog", () => ({
  useMeasurementProvenance: () => ({ file: "a.fits", image: "original" }),
}));

vi.mock("../../../utils/measurementLog", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  measurementLog: { append: mocks.append },
}));

vi.mock("../../../utils/spectrumExport", () => ({ saveCsvDialog: mocks.save }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf } from "../../../test/elementTree";
import { DepthPanelInner, DepthResultView } from "../DepthPanel";
import { DEFAULT_DEPTH_FIELDS, depthOptions } from "../depthPanelModel";
import { depthStore } from "../../../hooks/useDepthStore";
import { imageDepthEntry } from "../../../utils/measurementLogQa";
import { DEFAULT_RESOLUTION_SETTINGS } from "../../../utils/resolutionText";
import { RunButton } from "../../ui";

type Props = Record<string, unknown> & { children?: ReactNode };
type Element = ReactElement<Props>;

const DEPTH: ImageDepth = {
  radius_arcsec: 1,
  radius_px: 3.8167938931297707,
  pixel_scale_arcsec: 0.262,
  n_requested: 500,
  n_placed: 500,
  n_used: 498,
  seed: 1,
  n_sigma: 5,
  detect_nsigma: 3,
  masked_fraction: 0.45,
  median_sum_native: 0.0002,
  sigma_empty_native: 0.0118,
  sigma_clipped_native: 0.0117,
  n_after_clip: 490,
  sigma_empty_rel_err: 0.0522,
  sigma_formal_native: 0.0071,
  f_corr: 1.662,
  limit_native: 0.059,
  limit_jy: 2.142e-7,
  limit_ab: 25.573,
  psf_fwhm_arcsec: null,
  psf_limit_formal_native: null,
  psf_limit_formal_ab: null,
  sigma_pix_median_native: 0.00072,
  sigma_pix_spread: 1.99,
  bunit: "nanomaggy",
  calibration_origin: "BUNIT nanomaggy, zero point 22.5",
  err_origin: "inverse variance from brick-invvar-r.fits",
  dq_source: "maskbits from brick-maskbits.fits",
  definition: "5 sigma, empty apertures of radius 1.00 arcsec, 1.4826 x MAD of 498 sums, sources masked at 3 sigma, seed 1",
  apertures: [
    { x: 10.25, y: 20.5, sum: 0.012, formal_err: 0.007, usable: true },
    { x: 30, y: 40, sum: -0.004, formal_err: null, usable: false },
  ],
  notes: ["n1"],
  warnings: ["w1"],
  elapsed_ms: 900,
};

const NO_WCS = "The image has no celestial WCS: give the aperture radius in pixels";

const resultMarkup = (depth: ImageDepth) => renderToStaticMarkup(createElement(DepthResultView, { depth }));

function cardValue(markup: string, label: string): string | null {
  const match = new RegExp(`>${label}</div><div[^>]*>([^<]*)</div>`).exec(markup);
  return match ? match[1] : null;
}

type PanelProps = { filePath: string | null; measureKey?: string | null };
const show = (props: PanelProps) => elementsOf<Props>(hooks.render(DepthPanelInner, props)) as Element[];
const byData = (shown: Element[], name: string, value: string) => shown.find((e) => e.props[name] === value) as Element;
const field = (shown: Element[], name: string) => byData(shown, "data-field", name);
const action = (shown: Element[], name: string) => byData(shown, "data-action", name);
const runButton = (shown: Element[]) => shown.find((e) => e.type === RunButton) as Element;
const click = (element: Element) => (element.props.onClick as () => void)();
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("DepthResultView", () => {
  it("prints the depth with its definition", () => {
    const markup = resultMarkup(DEPTH);
    expect(markup).toContain("data-depth-result");
    expect(markup).toContain("5 sigma depth");
    expect(markup).toContain("25.57 AB mag (0.214 uJy)");
    expect(markup).toContain(DEPTH.definition);
    expect(markup).toContain("scatter known to 5.2 percent");
  });

  it("cards say where each number comes from", () => {
    const markup = resultMarkup(DEPTH);
    expect(cardValue(markup, "empty-aperture sigma")).toBe("0.0118 nanomaggy");
    expect(cardValue(markup, "formal sigma")).toBe("0.0071 nanomaggy");
    expect(markup).not.toContain("x pix");
    expect(cardValue(markup, "f_corr")).toBe("1.66");
    expect(cardValue(markup, "clipped sigma")).toBe("0.0117 nanomaggy");
    expect(cardValue(markup, "apertures")).toBe("498 used, 500 placed, 500 requested");
    expect(cardValue(markup, "masked")).toBe("45.0 percent");
    expect(cardValue(markup, "pixel sigma")).toBe("0.00072 nanomaggy, x1.99 across the image");
    expect(cardValue(markup, "calibration")).toBe("BUNIT nanomaggy, zero point 22.5");
    expect(cardValue(markup, "error layer")).toBe("inverse variance from brick-invvar-r.fits");
    expect(cardValue(markup, "mask")).toBe("maskbits from brick-maskbits.fits");
    expect(cardValue(markup, "point-source limit \\(formal\\)")).toBe("--");
    expect(markup).toContain('title="Median over the apertures of the error the error layer gives"');
    expect(markup).toContain(
      'title="Empty-aperture scatter over the formal error: the errors of the file are this many times too small for an aperture of this size"',
    );
    expect(markup).toContain('title="Standard deviation after 3 sigma clipping; the limit uses the MAD value"');
    expect(markup).toContain('title="No correlated-noise correction; needs an error layer and a PSF FWHM"');
    expect(markup).toContain('title="498 used, 500 placed, 500 requested"');
    expect(markup).toContain("break-words");

    const psf = resultMarkup({ ...DEPTH, psf_limit_formal_ab: 26.18, psf_limit_formal_native: 0.031, psf_fwhm_arcsec: 1.084 });
    expect(cardValue(psf, "point-source limit \\(formal\\)")).toBe("26.18 AB mag at FWHM 1.08 arcsec");
    const psfNative = resultMarkup({ ...DEPTH, psf_limit_formal_native: 0.031, psf_fwhm_arcsec: 1.084 });
    expect(cardValue(psfNative, "point-source limit \\(formal\\)")).toBe("0.031 nanomaggy");

    const surface = resultMarkup({ ...DEPTH, bunit: "MJy/sr" });
    expect(cardValue(surface, "empty-aperture sigma")).toBe("0.0118 MJy/sr x pix");
    expect(cardValue(surface, "pixel sigma")).toBe("0.00072 MJy/sr, x1.99 across the image");

    const even = resultMarkup({ ...DEPTH, sigma_pix_spread: 1.04 });
    expect(cardValue(even, "pixel sigma")).toBe("0.00072 nanomaggy");
  });

  it("an uncalibrated image keeps native units and says so", () => {
    const markup = resultMarkup({
      ...DEPTH,
      limit_ab: null,
      limit_jy: null,
      calibration_origin: null,
      bunit: null,
      sigma_formal_native: null,
      f_corr: null,
      err_origin: null,
      dq_source: null,
      sigma_pix_median_native: null,
    });
    expect(markup).toContain("0.059 image units x pix (not calibrated)");
    expect(markup).not.toContain("AB mag");
    expect(markup).not.toContain("uJy");
    for (const label of ["calibration", "error layer", "mask"]) expect(cardValue(markup, label)).toBe("none");
    for (const label of ["formal sigma", "f_corr", "pixel sigma"]) expect(cardValue(markup, label)).toBe("--");
    expect(cardValue(markup, "empty-aperture sigma")).toBe("0.0118 image units x pix");

    const jyOnly = resultMarkup({ ...DEPTH, limit_ab: null });
    expect(jyOnly).toContain(">0.214 uJy<");
  });

  it("warnings and notes of the command are shown", () => {
    const markup = resultMarkup(DEPTH);
    expect(markup).toContain("w1");
    expect(markup).toMatch(/<summary[^>]*>notes \(1\)<\/summary>/);
    expect(markup).toContain("n1");
  });
});

describe("DepthPanelInner", () => {
  let measure: MockInstance<typeof depthStore.measure>;

  beforeEach(() => {
    hooks.mount();
    live.state = { depth: null, loading: false, error: null };
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS };
    live.imagePsf = { psf: null, loading: false, error: null };
    mocks.add.mockClear();
    mocks.remove.mockClear();
    mocks.append.mockClear();
    mocks.save.mockClear();
    measure = vi.spyOn(depthStore, "measure").mockResolvedValue(undefined);
  });

  afterEach(() => {
    hooks.mount();
    measure.mockRestore();
  });

  it("the defaults ask the command for its own defaults", () => {
    expect(depthOptions(DEFAULT_DEPTH_FIELDS)).toEqual({
      radiusArcsec: null,
      radiusPx: null,
      nApertures: 500,
      nSigma: 5,
      seed: 1,
      detectNsigma: 3,
      psfFwhmArcsec: null,
      excludeDq: null,
    });
    const px = depthOptions({ ...DEFAULT_DEPTH_FIELDS, radius: 4, radiusUnit: "px", useMask: false });
    expect(px.radiusPx).toBe(4);
    expect(px.radiusArcsec).toBeNull();
    expect(px.excludeDq).toBe(false);
    expect(depthOptions({ ...DEFAULT_DEPTH_FIELDS, radius: 1.5 }).radiusArcsec).toBe(1.5);
  });

  it("Measure depth runs the store with the key of the measured image", () => {
    const props = { filePath: "a.fits", measureKey: "v1" };
    let shown = show(props);
    const run = runButton(shown);
    expect(run.props.label).toBe("Measure depth");
    expect(run.props.runningLabel).toBe("Measuring…");
    expect(run.props.disabled).toBe(false);
    click(run);
    expect(measure).toHaveBeenCalledWith("a.fits|v1", "a.fits", depthOptions(DEFAULT_DEPTH_FIELDS));

    (field(shown, "radius").props.onValueChange as (value: number | null) => void)(2);
    shown = show(props);
    (field(shown, "radiusUnit").props.onChange as (event: { target: { value: string } }) => void)({ target: { value: "px" } });
    shown = show(props);
    click(runButton(shown));
    expect(measure).toHaveBeenLastCalledWith("a.fits|v1", "a.fits", { ...depthOptions(DEFAULT_DEPTH_FIELDS), radiusPx: 2 });

    hooks.mount();
    const empty = runButton(show({ filePath: null }));
    expect(empty.props.disabled).toBe(true);
  });

  it("the result is drawn on the image and removed when hidden", () => {
    live.state = { depth: DEPTH, loading: false, error: null };
    const props = { filePath: "a.fits", measureKey: null };
    const shown = show(props);
    expect(mocks.add).toHaveBeenCalledTimes(1);
    expect(mocks.add.mock.calls[0][0]).toBe("rk");
    const layer = mocks.add.mock.calls[0][1] as { id: string; kind: string; visible: boolean; paint: unknown };
    expect(layer.id).toBe("depth-apertures");
    expect(layer.kind).toBe("depth-apertures");
    expect(layer.visible).toBe(true);
    expect(typeof layer.paint).toBe("function");
    expect(mocks.remove).not.toHaveBeenCalled();

    (field(shown, "showApertures").props.onChange as (event: { target: { checked: boolean } }) => void)({ target: { checked: false } });
    show(props);
    expect(mocks.remove).toHaveBeenCalledWith("rk", "depth-apertures");
    expect(mocks.add).toHaveBeenCalledTimes(1);
  });

  it("the PSF of the Resolution section can be copied into the field", () => {
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, psfSource: "typed", psfFwhmArcsec: 1.1 };
    const props = { filePath: "a.fits", measureKey: null };
    let shown = show(props);
    const copy = action(shown, "use-resolution-psf");
    expect(copy.props.disabled).toBe(false);
    click(copy);
    shown = show(props);
    expect(field(shown, "psfFwhmArcsec").props.value).toBe(1.1);
    click(runButton(shown));
    expect(measure).toHaveBeenLastCalledWith("a.fits|", "a.fits", { ...depthOptions(DEFAULT_DEPTH_FIELDS), psfFwhmArcsec: 1.1 });

    hooks.mount();
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, psfSource: "image" };
    expect(action(show(props), "use-resolution-psf").props.disabled).toBe(true);
  });

  it("Log and Save CSV use the builders of S2", async () => {
    live.state = { depth: DEPTH, loading: false, error: null };
    const props = { filePath: "a.fits", measureKey: null };
    const shown = show(props);
    click(action(shown, "log-depth"));
    expect(mocks.append).toHaveBeenCalledWith(imageDepthEntry({ file: "a.fits", image: "original" }, DEPTH));
    click(action(shown, "save-depth-csv"));
    await flush();
    expect(mocks.save).toHaveBeenCalledTimes(1);
    const [text, name, title] = mocks.save.mock.calls[0] as unknown as [string, string, string];
    expect(text.startsWith("# software: AstroBurst ")).toBe(true);
    expect(text).toContain("# file: a.fits");
    expect(name).toBe("a_depth.csv");
    expect(title).toBe("Save the empty apertures");
    for (const name of ["copy-depth-csv", "save-depth-csv", "log-depth"]) expect(action(shown, name).props.disabled).toBe(false);

    live.state = { depth: DEPTH, loading: true, error: null };
    const loading = show(props);
    for (const name of ["copy-depth-csv", "save-depth-csv", "log-depth"]) expect(action(loading, name).props.disabled).toBe(true);
    expect(runButton(loading).props.running).toBe(true);
  });

  it("an error of the command is shown as it came", () => {
    live.state = { depth: null, loading: false, error: NO_WCS };
    const markup = renderToStaticMarkup(createElement(DepthPanelInner, { filePath: "a.fits", measureKey: null }));
    expect(markup).toContain(`role="alert">${NO_WCS}<`);
    expect(markup).toContain("data-depth-panel");
    expect(markup).not.toContain("data-depth-result");
  });
});
