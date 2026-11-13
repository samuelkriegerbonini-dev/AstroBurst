import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import type { CompareBandInfo, ComparePreset, CompareProbe, CompareReport, CompareRow } from "../../../shared/types/catalogCompare";

const backend = vi.hoisted(() => ({
  open: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  probe: vi.fn<(path: string) => Promise<unknown>>(),
  compare: vi.fn<(request: unknown) => Promise<unknown>>(),
  clearRoute: vi.fn<(kind: string) => void>(),
  append: vi.fn<(draft: unknown) => unknown>(),
}));
const live = vi.hoisted(() => ({
  route: null as Record<string, unknown> | null,
  prov: { file: "brick-r.fits", image: "original" },
  filters: {} as Record<string, string>,
  statusCalls: [] as unknown[][],
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/catalogCompare", () => ({ probeCatalogueTable: backend.probe, compareWithCatalogue: backend.compare }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: backend.open }));
vi.mock("../../../hooks/useTableRouteStore", () => ({
  useTableRoute: () => live.route,
  tableRouteStore: { clear: backend.clearRoute },
}));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => live.prov }));
vi.mock("../../../hooks/usePhotometryStatus", () => ({
  usePhotometryStatus: (...args: unknown[]) => {
    live.statusCalls.push(args);
    const filter = typeof args[0] === "string" ? live.filters[args[0]] : undefined;
    return {
      loading: false,
      error: null,
      status: filter === undefined ? null : { filter: { dataset: "DECAM", filter, pivot_um: 0.64, source: "header FILTER" } },
    };
  },
}));
vi.mock("../../../utils/measurementLog", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../utils/measurementLog")>()),
  measurementLog: { append: backend.append },
}));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { compareRequestOf, formFromPreset, type CompareForm, type CompareRun } from "../../../utils/catalogCompare";
import { catalogCompareEntry } from "../../../utils/measurementLogCompare";
import { ErrorAlert } from "../../ui";
import CompareSummary from "../CompareSummary";
import StaleResultNote from "../StaleResultNote";

const { default: CatalogComparePanel } = await import("../CatalogComparePanel");

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

const LEGACY_G: ComparePreset = {
  ...LEGACY_R,
  band: "g",
  mapping: { ...LEGACY_R.mapping, flux: "apflux_g_5", flux_ivar: "apflux_ivar_g_5" },
  cuts: [
    { column: "type", op: "==", value: "PSF" },
    { column: "maskbits", op: "==", value: 0 },
    { column: "fracflux_g", op: "<", value: 0.01 },
  ],
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

const numeric = (name: string) => ({ name, numeric: true, unit: null });
const text = (name: string) => ({ name, numeric: false, unit: null });

const PROBE: CompareProbe = {
  table_path: "C:/cat/c1b.csv",
  file_name: "c1b.csv",
  format: "csv",
  n_rows: 69,
  columns: [
    text("ls_id"),
    numeric("ra"),
    numeric("dec"),
    text("type"),
    numeric("maskbits"),
    numeric("fracflux_g"),
    numeric("fracflux_r"),
    numeric("apflux_g_5"),
    numeric("apflux_ivar_g_5"),
    numeric("apflux_r_5"),
    numeric("apflux_ivar_r_5"),
  ],
  presets: [{ ...LEGACY_G, notes: ["apflux_g_5: circular aperture of 2.0 arcsec radius, a plain sum with no local sky"] }, LEGACY_R],
  notes: [],
};

const IMG = "C:/d/brick-r.fits";
const PROV = live.prov;
const REQUEST = compareRequestOf(formFromPreset(LEGACY_R), IMG, PROBE).request;

const INTRO_PART = "The app sends no request";
const NO_IMAGE_REASON = "Select an image first: the comparison measures its pixels at the catalogue positions.";
const NO_TABLE_REASON = "Choose a catalogue table saved as CSV or ECSV.";
const EDITED_NOTE = "Edited: the values below differ from the preset.";
const COMPARE_LABEL = "Compare with the catalogue";

type Props = Record<string, unknown> & { children?: ReactNode };
type Element = ReactElement<Props>;
type PanelProps = { filePath: string | null; measureKey?: string | null };

const panel = (CatalogComparePanel as unknown as { type: (props: PanelProps) => unknown }).type;
const show = (props: PanelProps = { filePath: IMG, measureKey: "v1" }) => hooks.render(panel, props) as ReactNode;
const all = (tree: ReactNode = show()): Element[] => elementsOf<Props>(tree);
const withProp = (name: string, tree?: ReactNode) => all(tree).filter((e) => e.props[name] !== undefined);
const byLabel = (label: string, tree?: ReactNode) => all(tree).find((e) => e.props.label === label);
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function run(label: string, tree?: ReactNode): void {
  const button = byLabel(label, tree);
  expect(button, `button ${label}`).toBeDefined();
  (button!.props.onClick as () => void)();
}

function fields(tree?: ReactNode): Props {
  const element = all(tree).find((e) => e.props.form !== undefined && Array.isArray(e.props.columns));
  expect(element, "the form fields").toBeDefined();
  return element!.props;
}

const formOf = (tree?: ReactNode) => fields(tree).form as CompareForm;
const edit = (change: Partial<CompareForm>) => (fields().onChange as (next: CompareForm) => void)({ ...formOf(), ...change });

function presetSelect(tree?: ReactNode): Props {
  const select = all(tree).find((e) => e.props["data-field"] === "preset");
  expect(select, "the preset select").toBeDefined();
  return select!.props;
}

const choosePreset = (value: string) => (presetSelect().onChange as (e: { target: { value: string } }) => void)({ target: { value } });
const summaries = (tree?: ReactNode) => all(tree).filter((e) => e.type === CompareSummary);
const textNow = () => textOf(show());

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

async function opened(): Promise<void> {
  show();
  run("Choose table");
  await vi.waitFor(() => expect(textNow()).toContain("c1b.csv: 69 rows, 11 columns (CSV)"));
}

async function compared(): Promise<void> {
  run(COMPARE_LABEL);
  await vi.waitFor(() => expect(summaries(show())).toHaveLength(1));
}

describe("CatalogComparePanel", () => {
  beforeEach(() => {
    hooks.mount();
    live.route = null;
    live.filters = {};
    live.statusCalls = [];
    backend.open.mockReset().mockResolvedValue("C:/cat/c1b.csv");
    backend.probe.mockReset().mockResolvedValue(PROBE);
    backend.compare.mockReset().mockResolvedValue(REPORT);
    backend.clearRoute.mockReset();
    backend.append.mockReset();
  });

  it("builds the request of the fixtures", () => {
    expect(REQUEST).not.toBeNull();
  });

  it("says why nothing can run without an image or a table", () => {
    const noImage = show({ filePath: null });
    expect(byLabel(COMPARE_LABEL, noImage)?.props.disabled).toBe(true);
    expect(textOf(noImage)).toContain(NO_IMAGE_REASON);
    const noTable = show();
    expect(byLabel(COMPARE_LABEL, noTable)?.props.disabled).toBe(true);
    expect(textOf(noTable)).toContain(NO_TABLE_REASON);
    const intro = all(noTable).find((e) => e.props["data-compare-intro"] !== undefined);
    expect(intro).toBeDefined();
    expect(textOf(intro)).toContain(INTRO_PART);
  });

  it("opens the table the user picks and starts from its first preset", async () => {
    await opened();
    expect(backend.open).toHaveBeenCalledTimes(1);
    expect((backend.open.mock.calls[0][0] as { filters: unknown }).filters).toEqual([{ name: "Catalogue table", extensions: ["csv", "ecsv"] }]);
    expect(backend.probe).toHaveBeenCalledWith("C:/cat/c1b.csv");
    const tree = show();
    const select = presetSelect(tree);
    expect(select.value).toBe("legacy_dr10:g");
    const options = all(select.children as ReactNode).map((e) => textOf(e));
    expect(options).toEqual(["Legacy Surveys DR10, g", "Legacy Surveys DR10, r", "Custom columns"]);
    expect(formOf(tree)).toEqual(formFromPreset(PROBE.presets[0]));
    expect(all(tree).some((e) => e.props["data-compare-intro"] !== undefined)).toBe(false);
    const notes = all(tree).filter((e) => e.props["data-preset-note"] !== undefined);
    expect(notes.map((e) => textOf(e))).toEqual(["apflux_g_5: circular aperture of 2.0 arcsec radius, a plain sum with no local sky"]);

    hooks.mount();
    backend.probe.mockClear();
    backend.open.mockResolvedValue(null);
    show();
    run("Choose table");
    await vi.waitFor(() => expect(backend.open).toHaveBeenCalledTimes(2));
    await flush();
    expect(backend.probe).not.toHaveBeenCalled();
  });

  it("takes the table handed over from Files and clears the route", async () => {
    const route = { kind: "catalogue", seq: 1, path: "C:/cat/x.csv", name: "x.csv", probe: {} };
    live.route = route;
    show();
    expect(backend.probe).toHaveBeenCalledWith("C:/cat/x.csv");
    expect(backend.clearRoute).toHaveBeenCalledTimes(1);
    expect(backend.clearRoute).toHaveBeenCalledWith("catalogue");
    show();
    expect(backend.probe).toHaveBeenCalledTimes(1);
    live.route = { ...route, seq: 2 };
    show();
    expect(backend.probe).toHaveBeenCalledTimes(2);

    hooks.mount();
    backend.probe.mockClear();
    live.route = route;
    show({ filePath: null });
    expect(backend.probe).toHaveBeenCalledTimes(1);
    expect(backend.probe).toHaveBeenCalledWith("C:/cat/x.csv");
  });

  it("choosing another preset refills the fields and keeps the flag switch; custom keeps the values", async () => {
    await opened();
    edit({ excludeDq: true });
    choosePreset("legacy_dr10:r");
    expect(formOf()).toEqual({ ...formFromPreset(LEGACY_R), excludeDq: true });
    choosePreset("custom");
    expect(formOf()).toEqual({ ...formFromPreset(LEGACY_R), excludeDq: true, presetKey: "custom" });
  });

  it("says when the values no longer equal the preset", async () => {
    await opened();
    const editedNote = () => all().filter((e) => e.props["data-preset-edited"] !== undefined);
    expect(editedNote()).toHaveLength(0);
    edit({ radiusText: "3" });
    expect(editedNote().map((e) => textOf(e))).toEqual([EDITED_NOTE]);
    edit({ radiusText: "2.0" });
    expect(editedNote()).toHaveLength(0);
  });

  it("the first problem of the form blocks the run and is shown", async () => {
    await opened();
    expect(byLabel(COMPARE_LABEL)?.props.disabled).toBe(false);
    edit({ radiusText: "" });
    const tree = show();
    expect(byLabel(COMPARE_LABEL, tree)?.props.disabled).toBe(true);
    const reason = all(tree).find((e) => e.props["data-run-blocked"] !== undefined);
    expect(textOf(reason)).toBe("Aperture radius: Enter a number");
    run(COMPARE_LABEL, tree);
    await flush();
    expect(backend.compare).not.toHaveBeenCalled();
  });

  it("runs the comparison with the request of the form, shows the summary and logs the run", async () => {
    await opened();
    const request = compareRequestOf(formOf(), IMG, PROBE).request;
    expect(request).not.toBeNull();
    await compared();
    expect(backend.compare).toHaveBeenCalledTimes(1);
    expect(backend.compare).toHaveBeenCalledWith(request);
    const shown = summaries()[0].props.run as CompareRun;
    expect(shown.report).toBe(REPORT);
    expect(shown.presetEdited).toBe(false);
    expect(backend.append).toHaveBeenCalledTimes(1);
    expect(backend.append).toHaveBeenCalledWith(catalogCompareEntry(PROV, REPORT, { request: request!, presetEdited: false }));

    edit({ radiusText: "3" });
    run(COMPARE_LABEL);
    await vi.waitFor(() => expect(backend.append).toHaveBeenCalledTimes(2));
    expect((summaries()[0].props.run as CompareRun).presetEdited).toBe(true);
  });

  it("a refusal of the command is shown as it came and keeps the earlier result", async () => {
    await opened();
    await compared();
    const message = "column 'flx' is not in the table; it has: ra, dec";
    backend.compare.mockRejectedValue(new Error(message));
    run(COMPARE_LABEL);
    await vi.waitFor(() => expect(all().find((e) => e.type === ErrorAlert)?.props.message).toBe(message));
    expect((summaries()[0].props.run as CompareRun).report).toBe(REPORT);
    expect(backend.append).toHaveBeenCalledTimes(1);
  });

  it("a table that cannot be read leaves no table", async () => {
    const message = "catalogue table cat.fits: only CSV and ECSV tables can be compared; save the catalogue as CSV";
    backend.probe.mockRejectedValue(new Error(message));
    show();
    run("Choose table");
    await vi.waitFor(() => expect(all().find((e) => e.type === ErrorAlert)?.props.message).toBe(message));
    const tree = show();
    expect(all(tree).some((e) => e.props["data-compare-table"] !== undefined)).toBe(false);
    expect(textOf(all(tree).find((e) => e.props["data-run-blocked"] !== undefined))).toBe(NO_TABLE_REASON);
  });

  it("keeps the result and marks it stale when a setting changes", async () => {
    await opened();
    await compared();
    const staleNotes = (tree: ReactNode) => all(tree).filter((e) => e.type === StaleResultNote);
    expect(staleNotes(show())).toHaveLength(0);
    edit({ minSnrText: "50" });
    const changed = show();
    expect(staleNotes(changed)).toHaveLength(1);
    expect(summaries(changed)).toHaveLength(1);
    edit({ minSnrText: "" });
    const back = show();
    expect(staleNotes(back)).toHaveLength(0);
    expect(summaries(back)).toHaveLength(1);
  });

  it("drops the result when the image or its version changes, and keeps the table and the fields", async () => {
    await opened();
    const form = formOf();
    await compared();
    show({ filePath: IMG, measureKey: "v2" });
    const newVersion = show({ filePath: IMG, measureKey: "v2" });
    expect(summaries(newVersion)).toHaveLength(0);
    expect(all(newVersion).some((e) => e.props["data-compare-table"] !== undefined)).toBe(true);
    expect(formOf(newVersion)).toEqual(form);

    run(COMPARE_LABEL, newVersion);
    await vi.waitFor(() => expect(summaries(show({ filePath: IMG, measureKey: "v2" }))).toHaveLength(1));
    show({ filePath: "C:/d/brick-g.fits", measureKey: "v2" });
    const otherImage = show({ filePath: "C:/d/brick-g.fits", measureKey: "v2" });
    expect(summaries(otherImage)).toHaveLength(0);
    expect(all(otherImage).some((e) => e.props["data-compare-table"] !== undefined)).toBe(true);
    expect(formOf(otherImage)).toEqual(form);
  });

  it("a late answer of a superseded run is dropped", async () => {
    await opened();
    const answer = deferred<CompareReport>();
    backend.compare.mockReturnValue(answer.promise);
    run(COMPARE_LABEL);
    show({ filePath: IMG, measureKey: "v2" });
    answer.resolve(REPORT);
    await flush();
    expect(summaries(show({ filePath: IMG, measureKey: "v2" }))).toHaveLength(0);
    expect(backend.append).not.toHaveBeenCalled();

    hooks.mount();
    const first = deferred<CompareProbe>();
    const second = deferred<CompareProbe>();
    backend.open.mockResolvedValueOnce("C:/cat/first.csv").mockResolvedValueOnce("C:/cat/second.csv");
    backend.probe.mockImplementation((path: string) => (path === "C:/cat/first.csv" ? first.promise : second.promise));
    show();
    run("Choose table");
    await vi.waitFor(() => expect(backend.probe).toHaveBeenCalledWith("C:/cat/first.csv"));
    run("Choose table");
    await vi.waitFor(() => expect(backend.probe).toHaveBeenCalledWith("C:/cat/second.csv"));
    second.resolve({ ...PROBE, table_path: "C:/cat/second.csv", file_name: "second.csv" });
    await flush();
    first.resolve(PROBE);
    await flush();
    const tableLine = all().find((e) => e.props["data-compare-table"] !== undefined);
    expect(textOf(tableLine)).toBe("second.csv: 69 rows, 11 columns (CSV)");
  });

  it("shows the rows of a run under its summary", async () => {
    await opened();
    expect(withProp("run")).toHaveLength(0);
    await compared();
    const withRun = withProp("run");
    expect(withRun).toHaveLength(2);
    expect(withRun[0].type).toBe(CompareSummary);
    expect(withRun[1].props.imagePath).toBe(IMG);
    expect(withRun[1].props.run).toBe(withRun[0].props.run);
  });

  it("nothing can be changed while a comparison runs", async () => {
    await opened();
    expect(fields().disabled).toBe(false);
    expect(byLabel("Choose table")?.props.disabled).toBe(false);
    backend.compare.mockReturnValue(deferred<CompareReport>().promise);
    run(COMPARE_LABEL);
    const tree = show();
    expect(fields(tree).disabled).toBe(true);
    expect(byLabel("Choose table", tree)?.props.disabled).toBe(true);
    expect(byLabel(COMPARE_LABEL, tree)?.props.running).toBe(true);
  });

  it("starts from the preset of the band the image names and follows the image until the user chooses", async () => {
    live.filters = { [IMG]: "r", "C:/d/brick-g.fits": "g" };
    await opened();
    expect(live.statusCalls.at(-1)).toEqual([IMG, false, "v1"]);
    expect(presetSelect().value).toBe("legacy_dr10:r");
    expect(formOf()).toEqual(formFromPreset(LEGACY_R));
    const onG = show({ filePath: "C:/d/brick-g.fits", measureKey: "v1" });
    expect(presetSelect(onG).value).toBe("legacy_dr10:g");
    choosePreset("legacy_dr10:r");
    const kept = show({ filePath: "C:/d/brick-g.fits", measureKey: "v1" });
    expect(presetSelect(kept).value).toBe("legacy_dr10:r");
    expect(formOf(kept)).toEqual(formFromPreset(LEGACY_R));

    hooks.mount();
    await opened();
    edit({ radiusText: "3" });
    const edited = show({ filePath: "C:/d/brick-g.fits", measureKey: "v1" });
    expect(presetSelect(edited).value).toBe("legacy_dr10:r");
    expect(formOf(edited).radiusText).toBe("3");
    run("Choose table", edited);
    await flush();
    await flush();
    expect(presetSelect(show({ filePath: "C:/d/brick-g.fits", measureKey: "v1" })).value).toBe("legacy_dr10:g");
  });
});
