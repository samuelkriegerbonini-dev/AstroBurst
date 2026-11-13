import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement } from "react";
import type { ReactElement, ReactNode } from "react";
import { Check } from "lucide-react";
import { renderToStaticMarkup } from "react-dom/server";
import type { GainModel, PhotCal, PhotometryMeasurement, StarPhotometry } from "../../../services/analysis";


const gainModelMock = vi.hoisted(() => vi.fn<(path: string) => Promise<GainModel>>());
const measureMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
const batchMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
const pixelClick = vi.hoisted(() => ({ current: null as { seq: number; x: number; y: number } | null }));
const clipboardWrite = vi.hoisted(() => vi.fn<(text: string) => Promise<void>>());

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/analysis", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/analysis")>()),
  photometryGainModel: gainModelMock,
  measurePhotometry: measureMock,
  measurePhotometryBatch: batchMock,
}));
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  getWcsInfo: () => new Promise(() => {}),
}));
vi.mock("../../../context/ToolHostContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../context/ToolHostContext")>()),
  useToolHost: () => ({ active: true, gpuDisplay: true }),
}));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => ({ excludeDq: false }) }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "502nmos.fits", image: "original" }) }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({ useMeasurementSource: () => null }));
vi.mock("../../../hooks/useMousePixelStore", () => ({ usePixelClick: () => pixelClick.current }));
const statusDoc = vi.hoisted(() => ({ current: { loading: false, status: null, error: null } as Record<string, unknown> }));
const keyedStatus = vi.hoisted(() => ({ doc: null as Record<string, unknown> | null, calls: [] as unknown[][] }));
vi.mock("../../../hooks/usePhotometryStatus", () => ({
  usePhotometryStatus: (...args: unknown[]) => {
    keyedStatus.calls.push(args);
    return args[3] != null && keyedStatus.doc !== null ? keyedStatus.doc : statusDoc.current;
  },
}));
const depthHolder = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("../../../hooks/usePhotometryLimit", () => ({ useImageDepth: () => depthHolder.current }));
vi.mock("../ResolutionCard", () => ({ default: () => null }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import PhotometryPanel from "../PhotometryPanel";
import PhotometryTablePanel from "../PhotometryTablePanel";
import { abCellText, fluxCellText } from "../../../utils/photometryTable";
import { formatFluxDensityJy, formatWithUncertainty } from "../../../utils/formatUncertainty";
import { measurementLog } from "../../../utils/measurementLog";
import { depthNoiseText } from "../../../utils/photometryPanel";
import { noiseFactorOffer } from "../../../utils/forcedPhotometryRequest";
import type { ImageDepth } from "../../../shared/types/depth";
import ResolutionCard from "../ResolutionCard";
import { ErrorAlert } from "../../ui";
import { adHocErrorBoxLines } from "../../../test/panelRules";
import photometryPanelSource from "../PhotometryPanel.tsx?raw";

const UNDERSAMPLED_NOTE =
  "undersampled: below 2 px the width depends on where the star falls on the pixel grid (a few percent at 1.5 px, up to 10 percent at 1 px)";

const GAIN_TITLE_TEXT = "Gain in e-/ADU; prefilled from the header when known";

vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {}, removeItem: () => {} });
vi.stubGlobal("navigator", { clipboard: { writeText: clipboardWrite } });

afterEach(() => {
  statusDoc.current = { loading: false, status: null, error: null };
  keyedStatus.doc = null;
  keyedStatus.calls = [];
});

const AB_USER: PhotCal = {
  area_applied: false,
  bunit: "adu",
  convention: { exptime: null, exptime_applied: false, keyword: "user", kind: "generic_zero_point", per_second: false, system: "ab", zp: 25.0 },
  jy_per_native_unit: 3.63078054770101e-7,
  mag_system: "ab",
  notes: ["user zero point: mag = 25.0 - 2.5 log10(sum)"],
  origin: "user zero point 25.0 AB",
  warnings: ["calibrated with a user zero point"],
  label: "user zero point (AB)",
};

const VEGA_USER: PhotCal = {
  area_applied: false,
  bunit: "adu",
  convention: { exptime: null, exptime_applied: false, keyword: "user", kind: "generic_zero_point", per_second: false, system: "vega", zp: 21.432 },
  jy_per_native_unit: null,
  mag_system: "vega",
  notes: ["user zero point: mag = 21.432 - 2.5 log10(sum)"],
  origin: "user zero point 21.432 Vega",
  warnings: [
    "calibrated with a user zero point",
    "the user zero point is in the Vega system: magnitudes are in the Vega system and no AB magnitude or flux in Jy is given",
  ],
  label: "user zero point (Vega)",
};

function gainInput(markup: string): string {
  const input = markup.match(new RegExp(`<input[^>]*title="${GAIN_TITLE_TEXT}"[^>]*>`));
  expect(input, "gain input with the header-prefill title").not.toBeNull();
  return input![0];
}

describe.each([
  ["PhotometryPanel", () => renderToStaticMarkup(createElement(PhotometryPanel, { filePath: "C:/data/502nmos.fits" }))],
  [
    "PhotometryTablePanel",
    () => renderToStaticMarkup(createElement(PhotometryTablePanel, { filePath: "C:/data/502nmos.fits", overlayKey: null, stars: [] })),
  ],
])("%s gain field", (_name, render) => {
  it("carries the header-prefill title on a decimal text box", () => {
    const input = gainInput(render());
    expect(input).toContain('type="text"');
    expect(input).toContain('inputMode="decimal"');
  });

  it("renders no number input, so a typed comma is never read as a thousands digit", () => {
    const markup = render();
    expect(markup).not.toContain('type="number"');
    expect(markup.match(/<input[^>]*inputMode="decimal"/g)?.length).toBe(4);
  });

  it("names no gain source before the header model is read", () => {
    expect(gainInput(render())).toContain('data-gain-source=""');
  });

  it("renders the gain caption element", () => {
    expect(render()).toMatch(/<div[^>]*\sdata-gain-caption="true"[^>]*>/);
  });
});

type Panel = (props: Record<string, unknown>) => ReactNode;
type GainField = { value: string; onChange: (text: string) => void };

function findGainField(node: ReactNode): GainField | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findGainField(child);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Record<string, unknown>>(node)) return null;
  if (node.props.title === GAIN_TITLE_TEXT) return node.props as unknown as GainField;
  return findGainField(node.props.children as ReactNode);
}

function renderWithHooks(panel: Panel, props: Record<string, unknown>): GainField {
  hooks.run.cursor = 0;
  hooks.run.effects = [];
  const tree = panel(props);
  for (const effect of hooks.run.effects) effect();
  const field = findGainField(tree);
  expect(field, "gain input in the rendered tree").not.toBeNull();
  return field!;
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const pending = () => new Promise<GainModel>(() => {});

function gainModel(overrides: Partial<GainModel>): GainModel {
  return {
    gain_e_per_adu: null,
    source: null,
    gain_card: null,
    unit_class: "counts",
    ncombine: null,
    combine_method: null,
    drizzle_scale: null,
    combine_scaled: false,
    effective_gain: null,
    fallback_gain: null,
    poisson_route: "unavailable",
    note: null,
    ...overrides,
  };
}

const ATODGAIN_7 = gainModel({ gain_e_per_adu: 7, source: "ATODGAIN", effective_gain: 7, poisson_route: "header_gain" });
const EGAIN_4 = gainModel({ gain_e_per_adu: 4, source: "EGAIN", effective_gain: 4, poisson_route: "header_gain" });
const ERR_PLANE = gainModel({ unit_class: "calibrated", poisson_route: "err_plane", note: "Poisson noise from the ERR plane" });

const FIRST = "C:/data/502nmos.fits";
const SECOND = "C:/data/f200w_i2d.fits";

describe.each([
  ["PhotometryPanel", PhotometryPanel.type as unknown as Panel, {}],
  ["PhotometryTablePanel", PhotometryTablePanel.type as unknown as Panel, { overlayKey: null, stars: [] }],
])("%s gain field across a file switch", (_name, panel, props) => {
  const show = (filePath: string) => renderWithHooks(panel, { ...props, filePath });

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset();
  });

  afterEach(() => {
    hooks.run.active = false;
  });

  it("clears the previous file's prefilled gain while the next file's model is loading", async () => {
    gainModelMock.mockResolvedValueOnce(ATODGAIN_7).mockReturnValueOnce(pending());
    show(FIRST);
    await flush();
    expect(show(FIRST).value).toBe("7");
    show(SECOND);
    expect(show(SECOND).value).toBe("");
  });

  it("keeps a typed gain while the next file's model is loading", async () => {
    gainModelMock.mockResolvedValueOnce(ATODGAIN_7).mockReturnValueOnce(pending());
    show(FIRST);
    await flush();
    show(FIRST).onChange("3.2");
    expect(show(FIRST).value).toBe("3.2");
    show(SECOND);
    expect(show(SECOND).value).toBe("3.2");
  });

  it("prefills the next file's header gain once its model arrives", async () => {
    gainModelMock.mockResolvedValueOnce(ATODGAIN_7).mockResolvedValueOnce(EGAIN_4);
    show(FIRST);
    await flush();
    show(SECOND);
    await flush();
    expect(show(SECOND).value).toBe("4");
  });

  it("leaves the field empty when the next model resolves before the panel renders again", async () => {
    gainModelMock.mockResolvedValueOnce(ATODGAIN_7).mockResolvedValueOnce(ERR_PLANE);
    show(FIRST);
    await flush();
    expect(show(FIRST).value).toBe("7");
    show(SECOND);
    await flush();
    expect(show(SECOND).value).toBe("");
  });
});

type Props = Record<string, unknown>;

function findElement(node: ReactNode, match: (props: Props) => boolean): Props | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findElement(child, match);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Props>(node)) return null;
  if (match(node.props)) return node.props;
  return findElement(node.props.children as ReactNode, match);
}

function renderTree(panel: Panel, props: Props): ReactNode {
  hooks.run.cursor = 0;
  hooks.run.effects = [];
  const tree = panel(props);
  for (const effect of hooks.run.effects) effect();
  return tree;
}

const APERTURE_TITLE_PREFIX = "Aperture radius in pixels";
const COMMA_ERROR = "r_ap px: Use a dot (.) as the decimal separator";
const byTitlePrefix = (prefix: string) => (props: Props) => typeof props.title === "string" && props.title.startsWith(prefix);
const fieldErrorText = (tree: ReactNode) => findElement(tree, (props) => props["data-field-error"] === true)?.children ?? null;

describe("PhotometryPanel with a decimal comma in the aperture", () => {
  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    measureMock.mockReset().mockReturnValue(new Promise(() => {}));
    pixelClick.current = null;
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
  });

  it("names the field instead of measuring 2,5 as 25, then measures 2.5 once it is typed with a dot", () => {
    const show = () => renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });
    let tree = show();
    (findElement(tree, (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
    (findElement(show(), byTitlePrefix(APERTURE_TITLE_PREFIX))!.onChange as (text: string) => void)("2,5");
    pixelClick.current = { seq: 1, x: 10, y: 12 };
    tree = show();
    expect(fieldErrorText(tree)).toBe(COMMA_ERROR);
    expect(measureMock).not.toHaveBeenCalled();

    (findElement(tree, byTitlePrefix(APERTURE_TITLE_PREFIX))!.onChange as (text: string) => void)("2.5");
    pixelClick.current = { seq: 2, x: 10, y: 12 };
    tree = show();
    expect(fieldErrorText(tree)).toBeNull();
    expect(measureMock).toHaveBeenCalledTimes(1);
    expect(measureMock.mock.calls[0][3]).toMatchObject({ apertureRadius: 2.5 });
  });
});

function measurementAt(sky: { ra: number; dec: number }): PhotometryMeasurement {
  return {
    photometry: {
      x: 10,
      y: 12,
      peak: 500,
      net_flux: 1234.5,
      flux_err: 12.3,
      mag_inst: -7.73,
      snr: 100.4,
      fwhm: 3.2,
      aperture_radius: 4.8,
      aperture_pixels: 72,
      aperture_area: 72.4,
      bg_mean: 10,
      bg_sigma: 1.5,
      bg_pixels: 300,
      saturated: false,
      saturation_source: "none",
      n_masked: 0,
      n_nonfinite: 0,
      n_saturated: 0,
      err_used: false,
      aperture_correction: null,
      ee_in_aperture: null,
      flux_total: null,
      plateau_radius: null,
      flux_jy: null,
      flux_err_jy: null,
      mag_ab: null,
      mag_ab_err: null,
      mag_ab_total: null,
      st_mag: null,
      sky_inner: 9.6,
      sky_outer: 14.4,
      growth_curve: [],
      ee50_radius: null,
      ee80_radius: null,
      warnings: [],
    },
    sky,
    gaia: null,
    photcal: null,
    warnings: [],
    masked: false,
    elapsed_ms: 3,
  };
}

const COPY_SKY_LABEL = "Copy sky position";
const CLIPBOARD_DENIED = "Document is not focused.";
const SKY_LINE = "83.822080 -5.391110 deg ICRS | 05h35m17.30s -05°23'28.0\"";

const skyCopyButton = (tree: ReactNode) => findElement(tree, (props) => props["aria-label"] === COPY_SKY_LABEL);
const copyStatusText = (tree: ReactNode) => findElement(tree, (props) => props["data-copy-status"] === true)?.children ?? null;
const showsCopied = (tree: ReactNode) => (skyCopyButton(tree)!.children as ReactElement).type === Check;

describe("PhotometryPanel sky position copy", () => {
  const show = () => renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });
  const measureStar = async (settle: () => Promise<unknown>) => {
    (findElement(show(), (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
    pixelClick.current = { seq: 1, x: 10, y: 12 };
    show();
    await settle();
  };

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    measureMock.mockReset().mockResolvedValue(measurementAt({ ra: 83.82208, dec: -5.39111 }));
    clipboardWrite.mockReset().mockResolvedValue(undefined);
    pixelClick.current = null;
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
    vi.useRealTimers();
  });

  it("copies the measured star's sky position in decimal degrees and sexagesimal", async () => {
    await measureStar(flush);
    const copy = skyCopyButton(show());
    expect(copy, "sky position copy button").not.toBeNull();
    expect(copy!.type).toBe("button");
    (copy!.onClick as () => void)();
    await vi.waitFor(() => expect(clipboardWrite).toHaveBeenCalledTimes(1));
    expect(clipboardWrite.mock.calls[0][0]).toBe(SKY_LINE);
  });

  it("offers no sky copy before a measurement", () => {
    const tree = renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });
    expect(skyCopyButton(tree)).toBeNull();
  });

  it("names a refused clipboard write instead of leaving the previous clipboard text unannounced", async () => {
    clipboardWrite.mockReset().mockRejectedValue(new DOMException(CLIPBOARD_DENIED, "NotAllowedError"));
    await measureStar(flush);
    (skyCopyButton(show())!.onClick as () => void)();
    await flush();
    const tree = show();
    expect(findElement(tree, (props) => props.message === `Clipboard copy failed: ${CLIPBOARD_DENIED}`)).not.toBeNull();
    expect(showsCopied(tree)).toBe(false);
    expect(copyStatusText(tree)).toBe("");
  });

  it("clears the clipboard error once a copy succeeds", async () => {
    clipboardWrite.mockReset().mockRejectedValueOnce(new DOMException(CLIPBOARD_DENIED, "NotAllowedError")).mockResolvedValue(undefined);
    await measureStar(flush);
    const failure = `Clipboard copy failed: ${CLIPBOARD_DENIED}`;
    (skyCopyButton(show())!.onClick as () => void)();
    await flush();
    expect(findElement(show(), (props) => props.message === failure)).not.toBeNull();
    (skyCopyButton(show())!.onClick as () => void)();
    await flush();
    expect(findElement(show(), (props) => props.message === failure)).toBeNull();
  });

  it("announces the copy to assistive technology", async () => {
    await measureStar(flush);
    expect(copyStatusText(show())).toBe("");
    (skyCopyButton(show())!.onClick as () => void)();
    await flush();
    expect(copyStatusText(show())).toBe("Copied");
  });

  it("keeps the copied mark for the full time after a second click", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const settle = () => vi.advanceTimersByTimeAsync(0);
    await measureStar(settle);
    (skyCopyButton(show())!.onClick as () => void)();
    await settle();
    await vi.advanceTimersByTimeAsync(1000);
    (skyCopyButton(show())!.onClick as () => void)();
    await settle();
    await vi.advanceTimersByTimeAsync(1000);
    expect(showsCopied(show())).toBe(true);
    await vi.advanceTimersByTimeAsync(600);
    expect(showsCopied(show())).toBe(false);
  });
});

function elementsWhere(node: ReactNode, match: (props: Props) => boolean): Props[] {
  if (Array.isArray(node)) return node.flatMap((child) => elementsWhere(child, match));
  if (!isValidElement<Props>(node)) return [];
  return [...(match(node.props) ? [node.props] : []), ...elementsWhere(node.props.children as ReactNode, match)];
}

function measuredStar(over: Record<string, unknown>): Record<string, unknown> {
  return {
    x: 10.04, y: 12.06, peak: 900, net_flux: 1234.5678, flux_err: 12, mag_inst: -7.73, snr: 102.9, fwhm: 3.214,
    aperture_radius: 5, aperture_pixels: 78, aperture_area: 78.5, bg_mean: 10.25, bg_sigma: 2, bg_pixels: 200,
    saturated: false, saturation_source: "none", n_masked: 0, n_nonfinite: 0, n_saturated: 0, err_used: true,
    aperture_correction: 1.0421, ee_in_aperture: 0.9596, flux_total: 1286.6, plateau_radius: 9, flux_jy: null, flux_err_jy: null,
    mag_ab: 24.5123, mag_ab_err: 0.0314, mag_ab_total: null, st_mag: null, sky_inner: 10, sky_outer: 15, ee50_radius: 2.345,
    warnings: [],
    ...over,
  };
}

describe("PhotometryTablePanel result table", () => {
  const STARS = [{ x: 10, y: 12, flux: 100, fwhm: 3, snr: 50 }, { x: 40, y: 42, flux: 60, fwhm: 3, snr: 30 }];

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    batchMock.mockReset().mockResolvedValue({
      rows: [
        { index: 0, photometry: measuredStar({}), sky: null, error: null },
        { index: 1, photometry: measuredStar({ net_flux: 2.5e7, mag_ab: null, mag_ab_err: null }), sky: null, error: null },
      ],
      photcal: AB_USER,
      warnings: [],
      masked: false,
      n_measured: 2,
      n_failed: 0,
      elapsed_ms: 8,
    });
  });

  afterEach(() => {
    hooks.run.active = false;
  });

  async function measuredTable(): Promise<ReactNode> {
    const show = () => renderTree(PhotometryTablePanel.type as unknown as Panel, { filePath: FIRST, overlayKey: null, stars: STARS });
    (findElement(show(), (props) => props.label === "Measure 2")!.onClick as () => void)();
    await flush();
    return show();
  }

  const cells = (tree: ReactNode, column: string) => elementsWhere(tree, (props) => props["data-column"] === column);

  it("tags every body cell with its column key, one cell per measured row", async () => {
    const tree = await measuredTable();
    for (const column of ["label", "x", "y", "net_flux", "snr", "mag_ab", "mag_inst", "fwhm", "ee50_radius", "aperture_correction"]) {
      expect(cells(tree, column), column).toHaveLength(2);
    }
  });

  it("writes the flux and AB cells with the shared cell formatters", async () => {
    const tree = await measuredTable();
    expect(cells(tree, "net_flux").map((cell) => cell.children)).toEqual([fluxCellText(1234.5678), fluxCellText(2.5e7)]);
    expect(cells(tree, "mag_ab").map((cell) => cell.children)).toEqual(["24.512 ± 0.031", "--"]);
    expect(cells(tree, "aperture_correction")[0].children).toBe("1.042");
  });

  it("keeps every body cell on one line, so an AB cell such as 24.512 ± 0.031 never wraps in the docked panel", async () => {
    const tree = await measuredTable();
    for (const cell of cells(tree, "label")) expect(String(cell.className).split(" ")).toContain("truncate");
    for (const cell of [...cells(tree, "mag_ab"), ...cells(tree, "net_flux"), ...cells(tree, "x")]) {
      expect(String(cell.className).split(" ")).toContain("whitespace-nowrap");
    }
  });
});

describe("PhotometryTablePanel with a decimal comma in the aperture", () => {
  const STARS = [{ x: 10, y: 12, flux: 100, fwhm: 3, snr: 50 }];

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    batchMock.mockReset().mockReturnValue(new Promise(() => {}));
  });

  afterEach(() => {
    hooks.run.active = false;
  });

  it("disables Measure and names the field until the radius is typed with a dot", () => {
    const show = () => renderTree(PhotometryTablePanel.type as unknown as Panel, { filePath: FIRST, overlayKey: null, stars: STARS });
    const measureButton = (tree: ReactNode) => findElement(tree, (props) => props.label === "Measure 1")!;
    (findElement(show(), byTitlePrefix(APERTURE_TITLE_PREFIX))!.onChange as (text: string) => void)("2,5");
    let tree = show();
    expect(measureButton(tree).disabled).toBe(true);
    expect(fieldErrorText(tree)).toBe(COMMA_ERROR);
    (measureButton(tree).onClick as () => void)();
    expect(batchMock).not.toHaveBeenCalled();

    (findElement(tree, byTitlePrefix(APERTURE_TITLE_PREFIX))!.onChange as (text: string) => void)("2.5");
    tree = show();
    expect(measureButton(tree).disabled).toBe(false);
    (measureButton(tree).onClick as () => void)();
    expect(batchMock).toHaveBeenCalledTimes(1);
    expect(batchMock.mock.calls[0][2]).toMatchObject({ apertureRadius: 2.5 });
  });
});

const JWST_I2D: PhotCal = {
  convention: { kind: "jwst_mjy_sr", pixar_sr: 2.29e-14, pixar_a2: 0.000974, derived_pixar: false },
  bunit: "MJy/sr",
  notes: [],
  warnings: [],
  label: "JWST MJy/sr, PIXAR_SR from header",
};
const HST_RATE: PhotCal = {
  convention: { kind: "hst_counts", photflam: 1.5e-19, photplam: 6563, photzpt: -21.1, per_second: true, exptime: null },
  bunit: "ELECTRONS/S",
  notes: [],
  warnings: [],
  label: "HST PHOTFLAM/PHOTPLAM (count rate), STmag to ABmag",
};
const FWHM_FALLBACK = "FWHM not measured: the automatic aperture radius of 6.0 px is a fallback, not 1.5 x FWHM";
const NO_DATA_3 = "3 aperture pixels without data (non-finite): the flux is a lower limit";
const PROCESSED = "photometry on processed data (stack)";

function measurementWith(star: Partial<StarPhotometry>, rest: Partial<PhotometryMeasurement> = {}): PhotometryMeasurement {
  const base = measurementAt({ ra: 83.82208, dec: -5.39111 });
  return { ...base, ...rest, photometry: { ...base.photometry, ...star } };
}

const texts = (tree: ReactNode) => elementsOf(tree).map((element) => textOf(element));

function cardValue(tree: ReactNode, label: string): string | null {
  const card = elementsOf(tree).find((element) => {
    const children = (element.props as Props).children;
    return Array.isArray(children) && isValidElement(children[0]) && textOf(children[0]) === label;
  });
  return card ? textOf(((card.props as Props).children as ReactNode[])[1]) : null;
}

describe("PhotometryPanel result card with the fields wave 0 added", () => {
  const show = () => renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });
  const measured = async (result: PhotometryMeasurement): Promise<ReactNode> => {
    measureMock.mockReset().mockResolvedValue(result);
    (findElement(show(), (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
    pixelClick.current = { seq: 1, x: 10, y: 12 };
    show();
    await flush();
    return show();
  };
  const measuredAgain = (result: PhotometryMeasurement): Promise<ReactNode> => {
    hooks.run.cells = [];
    pixelClick.current = null;
    return measured(result);
  };

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    pixelClick.current = null;
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
  });

  it("prints -- for an unmeasured FWHM instead of failing to render the card", async () => {
    const tree = await measured(measurementWith({ fwhm: null, warnings: [FWHM_FALLBACK] }));
    expect(cardValue(tree, "FWHM")).toBe("--");
  });

  it("keeps a measured FWHM in pixels", async () => {
    expect(cardValue(await measured(measurementWith({ fwhm: 3.214 })), "FWHM")).toBe("3.21 px");
  });

  it("labels the net flux, the total and the growth axis of an MJy/sr image with the unit of a sum over pixels", async () => {
    const tree = await measured(
      measurementWith(
        {
          aperture_correction: 1.0421,
          ee_in_aperture: 0.9596,
          flux_total: 1286.6,
          plateau_radius: 9,
          growth_curve: [
            { r: 1, flux: 400 },
            { r: 5, flux: 1234.5 },
          ],
        },
        { photcal: JWST_I2D },
      ),
    );
    const shown = texts(tree);
    expect(shown).not.toContain("Net flux MJy/sr");
    expect(shown).toContain("Net flux MJy/sr x pix");
    expect(shown).toContain("1.0421 → total 1.287e+3 MJy/sr x pix");
    expect(findElement(tree, (props) => typeof props.yLabel === "string")?.yLabel).toBe("net flux MJy/sr x pix");
  });

  it("keeps BUNIT as the unit of a summed count rate", async () => {
    expect(texts(await measured(measurementWith({}, { photcal: HST_RATE })))).toContain("Net flux ELECTRONS/S");
  });

  it("shows the enclosed fraction in the aperture beside the multiplicative correction", async () => {
    const tree = await measured(measurementWith({ aperture_correction: 1.0421, ee_in_aperture: 0.9596, flux_total: 1286.6, plateau_radius: 9 }));
    expect(texts(tree)).toContain("EE in aperture 0.9596");
  });

  it("badges the aperture pixels without data next to the masked count", async () => {
    const shown = texts(await measured(measurementWith({ n_masked: 2, n_nonfinite: 3, warnings: [NO_DATA_3] })));
    expect(shown).toContain("2 masked");
    expect(shown).toContain("3 no data");
  });

  it("shows no data badge when every aperture pixel holds data", async () => {
    expect(texts(await measured(measurementWith({}))).some((text) => text.endsWith(" no data"))).toBe(false);
  });

  it("shows the per-star warnings with the image warnings", async () => {
    const tree = await measured(measurementWith({ n_nonfinite: 3, warnings: [NO_DATA_3] }, { warnings: [PROCESSED] }));
    for (const warning of [PROCESSED, NO_DATA_3]) {
      expect(elementsWhere(tree, (props) => props.children === warning), warning).toHaveLength(1);
    }
  });

  it("shows a warning once when the star and the image both carry it", async () => {
    const tree = await measured(measurementWith({ n_nonfinite: 3, warnings: [NO_DATA_3] }, { warnings: [PROCESSED, NO_DATA_3] }));
    expect(elementsWhere(tree, (props) => props.children === NO_DATA_3)).toHaveLength(1);
  });

  it("shows the fallback-aperture warning of a star whose FWHM was not measured", async () => {
    const tree = await measured(measurementWith({ fwhm: null, aperture_radius: 6, warnings: [FWHM_FALLBACK] }));
    expect(elementsWhere(tree, (props) => props.children === FWHM_FALLBACK)).toHaveLength(1);
  });

  it("states the 6 px radius a blank aperture takes when the FWHM is not measured", () => {
    const tree = show();
    expect(elementsWhere(tree, (props) => props.children === "blank: r_ap = 1.5 × FWHM (6 px if the FWHM is not measured), sky 2–3 × r_ap")).toHaveLength(1);
    expect(findElement(tree, byTitlePrefix(APERTURE_TITLE_PREFIX))?.title).toBe(
      "Aperture radius in pixels; blank uses 1.5 × FWHM of the clicked star, or 6 px when its FWHM is not measured",
    );
  });

  it("prints the net flux rounded to its error", async () => {
    const value = cardValue(await measured(measurementWith({ net_flux: 1234.5678, flux_err: 12.3 })), "Net flux");
    expect(value).toBe(formatWithUncertainty(1234.5678, 12.3));
    expect(value).not.toBe("1.235e+3 ± 1.23e+1");
    expect(value).toBe("1235 ± 12");
  });

  it("prints the flux density with one prefix for value and error", async () => {
    const tree = await measured(measurementWith({ flux_jy: 1.23e-5, flux_err_jy: 9.9e-7 }));
    expect(cardValue(tree, "Flux density")).toBe(formatFluxDensityJy(1.23e-5, 9.9e-7));
    const shown = texts(tree);
    expect(shown.some((text) => text.includes("Flux (Jy)"))).toBe(false);
    expect(shown.some((text) => text.includes("nJy"))).toBe(false);
  });

  it("prints the AB magnitude with the table formatter", async () => {
    expect(cardValue(await measured(measurementWith({ mag_ab: 24.5123, mag_ab_err: 0.0314 }, { photcal: AB_USER })), "AB mag")).toBe("24.512 ± 0.031");
  });

  it("names the unit of the net flux of an uncalibrated image from the result", async () => {
    expect(texts(await measured(measurementWith({}, { photcal: null, summed_unit: "adu", bunit: "adu" })))).toContain("Net flux adu");
    expect(texts(await measuredAgain(measurementWith({}, { photcal: null })))).toContain("Net flux");
  });

  it("prints the FWHM in pixels and arcsec", async () => {
    statusDoc.current = { loading: false, status: { pixel_scale_arcsec: 0.1 }, error: null };
    expect(cardValue(await measured(measurementWith({ fwhm: 3.214 })), "FWHM")).toBe('3.21 px (0.321")');
    statusDoc.current = { loading: false, status: { pixel_scale_arcsec: null }, error: null };
    expect(cardValue(show(), "FWHM")).toBe("3.21 px");
    statusDoc.current = { loading: false, status: { pixel_scale_arcsec: 0.1 }, error: null };
    expect(cardValue(await measuredAgain(measurementWith({ fwhm: null })), "FWHM")).toBe("--");
  });

  it("says that a width below two pixels is undersampled", async () => {
    const notes = (tree: ReactNode) => texts(tree).filter((text) => text === UNDERSAMPLED_NOTE);
    expect(notes(await measured(measurementWith({ fwhm: 1.66 })))).toHaveLength(1);
    expect(notes(await measuredAgain(measurementWith({ fwhm: 2.0 })))).toHaveLength(0);
  });

  it("states the sky level, the pixel scatter and the error of the level separately", async () => {
    const tree = await measured(
      measurementWith({ bg_mean: 10.25, bg_sigma: 2, bg_pixels: 200, sky_inner: 10, sky_outer: 15 }, { bunit: "adu" }),
    );
    const value = cardValue(tree, "Sky level per pixel (200 px, annulus 10.0 - 15.0 px)");
    expect(value).toBe(
      `${formatWithUncertainty(10.25, null)} adu; pixel scatter ${formatWithUncertainty(2, null)}; error of the level ${formatWithUncertainty(0.1414213562373095, null)}`,
    );
    expect(value).not.toContain("±");
    expect(texts(tree).some((text) => text.includes("Background ("))).toBe(false);
  });

  it("announces a failed measurement through the shared error alert", async () => {
    const failure = "position (900, 900) lies outside the 64 x 64 image";
    measureMock.mockReset().mockRejectedValue(new Error(failure));
    (findElement(show(), (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
    pixelClick.current = { seq: 1, x: 900, y: 900 };
    show();
    await flush();
    const tree = show();
    expect(findElement(tree, (props) => props.message === failure)).not.toBeNull();
    const handWritten = elementsWhere(tree, (props) => typeof props.className === "string" && props.className.includes("text-red-400 bg-red-900/20"));
    expect(handWritten).toHaveLength(0);
  });

  it("holds no hand-made failure box", () => {
    expect(adHocErrorBoxLines(photometryPanelSource)).toEqual([]);
  });

  it("shows a failed measurement in the compact alert", async () => {
    const failure = "position (900, 900) lies outside the 64 x 64 image";
    measureMock.mockReset().mockRejectedValue(new Error(failure));
    (findElement(show(), (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
    pixelClick.current = { seq: 1, x: 900, y: 900 };
    show();
    await flush();
    const alert = elementsOf(show()).find((e) => e.type === ErrorAlert);
    expect(alert?.props.message).toBe(failure);
    expect(alert?.props.compact).toBe(true);
  });
});

describe("PhotometryPanel when the measured file is rewritten under the same path", () => {
  const CALIBRATED = "C:/out/x_calibrated.fits";
  const version = (n: number): Props => ({ filePath: CALIBRATED, measureKey: `${CALIBRATED}@${n}` });
  const show = (props: Props) => renderTree(PhotometryPanel.type as unknown as Panel, props);
  const click = async (props: Props, seq: number) => {
    pixelClick.current = { seq, x: 10 + seq, y: 12 };
    show(props);
    await flush();
  };
  const arm = (props: Props) => (findElement(show(props), (p) => p.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
  const cardShown = (tree: ReactNode) => texts(tree).includes("Net flux");
  const previousShown = (tree: ReactNode) => texts(tree).includes("Previous");

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    measureMock.mockReset().mockResolvedValue(measurementAt({ ra: 83.82208, dec: -5.39111 }));
    pixelClick.current = null;
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
  });

  it("drops the result card and the Previous list when a Stacking re-run rewrites the file and the measure key moves on", async () => {
    arm(version(3));
    await click(version(3), 1);
    await click(version(3), 2);
    let tree = show(version(3));
    expect(cardShown(tree)).toBe(true);
    expect(previousShown(tree)).toBe(true);
    show(version(4));
    tree = show(version(4));
    expect(cardShown(tree)).toBe(false);
    expect(previousShown(tree)).toBe(false);
  });

  it("starts a new Previous list on the rewritten file instead of mixing both versions", async () => {
    arm(version(3));
    await click(version(3), 1);
    show(version(4));
    await click(version(4), 2);
    const tree = show(version(4));
    expect(cardShown(tree)).toBe(true);
    expect(previousShown(tree)).toBe(false);
  });

  it("keeps the result while the measure key stays the same", async () => {
    arm(version(3));
    await click(version(3), 1);
    show(version(3));
    expect(cardShown(show(version(3)))).toBe(true);
  });
});

describe("PhotometryPanel status line", () => {
  const show = () => renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    pixelClick.current = null;
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
  });

  it("puts the status line above the measure toggle and gives it the file, the measure key and the DQ flag", async () => {
    const { default: PhotometryStatusLine } = await import("../PhotometryStatusLine");
    const order = elementsOf(show());
    const line = order.findIndex((element) => {
      const props = element.props as Props;
      return props.filePath === FIRST && props.measureKey === null && props.excludeDq === false;
    });
    const toggle = order.findIndex((element) => (element.props as Props).label === "Measure on image click");
    expect(line, "status line element").toBeGreaterThanOrEqual(0);
    expect(order[line].type).toBe(PhotometryStatusLine);
    expect((order[line].props as Props).zeroPoint).toBeNull();
    expect(line).toBeLessThan(toggle);
  });

  it("prints the EE radii in arcsec with the pixel scale of the status", async () => {
    statusDoc.current = { loading: false, status: { pixel_scale_arcsec: 0.1 }, error: null };
    measureMock.mockReset().mockResolvedValue(measurementWith({ ee50_radius: 2.345 }));
    (findElement(show(), (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
    pixelClick.current = { seq: 1, x: 10, y: 12 };
    show();
    await flush();
    expect(cardValue(show(), "EE50 r")).toBe('2.35 px (0.235")');
    statusDoc.current = { loading: false, status: { pixel_scale_arcsec: null }, error: null };
    expect(cardValue(show(), "EE50 r")).toBe("2.35 px");
  });
});

const RADIUS_TITLE_PREFIXES = ["Aperture radius in", "Inner sky radius in", "Outer sky radius in"];
const NO_ARCSEC_TITLE = "No celestial WCS: radii in arcsec are not available for this image";
const PIXEL_SCALE_PENDING = "Reading the pixel scale of this image...";
const NO_PIXEL_SCALE = "Radii in arcsec need the pixel scale of a celestial WCS, which this image does not have. Switch the unit to px.";
const scaleDoc = (pixelScale: number | null) => ({ loading: false, status: { pixel_scale_arcsec: pixelScale }, error: null });

describe("PhotometryPanel radius unit", () => {
  const show = () => renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });
  const unitSelect = (tree: ReactNode) => findElement(tree, (props) => props["aria-label"] === "Radius unit");
  const chooseUnit = (unit: string) =>
    (unitSelect(show())!.onChange as (event: { target: { value: string } }) => void)({ target: { value: unit } });
  const radiusField = (tree: ReactNode, index: number) => findElement(tree, byTitlePrefix(RADIUS_TITLE_PREFIXES[index]))!;
  const typeRadius = (index: number, text: string) => (radiusField(show(), index).onChange as (text: string) => void)(text);
  const radiusTexts = () => {
    const tree = show();
    return [0, 1, 2].map((index) => radiusField(tree, index).value);
  };
  const fieldLabels = (tree: ReactNode) => elementsWhere(tree, (props) => typeof props.htmlFor === "string").map((props) => textOf(props.children).trim());
  const echoText = (tree: ReactNode) => {
    const echo = findElement(tree, (props) => props["data-radius-echo"] === true);
    return echo ? textOf(echo.children) : null;
  };
  const radiusErrorText = (tree: ReactNode) => {
    const line = findElement(tree, (props) => props["data-radius-error"] === true);
    return line ? textOf(line.children) : null;
  };
  const arm = () => (findElement(show(), (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
  const clickStar = (seq: number) => {
    pixelClick.current = { seq, x: 10, y: 12 };
    return show();
  };
  const sentApertureRadius = () => (measureMock.mock.calls[0][3] as Props).apertureRadius;

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    measureMock.mockReset().mockReturnValue(new Promise(() => {}));
    pixelClick.current = null;
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
  });

  it("offers px and arcsec for the radii with px selected", () => {
    let tree = show();
    const select = unitSelect(tree);
    expect(select, "radius unit select").not.toBeNull();
    expect(select!.value).toBe("px");
    const options = elementsWhere(select!.children as ReactNode, (props) => typeof props.value === "string");
    expect(options.map((option) => option.value)).toEqual(["px", "arcsec"]);
    expect(options.map((option) => option.children)).toEqual(["px", "arcsec"]);
    expect(fieldLabels(tree).slice(0, 3)).toEqual(["r_ap px", "sky in px", "sky out px"]);

    statusDoc.current = scaleDoc(null);
    tree = show();
    const arcsec = elementsWhere(unitSelect(tree)!.children as ReactNode, (props) => props.value === "arcsec")[0];
    expect(arcsec.disabled).toBe(true);
    expect(arcsec.title).toBe(NO_ARCSEC_TITLE);

    statusDoc.current = scaleDoc(0.262);
    tree = show();
    expect(elementsWhere(unitSelect(tree)!.children as ReactNode, (props) => props.value === "arcsec")[0].disabled).toBe(false);
  });

  it("sends the pixel radius of an aperture typed in arcsec", () => {
    statusDoc.current = scaleDoc(0.262);
    chooseUnit("arcsec");
    typeRadius(0, "1");
    arm();
    const tree = clickStar(1);
    expect(measureMock).toHaveBeenCalledTimes(1);
    expect(sentApertureRadius()).toBeCloseTo(3.8167938931297707, 10);
    expect(echoText(tree)).toBe("r_ap 1 arcsec = 3.82 px");
    expect(fieldLabels(tree).slice(0, 3)).toEqual(["r_ap arcsec", "sky in arcsec", "sky out arcsec"]);
  });

  it("converts the typed radii when the unit changes", () => {
    statusDoc.current = scaleDoc(0.262);
    typeRadius(0, "5");
    typeRadius(1, "10");
    typeRadius(2, "15");
    chooseUnit("arcsec");
    expect(radiusTexts()).toEqual(["1.31", "2.62", "3.93"]);
    chooseUnit("px");
    expect(radiusTexts()).toEqual(["5", "10", "15"]);
  });

  it("refuses an arcsec aperture outside the pixel limits and does not measure", () => {
    statusDoc.current = scaleDoc(0.1);
    chooseUnit("arcsec");
    typeRadius(0, "7");
    arm();
    const tree = clickStar(1);
    expect(radiusErrorText(tree)).toBe(
      "Aperture radius 7 arcsec is 70.00 px at 0.1000 arcsec/px; it must be between 2 and 60 px (0.2000 to 6.000 arcsec).",
    );
    expect(measureMock).not.toHaveBeenCalled();
  });

  it("shows a typed pixel radius in arcsec", () => {
    statusDoc.current = scaleDoc(0.262);
    typeRadius(0, "5");
    expect(echoText(show())).toBe("r_ap 5 px = 1.310 arcsec");
    statusDoc.current = scaleDoc(null);
    expect(echoText(show())).toBeNull();
  });

  it("prints the measured aperture in pixels and arcsec", async () => {
    statusDoc.current = scaleDoc(0.1);
    measureMock.mockReset().mockResolvedValue(measurementWith({ aperture_radius: 4.8, aperture_pixels: 72 }));
    arm();
    clickStar(1);
    await flush();
    expect(texts(show())).toContain('r=4.80 px (0.480") (72px)');
    statusDoc.current = scaleDoc(null);
    const shown = texts(show());
    expect(shown).toContain("r=4.80 px (72px)");
    expect(shown).not.toContain("r=4.8 (72px)");
  });

  it("names the arcsec field instead of reading 2,5 as 25", () => {
    statusDoc.current = scaleDoc(0.262);
    chooseUnit("arcsec");
    (findElement(show(), byTitlePrefix("Aperture radius in arcsec"))!.onChange as (text: string) => void)("2,5");
    arm();
    const tree = clickStar(1);
    expect(fieldErrorText(tree)).toBe("r_ap arcsec: Use a dot (.) as the decimal separator");
    expect(measureMock).not.toHaveBeenCalled();
  });

  it("waits for the pixel scale instead of refusing an arcsec radius while the status is pending", () => {
    statusDoc.current = scaleDoc(0.262);
    chooseUnit("arcsec");
    typeRadius(0, "1");
    arm();
    const cases: [Record<string, unknown>, string][] = [
      [{ loading: true, status: null, error: null }, PIXEL_SCALE_PENDING],
      [{ loading: false, status: null, error: null }, PIXEL_SCALE_PENDING],
      [{ loading: false, status: null, error: "no such file" }, "Status not available: no such file"],
      [scaleDoc(null), NO_PIXEL_SCALE],
    ];
    cases.forEach(([doc, expected], index) => {
      statusDoc.current = doc;
      const tree = clickStar(index + 1);
      expect(radiusErrorText(tree)).toBe(expected);
      if (expected !== NO_PIXEL_SCALE) {
        expect(texts(tree).some((text) => text.includes("Radii in arcsec need the pixel scale"))).toBe(false);
      }
    });
    expect(measureMock).not.toHaveBeenCalled();
  });

  it("clears the radii when the unit goes back to px without a pixel scale", () => {
    const typeAllInArcsec = () => {
      statusDoc.current = scaleDoc(0.262);
      chooseUnit("arcsec");
      typeRadius(0, "1");
      typeRadius(1, "2");
      typeRadius(2, "3");
    };
    typeAllInArcsec();
    statusDoc.current = { loading: false, status: null, error: null };
    chooseUnit("px");
    expect(radiusTexts()).toEqual(["", "", ""]);
    arm();
    clickStar(1);
    expect(measureMock).toHaveBeenCalledTimes(1);
    expect(sentApertureRadius()).toBeUndefined();

    hooks.run.cells = [];
    pixelClick.current = null;
    typeAllInArcsec();
    chooseUnit("px");
    expect(radiusTexts()).toEqual(["3.817", "7.634", "11.45"]);
  });
});

const ZERO_POINT_TITLE = "Zero point in magnitudes of one pixel unit; used because the header has no calibration";
const NO_JY_TITLE = "No Jy for a zero point that is not AB: the conversion needs the AB offset of the band.";
const PER_SECOND_LABEL = "Zero point is per second";
const PER_SECOND_REFUSAL =
  "a zero point per second needs EXPTIME to turn the pixel values into a rate, and this header has none (BUNIT 'adu')";
const VEGA_MAG = 21.432 - 9.87;
const VEGA_MAG_ERR = (1.0857362047581294 * 50) / 8872;
const sourceDoc = (calibrationSource: string | null) => ({
  loading: false,
  status: { pixel_scale_arcsec: null, calibration_source: calibrationSource },
  error: null,
});

describe("PhotometryPanel calibration", () => {
  const show = () => renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });
  const arm = () => (findElement(show(), (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
  const clickStar = (seq: number) => {
    pixelClick.current = { seq, x: 10, y: 12 };
    return show();
  };
  const measured = async (result: PhotometryMeasurement): Promise<ReactNode> => {
    measureMock.mockReset().mockResolvedValue(result);
    arm();
    clickStar(1);
    await flush();
    return show();
  };
  const cardValueProps = (tree: ReactNode, label: string): Props | null => {
    const card = elementsOf(tree).find((element) => {
      const children = (element.props as Props).children;
      return Array.isArray(children) && isValidElement(children[0]) && textOf(children[0]) === label;
    });
    return card ? (((card.props as Props).children as ReactElement[])[1].props as Props) : null;
  };
  const zeroPointField = (tree: ReactNode) => findElement(tree, (props) => props.title === ZERO_POINT_TITLE);
  const systemSelect = (tree: ReactNode) => findElement(tree, (props) => props["aria-label"] === "Zero point system");
  const perSecondToggle = (tree: ReactNode) => findElement(tree, (props) => props.label === PER_SECOND_LABEL);
  const statusLine = (tree: ReactNode) => findElement(tree, (props) => props.filePath === FIRST && props.excludeDq === false && "zeroPoint" in props);
  const typeZeroPoint = (text: string) => (zeroPointField(show())!.onChange as (text: string) => void)(text);
  const chooseSystem = (system: string) =>
    (systemSelect(show())!.onChange as (event: { target: { value: string } }) => void)({ target: { value: system } });

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    measureMock.mockReset().mockReturnValue(new Promise(() => {}));
    pixelClick.current = null;
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
  });

  it("titles the magnitude of a Vega zero point Vega mag and shows no AB value", async () => {
    let tree = await measured(
      measurementWith(
        { mag_inst: -9.87, net_flux: 8872, flux_err: 50, mag_ab: null, mag_ab_err: null, flux_jy: null, flux_err_jy: null },
        { photcal: VEGA_USER, warnings: [] },
      ),
    );
    expect(cardValue(tree, "Vega mag")).toBe(abCellText(VEGA_MAG, VEGA_MAG_ERR));
    expect(cardValue(tree, "AB mag")).toBeNull();
    expect(cardValue(tree, "Flux density")).toBe("--");
    expect(cardValueProps(tree, "Flux density")!.title).toBe(NO_JY_TITLE);

    hooks.run.cells = [];
    pixelClick.current = null;
    const hstWithoutExptime: PhotCal = {
      convention: { kind: "hst_counts", photflam: 1.5e-19, photplam: 6563, photzpt: -21.1, per_second: false, exptime: null },
      bunit: "COUNTS",
      notes: [],
      warnings: [],
      label: "HST PHOTFLAM/PHOTPLAM (counts), STmag to ABmag",
      mag_system: "ab",
      jy_per_native_unit: null,
    };
    tree = await measured(measurementWith({ flux_jy: null, flux_err_jy: null }, { photcal: hstWithoutExptime }));
    expect(cardValue(tree, "Flux density")).toBe("--");
    expect(cardValueProps(tree, "Flux density")!.title).toBeUndefined();
  });

  it("says that the AB system was assumed when the header does not state it", async () => {
    const tree = await measured(measurementWith({ mag_ab: 24.5123, mag_ab_err: 0.0314 }, { photcal: { ...AB_USER, mag_system: "unknown" } }));
    expect(cardValue(tree, "AB mag (system assumed)")).toBe("24.512 ± 0.031");
    expect(cardValue(tree, "AB mag")).toBeNull();
  });

  it("prints the origin sentence of the calibration and folds its notes", async () => {
    const origin = "MAGZERO = 30.0, AB assumed";
    const note = "zero point applied to the native unit";
    let tree = await measured(measurementWith({}, { photcal: { ...AB_USER, origin, notes: [note] } }));
    expect(cardValue(tree, "Calibration")).toBe(origin);
    const summaries = elementsOf(tree).filter((element) => element.type === "summary");
    expect(summaries.map((summary) => textOf(summary))).toEqual(["calibration notes (1)"]);
    expect(texts(tree)).toContain(note);

    hooks.run.cells = [];
    pixelClick.current = null;
    tree = await measured(measurementWith({}, { photcal: { ...AB_USER, origin, notes: [] } }));
    expect(elementsOf(tree).filter((element) => element.type === "details")).toHaveLength(0);
  });

  it("keeps the old texts of a measurement without calibration", async () => {
    const tree = await measured(measurementWith({ mag_ab: 24.5123, mag_ab_err: 0.0314 }, { photcal: null }));
    expect(cardValue(tree, "Calibration")).toBe("no calibration in header");
    expect(cardValue(tree, "AB mag")).toBe("--");
  });

  it("offers the zero point fields only when the header has no calibration", () => {
    statusDoc.current = sourceDoc(null);
    let tree = show();
    expect(zeroPointField(tree), "zero point field").not.toBeNull();
    expect(systemSelect(tree), "system select").not.toBeNull();
    expect(perSecondToggle(tree), "per second toggle").not.toBeNull();
    const options = elementsWhere(systemSelect(tree)!.children as ReactNode, (props) => typeof props.value === "string");
    expect(options.map((option) => [option.value, option.children])).toEqual([
      ["ab", "AB"],
      ["vega", "Vega"],
    ]);
    expect(elementsWhere(tree, (props) => typeof props.htmlFor === "string").map((props) => textOf(props.children).trim())).toContain("zero point mag");

    for (const doc of [sourceDoc("header"), { loading: false, status: null, error: null }, { loading: true, status: null, error: null }]) {
      statusDoc.current = doc;
      tree = show();
      expect(zeroPointField(tree)).toBeNull();
      expect(systemSelect(tree)).toBeNull();
      expect(perSecondToggle(tree)).toBeNull();
    }
  });

  it("sends the typed zero point with the measurement and to the status line", () => {
    statusDoc.current = sourceDoc(null);
    typeZeroPoint("21.432");
    chooseSystem("vega");
    arm();
    const tree = clickStar(1);
    const expected = { zp: 21.432, system: "vega", per_second: false };
    expect(measureMock).toHaveBeenCalledTimes(1);
    expect((measureMock.mock.calls[0][3] as Props).zeroPoint).toEqual(expected);
    expect(statusLine(tree)!.zeroPoint).toEqual(expected);
    expect(keyedStatus.calls.length).toBeGreaterThan(0);
    for (const call of keyedStatus.calls) {
      expect(call).toHaveLength(3);
      expect(call[3]).toBeUndefined();
    }
  });

  it("sends no zero point when the header calibrates", () => {
    statusDoc.current = sourceDoc(null);
    typeZeroPoint("25");
    statusDoc.current = sourceDoc("header");
    arm();
    const tree = clickStar(1);
    expect(measureMock).toHaveBeenCalledTimes(1);
    expect((measureMock.mock.calls[0][3] as Props).zeroPoint).toBeUndefined();
    expect(statusLine(tree)!.zeroPoint).toBeNull();
  });

  it("names the zero point field in the error line and does not measure an invalid entry", () => {
    statusDoc.current = sourceDoc(null);
    typeZeroPoint("2,5");
    arm();
    const tree = clickStar(1);
    expect(fieldErrorText(tree)).toBe("zero point: Use a dot (.) as the decimal separator");
    expect(measureMock).not.toHaveBeenCalled();
  });

  it("explains in the help text that a zero point can be typed", () => {
    const shown = texts(show());
    expect(shown.some((text) => text.includes("a zero point can be typed when the header has none"))).toBe(true);
    expect(shown.some((text) => text.includes("MAGZPT-style keywords"))).toBe(false);
  });

  it("keeps the zero point row while the status of the typed value is being read", () => {
    statusDoc.current = sourceDoc(null);
    keyedStatus.doc = { loading: true, status: null, error: null };
    typeZeroPoint("2");
    const tree = show();
    expect(zeroPointField(tree)!.value).toBe("2");
    expect(systemSelect(tree)).not.toBeNull();
    expect(perSecondToggle(tree)).not.toBeNull();
    expect(statusLine(tree)!.zeroPoint).toEqual({ zp: 2, system: "ab", per_second: false });
  });

  it("keeps the row and shows the refusal of a per-second zero point in the status line", () => {
    statusDoc.current = sourceDoc(null);
    keyedStatus.doc = { loading: false, status: null, error: PER_SECOND_REFUSAL };
    typeZeroPoint("25");
    (perSecondToggle(show())!.onChange as (on: boolean) => void)(true);
    let tree = show();
    expect(zeroPointField(tree)).not.toBeNull();
    expect(perSecondToggle(tree)!.checked).toBe(true);
    expect(statusLine(tree)!.zeroPoint).toEqual({ zp: 25, system: "ab", per_second: true });
    (perSecondToggle(tree)!.onChange as (on: boolean) => void)(false);
    tree = show();
    expect(zeroPointField(tree)).not.toBeNull();
    expect(statusLine(tree)!.zeroPoint).toEqual({ zp: 25, system: "ab", per_second: false });
  });
});

describe("PhotometryPanel measurement log", () => {
  const show = () => renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });
  const arm = () => (findElement(show(), (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
  const chooseUnit = (unit: string) =>
    (findElement(show(), (props) => props["aria-label"] === "Radius unit")!.onChange as (event: { target: { value: string } }) => void)({
      target: { value: unit },
    });
  const typeAperture = (text: string) => (findElement(show(), byTitlePrefix("Aperture radius in"))!.onChange as (text: string) => void)(text);
  const typeZeroPoint = (text: string) => (findElement(show(), (props) => props.title === ZERO_POINT_TITLE)!.onChange as (text: string) => void)(text);
  const chooseSystem = (system: string) =>
    (findElement(show(), (props) => props["aria-label"] === "Zero point system")!.onChange as (event: { target: { value: string } }) => void)({
      target: { value: system },
    });
  const measuredLog = async (result: PhotometryMeasurement) => {
    measureMock.mockReset().mockResolvedValue(result);
    arm();
    pixelClick.current = { seq: 1, x: 10, y: 12 };
    show();
    await flush();
    show();
    return measurementLog.getSnapshot().at(-1)!;
  };
  const restart = () => {
    hooks.run.cells = [];
    pixelClick.current = null;
  };

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    measureMock.mockReset().mockReturnValue(new Promise(() => {}));
    pixelClick.current = null;
    measurementLog.clear();
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
  });

  it("logs the request as it was typed", async () => {
    statusDoc.current = scaleDoc(0.262);
    chooseUnit("arcsec");
    typeAperture("1");
    let entry = await measuredLog(measurementWith({}));
    expect(entry.params.radius_unit).toBe("arcsec");
    expect(entry.params.aperture_radius_arcsec).toBe(1);
    expect(entry.params.aperture_radius_px as number).toBeCloseTo(3.8167938931297707, 12);
    expect(entry.params.pixel_scale_arcsec).toBe(0.262);

    restart();
    typeAperture("5");
    entry = await measuredLog(measurementWith({}));
    expect(entry.params.radius_unit).toBe("px");
    expect(entry.params.aperture_radius_px).toBe(5);
    expect(entry.params.aperture_radius_arcsec).toBeNull();
    expect(entry.params.pixel_scale_arcsec).toBe(0.262);
  });

  it("logs the typed zero point", async () => {
    statusDoc.current = sourceDoc(null);
    typeZeroPoint("21.432");
    chooseSystem("vega");
    let entry = await measuredLog(
      measurementWith({ mag_inst: -9.87, net_flux: 8872, flux_err: 50, mag_ab: null, mag_ab_err: null, flux_jy: null, flux_err_jy: null }, { photcal: VEGA_USER }),
    );
    expect(entry.params.zero_point).toBe(21.432);
    expect(entry.params.zero_point_system).toBe("vega");
    expect(entry.params.calibration_source).toBe("user");

    restart();
    statusDoc.current = sourceDoc(null);
    typeZeroPoint("25");
    statusDoc.current = sourceDoc("header");
    entry = await measuredLog(measurementWith({}, { photcal: JWST_I2D }));
    expect(entry.params.zero_point).toBeNull();
  });
});

const PX_DEPTH = { radius_arcsec: null, radius_px: 3.82, f_corr: 1.65 } as unknown as ImageDepth;
const FAINT: Partial<StarPhotometry> = { net_flux: 20, flux_err: 12.3, flux_jy: 2.0e-7, flux_err_jy: 1.1e-7, mag_ab: 25.647, mag_ab_err: 0.6, err_used: true };
const NEAR_LIMIT: Partial<StarPhotometry> = { net_flux: 50, flux_err: 12.3, flux_jy: 5.0e-7, flux_err_jy: 1.1e-7, mag_ab: 24.653, mag_ab_err: 0.239, err_used: true };
const FACTOR_CLAUSE = "; errors multiplied by f_corr 1.65 from the Depth section";

describe("PhotometryPanel upper limit, Depth factor and Resolved or not card", () => {
  const show = () => renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });
  let clicks = 0;
  const measure = async (result: PhotometryMeasurement): Promise<ReactNode> => {
    measureMock.mockReset().mockResolvedValue(result);
    (findElement(show(), (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
    pixelClick.current = { seq: ++clicks, x: 10, y: 12 };
    show();
    await flush();
    return show();
  };
  const typeAperture = (text: string) => (findElement(show(), byTitlePrefix("Aperture radius in"))!.onChange as (text: string) => void)(text);
  const limitLine = (tree: ReactNode) => {
    const line = findElement(tree, (props) => props["data-photometry-limit"] === true);
    return line ? textOf(line.children) : null;
  };
  const offerLine = (tree: ReactNode) => findElement(tree, (props) => typeof props["data-depth-noise-offer"] === "string");
  const offerText = (tree: ReactNode) => (offerLine(tree)!.children as ReactNode[])[0];
  const offerButton = (tree: ReactNode) =>
    findElement(tree, (props) => props["data-action"] === "use-depth-fcorr" || props["data-action"] === "remove-depth-fcorr");
  const lastEntry = () => measurementLog.getSnapshot().at(-1)!;
  const resolutionCard = (tree: ReactNode) => elementsOf(tree).find((element) => (element.props as Props).context === "photometry");

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    measureMock.mockReset().mockReturnValue(new Promise(() => {}));
    pixelClick.current = null;
    clicks = 0;
    measurementLog.clear();
    depthHolder.current = null;
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
    depthHolder.current = null;
  });

  it("says not detected with the limit instead of leaving the magnitude empty", async () => {
    let tree = await measure(measurementWith(FAINT, { photcal: AB_USER }));
    expect(offerLine(tree)).toBeNull();
    expect(limitLine(tree)).toBe(`not detected at 3 sigma: AB > 25.10 (flux below ${formatFluxDensityJy(3.3e-7, null)})`);
    expect(limitLine(tree)).toBe("not detected at 3 sigma: AB > 25.10 (flux below 330.0 nJy)");
    expect(cardValue(tree, "AB mag")).toBe("> 25.10 (3 sigma)");
    expect(cardValue(tree, "Net flux adu")).toBe(formatWithUncertainty(20, 12.3));
    expect(cardValue(tree, "Net flux adu")).toBe("20 ± 12");
    expect(cardValue(tree, "Flux density")).toBe(formatFluxDensityJy(2.0e-7, 1.1e-7));

    tree = await measure(measurementWith({ ...FAINT, flux_jy: 5.0e-7, net_flux: 50 }, { photcal: AB_USER }));
    expect(limitLine(tree)).toBeNull();
    expect(cardValue(tree, "AB mag")).toBe(abCellText(25.647, 0.6));
  });

  it("offers the Depth factor for the typed aperture and applies it only after Use", async () => {
    depthHolder.current = PX_DEPTH;
    expect(offerLine(show())).toBeNull();
    typeAperture("3.82");
    const offer = noiseFactorOffer(PX_DEPTH, 3.82, "px");
    let tree = show();
    expect(offerLine(tree)!["data-depth-noise-offer"]).toBe("usable");
    expect(offerText(tree)).toBe(depthNoiseText(offer, null));
    expect(offerButton(tree)!["data-action"]).toBe("use-depth-fcorr");
    expect(offerButton(tree)!["aria-label"]).toBe("Use the Depth noise factor for this aperture");
    expect(offerButton(tree)!.children).toBe("Use");
    const nearLimit = measurementWith(NEAR_LIMIT, { photcal: AB_USER });
    tree = await measure(nearLimit);
    expect(limitLine(tree)).toBeNull();
    expect(cardValue(tree, "AB mag")).toBe(abCellText(24.653, 0.239));
    expect(lastEntry().params.f_corr).toBeNull();

    (offerButton(tree)!.onClick as () => void)();
    tree = show();
    expect(offerText(tree)).toBe(depthNoiseText(offer, 1.65));
    expect(offerButton(tree)!["data-action"]).toBe("remove-depth-fcorr");
    expect(offerButton(tree)!["aria-label"]).toBe("Do not use the Depth noise factor");
    expect(offerButton(tree)!.children).toBe("Remove");
    expect(limitLine(tree)).toBeNull();
    expect(cardValue(tree, "AB mag")).toBe(abCellText(24.653, 0.239));
    tree = await measure(nearLimit);
    expect(limitLine(tree)!.endsWith(FACTOR_CLAUSE)).toBe(true);
    expect(cardValue(tree, "AB mag")).toBe("> 24.56 (3 sigma)");
    expect(lastEntry().params.f_corr).toBe(1.65);

    tree = await measure(measurementWith({ ...NEAR_LIMIT, err_used: false }, { photcal: AB_USER }));
    expect(limitLine(tree)).toBeNull();

    (offerButton(tree)!.onClick as () => void)();
    tree = await measure(nearLimit);
    expect(limitLine(tree)).toBeNull();
    expect(lastEntry().params.f_corr).toBeNull();
  });

  it("logs the limit it showed", async () => {
    await measure(measurementWith(FAINT, { photcal: AB_USER }));
    let entry = lastEntry();
    expect(entry.params.limit_sigma).toBe(3);
    expect(entry.params.f_corr).toBeNull();
    expect(entry.values.upper_limit_jy as number).toBeCloseTo(3.3e-7, 15);
    expect(entry.values.upper_limit_ab as number).toBeCloseTo(25.103715150305284, 12);
    expect(entry.values.upper_limit_native as number).toBeCloseTo(36.9, 12);

    await measure(measurementWith({ ...FAINT, flux_jy: 5.0e-7, net_flux: 50 }, { photcal: AB_USER }));
    entry = lastEntry();
    expect(entry.params.limit_sigma).toBe(3);
    expect([entry.values.upper_limit_jy, entry.values.upper_limit_ab, entry.values.upper_limit_native]).toEqual([null, null, null]);
  });

  it("mounts the Resolved or not card under the result with the size and the signal-to-noise of the source", async () => {
    expect(resolutionCard(show())).toBeUndefined();
    statusDoc.current = { loading: false, status: { pixel_scale_arcsec: 0.1 }, error: null };
    const tree = await measure(measurementWith({ fwhm: 3.214, ee50_radius: 2.345, snr: 100.4, x: 10, y: 12 }));
    const card = resolutionCard(tree);
    expect(card?.type).toBe(ResolutionCard);
    const props = card!.props as Props;
    expect(props.filePath).toBe(FIRST);
    expect(props.fwhmArcsec as number).toBeCloseTo(0.3214, 12);
    expect(props.halfLightRadiusArcsec as number).toBeCloseTo(0.2345, 12);
    expect(props.snr).toBe(100.4);
    expect(props.sourceLabel).toBe("source at (10.0, 12.0)");
    const order = elementsOf(tree);
    const ee80 = order.findIndex((element) => textOf(element) === "EE80 r");
    expect(ee80).toBeGreaterThanOrEqual(0);
    expect(order.indexOf(card!)).toBeGreaterThan(ee80);

    statusDoc.current = { loading: false, status: { pixel_scale_arcsec: null }, error: null };
    const unscaled = resolutionCard(show())!.props as Props;
    expect(unscaled.fwhmArcsec).toBeNull();
    expect(unscaled.halfLightRadiusArcsec).toBeNull();
  });

  it("does not offer a factor measured with another aperture radius and drops an accepted one when the radius changes", async () => {
    depthHolder.current = PX_DEPTH;
    typeAperture("5");
    let tree = show();
    expect(offerLine(tree)!["data-depth-noise-offer"]).toBe("other_radius");
    expect(offerText(tree)).toBe(
      "The Depth section measured this image at r = 3.82 px, not with this aperture: its f_corr is not offered. Measure the depth again with this radius to get one.",
    );
    expect(offerButton(tree)).toBeNull();
    const nearLimit = measurementWith(NEAR_LIMIT, { photcal: AB_USER });
    tree = await measure(nearLimit);
    expect(limitLine(tree)).toBeNull();
    expect(lastEntry().params.f_corr).toBeNull();

    typeAperture("3.82");
    (offerButton(show())!.onClick as () => void)();
    expect(offerButton(show())!.children).toBe("Remove");
    typeAperture("5");
    show();
    tree = show();
    expect(offerLine(tree)!["data-depth-noise-offer"]).toBe("other_radius");
    tree = await measure(nearLimit);
    expect(limitLine(tree)).toBeNull();
    expect(lastEntry().params.f_corr).toBeNull();
    typeAperture("3.82");
    expect(offerButton(show())!.children).toBe("Use");
  });
});
