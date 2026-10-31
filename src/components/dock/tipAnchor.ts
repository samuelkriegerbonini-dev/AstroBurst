import type { DockToolId } from "../../utils/dockLayout";

export function tipAnchorSelector(tool: DockToolId): string {
  return `[data-dock-strip] [data-tool-id="${tool}"]`;
}
