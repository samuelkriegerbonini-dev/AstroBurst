import {
  useState,
  useCallback,
  useRef,
  memo,
} from "react";
import {
  ZoomIn,
  ZoomOut,
  Maximize,
  Square,
  Columns2,
  Move,
  RotateCcw,
  Crosshair,
  Eye,
  EyeOff,
  Loader2,
  ImageOff,
  RefreshCw,
} from "lucide-react";
import { useViewerTransform, ZOOM_PRESETS } from "../../hooks/useViewerTransform";
import { useImageRetry } from "../../hooks/useImageRetry";
import { screenToImagePixel } from "../../utils/pixelMapping";

interface ViewerImage {
  url: string;
  label: string;
  width?: number;
  height?: number;
}

interface AdvancedImageViewerProps {
  original?: ViewerImage | null;
  processed?: ViewerImage | null;
  pixelValue?: { x: number; y: number; value: number } | null;
  onMousePixel?: (x: number, y: number) => void;
  onMouseLeave?: () => void;
  overlayCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
  className?: string;
}

function AdvancedImageViewer({
  original,
  processed,
  pixelValue,
  onMousePixel,
  onMouseLeave,
  overlayCanvasRef,
  className = "",
}: AdvancedImageViewerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [imgNatural, setImgNatural] = useState<{ w: number; h: number } | null>(null);
  const [compareMode, setCompareMode] = useState(false);
  const [comparePos, setComparePos] = useState(50);
  const [showOverlay, setShowOverlay] = useState(true);
  const [cursorMode, setCursorMode] = useState<"pan" | "crosshair">("pan");
  const [isPanning, setIsPanning] = useState(false);
  const isPanningRef = useRef(false);
  const panStart = useRef({ x: 0, y: 0, tx: 0, ty: 0 });
  const compareDragging = useRef(false);

  const activeImage = processed ?? original;
  const hasComparison = !!original && !!processed;
  const renderW = imgNatural?.w ?? 0;
  const renderH = imgNatural?.h ?? 0;

  const {
    transform, transformRef, setTransform,
    fitToWindow, zoomTo, zoomIn, zoomOut, setOneToOne,
    hasRenderDims, zoomPct,
  } = useViewerTransform({ containerRef, renderW, renderH });

  const mainRetry = useImageRetry(activeImage?.url);
  const origRetry = useImageRetry(original?.url);
  const procRetry = useImageRetry(processed?.url);

  const handlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (compareMode && Math.abs(e.nativeEvent.offsetX - (containerRef.current?.clientWidth ?? 0) * comparePos / 100) < 12) {
        compareDragging.current = true;
        return;
      }
      if (e.button === 1 || (e.button === 0 && cursorMode === "pan")) {
        setIsPanning(true);
        isPanningRef.current = true;
        const t = transformRef.current;
        panStart.current = { x: e.clientX, y: e.clientY, tx: t.x, ty: t.y };
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
      }
    },
    [compareMode, comparePos, cursorMode, transformRef],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (compareDragging.current && containerRef.current) {
        const rect = containerRef.current.getBoundingClientRect();
        setComparePos(Math.max(2, Math.min(98, ((e.clientX - rect.left) / rect.width) * 100)));
        return;
      }
      if (isPanningRef.current) {
        const dx = e.clientX - panStart.current.x;
        const dy = e.clientY - panStart.current.y;
        setTransform((prev) => ({ ...prev, x: panStart.current.tx + dx, y: panStart.current.ty + dy }));
        return;
      }
      if (cursorMode === "crosshair" && onMousePixel && hasRenderDims) {
        const rect = containerRef.current?.getBoundingClientRect();
        if (!rect) return;
        const fitsW = activeImage?.width ?? renderW;
        const fitsH = activeImage?.height ?? renderH;
        const coord = screenToImagePixel(
          e.clientX, e.clientY, rect, transformRef.current,
          renderW, renderH, fitsW, fitsH,
        );
        if (coord) onMousePixel(coord.x, coord.y);
      }
    },
    [cursorMode, onMousePixel, activeImage, renderW, renderH, hasRenderDims, setTransform, transformRef],
  );

  const handlePointerUp = useCallback(() => {
    setIsPanning(false);
    isPanningRef.current = false;
    compareDragging.current = false;
  }, []);

  const handleImageLoad = useCallback((e: React.SyntheticEvent<HTMLImageElement>) => {
    const img = e.currentTarget;
    const nw = img.naturalWidth;
    const nh = img.naturalHeight;
    if (nw > 0 && nh > 0) {
      setImgNatural({ w: nw, h: nh });
      mainRetry.onLoad();
    }
  }, [mainRetry.onLoad]);

  const imgStyle: React.CSSProperties = {
    transform: `translate(${transform.x}px, ${transform.y}px) scale(${transform.scale})`,
    transformOrigin: "0 0",
    imageRendering: transform.scale >= 4 ? "pixelated" : "auto",
    willChange: "transform",
    position: "absolute",
    top: 0,
    left: 0,
  };

  if (!activeImage) {
    return (
      <div className={`ab-viewer-empty ${className}`}>
        <div className="flex flex-col items-center gap-3 opacity-40">
          <Eye size={40} strokeWidth={1} className="text-zinc-600" />
          <span className="text-xs text-zinc-600 tracking-wide uppercase">Select a file to preview</span>
          <kbd className="text-[10px] text-zinc-700 font-mono px-2 py-0.5 rounded border border-zinc-800 bg-zinc-900">
            Click on file list
          </kbd>
        </div>
      </div>
    );
  }

  return (
    <div className={`ab-viewer-root ${className}`}>
      <div className="ab-viewer-toolbar">
        <div className="ab-viewer-toolbar-group">
          <button onClick={zoomIn} className="ab-viewer-btn" title="Zoom In"><ZoomIn size={14} /></button>
          <button onClick={zoomOut} className="ab-viewer-btn" title="Zoom Out"><ZoomOut size={14} /></button>
          <button onClick={fitToWindow} className="ab-viewer-btn" title="Fit to Window"><Maximize size={14} /></button>
          <button onClick={setOneToOne} className="ab-viewer-btn" title="1:1 Pixel"><Square size={13} /></button>
          <button onClick={fitToWindow} className="ab-viewer-btn" title="Reset View"><RotateCcw size={13} /></button>
        </div>

        <div className="ab-viewer-toolbar-divider" />

        <div className="ab-viewer-toolbar-group">
          <button
            onClick={() => setCursorMode((m) => (m === "pan" ? "crosshair" : "pan"))}
            className={`ab-viewer-btn ${cursorMode === "crosshair" ? "ab-viewer-btn-active" : ""}`}
            title={cursorMode === "crosshair" ? "Switch to Pan" : "Switch to Crosshair"}
          >
            {cursorMode === "crosshair" ? <Crosshair size={14} /> : <Move size={14} />}
          </button>
          {hasComparison && (
            <button
              onClick={() => setCompareMode((v) => !v)}
              className={`ab-viewer-btn ${compareMode ? "ab-viewer-btn-active" : ""}`}
              title="Before / After comparison"
            >
              <Columns2 size={14} />
            </button>
          )}
          <button
            onClick={() => setShowOverlay((v) => !v)}
            className={`ab-viewer-btn ${showOverlay ? "ab-viewer-btn-active" : ""}`}
            title="Toggle status bar"
          >
            {showOverlay ? <Eye size={14} /> : <EyeOff size={14} />}
          </button>
        </div>

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
        </div>
      </div>

      <div
        ref={containerRef}
        className="ab-viewer-canvas"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerLeave={() => { handlePointerUp(); onMouseLeave?.(); }}
        style={{ cursor: isPanning ? "grabbing" : cursorMode === "pan" ? "grab" : "crosshair" }}
      >
        {mainRetry.loading && !hasRenderDims && !mainRetry.error && (
          <div className="ab-viewer-loading-overlay">
            <Loader2 size={24} className="animate-spin" style={{ color: "var(--ab-teal)", opacity: 0.5 }} />
          </div>
        )}

        {mainRetry.error && (
          <div className="ab-viewer-error-overlay">
            <ImageOff size={28} strokeWidth={1.5} className="text-zinc-600" />
            <span className="text-xs text-zinc-500 mt-2">Preview failed to load</span>
            <button onClick={mainRetry.retry} className="ab-viewer-retry-btn">
              <RefreshCw size={12} /> Retry
            </button>
          </div>
        )}

        {!mainRetry.error && compareMode && original && processed ? (
          <>
            <div style={{ ...imgStyle, zIndex: 1 }}>
              <img src={procRetry.src ?? ""} alt={processed.label} draggable={false}
                onLoad={handleImageLoad} onError={mainRetry.onError} style={{ display: "block" }} />
              {overlayCanvasRef && (
                <canvas ref={overlayCanvasRef}
                  style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%", pointerEvents: "none", display: "none" }} />
              )}
            </div>
            <div style={{ position: "absolute", top: 0, left: 0, width: `${comparePos}%`, height: "100%", overflow: "hidden", zIndex: 2 }}>
              <div style={imgStyle}>
                <img src={origRetry.src ?? ""} alt={original.label} draggable={false} style={{ display: "block" }} />
              </div>
            </div>
            <div className="ab-viewer-compare-line" style={{ left: `${comparePos}%`, zIndex: 3 }}>
              <div className="ab-viewer-compare-handle">
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                  <path d="M2 5H0M10 5H8M2 5L4 3M2 5L4 7M8 5L6 3M8 5L6 7" stroke="#222" strokeWidth="1.5" strokeLinecap="round" />
                </svg>
              </div>
            </div>
            <div className="ab-viewer-compare-label-left" style={{ zIndex: 4 }}>{original.label}</div>
            <div className="ab-viewer-compare-label-right" style={{ zIndex: 4 }}>{processed.label}</div>
          </>
        ) : !mainRetry.error ? (
          <div style={imgStyle}>
            <img src={mainRetry.src ?? ""} alt={activeImage.label} draggable={false}
              onLoad={handleImageLoad} onError={mainRetry.onError} style={{ display: "block" }} />
            {overlayCanvasRef && (
              <canvas ref={overlayCanvasRef}
                style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%", pointerEvents: "none", display: "none" }} />
            )}
          </div>
        ) : null}
      </div>

      {showOverlay && (
        <div className="ab-viewer-statusbar">
          <span className="ab-viewer-status-item">{zoomPct}</span>
          {activeImage.width && activeImage.height && (
            <span className="ab-viewer-status-item">{activeImage.width} x {activeImage.height}</span>
          )}
          {pixelValue && (
            <span className="ab-viewer-status-item">
              ({pixelValue.x}, {pixelValue.y}) = {pixelValue.value.toExponential(4)}
            </span>
          )}
          {activeImage.label && (
            <span className="ab-viewer-status-item ab-viewer-status-label">{activeImage.label}</span>
          )}
        </div>
      )}
    </div>
  );
}

export default memo(AdvancedImageViewer);
