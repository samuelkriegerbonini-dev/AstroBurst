import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const backend = vi.hoisted(() => ({
  fit: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  save: vi.fn<(text: string, defaultPath: string, title: string) => Promise<string | null>>(),
  copy: vi.fn<(text: string) => Promise<void>>(),
}));

vi.mock("react", async (io) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await io()));
vi.mock("../../../services/transit", () => ({ fitTransit: backend.fit }));
vi.mock("../../../utils/spectrumExport", async (io) => ({ ...(await io()), saveCsvDialog: backend.save }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { lightCurveStore } from "../../../hooks/useLightCurveStore";
import { EMPTY_TRANSIT_FIT, transitFitStore } from "../../../hooks/useTransitFitStore";
import { lightCurveModel, type LightCurveModel } from "../../../utils/lightCurveViewModel";
import {
  DEFAULT_TRANSIT_FIT_SETUP,
  setupWithStart,
  transitFitInput,
  transitFitRequest,
  transitFitSignature,
  transitStartFromQuickLook,
  type TransitFitSetup,
} from "../../../utils/transitFitSetup";
import {
  ECLIPSE_FIT_LABEL,
  TRANSIT_FIT_LABEL,
  transitFitCsvText,
  transitFitEntry,
  transitPlotSeries,
  transitResultRows,
} from "../../../utils/transitFitReport";
import { formatWithUncertainty } from "../../../utils/formatUncertainty";
import { measurementLog } from "../../../utils/measurementLog";
import { DEFAULT_LIGHT_CURVE_VIEW, type LightCurveDoc, type LightCurveViewState } from "../../../shared/types/lightCurve";
import type { TransitFitRequest, TransitFitResult } from "../../../shared/types/transit";
import ProfilePlot from "../../regions/ProfilePlot";
import ErrorAlert from "../../ui/ErrorAlert";
import CopyCsvButton from "../../ui/CopyCsvButton";
import type { TransitFitSectionProps } from "../TransitFitSection";

const { default: TransitFitSection } = await import("../TransitFitSection");

type Props = Record<string, unknown>;

const G = "table:C:/d/t.csv";

const DOC: LightCurveDoc = {
  id: "white",
  label: "white",
  quantity: "flux",
  unit: "Jy",
  timeScale: "bjd_tdb",
  time: Array.from({ length: 40 }, (_, i) => 2460310 + 0.01 * i),
  value: Array.from({ length: 40 }, (_, i) => 1000 - (i >= 18 && i <= 22 ? 10 : 0)),
  err: Array.from({ length: 40 }, () => 0.3),
  pointRef: Array.from({ length: 40 }, (_, i) => i),
  flags: Array.from({ length: 40 }, () => []),
  source: { kind: "table", path: "C:/d/t.csv", time_column: "t", value_column: "f" },
  notes: [],
};

const FULL: TransitFitSetup = {
  ...DEFAULT_TRANSIT_FIT_SETUP,
  period: "3",
  t0: { text: "0.2", free: true },
  rp: { text: "0.1", free: true },
  a_rs: { text: "9", free: true },
  b: { text: "0.3", free: true },
};

const RES: TransitFitResult = {
  model: "Mandel and Agol 2002, quadratic limb darkening, circular orbit",
  period: 3,
  parameters: [
    { name: "t0", value: 2460310.4999394, err: 0.0000828, free: true, at_bound: false },
    { name: "rp", value: 0.09982, err: 0.00048, free: true, at_bound: false },
    { name: "a_rs", value: 9, err: null, free: false, at_bound: false },
    { name: "b", value: 0.3, err: null, free: false, at_bound: false },
    { name: "u1", value: 0.4, err: null, free: false, at_bound: false },
    { name: "u2", value: 0.25, err: null, free: false, at_bound: false },
  ],
  baseline: { order: 1, reference_time: 2460310.4995, coefficients: [999.98, 19.92], errors: [0.026, 0.29] },
  depth: 0.009964,
  depth_err: 0.000095,
  chi2: 195.3,
  dof: 234,
  reduced_chi2: 0.8346,
  weighted: true,
  error_scale: 1,
  rms: 0.27,
  rms_ppm: 270.7,
  n_points: 3,
  n_in_transit: 1,
  iterations: 12,
  converged: true,
  free_names: ["t0", "rp", "c0", "c1"],
  time: [2460310.4, 2460310.5, 2460310.6],
  model_flux: [998, 988, 1002],
  baseline_flux: [998, 1000, 1002],
  residual: [0.5, -1, 0.25],
  notes: ["note a"],
  warnings: ["warn b"],
  correlation: null,
  derived: {
    inclination_deg: 88.09,
    inclination_err: null,
    duration_t14_days: 0.1126,
    duration_t14_err: 0.0005,
    duration_t23_days: null,
    duration_t23_err: null,
    stellar_density_cgs: 1.53,
    stellar_density_err: null,
  },
  time_averaging: { cadence_days: 0.001, bin_points: [1, 2], n_bins: [240, 120], rms_ppm: [270.7, 190], expected_ppm: [271.3, 192.2], beta: 1 },
  curve_time: [2460310.4, 2460310.6],
  curve_flux: [998, 1002],
  curve_baseline: [998, 1002],
  kind: "transit",
  elapsed_ms: 3,
};

const REQ: TransitFitRequest = {
  kind: "transit",
  time: [2460310.4, 2460310.5, 2460310.6],
  flux: [998.5, 987, 1002.25],
  err: [0.3, 0.3, 0.3],
  period: 3,
  t0: { value: 2460310.5, free: true },
  rp: { value: 0.1, free: true },
  a_rs: { value: 9, free: false },
  b: { value: 0.3, free: false },
  u1: { value: 0.4, free: false },
  u2: { value: 0.25, free: false },
  flux_ratio: null,
  baseline_order: 1,
  scale_errors: false,
};

const section = (TransitFitSection as unknown as { type: (p: TransitFitSectionProps) => unknown }).type;

function model(view: Partial<LightCurveViewState> = {}, doc: LightCurveDoc = DOC): LightCurveModel {
  return lightCurveModel(doc, { ...DEFAULT_LIGHT_CURVE_VIEW, ...view });
}

function render(m: LightCurveModel): unknown {
  return hooks.render(section, { group: G, model: m, csvName: "lightcurve" });
}

function all(tree: unknown): ReactElement<Props>[] {
  return elementsOf<Props>(tree);
}

function field(tree: unknown, text: string): Props | undefined {
  const elements = all(tree);
  const label = elements.find((e) => e.type === "label" && textOf(e.props.children) === text);
  if (label === undefined) return undefined;
  return elements.find((e) => e.type !== "label" && e.props.id === label.props.htmlFor)?.props;
}

function optionsOf(tree: unknown, label: string): string[] {
  return elementsOf<Props>(field(tree, label)?.children)
    .filter((e) => e.type === "option")
    .map((e) => textOf(e.props.children));
}

function choose(tree: unknown, label: string, value: string): void {
  (field(tree, label)!.onChange as (e: unknown) => void)({ target: { value } });
}

function rows(tree: unknown): ReactElement<Props>[] {
  return all(tree).filter((e) => e.props["data-fit-parameter"] !== undefined);
}

function row(tree: unknown, name: string): ReactElement<Props> {
  return rows(tree).find((e) => e.props["data-fit-parameter"] === name)!;
}

function rowField(tree: unknown, name: string): Props {
  return elementsOf<Props>(row(tree, name)).find((e) => typeof e.props["aria-label"] === "string")!.props;
}

function rowToggle(tree: unknown, name: string): Props {
  return elementsOf<Props>(row(tree, name)).find((e) => e.props.label === "free")!.props;
}

function rowLabelCell(tree: unknown, name: string): Props {
  return elementsOf<Props>(row(tree, name)).find((e) => e.type === "td")!.props;
}

function flip(toggle: Props): void {
  (toggle.onChange as (checked: boolean) => void)(!toggle.checked);
}

function marked(tree: unknown, hook: string): ReactElement<Props>[] {
  return all(tree).filter((e) => e.props[hook] !== undefined);
}

function markedText(tree: unknown, hook: string): string | undefined {
  const element = marked(tree, hook)[0];
  return element === undefined ? undefined : textOf(element.props.children);
}

function button(tree: unknown, text: string): Props | undefined {
  return all(tree).find((e) => e.type === "button" && textOf(e.props.children) === text)?.props;
}

function fitButton(tree: unknown): Props {
  return all(tree).find((e) => e.type === "button" && ["Fit", "Fitting..."].includes(textOf(e.props.children)))!.props;
}

function plots(tree: unknown): Props[] {
  return all(tree)
    .filter((e) => e.type === ProfilePlot)
    .map((e) => e.props);
}

function store(setup: TransitFitSetup): void {
  transitFitStore.setSetup(G, "white", setup);
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

function deferred<T>() {
  let resolve: (value: T) => void = () => {};
  let reject: (error: unknown) => void = () => {};
  const promise = new Promise<T>((ok, fail) => {
    resolve = ok;
    reject = fail;
  });
  return { promise, resolve, reject };
}

describe("TransitFitSection", () => {
  beforeEach(() => {
    hooks.mount();
    backend.fit.mockReset();
    backend.save.mockReset();
    backend.copy.mockReset();
    vi.spyOn(measurementLog, "append");
    vi.stubGlobal("navigator", { clipboard: { writeText: backend.copy } });
    transitFitStore.removeGroup(G);
    lightCurveStore.remove(G);
    lightCurveStore.put(G, [DOC]);
  });

  afterEach(() => {
    hooks.mount();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
    transitFitStore.removeGroup(G);
    lightCurveStore.remove(G);
  });

  it("shows the label of the fit, the controls and one row per parameter", () => {
    const m = model();
    const tree = render(m) as ReactElement<Props>;
    expect(tree.type).toBe("details");
    expect(tree.props["data-transit-fit"]).toBeDefined();
    expect(textOf(all(tree).find((e) => e.type === "summary")!.props.children)).toBe("Transit fit");
    expect(markedText(tree, "data-fit-label")).toBe(`Fit: ${TRANSIT_FIT_LABEL}`);
    expect(textOf(tree)).toContain("The quick-look depth above stays the first look. The period is typed and never fitted.");
    expect(field(tree, "fit")!.value).toBe("transit");
    expect(optionsOf(tree, "fit")).toEqual(["transit (planet in front)", "eclipse (planet behind)"]);
    expect(field(tree, "baseline")!.value).toBe("1");
    expect(optionsOf(tree, "baseline")).toEqual(["constant", "straight line", "parabola"]);
    const period = field(tree, "period d")!;
    expect(period.value).toBe("");
    expect(period.placeholder).toBe("days");
    expect(period.min).toBe(0);
    const scale = all(tree).find((e) => e.props.label === "Scale errors by sqrt(reduced chi2)")!.props;
    expect(scale.checked).toBe(false);
    expect(scale.accent).toBe("amber");
    expect(all(tree).filter((e) => e.type === "th").map((e) => textOf(e.props.children))).toEqual(["parameter", "value", "fit"]);
    expect(marked(tree, "data-fit-parameters")[0].type).toBe("table");
    const names = ["t0", "rp", "a_rs", "b", "u1", "u2"];
    expect(rows(tree).map((e) => e.props["data-fit-parameter"])).toEqual(names);
    expect(names.map((name) => textOf(rowLabelCell(tree, name).children))).toEqual([
      "mid-transit time",
      "Rp/Rs",
      "a/Rs",
      "impact parameter b",
      "limb darkening u1",
      "limb darkening u2",
    ]);
    expect(names.map((name) => rowField(tree, name)["aria-label"])).toEqual([
      "mid-transit time",
      "Rp/Rs",
      "a/Rs",
      "impact parameter b",
      "limb darkening u1",
      "limb darkening u2",
    ]);
    expect(rowField(tree, "u1").value).toBe("0");
    expect(rowField(tree, "u2").value).toBe("0");
    expect(names.map((name) => rowToggle(tree, name).checked)).toEqual([true, true, true, true, false, false]);
    expect(rowToggle(tree, "rp").accent).toBe("teal");
    expect(textOf(tree)).toContain("(BJD_TDB - 2460310)");
    expect(textOf(tree)).toContain("never taken from a stellar model");
    const cellId = rowLabelCell(tree, "rp").id as string;
    expect(textOf(rowLabelCell(tree, "rp").children)).toBe("Rp/Rs");
    expect(rowToggle(tree, "rp").describedBy).toBe(cellId);
    const toggleElement = elementsOf<Props>(row(tree, "rp")).find((e) => e.props.label === "free")!;
    expect(renderToStaticMarkup(toggleElement)).toContain(`aria-describedby="${cellId}"`);
    const ids = names.map((name) => rowLabelCell(tree, name).id);
    expect(new Set(ids).size).toBe(6);
    expect(ids.every((id) => typeof id === "string" && id !== "")).toBe(true);
  });

  it("typing a value or switching a parameter writes the setup of this curve", () => {
    const m = model();
    let tree = render(m);
    (field(tree, "period d")!.onChange as (text: string) => void)("3");
    expect(transitFitStore.get(G, "white").setup.period).toBe("3");
    tree = render(m);
    (rowField(tree, "rp").onChange as (text: string) => void)("0.1");
    tree = render(m);
    flip(rowToggle(tree, "a_rs"));
    expect(transitFitStore.get(G, "white").setup.rp).toEqual({ text: "0.1", free: true });
    expect(transitFitStore.get(G, "white").setup.a_rs.free).toBe(false);
    tree = render(m);
    choose(tree, "baseline", "2");
    expect(transitFitStore.get(G, "white").setup.baselineOrder).toBe(2);
    tree = render(m);
    flip(all(tree).find((e) => e.props.label === "Scale errors by sqrt(reduced chi2)")!.props);
    const setup = transitFitStore.get(G, "white").setup;
    expect(setup.scaleErrors).toBe(true);
    expect(setup.period).toBe("3");
    expect(setup.rp).toEqual({ text: "0.1", free: true });
    expect(setup.a_rs.free).toBe(false);
    expect(setup.baselineOrder).toBe(2);
    expect(transitFitStore.get(G, "other")).toBe(EMPTY_TRANSIT_FIT);
  });

  it("the eclipse kind shows the flux ratio in place of limb darkening", () => {
    const m = model();
    choose(render(m), "fit", "eclipse");
    expect(transitFitStore.get(G, "white").setup.kind).toBe("eclipse");
    const tree = render(m);
    expect(field(tree, "fit")!.value).toBe("eclipse");
    expect(rows(tree).map((e) => e.props["data-fit-parameter"])).toEqual(["t0", "rp", "a_rs", "b", "flux_ratio"]);
    expect(textOf(rowLabelCell(tree, "t0").children)).toBe("mid-eclipse time");
    expect(textOf(rowLabelCell(tree, "flux_ratio").children)).toBe("planet/star flux ratio");
    expect(rowField(tree, "flux_ratio").value).toBe("0.001");
    expect(rowToggle(tree, "flux_ratio").checked).toBe(true);
    expect(markedText(tree, "data-fit-label")).toBe(`Fit: ${ECLIPSE_FIT_LABEL}`);
    expect(textOf(tree)).toContain("usually fixed to the values of the transit fit");
    expect(textOf(tree)).not.toContain("never taken from a stellar model");
  });

  it("Fit is refused with the reason until the inputs can be used", () => {
    const m = model();
    let tree = render(m);
    expect(fitButton(tree).disabled).toBe(true);
    expect(fitButton(tree).title).toBe("type the period in days: it is given, never fitted");
    expect(markedText(tree, "data-fit-hint")).toBe("type the period in days: it is given, never fitted");
    (fitButton(tree).onClick as () => void)();

    store({ ...DEFAULT_TRANSIT_FIT_SETUP, period: "3" });
    tree = render(m);
    expect(fitButton(tree).disabled).toBe(true);
    expect(fitButton(tree).title).toBe("mid-transit time needs a value");
    expect(markedText(tree, "data-fit-hint")).toBe("mid-transit time needs a value");
    (fitButton(tree).onClick as () => void)();

    store(FULL);
    tree = render(m);
    expect(fitButton(tree).disabled).toBe(false);
    expect(marked(tree, "data-fit-hint")).toHaveLength(0);

    hooks.mount();
    const indexed = model({}, { ...DOC, timeScale: "index", time: DOC.time.map((_, i) => i) });
    tree = render(indexed);
    expect(fitButton(tree).disabled).toBe(true);
    expect(fitButton(tree).title).toBe("A transit fit needs a time axis.");
    expect(markedText(tree, "data-fit-hint")).toBe("A transit fit needs a time axis.");
    (fitButton(tree).onClick as () => void)();
    expect(backend.fit).not.toHaveBeenCalled();
  });

  it("Fit sends the kept points with the typed values and stores the result with its signature", async () => {
    store(FULL);
    const m = model();
    const pending = deferred<TransitFitResult>();
    backend.fit.mockReturnValueOnce(pending.promise);
    let tree = render(m);
    (fitButton(tree).onClick as () => void)();
    expect(backend.fit).toHaveBeenCalledTimes(1);
    const sent = backend.fit.mock.calls[0][0] as TransitFitRequest;
    expect(sent).toEqual(transitFitRequest(transitFitInput(m), FULL, m.offset).request);
    expect(sent.t0.value).toBeCloseTo(2460310.2, 9);
    tree = render(m);
    expect(textOf(fitButton(tree).children)).toBe("Fitting...");
    expect(fitButton(tree).disabled).toBe(true);
    pending.resolve(RES);
    await settle();
    tree = render(m);
    const record = transitFitStore.get(G, "white");
    expect(record.result).toBe(RES);
    expect(record.request).toBe(sent);
    expect(record.signature).toBe(transitFitSignature(transitFitInput(m)));
    expect(record.fittedUtc).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    expect(record.setup).toEqual(FULL);
    expect(textOf(fitButton(tree).children)).toBe("Fit");
    expect(fitButton(tree).disabled).toBe(false);
  });

  it("the model is drawn over the data and the residuals below", () => {
    const m = model();
    transitFitStore.setResult(G, "white", { request: REQ, result: RES, signature: transitFitSignature(transitFitInput(m)), fittedUtc: "t" });
    const tree = render(m);
    const drawn = plots(tree);
    expect(drawn).toHaveLength(2);
    const expected = transitPlotSeries(RES, { ...REQ, pointIndex: [] }, 2460310)!;
    const [fit, residual] = drawn;
    const fitSeries = fit.series as Props[];
    expect(fitSeries).toHaveLength(2);
    expect(fitSeries[0]).toEqual({ x: expected.time, y: expected.flux, yErr: expected.fluxErr, color: "#fbbf24", label: "data / baseline", mode: "points" });
    expect(fitSeries[1]).toEqual({ x: expected.curveTime, y: expected.curveFlux, color: "#67e8f9", label: "model", mode: "line" });
    expect(fit.xLabel).toBe("BJD_TDB - 2460310");
    expect(fit.yLabel).toBe("flux / fitted baseline");
    expect(fit.height).toBe(170);
    expect(fit.toolbar).toBe(true);
    expect(fit.logToggle).toBe(false);
    expect(fit.wheelZoom).toBe("modifier");
    expect(fit.csvName).toBe("lightcurve_transit_fit");
    const residualSeries = residual.series as Props[];
    expect(residualSeries).toEqual([{ x: expected.time, y: expected.residualPpm, color: "#fbbf24", label: "residual", mode: "points" }]);
    expect(residual.xLabel).toBe("BJD_TDB - 2460310");
    expect(residual.yLabel).toBe("residual (ppm of the baseline)");
    expect(residual.height).toBe(110);
    expect(residual.toolbar).toBe(true);
    expect(residual.logToggle).toBe(false);
    expect(residual.wheelZoom).toBe("modifier");
    expect(residual.referenceLines).toEqual([{ axis: "y", value: 0 }]);
    expect(residual.csvName).toBe("lightcurve_transit_residuals");
    expect(marked(tree, "data-stale-fit")).toHaveLength(0);
  });

  it("a fit is marked when the curve changed after it, and its warnings are shown", () => {
    const m = model();
    transitFitStore.setResult(G, "white", { request: REQ, result: RES, signature: "old", fittedUtc: "t" });
    const tree = render(m);
    expect(markedText(tree, "data-stale-fit")).toBe("The curve or its kept points changed since this fit; fit again.");
    const expected = transitPlotSeries(RES, { ...REQ, pointIndex: [] }, 2460310)!;
    expect((plots(tree)[0].series as Props[])[0].x).toEqual(expected.time);
    expect((plots(tree)[1].series as Props[])[0].y).toEqual(expected.residualPpm);
    const warnings = marked(tree, "data-fit-warning");
    expect(warnings).toHaveLength(1);
    expect(textOf(warnings[0].props.children)).toBe("warn b");
  });

  it("an error of the command is shown and a late result of an older click is dropped", async () => {
    store(FULL);
    const m = model();
    const message = "no transit: b 1.5 is at or above 1 + rp = 1.1, so the planet never crosses the star";
    backend.fit.mockReturnValueOnce(Promise.reject(new Error(message)));
    let tree = render(m);
    expect(all(tree).find((e) => e.type === ErrorAlert)!.props.message).toBeNull();
    (fitButton(tree).onClick as () => void)();
    await settle();
    tree = render(m);
    expect(all(tree).find((e) => e.type === ErrorAlert)!.props.message).toBe(message);
    expect(fitButton(tree).disabled).toBe(false);
    expect(transitFitStore.get(G, "white").result).toBeNull();

    const first = deferred<TransitFitResult>();
    const second = deferred<TransitFitResult>();
    const SECOND_RESULT: TransitFitResult = { ...RES, rms_ppm: 123 };
    backend.fit.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    tree = render(m);
    (fitButton(tree).onClick as () => void)();
    (fitButton(tree).onClick as () => void)();
    tree = render(m);
    expect(all(tree).find((e) => e.type === ErrorAlert)!.props.message).toBeNull();
    second.resolve(SECOND_RESULT);
    await settle();
    first.resolve(RES);
    await settle();
    expect(transitFitStore.get(G, "white").result).toBe(SECOND_RESULT);
    tree = render(m);
    expect(textOf(fitButton(tree).children)).toBe("Fit");

    const late = deferred<TransitFitResult>();
    const failing = deferred<TransitFitResult>();
    backend.fit.mockReturnValueOnce(late.promise).mockReturnValueOnce(failing.promise);
    (fitButton(tree).onClick as () => void)();
    (fitButton(tree).onClick as () => void)();
    failing.reject(new Error("later error"));
    await settle();
    late.reject(new Error("older error"));
    await settle();
    tree = render(m);
    expect(all(tree).find((e) => e.type === ErrorAlert)!.props.message).toBe("later error");
  });

  it("Start from the quick look fills the four values", () => {
    const window = { start: 2460310.18, end: 2460310.22 };
    lightCurveStore.patchView(G, { window });
    const setup = { ...DEFAULT_TRANSIT_FIT_SETUP, period: "3" };
    store(setup);
    const m = model({ window });
    let tree = render(m);
    const start = button(tree, "Start from the quick look")!;
    expect(start.disabled).toBe(false);
    expect(start.title).toBe("Fill the mid time, Rp/Rs, a/Rs and b from the transit window, the box depth and the period");
    (start.onClick as () => void)();
    const expected = setupWithStart(setup, transitStartFromQuickLook(window, m.depth?.depth ?? null, 3)!, 2460310);
    expect(transitFitStore.get(G, "white").setup).toEqual(expected);
    expect(transitFitStore.get(G, "white").setup.t0.text).toBe("0.2");

    hooks.mount();
    store(setup);
    const deeper = model({ window }, { ...DOC, value: DOC.value.map((v) => (v === 990 ? 960 : v)) });
    (button(render(deeper), "Start from the quick look")!.onClick as () => void)();
    expect(deeper.depth?.depth).toBeCloseTo(0.04, 12);
    expect(transitFitStore.get(G, "white").setup.rp.text).toBe("0.2");

    hooks.mount();
    lightCurveStore.patchView(G, { window: null });
    tree = render(model());
    expect(button(tree, "Start from the quick look")!.disabled).toBe(true);
    expect(button(tree, "Start from the quick look")!.title).toBe("Set a transit window in the quick look first");

    hooks.mount();
    lightCurveStore.patchView(G, { window });
    store({ ...DEFAULT_TRANSIT_FIT_SETUP });
    tree = render(model({ window }));
    expect(button(tree, "Start from the quick look")!.disabled).toBe(true);
    expect(button(tree, "Start from the quick look")!.title).toBe("Type the period first (the window must be shorter than half of it)");
    (button(tree, "Start from the quick look")!.onClick as () => void)();
    expect(transitFitStore.get(G, "white").setup).toEqual(DEFAULT_TRANSIT_FIT_SETUP);
  });

  it("Use fold period appears only while the period is blank and a fold is stored", () => {
    lightCurveStore.patchView(G, { fold: { periodDays: 1.4079405, t0: 2460310.2 } });
    const m = model();
    let tree = render(m);
    (button(tree, "Use fold period")!.onClick as () => void)();
    expect(transitFitStore.get(G, "white").setup.period).toBe("1.4079405");

    store({ ...DEFAULT_TRANSIT_FIT_SETUP, period: "3" });
    tree = render(m);
    expect(button(tree, "Use fold period")).toBeUndefined();

    hooks.mount();
    lightCurveStore.patchView(G, { fold: null });
    store({ ...DEFAULT_TRANSIT_FIT_SETUP });
    tree = render(m);
    expect(button(tree, "Use fold period")).toBeUndefined();
  });

  function stored(m: LightCurveModel, result: TransitFitResult = RES): void {
    transitFitStore.setResult(G, "white", { request: REQ, result, signature: transitFitSignature(transitFitInput(m)), fittedUtc: "t" });
  }

  function card(tree: unknown, name: string): string {
    return textOf(all(tree).find((e) => e.props["data-card"] === name));
  }

  function cellTexts(element: ReactElement<Props>, cell: "th" | "td"): string[] {
    return elementsOf<Props>(element)
      .filter((e) => e.type === cell)
      .map((e) => textOf(e.props.children));
  }

  function bodyRows(element: ReactElement<Props>): string[][] {
    return elementsOf<Props>(element)
      .filter((e) => e.type === "tr" && elementsOf<Props>(e.props.children).some((c) => c.type === "td"))
      .map((e) => cellTexts(e, "td"));
  }

  function alertMessage(tree: unknown): unknown {
    return all(tree).find((e) => e.type === ErrorAlert)!.props.message;
  }

  const SAVED_PATH = "C:/out/lightcurve_transit_fit.csv";
  const NOW = "2026-11-13T12:00:00.000Z";

  function expectedCsv(): string {
    return transitFitCsvText(RES, { ...REQ, pointIndex: [] }, { fileName: "t.csv", series: "white", timeName: "BJD_TDB", timeScale: "bjd_tdb", fluxUnit: "Jy", exportedUtc: NOW });
  }

  it("the cards print the depth, the scatter, the reduced chi2 and beta", () => {
    const m = model();
    stored(m);
    let tree = render(m);
    const depth = card(tree, "fit-depth");
    expect(depth).toContain("(Rp/Rs)^2");
    expect(depth).toContain(formatWithUncertainty(0.009964, 0.000095));
    expect(depth).toContain("least squares, 1 sigma from the covariance");
    expect(depth).not.toContain("errors multiplied");
    expect(card(tree, "fit-scatter")).toContain("270.7 ppm");
    expect(card(tree, "fit-scatter")).toContain("N = 3, 1 in transit");
    expect(card(tree, "fit-chi2")).toContain("Reduced chi2");
    expect(card(tree, "fit-chi2")).toContain("0.835");
    expect(card(tree, "fit-chi2")).toContain("chi2 195.3 for 234 degrees of freedom");
    expect(card(tree, "fit-beta")).toContain("Time-averaging beta");
    expect(card(tree, "fit-beta")).toContain("1");
    expect(card(tree, "fit-beta")).toContain("largest binned rms over the white-noise expectation; 1 is white noise");
    expect(card(tree, "fit-scatter")).toContain("Residual scatter");

    const elements = all(tree);
    const residualPlot = elements.findIndex((e) => e.type === ProfilePlot && e.props.yLabel === "residual (ppm of the baseline)");
    const firstCard = elements.findIndex((e) => e.props["data-card"] === "fit-depth");
    const order = [
      firstCard,
      elements.findIndex((e) => e.props["data-fit-results"] !== undefined),
      elements.findIndex((e) => e.props["data-fit-time-averaging"] !== undefined),
      elements.findIndex((e) => e.props["data-fit-notes"] !== undefined),
      elements.findIndex((e) => e.type === "button" && textOf(e.props.children) === "Save fit CSV"),
    ];
    expect(residualPlot).toBeGreaterThanOrEqual(0);
    expect(firstCard).toBeGreaterThan(residualPlot);
    expect(order).toEqual([...order].sort((a, b) => a - b));
    expect(new Set(order).size).toBe(5);
    expect(marked(tree, "data-fit-results-slot")).toHaveLength(0);

    hooks.mount();
    stored(m, { ...RES, weighted: false, error_scale: 0.0002747 });
    tree = render(m);
    expect(card(tree, "fit-chi2")).toContain("--");
    expect(card(tree, "fit-chi2")).toContain("equal weights: the points carry no errors, so there is no chi2 to judge");
    expect(card(tree, "fit-chi2")).not.toContain("0.835");
    expect(card(tree, "fit-chi2")).not.toContain("degrees of freedom");
    expect(card(tree, "fit-depth")).toContain("errors multiplied by 0.000275");

    hooks.mount();
    stored(m, { ...RES, time_averaging: { ...RES.time_averaging, beta: 1.2345 } });
    expect(card(render(m), "fit-beta")).toContain("Time-averaging beta1.23largest");
  });

  it("the result table lists every quantity with its value, unit and whether it was fitted", () => {
    const m = model();
    stored(m);
    const tree = render(m);
    const table = marked(tree, "data-fit-results")[0];
    expect(table.type).toBe("table");
    expect(cellTexts(table, "th")).toEqual(["quantity", "value", "unit", "fit"]);
    const expected = transitResultRows(RES, 2460310, "BJD_TDB - 2460310", "Jy");
    expect(expected).toHaveLength(13);
    expect(bodyRows(table)).toEqual(expected.map((r) => [r.label, r.value, r.unit, r.fit]));
    expect(elementsOf<Props>(table).filter((e) => e.props["data-fit-row"] !== undefined).map((e) => e.props["data-fit-row"])).toEqual(
      expected.map((r) => r.key),
    );
    const rowsShown = bodyRows(table);
    expect(rowsShown[0]).toEqual(["Mid-transit time", formatWithUncertainty(2460310.4999394 - 2460310, 0.0000828), "BJD_TDB - 2460310", "free"]);
    expect(rowsShown[2]).toEqual(["a/Rs", "9", "", "fixed"]);
    expect(rowsShown[9]).toEqual(["Duration, contacts 2 to 3", "--", "h", "derived"]);
    expect(rowsShown[12]).toEqual(["Baseline c1", expected[12].value, "Jy / d", "free"]);

    hooks.mount();
    const relative = model({}, { ...DOC, quantity: "rel_flux" });
    stored(relative);
    const relativeRows = bodyRows(marked(render(relative), "data-fit-results")[0]);
    expect(relativeRows.map((r) => r[2]).slice(11)).toEqual(["", "1 / d"]);
  });

  it("the time-averaging table gives bin lengths, both scatters and their ratio", () => {
    const m = model();
    stored(m);
    const tree = render(m);
    const block = marked(tree, "data-fit-time-averaging")[0];
    expect(block.type).toBe("details");
    expect(block.props.open).toBeFalsy();
    expect(textOf(elementsOf<Props>(block).find((e) => e.type === "summary")!.props.children)).toBe("Time-averaging test");
    expect(cellTexts(block, "th")).toEqual(["bin points", "bin min", "bins", "rms ppm", "white noise ppm", "ratio"]);
    expect(bodyRows(block)).toEqual([
      ["1", "1.44", "240", "270.7", "271.3", "0.998"],
      ["2", "2.88", "120", "190", "192.2", "0.989"],
    ]);
    expect(textOf(block)).toContain(
      "Residuals binned by 1, 2, 4, ... points. White noise falls as 1 / sqrt(points); a ratio well above 1 at long bins is correlated noise that the errors of the fit do not include.",
    );

    hooks.mount();
    stored(m, { ...RES, time_averaging: { ...RES.time_averaging, cadence_days: 0, expected_ppm: [0, 192.2] } });
    expect(bodyRows(marked(render(m), "data-fit-time-averaging")[0])).toEqual([
      ["1", "--", "240", "270.7", "0", "--"],
      ["2", "--", "120", "190", "192.2", "0.989"],
    ]);

    hooks.mount();
    stored(m, { ...RES, time_averaging: { ...RES.time_averaging, cadence_days: 0.0011 } });
    expect(bodyRows(marked(render(m), "data-fit-time-averaging")[0]).map((r) => r[1])).toEqual(["1.58", "3.17"]);
  });

  it("the notes of the fit are listed", () => {
    const m = model();
    stored(m, { ...RES, notes: ["note a", "note c"] });
    let tree = render(m);
    let block = marked(tree, "data-fit-notes")[0];
    expect(block.type).toBe("details");
    expect(block.props.open).toBeFalsy();
    expect(textOf(elementsOf<Props>(block).find((e) => e.type === "summary")!.props.children)).toBe("notes (2)");
    expect(elementsOf<Props>(block).filter((e) => e.type === "li").map((e) => textOf(e.props.children))).toEqual(["note a", "note c"]);

    hooks.mount();
    stored(m);
    tree = render(m);
    block = marked(tree, "data-fit-notes")[0];
    expect(textOf(elementsOf<Props>(block).find((e) => e.type === "summary")!.props.children)).toBe("notes (1)");
    expect(elementsOf<Props>(block).filter((e) => e.type === "li").map((e) => textOf(e.props.children))).toEqual(["note a"]);
  });

  it("an eclipse result names its card and its rows", () => {
    const m = model();
    const eclipse: TransitFitResult = {
      ...RES,
      kind: "eclipse",
      parameters: [...RES.parameters.slice(0, 4), { name: "flux_ratio", value: 0.002, err: 0.0000067, free: true, at_bound: false }],
      depth: 0.001996,
      depth_err: 0.0000067,
    };
    stored(m, eclipse);
    const tree = render(m);
    const depth = card(tree, "fit-depth");
    expect(depth).toContain("Eclipse depth");
    expect(depth).toContain(`${formatWithUncertainty(0.001996 * 1e6, 0.0000067 * 1e6)} ppm`);
    expect(depth).not.toContain("(Rp/Rs)^2");
    expect(card(tree, "fit-scatter")).toContain("N = 3, 1 in eclipse");
    expect(bodyRows(marked(tree, "data-fit-results")[0])[0][0]).toBe("Mid-eclipse time");
  });

  it("Save fit CSV writes the text of the fit and shows where", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(NOW));
    const m = model();
    stored(m);
    backend.save.mockResolvedValueOnce(SAVED_PATH);
    let tree = render(m);
    expect(marked(tree, "data-fit-saved")).toHaveLength(0);
    (button(tree, "Save fit CSV")!.onClick as () => void)();
    await settle();
    expect(backend.save).toHaveBeenCalledTimes(1);
    expect(backend.save).toHaveBeenCalledWith(expectedCsv(), "lightcurve_transit_fit.csv", "Save the transit fit");
    tree = render(m);
    expect(markedText(tree, "data-fit-saved")).toBe(`Saved ${SAVED_PATH}`);
    expect(alertMessage(tree)).toBeNull();

    backend.save.mockRejectedValueOnce(new Error("disk full"));
    (button(tree, "Save fit CSV")!.onClick as () => void)();
    await settle();
    tree = render(m);
    expect(alertMessage(tree)).toBe("disk full");
    expect(marked(tree, "data-fit-saved")).toHaveLength(0);

    backend.save.mockResolvedValueOnce(SAVED_PATH);
    (button(tree, "Save fit CSV")!.onClick as () => void)();
    await settle();
    tree = render(m);
    expect(alertMessage(tree)).toBeNull();
    expect(markedText(tree, "data-fit-saved")).toBe(`Saved ${SAVED_PATH}`);

    backend.save.mockResolvedValueOnce(null);
    (button(tree, "Save fit CSV")!.onClick as () => void)();
    await settle();
    tree = render(m);
    expect(marked(tree, "data-fit-saved")).toHaveLength(0);
    expect(alertMessage(tree)).toBeNull();

    backend.save.mockResolvedValueOnce(SAVED_PATH);
    (button(tree, "Save fit CSV")!.onClick as () => void)();
    await settle();
    tree = render(m);
    expect(markedText(tree, "data-fit-saved")).toBe(`Saved ${SAVED_PATH}`);
    store(FULL);
    backend.fit.mockReturnValueOnce(deferred<TransitFitResult>().promise);
    tree = render(m);
    (fitButton(tree).onClick as () => void)();
    tree = render(m);
    expect(marked(tree, "data-fit-saved")).toHaveLength(0);

    const tso = { kind: "tso", detector: "NRS1", sporder: null, band_um: null, mode: "pipeline" } as const;
    const sources: [LightCurveDoc["source"], string | null][] = [
      [{ kind: "frames", reference: "ref_0001.fits", target: "T", comps: [] }, "ref_0001.fits"],
      [{ ...tso, paths: ["C:/a/seg_1.fits", "D:\\b\\seg_2.fits"] }, "seg_1.fits; seg_2.fits"],
      [{ ...tso, paths: [] }, null],
    ];
    for (const [source, fileName] of sources) {
      hooks.mount();
      backend.save.mockClear();
      backend.save.mockResolvedValueOnce(null);
      const other = model({}, { ...DOC, source });
      stored(other);
      (button(render(other), "Save fit CSV")!.onClick as () => void)();
      await settle();
      const text = transitFitCsvText(RES, { ...REQ, pointIndex: [] }, { fileName, series: "white", timeName: "BJD_TDB", timeScale: "bjd_tdb", fluxUnit: "Jy", exportedUtc: NOW });
      expect(backend.save).toHaveBeenCalledWith(text, "lightcurve_transit_fit.csv", "Save the transit fit");
    }
  });

  it("Copy fit CSV puts the same text on the clipboard and reports a failure", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(NOW));
    const m = model();
    stored(m);
    const copyProps = (tree: unknown) => all(tree).find((e) => e.type === CopyCsvButton && e.props.label === "Copy fit CSV")!.props;
    let tree = render(m);
    expect((copyProps(tree).text as () => string | null)()).toBe(expectedCsv());
    tree = render(m);
    expect(alertMessage(tree)).toBeNull();

    (copyProps(tree).onError as (m: string) => void)("Clipboard copy failed: denied");
    tree = render(m);
    expect(alertMessage(tree)).toBe("Clipboard copy failed: denied");

    expect((copyProps(tree).text as () => string | null)()).toBe(expectedCsv());
    expect(alertMessage(render(m))).toBeNull();
  });

  it("Log transit fit appends the entry of the fit", () => {
    const m = model();
    stored(m);
    const tree = render(m);
    const log = button(tree, "Log transit fit")!;
    expect(log.title).toBe("Add the fitted values, their errors and the notes of the fit to the measurement log");
    (log.onClick as () => void)();
    expect(measurementLog.append).toHaveBeenCalledTimes(1);
    expect(vi.mocked(measurementLog.append).mock.calls[0][0]).toEqual(transitFitEntry(DOC, RES, "BJD_TDB"));
  });

  it("nothing of the results is shown before a fit", () => {
    const tree = render(model());
    for (const hook of ["data-card", "data-fit-results", "data-fit-time-averaging", "data-fit-notes", "data-fit-saved"]) {
      expect(marked(tree, hook)).toHaveLength(0);
    }
    for (const text of ["Save fit CSV", "Log transit fit"]) {
      expect(button(tree, text)).toBeUndefined();
    }
    expect(all(tree).filter((e) => e.type === CopyCsvButton)).toHaveLength(0);
  });
});
