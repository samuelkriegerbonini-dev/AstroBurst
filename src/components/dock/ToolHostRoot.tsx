import { useState } from "react";
import { DOCK_TOOL_IDS, anchorOf, type DockAnchor, type DockLayout, type DockToolId } from "../../utils/dockLayout";
import { DOCK_TOOLS } from "./toolRegistry";
import ToolMount from "./ToolMount";

export interface ToolHostRootProps {
  tools: Partial<Record<DockToolId, React.ReactNode>>;
  containers: React.RefObject<Record<DockAnchor, HTMLElement | null>>;
  layout: DockLayout;
  open: Record<DockAnchor, DockToolId | null>;
  shown: Record<DockAnchor, DockToolId | null>;
  hasFile: boolean;
  fileKey: string | null;
}

function registryContent(): Record<DockToolId, React.ReactNode> {
  const content = {} as Record<DockToolId, React.ReactNode>;
  for (const id of DOCK_TOOL_IDS) content[id] = DOCK_TOOLS[id].render();
  return content;
}

export default function ToolHostRoot({ tools, containers, layout, open, shown, hasFile, fileKey }: ToolHostRootProps) {
  const [registry] = useState(registryContent);
  return (
    <>
      {DOCK_TOOL_IDS.map((tool) => {
        const anchor = anchorOf(layout, tool);
        return (
          <ToolMount
            key={tool}
            tool={tool}
            content={tools[tool] !== undefined ? tools[tool] : registry[tool]}
            containers={containers}
            anchor={anchor}
            activeTool={open[anchor]}
            shownTool={shown[anchor]}
            anchorMounted={shown[anchor] !== null}
            hasFile={hasFile}
            fileKey={fileKey}
          />
        );
      })}
    </>
  );
}
