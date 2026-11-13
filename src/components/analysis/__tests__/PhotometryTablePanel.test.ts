import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";

const FIRST = "C:/data/502nmos.fits";
const SECOND = "C:/data/f200w_i2d.fits";

const backend = vi.hoisted(() => ({
  batch: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  saved: [] as string[],
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/analysis", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/analysis")>()),
  photometryGainModel: () => new Promise(() => {}),
  measurePhotometryBatch: backend.batch,
}));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => ({ excludeDq: false }) }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "502nmos.fits", image: "original" }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (dialog: { defaultPath: string }) => `C:/out/${dialog.defaultPath}` }));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile: async (path: string) => void backend.saved.push(path) }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf } from "../../../test/elementTree";

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
