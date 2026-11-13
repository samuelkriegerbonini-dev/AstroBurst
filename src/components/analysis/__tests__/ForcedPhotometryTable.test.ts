import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";

const dialog = vi.hoisted(() => ({ answer: "save" as string | null }));
const saved = vi.hoisted(() => [] as [string, string][]);
const settings = vi.hoisted(() => ({ current: null as unknown }));
const imagePsf = vi.hoisted(() => ({ current: null as unknown }));
const measurePsf = vi.hoisted(() => vi.fn<(path: string) => Promise<void>>());
const check = vi.hoisted(() => vi.fn<(request: unknown) => Promise<unknown>>());

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: async (d: { defaultPath: string }) => (dialog.answer === null ? null : `C:/out/${d.defaultPath}`),
}));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile: async (path: string, text: string) => void saved.push([path, text]) }));
vi.mock("../../../hooks/useResolutionSettings", () => ({
  useResolutionSettings: () => settings.current,
  useImagePsf: () => imagePsf.current,
  imagePsfStore: { measure: measurePsf },
  useHeaderSeeing: () => [],
}));
vi.mock("../../../services/resolution", () => ({ resolutionCheck: check }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { forcedPhotometryStore } from "../../../hooks/useForcedPhotometryStore";
import { forcedSizeStore } from "../../../hooks/useForcedSizes";
import { DEFAULT_RESOLUTION_SETTINGS, psfChoice, resolutionRequest } from "../../../utils/resolutionText";
import type { ImagePsfFwhm, ResolutionCheck, ResolutionSettings, ResolutionVerdict } from "../../../shared/types/resolution";
import { APP_VERSION } from "../../../utils/constants";
import {
  colourCellText,
  fluxCellText,
  forcedColour,
  forcedLongCsv,
  forcedWideCsv,
  magCellText,
} from "../../../utils/forcedPhotometry";
import type {
  ForcedApertureCorrection,
  ForcedBand,
  ForcedCell,
  ForcedPhotometryResult,
} from "../../../shared/types/forcedPhotometry";

const { ForcedPhotometryTableInner, FORCED_TABLE_ROW_LIMIT } = await import("../ForcedPhotometryTable");
const { flagsText, sizePsfText, sizeRequests, tableColours, tableColumns } = await import("../forcedPhotometryTableModel");

type Props = Record<string, unknown> & { children?: ReactNode };
type Element = ReactElement<Props>;

const NO_WCS_ERROR = "x.fits has no celestial WCS: sky positions cannot be placed on it";
const EXPORTED_UTC = "2026-11-12T00:00:00.000Z";
const APERTURE_SENTENCE = "Colours are between aperture magnitudes.";
const TOTAL_SENTENCE = "Colours are between total magnitudes.";
const MIXED_SENTENCE = "Colours are between total magnitudes where both bands have an aperture correction, aperture magnitudes otherwise.";

function cell(overrides: Partial<ForcedCell> = {}): ForcedCell {
  return {
    x: 10.5,
    y: 20.25,
    on_image: true,
    error: null,
    flux_native: 0.550813,
    flux_err_native: 0.0275406,
    flux_jy: 2e-6,
    flux_err_jy: 1e-7,
    flux_err_corr_jy: null,
    flux_total_jy: null,
    mag_ab: 23.147425,
    mag_ab_err: 0.054287,
    upper_limit_jy: null,
    upper_limit_ab: null,
    snr: 20,
    masked_fraction: 0,
    n_nonfinite: 0,
    n_masked: 0,
    flags_or: 0,
    saturated: false,
    fwhm_px: null,
    ...overrides,
  };
}

function band(index: number, label: string, pivot: number | null, overrides: Partial<ForcedBand> = {}): ForcedBand {
  return {
    index,
    path: `C:/data/${label}.fits`,
    file_name: `${label}.fits`,
    label,
    error: null,
    filter: pivot === null ? null : { dataset: "DECAM", filter: label, pivot_um: pivot, source: "header" },
    photcal: null,
    calibration_origin: "BUNIT nanomaggy, zero point 22.5",
    jy_per_native_unit: 3.631e-6,
    bunit: "nanomaggy",
    pixel_scale_arcsec: 0.262,
    aperture_radius_px: 7.6,
    annulus_px: null,
    aperture_correction: null,
    err_origin: null,
    dq_source: null,
    f_corr: null,
    n_measured: 1,
    n_failed: 0,
    warnings: [],
    ...overrides,
  };
}

function correction(factor: number): ForcedApertureCorrection {
  return { radius_px: 7.6, reference_radius_px: 13.4, factor, scatter: 0.01, n_stars: 12, n_candidates: 20 };
}

function failedBand(index: number, label: string, error: string): ForcedBand {
  return band(index, label, null, {
    error,
    calibration_origin: null,
    jy_per_native_unit: null,
    bunit: null,
    pixel_scale_arcsec: null,
    aperture_radius_px: null,
    n_measured: 0,
  });
}

function run(bands: ForcedBand[], cellsBySource: (ForcedCell | null)[][]): ForcedPhotometryResult {
  return {
    bands,
    sources: cellsBySource.map((cells, i) => ({ id: `s${i}`, ra_deg: 150.1 + i * 0.001, dec_deg: 2.2, cells })),
    aperture: {
      radius_arcsec: 2,
      radius_px: null,
      annulus_arcsec: null,
      annulus_px: null,
      correction: "none",
      reference_radius_arcsec: null,
    },
    limit_sigma: 3,
    elapsed_ms: 12,
    warnings: [],
  };
}

const gDetected = cell({ flux_jy: 1e-6, flux_err_jy: 1e-7, mag_ab: 23.9, mag_ab_err: 0.108574 });
const rDetected = cell({ flux_jy: 2e-6, flux_err_jy: 1e-7, mag_ab: 23.147425, mag_ab_err: 0.054287 });
const zDetected = cell({ flux_jy: 3e-6, flux_err_jy: 1e-7, mag_ab: 22.707197, mag_ab_err: 0.036191 });
const gLimit = cell({
  flux_jy: 5e-8,
  flux_err_jy: 1e-7,
  mag_ab: null,
  mag_ab_err: null,
  upper_limit_jy: 3e-7,
  upper_limit_ab: 25.207197,
  snr: 0.5,
});
const rSaturated = cell({ saturated: true, masked_fraction: 0.034, flags_or: 0 });
const zOffImage = cell({
  x: null,
  y: null,
  on_image: false,
  flux_native: null,
  flux_err_native: null,
  flux_jy: null,
  flux_err_jy: null,
  mag_ab: null,
  mag_ab_err: null,
  snr: null,
  masked_fraction: null,
});

const BANDS = [band(0, "r", 0.6418), band(1, "g", 0.4808), band(2, "z", 0.9168), failedBand(3, "x", NO_WCS_ERROR)];
const RESULT = run(BANDS, [
  [{ ...rDetected, fwhm_px: 3.74 }, gDetected, zDetected, null],
  [{ ...rDetected, fwhm_px: 3.9 }, gLimit, zDetected, null],
  [rSaturated, gDetected, zOffImage, null],
]);

const PSF: ImagePsfFwhm = {
  fwhm_px: 3.725,
  fwhm_px_scatter: 0.076,
  fwhm_arcsec: 0.976,
  fwhm_arcsec_scatter: 0.02,
  half_light_radius_px: null,
  half_light_radius_arcsec: null,
  n_stars: 70,
  n_measured: 77,
  n_candidates: 90,
  n_detected: 400,
  max_stars: 200,
  model: "width at half maximum of a radial fit to the core of each star, the FWHM of the Photometry panel",
  pixel_scale_arcsec: 0.262,
  pixels_per_fwhm: 3.725,
  undersampled: false,
  header_values: [],
  stars: [],
  notes: [],
  warnings: [],
  elapsed_ms: 1,
};
const MEASURE_TEXT = "Measure the PSF of this image in the Resolution section.";
const NO_SIZE_TEXT = "No band of this run has a measured FWHM: sizes cannot be checked.";
const SIZE_LEGEND =
  "Sizes: FWHM at the forced position in r against a PSF FWHM of 0.976 arcsec (70 stars of this image (width at half maximum of a radial fit to the core of each star, the FWHM of the Photometry panel)); thresholds as set in the Resolution section. A quick look for the first 200 rows: select a row for the inputs of its verdict.";
const NO_FWHM_LEGEND =
  '"no FWHM": the fit of the width found none at the forced position, which happens on a source too faint for it or far wider than the PSF (a large galaxy, a blend); it is not a verdict. Size such a source with an ellipse in the Image tool, section Profiles (R50 against the PSF).';

const cellOf = (source: number, bandIndex: number) => RESULT.sources[source].cells[bandIndex];
const csvInput = (result: ForcedPhotometryResult) => ({ result, exportedUtc: EXPORTED_UTC, appVersion: APP_VERSION });
const answer = (verdict: ResolutionVerdict | null) => ({ verdict }) as ResolutionCheck;
const withoutSizes = (result: ForcedPhotometryResult) =>
  run(
    result.bands,
    result.sources.map((s) => s.cells.map((c) => (c === null ? null : { ...c, fwhm_px: null }))),
  );

const clipboard = vi.fn<(text: string) => Promise<void>>();

const show = (result: ForcedPhotometryResult = RESULT, selected: number | null = null, currentPath: string | null = null) =>
  elementsOf<Props>(hooks.render(ForcedPhotometryTableInner, { result, selected, currentPath })) as Element[];
const statusOf = (shown: Element[]) => withData(shown, "data-forced-size-status");
const withData = (shown: Element[], name: string) => shown.find((e) => name in e.props);
const allWithData = (shown: Element[], name: string) => shown.filter((e) => name in e.props);
const byData = (shown: Element[], name: string, value: string) => shown.find((e) => e.props[name] === value);
const rowOf = (shown: Element[], index: number) => shown.find((e) => e.props["data-forced-row"] === index)!;
const cellsOfRow = (row: Element) => elementsOf<Props>(row.props.children).filter((e) => e.type === "td").map((e) => textOf(e.props.children));
const headers = (shown: Element[]) => shown.filter((e) => e.type === "th").map((e) => textOf(e.props.children));
const legendOf = (shown: Element[]) => textOf(withData(shown, "data-forced-legend")!.props.children);
const click = (element: Element | undefined) => (element!.props.onClick as () => void)();
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("ForcedPhotometryTable", () => {
  beforeEach(() => {
    hooks.mount();
    forcedPhotometryStore.reset();
    forcedSizeStore.clear();
    settings.current = { ...DEFAULT_RESOLUTION_SETTINGS };
    imagePsf.current = { psf: PSF, loading: false, error: null };
    measurePsf.mockReset().mockResolvedValue(undefined);
    check.mockReset();
    dialog.answer = "save";
    saved.length = 0;
    clipboard.mockReset().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText: clipboard } });
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-11-12T00:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    hooks.mount();
    forcedPhotometryStore.reset();
  });

  it("columns follow the measured bands by wavelength and the colours their neighbours", () => {
    expect(tableColumns(RESULT)).toEqual([
      { band: 1, label: "g", title: "g.fits" },
      { band: 0, label: "r", title: "r.fits" },
      { band: 2, label: "z", title: "z.fits" },
    ]);
    const colours = tableColours(RESULT);
    expect(colours.map((c) => c.label)).toEqual(["g-r", "r-z"]);
    expect(colours.map((c) => [c.a, c.b])).toEqual([
      [1, 0],
      [0, 2],
    ]);
    expect(colours[0].title).toBe("g minus r, AB magnitudes");
    const shown = show();
    expect(headers(shown)).toEqual(["ID", "g", "r", "z", "g-r", "r-z", "Flags"]);
    expect(shown.find((e) => e.type === "th" && e.props.children === "g")!.props.title).toBe("g.fits");
    expect(shown.find((e) => e.type === "th" && e.props.children === "g-r")!.props.title).toBe("g minus r, AB magnitudes");
    const table = withData(shown, "data-forced-table")!;
    expect(table.type).toBe("table");
    const caption = shown.find((e) => e.type === "caption")!;
    expect(caption.props.className).toBe("sr-only");
    expect(textOf(caption.props.children)).toBe("Multi-band photometry: one row per position, one column per band");
    expect(withData(shown, "data-forced-table-block")).toBeDefined();
  });

  it("cells are the texts of the numbers module", () => {
    const shown = show();
    expect(cellsOfRow(rowOf(shown, 0))).toEqual([
      "s0",
      fluxCellText(cellOf(0, 1), RESULT.bands[1]),
      fluxCellText(cellOf(0, 0), RESULT.bands[0]),
      fluxCellText(cellOf(0, 2), RESULT.bands[2]),
      colourCellText(forcedColour(RESULT, 0, 1, 0)),
      colourCellText(forcedColour(RESULT, 0, 0, 2)),
      "",
    ]);
    expect(cellsOfRow(rowOf(shown, 2))[3]).toBe(fluxCellText(cellOf(2, 2), RESULT.bands[2]));
  });

  it("the switch shows AB magnitudes or limits", () => {
    let shown = show();
    expect(byData(shown, "data-view", "flux")!.props["aria-pressed"]).toBe(true);
    expect(byData(shown, "data-view", "mag")!.props["aria-pressed"]).toBe(false);
    click(byData(shown, "data-view", "mag"));
    shown = show();
    expect(byData(shown, "data-view", "mag")!.props["aria-pressed"]).toBe(true);
    expect(byData(shown, "data-view", "flux")!.props["aria-pressed"]).toBe(false);
    expect(textOf(byData(shown, "data-view", "mag")!.props.children)).toBe("AB mag");
    const magCell = cellsOfRow(rowOf(shown, 1))[1];
    expect(magCell).toBe(magCellText(cellOf(1, 1)));
    expect(magCell.startsWith("> ")).toBe(true);
    expect(cellsOfRow(rowOf(shown, 0))[2]).toBe(magCellText(cellOf(0, 0)));
    click(byData(shown, "data-view", "flux"));
    shown = show();
    expect(textOf(byData(shown, "data-view", "flux")!.props.children)).toBe("Flux");
    expect(cellsOfRow(rowOf(shown, 1))[1].startsWith("< ")).toBe(true);
  });

  it("a limit makes the colour a bound", () => {
    const gr = cellsOfRow(rowOf(show(), 1))[4];
    expect(gr).toBe(colourCellText(forcedColour(RESULT, 1, 1, 0)));
    expect(gr.startsWith("> ")).toBe(true);
  });

  it("what is wrong with a cell is written per band", () => {
    expect(flagsText(RESULT, 2)).toBe("r: saturated, 3.4% no data");
    expect(flagsText(RESULT, 0)).toBe("");
    const flagged = run(
      [
        BANDS[0],
        band(1, "g", 0.4808, {
          flag_names: [
            { value: 1, name: "NPRIMARY" },
            { value: 4, name: "SATUR_G" },
          ],
        }),
        BANDS[2],
        BANDS[3],
      ],
      [
        [rDetected, gDetected, zDetected, null],
        [rDetected, gLimit, zDetected, null],
        [rSaturated, cell({ ...gDetected, flags_or: 5 }), zOffImage, null],
      ],
    );
    expect(flagsText(flagged, 2)).toBe("g: NPRIMARY, SATUR_G; r: saturated, 3.4% no data");
    expect(cellsOfRow(rowOf(show(flagged), 2))[6]).toBe("g: NPRIMARY, SATUR_G; r: saturated, 3.4% no data");
  });

  it("a row is selected by click, Enter or space and released by a second click", () => {
    const select = vi.spyOn(forcedPhotometryStore, "select");
    let shown = show();
    click(rowOf(shown, 1));
    expect(select).toHaveBeenLastCalledWith(1);
    const preventDefault = vi.fn();
    const keyDown = rowOf(shown, 2).props.onKeyDown as (e: { key: string; preventDefault: () => void }) => void;
    keyDown({ key: "Enter", preventDefault });
    expect(select).toHaveBeenLastCalledWith(2);
    select.mockClear();
    preventDefault.mockClear();
    keyDown({ key: " ", preventDefault });
    expect(select).toHaveBeenCalledWith(2);
    expect(preventDefault).toHaveBeenCalled();
    select.mockClear();
    keyDown({ key: "a", preventDefault: vi.fn() });
    expect(select).not.toHaveBeenCalled();

    shown = show(RESULT, 1);
    const rows = allWithData(shown, "data-forced-row");
    expect(rows).toHaveLength(3);
    for (const row of rows) expect(row.props.tabIndex).toBe(0);
    expect(rowOf(shown, 1).props["aria-selected"]).toBe(true);
    expect(rowOf(shown, 1).props["data-selected"]).toBe(true);
    for (const index of [0, 2]) {
      expect(rowOf(shown, index).props["aria-selected"]).toBe(false);
      expect(rowOf(shown, index).props["data-selected"]).toBeUndefined();
    }
    click(rowOf(shown, 1));
    expect(select).toHaveBeenLastCalledWith(null);
  });

  it("only the first 200 rows are drawn and the note says so", () => {
    const g = band(0, "g", 0.4808);
    expect(FORCED_TABLE_ROW_LIMIT).toBe(200);
    let shown = show(run([g], Array.from({ length: 250 }, () => [gDetected])));
    expect(allWithData(shown, "data-forced-row")).toHaveLength(200);
    expect(textOf(withData(shown, "data-forced-row-note")!.props.children)).toBe(
      "First 200 of 250 positions shown; the CSV holds every row.",
    );
    shown = show(run([g], Array.from({ length: 200 }, () => [gDetected])));
    expect(allWithData(shown, "data-forced-row")).toHaveLength(200);
    expect(withData(shown, "data-forced-row-note")).toBeUndefined();
  });

  it("the legend says which magnitudes the colours use", () => {
    const legendStart =
      'Flux in uJy with its one-sigma error (multiplied by the noise factor where a band has one). "< x" is the 3 sigma upper limit and "> m" the magnitude it corresponds to; the measured flux stays in the CSV.';
    expect(legendOf(show())).toBe(`${legendStart} ${APERTURE_SENTENCE}`);

    const corrected = (labels: string[]) =>
      run(
        BANDS.map((b) => (labels.includes(b.label) ? { ...b, aperture_correction: correction(1.2) } : b)),
        RESULT.sources.map((s) => s.cells),
      );
    hooks.mount();
    expect(legendOf(show(corrected(["g", "r", "z"])))).toBe(`${legendStart} ${TOTAL_SENTENCE}`);
    hooks.mount();
    expect(legendOf(show(corrected(["g", "r"])))).toBe(`${legendStart} ${MIXED_SENTENCE}`);

    hooks.mount();
    const single = run([band(0, "g", 0.4808), failedBand(1, "x", NO_WCS_ERROR)], [[gDetected, null]]);
    let shown = show({ ...single, limit_sigma: 5 });
    expect(legendOf(shown)).toBe(legendStart.replace("the 3 sigma", "the 5 sigma"));
    expect(headers(shown)).toEqual(["ID", "g", "Flags"]);

    hooks.mount();
    shown = show(run([failedBand(0, "x", NO_WCS_ERROR)], [[null], [null]]));
    expect(headers(shown)).toEqual(["ID", "Flags"]);
    expect(legendOf(shown)).toBe(legendStart);
    expect(cellsOfRow(rowOf(shown, 1))).toEqual(["s1", ""]);
  });

  it("copy writes the csv of the chosen kind to the clipboard", async () => {
    let shown = show();
    const kind = withData(shown, "data-field")!;
    expect(kind.props["data-field"]).toBe("csvKind");
    expect(kind.props.value).toBe("long");
    const options = elementsOf<Props>(kind.props.children).filter((e) => e.type === "option");
    expect(options.map((o) => [o.props.value, textOf(o.props.children)])).toEqual([
      ["long", "long: one row per position and band"],
      ["wide", "wide: one row per position"],
    ]);
    const copy = byData(shown, "data-action", "copy-forced-csv")!;
    expect(textOf(copy.props.children)).toBe("Copy CSV");
    click(copy);
    expect(clipboard).toHaveBeenCalledTimes(1);
    expect(clipboard).toHaveBeenLastCalledWith(forcedLongCsv(csvInput(RESULT)));
    await tick();
    shown = show();
    expect(textOf(withData(shown, "data-forced-export-notice")!.props.children)).toBe("Copied the long table");

    (kind.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "wide" } });
    shown = show();
    expect(withData(shown, "data-field")!.props.value).toBe("wide");
    click(byData(shown, "data-action", "copy-forced-csv"));
    expect(clipboard).toHaveBeenCalledTimes(2);
    expect(clipboard).toHaveBeenLastCalledWith(forcedWideCsv(csvInput(RESULT)));
    await tick();
    expect(textOf(withData(show(), "data-forced-export-notice")!.props.children)).toBe("Copied the wide table");
  });

  it("save asks for a file named after the kind and says where it went", async () => {
    let shown = show();
    (withData(shown, "data-field")!.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "wide" } });
    shown = show();
    const save = byData(shown, "data-action", "save-forced-csv")!;
    expect(textOf(save.props.children)).toBe("Save CSV");
    click(save);
    await vi.waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual(["C:/out/forced_photometry_wide.csv", forcedWideCsv(csvInput(RESULT))]);
    await tick();
    expect(textOf(withData(show(), "data-forced-export-notice")!.props.children)).toBe("Saved C:/out/forced_photometry_wide.csv");

    dialog.answer = null;
    click(byData(show(), "data-action", "save-forced-csv"));
    await tick();
    await tick();
    shown = show();
    expect(saved).toHaveLength(1);
    expect(withData(shown, "data-forced-export-notice")).toBeUndefined();
    expect(withData(shown, "data-forced-export-error")).toBeUndefined();
  });

  it("a failed export is told", async () => {
    clipboard.mockRejectedValueOnce(new Error("denied"));
    click(byData(show(), "data-action", "copy-forced-csv"));
    await tick();
    let shown = show();
    const error = withData(shown, "data-forced-export-error")!;
    expect(error.type).toBe("div");
    expect(error.props.role).toBe("alert");
    expect(textOf(error.props.children)).toBe("Multi-band export failed: denied");
    expect(withData(shown, "data-forced-export-notice")).toBeUndefined();
    click(byData(shown, "data-action", "copy-forced-csv"));
    await tick();
    shown = show();
    expect(withData(shown, "data-forced-export-error")).toBeUndefined();
    expect(textOf(withData(shown, "data-forced-export-notice")!.props.children)).toBe("Copied the long table");
  });

  it("requests carry the size and signal-to-noise of each row in the size band", () => {
    const choice = psfChoice(DEFAULT_RESOLUTION_SETTINGS, PSF);
    const requests = sizeRequests(RESULT, 0, DEFAULT_RESOLUTION_SETTINGS, choice, 200);
    expect(requests).toHaveLength(3);
    expect(requests[0]).toEqual(
      resolutionRequest(DEFAULT_RESOLUTION_SETTINGS, choice, 0.262, { fwhmArcsec: 3.74 * 0.262, snr: cellOf(0, 0)!.snr }),
    );
    expect(requests[0]!.snr).toBe(20);
    expect(Math.abs((requests[0]!.source_fwhm_arcsec as number) - 0.97988)).toBeLessThan(1e-12);
    expect(requests[0]!.psf_fwhm_arcsec).toBe(0.976);
    expect(requests[1]!.source_fwhm_arcsec).toBeCloseTo(3.9 * 0.262, 12);
    expect(requests[2]).toBeNull();
    expect(sizeRequests(RESULT, 0, DEFAULT_RESOLUTION_SETTINGS, choice, 2)).toHaveLength(2);
    expect(sizePsfText(choice)).toBe(
      "0.976 arcsec (70 stars of this image (width at half maximum of a radial fit to the core of each star, the FWHM of the Photometry panel))",
    );
  });

  it("Check sizes fills a column for the band with sizes and says against which PSF", async () => {
    check.mockResolvedValueOnce(answer("unresolved")).mockResolvedValueOnce(answer("marginal"));
    let shown = show(RESULT, null, "C:/data/g.fits");
    const button = byData(shown, "data-action", "check-sizes")!;
    expect(textOf(button.props.children)).toBe("Check sizes");
    expect(button.props.title).toBe(
      "Resolved or not, for the rows on screen: the FWHM measured at each forced position against the PSF of the Resolution section",
    );
    expect(button.props.disabled).toBe(false);
    expect(statusOf(shown)).toBeUndefined();
    expect(headers(shown)).toEqual(["ID", "g", "r", "z", "g-r", "r-z", "Flags"]);
    click(button);
    await tick();
    await tick();
    shown = show(RESULT, null, "C:/data/g.fits");
    expect(check).toHaveBeenCalledTimes(2);
    expect((check.mock.calls[0][0] as { source_fwhm_arcsec: number }).source_fwhm_arcsec).toBeCloseTo(0.97988, 12);
    expect(headers(shown)).toEqual(["ID", "g", "r", "z", "g-r", "r-z", "Size (r)", "Flags"]);
    expect([0, 1, 2].map((i) => cellsOfRow(rowOf(shown, i))[6])).toEqual(["Unresolved", "Marginally resolved", "no FWHM"]);
    expect(cellsOfRow(rowOf(shown, 0))[7]).toBe("");
    expect(legendOf(shown)).toContain(
      "Sizes: FWHM at the forced position in r against a PSF FWHM of 0.976 arcsec (70 stars of this image",
    );
    expect(legendOf(shown).endsWith(` ${SIZE_LEGEND} ${NO_FWHM_LEGEND}`)).toBe(true);
    expect(statusOf(shown)).toBeUndefined();

    const many = run([BANDS[0]], Array.from({ length: 201 }, () => [{ ...rDetected, fwhm_px: 3.74 }]));
    expect(sizeRequests(many, 0, DEFAULT_RESOLUTION_SETTINGS, psfChoice(DEFAULT_RESOLUTION_SETTINGS, PSF), 200)).toHaveLength(200);
    check.mockReset().mockResolvedValue(answer("unresolved"));
    hooks.mount();
    click(byData(show(many), "data-action", "check-sizes"));
    await vi.waitFor(() => expect(forcedSizeStore.getSnapshot().running).toBe(false));
    expect(check).toHaveBeenCalledTimes(200);
    shown = show(many);
    expect(allWithData(shown, "data-forced-row")).toHaveLength(200);
    expect(cellsOfRow(rowOf(shown, 199))[2]).toBe("Unresolved");
    expect(legendOf(shown)).not.toContain("no FWHM");
  });

  it("the verdicts of another result are not shown", async () => {
    check.mockResolvedValue(answer("unresolved"));
    click(byData(show(RESULT, null, "C:/data/g.fits"), "data-action", "check-sizes"));
    await tick();
    await tick();
    expect(headers(show(RESULT, null, "C:/data/g.fits"))).toContain("Size (r)");
    hooks.mount();
    const shown = show({ ...RESULT }, null, "C:/data/g.fits");
    expect(headers(shown)).toEqual(["ID", "g", "r", "z", "g-r", "r-z", "Flags"]);
    expect(legendOf(shown)).not.toContain("Sizes:");
    expect(cellsOfRow(rowOf(shown, 0))).toHaveLength(7);
  });

  it("without a PSF the check is blocked and the PSF can be measured here", () => {
    imagePsf.current = { psf: null, loading: false, error: null };
    let shown = show(RESULT, null, "C:/data/g.fits");
    expect(byData(shown, "data-action", "check-sizes")!.props.disabled).toBe(true);
    let status = statusOf(shown)!;
    const measure = elementsOf<Props>(status.props.children).find((e) => e.props["data-action"] === "measure-size-psf")!;
    expect(textOf(status.props.children)).toBe(`${MEASURE_TEXT}Measure the PSF of r`);
    expect(textOf(measure.props.children)).toBe("Measure the PSF of r");
    expect(measure.props.disabled).toBe(false);
    click(measure);
    expect(measurePsf).toHaveBeenCalledWith("C:/data/r.fits");

    imagePsf.current = { psf: null, loading: true, error: null };
    shown = show(RESULT, null, "C:/data/g.fits");
    const measuring = byData(shown, "data-action", "measure-size-psf")!;
    expect(measuring.props.disabled).toBe(true);
    expect(textOf(measuring.props.children)).toBe("Measuring the PSF\u2026");

    imagePsf.current = { psf: null, loading: false, error: "no star was measured" };
    shown = show(RESULT, null, "C:/data/g.fits");
    status = statusOf(shown)!;
    const failure = elementsOf<Props>(status.props.children).find((e) => e.props.role === "alert")!;
    expect(textOf(failure.props.children)).toBe("no star was measured");
    expect(textOf(status.props.children)).toContain(MEASURE_TEXT);

    imagePsf.current = { psf: PSF, loading: false, error: null };
    settings.current = { ...DEFAULT_RESOLUTION_SETTINGS, psfSource: "typed" } satisfies ResolutionSettings;
    shown = show(RESULT, null, "C:/data/g.fits");
    expect(byData(shown, "data-action", "check-sizes")!.props.disabled).toBe(true);
    expect(textOf(statusOf(shown)!.props.children)).toBe("Type a PSF FWHM in the Resolution section.");
    expect(byData(shown, "data-action", "measure-size-psf")).toBeUndefined();
  });

  it("no band with a measured size", () => {
    const shown = show(withoutSizes(RESULT), null, "C:/data/r.fits");
    expect(byData(shown, "data-action", "check-sizes")!.props.disabled).toBe(true);
    expect(textOf(statusOf(shown)!.props.children)).toBe(NO_SIZE_TEXT);
    expect(byData(shown, "data-action", "measure-size-psf")).toBeUndefined();
  });

  it("progress and failure are told", async () => {
    check.mockReturnValue(new Promise(() => undefined));
    let shown = show();
    click(byData(shown, "data-action", "check-sizes"));
    shown = show();
    expect(textOf(statusOf(shown)!.props.children)).toBe("Checking sizes: 0 of 2");
    expect(byData(shown, "data-action", "check-sizes")!.props.disabled).toBe(true);

    check.mockReset().mockRejectedValue(new Error("boom"));
    forcedSizeStore.clear();
    click(byData(show(), "data-action", "check-sizes"));
    await tick();
    shown = show();
    const failure = elementsOf<Props>(statusOf(shown)!.props.children).find((e) => e.props.role === "alert")!;
    expect(textOf(failure.props.children)).toBe("Size check failed: boom");
    expect(byData(shown, "data-action", "check-sizes")!.props.disabled).toBe(false);
  });
});
