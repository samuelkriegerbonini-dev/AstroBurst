import type { RegionTool } from "../shared/types/regions";

export function regionLayerSwallowsClick(tool: RegionTool, layerOwnsPress: boolean): boolean {
  if (tool === "none") return false;
  return tool !== "select" || layerOwnsPress;
}

export type ViewportClickRoute = "pixel" | "canvas" | "none";

export function viewportClickRoute(
  cursorMode: "pan" | "crosshair",
  onImage: boolean,
  hasPixelHandler: boolean,
  hasCanvasHandler: boolean,
): ViewportClickRoute {
  if (!onImage) return "none";
  if (cursorMode === "crosshair" && hasPixelHandler) return "pixel";
  return hasCanvasHandler ? "canvas" : "none";
}
