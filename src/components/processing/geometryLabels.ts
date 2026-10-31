import type { GeometryOp } from "../../services/imageGeometry";

export function geometryShowLabel(op: GeometryOp): string {
  return `Show the ${op === "flip_h" || op === "flip_v" ? "flipped" : "rotated"} image`;
}
