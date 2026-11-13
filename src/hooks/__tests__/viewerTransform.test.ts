import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../test/hookRuntime";
import { forgetViewerView, useViewerTransform } from "../useViewerTransform";

vi.mock("react", async (original) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await original()));

interface Sizes {
  renderW: number;
  renderH: number;
  fitsW?: number;
  fitsH?: number;
}

type Viewer = ReturnType<typeof useViewerTransform>;

const container = {
  clientWidth: 1024,
  clientHeight: 1024,
  getBoundingClientRect: () => ({ left: 0, top: 0, width: 1024, height: 1024 }),
  addEventListener: () => {},
  removeEventListener: () => {},
};
const containerRef = { current: null as HTMLDivElement | null };

const TEXTURE_OF_4096: Sizes = { renderW: 2048, renderH: 2048, fitsW: 4096, fitsH: 4096 };
const FITTED_4096 = { scale: 0.5, x: 0, y: 0 };
const ZOOMED_4X_AND_PANNED = { scale: 8, x: -6880, y: -7280 };

let frames: (() => void)[] = [];
let resizeCallbacks: (() => void)[] = [];

function flushFrames(): void {
  for (const frame of frames.splice(0)) frame();
}

function Harness(sizes: Sizes): Viewer {
  return useViewerTransform({ containerRef, ...sizes });
}

function show(sizes: Sizes): Viewer {
  return hooks.render(Harness, sizes) as Viewer;
}

function mountViewer(sizes: Sizes): Viewer {
  show(sizes).attachContainer(container as unknown as HTMLDivElement);
  show(sizes);
  flushFrames();
  return show(sizes);
}

function unmountViewer(): void {
  hooks.mount();
  containerRef.current = null;
}

function zoomTo4xAndPan(sizes: Sizes): void {
  mountViewer(sizes).zoomToFits(4);
  show(sizes).setTransform((t) => ({ ...t, x: t.x + 800, y: t.y + 400 }));
  show(sizes);
}

const fitsZoom = (viewer: Viewer) => viewer.transform.scale / viewer.fitsPerRender;

beforeEach(() => {
  unmountViewer();
  frames = [];
  resizeCallbacks = [];
  vi.stubGlobal("requestAnimationFrame", (frame: () => void) => frames.push(frame));
  vi.stubGlobal("cancelAnimationFrame", () => {});
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        resizeCallbacks.push(callback);
      }
      observe() {}
      disconnect() {}
    },
  );
});

afterEach(() => {
  unmountViewer();
  forgetViewerView();
  vi.unstubAllGlobals();
});

describe("useViewerTransform keeps the view of one FITS grid", () => {
  it("fits the first view of a grid to the window", () => {
    const viewer = mountViewer(TEXTURE_OF_4096);
    expect(viewer.transform).toEqual(FITTED_4096);
    expect(viewer.zoomPct).toBe("25%");
  });

  it("keeps 4x in FITS pixels and the centre when the viewer mounts again on the same grid, as at every GPU cube frame step", () => {
    zoomTo4xAndPan(TEXTURE_OF_4096);
    expect(show(TEXTURE_OF_4096).transform).toEqual(ZOOMED_4X_AND_PANNED);
    unmountViewer();
    const again = mountViewer(TEXTURE_OF_4096);
    expect(fitsZoom(again)).toBe(4);
    expect(again.transform).toEqual(ZOOMED_4X_AND_PANNED);
  });

  it("keeps 4x and the FITS centre when the grid comes back on a texture of another size", () => {
    zoomTo4xAndPan(TEXTURE_OF_4096);
    unmountViewer();
    const smallerTexture = mountViewer({ ...TEXTURE_OF_4096, renderW: 1024, renderH: 1024 });
    expect(smallerTexture.zoomPct).toBe("400%");
    expect(smallerTexture.transform).toEqual({ scale: 16, x: -6880, y: -7280 });
  });

  it("keeps the view when the image size drops to zero and comes back on the same grid, as when the CPU viewer loads a new stretch, frame or file URL", () => {
    zoomTo4xAndPan(TEXTURE_OF_4096);
    show({ ...TEXTURE_OF_4096, renderW: 0, renderH: 0 });
    show(TEXTURE_OF_4096);
    flushFrames();
    expect(show(TEXTURE_OF_4096).transform).toEqual(ZOOMED_4X_AND_PANNED);
  });

  it("does not refit a kept view when the container is resized later", () => {
    zoomTo4xAndPan(TEXTURE_OF_4096);
    unmountViewer();
    mountViewer(TEXTURE_OF_4096);
    for (const resized of resizeCallbacks) resized();
    flushFrames();
    expect(show(TEXTURE_OF_4096).transform).toEqual(ZOOMED_4X_AND_PANNED);
  });

  it("fits a view of another grid", () => {
    zoomTo4xAndPan(TEXTURE_OF_4096);
    unmountViewer();
    const otherGrid = mountViewer({ ...TEXTURE_OF_4096, fitsW: 8192, fitsH: 8192 });
    expect(otherGrid.transform).toEqual(FITTED_4096);
    expect(otherGrid.zoomPct).toBe("13%");
  });

  it("fits again after Fit to window, also on the same grid", () => {
    zoomTo4xAndPan(TEXTURE_OF_4096);
    show(TEXTURE_OF_4096).fitToWindow();
    show(TEXTURE_OF_4096);
    unmountViewer();
    expect(mountViewer(TEXTURE_OF_4096).transform).toEqual(FITTED_4096);
  });
});
