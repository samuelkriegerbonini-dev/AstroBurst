import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../../test/hookRuntime";
import { elementsOf } from "../../../test/elementTree";

type Fn = (props: Record<string, unknown>) => unknown;
type Call = { name: string; args: unknown[] };

const dpr = vi.hoisted(() => ({ size: vi.fn() }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../utils/canvasDpr", () => ({ devicePixelRatioOrOne: () => 1.5, sizeCanvas: dpr.size }));

import HistogramPanel from "../HistogramPanel";

const RECORDED_METHODS = ["fillRect", "clearRect", "beginPath", "moveTo", "lineTo", "stroke", "setLineDash", "fillText"] as const;

function recorder() {
  const calls: Call[] = [];
  const ctx: Record<string, unknown> = {
    measureText: (text: string) => {
      calls.push({ name: "measureText", args: [text] });
      return { width: 20 };
    },
  };
  for (const name of RECORDED_METHODS) ctx[name] = (...args: unknown[]) => void calls.push({ name, args });
  return { ctx, calls, argsOf: (name: string) => calls.filter((c) => c.name === name).map((c) => c.args) };
}

let barsCtx: ReturnType<typeof recorder>;
let overlayCtx: ReturnType<typeof recorder>;
let bars: { width: number; height: number; parentElement: { getBoundingClientRect: () => { width: number } } };
let overlay: { width: number; height: number };

function drawn(props: Record<string, unknown> = {}): void {
  const panel = (HistogramPanel as unknown as { type: Fn }).type;
  const tree = hooks.render(panel, { bins: [1, 4, 9, 4, 1], ...props });
  const canvases = elementsOf(tree).filter((el) => el.type === "canvas");
  expect(canvases).toHaveLength(2);
  (canvases[0].props.ref as { current: unknown }).current = bars;
  (canvases[1].props.ref as { current: unknown }).current = overlay;
  hooks.render(panel, { bins: [1, 4, 9, 4, 1], ...props });
}

beforeEach(() => {
  vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
    cb();
    return 1;
  });
  vi.stubGlobal("cancelAnimationFrame", () => {});
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  barsCtx = recorder();
  overlayCtx = recorder();
  bars = { width: 0, height: 0, parentElement: { getBoundingClientRect: () => ({ width: 320.6 }) } };
  overlay = { width: 0, height: 0 };
  dpr.size.mockReset();
  dpr.size.mockImplementation((canvas: unknown) => (canvas === bars ? barsCtx.ctx : overlayCtx.ctx));
});

afterEach(() => {
  hooks.mount();
  vi.unstubAllGlobals();
});

describe("HistogramPanel canvases on scaled displays (viewer-8)", () => {
  it("sizes both canvases for the display ratio and draws in CSS pixels", () => {
    drawn();
    expect(dpr.size).toHaveBeenCalledWith(bars, 320, 110, 1.5);
    expect(dpr.size).toHaveBeenCalledWith(overlay, 320, 110, 1.5);
    const fills = barsCtx.argsOf("fillRect");
    expect(fills[0]).toEqual([0, 0, 320, 110]);
    const lastBar = fills[fills.length - 1];
    expect(lastBar[0]).toBe(256);
    expect(lastBar[2]).toBe(63.5);
  });

  it("draws the markers across the CSS width, not the backing store", () => {
    drawn({ shadow: 0.25, midtone: 0.5, highlight: 0.75, dataMin: 0, dataMax: 1 });
    const moves = overlayCtx.argsOf("moveTo");
    expect(moves).toContainEqual([80, 0]);
    expect(moves).toContainEqual([240, 0]);
    expect(overlayCtx.argsOf("clearRect")[0]).toEqual([0, 0, 320, 110]);
  });

  it("draws nothing while the panel has no width", () => {
    bars.parentElement.getBoundingClientRect = () => ({ width: 0 });
    drawn();
    expect(dpr.size).not.toHaveBeenCalled();
    expect(barsCtx.calls).toEqual([]);
    expect(overlayCtx.calls).toEqual([]);
  });
});
