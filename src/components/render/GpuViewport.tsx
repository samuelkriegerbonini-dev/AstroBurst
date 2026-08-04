import { useCallback, useRef, useState, memo } from "react";
import { ZoomIn, ZoomOut, Maximize, Square, RotateCcw, Move, Crosshair } from "lucide-react";
import { useViewerTransform, ZOOM_PRESETS } from "../../hooks/useViewerTransform";
import { screenToImagePixel } from "../../utils/pixelMapping";

interface GpuViewportProps {
  renderW: number;
  renderH: number;
  fitsW?: number;
  fitsH?: number;
  crosshairEnabled?: boolean;
  onMousePixel?: (x: number, y: number) => void;
  onPixelClick?: (x: number, y: number) => void;
  onMouseLeave?: () => void;
  onCanvasClick?: (e: React.MouseEvent<HTMLElement>) => void;
  overlayCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
  children: React.ReactNode;
}

function GpuViewport({
  renderW,
  renderH,
  fitsW,
  fitsH,
  crosshairEnabled = false,
  onMousePixel,
  onPixelClick,
  onMouseLeave,
  onCanvasClick,
  overlayCanvasRef,
  children,
}: GpuViewportProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [cursorMode, setCursorMode] = useState<"pan" | "crosshair">("pan");
  const [isPanning, setIsPanning] = useState(false);
  const isPanningRef = useRef(false);
  const panStart = useRef({ x: 0, y: 0, tx: 0, ty: 0 });
  const clickStart = useRef<{ x: number; y: number } | null>(null);

  const {
    transform, transformRef, setTransform,
    fitToWindow, zoomTo, zoomIn, zoomOut, setOneToOne,
    hasRenderDims, zoomPct,
  } = useViewerTransform({ containerRef, renderW, renderH });

  const effFitsW = fitsW ?? renderW;
  const effFitsH = fitsH ?? renderH;

  const handlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      clickStart.current = { x: e.clientX, y: e.clientY };
      if (e.button === 1 || (e.button === 0 && cursorMode === "pan")) {
        setIsPanning(true);
        isPanningRef.current = true;
        const t = transformRef.current;
        panStart.current = { x: e.clientX, y: e.clientY, tx: t.x, ty: t.y };
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
      }
    },
    [cursorMode, transformRef],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (isPanningRef.current) {
        const dx = e.clientX - panStart.current.x;
        const dy = e.clientY - panStart.current.y;
        setTransform((prev) => ({ ...prev, x: panStart.current.tx + dx, y: panStart.current.ty + dy }));
        return;
      }
      if (cursorMode === "crosshair" && onMousePixel && hasRenderDims) {
        const rect = containerRef.current?.getBoundingClientRect();
        if (!rect) return;
        const coord = screenToImagePixel(
          e.clientX, e.clientY, rect, transformRef.current,
          renderW, renderH, effFitsW, effFitsH,
        );
        if (coord) onMousePixel(coord.x, coord.y);
      }
    },
    [cursorMode, onMousePixel, hasRenderDims, renderW, renderH, effFitsW, effFitsH, setTransform, transformRef],
  );

  const handlePointerUp = useCallback(() => {
    setIsPanning(false);
    isPanningRef.current = false;
  }, []);

  const handleClick = useCallback(
    (e: React.MouseEvent<HTMLElement>) => {
      const start = clickStart.current;
      if (start && Math.hypot(e.clientX - start.x, e.clientY - start.y) > 4) return;
      if (cursorMode === "crosshair" && onPixelClick && hasRenderDims) {
        const rect = containerRef.current?.getBoundingClientRect();
        if (rect) {
          const coord = screenToImagePixel(
            e.clientX, e.clientY, rect, transformRef.current,
            renderW, renderH, effFitsW, effFitsH,
          );
          if (coord) {
            onPixelClick(coord.x, coord.y);
            return;
          }
        }
      }
      onCanvasClick?.(e);
    },
    [cursorMode, onPixelClick, hasRenderDims, renderW, renderH, effFitsW, effFitsH, onCanvasClick, transformRef],
  );

  const canvasLayerStyle: React.CSSProperties = {
    transform: `translate(${transform.x}px, ${transform.y}px) scale(${transform.scale})`,
    transformOrigin: "0 0",
    willChange: "transform",
    position: "absolute",
    top: 0,
    left: 0,
    width: renderW,
    height: renderH,
    imageRendering: transform.scale >= 4 ? "pixelated" : "auto",
  };

  return (
    <div className="ab-viewer-root">
      <div className="ab-viewer-toolbar">
        <div className="ab-viewer-toolbar-group">
          <button onClick={zoomIn} className="ab-viewer-btn" title="Zoom In"><ZoomIn size={14} /></button>
          <button onClick={zoomOut} className="ab-viewer-btn" title="Zoom Out"><ZoomOut size={14} /></button>
          <button onClick={fitToWindow} className="ab-viewer-btn" title="Fit to Window"><Maximize size={14} /></button>
          <button onClick={setOneToOne} className="ab-viewer-btn" title="1:1 Pixel"><Square size={13} /></button>
          <button onClick={fitToWindow} className="ab-viewer-btn" title="Reset View"><RotateCcw size={13} /></button>
        </div>

        {crosshairEnabled && (
          <>
            <div className="ab-viewer-toolbar-divider" />
            <div className="ab-viewer-toolbar-group">
              <button
                onClick={() => setCursorMode((m) => (m === "pan" ? "crosshair" : "pan"))}
                className={`ab-viewer-btn ${cursorMode === "crosshair" ? "ab-viewer-btn-active" : ""}`}
                title={cursorMode === "crosshair" ? "Switch to Pan" : "Switch to Crosshair"}
              >
                {cursorMode === "crosshair" ? <Crosshair size={14} /> : <Move size={14} />}
              </button>
            </div>
          </>
        )}

        <div className="ab-viewer-toolbar-group ml-auto">
          {ZOOM_PRESETS.map((z) => (
            <button
              key={z}
              onClick={() => zoomTo(z)}
              className={`ab-viewer-zoom-preset ${Math.abs(transform.scale - z) < 0.01 ? "ab-viewer-zoom-preset-active" : ""}`}
            >
              {z >= 1 ? `${z}x` : `${Math.round(z * 100)}%`}
            </button>
          ))}
          <span className="ab-viewer-status-item">{zoomPct}</span>
        </div>
      </div>

      <div
        ref={containerRef}
        className="ab-viewer-canvas"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerLeave={() => { handlePointerUp(); onMouseLeave?.(); }}
        onClick={handleClick}
        style={{ cursor: isPanning ? "grabbing" : cursorMode === "pan" ? "grab" : "crosshair" }}
      >
        <div style={canvasLayerStyle}>
          {children}
          {overlayCanvasRef && (
            <canvas
              ref={overlayCanvasRef}
              style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%", pointerEvents: "none", display: "none" }}
            />
          )}
        </div>
      </div>
    </div>
  );
}

export default memo(GpuViewport);
