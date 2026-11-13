import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { ReactElement } from "react";

vi.mock("react", async (io) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await io()));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { lightCurveStore } from "../../../hooks/useLightCurveStore";
import { jdOffsetLabel } from "../../../utils/differentialPhotometry";
import { measurementLog } from "../../../utils/measurementLog";
import { lightCurveModel, quickLookOf } from "../../../utils/lightCurveViewModel";
import { quickLookEntry } from "../../../utils/lightCurveDoc";
import { QUICK_LOOK_LABEL } from "../../../utils/lightCurveAnalysis";
import { formatWithUncertainty } from "../../../utils/formatUncertainty";
import { WINDOW_NEEDS_BOTH } from "../../../utils/lightCurveInputs";
import type { LightCurveDoc, LightCurveWindow } from "../../../shared/types/lightCurve";
import type { LightCurveViewProps } from "../LightCurveView";

const { default: LightCurveView, NO_POINTS_HINT } = await import("../LightCurveView");

const G = "frames:C:/run/f0.fits";
const NONE = "frames:none";

type Props = Record<string, unknown>;

const view = (LightCurveView as unknown as { type: (p: LightCurveViewProps) => unknown }).type;
const show = (props: LightCurveViewProps) => hooks.render(view, props);

function plotOf(tree: unknown): Props | undefined {
  return elementsOf<Props>(tree)
    .map((e) => e.props)
    .find((p) => typeof p.xLabel === "string");
}

function selectOf(tree: unknown): ReactElement<Props> | undefined {
  return elementsOf<Props>(tree).find((e) => e.type === "select");
}

function optionTexts(tree: unknown): string[] {
  return elementsOf<Props>(selectOf(tree)?.props.children)
    .filter((e) => e.type === "option")
    .map((e) => textOf(e.props.children));
}

function mag(id: string, label: string, extra: Partial<LightCurveDoc> = {}): LightCurveDoc {
  return {
    id,
    label,
    quantity: "diff_mag",
    unit: "mag",
    timeScale: "bjd_tdb",
    time: [2460310.5, 2460310.6, 2460310.7],
    value: [1.5, 1.6, 1.7],
    err: [0.01, 0.01, 0.01],
    pointRef: [0, 1, 2],
    flags: [[], [], []],
    source: { kind: "frames", reference: "f0.fits", target: "T", comps: ["C1", "C2"] },
    notes: [],
    magErrFactor: 1.0857,
    ...extra,
  };
}

function raw(id: string, label: string): LightCurveDoc {
  return {
    id,
    label,
    quantity: "flux",
    unit: null,
    timeScale: "bjd_tdb",
    time: [2460310.5, 2460310.6, 2460310.7],
    value: [1000, 1010, 990],
    err: [10, 10, 10],
    pointRef: [0, 1, 2],
    flags: [[], [], []],
    source: { kind: "frames", reference: "f0.fits", target: label, comps: [] },
    notes: [],
    overlay: "raw comps",
  };
}

function storeTargetAndOverlay(): void {
  lightCurveStore.put(G, [mag("target", "target T"), mag("check:3", "check star K"), raw("raw:1", "C1"), raw("raw:2", "C2")]);
}

describe("LightCurveView", () => {
  beforeEach(() => {
    hooks.mount();
    lightCurveStore.remove(G);
    lightCurveStore.remove(NONE);
  });

  afterEach(() => {
    hooks.mount();
    lightCurveStore.remove(G);
    lightCurveStore.remove(NONE);
  });

  it("offers one choice per document and one per overlay and draws the first", () => {
    storeTargetAndOverlay();
    const tree = show({ group: G });
    expect(optionTexts(tree)).toEqual(["target T", "check star K", "raw comps"]);
    expect(selectOf(tree)?.props.value).toBe("target");
    const plot = plotOf(tree)!;
    const series = plot.series as { x: number[]; y: number[]; yErr: number[]; color: string; label: string; mode: string }[];
    expect(series).toHaveLength(1);
    expect(series[0].label).toBe("target T");
    expect(series[0].color).toBe("#fbbf24");
    expect(series[0].mode).toBe("points");
    expect(series[0].x).toEqual([2460310.5 - 2460310, 2460310.6 - 2460310, 2460310.7 - 2460310]);
    [0.5, 0.6, 0.7].forEach((x, i) => expect(series[0].x[i]).toBeCloseTo(x, 9));
    expect(series[0].y).toEqual([1.5, 1.6, 1.7]);
    expect(series[0].yErr).toEqual([0.01, 0.01, 0.01]);
    expect(plot.xLabel).toBe("BJD_TDB - 2460310");
    expect(plot.yLabel).toBe("diff mag (sum of 2 comps)");
    expect(plot.invertY).toBe(true);
    expect(plot.height).toBe(170);
    expect(plot.toolbar).toBe(true);
    expect(plot.logToggle).toBe(false);
    expect(plot.wheelZoom).toBe("modifier");
    expect(plot.csvName).toBe("lightcurve");
    expect(plot.xTickFormat).toBe(jdOffsetLabel);
  });

  it("choosing the overlay stores the selection and draws every document of it", () => {
    storeTargetAndOverlay();
    const tree = show({ group: G });
    (selectOf(tree)!.props.onChange as (e: unknown) => void)({ target: { value: "overlay:raw comps" } });
    expect(lightCurveStore.getView(G).selection).toBe("overlay:raw comps");
    const plot = plotOf(show({ group: G }))!;
    const series = plot.series as { label: string; color: string }[];
    expect(series.map((s) => s.label)).toEqual(["C1", "C2"]);
    expect(series.map((s) => s.color)).toEqual(["#fbbf24", "#67e8f9"]);
    expect(plot.yLabel).toBe("flux");
    expect(plot.invertY).toBe(false);
    expect(plot.logToggle).toBe(true);
  });

  it("a selection that is gone falls back to the first choice", () => {
    storeTargetAndOverlay();
    lightCurveStore.patchView(G, { selection: "check:9" });
    const tree = show({ group: G });
    expect(selectOf(tree)?.props.value).toBe("target");
    expect((plotOf(tree)!.series as { label: string }[])[0].label).toBe("target T");
  });

  it("a click on a point reports the document and its point reference", () => {
    lightCurveStore.put(G, [mag("target", "target T", { pointRef: [4, 5, 6] }), mag("check:3", "check star K", { pointRef: [4, 5, 6] })]);
    const onSelectPoint = vi.fn();
    const plot = plotOf(show({ group: G, onSelectPoint }))!;
    const onSelect = plot.onSelect as (hit: unknown) => void;
    onSelect({ seriesIndex: 0, index: 2, x: 0, y: 0 });
    expect(onSelectPoint).toHaveBeenCalledTimes(1);
    expect(onSelectPoint.mock.calls[0][0]).toBe(lightCurveStore.get(G)[0]);
    expect(onSelectPoint.mock.calls[0][1]).toBe(6);
    onSelect({ seriesIndex: 3, index: 0, x: 0, y: 0 });
    expect(onSelectPoint).toHaveBeenCalledTimes(1);
    hooks.mount();
    const bare = plotOf(show({ group: G }))!;
    expect(() => (bare.onSelect as (hit: unknown) => void)({ seriesIndex: 0, index: 2, x: 0, y: 0 })).not.toThrow();
  });

  it("the point of the frame on screen is marked on the first series", () => {
    lightCurveStore.put(G, [mag("target", "target T", { pointRef: [4, 5, 6] })]);
    expect(plotOf(show({ group: G, selectedPointRef: 5 }))!.selected).toEqual({ seriesIndex: 0, index: 1 });
    expect(plotOf(show({ group: G, selectedPointRef: 9 }))!.selected).toBeNull();
    expect(plotOf(show({ group: G, selectedPointRef: null }))!.selected).toBeNull();
    expect(plotOf(show({ group: G }))!.selected).toBeNull();
  });

  it("an index axis has no tick formatter and a custom file name reaches the plot", () => {
    lightCurveStore.put(G, [mag("target", "target T", { timeScale: "index", time: [0, 1, 2] })]);
    const plot = plotOf(show({ group: G, csvName: "wasp12" }))!;
    expect(plot.xTickFormat).toBeUndefined();
    expect(plot.xLabel).toBe("frame index");
    expect(plot.csvName).toBe("wasp12");
  });

  it("says so when no point has a value", () => {
    lightCurveStore.put(G, [mag("target", "target T", { value: [null, null, null] })]);
    const tree = show({ group: G });
    expect(textOf(tree)).toContain(NO_POINTS_HINT);
    expect(plotOf(tree)).toBeUndefined();
    expect(selectOf(tree)).toBeDefined();
  });

  it("renders nothing for a group without documents", () => {
    expect(show({ group: NONE })).toBeNull();
  });
});

const E = "tso:test:NRS1:0";

interface WhiteLightFixture {
  int_num: number[];
  bmjd_tdb: number[];
  flux: number[];
  flux_err: number[];
}

const E1_FIXTURE: WhiteLightFixture = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../../../src-tauri/tests/fixtures/lightcurve/e1_white_light.json", import.meta.url)), "utf8"),
);

function e1(): LightCurveDoc {
  return {
    id: "white",
    label: "white light NRS1",
    quantity: "flux",
    unit: "Jy",
    timeScale: "bmjd_tdb",
    time: [...E1_FIXTURE.bmjd_tdb],
    value: [...E1_FIXTURE.flux],
    err: [...E1_FIXTURE.flux_err],
    pointRef: [...E1_FIXTURE.int_num],
    flags: E1_FIXTURE.flux.map(() => []),
    source: { kind: "tso", paths: ["white.fits"], detector: "NRS1", sporder: null, band_um: null, mode: "pipeline" },
    notes: [],
  };
}

const W1: LightCurveWindow = { start: E1_FIXTURE.bmjd_tdb[182], end: E1_FIXTURE.bmjd_tdb[187] };
const TYPED_WINDOW: LightCurveWindow = { start: 60061.50682013, end: 60061.50859614 };
const EIGHT_TIMES = Array.from({ length: 8 }, (_, k) => 2460310.5 + k / 10);

function eight(flagOfPointOne: string[]): LightCurveDoc {
  return mag("target", "target T", {
    time: [...EIGHT_TIMES],
    value: EIGHT_TIMES.map(() => 1.5),
    err: EIGHT_TIMES.map(() => 0.01),
    pointRef: EIGHT_TIMES.map((_, k) => k),
    flags: EIGHT_TIMES.map((_, k) => (k === 1 ? flagOfPointOne : [])),
  });
}

function field(tree: unknown, text: string): Props | undefined {
  const all = elementsOf<Props>(tree);
  const label = all.find((e) => e.type === "label" && textOf(e.props.children) === text);
  if (label === undefined) return undefined;
  return all.find((e) => e.type !== "label" && e.props.id === label.props.htmlFor)?.props;
}

function typeInto(tree: unknown, label: string, text: string): void {
  (field(tree, label)!.onChange as (t: string) => void)(text);
}

function optionsOf(tree: unknown, label: string): string[] {
  return elementsOf<Props>(field(tree, label)?.children)
    .filter((e) => e.type === "option")
    .map((e) => textOf(e.props.children));
}

function choose(tree: unknown, label: string, value: string): void {
  (field(tree, label)!.onChange as (e: unknown) => void)({ target: { value } });
}

function card(tree: unknown, name: string): string {
  const element = elementsOf<Props>(tree).find((e) => e.props["data-card"] === name);
  return element === undefined ? "" : textOf(element.props.children);
}

function hasCard(tree: unknown, name: string): boolean {
  return elementsOf<Props>(tree).some((e) => e.props["data-card"] === name);
}

function hintOf(tree: unknown): string | undefined {
  const element = elementsOf<Props>(tree).find((e) => e.props["data-window-hint"] !== undefined);
  return element === undefined ? undefined : textOf(element.props.children);
}

function toggleOf(tree: unknown): Props | undefined {
  return elementsOf<Props>(tree)
    .map((e) => e.props)
    .find((p) => typeof p.label === "string" && p.label.startsWith("Leave out flagged points"));
}

function logButton(tree: unknown): Props {
  return elementsOf<Props>(tree).find((e) => e.type === "button" && textOf(e.props.children) === "Log quick look")!.props;
}

type Band = { x0: number; x1: number; color?: string; label?: string };
type Series = { x: number[]; y: (number | null)[]; color: string; label: string; mode: string };

describe("LightCurveView quick look", () => {
  beforeEach(() => {
    hooks.mount();
    lightCurveStore.remove(G);
    lightCurveStore.remove(E);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    measurementLog.clear();
    hooks.mount();
    lightCurveStore.remove(G);
    lightCurveStore.remove(E);
  });

  it("offers the native quantity and normalised flux as displays", () => {
    lightCurveStore.put(G, [mag("target", "target T")]);
    const magTree = show({ group: G });
    expect(optionsOf(magTree, "display")).toEqual(["magnitude", "normalised flux"]);
    expect(field(magTree, "display")!.value).toBe("native");
    hooks.mount();
    lightCurveStore.put(G, [mag("target", "target T", { quantity: "rel_flux", unit: null })]);
    expect(optionsOf(show({ group: G }), "display")).toEqual(["relative flux", "normalised flux"]);

    hooks.mount();
    lightCurveStore.put(E, [e1()]);
    const tree = show({ group: E });
    expect(optionsOf(tree, "display")).toEqual(["flux", "normalised flux"]);
    choose(tree, "display", "normalised");
    expect(lightCurveStore.getView(E).display).toBe("normalised");
    lightCurveStore.patchView(E, { window: W1, marginDays: 0.0044, display: "normalised" });
    const plot = plotOf(show({ group: E }))!;
    expect(plot.yLabel).toBe("normalised flux");
    expect(plot.invertY).toBe(false);
    expect((plot.series as Series[])[0].y[185]).toBeCloseTo(0.03126827293006477, 10);
  });

  it("typing the two edges stores the window in absolute time and shades it", () => {
    lightCurveStore.put(E, [e1()]);
    let tree = show({ group: E });
    typeInto(tree, "transit start", "0.50682013");
    tree = show({ group: E });
    expect(lightCurveStore.getView(E).window).toBeNull();
    expect(hintOf(tree)).toBe(WINDOW_NEEDS_BOTH);
    typeInto(tree, "transit end", "0.50859614");
    const stored = lightCurveStore.getView(E).window!;
    expect(stored.start).toBeCloseTo(60061.50682013, 8);
    expect(stored.end).toBeCloseTo(60061.50859614, 8);
    tree = show({ group: E });
    expect(hintOf(tree)).toBeUndefined();
    const bands = plotOf(tree)!.bands as Band[];
    expect(bands).toHaveLength(1);
    expect(bands[0].x0).toBeCloseTo(0.50682013, 7);
    expect(bands[0].x1).toBeCloseTo(0.50859614, 7);
    expect(bands[0].color).toBe("rgba(251,191,36,0.14)");
    expect(bands[0].label).toBe("in transit");
  });

  it("a draft that cannot be read leaves the stored window alone and says why", () => {
    lightCurveStore.put(E, [e1()]);
    lightCurveStore.patchView(E, { window: TYPED_WINDOW });
    show({ group: E });
    const tree = show({ group: E });
    typeInto(tree, "transit end", "0.4");
    expect(lightCurveStore.getView(E).window).toEqual(TYPED_WINDOW);
    const next = show({ group: E });
    expect(hintOf(next)).toBe("transit end is before transit start");
    expect(field(next, "transit end")!.value).toBe("0.4");
    expect(lightCurveStore.getView(E).window).toEqual(TYPED_WINDOW);
  });

  it("the fields follow a window set elsewhere and keep what is being typed", () => {
    lightCurveStore.put(E, [e1()]);
    lightCurveStore.patchView(E, { window: W1 });
    show({ group: E });
    let tree = show({ group: E });
    expect(field(tree, "transit start")!.value).toBe("0.50682013334");
    expect(field(tree, "transit end")!.value).toBe("0.508596144");
    typeInto(tree, "transit start", "0.506820");
    show({ group: E });
    tree = show({ group: E });
    expect(field(tree, "transit start")!.value).toBe("0.506820");
    expect(lightCurveStore.getView(E).window!.start).toBeCloseTo(60061.50682, 8);
    expect(lightCurveStore.getView(E).window!.end).toBe(W1.end);
    const full = e1();
    const cut: LightCurveDoc = {
      ...full,
      time: full.time.slice(0, 239),
      value: full.value.slice(0, 239),
      err: full.err.slice(0, 239),
      pointRef: full.pointRef.slice(0, 239),
      flags: full.flags.slice(0, 239),
    };
    lightCurveStore.put(E, [cut]);
    show({ group: E });
    tree = show({ group: E });
    expect(field(tree, "transit start")!.value).toBe("");
    expect(field(tree, "transit end")!.value).toBe("");
    expect(lightCurveStore.getView(E).window).toBeNull();
  });

  it("the margin is typed in minutes, or in points on an index axis, and draws two margin bands", () => {
    lightCurveStore.put(E, [e1()]);
    lightCurveStore.patchView(E, { window: W1 });
    let tree = show({ group: E });
    expect(field(tree, "margin points")).toBeUndefined();
    typeInto(tree, "margin min", "6.336");
    expect(lightCurveStore.getView(E).marginDays).toBeCloseTo(0.0044, 12);
    tree = show({ group: E });
    const bands = plotOf(tree)!.bands as Band[];
    expect(bands).toHaveLength(3);
    expect(bands.slice(1).map((b) => b.color)).toEqual(["rgba(161,161,170,0.08)", "rgba(161,161,170,0.08)"]);
    expect(bands.slice(1).map((b) => b.label)).toEqual([undefined, undefined]);

    lightCurveStore.put(G, [mag("target", "target T", { timeScale: "index", time: [0, 1, 2] })]);
    hooks.mount();
    const indexTree = show({ group: G });
    expect(field(indexTree, "margin min")).toBeUndefined();
    typeInto(indexTree, "margin points", "2");
    expect(lightCurveStore.getView(G).marginDays).toBe(2);
  });

  it("the baseline select stores its kind", () => {
    lightCurveStore.put(E, [e1()]);
    const tree = show({ group: E });
    expect(optionsOf(tree, "baseline")).toEqual(["mean", "median", "straight line"]);
    choose(tree, "baseline", "linear");
    expect(lightCurveStore.getView(E).baselineKind).toBe("linear");
  });

  it("the cards print the scatter, the formal error and the depth of the model", () => {
    lightCurveStore.put(E, [e1()]);
    lightCurveStore.patchView(E, { window: W1, marginDays: 0.0044 });
    const tree = show({ group: E });
    const m = lightCurveModel(lightCurveStore.get(E)[0], lightCurveStore.getView(E));
    const scatter = card(tree, "scatter");
    expect(scatter).toContain("Out-of-transit scatter");
    expect(scatter).toContain("927.4 ppm");
    expect(scatter).toContain("1.007 mmag, N = 210, sample standard deviation");
    const formal = card(tree, "formal");
    expect(formal).toContain("Mean formal error");
    expect(formal).toContain("512.8 ppm");
    expect(formal).toContain("scatter / formal = 1.81");
    const depth = card(tree, "depth");
    expect(depth).toContain("Depth (box)");
    expect(depth).toContain(formatWithUncertainty(m.depth!.depth, m.depth!.err));
    expect(depth).toContain("scatter 0.0063, formal 0.00028");
    expect(depth).toContain("N in 6, out 210");
    expect(depth).toContain(QUICK_LOOK_LABEL);
    const points = card(tree, "points");
    expect(points).toContain("240 of 240");
    expect(points).not.toContain("left out");
  });

  it("without a window the scatter is over every kept point and there is no depth card", () => {
    lightCurveStore.put(E, [e1()]);
    const tree = show({ group: E });
    expect(card(tree, "scatter")).toContain("Scatter (all kept points)");
    expect(card(tree, "scatter")).not.toContain("Out-of-transit");
    expect(hasCard(tree, "depth")).toBe(false);
    const button = logButton(tree);
    expect(button.disabled).toBe(true);
    expect(button.title).toBe("Set a transit window first");
  });

  it("a curve without errors says so on the formal card", () => {
    const values = [1.5, 1.51, 1.49, 1.6, 1.62, 1.5, 1.52, 1.48];
    lightCurveStore.put(G, [{ ...eight([]), value: values, err: values.map(() => null) }]);
    lightCurveStore.patchView(G, { window: { start: EIGHT_TIMES[3], end: EIGHT_TIMES[4] } });
    const tree = show({ group: G });
    expect(card(tree, "formal")).toContain("--");
    expect(card(tree, "formal")).toContain("not every point has an error");
    expect(card(tree, "depth")).toContain("formal --");
  });

  it("flagged points are left out by default, drawn dimmed, and the switch brings them back", () => {
    lightCurveStore.put(G, [eight(["saturated"])]);
    let tree = show({ group: G });
    const toggle = toggleOf(tree)!;
    expect(toggle.label).toBe("Leave out flagged points (1)");
    expect(toggle.checked).toBe(true);
    const series = plotOf(tree)!.series as Series[];
    expect(series).toHaveLength(2);
    expect(series[1].label).toBe("target T (left out)");
    expect(series[1].color).toBe("rgba(161,161,170,0.45)");
    expect(series[1].y).toEqual([null, 1.5, null, null, null, null, null, null]);
    expect(card(tree, "points")).toContain("7 of 8");
    expect(card(tree, "points")).toContain("left out: 1 flagged");
    (toggle.onChange as (v: boolean) => void)(false);
    expect(lightCurveStore.getView(G).excludeFlagged).toBe(false);
    tree = show({ group: G });
    expect(plotOf(tree)!.series as Series[]).toHaveLength(1);
    expect(card(tree, "points")).toContain("8 of 8");
    expect(card(tree, "points")).not.toContain("left out");
    expect(toggleOf(tree)!.checked).toBe(false);

    hooks.mount();
    lightCurveStore.put(G, [eight(["pixel loss"])]);
    expect(toggleOf(show({ group: G }))).toBeUndefined();
  });

  it("a click still reports the right point when a dimmed series sits between the curves", () => {
    lightCurveStore.put(G, [
      { ...raw("raw:1", "C1"), flags: [[], ["saturated"], []] },
      { ...raw("raw:2", "C2"), pointRef: [10, 11, 12] },
    ]);
    lightCurveStore.patchView(G, { selection: "overlay:raw comps" });
    const onSelectPoint = vi.fn();
    const plot = plotOf(show({ group: G, onSelectPoint }))!;
    expect((plot.series as Series[]).map((s) => s.label)).toEqual(["C1", "C1 (left out)", "C2"]);
    const onSelect = plot.onSelect as (hit: unknown) => void;
    onSelect({ seriesIndex: 2, index: 1, x: 0, y: 0 });
    expect(onSelectPoint.mock.calls[0][0]).toBe(lightCurveStore.get(G)[1]);
    expect(onSelectPoint.mock.calls[0][1]).toBe(11);
    onSelect({ seriesIndex: 1, index: 1, x: 0, y: 0 });
    expect(onSelectPoint.mock.calls[1][0]).toBe(lightCurveStore.get(G)[0]);
    expect(onSelectPoint.mock.calls[1][1]).toBe(1);
  });

  it("notes of the model are shown once each", () => {
    lightCurveStore.put(G, [eight(["pixel loss"])]);
    lightCurveStore.patchView(G, { window: { start: EIGHT_TIMES[1], end: EIGHT_TIMES[1] } });
    const notes = elementsOf<Props>(show({ group: G })).filter((e) => e.props["data-curve-note"] !== undefined);
    expect(notes.map((e) => textOf(e.props.children))).toEqual(["1 of 1 in-transit points are flagged: pixel loss"]);
  });

  it("Log quick look writes the entry of the model", () => {
    const append = vi.spyOn(measurementLog, "append");
    lightCurveStore.put(E, [e1()]);
    lightCurveStore.patchView(E, { window: W1, marginDays: 0.0044 });
    const button = logButton(show({ group: E }));
    expect(button.disabled).toBe(false);
    expect(button.title).toBe("Add the depth, its two errors, the scatter and the window to the measurement log");
    (button.onClick as () => void)();
    const doc = lightCurveStore.get(E)[0];
    const view = lightCurveStore.getView(E);
    expect(append).toHaveBeenCalledTimes(1);
    expect(append.mock.calls[0][0]).toEqual(quickLookEntry(quickLookOf(lightCurveModel(doc, view), view)!));
    expect(append.mock.calls[0][0].values.box_depth).toBeCloseTo(0.9503918297804989, 12);
    expect(append.mock.calls[0][0].notes[0]).toBe(QUICK_LOOK_LABEL);
  });

  it("an overlay shows one table row per curve", () => {
    lightCurveStore.put(G, [raw("raw:1", "C1"), raw("raw:2", "C2")]);
    lightCurveStore.patchView(G, { selection: "overlay:raw comps", window: { start: 2460310.6, end: 2460310.6 } });
    const tree = show({ group: G });
    const table = elementsOf<Props>(tree).find((e) => e.props["data-curve-table"] !== undefined)!;
    expect(table).toBeDefined();
    const cells = elementsOf<Props>(table);
    expect(cells.filter((e) => e.type === "th").map((e) => textOf(e.props.children))).toEqual(["series", "N", "scatter ppm", "depth"]);
    const body = cells.find((e) => e.type === "tbody")!;
    const rows = elementsOf<Props>(body.props.children).filter((e) => e.type === "tr");
    expect(rows).toHaveLength(2);
    expect(rows.map((row) => textOf(elementsOf<Props>(row.props.children).find((e) => e.type === "td")!.props.children))).toEqual(["C1", "C2"]);
    expect(elementsOf<Props>(tree).some((e) => e.props["data-card"] !== undefined)).toBe(false);
  });
});

type BinnedPlotSeries = Series & { yErr: (number | null)[] };

function clickToggleOf(tree: unknown): Props | undefined {
  return elementsOf<Props>(tree)
    .map((e) => e.props)
    .find((p) => p.label === "Click a point to leave it out");
}

function putBackButton(tree: unknown): Props | undefined {
  return elementsOf<Props>(tree).find((e) => e.type === "button" && textOf(e.props.children).startsWith("Put "))?.props;
}

function leftOutPositions(series: Series): number[] {
  return series.y.flatMap((value, i) => (value === null ? [] : [i]));
}

function clickPlot(tree: unknown, seriesIndex: number, index: number): void {
  (plotOf(tree)!.onSelect as (hit: unknown) => void)({ seriesIndex, index, x: 0, y: 0 });
}

describe("LightCurveView binning, clipping and points left out by hand", () => {
  beforeEach(() => {
    hooks.mount();
    lightCurveStore.remove(G);
    lightCurveStore.remove(E);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    measurementLog.clear();
    hooks.mount();
    lightCurveStore.remove(G);
    lightCurveStore.remove(E);
  });

  it("binning by points draws the binned curve over the dimmed points and adds its card", () => {
    lightCurveStore.put(E, [e1()]);
    lightCurveStore.patchView(E, { window: W1, marginDays: 0.0044, display: "normalised" });
    show({ group: E });
    let tree = show({ group: E });
    expect(optionsOf(tree, "bin")).toEqual(["no bins", "by points", "by minutes"]);
    choose(tree, "bin", "points");
    tree = show({ group: E });
    typeInto(tree, "bin size", "10");
    expect(lightCurveStore.getView(E).bin).toEqual({ kind: "points", points: 10 });
    tree = show({ group: E });
    const series = plotOf(tree)!.series as BinnedPlotSeries[];
    expect(series).toHaveLength(2);
    expect(series[0].color).toBe("#fbbf2466");
    expect(series[1].label).toBe("white light NRS1 (10 points)");
    expect(series[1].mode).toBe("both");
    expect(series[1].color).toBe("#fbbf24");
    expect(series[1].x).toHaveLength(24);
    expect(series[1].y).toHaveLength(24);
    expect(series[1].y[0]).toBeCloseTo(0.999936467184672, 10);
    expect(series[1].yErr[0]).toBeCloseTo(0.00016215528303869324, 10);
    const binned = card(tree, "binned");
    expect(binned).toContain("Binned (10 points)");
    expect(binned).toContain("24 bins");
    expect(binned).toContain("out-of-transit scatter 305.2 ppm, N = 21");
    const m = lightCurveModel(lightCurveStore.get(E)[0], lightCurveStore.getView(E));
    const depth = card(tree, "depth");
    expect(depth).toContain(formatWithUncertainty(m.depth!.depth, m.depth!.err));
    expect(depth).toContain("scatter 0.0063, formal 0.00028");
    expect(depth).toContain("N in 6, out 210");
  });

  it("binning by minutes is refused on an index axis", () => {
    lightCurveStore.put(G, [mag("target", "target T", { timeScale: "index", time: [0, 1, 2] })]);
    const indexTree = show({ group: G });
    const minutes = elementsOf<Props>(field(indexTree, "bin")?.children).find((e) => e.type === "option" && e.props.value === "minutes")!;
    expect(minutes.props.disabled).toBe(true);
    expect(field(indexTree, "bin size")!.disabled).toBe(true);

    hooks.mount();
    lightCurveStore.put(E, [e1()]);
    const tree = show({ group: E });
    const timeMinutes = elementsOf<Props>(field(tree, "bin")?.children).find((e) => e.type === "option" && e.props.value === "minutes")!;
    expect(timeMinutes.props.disabled).toBeFalsy();
    choose(tree, "bin", "minutes");
    expect(lightCurveStore.getView(E).bin).toEqual({ kind: "minutes", minutes: 5 });
    const next = show({ group: E });
    expect(field(next, "bin size")!.disabled).toBeFalsy();
    expect(card(next, "binned")).toContain("Binned (5 min)");
    expect((plotOf(next)!.series as Series[])[1].label).toBe("white light NRS1 (5 min)");
  });

  it("a bin size that cannot be read keeps the curve unbinned and says why", () => {
    lightCurveStore.put(E, [e1()]);
    let tree = show({ group: E });
    choose(tree, "bin", "points");
    tree = show({ group: E });
    typeInto(tree, "bin size", "1");
    show({ group: E });
    tree = show({ group: E });
    expect(lightCurveStore.getView(E).bin).toEqual({ kind: "none" });
    expect(hintOf(tree)).toBe("bin size must be a whole number of at least 2");
    expect(field(tree, "bin")!.value).toBe("points");
    expect(field(tree, "bin size")!.value).toBe("1");
    const series = plotOf(tree)!.series as Series[];
    expect(series).toHaveLength(1);
    expect(series[0].color).toBe("#fbbf24");
    expect(hasCard(tree, "binned")).toBe(false);
  });

  it("clipping marks the outliers as left out and the cards follow", () => {
    lightCurveStore.put(E, [e1()]);
    lightCurveStore.patchView(E, { window: W1, marginDays: 0.0044 });
    let tree = show({ group: E });
    expect(field(tree, "clip sigma")!.placeholder).toBe("off");
    expect(field(tree, "clip window")!.placeholder).toBe("11");
    expect(field(tree, "clip window")!.integer).toBe(true);
    typeInto(tree, "clip sigma", "5");
    expect(lightCurveStore.getView(E).clipSigma).toBe(5);
    expect(lightCurveStore.getView(E).clipWindowPoints).toBe(11);
    tree = show({ group: E });
    const series = plotOf(tree)!.series as Series[];
    expect(series[1].label).toBe("white light NRS1 (left out)");
    expect(leftOutPositions(series[1])).toEqual([125, 210, 212]);
    expect(card(tree, "points")).toContain("237 of 240");
    expect(card(tree, "points")).toContain("left out: 3 clipped");
    expect(card(tree, "scatter")).toContain("635.2 ppm");
    expect(card(tree, "scatter")).toContain("N = 207");
    typeInto(tree, "clip sigma", "");
    expect(lightCurveStore.getView(E).clipSigma).toBeNull();
  });

  it("in click mode a click leaves a point out and a second click puts it back", () => {
    lightCurveStore.put(G, [eight([])]);
    const onSelectPoint = vi.fn();
    let tree = show({ group: G, onSelectPoint });
    const toggle = clickToggleOf(tree)!;
    expect(toggle.checked).toBe(false);
    expect(toggle.accent).toBe("amber");
    (toggle.onChange as (v: boolean) => void)(true);
    expect(lightCurveStore.getView(G).clickExcludes).toBe(true);
    tree = show({ group: G, onSelectPoint });
    clickPlot(tree, 0, 3);
    expect(lightCurveStore.getView(G).excluded).toEqual([3]);
    expect(onSelectPoint).not.toHaveBeenCalled();
    tree = show({ group: G, onSelectPoint });
    const series = plotOf(tree)!.series as Series[];
    expect(leftOutPositions(series[1])).toEqual([3]);
    expect(card(tree, "points")).toContain("left out: 1 by hand");
    clickPlot(tree, 0, 5);
    expect(lightCurveStore.getView(G).excluded).toEqual([3, 5]);
    tree = show({ group: G, onSelectPoint });
    clickPlot(tree, 1, 3);
    expect(lightCurveStore.getView(G).excluded).toEqual([5]);
    expect(onSelectPoint).not.toHaveBeenCalled();
  });

  it("a flagged or clipped point is not toggled by a click", () => {
    lightCurveStore.put(G, [eight(["saturated"])]);
    lightCurveStore.patchView(G, { clickExcludes: true });
    const onSelectPoint = vi.fn();
    const tree = show({ group: G, onSelectPoint });
    clickPlot(tree, 1, 1);
    expect(lightCurveStore.getView(G).excluded).toEqual([]);
    expect(onSelectPoint).not.toHaveBeenCalled();
  });

  it("Put points back clears the manual exclusions", () => {
    lightCurveStore.put(G, [eight([])]);
    expect(putBackButton(show({ group: G }))).toBeUndefined();
    lightCurveStore.patchView(G, { excluded: [2, 6] });
    const button = putBackButton(show({ group: G }))!;
    expect(textOf(button.children)).toBe("Put 2 points back");
    (button.onClick as () => void)();
    expect(lightCurveStore.getView(G).excluded).toEqual([]);
    expect(putBackButton(show({ group: G }))).toBeUndefined();
    lightCurveStore.patchView(G, { excluded: [4] });
    expect(textOf(putBackButton(show({ group: G }))!.children)).toBe("Put 1 point back");
  });

  it("with click mode off a click reports the point as before", () => {
    lightCurveStore.put(E, [e1()]);
    lightCurveStore.patchView(E, { bin: { kind: "points", points: 10 } });
    const onSelectPoint = vi.fn();
    let tree = show({ group: E, onSelectPoint });
    expect((plotOf(tree)!.series as Series[])[1].label).toBe("white light NRS1 (10 points)");
    clickPlot(tree, 1, 0);
    expect(onSelectPoint).not.toHaveBeenCalled();
    expect(lightCurveStore.getView(E).excluded).toEqual([]);
    clickPlot(tree, 0, 7);
    expect(onSelectPoint).toHaveBeenCalledTimes(1);
    expect(onSelectPoint.mock.calls[0][1]).toBe(E1_FIXTURE.int_num[7]);
    lightCurveStore.patchView(E, { clickExcludes: true });
    tree = show({ group: E, onSelectPoint });
    clickPlot(tree, 1, 0);
    expect(lightCurveStore.getView(E).excluded).toEqual([]);
    expect(onSelectPoint).toHaveBeenCalledTimes(1);
  });

  it("choosing a series of another length clears the manual exclusions", () => {
    const five = mag("five", "five points", {
      time: EIGHT_TIMES.slice(0, 5),
      value: [1.5, 1.6, 1.7, 1.6, 1.5],
      err: [0.01, 0.01, 0.01, 0.01, 0.01],
      pointRef: [0, 1, 2, 3, 4],
      flags: [[], [], [], [], []],
    });
    lightCurveStore.put(G, [mag("three", "three points"), five, mag("other", "other three")]);
    lightCurveStore.patchView(G, { excluded: [1] });
    let tree = show({ group: G });
    choose(tree, "series", "other");
    expect(lightCurveStore.getView(G).selection).toBe("other");
    expect(lightCurveStore.getView(G).excluded).toEqual([1]);
    tree = show({ group: G });
    choose(tree, "series", "five");
    expect(lightCurveStore.getView(G).selection).toBe("five");
    expect(lightCurveStore.getView(G).excluded).toEqual([]);
  });

  it("the logged quick look carries the binning and the counts", () => {
    const append = vi.spyOn(measurementLog, "append");
    lightCurveStore.put(E, [e1()]);
    lightCurveStore.patchView(E, { window: W1, marginDays: 0.0044, clipSigma: 5, bin: { kind: "points", points: 10 } });
    (logButton(show({ group: E })).onClick as () => void)();
    expect(append).toHaveBeenCalledTimes(1);
    const entry = append.mock.calls[0][0];
    expect(entry.params.binned).toBe("10 points");
    expect(entry.params.n_clipped).toBe(3);
    expect(entry.params.n_excluded).toBe(0);
    expect(entry.values.box_depth).toBeCloseTo(0.9503957291573791, 9);
  });
});

describe("LightCurveView transit fit", () => {
  beforeEach(() => {
    hooks.mount();
    lightCurveStore.remove(G);
  });

  afterEach(() => {
    hooks.mount();
    lightCurveStore.remove(G);
  });

  it("the view offers the transit fit for one curve and not for an overlay", async () => {
    const { default: TransitFitSection } = await import("../TransitFitSection");
    storeTargetAndOverlay();
    const single = elementsOf<Props>(show({ group: G })).filter((e) => e.type === TransitFitSection);
    expect(single).toHaveLength(1);
    expect(single[0].props.group).toBe(G);
    expect(single[0].props.csvName).toBe("lightcurve");
    expect(single[0].props.model).toEqual(lightCurveModel(lightCurveStore.get(G)[0], lightCurveStore.getView(G)));
    lightCurveStore.patchView(G, { selection: "overlay:raw comps" });
    hooks.mount();
    expect(elementsOf<Props>(show({ group: G })).filter((e) => e.type === TransitFitSection)).toHaveLength(0);
  });
});

describe("LightCurveView period search", () => {
  beforeEach(() => {
    hooks.mount();
    lightCurveStore.remove(G);
  });

  afterEach(() => {
    hooks.mount();
    lightCurveStore.remove(G);
  });

  it("the view offers the period search for one curve and not for an overlay", async () => {
    const { default: PeriodogramSection } = await import("../PeriodogramSection");
    const { default: TransitFitSection } = await import("../TransitFitSection");
    storeTargetAndOverlay();
    const tree = show({ group: G, csvName: "target_lc" });
    const elements = elementsOf<Props>(tree);
    const single = elements.filter((e) => e.type === PeriodogramSection);
    expect(single).toHaveLength(1);
    expect(single[0].props.group).toBe(G);
    expect(single[0].props.csvName).toBe("target_lc");
    expect(single[0].props.model).toEqual(lightCurveModel(lightCurveStore.get(G)[0], lightCurveStore.getView(G)));
    const transitAt = elements.findIndex((e) => e.type === TransitFitSection);
    expect(transitAt).toBeGreaterThanOrEqual(0);
    expect(transitAt).toBeLessThan(elements.indexOf(single[0]));
    const unnamed = elementsOf<Props>(show({ group: G })).filter((e) => e.type === PeriodogramSection);
    expect(unnamed[0].props.csvName).toBe("lightcurve");
    lightCurveStore.patchView(G, { selection: "overlay:raw comps" });
    hooks.mount();
    expect(elementsOf<Props>(show({ group: G })).filter((e) => e.type === PeriodogramSection)).toHaveLength(0);
  });
});
