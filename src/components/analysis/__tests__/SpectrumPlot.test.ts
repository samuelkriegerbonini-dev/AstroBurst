import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { LineMeasurement } from "../../../shared/types/spectral";
import type { BandMark, LineMark } from "../../../utils/lineList";

type RecordedFrame = { top: number; height: number; yMin: number; yMax: number };

const overlayAxes = vi.hoisted(() => ({ calls: [] as number[][], frames: [] as RecordedFrame[], fitFrames: [] as RecordedFrame[] }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../utils/lineMeasure", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../utils/lineMeasure")>();
  return {
    ...actual,
    lineOverlayPolylines: (...args: Parameters<typeof actual.lineOverlayPolylines>) => {
      overlayAxes.calls.push(args[1]);
      overlayAxes.frames.push(args[3]);
      return actual.lineOverlayPolylines(...args);
    },
  };
});
vi.mock("../../../utils/lineFit", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../utils/lineFit")>();
  return {
    ...actual,
    lineFitOverlayPolylines: (...args: Parameters<typeof actual.lineFitOverlayPolylines>) => {
      overlayAxes.fitFrames.push(args[2]);
      return actual.lineFitOverlayPolylines(...args);
    },
  };
});

import { hooks } from "../../../test/hookRuntime";
import SpectrumPlot, { type SpectrumPlotProps } from "../SpectrumPlot";
import type { LineFitSpaxel } from "../../../shared/types/cube";
import {
  NATIVE_FLUX_STATE,
  SPECTRUM_PLOT_PAD,
  buildSpectrumPlotModel,
  spectrumPlotMapping,
  spectrumReadout,
  type SpectrumPlotBins,
  type SpectrumPlotModel,
} from "../../../utils/spectrumPlotModel";
import { panDomain, zoomDomain } from "../../../utils/plotInteraction";

type Props = Record<string, unknown>;
type PathOp = ["M" | "L", number, number];

const plotFunction = (SpectrumPlot as unknown as { type: (props: SpectrumPlotProps) => ReactNode }).type;

const WIDTH = 400;
const BASE_CLASS = "w-full rounded-md cursor-crosshair";
const OVERLAY_CLASS = "absolute inset-0 w-full pointer-events-none";
const XS = Array.from({ length: 40 }, (_, i) => 0.6555 + i * 1e-4);

const PLOT_SPAN = WIDTH - SPECTRUM_PLOT_PAD.left - SPECTRUM_PLOT_PAD.right;
const px = (i: number) => SPECTRUM_PLOT_PAD.left + (PLOT_SPAN * i) / 39;

function plotModel(extra: Partial<SpectrumPlotModel> = {}): SpectrumPlotModel {
  return {
    n: 40,
    xValues: XS,
    xLabel: "Vacuum wavelength (μm)",
    xUnit: "μm",
    xScale: 1,
    wavelengthUnit: "um",
    fluxState: NATIVE_FLUX_STATE,
    series: XS.map((_, i) => i / 39),
    dqExcluded: null,
    errors: null,
    errorBand: "none",
    readoutErr: null,
    fluxUnit: "DN",
    color: "#a855f7",
    yLabel: null,
    xMin: XS[0],
    xMax: XS[39],
    xExtent: [XS[0], XS[39]],
    yMin: 0,
    yMax: 1,
    logY: false,
    offScale: 0,
    farOutliers: 0,
    logHidden: 0,
    bins: null,
    ...extra,
  };
}

const ZOOMED: Partial<SpectrumPlotModel> = {
  xScale: 1000,
  xValues: XS.map((x) => x * 1000),
  xMin: XS[0] * 1000,
  xMax: XS[39] * 1000,
  xExtent: [XS[0] * 1000, XS[39] * 1000],
};

function mark(id: string, value: number): LineMark {
  return { id, label: id.toUpperCase(), family: "optical", restVacuumUm: value, value };
}

function findProps(node: ReactNode, match: (props: Props) => boolean): Props | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findProps(child, match);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Props>(node)) return null;
  if (match(node.props)) return node.props;
  return findProps(node.props.children as ReactNode, match);
}

function canvasProps(tree: ReactNode, className: string): Props {
  const found = findProps(tree, (p) => p.className === className);
  expect(found, className).not.toBeNull();
  return found!;
}

function recordingCanvas() {
  const strokes: { style: unknown; width: unknown; dash: readonly number[]; ops: PathOp[]; alpha: unknown }[] = [];
  const texts: { text: string; x: number; y: number; align: unknown; style: unknown }[] = [];
  const rects: { style: unknown; x: number; y: number; w: number; h: number }[] = [];
  const fills: unknown[] = [];
  const fillPaths: { style: unknown; ops: PathOp[] }[] = [];
  const arcs: { style: unknown; x: number; y: number; r: number }[] = [];
  const turns: (string | number)[][] = [];
  let ops: PathOp[] = [];
  let dash: readonly number[] = [];
  const ctx = new Proxy({} as Record<string | symbol, unknown>, {
    get: (target, key) => {
      if (key === "translate") return (x: number, y: number) => turns.push(["translate", x, y]);
      if (key === "rotate") return (angle: number) => turns.push(["rotate", angle]);
      if (key === "beginPath") return () => (ops = []);
      if (key === "moveTo") return (x: number, y: number) => ops.push(["M", x, y]);
      if (key === "lineTo") return (x: number, y: number) => ops.push(["L", x, y]);
      if (key === "setLineDash") return (next: readonly number[]) => (dash = next);
      if (key === "stroke") return () => strokes.push({ style: target.strokeStyle, width: target.lineWidth, dash, ops, alpha: target.globalAlpha ?? 1 });
      if (key === "fill")
        return () => {
          fills.push(target.fillStyle);
          fillPaths.push({ style: target.fillStyle, ops });
        };
      if (key === "arc") return (x: number, y: number, r: number) => arcs.push({ style: target.fillStyle, x, y, r });
      if (key === "fillRect") return (x: number, y: number, w: number, h: number) => rects.push({ style: target.fillStyle, x, y, w, h });
      if (key === "fillText") return (text: string, x: number, y: number) => texts.push({ text, x, y, align: target.textAlign, style: target.fillStyle });
      if (key === "measureText") return () => ({ width: 10 });
      return key in target ? target[key] : () => {};
    },
    set: (target, key, value) => {
      target[key] = value;
      return true;
    },
  });
  const canvas = {
    width: WIDTH,
    height: 0,
    parentElement: { getBoundingClientRect: () => ({ width: WIDTH }) },
    getBoundingClientRect: () => ({ left: 0, width: WIDTH }),
    getContext: () => ctx,
  };
  return { canvas, strokes, texts, rects, fills, fillPaths, arcs, turns };
}

function listenerTarget() {
  const listeners: { type: string; handler: (e: unknown) => void; options: unknown }[] = [];
  const element = {
    addEventListener: (type: string, handler: (e: unknown) => void, options: unknown) => listeners.push({ type, handler, options }),
    removeEventListener: (type: string, handler: (e: unknown) => void) => {
      const index = listeners.findIndex((l) => l.type === type && l.handler === handler);
      if (index >= 0) listeners.splice(index, 1);
    },
  };
  return { element, listeners };
}

function mounted(props: SpectrumPlotProps) {
  hooks.mount();
  const base = recordingCanvas();
  const overlay = recordingCanvas();
  const host = listenerTarget();
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const first = hooks.render(plotFunction, props) as ReactNode;
  (canvasProps(first, BASE_CLASS).ref as { current: unknown }).current = base.canvas;
  (canvasProps(first, OVERLAY_CLASS).ref as { current: unknown }).current = overlay.canvas;
  (findProps(first, (p) => p.role === "img")!.ref as { current: unknown }).current = host.element;
  const current: SpectrumPlotProps = { ...props, model: { ...props.model } };
  let tree = hooks.render(plotFunction, current) as ReactNode;
  const send = (handler: string, clientX: number, button = 0, shiftKey = false) => {
    (canvasProps(tree, BASE_CLASS)[handler] as (e: unknown) => void)({ clientX, button, shiftKey, preventDefault: () => {} });
    tree = hooks.render(plotFunction, current) as ReactNode;
  };
  const root = () => findProps(tree, (p) => p.role === "img")!;
  const key = (name: string, shiftKey = false) => {
    const preventDefault = vi.fn();
    (root().onKeyDown as (e: unknown) => void)({ key: name, shiftKey, preventDefault });
    tree = hooks.render(plotFunction, current) as ReactNode;
    return preventDefault.mock.calls.length > 0;
  };
  const status = () => findProps(tree, (p) => "data-spectrum-status" in p)!.children;
  return { base, overlay, host, model: current.model, send, root, key, status, tree: () => tree };
}

function measuredLine(): LineMeasurement {
  return {
    z0: 15,
    z1: 25,
    n_channels: 11,
    axis_unit: "um",
    flux_unit: "DN x um",
    continuum_level: 0.5,
    continuum_slope: 0,
    continuum_sigma: 0.01,
    continuum_reference: XS[20],
    continuum_linear: false,
    continuum_channels: 10,
    continuum_windows: [
      [5, 9],
      [30, 34],
    ],
    flux: 1e-4,
    flux_err: 1e-6,
    equivalent_width: -1e-4,
    centroid: XS[20],
    sigma: 2e-4,
    fwhm: 4.7e-4,
    peak: 1,
    peak_channel: 20,
    snr: 100,
    velocity: null,
    fit: null,
    err_used: false,
    flux_err_terms: null,
    n_dq_excluded: 0,
    n_nonfinite: 0,
    n_bad_err: 0,
    err_source: "none",
    notes: [],
    elapsed_ms: 1,
    source: { kind: "file", hdu: 0 },
  };
}

function subpaths(ops: PathOp[]): PathOp[][] {
  return ops.reduce<PathOp[][]>((paths, op) => {
    if (op[0] === "M" || paths.length === 0) paths.push([op]);
    else paths[paths.length - 1].push(op);
    return paths;
  }, []);
}

const verticalAt = (ops: PathOp[], x: number) =>
  ops.length === 2 && ops[0][0] === "M" && ops[1][0] === "L" && Math.abs(ops[0][1] - x) < 1e-6 && Math.abs(ops[1][1] - x) < 1e-6;

beforeEach(() => {
  hooks.mount();
  overlayAxes.calls = [];
  overlayAxes.frames = [];
  overlayAxes.fitFrames = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("SpectrumPlot base layer", () => {
  it("strokes the series at the pixels of the mapping", () => {
    const plot = mounted({ model: plotModel() });
    const series = plot.base.strokes.filter((s) => s.style === "#a855f7").at(-1)!;
    expect(series.ops).toHaveLength(40);
    expect(series.ops[0][0]).toBe("M");
    expect(series.ops[0][1]).toBeCloseTo(SPECTRUM_PLOT_PAD.left, 6);
    expect(series.ops[0][2]).toBeCloseTo(156, 6);
    expect(series.ops[39][1]).toBeCloseTo(388, 6);
    expect(series.ops[39][2]).toBeCloseTo(10, 6);
  });

  it("lifts the pen across a gap", () => {
    const series = XS.map((_, i) => (i >= 10 && i <= 14 ? NaN : i / 39));
    const plot = mounted({ model: plotModel({ series }) });
    const paths = subpaths(plot.base.strokes.filter((s) => s.style === "#a855f7").at(-1)!.ops);
    expect(paths.map((path) => path.length)).toEqual([10, 25]);
    expect(paths[0].at(-1)![1]).toBeCloseTo(px(9), 6);
    expect(paths[1][0][1]).toBeCloseTo(px(15), 6);
  });

  it("labels five y ticks and the x axis", () => {
    const plot = mounted({ model: plotModel() });
    const ticks = plot.base.texts.filter((t) => t.align === "right");
    expect(ticks.map((t) => t.text)).toEqual(["1.00", "0.75", "0.50", "0.25", "0"]);
    ticks.forEach((t) => expect(t.x).toBeCloseTo(SPECTRUM_PLOT_PAD.left - 4, 6));
    [13, 49.5, 86, 122.5, 159].forEach((y, k) => expect(ticks[k].y).toBeCloseTo(y, 6));
    const label = plot.base.texts.find((t) => t.text === "Vacuum wavelength (μm)");
    expect(label).toMatchObject({ x: 200, y: 176 });
  });

  it("marks DQ-excluded channels at the bottom edge", () => {
    const dqExcluded = XS.map((_, i) => i >= 10 && i <= 14);
    const plot = mounted({ model: plotModel({ dqExcluded }) });
    const ticks = plot.base.strokes.filter((s) => s.style === "rgba(244,63,94,0.7)").at(-1)!.ops;
    const starts = ticks.filter((op) => op[0] === "M");
    expect(starts).toHaveLength(5);
    starts.forEach((op, k) => {
      expect(op[1]).toBeCloseTo(Math.round(px(10 + k)) + 0.5, 6);
      expect(op[2]).toBe(156);
    });
    ticks.filter((op) => op[0] === "L").forEach((op) => expect(op[2]).toBe(152));
  });

  it("fills the error band inside the plot and outlines the scaled one", () => {
    const errors = { err: XS.map(() => 0.1), scaled: XS.map(() => 0.13) };
    const model = plotModel({ errors, errorBand: "propagated+scaled" });
    const plot = mounted({ model });
    expect(plot.base.fills).toContain("rgba(34,211,238,0.16)");
    const outline = plot.base.strokes.filter((s) => s.style === "rgba(34,211,238,0.55)");
    expect(outline.length).toBeGreaterThan(0);
    outline.forEach((s) => expect(s.dash).toEqual([3, 3]));
    hooks.mount();
    expect(renderToStaticMarkup(hooks.render(plotFunction, { model }) as ReactElement)).toContain('data-error-band="propagated+scaled"');
    hooks.mount();
    expect(renderToStaticMarkup(hooks.render(plotFunction, { model: plotModel() }) as ReactElement)).toContain('data-error-band="none"');
  });

  it("draws the y title rotated beside the ticks", () => {
    const titled = mounted({ model: plotModel({ yLabel: "Flux (Jy)" }) });
    expect(titled.base.turns).toContainEqual(["translate", 9, 83]);
    expect(titled.base.turns).toContainEqual(["rotate", -Math.PI / 2]);
    expect(titled.base.texts).toContainEqual(expect.objectContaining({ text: "Flux (Jy)", x: 0, y: 0, align: "center" }));
    const untitled = mounted({ model: plotModel({ yLabel: null }) });
    expect(untitled.base.turns.some((turn) => turn[0] === "rotate")).toBe(false);
  });
});

describe("SpectrumPlot pointer", () => {
  it("prints the readout of the hovered channel", () => {
    const plot = mounted({ model: plotModel() });
    plot.send("onMouseMove", px(20));
    expect(plot.overlay.texts.map((t) => t.text)).toContain(spectrumReadout(plot.model, 20, null));
    const before = plot.overlay.texts.length;
    plot.send("onMouseLeave", 0);
    const after = plot.overlay.texts.slice(before);
    expect(after.some((t) => t.text.startsWith("ch "))).toBe(false);
  });

  it("reports a drag as a channel range and a click on a mark as a pick", () => {
    const onBrush = vi.fn();
    const onPickMark = vi.fn();
    const onPress = vi.fn();
    const marks = [mark("a", XS[20]), mark("b", XS[30])];
    const plot = mounted({ model: plotModel(), marks, onBrush, onPickMark, onPress });
    plot.send("onMouseDown", px(5));
    plot.send("onMouseMove", px(12));
    plot.send("onMouseUp", px(12));
    expect(onBrush).toHaveBeenCalledTimes(1);
    expect(onBrush).toHaveBeenCalledWith({ z0: 5, z1: 12 });
    expect(onPickMark).not.toHaveBeenCalled();
    plot.send("onMouseDown", px(20) + 4);
    plot.send("onMouseUp", px(20) + 4);
    expect(onPickMark).toHaveBeenCalledTimes(1);
    expect(onPickMark).toHaveBeenCalledWith(marks[0]);
    plot.send("onMouseDown", px(20) + 9);
    plot.send("onMouseUp", px(20) + 9);
    expect(onPickMark).toHaveBeenCalledTimes(1);
    expect(onBrush).toHaveBeenCalledTimes(1);
    expect(onPress).toHaveBeenCalledTimes(3);
    plot.send("onMouseDown", px(5), 2);
    plot.send("onMouseMove", px(12));
    plot.send("onMouseUp", px(12));
    expect(onPress).toHaveBeenCalledTimes(3);
    expect(onBrush).toHaveBeenCalledTimes(1);
    expect(onPickMark).toHaveBeenCalledTimes(1);

    const zoomedPick = vi.fn();
    const zoomed = mounted({ model: plotModel(ZOOMED), marks, onPickMark: zoomedPick });
    zoomed.send("onMouseDown", px(20) + 4);
    zoomed.send("onMouseUp", px(20) + 4);
    expect(zoomedPick).toHaveBeenCalledTimes(1);
    expect(zoomedPick).toHaveBeenCalledWith(marks[0]);
  });

  it("keeps the hover on the channel under the cursor at a pixel ratio of two", () => {
    vi.stubGlobal("window", { devicePixelRatio: 2 });
    const onBrush = vi.fn();
    const plot = mounted({ model: plotModel(), onBrush });
    expect(plot.base.canvas.width).toBe(800);
    expect(plot.base.canvas.height).toBe(360);
    plot.send("onMouseMove", px(20));
    const readout = plot.overlay.texts.map((t) => t.text).filter((text) => text.startsWith("ch "));
    expect(readout.at(-1)).toMatch(/^ch 20 /);
    expect(plot.status()).toBe(spectrumReadout(plot.model, 20, null));
    plot.send("onMouseDown", px(5));
    plot.send("onMouseMove", px(12));
    plot.send("onMouseUp", px(12));
    expect(onBrush).toHaveBeenCalledTimes(1);
    expect(onBrush).toHaveBeenCalledWith({ z0: 5, z1: 12 });
  });

  it("reports a double click with the mapping", () => {
    const onPlotDoubleClick = vi.fn();
    const plot = mounted({ model: plotModel(), onPlotDoubleClick });
    plot.send("onDoubleClick", px(7));
    expect(onPlotDoubleClick).toHaveBeenCalledTimes(1);
    const [clickPx, mapping] = onPlotDoubleClick.mock.calls[0];
    expect(clickPx).toBeCloseTo(px(7), 6);
    expect(mapping).toEqual(spectrumPlotMapping(plot.model, WIDTH));
  });
});

describe("SpectrumPlot overlay", () => {
  it("shades the line range and the continuum windows", () => {
    const plot = mounted({
      model: plotModel(),
      range: { z0: 5, z1: 12 },
      windows: [
        [0, 3],
        [20, 24],
      ],
    });
    const shaded = plot.overlay.rects.filter((r) => r.style === "rgba(168,85,247,0.22)");
    expect(shaded).toHaveLength(1);
    expect(shaded[0].x).toBeCloseTo(px(4.5), 6);
    expect(shaded[0].x + shaded[0].w).toBeCloseTo(px(12.5), 6);
    expect(shaded[0].y).toBe(10);
    expect(shaded[0].h).toBe(146);
    expect(plot.overlay.rects.filter((r) => r.style === "rgba(245,158,11,0.18)")).toHaveLength(2);
  });

  it("draws rest-line marks and the picked one solid", () => {
    const marks = [mark("a", XS[10]), mark("b", XS[30])];
    for (const model of [plotModel(), plotModel(ZOOMED)]) {
      const plot = mounted({ model, marks, pickedMarkId: "b" });
      const first = plot.overlay.strokes.filter((s) => verticalAt(s.ops, px(10)));
      const second = plot.overlay.strokes.filter((s) => verticalAt(s.ops, px(30)));
      expect(first).toHaveLength(1);
      expect(second).toHaveLength(1);
      expect(first[0].ops.map((op) => op[2])).toEqual([10, 156]);
      expect(second[0].ops.map((op) => op[2])).toEqual([10, 156]);
      expect(first[0]).toMatchObject({ style: "rgba(34,211,238,0.6)", dash: [2, 3] });
      expect(second[0]).toMatchObject({ style: "rgba(34,211,238,1)", dash: [], width: 1.5 });
    }
  });

  it("draws the frame marker with its label", () => {
    const plot = mounted({ model: plotModel(), frameMarker: { channel: 20, text: "ch 20" } });
    const marker = plot.overlay.strokes.filter((s) => s.style === "rgba(167,139,250,0.95)");
    expect(marker).toHaveLength(1);
    expect(verticalAt(marker[0].ops, px(20))).toBe(true);
    expect(plot.overlay.texts.map((t) => t.text)).toContain("ch 20");
  });

  it("asks the measurement helpers for the overlay curves on the axis it was given", () => {
    mounted({ model: plotModel(), lineOverlay: { result: measuredLine(), measurementAxis: XS } });
    expect(overlayAxes.calls.length).toBeGreaterThan(0);
    expect(overlayAxes.calls.at(-1)).toEqual(XS);
  });
});

describe("SpectrumPlot without a mouse", () => {
  const markup = (props: SpectrumPlotProps) => {
    hooks.mount();
    return renderToStaticMarkup(hooks.render(plotFunction, props) as ReactElement);
  };

  it("names the plot for a screen reader", () => {
    const titled = markup({ model: plotModel({ yLabel: "Flux (Jy)" }) });
    expect(titled).toContain('role="img"');
    expect(titled).toContain('aria-label="Spectrum plot: Flux (Jy) against Vacuum wavelength (μm), 40 channels"');
    expect(markup({ model: plotModel({ yLabel: null }) })).toContain('aria-label="Spectrum plot: flux against Vacuum wavelength (μm), 40 channels"');
  });

  it("moves a channel cursor with the arrow keys and speaks its readout", () => {
    const plot = mounted({ model: plotModel(), cursorKeys: true });
    const spoken = (channel: number) => spectrumReadout(plot.model, channel, null);
    expect(plot.root().tabIndex).toBe(0);
    expect(plot.status()).toBe("");
    expect(plot.key("ArrowRight")).toBe(true);
    expect(plot.status()).toBe(spoken(0));
    expect(plot.key("ArrowRight")).toBe(true);
    expect(plot.status()).toBe(spoken(1));
    expect(plot.key("ArrowRight", true)).toBe(true);
    expect(plot.status()).toBe(spoken(11));
    const crosshair = plot.overlay.strokes.filter((s) => verticalAt(s.ops, px(11)));
    expect(crosshair.length).toBeGreaterThan(0);
    expect(plot.key("End")).toBe(true);
    expect(plot.status()).toBe(spoken(39));
    expect(plot.key("ArrowRight")).toBe(true);
    expect(plot.status()).toBe(spoken(39));
    expect(plot.key("Home")).toBe(true);
    expect(plot.status()).toBe(spoken(0));
    expect(plot.key("ArrowLeft")).toBe(true);
    expect(plot.status()).toBe(spoken(0));
    expect(plot.key("Escape")).toBe(true);
    expect(plot.status()).toBe("");
    expect(plot.key("ArrowLeft")).toBe(true);
    expect(plot.status()).toBe(spoken(39));
    expect(plot.key("a")).toBe(false);
    expect(plot.status()).toBe(spoken(39));

    const empty = mounted({ model: plotModel({ n: 0, xValues: [], series: [] }), cursorKeys: true });
    expect(() => empty.key("ArrowRight")).not.toThrow();
    expect(() => empty.key("End")).not.toThrow();
    expect(empty.status()).toBe("");
  });

  it("is not a tab stop and ignores keys without cursorKeys", () => {
    const plot = mounted({ model: plotModel() });
    expect(plot.root().tabIndex).toBeUndefined();
    expect(plot.key("ArrowRight")).toBe(false);
    expect(plot.status()).toBe("");
  });

  it("speaks the status the panel passes instead of the readout", () => {
    const status = "ch 12 (0–39) - 0.6567 μm";
    const plot = mounted({ model: plotModel(), status });
    plot.send("onMouseMove", px(20));
    expect(plot.overlay.texts.map((t) => t.text)).toContain(spectrumReadout(plot.model, 20, null));
    expect(plot.status()).toBe(status);
  });

  it("links the key hint", () => {
    const hint = "Left and Right arrows move the channel cursor.";
    const hinted = mounted({ model: plotModel(), cursorKeys: true, keyHint: hint });
    const describedBy = hinted.root()["aria-describedby"];
    expect(describedBy).toEqual(expect.any(String));
    const span = findProps(hinted.tree(), (p) => p.id === describedBy);
    expect(span).toMatchObject({ className: "sr-only", children: hint });
    const plain = mounted({ model: plotModel(), cursorKeys: true });
    expect(plain.root()["aria-describedby"]).toBeUndefined();
    expect(findProps(plain.tree(), (p) => p.className === "sr-only" && p.children === hint)).toBeNull();
  });
});

const EXTENT: [number, number] = [XS[0], XS[39]];

function wheelOf(plot: ReturnType<typeof mounted>) {
  const wheel = plot.host.listeners.filter((l) => l.type === "wheel");
  expect(wheel).toHaveLength(1);
  expect(wheel[0].options).toEqual({ passive: false });
  return (deltaY: number, clientX: number) => {
    const preventDefault = vi.fn();
    wheel[0].handler({ deltaY, clientX, preventDefault });
    return preventDefault;
  };
}

function expectDomain(actual: unknown, expected: [number, number]) {
  const domain = (actual as { xDomain: [number, number] }).xDomain;
  expect(domain).toHaveLength(2);
  expect(domain[0]).toBeCloseTo(expected[0], 12);
  expect(domain[1]).toBeCloseTo(expected[1], 12);
}

describe("SpectrumPlot zoom and pan", () => {
  it("zooms around the cursor with the wheel", () => {
    const onViewChange = vi.fn();
    const wheel = wheelOf(mounted({ model: plotModel(), onViewChange }));
    const zoomIn = wheel(-100, px(20));
    expect(onViewChange).toHaveBeenCalledTimes(1);
    expectDomain(onViewChange.mock.calls[0][0], zoomDomain(EXTENT, EXTENT, XS[20], 1 / 1.15));
    expect(zoomIn).toHaveBeenCalledTimes(1);
    const atExtent = wheel(100, px(20));
    expect(onViewChange).toHaveBeenCalledTimes(1);
    expect(atExtent).not.toHaveBeenCalled();

    const outward = vi.fn();
    const zoomOut = wheelOf(mounted({ model: plotModel({ xMin: XS[0], xMax: XS[35] }), onViewChange: outward }))(100, px(20));
    expect(outward).toHaveBeenCalledWith({ xDomain: null });
    expect(zoomOut).toHaveBeenCalledTimes(1);

    const plain = mounted({ model: plotModel() });
    expect(plain.host.listeners.filter((l) => l.type === "wheel")).toHaveLength(0);
  });

  it("pans with shift and drag, and brushes without it", () => {
    const zoomed = plotModel({ xMin: XS[10], xMax: XS[30] });
    const onViewChange = vi.fn();
    const onBrush = vi.fn();
    const onPickMark = vi.fn();
    const marks = [mark("a", XS[20])];
    const plot = mounted({ model: zoomed, onViewChange, onBrush, onPickMark, marks });
    plot.send("onMouseDown", px(10), 0, true);
    plot.send("onMouseMove", px(14), 0, true);
    expect(onViewChange).toHaveBeenCalledTimes(1);
    expectDomain(onViewChange.mock.calls[0][0], panDomain([XS[10], XS[30]], EXTENT, (-(px(14) - px(10)) / 326) * (XS[30] - XS[10])));
    plot.send("onMouseUp", px(14), 0, true);
    expect(onBrush).not.toHaveBeenCalled();
    expect(onPickMark).not.toHaveBeenCalled();

    const brushView = vi.fn();
    const brush = vi.fn();
    const plain = mounted({ model: zoomed, onViewChange: brushView, onBrush: brush });
    plain.send("onMouseDown", px(10));
    plain.send("onMouseMove", px(14));
    plain.send("onMouseUp", px(14));
    expect(brush).toHaveBeenCalledTimes(1);
    expect(brushView).not.toHaveBeenCalled();
  });

  it("resets the zoom on a double click when asked to", () => {
    const onViewChange = vi.fn();
    const onPlotDoubleClick = vi.fn();
    const reset = mounted({ model: plotModel(), onViewChange, onPlotDoubleClick, doubleClick: "reset" });
    reset.send("onDoubleClick", px(7));
    expect(onViewChange).toHaveBeenCalledWith({ xDomain: null });
    expect(onPlotDoubleClick).not.toHaveBeenCalled();
    const framed = mounted({ model: plotModel(), onViewChange, onPlotDoubleClick });
    framed.send("onDoubleClick", px(7));
    expect(onPlotDoubleClick).toHaveBeenCalledTimes(1);
    expect(onViewChange).toHaveBeenCalledTimes(1);
  });

  it("keeps hover, brush and marks on the channels of a zoomed plot", () => {
    const zoomed = plotModel({ xMin: XS[10], xMax: XS[30] });
    const plot = mounted({ model: zoomed, marks: [mark("out", XS[5]), mark("in", XS[20])] });
    const outside = SPECTRUM_PLOT_PAD.left + (PLOT_SPAN * (XS[5] - XS[10])) / (XS[30] - XS[10]);
    expect(plot.overlay.strokes.filter((s) => verticalAt(s.ops, outside))).toHaveLength(0);
    expect(plot.overlay.strokes.filter((s) => verticalAt(s.ops, SPECTRUM_PLOT_PAD.left + PLOT_SPAN / 2))).toHaveLength(1);
    plot.send("onMouseMove", SPECTRUM_PLOT_PAD.left);
    expect(plot.status()).toBe(spectrumReadout(plot.model, 10, null));
    plot.send("onMouseMove", WIDTH - SPECTRUM_PLOT_PAD.right);
    expect(plot.status()).toBe(spectrumReadout(plot.model, 30, null));
    const onBrush = vi.fn();
    const brushed = mounted({ model: zoomed, onBrush });
    brushed.send("onMouseDown", SPECTRUM_PLOT_PAD.left);
    brushed.send("onMouseMove", SPECTRUM_PLOT_PAD.left + PLOT_SPAN / 2);
    brushed.send("onMouseUp", SPECTRUM_PLOT_PAD.left + PLOT_SPAN / 2);
    expect(onBrush).toHaveBeenCalledWith({ z0: 10, z1: 20 });
  });

  it("draws log ticks and lifts the pen at samples that have no logarithm", () => {
    const model = buildSpectrumPlotModel({
      series: [-1, 0, 0.01, 0.1, 1, 10],
      dqExcluded: null,
      axis: { formatted: null, mode: "wavelength_vac", correction: "none", correctionResult: null, fallbackWavelengths: null, fallbackUnit: null },
      errors: null,
      readoutErr: null,
      fluxUnit: "DN",
      color: "#a855f7",
      yLabel: null,
      errorsWidenRange: true,
      view: { xDomain: null, logY: true, yLimits: "full" },
    });
    expect(model.logY).toBe(true);
    const plot = mounted({ model });
    const paths = subpaths(plot.base.strokes.filter((s) => s.style === "#a855f7").at(-1)!.ops);
    expect(paths.map((path) => path.length)).toEqual([4]);
    expect(paths[0][0][1]).toBeCloseTo(SPECTRUM_PLOT_PAD.left + (PLOT_SPAN * 2) / 5, 6);
    const labels = plot.base.texts.filter((t) => t.align === "right").map((t) => t.text);
    expect(labels).toEqual(expect.arrayContaining(["0.1", "1"]));
  });

  it("draws no measured-line overlay on a log axis", () => {
    const lineOverlay = { result: measuredLine(), measurementAxis: XS };
    mounted({ model: plotModel({ logY: true, yMin: 0.01, yMax: 1 }), lineOverlay });
    expect(overlayAxes.calls).toHaveLength(0);
    mounted({ model: plotModel(), lineOverlay });
    expect(overlayAxes.calls.length).toBeGreaterThan(0);
  });
});

describe("SpectrumPlot display units", () => {
  it("puts the smallest magnitude at the top of an inverted axis", () => {
    const inverted = mounted({ model: plotModel({ fluxState: { choice: "ab", scale: null, invertY: true, note: null } }) });
    const ticks = inverted.base.texts.filter((t) => t.align === "right");
    expect(ticks.find((t) => t.text === "0")?.y).toBeCloseTo(13, 6);
    expect(ticks.find((t) => t.text === "1.00")?.y).toBeCloseTo(159, 6);
    const series = inverted.base.strokes.filter((s) => s.style === "#a855f7").at(-1)!.ops;
    expect(series[0][1]).toBeCloseTo(px(0), 6);
    expect(series[0][2]).toBeCloseTo(10, 6);
    expect(series[39][1]).toBeCloseTo(px(39), 6);
    expect(series[39][2]).toBeCloseTo(156, 6);
    const upright = mounted({ model: plotModel({ fluxState: { choice: "native", scale: 1, invertY: false, note: null } }) });
    expect(upright.base.texts.filter((t) => t.align === "right").find((t) => t.text === "1.00")?.y).toBeCloseTo(13, 6);
  });

  it("draws the measured line in a unit that scales the file's flux, and not otherwise", () => {
    const scaled = { yMin: 0, yMax: 2000, fluxState: { choice: "mJy" as const, scale: 1000, invertY: false, note: null } };
    const unscaled = { yMin: 0, yMax: 2000, fluxState: { choice: "uJy" as const, scale: null, invertY: false, note: null } };
    const lineOverlay = { result: measuredLine(), measurementAxis: XS };
    mounted({ model: plotModel(scaled), lineOverlay });
    expect(overlayAxes.frames.length).toBeGreaterThan(0);
    expect(overlayAxes.frames.at(-1)).toMatchObject({ yMin: 0, yMax: 2 });
    overlayAxes.frames = [];
    mounted({ model: plotModel(unscaled), lineOverlay });
    expect(overlayAxes.frames).toHaveLength(0);

    const fitOverlay = { model: null, ncomp: 1 } as unknown as LineFitSpaxel;
    mounted({ model: plotModel(scaled), fitOverlay });
    expect(overlayAxes.fitFrames.length).toBeGreaterThan(0);
    expect(overlayAxes.fitFrames.at(-1)).toMatchObject({ yMin: 0, yMax: 2 });
    overlayAxes.fitFrames = [];
    mounted({ model: plotModel(unscaled), fitOverlay });
    expect(overlayAxes.fitFrames).toHaveLength(0);
  });
});

const BIN_XS = Array.from({ length: 10 }, (_, k) => (XS[4 * k] + XS[4 * k + 1] + XS[4 * k + 2] + XS[4 * k + 3]) / 4);
const pxOf = (v: number) => SPECTRUM_PLOT_PAD.left + (PLOT_SPAN * (v - XS[0])) / (XS[39] - XS[0]);
const binY = (k: number) => 156 - (146 * k) / 9;

function binsOf(extra: Partial<SpectrumPlotBins> = {}): SpectrumPlotBins {
  return {
    factor: 4,
    first: BIN_XS.map((_, k) => 4 * k),
    last: BIN_XS.map((_, k) => 4 * k + 3),
    nUsed: BIN_XS.map(() => 4),
    x: BIN_XS,
    y: BIN_XS.map((_, k) => k / 9),
    err: null,
    note: "binned by 4: 10 bins; mean of the unmasked samples, errors assume independent channels",
    ...extra,
  };
}

const smallestX = (ops: PathOp[]) => Math.min(...ops.map((op) => op[1]));

describe("SpectrumPlot bins", () => {
  it("draws the bins over a faint copy of the channels", () => {
    const plot = mounted({ model: plotModel({ bins: binsOf() }) });
    const series = plot.base.strokes.filter((s) => s.style === "#a855f7");
    expect(series).toHaveLength(2);
    expect(series[0]).toMatchObject({ alpha: 0.3, width: 1.2 });
    expect(series[0].ops).toHaveLength(40);
    expect(series[1]).toMatchObject({ alpha: 1, width: 1.5 });
    expect(series[1].ops).toHaveLength(10);
    expect(series[1].ops[0][0]).toBe("M");
    expect(series[1].ops[0][1]).toBeCloseTo(pxOf(BIN_XS[0]), 6);
    expect(series[1].ops[0][2]).toBeCloseTo(156, 6);
    expect(series[1].ops[9][1]).toBeCloseTo(pxOf(BIN_XS[9]), 6);
    expect(series[1].ops[9][2]).toBeCloseTo(10, 6);
    expect(plot.base.strokes.filter((s) => s.alpha !== 1)).toEqual([series[0]]);
    const dots = plot.base.arcs.filter((a) => a.r === 1.5);
    expect(dots).toHaveLength(10);
    dots.forEach((dot, k) => {
      expect(dot.style).toBe("#a855f7");
      expect(dot.x).toBeCloseTo(pxOf(BIN_XS[k]), 6);
      expect(dot.y).toBeCloseTo(binY(k), 6);
    });

    const plain = mounted({ model: plotModel() });
    const channels = plain.base.strokes.filter((s) => s.style === "#a855f7");
    expect(channels).toHaveLength(1);
    expect(channels[0].alpha).toBe(1);
    expect(channels[0].ops).toHaveLength(40);
    expect(plain.base.arcs.filter((a) => a.r === 1.5)).toHaveLength(0);
  });

  it("lifts the pen at an empty bin", () => {
    const y = BIN_XS.map((_, k) => (k === 3 ? NaN : k / 9));
    const plot = mounted({ model: plotModel({ bins: binsOf({ y }) }) });
    const binStroke = plot.base.strokes.filter((s) => s.style === "#a855f7").at(-1)!;
    expect(subpaths(binStroke.ops).map((path) => path.length)).toEqual([3, 6]);
    expect(plot.base.arcs.filter((a) => a.r === 1.5)).toHaveLength(9);
  });

  it("starts the error band at the first bin", () => {
    const errors = { err: XS.map(() => 0.1), scaled: null };
    const band = (model: SpectrumPlotModel) => {
      const fills = mounted({ model }).base.fillPaths.filter((f) => f.style === "rgba(34,211,238,0.16)");
      expect(fills).toHaveLength(1);
      return smallestX(fills[0].ops);
    };
    const binned = band(plotModel({ errors, errorBand: "propagated", bins: binsOf({ err: BIN_XS.map(() => 0.02) }) }));
    expect(binned).toBeCloseTo(pxOf(BIN_XS[0]), 6);
    expect(binned).toBeGreaterThan(px(0) + 1);
    expect(band(plotModel({ errors, errorBand: "propagated" }))).toBeCloseTo(px(0), 6);
  });

  it("hovers bins and brushes channels", () => {
    const onBrush = vi.fn();
    const plot = mounted({ model: plotModel({ bins: binsOf() }), onBrush });
    plot.send("onMouseMove", pxOf(BIN_XS[5]));
    const readout = spectrumReadout(plot.model, 5, null)!;
    expect(readout.startsWith("bin 5 (ch 20–23)")).toBe(true);
    expect(plot.overlay.texts.map((t) => t.text)).toContain(readout);
    expect(plot.status()).toBe(readout);
    plot.send("onMouseDown", px(5));
    plot.send("onMouseMove", px(12));
    plot.send("onMouseUp", px(12));
    expect(onBrush).toHaveBeenCalledTimes(1);
    expect(onBrush).toHaveBeenCalledWith({ z0: 5, z1: 12 });
  });

  it("moves the keyboard cursor over the bins", () => {
    const plot = mounted({ model: plotModel({ bins: binsOf() }), cursorKeys: true });
    const spoken = (bin: number) => spectrumReadout(plot.model, bin, null);
    expect(plot.key("End")).toBe(true);
    expect(plot.status()).toBe(spoken(9));
    expect(spoken(9)!.startsWith("bin 9 (ch 36–39)")).toBe(true);
    expect(plot.overlay.strokes.filter((s) => verticalAt(s.ops, pxOf(BIN_XS[9]))).length).toBeGreaterThan(0);
    expect(plot.key("ArrowRight")).toBe(true);
    expect(plot.status()).toBe(spoken(9));
    expect(plot.key("Home")).toBe(true);
    expect(plot.status()).toBe(spoken(0));
  });

  it("names the bins for a screen reader", () => {
    const markup = (model: SpectrumPlotModel) => {
      hooks.mount();
      return renderToStaticMarkup(hooks.render(plotFunction, { model }) as ReactElement);
    };
    const label = (html: string) => html.match(/aria-label="([^"]*)"/)![1];
    expect(label(markup(plotModel({ bins: binsOf() }))).endsWith(", 40 channels in 10 bins")).toBe(true);
    expect(label(markup(plotModel())).endsWith(", 40 channels")).toBe(true);
  });
});

describe("SpectrumPlot band strips", () => {
  const band: BandMark = { id: "b", label: "CO2 4.3", species: "CO2", kind: "molecular_band", lo: XS[10], hi: XS[20] };
  const stripsOf = (rects: { style: unknown; x: number; y: number; w: number; h: number }[], style: string) => rects.filter((r) => r.style === style);

  it("shades a band and labels it at the top", () => {
    const plot = mounted({ model: plotModel(), bands: [band] });
    const strips = stripsOf(plot.overlay.rects, "rgba(52,211,153,0.12)");
    expect(strips).toHaveLength(1);
    expect(strips[0].x).toBeCloseTo(px(10), 6);
    expect(strips[0].y).toBe(10);
    expect(strips[0].w).toBeCloseTo(px(20) - px(10), 6);
    expect(strips[0].h).toBe(146);
    const label = plot.overlay.texts.filter((t) => t.text === "CO2 4.3");
    expect(label).toHaveLength(1);
    expect(label[0].x).toBeCloseTo(px(10) + 3, 6);
    expect(label[0].y).toBe(19);
    expect(label[0].style).toBe("rgba(52,211,153,0.9)");

    const atomic = mounted({ model: plotModel(), bands: [{ ...band, kind: "atomic_line" }] });
    expect(stripsOf(atomic.overlay.rects, "rgba(250,204,21,0.14)")).toHaveLength(1);
    expect(atomic.overlay.texts.find((t) => t.text === "CO2 4.3")?.style).toBe("rgba(250,204,21,0.9)");
    const instrument = mounted({ model: plotModel(), bands: [{ ...band, kind: "instrument" }] });
    expect(stripsOf(instrument.overlay.rects, "rgba(161,161,170,0.16)")).toHaveLength(1);
    expect(instrument.overlay.texts.find((t) => t.text === "CO2 4.3")?.style).toBe("rgba(161,161,170,0.9)");

    const zoomed = mounted({ model: plotModel(ZOOMED), bands: [band] });
    const zoomedStrip = stripsOf(zoomed.overlay.rects, "rgba(52,211,153,0.12)");
    expect(zoomedStrip).toHaveLength(1);
    expect(zoomedStrip[0].x).toBeCloseTo(px(10), 6);
    expect(zoomedStrip[0].w).toBeCloseTo(px(20) - px(10), 6);

    const bare = mounted({ model: plotModel() });
    expect(stripsOf(bare.overlay.rects, "rgba(52,211,153,0.12)")).toHaveLength(0);
  });

  it("skips a label that would run into the one before it", () => {
    const crowding: BandMark = { ...band, id: "c", label: "CO 4.7", lo: XS[10] + 2e-5, hi: XS[25] };
    const plot = mounted({ model: plotModel(), bands: [band, crowding] });
    expect(stripsOf(plot.overlay.rects, "rgba(52,211,153,0.12)")).toHaveLength(2);
    expect(plot.overlay.texts.map((t) => t.text)).toContain("CO2 4.3");
    expect(plot.overlay.texts.map((t) => t.text)).not.toContain("CO 4.7");
    const apart = mounted({ model: plotModel(), bands: [band, { ...crowding, lo: XS[30], hi: XS[35] }] });
    expect(apart.overlay.texts.map((t) => t.text)).toContain("CO2 4.3");
    expect(apart.overlay.texts.map((t) => t.text)).toContain("CO 4.7");
  });

  it("reports a click inside a band, after the rest lines", () => {
    const onPickBand = vi.fn();
    const onPickMark = vi.fn();
    const onBrush = vi.fn();
    const plot = mounted({ model: plotModel(), bands: [band], onPickBand, onPickMark, onBrush });
    plot.send("onMouseDown", px(15));
    plot.send("onMouseUp", px(15));
    expect(onPickBand).toHaveBeenCalledTimes(1);
    expect(onPickBand.mock.calls[0][0]).toBe(band);
    plot.send("onMouseDown", px(25));
    plot.send("onMouseUp", px(25));
    expect(onPickBand).toHaveBeenCalledTimes(1);
    plot.send("onMouseDown", px(12));
    plot.send("onMouseMove", px(18));
    plot.send("onMouseUp", px(18));
    expect(onBrush).toHaveBeenCalledTimes(1);
    expect(onPickBand).toHaveBeenCalledTimes(1);
    expect(onPickMark).not.toHaveBeenCalled();

    const markPick = vi.fn();
    const bandPick = vi.fn();
    const withMark = mounted({ model: plotModel(), bands: [band], marks: [mark("m", XS[15])], onPickBand: bandPick, onPickMark: markPick });
    withMark.send("onMouseDown", px(15) + 2);
    withMark.send("onMouseUp", px(15) + 2);
    expect(markPick).toHaveBeenCalledTimes(1);
    expect(bandPick).not.toHaveBeenCalled();

    const silent = mounted({ model: plotModel(), bands: [band] });
    expect(() => {
      silent.send("onMouseDown", px(15));
      silent.send("onMouseUp", px(15));
    }).not.toThrow();
  });

  it("finds a band thinner than a pixel", () => {
    const line: BandMark = { ...band, kind: "atomic_line", lo: XS[20], hi: XS[20] };
    const onPickBand = vi.fn();
    const plot = mounted({ model: plotModel(), bands: [line], onPickBand });
    plot.send("onMouseDown", px(20) + 4);
    plot.send("onMouseUp", px(20) + 4);
    expect(onPickBand).toHaveBeenCalledTimes(1);
    expect(onPickBand.mock.calls[0][0]).toBe(line);
    plot.send("onMouseDown", px(20) + 9);
    plot.send("onMouseUp", px(20) + 9);
    expect(onPickBand).toHaveBeenCalledTimes(1);
  });
});
