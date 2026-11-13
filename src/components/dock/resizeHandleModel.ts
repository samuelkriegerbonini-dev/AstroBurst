import type { DockSizes } from "../../utils/dockLayout";
import type { ResizeSession } from "./ResizeHandle";

export const RESIZE_STEP_PX = 16;
export const RESIZE_STEP_LARGE_PX = 64;
export const RESIZE_HANDLE_LABELS: Record<keyof DockSizes, string> = {
  leftW: "Left panel width",
  rightW: "Right panel width",
  bottomH: "Bottom panel height",
  bottomSplit: "Bottom panel split",
};

export type ResizeKeyMove = { kind: "step"; pixels: number } | { kind: "min" } | { kind: "max" };

export function resizeKeyMove(key: string, shiftKey: boolean, axis: "x" | "y"): ResizeKeyMove | null {
  if (key === "Home") return { kind: "min" };
  if (key === "End") return { kind: "max" };
  const step = shiftKey ? RESIZE_STEP_LARGE_PX : RESIZE_STEP_PX;
  const [backward, forward] = axis === "x" ? ["ArrowLeft", "ArrowRight"] : ["ArrowUp", "ArrowDown"];
  if (key === forward) return { kind: "step", pixels: step };
  if (key === backward) return { kind: "step", pixels: -step };
  return null;
}

export function resizeTarget(move: ResizeKeyMove, session: Pick<ResizeSession, "start" | "min" | "max" | "direction">): number {
  const top = Math.max(session.min, session.max);
  if (move.kind === "min") return session.min;
  if (move.kind === "max") return top;
  return Math.round(Math.max(session.min, Math.min(top, session.start + session.direction * move.pixels)));
}
