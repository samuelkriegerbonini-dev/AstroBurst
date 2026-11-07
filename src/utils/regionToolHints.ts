import type { RegionTool } from "../shared/types/regions";

const REGION_TOOL_HINTS: Record<Exclude<RegionTool, "none">, string> = {
  select: "Click a region to select, drag to move; Delete removes; Esc deselects",
  circle: "Drag from the center out to the radius",
  ellipse: "Drag from the center; horizontal and vertical distances set the radii",
  box: "Drag from one corner to the opposite corner",
  annulus: "Drag from the center to the outer radius; the inner radius starts at half and has its own handle",
  polygon: "Click each vertex; double-click or Enter closes; Esc cancels",
  line: "Drag from start to end; Esc cancels",
  point: "Click to place a point",
};

export function regionToolHint(tool: RegionTool): string | null {
  return tool === "none" ? null : REGION_TOOL_HINTS[tool];
}
