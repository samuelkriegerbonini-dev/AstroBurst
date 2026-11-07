import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DqMaskData, DqOverlaySettings } from "../../../shared/types/dq";

const dq = vi.hoisted(() => ({ state: { dqMask: null, overlay: { enabled: false, mask: 0 } } as { dqMask: DqMaskData | null; overlay: DqOverlaySettings } }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => dq.state }));

import DqOverlayCanvas from "../DqOverlayCanvas";
import { hooks } from "../../../test/hookRuntime";
import { DQ_OVERLAY_ALPHA, DQ_OVERLAY_RGB } from "../../../utils/dqOverlay";

interface FakeImage {
  data: Uint8ClampedArray;
}

function fakeCanvas() {
  const painted: FakeImage[] = [];
  const context = {
    createImageData: (width: number, height: number): FakeImage => ({ data: new Uint8ClampedArray(width * height * 4) }),
    putImageData: vi.fn((image: FakeImage) => { painted.push(image); }),
  };
  const canvas = { width: 0, height: 0, style: { display: "none" }, getContext: vi.fn(() => context) };
  return { canvas, context, painted };
}

function mask(cells: number[], width = 2, height = 2): DqMaskData {
  return { width, height, mask: 1, tableId: 1, cells: Uint8Array.from(cells) };
}

function render(canvas: unknown, state: Partial<typeof dq.state>): void {
  dq.state = { ...dq.state, ...state };
  hooks.render(DqOverlayCanvas, { canvasRef: { current: canvas as HTMLCanvasElement } });
}

beforeEach(() => {
  hooks.mount();
  dq.state = { dqMask: null, overlay: { enabled: false, mask: 0 } };
});

afterEach(() => hooks.mount());

describe("DqOverlayCanvas", () => {
  it("paints the flagged cells of an active mask at the mask size and shows the canvas", () => {
    const { canvas, painted } = fakeCanvas();
    render(canvas, { overlay: { enabled: true, mask: 1 }, dqMask: mask([1, 0, 0, 1]) });
    expect(canvas.style.display).toBe("block");
    expect([canvas.width, canvas.height]).toEqual([2, 2]);
    expect(painted).toHaveLength(1);
    const alpha = Math.round(255 * DQ_OVERLAY_ALPHA);
    expect([...painted[0].data]).toEqual([...DQ_OVERLAY_RGB, alpha, 0, 0, 0, 0, 0, 0, 0, 0, ...DQ_OVERLAY_RGB, alpha]);
  });

  it("hides the canvas when the mask goes away or the overlay is switched off", () => {
    const { canvas } = fakeCanvas();
    render(canvas, { overlay: { enabled: true, mask: 1 }, dqMask: mask([1, 0, 0, 1]) });
    render(canvas, { dqMask: null });
    expect(canvas.style.display).toBe("none");
    render(canvas, { dqMask: mask([0, 1, 0, 0]) });
    expect(canvas.style.display).toBe("block");
    render(canvas, { overlay: { enabled: false, mask: 1 } });
    expect(canvas.style.display).toBe("none");
  });

  it("repaints only when the mask object changes", () => {
    const { canvas, painted } = fakeCanvas();
    const first = mask([1, 0, 0, 0]);
    render(canvas, { overlay: { enabled: true, mask: 1 }, dqMask: first });
    render(canvas, { dqMask: first });
    expect(painted).toHaveLength(1);
    render(canvas, { dqMask: mask([0, 0, 0, 1]) });
    expect(painted).toHaveLength(2);
  });

  it("waits for the canvas and paints the active mask once it is attached", () => {
    expect(() => render(null, { overlay: { enabled: true, mask: 1 }, dqMask: mask([1, 1, 1, 1]) })).not.toThrow();
    const { canvas, painted } = fakeCanvas();
    render(canvas, {});
    expect(painted).toHaveLength(1);
    expect(canvas.style.display).toBe("block");
  });
});
