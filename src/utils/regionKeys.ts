import type { RegionShapeKind, RegionTool } from "../shared/types/regions";

export type RegionKeyAction = "delete-selected" | "close-polygon" | "clear-draft" | "deselect" | "tool-off";

export interface RegionKeyContext {
  editable: boolean;
  inViewer: boolean;
  inModal: boolean;
  inWorkspace?: boolean;
  draft: RegionShapeKind | null;
  hasSelection: boolean;
  tool: RegionTool;
}

export function regionKeyAction(key: string, ctx: RegionKeyContext): RegionKeyAction | null {
  if (ctx.editable || ctx.inModal || ctx.inWorkspace === false) return null;
  if (key === "Delete" || key === "Backspace") return ctx.inViewer && ctx.hasSelection ? "delete-selected" : null;
  if (key === "Enter") return ctx.draft === "polygon" ? "close-polygon" : null;
  if (key !== "Escape") return null;
  if (ctx.draft !== null) return "clear-draft";
  if (ctx.hasSelection) return "deselect";
  return ctx.tool === "none" ? null : "tool-off";
}
