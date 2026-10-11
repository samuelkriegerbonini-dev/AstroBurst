import type { GpuToggleView } from "./previewShell";

export type GpuLossFallback = "worker" | "cpu";

export const GPU_LOST_LABEL = "GPU lost";
export const GPU_LOST_WORKER_TITLE = "GPU device lost: a CPU worker renders the display; click to switch to CPU";

export function afterGpuLoss({ rgbView }: { rgbView: boolean }): GpuLossFallback {
  return rgbView ? "cpu" : "worker";
}

export function gpuLossToggleView(view: GpuToggleView, workerRenders: boolean): GpuToggleView {
  if (!workerRenders || view.state !== "gpu") return view;
  return { ...view, label: GPU_LOST_LABEL, title: GPU_LOST_WORKER_TITLE, tone: "failed" };
}
