import { firstRendered, focusStripButton, rightRailToolsTrigger } from "./dockGeometry";
import type { DockToolId } from "../../utils/dockLayout";

export function railToolTarget(tool: DockToolId, root: ParentNode = document): HTMLElement | null {
  return firstRendered(root.querySelectorAll<HTMLElement>(`[data-dock-strip] [data-tool-id="${tool}"]`)) ?? rightRailToolsTrigger(root);
}

export function focusRailTool(tool: DockToolId, root: ParentNode = document): void {
  focusStripButton(railToolTarget(tool, root));
}
