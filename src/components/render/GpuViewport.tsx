import { useCallback, useRef, useState, memo } from "react";
import { ZoomIn, ZoomOut, Maximize, Move, Crosshair, Eye } from "lucide-react";
import { useViewerTransform, ZOOM_PRESETS } from "../../hooks/useViewerTransform";
import { screenToImagePixel } from "../../utils/pixelMapping";
import { viewportClickRoute } from "../../utils/regionClick";
import { imageRenderingFor, previewTextureBadge } from "../../utils/viewerZoom";
import { previewTextureTitle } from "../../utils/previewShell";
import RegionToolbar from "../regions/RegionToolbar";
import RegionsLayer from "../regions/RegionsLayer";
import OverlayLayer from "../viewer/OverlayLayer";

export interface ViewportOriginal {
  url: string;
  disabledReason: string | null;
}

interface GpuViewportProps {
  renderW: number;
  renderH: number;
  fitsW?: number;
  fitsH?: number;
  deepZoomOffered?: boolean;
  crosshairEnabled?: boolean;
  regionsEnabled?: boolean;
  original?: ViewportOriginal | null;
  onMousePixel?: (x: number, y: number) => void;
  onPixelClick?: (x: number, y: number) => void;
  onMouseLeave?: () => void;
  onCanvasPixelClick?: (x: number, y: number) => void;
  overlayCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
  dqCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
  children: React.ReactNode;
}

const HOLD_KEYS = new Set([" ", "Enter"]);

function GpuViewport({
  renderW,
  renderH,
  fitsW,
  fitsH,
  deepZoomOffered = true,
  crosshairEnabled = false,
  regionsEnabled = false,
  original = null,
  onMousePixel,
  onPixelClick,
  onMouseLeave,
  onCanvasPixelClick,
  overlayCanvasRef,
  dqCanvasRef,
  children,
}: GpuViewportProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [cursorMode, setCursorMode] = useState<"pan" | "crosshair">("pan");
  const [isPanning, setIsPanning] = useState(false);
  const [holdOriginal, setHoldOriginal] = useState(false);
  const isPanningRef = useRef(false);
  const panStart = useRef({ x: 0, y: 0, tx: 0, ty: 0 });
  const clickStart = useRef<{ x: number; y: number } | null>(null);

  const effFitsW = fitsW ?? renderW;
  const effFitsH = fitsH ?? renderH;

  const {
    attachContainer,
    transform, transformRef, setTransform,
    fitToWindow, zoomToFits, zoomIn, zoomOut, setOneToOne,
    hasRenderDims, zoomPct, isPresetActive,
  } = useViewerTransform({ containerRef, renderW, renderH, fitsW: effFitsW });

  const regionsActive = regionsEnabled && renderW > 0 && renderH > 0 && effFitsW > 0 && effFitsH > 0;
  const originalUsable = original !== null && original.disabledReason === null;
  if (holdOriginal && !originalUsable) setHoldOriginal(false);
  const showOriginal = holdOriginal && originalUsable;
  const textureBadge = previewTextureBadge(renderW, effFitsW);

  const startHold = useCallback((e: React.PointerEvent<HTMLButtonElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setHoldOriginal(true);
  }, []);

  const endHold = useCallback(() => setHoldOriginal(false), []);

  const holdKeyDown = useCallback((e: React.KeyboardEvent<HTMLButtonElement>) => {
    if (!HOLD_KEYS.has(e.key)) return;
    e.preventDefault();
    setHoldOriginal(true);
  }, []);

  const holdKeyUp = useCallback((e: React.KeyboardEvent<HTMLButtonElement>) => {
    if (HOLD_KEYS.has(e.key)) setHoldOriginal(false);
  }, []);

  const handlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      clickStart.current = { x: e.clientX, y: e.clientY };
      if (e.button === 1 || (e.button === 0 && cursorMode === "pan")) {
        setIsPanning(true);
        isPanningRef.current = true;
        const t = transformRef.current;
        panStart.current = { x: e.clientX, y: e.clientY, tx: t.x, ty: t.y };
        e.currentTarget.setPointerCapture(e.pointerId);
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
      if (onMousePixel && hasRenderDims) {
        const rect = containerRef.current?.getBoundingClientRect();
        if (!rect) return;
        const coord = screenToImagePixel(
          e.clientX, e.clientY, rect, transformRef.current,
          renderW, renderH, effFitsW, effFitsH,
        );
        if (coord) onMousePixel(coord.x, coord.y);
      }
    },
    [onMousePixel, hasRenderDims, renderW, renderH, effFitsW, effFitsH, setTransform, transformRef],
  );

  const handlePointerUp = useCallback(() => {
    setIsPanning(false);
    isPanningRef.current = false;
  }, []);

  const handleClick = useCallback(
    (e: React.MouseEvent<HTMLElement>) => {
      const start = clickStart.current;
      if (start && Math.hypot(e.clientX - start.x, e.clientY - start.y) > 4) return;
      const rect = hasRenderDims ? containerRef.current?.getBoundingClientRect() : undefined;
      const coord = rect
        ? screenToImagePixel(e.clientX, e.clientY, rect, transformRef.current, renderW, renderH, effFitsW, effFitsH)
        : null;
      const route = viewportClickRoute(cursorMode, coord !== null, !!onPixelClick, !!onCanvasPixelClick);
      if (!coord || route === "none") return;
      if (route === "pixel") onPixelClick?.(coord.x, coord.y);
      else onCanvasPixelClick?.(coord.x, coord.y);
    },
    [cursorMode, onPixelClick, hasRenderDims, renderW, renderH, effFitsW, effFitsH, onCanvasPixelClick, transformRef],
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
    imageRendering: imageRenderingFor(transform.scale),
  };

  return (
    <div className="ab-viewer-root">
      <div className="ab-viewer-toolbar">
        <div className="ab-viewer-toolbar-group">
          <button onClick={zoomIn} className="ab-viewer-btn" title="Zoom In"><ZoomIn size={14} /></button>
          <button onClick={zoomOut} className="ab-viewer-btn" title="Zoom Out"><ZoomOut size={14} /></button>
          <button onClick={fitToWindow} className="ab-viewer-btn" title="Fit to Window"><Maximize size={14} /></button>
          <button onClick={setOneToOne} className="ab-viewer-btn ab-viewer-btn-text" title="1:1 Pixel (one FITS pixel per screen pixel)">1:1</button>
        </div>

        {crosshairEnabled && (
          <>
            <div className="ab-viewer-toolbar-divider" />
            <div className="ab-viewer-toolbar-group" role="group" aria-label="Pointer mode">
              <button
                onClick={() => setCursorMode("pan")}
                className={`ab-viewer-btn ${cursorMode === "pan" ? "ab-viewer-btn-active" : ""}`}
                title="Pan"
                aria-pressed={cursorMode === "pan"}
              >
                <Move size={14} />
              </button>
              <button
                onClick={() => setCursorMode("crosshair")}
                className={`ab-viewer-btn ${cursorMode === "crosshair" ? "ab-viewer-btn-active" : ""}`}
                title="Crosshair"
                aria-pressed={cursorMode === "crosshair"}
              >
                <Crosshair size={14} />
              </button>
            </div>
          </>
        )}

        {original && (
          <>
            <div className="ab-viewer-toolbar-divider" />
            <div className="ab-viewer-toolbar-group">
              <button
                className={`ab-viewer-btn ab-viewer-btn-wide ${showOriginal ? "ab-viewer-btn-active" : ""}`}
                disabled={!originalUsable}
                title={original.disabledReason ?? "Hold to show the original (the loaded file) under the same zoom and pan"}
                aria-pressed={showOriginal}
                onPointerDown={startHold}
                onPointerUp={endHold}
                onPointerCancel={endHold}
                onLostPointerCapture={endHold}
                onKeyDown={holdKeyDown}
                onKeyUp={holdKeyUp}
                onBlur={endHold}
              >
                <Eye size={13} />
                Original
              </button>
            </div>
          </>
        )}

        {regionsActive && <RegionToolbar />}

        <div className="ab-viewer-toolbar-group ml-auto">
          <span className="ab-viewer-status-item ab-viewer-zoom-readout" title="Zoom level in FITS pixels (100% = one FITS pixel per screen pixel)">{zoomPct}</span>
          {textureBadge && (
            <span
              className="ab-viewer-preview-badge"
              title={previewTextureTitle({ renderW, renderH, fitsW: effFitsW, fitsH: effFitsH, deepZoomOffered })}
            >
              {textureBadge}
            </span>
          )}
          {ZOOM_PRESETS.map((z) => (
            <button
              key={z}
              onClick={() => zoomToFits(z)}
              className={`ab-viewer-zoom-preset ${isPresetActive(z) ? "ab-viewer-zoom-preset-active" : ""}`}
            >
              {z >= 1 ? `${z}x` : `${Math.round(z * 100)}%`}
            </button>
          ))}
        </div>
      </div>

      <div
        ref={attachContainer}
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
          {original && originalUsable && (
            <img
              src={original.url}
              alt="Original"
              draggable={false}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                height: "100%",
                maxWidth: "none",
                pointerEvents: "none",
                display: showOriginal ? "block" : "none",
              }}
            />
          )}
          {overlayCanvasRef && (
            <canvas
              ref={overlayCanvasRef}
              style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%", pointerEvents: "none", display: "none" }}
            />
          )}
          {dqCanvasRef && (
            <canvas
              ref={dqCanvasRef}
              style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%", pointerEvents: "none", display: "none" }}
            />
          )}
        </div>
        <OverlayLayer
          containerRef={containerRef}
          transform={transform}
          renderW={renderW}
          renderH={renderH}
          fitsW={effFitsW}
          fitsH={effFitsH}
          enabled={regionsActive}
        />
        <RegionsLayer
          containerRef={containerRef}
          transform={transform}
          renderW={renderW}
          renderH={renderH}
          fitsW={effFitsW}
          fitsH={effFitsH}
          enabled={regionsActive}
        />
        {showOriginal && (
          <div className="ab-viewer-compare-label-left" style={{ zIndex: 4 }}>Original</div>
        )}
      </div>
    </div>
  );
}

export default memo(GpuViewport);
