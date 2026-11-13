import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";

const backend = vi.hoisted(() => ({
  search: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
}));

vi.mock("react", async (io) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await io()));
vi.mock("../../../services/periodogram", () => ({ lightCurvePeriodogram: backend.search }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { lightCurveStore } from "../../../hooks/useLightCurveStore";
import { lightCurveModel, periodogramInput, type LightCurveModel } from "../../../utils/lightCurveViewModel";
import { periodogramEntry } from "../../../utils/lightCurveDoc";
import { measurementLog } from "../../../utils/measurementLog";
import type { LightCurveDoc } from "../../../shared/types/lightCurve";
import type { PeriodogramSearch } from "../../../services/periodogram";
import { periodogramAliasWarning, periodogramRangeText } from "../../../utils/lightCurveInputs";
import ProfilePlot from "../../regions/ProfilePlot";
import ErrorAlert from "../../ui/ErrorAlert";
import type { PeriodogramSectionProps } from "../PeriodogramSection";

const { default: PeriodogramSection } = await import("../PeriodogramSection");

type Props = Record<string, unknown>;

const G = "table:C:/d/v.csv";

const DOC: LightCurveDoc = {
  id: "v",
  label: "variable",
  quantity: "flux",
  unit: "Jy",
  timeScale: "bjd_tdb",
  time: Array.from({ length: 40 }, (_, i) => 2460310 + 0.37 * i),
  value: Array.from({ length: 40 }, (_, i) => 1 + 0.01 * Math.sin(i)),
  err: Array.from({ length: 40 }, () => 0.001),
  pointRef: Array.from({ length: 40 }, (_, i) => i),
  flags: Array.from({ length: 40 }, () => []),
  source: { kind: "table", path: "C:/d/v.csv", time_column: "t", value_column: "f" },
  notes: [],
};

const RES: PeriodogramSearch = {
  frequency: [0.3315762781484169, 0.3453919564046009, 0.359207634660785],
  power: [0.61, 0.8675526073476112, 0.58],
  best_frequency: 0.3453919564046009,
  best_period: 2.895261402175141,
  best_power: 0.8675526073476112,
  n_points: 40,
  n_dropped: 0,
  n_frequencies: 500,
  baseline_days: 14.476307010875704,
  frequency_step: 0.013815678256184037,
  first_frequency: 0.013815678256184037,
  last_frequency: 6.907839128092019,
  upper_limit: "five_mean_nyquist",
  cadence_days: null,
  normalization: "standard",
  weighted: true,
  false_alarm_probability: 6.64610294915559e-14,
  fap_method: "baluev",
  notes: ["note a", "note b"],
  elapsed_ms: 3,
};

const SEARCH = "Search period";

const section = (PeriodogramSection as unknown as { type: (p: PeriodogramSectionProps) => unknown }).type;

function model(doc: LightCurveDoc = DOC): LightCurveModel {
  return lightCurveModel(doc, lightCurveStore.getView(G));
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

function type(tree: unknown, label: string, text: string): void {
  (field(tree, label)!.onChange as (text: string) => void)(text);
}

function button(tree: unknown, text: string): Props | undefined {
  return all(tree).find((e) => e.type === "button" && textOf(e.props.children) === text)?.props;
}

function searchButton(tree: unknown): Props {
  return all(tree).find((e) => e.type === "button" && [SEARCH, "Searching..."].includes(textOf(e.props.children)))!.props;
}

function click(props: Props | undefined): void {
  (props!.onClick as () => void)();
}

function marked(tree: unknown, hook: string): ReactElement<Props>[] {
  return all(tree).filter((e) => e.props[hook] !== undefined);
}

function markedText(tree: unknown, hook: string): string | undefined {
  const element = marked(tree, hook)[0];
  return element === undefined ? undefined : textOf(element.props.children);
}

function card(tree: unknown, name: string): string {
  return textOf(all(tree).find((e) => e.props["data-card"] === name));
}

function plots(tree: unknown): Props[] {
  return all(tree)
    .filter((e) => e.type === ProfilePlot)
    .map((e) => e.props);
}

function alertMessage(tree: unknown): unknown {
  return all(tree).find((e) => e.type === ErrorAlert)!.props.message;
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

async function searched(m: LightCurveModel, result: PeriodogramSearch = RES): Promise<unknown> {
  backend.search.mockResolvedValueOnce(result);
  click(searchButton(render(m)));
  await settle();
  return render(m);
}

describe("PeriodogramSection", () => {
  beforeEach(() => {
    hooks.mount();
    backend.search.mockReset();
    vi.spyOn(measurementLog, "append");
    lightCurveStore.remove(G);
    lightCurveStore.put(G, [DOC]);
  });

  afterEach(() => {
    hooks.mount();
    vi.restoreAllMocks();
    lightCurveStore.remove(G);
  });

  it("the search sends the kept points of the model and no range by default", () => {
    const m = model();
    let tree = render(m) as ReactElement<Props>;
    expect(tree.type).toBe("details");
    expect(tree.props.open).toBeUndefined();
    expect(tree.props["data-periodogram"]).toBeDefined();
    expect(textOf(all(tree).find((e) => e.type === "summary")!.props.children)).toBe("Period search and fold");
    for (const label of ["shortest period d", "longest period d"]) {
      expect(field(tree, label)!.optional).toBe(true);
      expect(field(tree, label)!.placeholder).toBe("automatic");
      expect(field(tree, label)!.value).toBe("");
    }
    expect(searchButton(tree).disabled).toBe(false);
    expect(marked(tree, "data-search-hint")).toHaveLength(0);
    expect(plots(tree)).toHaveLength(0);
    const pending = deferred<PeriodogramSearch>();
    backend.search.mockReturnValueOnce(pending.promise);
    click(searchButton(tree));
    expect(backend.search).toHaveBeenCalledTimes(1);
    const input = periodogramInput(m);
    expect(input.time).toHaveLength(40);
    expect(input.err).not.toBeNull();
    expect(backend.search.mock.calls[0]).toEqual([input.time, input.value, input.err, { minFrequency: null, maxFrequency: null }]);
    tree = render(m) as ReactElement<Props>;
    expect(textOf(searchButton(tree).children)).toBe("Searching...");
    expect(searchButton(tree).disabled).toBe(true);
    expect(searchButton(tree).title).toBeUndefined();
  });

  it("the result is drawn against frequency with the peak marked, and the cards print it", async () => {
    const m = model();
    const tree = await searched(m);
    expect(textOf(searchButton(tree).children)).toBe(SEARCH);
    expect(searchButton(tree).disabled).toBe(false);
    const drawn = plots(tree);
    expect(drawn).toHaveLength(1);
    const [periodogram] = drawn;
    expect(periodogram.xLabel).toBe("frequency (cycles per day)");
    expect(periodogram.yLabel).toBe("power");
    expect(periodogram.series).toEqual([{ x: RES.frequency, y: RES.power, color: "#67e8f9", label: "power", mode: "line" }]);
    expect(periodogram.referenceLines).toEqual([{ axis: "x", value: 0.3453919564046009, label: "P = 2.89526 d" }]);
    expect(periodogram.height).toBe(130);
    expect(periodogram.toolbar).toBe(true);
    expect(periodogram.logToggle).toBe(false);
    expect(periodogram.wheelZoom).toBe("modifier");
    expect(periodogram.csvName).toBe("lightcurve_periodogram");
    expect(card(tree, "period")).toBe("Best period2.89526 dfrequency 0.345392 per day, power 0.868");
    expect(card(tree, "fap")).toBe("False-alarm probability6.6e-14Baluev 2008 upper limit for the highest peak, white Gaussian noise");
    expect(card(tree, "resolution")).toBe("Grid resolution0.12 done step of 500 frequencies; 40 points, weighted by 1 / err^2");
    const notes = marked(tree, "data-periodogram-notes")[0];
    expect(notes.type).toBe("details");
    expect(notes.props.open).toBeUndefined();
    expect(textOf(elementsOf<Props>(notes).find((e) => e.type === "summary")!.props.children)).toBe("notes (2)");
    expect(elementsOf<Props>(notes).filter((e) => e.type === "li").map((e) => textOf(e.props.children))).toEqual(["note a", "note b"]);
    expect(button(tree, "Fold at this period")).toBeDefined();
    expect(button(tree, "Log period search")).toBeDefined();
    expect(marked(tree, "data-stale-search")).toHaveLength(0);

    hooks.mount();
    const plain = await searched(m, { ...RES, weighted: false, false_alarm_probability: null });
    expect(card(plain, "resolution")).toContain("40 points, equal weights");
    expect(card(plain, "fap")).toContain("--");
  });

  it("the searched range and its limit are stated, and a peak above the Nyquist frequency of a regular cadence is flagged", async () => {
    const m = model();
    const regular: PeriodogramSearch = {
      ...RES,
      first_frequency: 0.5858166,
      last_frequency: 534.2647,
      upper_limit: "cadence_nyquist",
      cadence_days: 0.0009353528221254237,
      best_frequency: 4.1,
      best_period: 1 / 4.1,
    };
    let tree = await searched(m, regular);
    expect(markedText(tree, "data-search-range")).toBe(periodogramRangeText(regular));
    expect(markedText(tree, "data-search-range")).toContain("twice the regular cadence of 0.0009354 d (Nyquist frequency 534.6 per day)");
    expect(marked(tree, "data-alias-warning")).toHaveLength(0);
    expect(markedText(await searched(m, RES), "data-search-range")).toBe(periodogramRangeText(RES));

    const alias: PeriodogramSearch = { ...regular, upper_limit: "given", last_frequency: 2680, best_frequency: 2134.13, best_period: 1 / 2134.13 };
    tree = await searched(m, alias);
    expect(markedText(tree, "data-alias-warning")).toBe(periodogramAliasWarning(alias));
    expect(marked(tree, "data-alias-warning")[0].props.className).toBe("text-[9px] text-amber-400/90");
  });

  it("a period range is sent as a frequency range and a wrong range blocks the search", () => {
    const m = model();
    backend.search.mockReturnValue(new Promise(() => {}));
    let tree = render(m);
    type(tree, "shortest period d", "0.5");
    tree = render(m);
    type(tree, "longest period d", "10");
    tree = render(m);
    expect(field(tree, "shortest period d")!.value).toBe("0.5");
    expect(field(tree, "longest period d")!.value).toBe("10");
    click(searchButton(tree));
    expect(backend.search.mock.calls[0][3]).toEqual({ minFrequency: 0.1, maxFrequency: 2 });

    hooks.mount();
    backend.search.mockClear();
    tree = render(m);
    type(tree, "shortest period d", "5");
    tree = render(m);
    type(tree, "longest period d", "5");
    tree = render(m);
    const refused = "the shortest period must be below the longest";
    expect(searchButton(tree).disabled).toBe(true);
    expect(searchButton(tree).title).toBe(refused);
    expect(markedText(tree, "data-search-hint")).toBe(refused);
    click(searchButton(tree));
    expect(backend.search).not.toHaveBeenCalled();
  });

  it("the search is refused on an index axis and below four points", () => {
    const indexed = model({ ...DOC, timeScale: "index", time: DOC.time.map((_, i) => i) });
    let tree = render(indexed);
    expect(searchButton(tree).disabled).toBe(true);
    expect(searchButton(tree).title).toBe("A period search needs a time axis.");
    expect(markedText(tree, "data-search-hint")).toBe("A period search needs a time axis.");
    expect(field(tree, "fold period d")).toBeUndefined();
    expect(field(tree, "fold epoch")).toBeUndefined();
    click(searchButton(tree));

    hooks.mount();
    const three: LightCurveDoc = {
      ...DOC,
      time: DOC.time.slice(0, 4),
      value: [1, 1.01, null, 0.99],
      err: DOC.err.slice(0, 4),
      pointRef: [0, 1, 2, 3],
      flags: [[], [], [], []],
    };
    tree = render(model(three));
    expect(searchButton(tree).disabled).toBe(true);
    expect(searchButton(tree).title).toBe("A period search needs at least 4 points.");
    expect(markedText(tree, "data-search-hint")).toBe("A period search needs at least 4 points.");
    expect(field(tree, "fold period d")).toBeDefined();
    click(searchButton(tree));
    expect(backend.search).not.toHaveBeenCalled();
  });

  it("an error of the command is shown and a late result of an older request is dropped", async () => {
    const m = model();
    const message = "the values are constant: there is nothing to search";
    backend.search.mockReturnValueOnce(Promise.reject(new Error(message)));
    let tree = render(m);
    expect(alertMessage(tree)).toBeNull();
    click(searchButton(tree));
    await settle();
    tree = render(m);
    expect(alertMessage(tree)).toBe(message);
    expect(plots(tree)).toHaveLength(0);
    expect(searchButton(tree).disabled).toBe(false);

    const first = deferred<PeriodogramSearch>();
    const second = deferred<PeriodogramSearch>();
    const SECOND: PeriodogramSearch = { ...RES, best_period: 5, best_frequency: 0.2 };
    backend.search.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    tree = render(m);
    click(searchButton(tree));
    expect(alertMessage(render(m))).toBeNull();
    click(searchButton(tree));
    second.resolve(SECOND);
    await settle();
    first.resolve(RES);
    await settle();
    tree = render(m);
    expect(card(tree, "period")).toContain("5 d");
    expect(card(tree, "period")).not.toContain("2.89526");
    expect(searchButton(tree).disabled).toBe(false);

    hooks.mount();
    const late = deferred<PeriodogramSearch>();
    backend.search.mockReturnValueOnce(late.promise);
    click(searchButton(render(m)));
    const other = model({ ...DOC, id: "other", label: "other" });
    render(other);
    late.resolve(RES);
    await settle();
    tree = render(other);
    expect(plots(tree)).toHaveLength(0);
    expect(textOf(searchButton(tree).children)).toBe(SEARCH);

    hooks.mount();
    const failed = deferred<PeriodogramSearch>();
    const kept = deferred<PeriodogramSearch>();
    backend.search.mockReturnValueOnce(failed.promise).mockReturnValueOnce(kept.promise);
    tree = render(m);
    click(searchButton(tree));
    click(searchButton(tree));
    kept.resolve(RES);
    await settle();
    failed.reject(new Error("older"));
    await settle();
    tree = render(m);
    expect(alertMessage(tree)).toBeNull();
    expect(plots(tree)).toHaveLength(1);
  });

  it("a result is marked when the curve changed after the search", async () => {
    const m = model();
    let tree = await searched(m);
    expect(marked(tree, "data-stale-search")).toHaveLength(0);
    lightCurveStore.patchView(G, { excluded: [3] });
    const fewer = model();
    expect(fewer.counts.kept).toBe(39);
    tree = render(fewer);
    expect(markedText(tree, "data-stale-search")).toBe("The curve changed since this search; search again.");
    expect(marked(tree, "data-stale-search")[0].props.className).toBe("text-[9px] text-amber-400/90");
    expect(plots(tree)).toHaveLength(1);
    lightCurveStore.patchView(G, { excluded: [] });
    tree = render(model());
    expect(marked(tree, "data-stale-search")).toHaveLength(0);
    lightCurveStore.patchView(G, { display: "normalised" });
    const normalised = model();
    expect(normalised.normalised).toBe(true);
    expect(normalised.counts.kept).toBe(40);
    tree = render(normalised);
    expect(markedText(tree, "data-stale-search")).toBe("The curve changed since this search; search again.");
  });

  it("Fold at this period stores the peak period with the window centre as epoch", async () => {
    lightCurveStore.patchView(G, { window: { start: 2460312, end: 2460313 } });
    let tree = await searched(model());
    click(button(tree, "Fold at this period"));
    expect(lightCurveStore.getView(G).fold).toEqual({ periodDays: 2.895261402175141, t0: 2460312.5 });

    hooks.mount();
    lightCurveStore.patchView(G, { window: null, fold: null });
    tree = await searched(model());
    click(button(tree, "Fold at this period"));
    expect(lightCurveStore.getView(G).fold).toEqual({ periodDays: 2.895261402175141, t0: 2460310 });

    hooks.mount();
    lightCurveStore.patchView(G, { fold: null, excluded: [0] });
    tree = await searched(model());
    click(button(tree, "Fold at this period"));
    expect(lightCurveStore.getView(G).fold).toEqual({ periodDays: 2.895261402175141, t0: DOC.time[1] });
  });

  it("the fold fields write the store, Unfold clears it, and the folded curve is drawn against phase", () => {
    let m = model();
    let tree = render(m);
    expect(field(tree, "fold period d")!.value).toBe("");
    expect(field(tree, "fold epoch")!.value).toBe("");
    expect(field(tree, "fold period d")!.optional).toBe(true);
    expect(field(tree, "fold epoch")!.optional).toBe(true);
    expect(button(tree, "Unfold")).toBeUndefined();
    type(tree, "fold period d", "2");
    tree = render(m);
    expect(markedText(tree, "data-fold-hint")).toBe("the fold needs an epoch");
    expect(lightCurveStore.getView(G).fold).toBeNull();
    type(tree, "fold epoch", "0.5");
    expect(lightCurveStore.getView(G).fold).toEqual({ periodDays: 2, t0: 2460310.5 });

    m = model();
    expect(m.folded).not.toBeNull();
    tree = render(m);
    expect(field(tree, "fold period d")!.value).toBe("2");
    expect(field(tree, "fold epoch")!.value).toBe("0.5");
    expect(marked(tree, "data-fold-hint")).toHaveLength(0);
    const folded = plots(tree);
    expect(folded).toHaveLength(1);
    expect(folded[0].xLabel).toBe("phase (period 2 d)");
    expect(folded[0].series).toEqual([
      { x: m.folded!.phase, y: m.folded!.y, yErr: m.folded!.yErr, color: "#fbbf24", label: "variable", mode: "points" },
    ]);
    expect(folded[0].yLabel).toBe(m.yLabel);
    expect(folded[0].invertY).toBe(m.invertY);
    expect(folded[0].height).toBe(150);
    expect(folded[0].toolbar).toBe(true);
    expect(folded[0].logToggle).toBe(false);
    expect(folded[0].wheelZoom).toBe("modifier");
    expect(folded[0].csvName).toBe("lightcurve_folded");

    hooks.mount();
    lightCurveStore.patchView(G, { fold: { periodDays: 1.4079405, t0: 2460310 + 0.5077 } });
    tree = render(model());
    expect(field(tree, "fold period d")!.value).toBe("1.4079405");
    expect(field(tree, "fold epoch")!.value).toBe("0.5077");
    type(tree, "fold epoch", "x");
    tree = render(model());
    expect(markedText(tree, "data-fold-hint")).toBe("fold epoch is not a number");
    expect(lightCurveStore.getView(G).fold).toEqual({ periodDays: 1.4079405, t0: 2460310 + 0.5077 });
    type(tree, "fold epoch", "0.75");
    expect(lightCurveStore.getView(G).fold).toEqual({ periodDays: 1.4079405, t0: 2460310.75 });
    tree = render(model());

    click(button(tree, "Unfold"));
    expect(lightCurveStore.getView(G).fold).toBeNull();
    m = model();
    expect(m.folded).toBeNull();
    render(m);
    tree = render(m);
    expect(field(tree, "fold period d")!.value).toBe("");
    expect(field(tree, "fold epoch")!.value).toBe("");
    expect(button(tree, "Unfold")).toBeUndefined();
    expect(plots(tree)).toHaveLength(0);
    type(tree, "fold period d", "2");
    tree = render(m);
    expect(markedText(tree, "data-fold-hint")).toBe("the fold needs an epoch");
    expect(lightCurveStore.getView(G).fold).toBeNull();
  });

  it("Log period search appends the entry of the result", async () => {
    const m = model();
    const tree = await searched(m);
    click(button(tree, "Log period search"));
    expect(measurementLog.append).toHaveBeenCalledTimes(1);
    expect(measurementLog.append).toHaveBeenCalledWith(
      periodogramEntry({ doc: m.doc, valueName: m.yLabel, result: RES, minFrequency: null, maxFrequency: null, fold: null }),
    );

    hooks.mount();
    const fold = { periodDays: 3, t0: 2460311 };
    lightCurveStore.patchView(G, { fold });
    let ranged = render(model());
    type(ranged, "longest period d", "10");
    ranged = render(model());
    backend.search.mockResolvedValueOnce(RES);
    click(searchButton(ranged));
    await settle();
    ranged = render(model());
    type(ranged, "longest period d", "20");
    ranged = render(model());
    click(button(ranged, "Log period search"));
    expect(vi.mocked(measurementLog.append).mock.calls[1][0]).toEqual(
      periodogramEntry({ doc: m.doc, valueName: m.yLabel, result: RES, minFrequency: 0.1, maxFrequency: null, fold }),
    );
  });
});
