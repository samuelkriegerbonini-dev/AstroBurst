import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import type { CompareBandInfo, ComparePreset, CompareProbe, CompareReport, CompareRow } from "../../../shared/types/catalogCompare";
import type { CatalogPainterOptions } from "../../viewer/painters/catalogPainter";

const backend = vi.hoisted(() => ({
  add: vi.fn<(key: string, layer: Record<string, unknown>) => void>(),
  remove: vi.fn<(key: string, id: string) => void>(),
  painter: vi.fn<(options: unknown) => () => void>(),
  save: vi.fn<(text: string, defaultPath: string, title: string) => Promise<string | null>>(),
}));
const live = vi.hoisted(() => ({ key: "rk" as string | null }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => live.key }));
vi.mock("../../../utils/overlayStore", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../utils/overlayStore")>()),
  overlayStore: { add: backend.add, remove: backend.remove },
}));
vi.mock("../../viewer/painters/catalogPainter", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../viewer/painters/catalogPainter")>()),
  createCatalogPainter: backend.painter,
}));
vi.mock("../../../utils/spectrumExport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../utils/spectrumExport")>()),
  saveCsvDialog: backend.save,
}));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import {
  compareCsvHeaderLines,
  compareCsvText,
  compareRequestOf,
  fluxCellUjy,
  formFromPreset,
  type CompareRun,
} from "../../../utils/catalogCompare";
import { formatWithUncertainty } from "../../../utils/formatUncertainty";
import { CATALOG_LAYER_KIND } from "../../viewer/painters/catalogPainter";
import { ErrorAlert } from "../../ui";

const { default: CompareRowsView } = await import("../CompareRowsView");
const { reasonText } = await import("../compareRowsViewModel");

const LEGACY_R: ComparePreset = {
  id: "legacy_dr10",
  label: "Legacy Surveys DR10",
  band: "r",
  mapping: {
    id: "ls_id",
    ra: "ra",
    dec: "dec",
    flux: "apflux_r_5",
    flux_unit: "nanomaggy",
    zero_point: null,
    flux_err: null,
    flux_ivar: "apflux_ivar_r_5",
    class: "type",
    missing_value: null,
  },
  aperture: { radius_arcsec: 2, radius_px: null, annulus_arcsec: null, annulus_px: null },
  cuts: [
    { column: "type", op: "==", value: "PSF" },
    { column: "maskbits", op: "==", value: 0 },
    { column: "fracflux_r", op: "<", value: 0.01 },
  ],
  min_snr: null,
  max_snr: null,
  min_neighbour_arcsec: null,
  notes: [],
};

const BAND: CompareBandInfo = {
  file_name: "legacysurvey-0532m280-image-r.fits.fz",
  label: "r",
  filter: { dataset: "DECAM", filter: "r", pivot_um: 0.6418, source: "header FILTER" },
  calibration_origin: "BUNIT nanomaggy, zero point 22.5",
  jy_per_native_unit: 3.6307805477010175e-6,
  bunit: "nanomaggy",
  pixel_scale_arcsec: 0.262,
  aperture_radius_px: 7.633587786259541,
  annulus_px: null,
  err_origin: {
    role: "sigma",
    source: "inverse variance from legacysurvey-0532m280-invvar-r.fits.fz",
    assumed: false,
    n_no_data: 12,
  },
  dq_source: null,
  warnings: [],
};

const ROWS: CompareRow[] = [
  {
    row: 1,
    id: "10995300315432397",
    ra_deg: 53.2001,
    dec_deg: -27.9002,
    x: null,
    y: null,
    class: "REX",
    flux_cat_jy: 1e-4,
    flux_err_cat_jy: null,
    snr_cat: null,
    flux_native: null,
    flux_err_native: null,
    flux_jy: null,
    flux_err_jy: null,
    ratio: null,
    masked_fraction: null,
    used: false,
    reason: "cut type == PSF",
  },
  {
    row: 2,
    id: "10995300315432398",
    ra_deg: 53.2101,
    dec_deg: -27.9102,
    x: 3333.8115,
    y: 625.4192,
    class: "PSF",
    flux_cat_jy: 2.871799e-4,
    flux_err_cat_jy: 1.0e-7,
    snr_cat: 2871.799,
    flux_native: 79.1,
    flux_err_native: 0.03,
    flux_jy: 2.872e-4,
    flux_err_jy: 1.1e-7,
    ratio: 1.00007,
    masked_fraction: 0,
    used: true,
    reason: null,
  },
  {
    row: 3,
    id: "row 3",
    ra_deg: 53.2201,
    dec_deg: -27.9202,
    x: 10.5,
    y: 20.25,
    class: null,
    flux_cat_jy: 5e-5,
    flux_err_cat_jy: 2e-7,
    snr_cat: 250,
    flux_native: 13.5,
    flux_err_native: 0.05,
    flux_jy: 4.9e-5,
    flux_err_jy: 1.8e-7,
    ratio: 0.98,
    masked_fraction: 0.25,
    used: false,
    reason: "aperture touches pixels without data",
  },
  {
    row: 4,
    id: "a,b",
    ra_deg: 53.2301,
    dec_deg: -27.9302,
    x: 100,
    y: 200,
    class: "PSF",
    flux_cat_jy: 1e-4,
    flux_err_cat_jy: 1e-7,
    snr_cat: 1000,
    flux_native: 27.5,
    flux_err_native: 0.03,
    flux_jy: 9.99e-5,
    flux_err_jy: 1.1e-7,
    ratio: 0.999,
    masked_fraction: 0,
    used: true,
    reason: null,
  },
];

const REPORT: CompareReport = {
  n_table: 69,
  n_on_image: 69,
  n_after_cuts: 21,
  n_measured: 21,
  n_used: 21,
  ratio_median: 1.00007,
  ratio_nmad: 0.00021,
  ratio_median_err: 0.00012,
  n_ratio: 21,
  ratio_offset_mag: -0.000076,
  err_ratio_median: 0.98434,
  err_ratio_nmad: 0.0054,
  n_err: 21,
  implied_zero_point_ab: 22.50008,
  implied_zero_point_nmad: 0.0012,
  n_zero_point: 21,
  rows: ROWS,
  rejected: [
    { reason: "cut type == PSF", count: 33 },
    { reason: "cut maskbits == 0", count: 14 },
    { reason: "cut fracflux_r < 0.01", count: 1 },
  ],
  aperture: { radius_px: 7.633587786259541, radius_arcsec: 2, annulus_px: null, pixel_scale_arcsec: 0.262, local_sky: false },
  band: BAND,
  calibration_origin: "BUNIT nanomaggy, zero point 22.5",
  preset: "legacy_dr10",
  catalogue_band: "r",
  table: {
    path: "C:/cat/C1b_ls_dr10_brick_0532m280.csv",
    file_name: "C1b_ls_dr10_brick_0532m280.csv",
    format: "csv",
    n_rows: 69,
  },
  notes: ["note one", "note two"],
  warnings: ["w1"],
  elapsed_ms: 12,
};

function probeFor(preset: ComparePreset, textColumns: string[] = []): CompareProbe {
  const m = preset.mapping;
  const used = [m.id, m.ra, m.dec, m.flux, m.flux_err, m.flux_ivar, m.class, ...preset.cuts.map((c) => c.column)];
  const names = [...new Set(used.filter((name): name is string => name !== null))];
  return {
    table_path: "C:/cat/t.csv",
    file_name: "t.csv",
    format: "csv",
    n_rows: 3,
    columns: names.map((name) => ({ name, numeric: name !== m.id && !textColumns.includes(name), unit: null })),
    presets: [preset],
    notes: [],
  };
}

const IMG = "C:/d/brick-r.fits";
const REQUEST = compareRequestOf(formFromPreset(LEGACY_R), IMG, probeFor(LEGACY_R, ["type"])).request!;
const RUN: CompareRun = { report: REPORT, request: REQUEST, presetEdited: false };
const SAVED_TO = "C:/out/brick-r_catalog_compare.csv";
const MARKS_LABEL = "Marks on image";
const EXPORTED_LINE = /^# exported_utc: \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/;

type Props = Record<string, unknown> & { children?: ReactNode };
type Element = ReactElement<Props>;
type ViewProps = { run: CompareRun; imagePath: string | null };

const view = (CompareRowsView as unknown as { type: (props: ViewProps) => unknown }).type;
const show = (run: CompareRun = RUN) => hooks.render(view, { run, imagePath: IMG }) as ReactNode;
const all = (tree: ReactNode): Element[] => elementsOf<Props>(tree);
const rowsOf = (tree: ReactNode) => all(tree).filter((e) => e.props["data-row"] !== undefined);
const rowNumbered = (tree: ReactNode, n: number) => rowsOf(tree).find((e) => e.props["data-row"] === n)!;
const cellsOf = (row: Element) => all(row.props.children).filter((e) => e.type === "td").map((e) => textOf(e));
const headOf = (tree: ReactNode) => all(tree).filter((e) => e.type === "th").map((e) => textOf(e));
const withData = (tree: ReactNode, name: string) => all(tree).filter((e) => e.props[name] !== undefined);
const marksToggle = (tree: ReactNode) => all(tree).find((e) => e.props.label === MARKS_LABEL)!;
const lastPainterOptions = () => backend.painter.mock.calls.at(-1)![0] as CatalogPainterOptions;
const runWith = (report: Partial<CompareReport>, presetEdited = false): CompareRun => ({
  report: { ...REPORT, ...report },
  request: REQUEST,
  presetEdited,
});

function copyText(tree: ReactNode): string {
  const button = all(tree).find((e) => typeof e.props.text === "function");
  expect(button, "the copy button").toBeDefined();
  return (button!.props.text as () => string)();
}

function clickSave(tree: ReactNode): void {
  const button = all(tree).find((e) => e.props["data-action"] === "save-compare-csv");
  expect(button, "the save button").toBeDefined();
  (button!.props.onClick as () => void)();
}

const withoutExportLine = (text: string) => text.split("\r\n").filter((line) => !line.startsWith("# exported_utc:"));

describe("CompareRowsView", () => {
  beforeEach(() => {
    hooks.mount();
    live.key = "rk";
    backend.add.mockReset();
    backend.remove.mockReset();
    backend.painter.mockReset().mockReturnValue(() => undefined);
    backend.save.mockReset().mockResolvedValue(SAVED_TO);
  });

  it("builds the request of the fixtures", () => {
    expect(REQUEST).not.toBeNull();
  });

  it("lists the used rows first, each with its catalogue flux, its measured flux and its ratio", () => {
    const tree = show();
    expect(rowsOf(tree).map((r) => r.props["data-row"])).toEqual([2, 4, 1, 3]);
    const second = textOf(rowNumbered(tree, 2));
    expect(second).toContain("10995300315432398");
    expect(second).toContain("PSF");
    expect(second).toContain(fluxCellUjy(2.871799e-4, 1.0e-7));
    expect(second).toContain(fluxCellUjy(2.872e-4, 1.1e-7));
    expect(second).toContain("1.0001");
    const first = cellsOf(rowNumbered(tree, 1));
    expect(first[2]).toBe("REX");
    expect(first[3]).toBe(fluxCellUjy(1e-4, null));
    expect(first[4]).toBe("--");
    expect(first[5]).toBe("--");
    expect(headOf(tree)).toEqual(["row", "id", "class", "catalogue (uJy)", "measured (uJy)", "ratio", "left out because"]);
  });

  it("says why a row was left out, with the share of the aperture without data", () => {
    const tree = show();
    expect(textOf(rowNumbered(tree, 3))).toContain("aperture touches pixels without data (25.0 percent of the aperture)");
    expect(textOf(rowNumbered(tree, 1))).toContain("cut type == PSF");
    expect(reasonText({ reason: "aperture touches pixels without data", masked_fraction: 0.00546 }).endsWith("(0.5 percent of the aperture)")).toBe(true);
    expect(reasonText({ reason: "saturated in the image", masked_fraction: 0 })).toBe("saturated in the image");
    expect(reasonText({ reason: null, masked_fraction: 0 })).toBe("");
    expect(reasonText({ reason: "cut type == PSF", masked_fraction: null })).toBe("cut type == PSF");
  });

  it("shows image units when the image has no calibration in Jy", () => {
    const rows = ROWS.map((r) => (r.row === 2 ? { ...r, flux_jy: null, flux_err_jy: null, ratio: null } : r));
    const tree = show(runWith({ band: { ...BAND, jy_per_native_unit: null }, rows }));
    expect(headOf(tree)).toContain("measured (image units)");
    expect(headOf(tree)).not.toContain("measured (uJy)");
    expect(textOf(rowNumbered(tree, 2))).toContain(formatWithUncertainty(79.1, 0.03));
  });

  it("shows at most 200 rows and says so", () => {
    const many = Array.from({ length: 450 }, (_, i) => ({ ...ROWS[1], row: i + 1, id: `s${i + 1}`, used: i < 300 }));
    const tree = show(runWith({ rows: many }));
    const shown = rowsOf(tree);
    expect(shown).toHaveLength(200);
    expect(shown.every((r) => ((r.props.className as string) ?? "").includes("text-amber-200"))).toBe(true);
    expect(shown.every((r) => (r.props["data-row"] as number) <= 300)).toBe(true);
    expect(withData(tree, "data-rows-note").map((e) => textOf(e))).toEqual([
      "first 200 of 450 rows shown, used rows first; the CSV holds every row",
    ]);

    hooks.mount();
    expect(withData(show(), "data-rows-note")).toHaveLength(0);

    hooks.mount();
    const empty = show(runWith({ rows: [] }));
    expect(all(empty).some((e) => e.type === "table")).toBe(false);
    expect(all(empty).some((e) => typeof e.props.text === "function")).toBe(true);
    expect(all(empty).some((e) => e.props["data-action"] === "save-compare-csv")).toBe(true);
  });

  it("marks the measured rows on the image and removes the marks when hidden or closed", () => {
    const tree = show();
    expect(backend.add).toHaveBeenCalledTimes(1);
    const [key, layer] = backend.add.mock.calls[0];
    expect(key).toBe("rk");
    expect(layer.id).toBe("catalog-compare");
    expect(layer.kind).toBe(CATALOG_LAYER_KIND);
    expect(layer.visible).toBe(true);
    const options = lastPainterOptions();
    expect(options.rows.map((r) => r.id)).toEqual(["10995300315432398", "row 3", "a,b"]);
    expect([...options.matchedIds]).toEqual(["10995300315432398", "a,b"]);
    expect(options.labels).toBe(false);
    expect(options.highlightId).toBeNull();
    expect(withData(tree, "data-marks-legend")).toHaveLength(1);
    expect(marksToggle(tree).props.disabled).toBe(false);

    (marksToggle(tree).props.onChange as (v: boolean) => void)(false);
    const hidden = show();
    expect(backend.remove).toHaveBeenCalledWith("rk", "catalog-compare");
    expect(withData(hidden, "data-marks-legend")).toHaveLength(0);

    hooks.mount();
    backend.remove.mockReset();
    show();
    hooks.mount();
    expect(backend.remove).toHaveBeenCalledWith("rk", "catalog-compare");
  });

  it("highlights the mark of the row under the pointer", () => {
    const tree = show();
    (rowNumbered(tree, 2).props.onMouseEnter as () => void)();
    const hovered = show();
    expect(lastPainterOptions().highlightId).toBe("10995300315432398");
    expect(rowNumbered(hovered, 2).props.className).toContain("bg-zinc-800/80");
    (rowNumbered(hovered, 2).props.onMouseLeave as () => void)();
    show();
    expect(lastPainterOptions().highlightId).toBeNull();
  });

  it("draws nothing without an image key or without a measured row", () => {
    live.key = null;
    const noKey = show();
    expect(backend.add).not.toHaveBeenCalled();
    expect(marksToggle(noKey).props.disabled).toBe(true);

    hooks.mount();
    live.key = "rk";
    const unplaced = show(runWith({ rows: [ROWS[0]] }));
    expect(backend.add).not.toHaveBeenCalled();
    expect(marksToggle(unplaced).props.disabled).toBe(true);
    expect(withData(unplaced, "data-marks-legend")).toHaveLength(0);
  });

  it("copies the CSV with its header lines", () => {
    const text = copyText(show());
    const expected = compareCsvText(REPORT, compareCsvHeaderLines(REPORT, { exportedUtc: "x", request: REQUEST, presetEdited: false }));
    expect(withoutExportLine(text)).toEqual(withoutExportLine(expected));
    expect(text.split("\r\n").filter((line) => EXPORTED_LINE.test(line))).toHaveLength(1);

    hooks.mount();
    const edited = copyText(show(runWith({}, true)));
    expect(edited.split("\r\n")).toContain("# preset: legacy_dr10 (edited)");
  });

  it("saves the CSV under the name of the image and says where", async () => {
    clickSave(show());
    await vi.waitFor(() => expect(textOf(show())).toContain(`Saved ${SAVED_TO}`));
    expect(backend.save).toHaveBeenCalledTimes(1);
    const [text, name, title] = backend.save.mock.calls[0];
    expect(text.split("\r\n")[0].startsWith("# software: AstroBurst ")).toBe(true);
    expect(name).toBe("brick-r_catalog_compare.csv");
    expect(title).toBe("Save the catalogue comparison");

    hooks.mount();
    backend.save.mockReset().mockResolvedValue(null);
    clickSave(show());
    await vi.waitFor(() => expect(backend.save).toHaveBeenCalledTimes(1));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(textOf(show())).not.toContain("Saved");

    hooks.mount();
    backend.save.mockReset().mockRejectedValue(new Error("disk full"));
    clickSave(show());
    await vi.waitFor(() => expect(all(show()).find((e) => e.type === ErrorAlert)?.props.message).toBe("disk full"));
  });
});
