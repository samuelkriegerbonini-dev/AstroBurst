import { memo, useCallback, useEffect, useId, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { MousePointer2, Circle, Egg, Square, CircleDot, Pentagon, Minus, Dot, Info, X, type LucideIcon } from "lucide-react";
import type { RegionTool } from "../../shared/types/regions";
import { regionStore } from "../../utils/regionStore";
import { useRegionTool } from "../../hooks/useRegionStore";
import { appModeStore } from "../../hooks/useAppMode";
import { placeHelpCard, type HelpCardPlacement } from "./helpCardPlacement";
import "./region-tool-help.css";

const LINE_HELP_SESSION_KEY = "astroburst.line-cut-help.seen";
let lineHelpSeen = false;

function firstLineHelp(): boolean {
  try {
    if (sessionStorage.getItem(LINE_HELP_SESSION_KEY) === "true") return false;
    sessionStorage.setItem(LINE_HELP_SESSION_KEY, "true");
    lineHelpSeen = true;
    return true;
  } catch {
    const first = !lineHelpSeen;
    lineHelpSeen = true;
    return first;
  }
}

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
  const mode = useSyncExternalStore(appModeStore.subscribe, appModeStore.get, appModeStore.get);
  const [helpOpen, setHelpOpen] = useState(false);
  const [placement, setPlacement] = useState<HelpCardPlacement | null>(null);
  const lineButtonRef = useRef<HTMLButtonElement>(null);
  const helpButtonRef = useRef<HTMLButtonElement>(null);
  const helpRef = useRef<HTMLDivElement>(null);
  const helpId = useId();
  const helpBodyId = `${helpId}-body`;
  const lineActive = active === "line";
  const helpVisible = helpOpen && lineActive && mode === "workspace";
  const pick = useCallback((t: RegionTool) => {
    regionStore.clearDraft();
    const next = regionStore.getTool() === t ? "none" : t;
    regionStore.setTool(next);
    setHelpOpen(next === "line" && firstLineHelp());
  }, []);

  useEffect(() => {
    if (active !== "line" || mode !== "workspace") setHelpOpen(false);
  }, [active, mode]);

  useLayoutEffect(() => {
    const button = lineButtonRef.current;
    const card = helpRef.current;
    if (!helpVisible || !button || !card) return;
    const place = () => {
      const next = placeHelpCard(button.getBoundingClientRect(), card.offsetHeight, { width: window.innerWidth, height: window.innerHeight });
      setPlacement((current) => current?.left === next.left && current.top === next.top && current.width === next.width ? current : next);
    };
    place();
    const observer = new ResizeObserver(place);
    observer.observe(card);
    const toolbar = button.closest(".ab-viewer-toolbar");
    if (toolbar) observer.observe(toolbar);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [helpVisible]);

  useEffect(() => {
    if (!helpVisible) return;
    const dismissOutside = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (helpRef.current?.contains(target) || lineButtonRef.current?.contains(target) || helpButtonRef.current?.contains(target)) return;
      setHelpOpen(false);
    };
    document.addEventListener("pointerdown", dismissOutside, true);
    return () => document.removeEventListener("pointerdown", dismissOutside, true);
  }, [helpVisible]);

  const dismissHelp = () => {
    setHelpOpen(false);
    lineButtonRef.current?.focus({ preventScroll: true });
  };

  return (
    <>
      <div className="ab-viewer-toolbar-divider" />
      <div className="ab-viewer-toolbar-group">
        {TOOLS.map(({ tool, title, Icon }) => (
          <button
            key={tool}
            ref={tool === "line" ? lineButtonRef : undefined}
            type="button"
            onClick={() => pick(tool)}
            className={`ab-viewer-btn ${active === tool ? "ab-viewer-btn-active" : ""}`}
            title={title}
            aria-label={title}
            aria-pressed={active === tool}
            aria-describedby={tool === "line" && helpVisible ? helpBodyId : undefined}
          >
            <Icon size={14} />
          </button>
        ))}
        <button
          ref={helpButtonRef}
          type="button"
          className={`ab-viewer-btn ab-region-help-trigger ${lineActive ? "" : "ab-region-help-trigger-idle"}`}
          title="Line cut help"
          aria-label="Line cut help"
          aria-expanded={lineActive ? helpVisible : undefined}
          aria-controls={helpVisible ? helpId : undefined}
          onClick={() => setHelpOpen((open) => !open)}
        >
          <Info size={13} />
        </button>
      </div>
      {helpVisible && createPortal(
        <div
          ref={helpRef}
          id={helpId}
          role="note"
          aria-label="Line cut help"
          className="ab-region-help"
          style={placement ?? { left: 0, top: 0, visibility: "hidden" }}
          onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            event.preventDefault();
            event.stopPropagation();
            dismissHelp();
          }}
        >
          <div className="ab-region-help-heading">
            <strong>Line cut</strong>
            <button type="button" title="Dismiss line cut help" aria-label="Dismiss line cut help" onClick={dismissHelp}>
              <X size={12} />
            </button>
          </div>
          <p id={helpBodyId}>Drag from start to end on the image. Image &gt; Profiles shows the line cut and length; WCS adds sky separation. Escape cancels an unfinished line.</p>
        </div>,
        document.body,
      )}
    </>
  );
}

export default memo(RegionToolbar);
