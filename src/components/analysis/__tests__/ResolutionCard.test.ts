import { createElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { ResolutionCardInner, ResolutionCardView, type ResolutionCardProps, type ResolutionCardViewProps } from "../ResolutionCard";
import { imagePsfStore, resolutionSettingsStore } from "../../../hooks/useResolutionSettings";
import { CONTEXT_NOTE, NO_VERDICT_TEXT, VERDICT_SCIENCE, psfChoice, resolutionRequest, resolutionRows } from "../../../utils/resolutionText";
import { resolutionCheckEntry } from "../../../utils/measurementLogQa";
import type { ImagePsfFwhm, ResolutionCheck } from "../../../shared/types/resolution";

const mocks = vi.hoisted(() => ({
  resolutionCheck: vi.fn(),
  imagePsfFwhm: vi.fn(),
  getWcsInfo: vi.fn(),
  getHeader: vi.fn(),
  append: vi.fn(),
}));

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

vi.mock("../../../services/resolution", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  resolutionCheck: mocks.resolutionCheck,
  imagePsfFwhm: mocks.imagePsfFwhm,
}));

vi.mock("../../../services/astrometry", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  getWcsInfo: mocks.getWcsInfo,
}));

vi.mock("../../../services/header", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  getHeader: mocks.getHeader,
}));

vi.mock("../../../hooks/useMeasurementLog", () => ({
  useMeasurementProvenance: () => ({ file: "a.fits", image: "original" }),
}));

vi.mock("../../../utils/measurementLog", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  measurementLog: { append: mocks.append },
}));

const COSMOLOGY = "flat LambdaCDM, H0 67.66, Omega_m 0.30966, plus Planck 2018 neutrinos and radiation";

const CHECK: ResolutionCheck = {
  verdict: "marginal",
  size_basis: "fwhm",
  psf_fwhm_arcsec: 1.14,
  psf_reference: "typed value",
  pixels_per_fwhm: 5.7,
  nyquist_sampled: true,
  psf_area_px: 73.62819050398275,
  intrinsic_size_arcsec: 0.374699879903904,
  snr_floor_fraction: 0.250290234995889,
  psf_floor_fraction: 0.24677925358506123,
  floor_fraction: 0.250290234995889,
  size_floor_arcsec: 0.2853308678953134,
  resolution_elements: 0.3286841051788632,
  kpc_per_arcsec: 8.23125144264795,
  kpc_per_pixel: 1.6462502885295902,
  angular_diameter_distance_mpc: 1697.8174839889118,
  psf_fwhm_kpc: 9.383626644618662,
  intrinsic_size_kpc: 3.084248927019024,
  size_floor_kpc: 2.3486301179952904,
  scale_of_interest_in_psf: 0.10656860485529672,
  snr: 50,
  redshift: 1,
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

const MISSING_IMAGE_PSF = "Measure the PSF of this image in the Resolution section.";
const MISSING_HEADER_CARD = "This file has no SEEING, PSF_FWHM or PSFFWHM card: choose another PSF source.";

function viewProps(over: Partial<ResolutionCardViewProps> = {}): ResolutionCardViewProps {
  return {
    check: CHECK,
    missing: null,
    context: "photometry",
    loading: false,
    error: null,
    canMeasure: false,
    measuring: false,
    onMeasure: vi.fn(),
    onLog: vi.fn(),
    ...over,
  };
}

const markupOf = (props: ResolutionCardViewProps) => renderToStaticMarkup(createElement(ResolutionCardView, props));

function buttonTag(markup: string, text: string): string | null {
  return [...markup.matchAll(/<button[^>]*>([^<]*)<\/button>/g)].find((match) => match[1] === text)?.[0] ?? null;
}

describe("ResolutionCardView", () => {
  it("shows the class, what it allows and every input of the verdict", () => {
    const markup = markupOf(viewProps());
    expect(markup).toContain('data-resolution-card="photometry"');
    expect(markup).toContain('data-resolution-verdict="marginal"');
    expect(markup).toContain("Resolved or not");
    expect(markup).toContain(">Marginally resolved<");
    expect(markup).toContain(VERDICT_SCIENCE.marginal);
    const rows = resolutionRows(CHECK);
    expect(rows.length).toBeGreaterThan(5);
    for (const [label, value] of rows) {
      expect(markup).toContain(`>${label}<`);
      expect(markup).toContain(`>${value}<`);
    }
    expect(markup).toContain("1.14 arcsec = 9.38 kpc");
    expect(markup).toContain(COSMOLOGY);
    expect(markup).toContain(CONTEXT_NOTE.photometry);
    expect(buttonTag(markup, "Log")).not.toBeNull();
    expect(buttonTag(markup, "Log")).toContain('title="Log this check with its inputs and thresholds"');
    expect(buttonTag(markup, "Log")).not.toContain('disabled=""');
    expect(markup).not.toContain("Checking…");
  });

  it("says what is missing and offers to measure the PSF", () => {
    const missing = viewProps({ check: null, missing: MISSING_IMAGE_PSF, canMeasure: true });
    const markup = markupOf(missing);
    expect(markup).toContain(MISSING_IMAGE_PSF);
    expect(markup).toContain('data-resolution-verdict="none"');
    const measure = buttonTag(markup, "Measure PSF from stars");
    expect(measure).not.toBeNull();
    expect(measure).not.toContain('disabled=""');
    expect(buttonTag(markup, "Log")).toBeNull();

    const measuring = buttonTag(markupOf({ ...missing, measuring: true }), "Measuring the PSF…");
    expect(measuring).not.toBeNull();
    expect(measuring).toContain('disabled=""');

    const noButton = markupOf({ ...missing, canMeasure: false });
    expect(noButton).toContain(MISSING_IMAGE_PSF);
    expect(noButton).not.toContain("<button");
  });

  it("a PSF without a source size gives the floor and no class", () => {
    const floorOnly: ResolutionCheck = {
      ...CHECK,
      verdict: null,
      size_basis: null,
      intrinsic_size_arcsec: null,
      intrinsic_size_kpc: null,
      resolution_elements: null,
      snr: null,
    };
    const markup = markupOf(viewProps({ check: floorOnly }));
    expect(markup).toContain(NO_VERDICT_TEXT);
    expect(markup).toContain('data-resolution-verdict="none"');
    for (const label of ["Unresolved", "Marginally resolved", "Resolved"]) expect(markup).not.toContain(`>${label}<`);
    for (const science of Object.values(VERDICT_SCIENCE)) expect(markup).not.toContain(science);
    expect(markupOf(viewProps())).not.toContain(NO_VERDICT_TEXT);
  });

  it("warnings are shown and notes are folded", () => {
    const markup = markupOf(viewProps({ check: { ...CHECK, warnings: ["w one"], notes: ["n one", "n two"] } }));
    expect(markup).toContain("w one");
    expect(markup).toMatch(/<details[^>]*><summary[^>]*>notes \(2\)<\/summary>/);
    expect(markup).toContain("n one");
    expect(markup).toContain("n two");
    expect(markupOf(viewProps())).not.toContain("<details");
  });

  it("loading and errors", () => {
    const loading = markupOf(viewProps({ loading: true }));
    expect(loading).toContain("Checking…");
    expect(buttonTag(loading, "Log")).toContain('disabled=""');
    const failed = markupOf(viewProps({ check: null, error: "PSF FWHM must be a positive number of arcsec" }));
    expect(failed).toMatch(/<div[^>]*role="alert"[^>]*>PSF FWHM must be a positive number of arcsec<\/div>/);
  });
});

type Tree = ReactElement<ResolutionCardViewProps> | null;

function render(props: ResolutionCardProps): Tree {
  return hooks.render(ResolutionCardInner, props) as Tree;
}

function viewOf(tree: Tree): ResolutionCardViewProps {
  if (tree === null) throw new Error("the card rendered nothing");
  return tree.props;
}

function shownText(tree: Tree): string {
  return textOf(ResolutionCardView(viewOf(tree)));
}

function buttonIn(tree: Tree, text: string) {
  return elementsOf(ResolutionCardView(viewOf(tree))).find((node) => node.type === "button" && textOf(node) === text);
}

async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const PHOTOMETRY: ResolutionCardProps = { filePath: "a.fits", fwhmArcsec: 1.2, snr: 50, context: "photometry" };

describe("ResolutionCardInner", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    hooks.mount();
    resolutionSettingsStore.reset();
    imagePsfStore.forget("a.fits");
    mocks.resolutionCheck.mockReset();
    mocks.imagePsfFwhm.mockReset().mockReturnValue(new Promise<ImagePsfFwhm>(() => undefined));
    mocks.getWcsInfo.mockReset().mockResolvedValue({ pixel_scale_arcsec: 0.2 });
    mocks.getHeader.mockReset().mockResolvedValue({});
    mocks.append.mockReset();
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
  });

  it("renders nothing without a file or without a size", () => {
    expect(render({ filePath: null, fwhmArcsec: 1.2, context: "photometry" })).toBeNull();
    hooks.mount();
    expect(render({ filePath: "a.fits", context: "photometry" })).toBeNull();
    hooks.mount();
    expect(render({ filePath: "a.fits", fwhmArcsec: Number.NaN, halfLightRadiusArcsec: 0, context: "profile" })).toBeNull();
    hooks.mount();
    const manual = render({ filePath: "a.fits", context: "manual" });
    expect(manual).not.toBeNull();
    expect(viewOf(manual).context).toBe("manual");
  });

  it("asks the command once per request, after the debounce, with the request of the settings", async () => {
    resolutionSettingsStore.update({ psfSource: "typed", psfFwhmArcsec: 1.14, redshift: 1 });
    const settings = resolutionSettingsStore.getSnapshot();
    mocks.resolutionCheck.mockResolvedValue(CHECK);
    render(PHOTOMETRY);
    await flush();
    render(PHOTOMETRY);
    expect(viewOf(render(PHOTOMETRY)).loading).toBe(true);
    vi.advanceTimersByTime(149);
    expect(mocks.resolutionCheck).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(mocks.resolutionCheck).toHaveBeenCalledTimes(1);
    expect(mocks.resolutionCheck).toHaveBeenCalledWith(
      resolutionRequest(settings, psfChoice(settings, null), 0.2, { fwhmArcsec: 1.2, halfLightRadiusArcsec: undefined, snr: 50 }),
    );
    await flush();
    const tree = render(PHOTOMETRY);
    expect(viewOf(tree).loading).toBe(false);
    expect(viewOf(tree).check).toBe(CHECK);
    expect(renderToStaticMarkup(tree as ReactElement)).toContain(">Marginally resolved<");
    vi.advanceTimersByTime(1000);
    expect(mocks.resolutionCheck).toHaveBeenCalledTimes(1);
  });

  it("a newer request wins", async () => {
    resolutionSettingsStore.update({ psfSource: "typed", psfFwhmArcsec: 1.14, redshift: 1 });
    const first = deferred<ResolutionCheck>();
    const second = deferred<ResolutionCheck>();
    mocks.resolutionCheck.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    render(PHOTOMETRY);
    await flush();
    render(PHOTOMETRY);
    vi.advanceTimersByTime(150);
    expect(mocks.resolutionCheck).toHaveBeenCalledTimes(1);
    const larger = { ...PHOTOMETRY, fwhmArcsec: 5 };
    render(larger);
    vi.advanceTimersByTime(150);
    expect(mocks.resolutionCheck).toHaveBeenCalledTimes(2);
    second.resolve(CHECK);
    await flush();
    first.resolve({ ...CHECK, verdict: "resolved" });
    await flush();
    const tree = render(larger);
    expect(viewOf(tree).check).toBe(CHECK);
    expect(shownText(tree)).toContain("Marginally resolved");
    expect(shownText(tree)).not.toContain(VERDICT_SCIENCE.resolved);
  });

  it("no PSF and no redshift means no command", async () => {
    render(PHOTOMETRY);
    await flush();
    const tree = render(PHOTOMETRY);
    vi.advanceTimersByTime(1000);
    expect(mocks.resolutionCheck).not.toHaveBeenCalled();
    expect(viewOf(tree).check).toBeNull();
    expect(viewOf(tree).loading).toBe(false);
    expect(shownText(tree)).toContain(MISSING_IMAGE_PSF);
    const button = buttonIn(tree, "Measure PSF from stars");
    expect(button).toBeDefined();
    (button?.props.onClick as () => void)();
    expect(mocks.imagePsfFwhm).toHaveBeenCalledTimes(1);
    expect(mocks.imagePsfFwhm.mock.calls[0][0]).toBe("a.fits");
    const measuring = render(PHOTOMETRY);
    expect(viewOf(measuring).measuring).toBe(true);
    expect(buttonIn(measuring, "Measuring the PSF…")?.props.disabled).toBe(true);
  });

  it("Log appends the entry of S2", async () => {
    resolutionSettingsStore.update({ psfSource: "typed", psfFwhmArcsec: 1.14, redshift: 1 });
    mocks.resolutionCheck.mockResolvedValue(CHECK);
    render(PHOTOMETRY);
    await flush();
    render(PHOTOMETRY);
    vi.advanceTimersByTime(150);
    await flush();
    const tree = render(PHOTOMETRY);
    (buttonIn(tree, "Log")?.props.onClick as () => void)();
    expect(mocks.append).toHaveBeenCalledTimes(1);
    expect(mocks.append).toHaveBeenCalledWith(resolutionCheckEntry({ file: "a.fits", image: "original" }, CHECK, "photometry", "photometry source"));

    const labelled = render({ ...PHOTOMETRY, sourceLabel: "star 3" });
    viewOf(labelled).onLog();
    expect(mocks.append).toHaveBeenCalledTimes(2);
    expect(mocks.append).toHaveBeenLastCalledWith(resolutionCheckEntry({ file: "a.fits", image: "original" }, CHECK, "photometry", "star 3"));
  });

  it("a header card is the PSF before any star measurement", async () => {
    resolutionSettingsStore.update({ psfSource: "header", redshift: null });
    const settings = resolutionSettingsStore.getSnapshot();
    mocks.getHeader.mockResolvedValue({ SEEING: "0.95" });
    mocks.resolutionCheck.mockResolvedValue(CHECK);
    render(PHOTOMETRY);
    await flush();
    render(PHOTOMETRY);
    vi.advanceTimersByTime(150);
    const expected = resolutionRequest(settings, psfChoice(settings, null, [{ key: "SEEING", value_arcsec: 0.95 }]), 0.2, {
      fwhmArcsec: 1.2,
      halfLightRadiusArcsec: undefined,
      snr: 50,
    });
    expect(expected.psf_fwhm_arcsec).toBe(0.95);
    expect(expected.psf_reference).toBe("header card SEEING");
    expect(mocks.resolutionCheck).toHaveBeenCalledTimes(1);
    expect(mocks.resolutionCheck).toHaveBeenCalledWith(expected);
    expect(mocks.getHeader).toHaveBeenCalledTimes(1);
    expect(mocks.getHeader).toHaveBeenCalledWith("a.fits");

    hooks.mount();
    mocks.resolutionCheck.mockClear();
    mocks.getHeader.mockReset().mockResolvedValue({});
    render(PHOTOMETRY);
    await flush();
    const noCard = render(PHOTOMETRY);
    vi.advanceTimersByTime(1000);
    expect(shownText(noCard)).toContain(MISSING_HEADER_CARD);
    expect(buttonIn(noCard, "Measure PSF from stars")).toBeUndefined();
    expect(mocks.resolutionCheck).not.toHaveBeenCalled();

    hooks.mount();
    mocks.getHeader.mockReset().mockResolvedValue({});
    resolutionSettingsStore.update({ psfSource: "image" });
    render(PHOTOMETRY);
    await flush();
    render(PHOTOMETRY);
    expect(mocks.getHeader).not.toHaveBeenCalled();
  });
});
