import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";

const FIRST = "C:/data/502nmos.fits";
const SECOND = "C:/data/f200w_i2d.fits";

const backend = vi.hoisted(() => ({
  batch: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  saved: [] as string[],
  savedText: [] as string[],
}));
const clipboardWrite = vi.hoisted(() => vi.fn<(text: string) => Promise<void>>());

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/analysis", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/analysis")>()),
  photometryGainModel: () => new Promise(() => {}),
  measurePhotometryBatch: backend.batch,
}));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => ({ excludeDq: false }) }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "502nmos.fits", image: "original" }) }));
const dialog = vi.hoisted(() => ({ cancel: false }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (d: { defaultPath: string }) => (dialog.cancel ? null : `C:/out/${d.defaultPath}`) }));
vi.mock("@tauri-apps/plugin-fs", () => ({
  writeTextFile: async (path: string, text: string) => {
    backend.saved.push(path);
    backend.savedText.push(text);
  },
}));
vi.stubGlobal("navigator", { clipboard: { writeText: clipboardWrite } });
const statusDoc = vi.hoisted(() => ({ current: { loading: false, status: null, error: null } as Record<string, unknown> }));
const keyedStatus = vi.hoisted(() => ({ doc: null as Record<string, unknown> | null, calls: [] as unknown[][] }));
vi.mock("../../../hooks/usePhotometryStatus", () => ({
  usePhotometryStatus: (...args: unknown[]) => {
    keyedStatus.calls.push(args);
    return args[3] != null && keyedStatus.doc !== null ? keyedStatus.doc : statusDoc.current;
  },
}));
const worldToPixel = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  worldToPixel,
}));
const depthHolder = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("../../../hooks/usePhotometryLimit", () => ({ useImageDepth: () => depthHolder.current }));
const dockDispatch = vi.hoisted(() => vi.fn<(action: unknown) => void>());
vi.mock("../../../hooks/useDockLayout", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../hooks/useDockLayout")>();
  return { ...actual, dockStore: { ...actual.dockStore, dispatch: dockDispatch } };
});

import { hooks } from "../../../test/hookRuntime";
import { elementsOf } from "../../../test/elementTree";
import {
  NO_DETECTED_STARS_HINT,
  PHOTOMETRY_COLUMN_KEYS,
  PHOTOMETRY_CSV_COLUMNS,
  STARS_ELSEWHERE_NOTICE,
  abCellText,
  photometryTableCsv,
  tableUnitsCaption,
} from "../../../utils/photometryTable";
import { calibratedMagnitude } from "../../../utils/photometryPanel";
import type { PhotCal, StarPhotometry } from "../../../services/analysis";
import type { ImageDepth } from "../../../shared/types/depth";
import { BATCH_ROWS_NOTE, measurementLog } from "../../../utils/measurementLog";
import { provenanceLines } from "../../../utils/csvProvenance";
import { CopyCsvButton, ErrorAlert } from "../../ui";
import { localSmallButtonLines } from "../../../test/panelRules";
import photometryTablePanelSource from "../PhotometryTablePanel.tsx?raw";

const { default: PhotometryTablePanel } = await import("../PhotometryTablePanel");

type Props = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): Props[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Props>(node)) return [];
  return [node.props, ...elements(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<Props>(node) ? textOf(node.props.children) : "";
}

function measuredStar(): Record<string, unknown> {
  return {
    x: 10.04, y: 12.06, peak: 900, net_flux: 1234.5678, flux_err: 12, mag_inst: -7.73, snr: 102.9, fwhm: 3.214,
    aperture_radius: 5, aperture_pixels: 78, aperture_area: 78.5, bg_mean: 10.25, bg_sigma: 2, bg_pixels: 200,
    saturated: false, saturation_source: "none", n_masked: 0, n_nonfinite: 0, n_saturated: 0, err_used: true,
    aperture_correction: 1.0421, ee_in_aperture: 0.9596, flux_total: 1286.6, plateau_radius: 9, flux_jy: null, flux_err_jy: null,
    mag_ab: 24.5123, mag_ab_err: 0.0314, mag_ab_total: null, st_mag: null, sky_inner: 10, sky_outer: 15, ee50_radius: 2.345,
    warnings: [],
  };
}

const STARS = [{ x: 10, y: 12, flux: 100, fwhm: 3, snr: 50 }];
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

type Panel = (props: Record<string, unknown>) => unknown;
const panel = (PhotometryTablePanel as unknown as { type: Panel }).type;
const show = (filePath = FIRST) => hooks.render(panel, { filePath, overlayKey: null, stars: STARS }) as ReactNode;
const notice = (filePath = FIRST) =>
  elements(show(filePath)).find((p) => typeof p.className === "string" && p.className.includes("text-emerald-400/90"));

function click(text: string): void {
  const found = elements(show()).find((p) => p.children !== undefined && textOf(p.children as ReactNode).trim() === text);
  expect(found?.disabled, `${text} is enabled`).not.toBe(true);
  (found!.onClick as () => void)();
}

function copyText(tree: ReactNode): string | null {
  const copyButton = elementsOf(tree).find((e) => e.type === CopyCsvButton);
  expect(copyButton, "Copy CSV button").toBeDefined();
  return ((copyButton!.props as Props).text as () => string | null)();
}

async function measured(): Promise<void> {
  const run = elements(show()).find((p) => p.label === "Measure 1");
  (run!.onClick as () => void)();
  await flush();
}

async function savedOnce(): Promise<void> {
  await measured();
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
  click("Save CSV");
  await vi.waitFor(() => expect(backend.saved).toHaveLength(1));
  expect(textOf(notice()?.children as ReactNode)).toBe(`Saved ${backend.saved[0]}`);
}

describe("PhotometryTablePanel Saved notice", () => {
  beforeEach(() => {
    hooks.mount();
    backend.saved = [];
    backend.batch.mockReset().mockResolvedValue({
      rows: [{ index: 0, photometry: measuredStar(), sky: null, error: null }],
      photcal: null,
      warnings: [],
      masked: false,
      n_measured: 1,
      n_failed: 0,
      elapsed_ms: 8,
    });
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
  });

  it("cancels the notice timer when the panel closes", async () => {
    await savedOnce();
    expect(vi.getTimerCount(), "the notice timer runs while the panel is open").toBe(1);
    hooks.mount();
    expect(vi.getTimerCount(), "no timer outlives the panel").toBe(0);
  });

  it("shows each save in one timed notice that a later save restarts", async () => {
    await savedOnce();
    await vi.advanceTimersByTimeAsync(3_000);
    click("Save CSV");
    await vi.waitFor(() => expect(backend.saved).toHaveLength(2));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(notice(), "the second save is still shown 5 s after it").toBeDefined();
    await vi.advanceTimersByTimeAsync(1_500);
    expect(notice()).toBeUndefined();
  });

  it("drops the notice and its timer when the table is cleared", async () => {
    await savedOnce();
    click("Clear");
    expect(notice()).toBeUndefined();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("drops the notice and its timer when another file is measured", async () => {
    await savedOnce();
    show(SECOND);
    expect(notice(SECOND)).toBeUndefined();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("announces the saved file as a status", async () => {
    await savedOnce();
    expect(notice()?.role).toBe("status");
  });
});

const SORTABLE_HEADERS = ["label", "x", "y", "flux", "SNR", "AB", "inst", "FWHM", "EE50", "corr"];
const SNR_COLUMN = SORTABLE_HEADERS.indexOf("SNR");
const FWHM_FALLBACK = "FWHM not measured: the automatic aperture radius of 6.0 px is a fallback, not 1.5 x FWHM";
const NO_DATA_3 = "3 aperture pixels without data (non-finite): the flux is a lower limit";
const PROCESSED = "photometry on processed data (stack)";

function batchOf(stars: Record<string, unknown>[], warnings: string[] = []): Record<string, unknown> {
  return {
    rows: stars.map((photometry, index) => ({ index, photometry, sky: null, error: null })),
    photcal: null,
    warnings,
    masked: false,
    n_measured: stars.length,
    n_failed: 0,
    elapsed_ms: 8,
  };
}

const headerCells = () => elementsOf(show()).filter((e) => e.type === "th");
const buttonIn = (th: { props: Props }) => elementsOf(th.props.children).find((e) => e.type === "button");
const sortGlyph = (th: { props: Props }) => elementsOf(th.props.children).find((e) => (e.props as Props)["aria-hidden"] === "true");

describe("PhotometryTablePanel sorting from the keyboard", () => {
  beforeEach(() => {
    hooks.mount();
    backend.batch.mockReset().mockResolvedValue(batchOf([measuredStar()]));
  });

  afterEach(() => {
    hooks.mount();
  });

  it("puts every sortable header label in a button and marks none as sorted while the rows keep their input order", async () => {
    await measured();
    const ths = headerCells();
    expect(ths.map((th) => textOf((th.props as Props).children))).toEqual([...SORTABLE_HEADERS, "flags"]);
    expect(ths.map((th) => (buttonIn(th)?.props as Props | undefined)?.type ?? null)).toEqual([...SORTABLE_HEADERS.map(() => "button"), null]);
    expect(ths.map((th) => (th.props as Props)["aria-sort"] ?? null)).toEqual([...SORTABLE_HEADERS.map(() => "none"), null]);
    expect((buttonIn(ths[SNR_COLUMN])?.props as Props | undefined)?.title).toBe("Sort by SNR");
  });

  it("sorts by SNR from its header button and announces ascending, then descending, with the arrow hidden from screen readers", async () => {
    await measured();
    const snrButton = () => buttonIn(headerCells()[SNR_COLUMN]);
    expect(snrButton(), "SNR header button").toBeDefined();
    ((snrButton()!.props as Props).onClick as () => void)();
    let ths = headerCells();
    expect((ths[SNR_COLUMN].props as Props)["aria-sort"]).toBe("ascending");
    expect(ths.filter((th) => (th.props as Props)["aria-sort"] === "none")).toHaveLength(SORTABLE_HEADERS.length - 1);
    expect(textOf((sortGlyph(ths[SNR_COLUMN])?.props as Props | undefined)?.children)).toBe("▲");
    ((snrButton()!.props as Props).onClick as () => void)();
    ths = headerCells();
    expect((ths[SNR_COLUMN].props as Props)["aria-sort"]).toBe("descending");
    expect(textOf((sortGlyph(ths[SNR_COLUMN])?.props as Props | undefined)?.children)).toBe("▼");
  });
});

describe("PhotometryTablePanel fields added by wave 0", () => {
  beforeEach(() => {
    hooks.mount();
  });

  afterEach(() => {
    hooks.mount();
  });

  const cellsOf = (column: string) => elementsOf(show()).filter((e) => e.type === "td" && (e.props as Props)["data-column"] === column);

  it("flags a row whose aperture holds pixels without data", async () => {
    backend.batch.mockReset().mockResolvedValue(batchOf([{ ...measuredStar(), n_nonfinite: 3, warnings: [NO_DATA_3] }]));
    await measured();
    const flags = elementsOf(show()).filter((e) => e.type === "td" && (e.props as Props)["data-column"] === undefined);
    expect(flags.map((td) => textOf((td.props as Props).children))).toEqual(["3 no data"]);
  });

  it("prints -- for a star whose FWHM was not measured", async () => {
    backend.batch.mockReset().mockResolvedValue(batchOf([{ ...measuredStar(), fwhm: null, warnings: [FWHM_FALLBACK] }]));
    await measured();
    expect(cellsOf("fwhm").map((td) => textOf((td.props as Props).children))).toEqual(["--"]);
  });

  it("lists each distinct per-star warning once, after the image warnings", async () => {
    backend.batch
      .mockReset()
      .mockResolvedValue(
        batchOf(
          [
            { ...measuredStar(), fwhm: null, warnings: [FWHM_FALLBACK] },
            { ...measuredStar(), fwhm: null, n_nonfinite: 3, warnings: [FWHM_FALLBACK, NO_DATA_3] },
          ],
          [PROCESSED],
        ),
      );
    await measured();
    const list = elementsOf(show()).find((e) => Array.isArray((e.props as Props).warnings));
    expect((list?.props as Props | undefined)?.warnings).toEqual([PROCESSED, FWHM_FALLBACK, NO_DATA_3]);
  });

  it("states the 6 px radius a blank aperture takes for a star whose FWHM is not measured", () => {
    const tree = show();
    const texts = elementsOf(tree).map((e) => (e.props as Props).children);
    expect(texts).toContain("blank: r_ap = 1.5 × FWHM (6 px if the FWHM is not measured), sky 2–3 × r_ap");
    const field = elementsOf(tree).find((e) => String((e.props as Props).title ?? "").startsWith("Aperture radius in pixels"));
    expect((field?.props as Props | undefined)?.title).toBe("Aperture radius in pixels; blank uses 1.5 × FWHM of each star, or 6 px for a star whose FWHM is not measured");
  });
});

describe("PhotometryTablePanel status line", () => {
  beforeEach(() => {
    hooks.mount();
  });

  afterEach(() => {
    hooks.mount();
  });

  it("puts the status line above the positions", () => {
    const order = elements(show());
    const line = order.findIndex((p) => p.filePath === FIRST && p.measureKey === null && p.excludeDq === false);
    const positions = order.findIndex((p) => p.children !== undefined && textOf(p.children as ReactNode).trim() === "Positions");
    expect(line, "status line element").toBeGreaterThanOrEqual(0);
    expect(order[line].zeroPoint).toBeNull();
    expect(positions, "Positions label").toBeGreaterThanOrEqual(0);
    expect(line).toBeLessThan(positions);
  });
});

describe("PhotometryTablePanel units caption", () => {
  beforeEach(() => {
    hooks.mount();
  });

  afterEach(() => {
    hooks.mount();
  });

  const captions = () => elementsOf(show()).filter((e) => typeof (e.props as Props).children === "string").map((e) => (e.props as Props).children);

  it("captions the table with the units of its columns", async () => {
    backend.batch.mockReset().mockResolvedValue({ ...batchOf([measuredStar()]), summed_unit: "MJy/sr x pix" });
    await measured();
    expect(captions()).toContain(tableUnitsCaption("MJy/sr x pix"));
    hooks.mount();
    backend.batch.mockReset().mockResolvedValue(batchOf([measuredStar()]));
    await measured();
    expect(captions()).toContain(tableUnitsCaption(null));
  });
});

describe("PhotometryTablePanel without detected stars", () => {
  beforeEach(() => {
    hooks.mount();
    dockDispatch.mockReset();
  });

  afterEach(() => {
    hooks.mount();
  });

  const showWith = (props: Record<string, unknown>) =>
    elements(hooks.render(panel, { filePath: FIRST, overlayKey: null, stars: [], ...props }) as ReactNode);
  const hint = (shown: Props[]) => shown.filter((p) => p.children === NO_DETECTED_STARS_HINT);
  const openButton = (shown: Props[]) => shown.filter((p) => p.type === "button" && textOf(p.children as ReactNode).trim() === "Open Astrometry");
  const elsewhere = (shown: Props[]) => shown.filter((p) => p.children === STARS_ELSEWHERE_NOTICE);

  it("points to Detect Stars when no star was detected", () => {
    const shown = showWith({});
    expect(hint(shown)).toHaveLength(1);
    expect(openButton(shown)).toHaveLength(1);
    (openButton(shown)[0].onClick as () => void)();
    expect(dockDispatch).toHaveBeenCalledWith({ type: "open", tool: "astrometry" });

    hooks.mount();
    const withStars = showWith({ stars: STARS });
    expect(hint(withStars)).toHaveLength(0);
    expect(openButton(withStars)).toHaveLength(0);

    hooks.mount();
    const fromComposite = showWith({ stars: STARS, starsElsewhere: true });
    expect(hint(fromComposite)).toHaveLength(0);
    expect(openButton(fromComposite)).toHaveLength(0);
    expect(elsewhere(fromComposite)).toHaveLength(1);
  });
});

const PIXEL_SCALE_PENDING = "Reading the pixel scale of this image...";
const NO_PIXEL_SCALE = "Radii in arcsec need the pixel scale of a celestial WCS, which this image does not have. Switch the unit to px.";
const SKY_PLACEHOLDER = "53.1301 -28.1002 a\n03:32:31.2 -28:06:00.7 b";
const SKY_POSITIONS_NOTE =
  "Each sky position is a starting point: it snaps to the brightest pixel within 8 px and is recentred, like every other source of positions. For positions that must not move, use the Multi-band section.";
const NO_CELESTIAL_WCS = "This image has no celestial WCS: sky positions cannot be placed on it.";
const scaleDoc = (pixelScale: number | null) => ({ loading: false, status: { pixel_scale_arcsec: pixelScale }, error: null });
const shownTexts = () => elements(show()).map((p) => textOf(p.children as ReactNode));
const measureButton = (label: string) => elements(show()).find((p) => p.label === label);
const radiusErrorText = () => {
  const line = elements(show()).find((p) => p["data-radius-error"] === true);
  return line ? textOf(line.children as ReactNode) : null;
};

describe("PhotometryTablePanel radius unit", () => {
  beforeEach(() => {
    hooks.mount();
    statusDoc.current = scaleDoc(0.262);
    backend.batch.mockReset().mockReturnValue(new Promise(() => {}));
  });

  afterEach(() => {
    hooks.mount();
    statusDoc.current = { loading: false, status: null, error: null };
  });

  const chooseUnit = (unit: string) =>
    (elements(show()).find((p) => p["aria-label"] === "Radius unit")!.onChange as (event: { target: { value: string } }) => void)({
      target: { value: unit },
    });
  const apertureField = () => elements(show()).find((p) => String(p.title ?? "").startsWith("Aperture radius in"))!;

  it("sends the pixel radius of an aperture typed in arcsec to the batch", () => {
    chooseUnit("arcsec");
    expect(apertureField().value).toBe("1.31");
    (apertureField().onChange as (text: string) => void)("1");
    const run = measureButton("Measure 1")!;
    expect(run.disabled).toBe(false);
    (run.onClick as () => void)();
    expect(backend.batch).toHaveBeenCalledTimes(1);
    expect((backend.batch.mock.calls[0][2] as Props).apertureRadius).toBeCloseTo(3.8167938931297707, 10);
  });

  it("waits for the pixel scale instead of refusing an arcsec radius while the status is pending", () => {
    chooseUnit("arcsec");
    (apertureField().onChange as (text: string) => void)("1");
    const cases: [Record<string, unknown>, string][] = [
      [{ loading: true, status: null, error: null }, PIXEL_SCALE_PENDING],
      [{ loading: false, status: null, error: null }, PIXEL_SCALE_PENDING],
      [{ loading: false, status: null, error: "no such file" }, "Status not available: no such file"],
      [scaleDoc(null), NO_PIXEL_SCALE],
    ];
    for (const [doc, expected] of cases) {
      statusDoc.current = doc;
      expect(radiusErrorText()).toBe(expected);
      if (expected !== NO_PIXEL_SCALE) expect(shownTexts().some((text) => text.includes("Radii in arcsec need the pixel scale"))).toBe(false);
      const run = measureButton("Measure 1")!;
      expect(run.disabled).toBe(true);
      (run.onClick as () => void)();
    }
    expect(backend.batch).not.toHaveBeenCalled();
  });
});

describe("PhotometryTablePanel sky positions", () => {
  beforeEach(() => {
    hooks.mount();
    statusDoc.current = scaleDoc(0.262);
    worldToPixel.mockReset();
    backend.batch
      .mockReset()
      .mockImplementation(async (_path: unknown, points: unknown) => batchOf((points as unknown[]).map(() => measuredStar())));
  });

  afterEach(() => {
    hooks.mount();
    statusDoc.current = { loading: false, status: null, error: null };
    vi.restoreAllMocks();
  });

  const chooseSky = () =>
    (elements(show()).find((p) => typeof p.onChange === "function" && p.value === "stars")!.onChange as (event: {
      target: { value: string };
    }) => void)({ target: { value: "sky" } });
  const typeSky = (text: string) =>
    (elements(show()).find((p) => p.placeholder === SKY_PLACEHOLDER)!.onChange as (event: { target: { value: string } }) => void)({
      target: { value: text },
    });
  const labelCells = () =>
    elementsOf(show())
      .filter((e) => e.type === "td" && (e.props as Props)["data-column"] === "label")
      .map((td) => textOf((td.props as Props).children as ReactNode));
  const placementNote = () => {
    const note = elements(show()).find((p) => p["data-placement-note"] === true);
    return note ? textOf(note.children as ReactNode) : null;
  };
  const placed = (points: ([number, number] | null)[], onImage: boolean[]) => ({ points, on_image: onImage, frame: "icrs", naxis1: 64, naxis2: 64 });
  const measureSky = async (label: string) => {
    const run = measureButton(label);
    expect(run, label).toBeDefined();
    expect(run!.disabled).toBe(false);
    (run!.onClick as () => void)();
    await flush();
  };

  it("offers sky positions after the pasted positions", () => {
    const options = elements(show()).filter((p) => typeof p.value === "string" && ["stars", "regions", "pasted", "sky"].includes(p.value as string) && p.onChange === undefined);
    expect(options.map((p) => p.value)).toEqual(["stars", "regions", "pasted", "sky"]);
    expect(textOf(options[3].children as ReactNode)).toBe("Sky positions (RA Dec)");
  });

  it("measures a pasted list of sky positions after placing them on the image", async () => {
    worldToPixel.mockResolvedValue(placed([[10.2, 12.1], null], [true, false]));
    chooseSky();
    typeSky("53.1301 -28.1002 a\n53.2 -28.3");
    await measureSky("Measure 2");
    expect(worldToPixel).toHaveBeenCalledWith(FIRST, [[53.1301, -28.1002], [53.2, -28.3]], "icrs");
    expect(backend.batch).toHaveBeenCalledTimes(1);
    expect(backend.batch.mock.calls[0][1]).toEqual([[10.2, 12.1]]);
    expect(labelCells()[0]).toBe("a");
    expect(placementNote()).toBe("1 of 2 sky positions are outside the image and were left out");
    const clear = elements(show()).find((p) => p.children !== undefined && textOf(p.children as ReactNode).trim() === "Clear");
    (clear!.onClick as () => void)();
    expect(placementNote()).toBeNull();
  });

  it("labels unnamed sky positions by their place in the list", async () => {
    worldToPixel.mockResolvedValue(placed([[10, 12], [30, 32]], [true, true]));
    chooseSky();
    typeSky("53.1 -28.1\n53.2 -28.3");
    await measureSky("Measure 2");
    expect(labelCells()).toEqual(["s1", "s2"]);
    expect(placementNote()).toBeNull();
  });

  it("says when no sky position falls on the image and measures nothing", async () => {
    worldToPixel.mockResolvedValue(placed([[-40, -40], [900, 900]], [false, false]));
    chooseSky();
    typeSky("53.1 -28.1\n53.2 -28.3");
    await measureSky("Measure 2");
    expect(elements(show()).find((p) => p.message === "None of the 2 sky positions falls on this image.")).toBeDefined();
    expect(backend.batch).not.toHaveBeenCalled();
  });

  it("cannot place sky positions on an image without a celestial WCS", () => {
    statusDoc.current = scaleDoc(null);
    chooseSky();
    typeSky("53.1 -28.1");
    expect(shownTexts()).toContain(NO_CELESTIAL_WCS);
    const run = measureButton("Measure 1")!;
    expect(run.disabled).toBe(true);
    (run.onClick as () => void)();
    expect(worldToPixel).not.toHaveBeenCalled();
  });

  it("lists the lines of the sky list that do not parse", () => {
    chooseSky();
    typeSky("53.1 -28.1\nhello");
    const errors = elements(show()).filter((p) => typeof p.children === "string" && (p.children as string).startsWith("line "));
    expect(errors).toHaveLength(1);
    expect(errors[0].children as string).toMatch(/^line 2:/);
    expect(measureButton("Measure 1")).toBeDefined();
  });

  it("explains that sky positions are recentred", () => {
    expect(shownTexts()).not.toContain(SKY_POSITIONS_NOTE);
    chooseSky();
    expect(shownTexts()).toContain(SKY_POSITIONS_NOTE);
  });

  it("logs sky positions as their own source", async () => {
    measurementLog.clear();
    worldToPixel.mockResolvedValue(placed([[10.2, 12.1], null], [true, false]));
    chooseSky();
    typeSky("53.1301 -28.1002 a\n53.2 -28.3");
    await measureSky("Measure 2");
    const entry = measurementLog.getSnapshot().at(-1)!;
    expect(entry.source).toBe('{"kind":"points","n":1,"from":"sky"}');
    expect(entry.values.n_sky_requested).toBe(2);
    expect(entry.values.n_sky_outside).toBe(1);
  });
});

describe("PhotometryTablePanel measurement log", () => {
  beforeEach(() => {
    hooks.mount();
    backend.saved = [];
    dialog.cancel = false;
    measurementLog.clear();
    backend.batch.mockReset().mockResolvedValue({
      rows: [{ index: 0, photometry: measuredStar(), sky: null, error: null }],
      photcal: null,
      warnings: [],
      masked: false,
      n_measured: 1,
      n_failed: 0,
      elapsed_ms: 8,
    });
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
    dialog.cancel = false;
  });

  it("logs the saved table once it is written", async () => {
    await savedOnce();
    const entries = measurementLog.getSnapshot();
    expect(entries.map((entry) => entry.kind)).toEqual(["photometry_batch", "photometry_batch"]);
    expect(entries[0].notes.at(-1)).toBe(BATCH_ROWS_NOTE);
    expect(entries[1].params.csv_path).toBe(backend.saved[0]);
    expect(entries[1].params.csv_rows).toBe(1);
    expect(entries[1].notes.at(-1)).toBe("per-source rows saved to 502nmos_photometry.csv (1 row)");
  });

  it("logs nothing for a cancelled save or a copy", async () => {
    await measured();
    expect(measurementLog.getSnapshot()).toHaveLength(1);
    dialog.cancel = true;
    click("Save CSV");
    await flush();
    await flush();
    expect(backend.saved).toHaveLength(0);
    expect(measurementLog.getSnapshot()).toHaveLength(1);
    copyText(show());
    await flush();
    expect(measurementLog.getSnapshot()).toHaveLength(1);
  });
});

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
const ZERO_POINT_TITLE = "Zero point in magnitudes of one pixel unit; used because the header has no calibration";
const PER_SECOND_LABEL = "Zero point is per second";
const PER_SECOND_REFUSAL =
  "a zero point per second needs EXPTIME to turn the pixel values into a rate, and this header has none (BUNIT 'adu')";
const sourceDoc = (calibrationSource: string | null) => ({
  loading: false,
  status: { pixel_scale_arcsec: null, calibration_source: calibrationSource },
  error: null,
});
const vegaStar = () => ({ ...measuredStar(), mag_inst: -9.87, net_flux: 8872, flux_err: 50, mag_ab: null, mag_ab_err: null });

describe("PhotometryTablePanel calibration", () => {
  beforeEach(() => {
    hooks.mount();
    backend.batch.mockReset().mockReturnValue(new Promise(() => {}));
  });

  afterEach(() => {
    hooks.mount();
    statusDoc.current = { loading: false, status: null, error: null };
    keyedStatus.doc = null;
    keyedStatus.calls = [];
  });

  const magHeader = () => {
    const th = headerCells()[SORTABLE_HEADERS.indexOf("AB")];
    return textOf(((buttonIn(th)?.props as Props | undefined)?.children ?? null) as ReactNode);
  };
  const magCells = () =>
    elementsOf(show())
      .filter((e) => e.type === "td" && (e.props as Props)["data-column"] === "mag_ab")
      .map((td) => textOf((td.props as Props).children));
  const zeroPointField = () => elements(show()).find((p) => p.title === ZERO_POINT_TITLE);
  const systemSelect = () => elements(show()).find((p) => p["aria-label"] === "Zero point system");
  const perSecondToggle = () => elements(show()).find((p) => p.label === PER_SECOND_LABEL);
  const statusLine = () => elements(show()).find((p) => p.filePath === FIRST && p.excludeDq === false && "zeroPoint" in p);
  const typeZeroPoint = (text: string) => (zeroPointField()!.onChange as (text: string) => void)(text);

  it("heads the magnitude column by the system and fills it for a Vega zero point", async () => {
    backend.batch.mockReset().mockResolvedValue({ ...batchOf([vegaStar(), vegaStar()]), photcal: VEGA_USER });
    await measured();
    expect(magHeader()).toBe("Vega");
    const expected = abCellText(21.432 - 9.87, (1.0857362047581294 * 50) / 8872);
    expect(magCells()).toEqual([expected, expected]);

    hooks.mount();
    backend.batch.mockReset().mockResolvedValue(batchOf([measuredStar()]));
    await measured();
    expect(magHeader()).toBe("AB");
    expect(magCells()).toEqual(["--"]);
  });

  it("sends the typed zero point with the batch", () => {
    statusDoc.current = sourceDoc(null);
    typeZeroPoint("21.432");
    (systemSelect()!.onChange as (event: { target: { value: string } }) => void)({ target: { value: "vega" } });
    const expected = { zp: 21.432, system: "vega", per_second: false };
    expect(statusLine()!.zeroPoint).toEqual(expected);
    const run = measureButton("Measure 1")!;
    expect(run.disabled).toBe(false);
    (run.onClick as () => void)();
    expect(backend.batch).toHaveBeenCalledTimes(1);
    expect((backend.batch.mock.calls[0][2] as Props).zeroPoint).toEqual(expected);
    expect(keyedStatus.calls.length).toBeGreaterThan(0);
    for (const call of keyedStatus.calls) expect(call).toHaveLength(3);
  });

  it("sends no zero point and offers no field when the header calibrates", () => {
    statusDoc.current = sourceDoc(null);
    typeZeroPoint("25");
    statusDoc.current = sourceDoc("header");
    expect(zeroPointField()).toBeUndefined();
    expect(systemSelect()).toBeUndefined();
    expect(perSecondToggle()).toBeUndefined();
    (measureButton("Measure 1")!.onClick as () => void)();
    expect((backend.batch.mock.calls[0][2] as Props).zeroPoint).toBeUndefined();
  });

  it("names the zero point field in the error line and does not measure an invalid entry", () => {
    statusDoc.current = sourceDoc(null);
    typeZeroPoint("2,5");
    const error = elements(show()).find((p) => p["data-field-error"] === true);
    expect(textOf(error?.children as ReactNode)).toBe("zero point: Use a dot (.) as the decimal separator");
    const run = measureButton("Measure 1")!;
    expect(run.disabled).toBe(true);
    (run.onClick as () => void)();
    expect(backend.batch).not.toHaveBeenCalled();
  });

  it("shows the origin sentence in the calibration card", async () => {
    backend.batch.mockReset().mockResolvedValue({ ...batchOf([vegaStar()]), photcal: VEGA_USER });
    await measured();
    const card = elements(show()).find((p) => Array.isArray(p.children) && textOf((p.children as ReactNode[])[0]) === "Calibration");
    expect(textOf((card!.children as ReactNode[])[1])).toBe("user zero point 21.432 Vega");
    hooks.mount();
    backend.batch.mockReset().mockResolvedValue(batchOf([measuredStar()]));
    await measured();
    const uncalibrated = elements(show()).find((p) => Array.isArray(p.children) && textOf((p.children as ReactNode[])[0]) === "Calibration");
    expect(textOf((uncalibrated!.children as ReactNode[])[1])).toBe("uncalibrated");
  });

  it("keeps the zero point row while the status of the typed value is being read", () => {
    statusDoc.current = sourceDoc(null);
    keyedStatus.doc = { loading: true, status: null, error: null };
    typeZeroPoint("2");
    expect(zeroPointField()!.value).toBe("2");
    expect(systemSelect()).toBeDefined();
    expect(perSecondToggle()).toBeDefined();
    expect(statusLine()!.zeroPoint).toEqual({ zp: 2, system: "ab", per_second: false });
  });

  it("keeps the row and shows the refusal of a per-second zero point in the status line", () => {
    statusDoc.current = sourceDoc(null);
    keyedStatus.doc = { loading: false, status: null, error: PER_SECOND_REFUSAL };
    typeZeroPoint("25");
    (perSecondToggle()!.onChange as (on: boolean) => void)(true);
    expect(zeroPointField()).toBeDefined();
    expect(perSecondToggle()!.checked).toBe(true);
    expect(statusLine()!.zeroPoint).toEqual({ zp: 25, system: "ab", per_second: true });
    (perSecondToggle()!.onChange as (on: boolean) => void)(false);
    expect(zeroPointField()).toBeDefined();
    expect(statusLine()!.zeroPoint).toEqual({ zp: 25, system: "ab", per_second: false });
  });
});

const F200W_STATUS = {
  loading: false,
  status: {
    pixel_scale_arcsec: 0.031,
    calibration_source: null,
    filter: { dataset: "JWST", filter: "F200W", pivot_um: 1.988648, source: "header FILTER", warning: null },
    err: { origin: "EXTNAME ERR, HDU 2: as is" },
    dq: { source: "EXTNAME DQ, HDU 3: JWST DQ flags", exclusion_available: true, unavailable_reason: null },
  },
  error: null,
};
const withoutExportTime = (text: string) => text.split("\r\n").filter((line) => !line.startsWith("# exported_utc: "));

describe("PhotometryTablePanel CSV provenance", () => {
  beforeEach(() => {
    hooks.mount();
    backend.saved = [];
    backend.savedText = [];
    dialog.cancel = false;
    clipboardWrite.mockReset().mockResolvedValue(undefined);
    statusDoc.current = F200W_STATUS;
    backend.batch.mockReset().mockResolvedValue(batchOf([measuredStar()]));
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
    statusDoc.current = { loading: false, status: null, error: null };
  });

  it("saves the table with its provenance", async () => {
    await savedOnce();
    const text = backend.savedText[0];
    expect(text.startsWith("# software: AstroBurst ")).toBe(true);
    const lines = text.split("\r\n");
    expect(lines).toContain("# file: 502nmos.fits");
    expect(lines).toContain("# position_source: detected stars");
    expect(lines).toContain("# filter: JWST F200W, pivot 1.98865 um, from the header FILTER");
    expect(lines).toContain("# error_layer: EXTNAME ERR, HDU 2: as is");
    expect(lines).toContain("# calibration: none");
    const star = measuredStar() as unknown as StarPhotometry;
    const body = photometryTableCsv([
      { index: 0, label: "star 1", photometry: star, sky: null, error: null, magnitude: calibratedMagnitude(star, null) },
    ]);
    expect(text.endsWith(body)).toBe(true);
  });

  it("copies the same text", async () => {
    await savedOnce();
    const copied = copyText(show())!;
    expect(copied.split("\r\n").some((line) => line.startsWith("# exported_utc: "))).toBe(true);
    expect(withoutExportTime(copied)).toEqual(withoutExportTime(backend.savedText[0]));
  });

  it("describes the image as it was measured", async () => {
    await measured();
    statusDoc.current = {
      ...F200W_STATUS,
      status: { ...F200W_STATUS.status, dq: { source: "EXTNAME MASK, HDU 4: other flags", exclusion_available: true, unavailable_reason: null } },
    };
    const lines = copyText(show())!.split("\r\n");
    expect(lines).toContain("# mask_layer: EXTNAME DQ, HDU 3: JWST DQ flags");
    expect(lines).not.toContain("# mask_layer: EXTNAME MASK, HDU 4: other flags");
  });
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
const PX_DEPTH = { radius_arcsec: null, radius_px: 3.82, f_corr: 1.65 } as unknown as ImageDepth;
const TWO_STARS = [
  { x: 10, y: 12, flux: 100, fwhm: 3, snr: 50 },
  { x: 40, y: 42, flux: 60, fwhm: 3, snr: 30 },
];
const faintStar = () => ({ ...measuredStar(), net_flux: 20, flux_err: 12.3, flux_jy: 2.0e-7, flux_err_jy: 1.1e-7, mag_ab: 25.647, mag_ab_err: 0.6, err_used: true });
const brightStar = () => ({
  ...measuredStar(),
  x: 40.1,
  y: 42.2,
  net_flux: 50,
  flux_err: 12.3,
  flux_jy: 5.0e-7,
  flux_err_jy: 1.1e-7,
  mag_ab: 24.653,
  mag_ab_err: 0.239,
  err_used: true,
});
const LIMIT_CAPTION =
  "1 of 2 measured rows are below 3 sigma: a magnitude cell with > is a 3 sigma limit; the CSV keeps the measured flux and adds the limit columns";

describe("PhotometryTablePanel upper limits and the Depth factor", () => {
  const showTwo = () => hooks.render(panel, { filePath: FIRST, overlayKey: null, stars: TWO_STARS }) as ReactNode;
  const measureTwo = async () => {
    const run = elements(showTwo()).find((p) => p.label === "Measure 2");
    expect(run?.disabled).toBe(false);
    (run!.onClick as () => void)();
    await flush();
  };
  const cellsOf = (column: string) =>
    elementsOf(showTwo())
      .filter((e) => e.type === "td" && (e.props as Props)["data-column"] === column)
      .map((td) => textOf((td.props as Props).children as ReactNode));
  const flagCells = () =>
    elementsOf(showTwo())
      .filter((e) => e.type === "td" && (e.props as Props)["data-column"] === undefined)
      .map((td) => textOf((td.props as Props).children as ReactNode));
  const limitCaption = () => {
    const caption = elements(showTwo()).find((p) => p["data-limit-caption"] === true);
    return caption ? textOf(caption.children as ReactNode) : null;
  };
  const offerLine = () => elements(showTwo()).find((p) => typeof p["data-depth-noise-offer"] === "string");
  const offerButton = () => elements(showTwo()).find((p) => p["data-action"] === "use-depth-fcorr" || p["data-action"] === "remove-depth-fcorr");
  const typeAperture = (text: string) =>
    (elements(showTwo()).find((p) => String(p.title ?? "").startsWith("Aperture radius in"))!.onChange as (text: string) => void)(text);
  const runEntries = () => measurementLog.getSnapshot().filter((entry) => entry.params.csv_path === undefined);
  const savedLines = async () => {
    const before = backend.savedText.length;
    const save = elements(showTwo()).find((p) => p.children !== undefined && textOf(p.children as ReactNode).trim() === "Save CSV");
    (save!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.savedText).toHaveLength(before + 1));
    return backend.savedText[before].split("\r\n");
  };

  beforeEach(() => {
    hooks.mount();
    backend.saved = [];
    backend.savedText = [];
    dialog.cancel = false;
    depthHolder.current = null;
    measurementLog.clear();
    backend.batch.mockReset().mockResolvedValue({ ...batchOf([faintStar(), brightStar()]), photcal: AB_USER });
  });

  afterEach(() => {
    hooks.mount();
    depthHolder.current = null;
  });

  it("prints a 3 sigma limit in the magnitude cell and flags the row", async () => {
    await measureTwo();
    expect(offerLine()).toBeUndefined();
    expect(cellsOf("mag_ab")).toEqual(["> 25.10", abCellText(24.653, 0.239)]);
    const flags = flagCells();
    expect(flags[0].split(", ")).toContain("below 3 sigma");
    expect(flags[1]).toBe("");
    expect(limitCaption()).toBe(LIMIT_CAPTION);

    hooks.mount();
    backend.batch.mockReset().mockResolvedValue({ ...batchOf([brightStar(), { ...brightStar(), x: 70, y: 72 }]), photcal: AB_USER });
    await measureTwo();
    expect(limitCaption()).toBeNull();
    expect(flagCells()).toEqual(["", ""]);
  });

  it("writes the limit columns and the limit lines of the provenance", async () => {
    await measureTwo();
    const lines = await savedLines();
    expect(lines).toContain(provenanceLines([["limit_sigma", 3]])[0]);
    expect(lines).toContain("# limit_sigma: 3");
    expect(lines).toContain("# f_corr: not applied");
    const header = lines.findIndex((line) => line.startsWith("index,label,"));
    const first = lines[header + 1].split(",");
    const second = lines[header + 2].split(",");
    expect(first[PHOTOMETRY_COLUMN_KEYS.indexOf("flux_jy")]).toBe("2e-7");
    expect(Number(first.at(-2))).toBeCloseTo(3.3e-7, 10);
    expect(Number(first.at(-1))).toBeCloseTo(25.103715150305284, 10);
    expect(second.slice(-2)).toEqual(["", ""]);
  });

  it("logs how many rows are limits", async () => {
    await measureTwo();
    const entry = runEntries().at(-1)!;
    expect(entry.values.n_limits).toBe(1);
    expect(entry.params.limit_sigma).toBe(3);
    expect(entry.params.f_corr).toBeNull();
    await savedLines();
    const saved = measurementLog.getSnapshot().at(-1)!;
    expect(saved.params.csv_path).toBeDefined();
    expect(saved.values.n_limits).toBe(1);
    expect(saved.params.limit_sigma).toBe(3);
  });

  it("applies the Depth factor to a batch only after Use and only for the radius it was measured with", async () => {
    depthHolder.current = PX_DEPTH;
    typeAperture("3.82");
    expect(offerLine()!["data-depth-noise-offer"]).toBe("usable");
    expect(offerButton()!["data-action"]).toBe("use-depth-fcorr");
    expect(offerButton()!.children).toBe("Use");
    await measureTwo();
    expect(cellsOf("mag_ab")).toEqual(["> 25.10", abCellText(24.653, 0.239)]);
    expect(runEntries().at(-1)!.values.n_limits).toBe(1);
    expect(runEntries().at(-1)!.params.f_corr).toBeNull();
    expect(await savedLines()).toContain("# f_corr: not applied");

    (offerButton()!.onClick as () => void)();
    expect(offerButton()!.children).toBe("Remove");
    expect(cellsOf("mag_ab")).toEqual(["> 25.10", abCellText(24.653, 0.239)]);
    await measureTwo();
    expect(cellsOf("mag_ab")).toEqual(["> 24.56", "> 24.56"]);
    expect(limitCaption()).toBe(
      "2 of 2 measured rows are below 3 sigma: a magnitude cell with > is a 3 sigma limit; the CSV keeps the measured flux and adds the limit columns Errors multiplied by f_corr 1.65 from the Depth section where an error layer was used.",
    );
    expect(runEntries().at(-1)!.values.n_limits).toBe(2);
    expect(runEntries().at(-1)!.params.f_corr).toBe(1.65);
    expect(await savedLines()).toContain(
      "# f_corr: 1.65 from the Depth section (r = 3.82 px), applied on request to rows measured with an error layer",
    );

    typeAperture("5");
    expect(offerLine()!["data-depth-noise-offer"]).toBe("other_radius");
    expect(offerButton()).toBeUndefined();
    await measureTwo();
    expect(runEntries().at(-1)!.params.f_corr).toBeNull();
    expect(runEntries().at(-1)!.values.n_limits).toBe(1);
  });
});

describe("PhotometryTablePanel shared controls", () => {
  beforeEach(() => {
    hooks.mount();
    backend.batch.mockReset().mockResolvedValue(batchOf([measuredStar()]));
  });

  afterEach(() => hooks.mount());

  it("keeps no private copy of the small button class", () => {
    expect(localSmallButtonLines(photometryTablePanelSource)).toEqual([]);
    expect(photometryTablePanelSource).toMatch(/import \{[^}]*\bSMALL_BUTTON_CLASS\b[^}]*\} from "\.\.\/ui";/);
  });

  it("hands the measured table to the shared copy button", async () => {
    await measured();
    const copyButtons = elementsOf(show()).filter((e) => e.type === CopyCsvButton);
    expect(copyButtons).toHaveLength(1);
    const text = ((copyButtons[0].props as Props).text as () => string | null)();
    expect(typeof text).toBe("string");
    expect(text).toContain(PHOTOMETRY_CSV_COLUMNS.map((c) => c.header).join(","));
  });

  it("shows a refused clipboard write in the panel's alert", async () => {
    await measured();
    const copyButton = elementsOf(show()).find((e) => e.type === CopyCsvButton);
    ((copyButton!.props as Props).onError as (m: string) => void)("Clipboard copy failed: Document is not focused.");
    const alert = elementsOf(show()).find((e) => e.type === ErrorAlert);
    expect((alert?.props as Props | undefined)?.message).toBe("Clipboard copy failed: Document is not focused.");
  });
});
