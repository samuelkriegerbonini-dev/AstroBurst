import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react", async (io) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await io()));

const plugins = vi.hoisted(() => ({ writeFile: vi.fn<(path: string, bytes: Uint8Array) => Promise<void>>(async () => {}) }));

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async () => "C:/out/plot.png" }));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeFile: plugins.writeFile }));

import { hooks } from "../../../test/hookRuntime";
import { plotSummary } from "../../../utils/plotInteraction";

const { default: ProfilePlot } = await import("../ProfilePlot");
type ProfilePlotProps = import("../ProfilePlot").ProfilePlotProps;
type ProfileSeries = import("../ProfilePlot").ProfileSeries;

type Call = [string, unknown[], unknown, unknown];

const AXIS_STROKE = "rgba(161,161,170,0.35)";

const round = (v: unknown): unknown => {
  if (Array.isArray(v)) return v.map(round);
  if (typeof v !== "number") return v;
  const r = Math.round(v * 1000) / 1000;
  return r === 0 ? 0 : r;
};

function recordingContext() {
  const state: Record<string, unknown> = {};
  const calls: Call[] = [];
  const aligns: unknown[] = [];
  const ctx = new Proxy(state, {
    get(target, name) {
      if (typeof name !== "string") return undefined;
      if (name === "measureText") return (text: string) => ({ width: text.length * 5.4 });
      if (name in target) return target[name];
      return (...args: unknown[]) => {
        if (name === "fillText") aligns.push(target.textAlign ?? null);
        calls.push([name, args.map(round), target.fillStyle ?? null, target.strokeStyle ?? null]);
      };
    },
    set(target, name, value) {
      target[name as string] = value;
      return true;
    },
  });
  return { ctx, calls, aligns };
}

const component = (ProfilePlot as unknown as { type: (p: ProfilePlotProps) => unknown }).type;

function renderPlot(props: ProfilePlotProps, width = 323) {
  const base = recordingContext();
  const overlay = recordingContext();
  const listeners: Record<string, (e: unknown) => void> = {};
  hooks.mount();
  hooks.run.cells[0] = { value: { current: { clientWidth: width } }, deps: [] };
  hooks.run.cells[1] = { value: { current: { getContext: () => base.ctx, toBlob() {} } }, deps: [] };
  hooks.run.cells[2] = {
    value: {
      current: {
        getContext: () => overlay.ctx,
        addEventListener(type: string, fn: (e: unknown) => void) {
          listeners[type] = fn;
        },
        removeEventListener() {},
        getBoundingClientRect: () => ({ left: 0, top: 0 }),
      },
    },
    deps: [],
  };
  hooks.render(component, props);
  const tree = hooks.render(component, props);
  return { tree, calls: base.calls, aligns: base.aligns, listeners };
}

function draw(props: ProfilePlotProps, width = 323): Call[] {
  return renderPlot(props, width).calls;
}

const texts = (calls: Call[]) =>
  calls.filter((c) => c[0] === "fillText").map((c) => ({ text: c[1][0] as string, x: c[1][1] as number, y: c[1][2] as number }));

const axisStart = (calls: Call[]) => calls.find((c) => c[0] === "moveTo" && c[3] === AXIS_STROKE)?.[1];

const line = (label: string, color = "#fff"): ProfileSeries => ({ x: [1, 2, 3], y: [1, 2, 3], color, label });

const X1D_LABELS = ["jw02734-o002_t001_nirspec_g395h-f290lp_x1d.fits", "jw02734-o003_t001_nirspec_g395h-f290lp_x1d.fits"];

const ZERO_POINT_SERIES: ProfileSeries[] = ["matched stars", "mag - ZP (25.123)", "outside fit", "no BP-RP (not in colour fit)"].map(
  (label) => ({ ...line(label), mode: "points" }),
);

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("window", { devicePixelRatio: 1, setTimeout });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const PARITY_PROPS: ProfilePlotProps = {
  series: [
    { x: [0, 1, 2, 3], y: [1, 2, 1.5, 2.5], color: "#fbbf24", label: "target", mode: "points", yErr: [0.1, 0.1, null, 0.2] },
    { x: [0, 1, 2, 3], y: [2, 2, 2, 2], color: "#67e8f9", label: "model" },
  ],
  xLabel: "time",
  yLabel: "flux",
  height: 160,
  referenceLines: [{ axis: "y", value: 2, label: "ref" }],
};

const PARITY_GOLDEN: Call[] = [
  ["setTransform",[1,0,0,1,0,0],null,null],
  ["clearRect",[0,0,323,160],null,null],
  ["beginPath",[],null,"rgba(161,161,170,0.12)"],
  ["moveTo",[52,134.5],null,"rgba(161,161,170,0.12)"],
  ["lineTo",[313,134.5],null,"rgba(161,161,170,0.12)"],
  ["stroke",[],null,"rgba(161,161,170,0.12)"],
  ["beginPath",[],null,"rgba(161,161,170,0.12)"],
  ["moveTo",[52,109.5],null,"rgba(161,161,170,0.12)"],
  ["lineTo",[313,109.5],null,"rgba(161,161,170,0.12)"],
  ["stroke",[],null,"rgba(161,161,170,0.12)"],
  ["beginPath",[],null,"rgba(161,161,170,0.12)"],
  ["moveTo",[52,84.5],null,"rgba(161,161,170,0.12)"],
  ["lineTo",[313,84.5],null,"rgba(161,161,170,0.12)"],
  ["stroke",[],null,"rgba(161,161,170,0.12)"],
  ["beginPath",[],null,"rgba(161,161,170,0.12)"],
  ["moveTo",[52,58.5],null,"rgba(161,161,170,0.12)"],
  ["lineTo",[313,58.5],null,"rgba(161,161,170,0.12)"],
  ["stroke",[],null,"rgba(161,161,170,0.12)"],
  ["beginPath",[],null,"rgba(161,161,170,0.12)"],
  ["moveTo",[52,33.5],null,"rgba(161,161,170,0.12)"],
  ["lineTo",[313,33.5],null,"rgba(161,161,170,0.12)"],
  ["stroke",[],null,"rgba(161,161,170,0.12)"],
  ["beginPath",[],null,"rgba(161,161,170,0.12)"],
  ["moveTo",[52,8.5],null,"rgba(161,161,170,0.12)"],
  ["lineTo",[313,8.5],null,"rgba(161,161,170,0.12)"],
  ["stroke",[],null,"rgba(161,161,170,0.12)"],
  ["beginPath",[],null,"rgba(161,161,170,0.35)"],
  ["moveTo",[52.5,8],null,"rgba(161,161,170,0.35)"],
  ["lineTo",[52.5,134.5],null,"rgba(161,161,170,0.35)"],
  ["lineTo",[313,134.5],null,"rgba(161,161,170,0.35)"],
  ["stroke",[],null,"rgba(161,161,170,0.35)"],
  ["fillText",["0.5",48,134],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["1.0",48,108.8],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["1.5",48,83.6],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["2.0",48,58.4],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["2.5",48,33.2],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["3.0",48,8],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["0",52,137],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["0.5",95.5,137],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["1.0",139,137],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["1.5",182.5,137],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["2.0",226,137],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["2.5",269.5,137],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["3.0",313,137],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["time",182.5,149],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["save",[],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["translate",[9,71],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["rotate",[-1.571],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["fillText",["flux",0,0],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["restore",[],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["save",[],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["beginPath",[],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["rect",[52,8,261,126],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["clip",[],"#a1a1aa","rgba(161,161,170,0.35)"],
  ["setLineDash",[[]],"#a1a1aa","#fbbf24"],
  ["beginPath",[],"#a1a1aa","#fbbf24"],
  ["moveTo",[52,113.84],"#a1a1aa","#fbbf24"],
  ["lineTo",[52,103.76],"#a1a1aa","#fbbf24"],
  ["moveTo",[49,113.84],"#a1a1aa","#fbbf24"],
  ["lineTo",[55,113.84],"#a1a1aa","#fbbf24"],
  ["moveTo",[49,103.76],"#a1a1aa","#fbbf24"],
  ["lineTo",[55,103.76],"#a1a1aa","#fbbf24"],
  ["stroke",[],"#a1a1aa","#fbbf24"],
  ["beginPath",[],"#a1a1aa","#fbbf24"],
  ["moveTo",[139,63.44],"#a1a1aa","#fbbf24"],
  ["lineTo",[139,53.36],"#a1a1aa","#fbbf24"],
  ["moveTo",[136,63.44],"#a1a1aa","#fbbf24"],
  ["lineTo",[142,63.44],"#a1a1aa","#fbbf24"],
  ["moveTo",[136,53.36],"#a1a1aa","#fbbf24"],
  ["lineTo",[142,53.36],"#a1a1aa","#fbbf24"],
  ["stroke",[],"#a1a1aa","#fbbf24"],
  ["beginPath",[],"#a1a1aa","#fbbf24"],
  ["moveTo",[313,43.28],"#a1a1aa","#fbbf24"],
  ["lineTo",[313,23.12],"#a1a1aa","#fbbf24"],
  ["moveTo",[310,43.28],"#a1a1aa","#fbbf24"],
  ["lineTo",[316,43.28],"#a1a1aa","#fbbf24"],
  ["moveTo",[310,23.12],"#a1a1aa","#fbbf24"],
  ["lineTo",[316,23.12],"#a1a1aa","#fbbf24"],
  ["stroke",[],"#a1a1aa","#fbbf24"],
  ["beginPath",[],"#fbbf24","#fbbf24"],
  ["arc",[52,108.8,2.5,0,6.283],"#fbbf24","#fbbf24"],
  ["fill",[],"#fbbf24","#fbbf24"],
  ["beginPath",[],"#fbbf24","#fbbf24"],
  ["arc",[139,58.4,2.5,0,6.283],"#fbbf24","#fbbf24"],
  ["fill",[],"#fbbf24","#fbbf24"],
  ["beginPath",[],"#fbbf24","#fbbf24"],
  ["arc",[226,83.6,2.5,0,6.283],"#fbbf24","#fbbf24"],
  ["fill",[],"#fbbf24","#fbbf24"],
  ["beginPath",[],"#fbbf24","#fbbf24"],
  ["arc",[313,33.2,2.5,0,6.283],"#fbbf24","#fbbf24"],
  ["fill",[],"#fbbf24","#fbbf24"],
  ["setLineDash",[[]],"#fbbf24","#67e8f9"],
  ["beginPath",[],"#fbbf24","#67e8f9"],
  ["moveTo",[52,58.4],"#fbbf24","#67e8f9"],
  ["lineTo",[139,58.4],"#fbbf24","#67e8f9"],
  ["lineTo",[226,58.4],"#fbbf24","#67e8f9"],
  ["lineTo",[313,58.4],"#fbbf24","#67e8f9"],
  ["stroke",[],"#fbbf24","#67e8f9"],
  ["setLineDash",[[]],"#fbbf24","#67e8f9"],
  ["setLineDash",[[3,3]],"#fbbf24","#a1a1aa"],
  ["beginPath",[],"#fbbf24","#a1a1aa"],
  ["moveTo",[52,58.5],"#fbbf24","#a1a1aa"],
  ["lineTo",[313,58.5],"#fbbf24","#a1a1aa"],
  ["stroke",[],"#fbbf24","#a1a1aa"],
  ["save",[],"#a1a1aa","#a1a1aa"],
  ["strokeText",["ref",310,56.5],"#a1a1aa","rgba(0,0,0,0.8)"],
  ["fillText",["ref",310,56.5],"#a1a1aa","rgba(0,0,0,0.8)"],
  ["restore",[],"#a1a1aa","rgba(0,0,0,0.8)"],
  ["setLineDash",[[]],"#a1a1aa","rgba(0,0,0,0.8)"],
  ["restore",[],"#a1a1aa","rgba(0,0,0,0.8)"],
  ["fillRect",[58,10,8,2],"#fbbf24","rgba(0,0,0,0.8)"],
  ["fillText",["target",69,7],"#fbbf24","rgba(0,0,0,0.8)"],
  ["fillRect",[111.4,10,8,2],"#67e8f9","rgba(0,0,0,0.8)"],
  ["fillText",["model",122.4,7],"#67e8f9","rgba(0,0,0,0.8)"],
];

describe("ProfilePlot parity", () => {
  it("a plot with a short legend and short tick labels is drawn exactly as before", () => {
    expect(draw(PARITY_PROPS)).toEqual(PARITY_GOLDEN);
  });
});

function indexAfter(calls: Call[], from: number, match: (c: Call) => boolean): number {
  for (let i = from; i < calls.length; i++) if (match(calls[i])) return i;
  return -1;
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

describe("ProfilePlot legend", () => {
  it("draws both x1d file names inside a 323 px canvas", () => {
    const drawn = texts(draw({ series: X1D_LABELS.map((label) => line(label)), xLabel: "x", yLabel: "y", toolbar: true }));
    const labels = X1D_LABELS.map((label) => drawn.find((t) => t.text === label));
    expect(labels.every(Boolean)).toBe(true);
    for (const t of labels) expect(t!.x + 5.4 * t!.text.length).toBeLessThanOrEqual(323);
    expect(labels.map((t) => [t!.x, t!.y])).toEqual([
      [69, 7],
      [69, 18],
    ]);
  });

  it("keeps every zero-point legend entry inside a 323 px canvas", () => {
    const drawn = texts(draw({ series: ZERO_POINT_SERIES, xLabel: "x", yLabel: "y" }));
    const labels = ZERO_POINT_SERIES.map((s) => drawn.find((t) => t.text === s.label)!);
    for (const t of labels) expect(t.x + 5.4 * t.text.length).toBeLessThanOrEqual(323);
    expect(labels.map((t) => [t.x, t.y])).toEqual([
      [69, 7],
      [160.2, 7],
      [69, 18],
      [149.4, 18],
    ]);
  });

  it("legend none draws no series label and keeps the top margin", () => {
    const calls = draw({ ...PARITY_PROPS, legend: "none" });
    expect(texts(calls).filter((t) => t.text === "target" || t.text === "model")).toEqual([]);
    expect(axisStart(calls)).toEqual([52.5, 8]);
  });

  it("legend wrap keeps the rows above the plot area", () => {
    const calls = draw({ series: ZERO_POINT_SERIES, xLabel: "x", yLabel: "y", legend: "wrap" });
    const labels = new Set(ZERO_POINT_SERIES.map((s) => s.label));
    expect([...new Set(texts(calls).filter((t) => labels.has(t.text)).map((t) => t.y))]).toEqual([1, 12]);
    expect(axisStart(calls)).toEqual([52.5, 30]);
  });

  it("a dashed series shows its dash in the legend", () => {
    const calls = draw({
      series: [line("one", "#67e8f9"), { x: [1, 2, 3], y: [3, 2, 1], color: "#67e8f9", label: "two", dashed: true }],
      xLabel: "x",
      yLabel: "y",
    });
    const lastRestore = calls.map((c) => c[0]).lastIndexOf("restore");
    const legend = calls.slice(lastRestore + 1);
    expect(legend.filter((c) => c[0] === "fillRect").map((c) => c[1])).toEqual([[58, 10, 8, 2]]);
    const steps: ((c: Call) => boolean)[] = [
      (c) => c[0] === "setLineDash" && same(c[1], [[4, 3]]),
      (c) => c[0] === "moveTo" && same(c[1], [95.2, 11]),
      (c) => c[0] === "lineTo" && same(c[1], [103.2, 11]),
      (c) => c[0] === "stroke" && c[3] === "#67e8f9",
      (c) => c[0] === "setLineDash" && same(c[1], [[]]),
    ];
    let at = 0;
    for (const step of steps) {
      at = indexAfter(legend, at, step);
      expect(at).toBeGreaterThanOrEqual(0);
      at++;
    }
    expect(texts(legend).map((t) => [t.text, t.x, t.y])).toEqual([
      ["one", 69, 7],
      ["two", 106.2, 7],
    ]);
    const body = calls.slice(0, lastRestore);
    const dash = indexAfter(body, 0, (c) => c[0] === "setLineDash" && same(c[1], [[4, 3]]) && c[3] === "#67e8f9");
    expect(dash).toBeGreaterThanOrEqual(0);
    expect(body[dash + 1][0]).toBe("beginPath");
  });
});

describe("ProfilePlot axes", () => {
  it("the left margin grows with the widest y tick label so the y title stays clear", () => {
    const calls = draw({ series: [{ x: [0, 1, 2], y: [-0.0015, 0, 0.0015], color: "#fff", label: "s" }], xLabel: "x", yLabel: "y" });
    expect(axisStart(calls)).toEqual([55.5, 8]);
    const drawn = texts(calls);
    for (const label of ["-0.002", "-0.001", "0.001", "0.002"]) expect(drawn.find((t) => t.text === label)?.x).toBe(51);
    expect(axisStart(draw({ series: [line("s")], xLabel: "x", yLabel: "y" }))).toEqual([52.5, 8]);
  });
});

describe("ProfilePlot bands", () => {
  it("shades a band under the series and writes its label", () => {
    const calls = draw({ ...PARITY_PROPS, bands: [{ x0: 1, x1: 2, color: "rgba(251,191,36,0.14)", label: "in transit" }] });
    const fills = calls.map((c, i) => [c, i] as const).filter(([c]) => c[0] === "fillRect" && c[2] === "rgba(251,191,36,0.14)");
    expect(fills.map(([c]) => c[1])).toEqual([[139, 8, 87, 126]]);
    const at = fills[0][1];
    const axisStroke = calls.findIndex((c) => c[0] === "stroke" && c[3] === AXIS_STROKE);
    const firstArc = calls.findIndex((c) => c[0] === "arc");
    expect(at).toBeGreaterThan(axisStroke);
    expect(at).toBeLessThan(firstArc);
    expect(texts(calls).filter((t) => t.text === "in transit")).toEqual([{ text: "in transit", x: 182.5, y: 21 }]);
    expect(draw(PARITY_PROPS)).toEqual(PARITY_GOLDEN);
    expect(draw({ ...PARITY_PROPS, bands: [] })).toEqual(PARITY_GOLDEN);
  });

  it("a band outside the domain, of zero width or with a non-finite edge draws nothing", () => {
    const calls = draw({ ...PARITY_PROPS, bands: [{ x0: 10, x1: 12 }, { x0: 1, x1: 1 }, { x0: NaN, x1: 2 }] });
    expect(calls).toEqual(PARITY_GOLDEN);
  });

  const SPECTRUM: ProfileSeries = { x: [0.6, 1.5, 2.5, 3.5, 4.5, 5.3], y: [1, 2, 1, 2, 1, 2], color: "#fff", label: "depth" };
  const STRIPS = [
    { x0: 2.5, x1: 3.0, label: "H2O 2.7" },
    { x0: 4.15, x1: 4.5, label: "CO2 4.3" },
    { x0: 4.3, x1: 5.1, label: "CO 4.7" },
    { x0: 3.15, x1: 3.5, label: "CH4 3.3" },
  ];
  const stripLabels = (bands: typeof STRIPS) => {
    const names = new Set(bands.map((b) => b.label));
    return texts(draw({ series: [SPECTRUM], xLabel: "um", yLabel: "depth", bands, legend: "none" })).filter((t) => names.has(t.text));
  };
  const extent = (t: { text: string; x: number }) => [t.x - (5.4 * t.text.length) / 2, t.x + (5.4 * t.text.length) / 2];

  it("labels of close or overlapping spans move to a further row instead of overprinting", () => {
    const drawn = stripLabels(STRIPS);
    expect(drawn.map((t) => t.text)).toEqual(STRIPS.map((b) => b.label));
    expect(drawn.map((t) => t.y)).toEqual([10, 10, 21, 21]);
    const stacked = [STRIPS[0], { x0: 2.85, x1: 3.05, label: "NH3 3.0" }, { x0: 2.95, x1: 3.1, label: "HCN 3.0" }];
    expect(stripLabels(stacked).map((t) => [t.text, t.y])).toEqual([
      ["H2O 2.7", 10],
      ["NH3 3.0", 21],
      ["HCN 3.0", 32],
    ]);
    for (let i = 0; i < drawn.length; i++) {
      for (let j = i + 1; j < drawn.length; j++) {
        if (drawn[i].y !== drawn[j].y) continue;
        const [a0, a1] = extent(drawn[i]);
        const [b0, b1] = extent(drawn[j]);
        expect(a1 <= b0 || b1 <= a0).toBe(true);
      }
    }
  });

  it("labels that do not collide stay on the first row", () => {
    const apart = [STRIPS[0], STRIPS[2]];
    expect(stripLabels(apart).map((t) => t.y)).toEqual([10, 10]);
    expect(stripLabels([STRIPS[3]]).map((t) => t.y)).toEqual([10]);
  });
});

describe("ProfilePlot reference line labels", () => {
  const PEAK = "P = 12.3456 d";
  const labelOf = (value: number) => {
    const { calls, aligns } = renderPlot({ ...PARITY_PROPS, referenceLines: [{ axis: "x", value, label: PEAK }] });
    const fills = calls.filter((c) => c[0] === "fillText");
    const k = fills.findIndex((c) => c[1][0] === PEAK);
    return { x: fills[k][1][1], y: fills[k][1][2], align: aligns[k] };
  };

  it("an x line label that would pass the right edge is drawn right-aligned left of the line", () => {
    expect(labelOf(2.9)).toEqual({ x: 301.5, y: 132, align: "right" });
  });

  it("an x line label with room on the right keeps its place", () => {
    expect(labelOf(1)).toEqual({ x: 142.5, y: 132, align: "left" });
  });
});

interface TreeElement {
  type: unknown;
  props: Record<string, unknown>;
}

function elementsOf(node: unknown): TreeElement[] {
  if (Array.isArray(node)) return node.flatMap(elementsOf);
  if (node === null || typeof node !== "object" || !("props" in node)) return [];
  const element = node as TreeElement;
  return [element, ...elementsOf(element.props.children)];
}

const canvasesOf = (tree: unknown) => elementsOf(tree).filter((e) => e.type === "canvas");

describe("ProfilePlot text alternative", () => {
  it("the plot is exposed as an image with a label and a described-by summary", () => {
    const all = elementsOf(renderPlot(PARITY_PROPS).tree);
    const [baseCanvas] = all.filter((e) => e.type === "canvas");
    expect(baseCanvas.props.role).toBe("img");
    expect(baseCanvas.props["aria-label"]).toBe("Plot of flux against time: target, model");
    const describedBy = baseCanvas.props["aria-describedby"];
    expect(typeof describedBy).toBe("string");
    const summary = all.find((e) => e.type === "p" && e.props.id === describedBy);
    expect(summary).toBeDefined();
    expect(String(summary!.props.className)).toContain("sr-only");
    expect("data-plot-summary" in summary!.props).toBe(true);
    expect(summary!.props.children).toBe(plotSummary(PARITY_PROPS.series, "time", "flux", null));
  });
});

describe("ProfilePlot wheel", () => {
  function wheel(extra: Partial<ProfilePlotProps>, keys: { ctrlKey: boolean; metaKey: boolean }) {
    const onDomainChange = vi.fn();
    const preventDefault = vi.fn();
    const { listeners } = renderPlot({ ...PARITY_PROPS, toolbar: true, onDomainChange, ...extra });
    listeners.wheel({ deltaY: -100, clientX: 150, clientY: 50, ...keys, preventDefault });
    return { onDomainChange, preventDefault };
  }

  it("a plain wheel zooms by default and is left to the panel in modifier mode", () => {
    const plain = { ctrlKey: false, metaKey: false };
    const zoomed = wheel({}, plain);
    expect(zoomed.preventDefault).toHaveBeenCalledTimes(1);
    expect(zoomed.onDomainChange).toHaveBeenCalledTimes(1);
    const [domain] = zoomed.onDomainChange.mock.calls[0] as [[number, number]];
    expect(domain[0]).toBeGreaterThanOrEqual(0);
    expect(domain[1]).toBeLessThanOrEqual(3);
    expect(domain[1] - domain[0]).toBeLessThan(3);
    const left = wheel({ wheelZoom: "modifier" }, plain);
    expect(left.preventDefault).not.toHaveBeenCalled();
    expect(left.onDomainChange).not.toHaveBeenCalled();
    for (const keys of [
      { ctrlKey: true, metaKey: false },
      { ctrlKey: false, metaKey: true },
    ]) {
      const held = wheel({ wheelZoom: "modifier" }, keys);
      expect(held.preventDefault).toHaveBeenCalledTimes(1);
      expect(held.onDomainChange).toHaveBeenCalledTimes(1);
    }
  });

  it("the overlay explains the gestures only in modifier mode", () => {
    const overlayOf = (props: ProfilePlotProps) => canvasesOf(renderPlot(props).tree)[1];
    expect(overlayOf({ ...PARITY_PROPS, wheelZoom: "modifier" }).props.title).toBe("Ctrl+wheel zooms, drag pans, double-click resets");
    expect(overlayOf(PARITY_PROPS).props.title).toBeUndefined();
  });
});

describe("ProfilePlot PNG", () => {
  it("the saved PNG is painted opaque at twice the size", async () => {
    const png = recordingContext();
    const pngCanvas = {
      width: 0,
      height: 0,
      getContext: () => png.ctx,
      toBlob: (cb: (blob: unknown) => void) => cb({ arrayBuffer: async () => new ArrayBuffer(8) }),
    };
    const createElement = vi.fn((_tag: string) => pngCanvas);
    const { tree } = renderPlot({ ...PARITY_PROPS, toolbar: true });
    vi.stubGlobal("document", { createElement });
    const button = elementsOf(tree).find((e) => e.type === "button" && e.props.title === "Save the plot as a PNG image");
    (button!.props.onClick as () => void)();
    await vi.waitFor(() => expect(createElement).toHaveBeenCalledWith("canvas"));
    await vi.waitFor(() => expect(plugins.writeFile).toHaveBeenCalled());
    expect([pngCanvas.width, pngCanvas.height]).toEqual([646, 320]);
    expect(png.calls.slice(0, 2)).toEqual([
      ["setTransform", [2, 0, 0, 2, 0, 0], null, null],
      ["fillRect", [0, 0, 323, 160], "#0a0a0f", null],
    ]);
    expect(png.calls.some((c) => c[0] === "clearRect")).toBe(false);
    const drawn = texts(png.calls).map((t) => t.text);
    expect(drawn).toContain("target");
    expect(drawn).toContain("model");
    const [path, bytes] = plugins.writeFile.mock.calls[0];
    expect(path).toBe("C:/out/plot.png");
    expect(bytes).toBeInstanceOf(Uint8Array);
    expect(bytes.length).toBe(8);
  });
});
