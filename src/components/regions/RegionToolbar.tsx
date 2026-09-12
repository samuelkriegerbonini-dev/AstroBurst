import { memo, useCallback } from "react";
import { MousePointer2, Circle, Egg, Square, CircleDot, Pentagon, Minus, Dot, type LucideIcon } from "lucide-react";
import type { RegionTool } from "../../shared/types/regions";
import { regionStore } from "../../utils/regionStore";
import { useRegionTool } from "../../hooks/useRegionStore";

const TOOLS: { tool: Exclude<RegionTool, "none">; title: string; Icon: LucideIcon }[] = [
  { tool: "select", title: "Select / move regions", Icon: MousePointer2 },
  { tool: "circle", title: "Draw circle", Icon: Circle },
  { tool: "ellipse", title: "Draw ellipse", Icon: Egg },
  { tool: "box", title: "Draw box", Icon: Square },
  { tool: "annulus", title: "Draw annulus", Icon: CircleDot },
  { tool: "polygon", title: "Draw polygon (click vertices, double-click or Enter to close)", Icon: Pentagon },
  { tool: "line", title: "Draw line cut", Icon: Minus },
  { tool: "point", title: "Place point", Icon: Dot },
];

function RegionToolbar() {
  const active = useRegionTool();
  const pick = useCallback((t: RegionTool) => {
    regionStore.clearDraft();
    regionStore.setTool(regionStore.getTool() === t ? "none" : t);
  }, []);

  return (
    <>
      <div className="ab-viewer-toolbar-divider" />
      <div className="ab-viewer-toolbar-group">
        {TOOLS.map(({ tool, title, Icon }) => (
          <button
            key={tool}
            onClick={() => pick(tool)}
            className={`ab-viewer-btn ${active === tool ? "ab-viewer-btn-active" : ""}`}
            title={title}
          >
            <Icon size={14} />
          </button>
        ))}
      </div>
    </>
  );
}

export default memo(RegionToolbar);
