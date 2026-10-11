import { describe, expect, it } from "vitest";
import { GPU_LOST_LABEL, GPU_LOST_WORKER_TITLE, afterGpuLoss, gpuLossToggleView } from "../gpuLoss";
import { gpuToggleView, type GpuToggleInput } from "../previewShell";

const onGpu: GpuToggleInput = {
  probing: false,
  loading: false,
  available: true,
  supported: true,
  useGpu: true,
  loadError: null,
  reason: null,
  pngOnlyMono: false,
  cpuViewerShown: false,
};

describe("afterGpuLoss", () => {
  it("keeps a mono view on the GPU viewport, where the CPU worker takes over the rendering", () => {
    expect(afterGpuLoss({ rgbView: false })).toBe("worker");
  });

  it("drops an RGB view to the CPU viewer, as before", () => {
    expect(afterGpuLoss({ rgbView: true })).toBe("cpu");
  });
});

describe("gpuLossToggleView", () => {
  it("names the device loss and the CPU worker on the toggle while the worker renders", () => {
    const view = gpuLossToggleView(gpuToggleView(onGpu), true);
    expect(view).toEqual({ state: "gpu", label: GPU_LOST_LABEL, title: GPU_LOST_WORKER_TITLE, tone: "failed" });
    expect(view.title).toContain("GPU device lost: a CPU worker renders the display");
    expect(view.label).toBe("GPU lost");
  });

  it("leaves the toggle alone once a new GPU viewport renders", () => {
    expect(gpuLossToggleView(gpuToggleView(onGpu), false)).toEqual(gpuToggleView(onGpu));
  });

  it("leaves the loading spinner and a load failure alone", () => {
    const loading = gpuToggleView({ ...onGpu, loading: true });
    const failed = gpuToggleView({ ...onGpu, loadError: "read error" });
    expect(gpuLossToggleView(loading, true)).toEqual(loading);
    expect(gpuLossToggleView(failed, true)).toEqual(failed);
  });
});
