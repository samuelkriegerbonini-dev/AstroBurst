import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement } from "react";
import type { ReactElement, ReactNode } from "react";
import { Check } from "lucide-react";
import { renderToStaticMarkup } from "react-dom/server";
import type { GainModel, PhotometryMeasurement } from "../../../services/analysis";


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

import { hooks } from "../../../test/hookRuntime";
import PhotometryPanel from "../PhotometryPanel";
import PhotometryTablePanel from "../PhotometryTablePanel";
import { fluxCellText } from "../../../utils/photometryTable";

const GAIN_TITLE_TEXT = "Gain in e-/ADU; prefilled from the header when known";

vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {}, removeItem: () => {} });
vi.stubGlobal("navigator", { clipboard: { writeText: clipboardWrite } });

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
      n_saturated: 0,
      err_used: false,
      aperture_correction: null,
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
    expect(findElement(tree, (props) => props.children === `Clipboard copy failed: ${CLIPBOARD_DENIED}`)).not.toBeNull();
    expect(showsCopied(tree)).toBe(false);
    expect(copyStatusText(tree)).toBe("");
  });

  it("clears the clipboard error once a copy succeeds", async () => {
    clipboardWrite.mockReset().mockRejectedValueOnce(new DOMException(CLIPBOARD_DENIED, "NotAllowedError")).mockResolvedValue(undefined);
    await measureStar(flush);
    const failure = `Clipboard copy failed: ${CLIPBOARD_DENIED}`;
    (skyCopyButton(show())!.onClick as () => void)();
    await flush();
    expect(findElement(show(), (props) => props.children === failure)).not.toBeNull();
    (skyCopyButton(show())!.onClick as () => void)();
    await flush();
    expect(findElement(show(), (props) => props.children === failure)).toBeNull();
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
    saturated: false, saturation_source: "none", n_masked: 0, n_saturated: 0, err_used: true,
    aperture_correction: 1.0421, flux_total: 1286.6, plateau_radius: 9, flux_jy: null, flux_err_jy: null,
    mag_ab: 24.5123, mag_ab_err: 0.0314, mag_ab_total: null, st_mag: null, sky_inner: 10, sky_outer: 15, ee50_radius: 2.345,
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
      photcal: null,
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
