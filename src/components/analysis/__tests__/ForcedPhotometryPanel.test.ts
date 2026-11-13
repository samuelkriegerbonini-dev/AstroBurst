import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";

const backend = vi.hoisted(() => ({
  run: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  cancel: vi.fn<() => Promise<boolean>>(),
}));
const filesState = vi.hoisted(() => ({ current: [] as Record<string, unknown>[] }));
const progressState = vi.hoisted(() => ({ current: {} as Record<string, unknown> }));
const regionKey = vi.hoisted(() => ({ current: null as string | null }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/forcedPhotometry", () => ({ forcedPhotometry: backend.run, cancelForcedPhotometry: backend.cancel }));
vi.mock("../../../hooks/useFileStore", () => ({ useDoneFiles: () => filesState.current }));
vi.mock("../../../hooks/useProgress", () => ({ useProgress: () => progressState.current }));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => regionKey.current }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { forcedPhotometryStore } from "../../../hooks/useForcedPhotometryStore";
import { forcedRequest } from "../../../utils/forcedPhotometryRequest";
import { overlayStore } from "../../../utils/overlayStore";
import { measurementLog } from "../../../utils/measurementLog";
import { forcedPhotometryEntry } from "../../../utils/measurementLogForced";
import type { ForcedBand, ForcedCell, ForcedPhotometryResult } from "../../../shared/types/forcedPhotometry";
import { ErrorAlert, RunButton, WarningList } from "../../ui";
import Toggle from "../../ui/Toggle";
import ForcedPhotometryInputs from "../ForcedPhotometryInputs";
import ForcedBandCards from "../ForcedBandCards";
import ForcedPhotometryTable from "../ForcedPhotometryTable";
import ForcedSourceDetail from "../ForcedSourceDetail";

const { ForcedPhotometryPanelInner } = await import("../ForcedPhotometryPanel");
const { runLabel, runSummary, progressText } = await import("../forcedPhotometryPanelModel");

type Props = Record<string, unknown> & { children?: ReactNode };
type Element = ReactElement<Props>;

const INTRO =
  "Measure the same sky positions in several band images with one aperture. Each band is measured on its own pixels through its own WCS; positions are not recentred.";
const STALE = "The fields changed after this run: the results below belong to the earlier request.";
const OVERLAY_HINT = "Select one of the band images in Files to see its apertures on the image.";
const SELECT_HINT = "Select a row to see the SED of that source.";
const LEGACY_RADIUS_PX = 7.633587786259541;

const file = (name: string) => ({ id: `id-${name}`, name, path: `C:/d/${name}`, status: "done" });

const band = (index: number, name: string): ForcedBand => ({
  index,
  path: `C:/d/${name}`,
  file_name: name,
  label: name,
  error: null,
  filter: null,
  photcal: null,
  calibration_origin: null,
  jy_per_native_unit: null,
  bunit: null,
  pixel_scale_arcsec: 0.262,
  aperture_radius_px: LEGACY_RADIUS_PX,
  annulus_px: null,
  aperture_correction: null,
  err_origin: null,
  dq_source: null,
  f_corr: null,
  n_measured: 2,
  n_failed: 0,
  warnings: [],
});

const cell = (x: number, y: number): ForcedCell => ({
  x,
  y,
  on_image: true,
  error: null,
  flux_native: 10,
  flux_err_native: 1,
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
  flags_or: null,
  saturated: false,
  fwhm_px: null,
});

const RESULT: ForcedPhotometryResult = {
  bands: [band(0, "brick-g.fits"), band(1, "brick-r.fits")],
  sources: [
    { id: "a", ra_deg: 53.1, dec_deg: -28.1, cells: [cell(100.25, 200.5), cell(101.5, 202)] },
    { id: "b", ra_deg: 53.2, dec_deg: -28.2, cells: [cell(300, 400.75), cell(301, 402.5)] },
  ],
  aperture: { radius_arcsec: 1, correction: "none" },
  limit_sigma: 3,
  elapsed_ms: 420,
  warnings: ["1 of 2 bands have no identified filter: they stay off the SED"],
};

const idleProgress = () => ({ current: 0, total: 0, percent: 0, stage: "", active: false, reset: () => {} });

const show = (filePath: string | null = null) =>
  elementsOf<Props>(hooks.render(ForcedPhotometryPanelInner, { filePath })) as Element[];
const ofType = (shown: Element[], type: unknown) => shown.find((e) => e.type === type);
const withData = (shown: Element[], name: string) => shown.find((e) => name in e.props);
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const overlay = { add: vi.spyOn(overlayStore, "add"), remove: vi.spyOn(overlayStore, "remove") };
const append = vi.spyOn(measurementLog, "append");
const logButton = (shown: Element[]) => shown.find((e) => e.props["data-action"] === "log-forced");

function ready() {
  for (const f of filesState.current) forcedPhotometryStore.addBand({ name: f.name as string, path: f.path as string });
  forcedPhotometryStore.setFields({ positionsText: "53.1 -28.1 a\n53.2 -28.2 b" });
}

async function runOnce() {
  ready();
  backend.run.mockResolvedValue(RESULT);
  (ofType(show(), RunButton)!.props.onClick as () => void)();
  await tick();
}

describe("ForcedPhotometryPanel", () => {
  beforeEach(() => {
    hooks.mount();
    forcedPhotometryStore.reset();
    backend.run.mockReset();
    backend.cancel.mockReset().mockResolvedValue(true);
    filesState.current = [file("brick-g.fits"), file("brick-r.fits")];
    progressState.current = idleProgress();
    regionKey.current = "C:/d/brick-r.fits";
    overlay.add.mockClear();
    overlay.remove.mockClear();
  });

  afterEach(() => {
    hooks.mount();
    forcedPhotometryStore.reset();
  });

  it("the stub is replaced by a panel that is there without a file", () => {
    const shown = show(null);
    expect(withData(shown, "data-forced-panel")).toBeDefined();
    expect(shown.some((e) => textOf(e.props.children) === "Multi-band")).toBe(true);
    expect(shown.some((e) => e.props.children === INTRO)).toBe(true);
    const inputs = ofType(shown, ForcedPhotometryInputs);
    expect(inputs).toBeDefined();
    expect(inputs!.props.files).toEqual([
      { name: "brick-g.fits", path: "C:/d/brick-g.fits" },
      { name: "brick-r.fits", path: "C:/d/brick-r.fits" },
    ]);
    expect(inputs!.props.fields).toBe(forcedPhotometryStore.getSnapshot().fields);
    expect(inputs!.props.disabled).toBe(false);
  });

  it("says why a run cannot start", () => {
    const shown = show();
    const button = ofType(shown, RunButton)!;
    expect(button.props.disabled).toBe(true);
    expect(button.props.label).toBe("Measure 0 positions in 0 bands");
    const blocked = withData(shown, "data-run-blocked");
    expect(blocked).toBeDefined();
    expect(textOf(blocked!.props.children)).toBe("Choose at least one band image.");
    expect(blocked!.props.id).toBe(button.props.describedBy);
  });

  it("runs the request of the fields and shows the summary", async () => {
    ready();
    const before = ofType(show(), RunButton)!;
    expect(before.props.label).toBe("Measure 2 positions in 2 bands");
    expect(before.props.disabled).toBe(false);
    backend.run.mockResolvedValue(RESULT);
    (before.props.onClick as () => void)();
    await tick();
    const shown = show();
    expect(backend.run).toHaveBeenCalledTimes(1);
    expect(backend.run.mock.calls[0][0]).toEqual(forcedRequest(forcedPhotometryStore.getSnapshot().fields).request);
    expect(textOf(withData(shown, "data-forced-summary")?.props.children)).toBe("2 positions x 2 bands in 0.4 s, limits at 3 sigma");
    expect(ofType(shown, WarningList)!.props.warnings).toEqual(RESULT.warnings);
    const cards = ofType(shown, ForcedBandCards)!;
    expect(cards.props.result).toBe(RESULT);
    expect(cards.props.currentPath).toBeNull();
    expect(withData(shown, "data-run-blocked")).toBeUndefined();
    expect(withData(shown, "data-forced-stale")).toBeUndefined();
  });

  it("shows the band being measured and cancels through the store", () => {
    ready();
    backend.run.mockReturnValue(new Promise(() => {}));
    (ofType(show(), RunButton)!.props.onClick as () => void)();
    progressState.current = { current: 1, total: 2, percent: 50, stage: "brick-g.fits", active: true, reset: () => {} };
    const shown = show();
    expect(textOf(withData(shown, "data-forced-progress")?.props.children)).toContain("1 of 2 bands measured, last brick-g.fits");
    expect(ofType(shown, RunButton)!.props.running).toBe(true);
    expect(ofType(shown, ForcedPhotometryInputs)!.props.disabled).toBe(true);
    const cancel = shown.find((e) => e.props["aria-label"] === "Cancel the multi-band measurement");
    expect(cancel).toBeDefined();
    expect(textOf(cancel!.props.children)).toBe("Cancel");
    (cancel!.props.onClick as () => void)();
    expect(backend.cancel).toHaveBeenCalledTimes(1);

    progressState.current = { ...idleProgress(), current: 0, total: 0, stage: "brick-g.fits", active: true };
    expect(textOf(withData(show(), "data-forced-progress")?.props.children)).toContain("starting");
  });

  it("shows the failure of the backend as it is", async () => {
    const message =
      "an aperture radius in pixels needs bands of one pixel scale: brick-g.fits has 0.2620 and cut.fits has 0.2500 arcsec/px; give the radius in arcsec";
    ready();
    backend.run.mockRejectedValue(new Error(message));
    (ofType(show(), RunButton)!.props.onClick as () => void)();
    await tick();
    const shown = show();
    expect(ofType(shown, ErrorAlert)!.props.message).toBe(message);
    expect(withData(shown, "data-forced-summary")).toBeUndefined();
    expect(withData(shown, "data-forced-progress")).toBeUndefined();
  });

  it("marks a result as stale when the fields change after the run", async () => {
    await runOnce();
    forcedPhotometryStore.setFields({ radius: 2.5 });
    const shown = show();
    const stale = withData(shown, "data-forced-stale");
    expect(stale).toBeDefined();
    expect(textOf(stale!.props.children)).toBe(STALE);
    expect(textOf(withData(shown, "data-forced-summary")?.props.children)).toBe("2 positions x 2 bands in 0.4 s, limits at 3 sigma");
  });

  it("passes the image on screen to the band cards", async () => {
    await runOnce();
    expect(ofType(show("C:/d/brick-r.fits"), ForcedBandCards)!.props.currentPath).toBe("C:/d/brick-r.fits");
  });

  it("shows the table of the result under the band cards", async () => {
    expect(ofType(show("C:/d/brick-r.fits"), ForcedPhotometryTable)).toBeUndefined();
    await runOnce();
    let shown = show("C:/d/brick-r.fits");
    const table = ofType(shown, ForcedPhotometryTable)!;
    expect(table).toBeDefined();
    expect(table.props.result).toBe(RESULT);
    expect(table.props.selected).toBeNull();
    expect(table.props.currentPath).toBe("C:/d/brick-r.fits");
    expect(shown.indexOf(table)).toBeGreaterThan(shown.indexOf(ofType(shown, ForcedBandCards)!));
    forcedPhotometryStore.select(1);
    shown = show("C:/d/brick-r.fits");
    expect(ofType(shown, ForcedPhotometryTable)!.props.selected).toBe(1);
  });

  it("draws the apertures of the band on screen and removes them on cleanup", async () => {
    await runOnce();
    let shown = show("C:/d/brick-r.fits");
    expect(overlay.add).toHaveBeenCalledTimes(1);
    const [key, layer] = overlay.add.mock.calls[0];
    expect(key).toBe("C:/d/brick-r.fits");
    expect(layer).toEqual({ id: "forced-apertures", kind: "apertures", visible: true, paint: expect.any(Function) });
    const arcs: number[][] = [];
    const ctx = new Proxy({} as Record<string | symbol, unknown>, {
      get: (target, name) => (name === "arc" ? (...args: number[]) => void arcs.push(args) : (target[name] ?? (() => {}))),
      set: (target, name, value) => Reflect.set(target, name, value),
    });
    layer.paint({ ctx: ctx as unknown as CanvasRenderingContext2D, toScreen: (p) => p, width: 1000, height: 1000, screenPxPerImagePx: 1 });
    expect(arcs.map(([x, y, r]) => [x, y, r])).toEqual([
      [101.5, 202, LEGACY_RADIUS_PX],
      [101.5, 202, 0.5],
      [101.5, 202, 0.5],
      [301, 402.5, LEGACY_RADIUS_PX],
      [301, 402.5, 0.5],
      [301, 402.5, 0.5],
    ]);
    const toggle = ofType(shown, Toggle)!;
    expect(toggle).toBeDefined();
    expect(toggle.props.label).toBe("Show apertures");
    expect(toggle.props.accent).toBe("amber");
    expect(toggle.props.checked).toBe(true);
    expect(toggle.props.disabled).toBe(false);
    expect(withData(shown, "data-forced-overlay-hint")).toBeUndefined();
    expect(overlay.remove).not.toHaveBeenCalled();
    hooks.mount();
    expect(overlay.remove).toHaveBeenCalledWith("C:/d/brick-r.fits", "forced-apertures");
    expect(overlayStore.has("C:/d/brick-r.fits", "forced-apertures")).toBe(false);

    shown = show("C:/d/brick-r.fits");
    expect(overlayStore.has("C:/d/brick-r.fits", "forced-apertures")).toBe(true);
    overlay.add.mockClear();
    overlay.remove.mockClear();
    (ofType(shown, Toggle)!.props.onChange as (on: boolean) => void)(false);
    shown = show("C:/d/brick-r.fits");
    expect(overlay.remove).toHaveBeenCalledWith("C:/d/brick-r.fits", "forced-apertures");
    expect(overlay.add).not.toHaveBeenCalled();
    expect(ofType(shown, Toggle)!.props.checked).toBe(false);
    expect(overlayStore.has("C:/d/brick-r.fits", "forced-apertures")).toBe(false);
  });

  it("no aperture is drawn on an image that is not one of the bands", async () => {
    await runOnce();
    for (const filePath of ["C:/d/other.fits", null]) {
      const shown = show(filePath);
      expect(overlay.add).not.toHaveBeenCalled();
      expect(ofType(shown, Toggle)).toBeUndefined();
      expect(textOf(withData(shown, "data-forced-overlay-hint")?.props.children)).toBe(OVERLAY_HINT);
    }
  });

  it("the selected source is shown under the table", async () => {
    await runOnce();
    let shown = show("C:/d/brick-r.fits");
    expect(textOf(withData(shown, "data-forced-select-hint")?.props.children)).toBe(SELECT_HINT);
    expect(ofType(shown, ForcedSourceDetail)).toBeUndefined();
    expect(overlay.add).toHaveBeenCalledTimes(1);
    forcedPhotometryStore.select(1);
    shown = show("C:/d/brick-r.fits");
    const detail = ofType(shown, ForcedSourceDetail)!;
    expect(detail).toBeDefined();
    expect(detail.props.result).toBe(RESULT);
    expect(detail.props.source).toBe(1);
    expect(detail.props.currentPath).toBe("C:/d/brick-r.fits");
    expect(shown.indexOf(detail)).toBeGreaterThan(shown.indexOf(ofType(shown, ForcedPhotometryTable)!));
    expect(withData(shown, "data-forced-select-hint")).toBeUndefined();
    expect(overlay.add).toHaveBeenCalledTimes(2);
    expect(overlay.remove).toHaveBeenCalledTimes(1);
  });

  it("the Log button writes the run of the result on screen, with the selected source", async () => {
    append.mockClear();
    expect(logButton(show())).toBeUndefined();
    await runOnce();
    const fieldsAtRunTime = forcedPhotometryStore.getSnapshot().fields;
    const runRequest = forcedRequest(fieldsAtRunTime).request!;
    let button = logButton(show())!;
    expect(button).toBeDefined();
    expect(textOf(button.props.children)).toBe("Log");
    expect(button.props.title).toBe(
      "Write this run to the measurement log: the aperture, the bands and, when a row is selected, the fluxes of that source",
    );
    (button.props.onClick as () => void)();
    expect(append).toHaveBeenCalledTimes(1);
    expect(append.mock.calls[0][0]).toEqual(forcedPhotometryEntry(runRequest, RESULT, null));
    expect(textOf(logButton(show())!.props.children)).toBe("Logged");

    forcedPhotometryStore.setFields({ radius: 2.5 });
    expect(textOf(logButton(show())!.props.children)).toBe("Logged");
    forcedPhotometryStore.select(1);
    button = logButton(show())!;
    expect(textOf(button.props.children)).toBe("Log");
    (button.props.onClick as () => void)();
    expect(append).toHaveBeenCalledTimes(2);
    expect(append.mock.calls[1][0]).toEqual(forcedPhotometryEntry(runRequest, RESULT, 1));
    const fieldsNow = forcedRequest(forcedPhotometryStore.getSnapshot().fields).request!;
    expect(fieldsNow).not.toEqual(runRequest);
    expect(append.mock.calls[1][0]).not.toEqual(forcedPhotometryEntry(fieldsNow, RESULT, 1));
    expect(textOf(logButton(show())!.props.children)).toBe("Logged");
  });

  it("the three texts", () => {
    expect(runLabel(1, 1)).toBe("Measure 1 position in 1 band");
    expect(runLabel(69, 3)).toBe("Measure 69 positions in 3 bands");
    expect(runLabel(0, 0)).toBe("Measure 0 positions in 0 bands");
    expect(runSummary({ ...RESULT, elapsed_ms: 1234, limit_sigma: 5 })).toBe("2 positions x 2 bands in 1.2 s, limits at 5 sigma");
    expect(progressText({ current: 2, total: 3, stage: "complete" })).toBe("starting");
    expect(progressText({ current: 0, total: 3, stage: "" })).toBe("starting");
    expect(progressText({ current: 1, total: 1, stage: "r.fits" })).toBe("1 of 1 band measured, last r.fits");
  });
});
