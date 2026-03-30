import {
  useState,
  useCallback,
  useRef,
  useEffect,
  useMemo,
  memo,
} from "react";
import {
  ZoomIn,
  ZoomOut,
  Maximize,
  Square,
  SquareSplitHorizontal,
  Move,
  RotateCcw,
  Crosshair,
  Eye,
  EyeOff,
  Loader2,
  ImageOff,
  RefreshCw,
} from "lucide-react";

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
  overlayCanvasRef?: React.RefObject<HTMLCanvasElement>;
  className?: string;
}

interface Transform {
  scale: number;
  x: number;
  y: number;
}

const ZOOM_MIN = 0.1;
const ZOOM_MAX = 32;
const ZOOM_STEP = 1.15;
const ZOOM_PRESETS = [0.25, 0.5, 1, 2, 4, 8];
const MAX_RETRIES = 3;
const RETRY_DELAYS = [200, 600, 1500] as const;

function clampTransform(t: Transform): Transform {
  return { scale: Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, t.scale)), x: t.x, y: t.y };
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
  const [transform, setTransform] = useState<Transform>({ scale: 1, x: 0, y: 0 });
  const transformRef = useRef(transform);
  transformRef.current = transform;
  const [compareMode, setCompareMode] = useState(false);
  const [comparePos, setComparePos] = useState(50);
  const [showOverlay, setShowOverlay] = useState(true);
  const [cursorMode, setCursorMode] = useState<"pan" | "crosshair">("pan");
  const [isPanning, setIsPanning] = useState(false);
  const isPanningRef = useRef(false);
  const [imgNatural, setImgNatural] = useState<{ w: number; h: number } | null>(null);
  const panStart = useRef({ x: 0, y: 0, tx: 0, ty: 0 });
  const compareDragging = useRef(false);

  const [imgLoading, setImgLoading] = useState(false);
  const [imgError, setImgError] = useState(false);
  const [retryCount, setRetryCount] = useState(0);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const activeImage = processed ?? original;
  const hasComparison = !!original && !!processed;

  const renderW = imgNatural?.w ?? 0;
  const renderH = imgNatural?.h ?? 0;
  const hasRenderDims = renderW > 0 && renderH > 0;

  const fitToWindowRef = useRef<() => void>(() => {});

  const imgSrc = useMemo(() => {
    if (!activeImage?.url) return null;
    if (retryCount === 0) return activeImage.url;
    const sep = activeImage.url.includes("?") ? "&" : "?";
    return `${activeImage.url}${sep}_retry=${retryCount}&t=${Date.now()}`;
  }, [activeImage?.url, retryCount]);

  const origSrc = useMemo(() => {
    if (!original?.url) return null;
    if (retryCount === 0) return original.url;
    const sep = original.url.includes("?") ? "&" : "?";
    return `${original.url}${sep}_retry=${retryCount}&t=${Date.now()}`;
  }, [original?.url, retryCount]);

  const procSrc = useMemo(() => {
    if (!processed?.url) return null;
    if (retryCount === 0) return processed.url;
    const sep = processed.url.includes("?") ? "&" : "?";
    return `${processed.url}${sep}_retry=${retryCount}&t=${Date.now()}`;
  }, [processed?.url, retryCount]);

  useEffect(() => {
    if (activeImage?.url) {
      setImgNatural(null);
      setImgLoading(true);
      setImgError(false);
      setRetryCount(0);
      if (retryTimerRef.current) {
        clearTimeout(retryTimerRef.current);
        retryTimerRef.current = null;
      }
    }
  }, [activeImage?.url]);

  useEffect(() => {
    return () => {
      if (retryTimerRef.current) clearTimeout(retryTimerRef.current);
    };
  }, []);

  const fitToWindow = useCallback(() => {
    const container = containerRef.current;
    if (!container || !hasRenderDims) return;
    const cw = container.clientWidth;
    const ch = container.clientHeight;
    if (cw === 0 || ch === 0) return;
    const scale = Math.min(cw / renderW, ch / renderH, 1);
    setTransform({
      scale,
      x: (cw - renderW * scale) / 2,
      y: (ch - renderH * scale) / 2,
    });
  }, [renderW, renderH, hasRenderDims]);

  fitToWindowRef.current = fitToWindow;

  const zoomTo = useCallback((newScale: number, centerX?: number, centerY?: number) => {
    setTransform((prev) => {
      const container = containerRef.current;
      if (!container) return { ...prev, scale: Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, newScale)) };
      const rect = container.getBoundingClientRect();
      const cx = centerX ?? rect.width / 2;
      const cy = centerY ?? rect.height / 2;
      const ratio = newScale / prev.scale;
      return clampTransform({
        scale: newScale,
        x: cx - (cx - prev.x) * ratio,
        y: cy - (cy - prev.y) * ratio,
      });
    });
  }, []);

  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      e.preventDefault();
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;
      const factor = e.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP;
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      setTransform((prev) => {
        const newScale = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, prev.scale * factor));
        const ratio = newScale / prev.scale;
        return clampTransform(
          { scale: newScale, x: cx - (cx - prev.x) * ratio, y: cy - (cy - prev.y) * ratio },
        );
      });
    },
    [],
  );

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
    [compareMode, comparePos, cursorMode],
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
        const t = transformRef.current;
        const imgX = (e.clientX - rect.left - t.x) / t.scale;
        const imgY = (e.clientY - rect.top - t.y) / t.scale;
        if (imgX >= 0 && imgX < renderW && imgY >= 0 && imgY < renderH) {
          const fitsW = activeImage?.width ?? renderW;
          const fitsH = activeImage?.height ?? renderH;
          const fx = Math.floor((imgX / renderW) * fitsW);
          const fy = Math.floor((imgY / renderH) * fitsH);
          onMousePixel(fx, fy);
        }
      }
    },
    [cursorMode, onMousePixel, activeImage, renderW, renderH, hasRenderDims],
  );

  const handlePointerUp = useCallback(() => {
    setIsPanning(false);
    isPanningRef.current = false;
    compareDragging.current = false;
  }, []);

  const resetView = useCallback(() => {
    fitToWindow();
  }, [fitToWindow]);

  const setOneToOne = useCallback(() => {
    const container = containerRef.current;
    if (!container || !hasRenderDims) return;
    setTransform({
      scale: 1,
      x: (container.clientWidth - renderW) / 2,
      y: (container.clientHeight - renderH) / 2,
    });
  }, [renderW, renderH, hasRenderDims]);

  const handleImageLoad = useCallback((e: React.SyntheticEvent<HTMLImageElement>) => {
    const img = e.currentTarget;
    const nw = img.naturalWidth;
    const nh = img.naturalHeight;
    if (nw > 0 && nh > 0) {
      setImgNatural({ w: nw, h: nh });
      setImgLoading(false);
      setImgError(false);
    }
  }, []);

  const handleImageError = useCallback(() => {
    if (retryTimerRef.current) return;
    if (retryCount < MAX_RETRIES) {
      const delay = RETRY_DELAYS[retryCount] ?? 1500;
      retryTimerRef.current = setTimeout(() => {
        retryTimerRef.current = null;
        setRetryCount((c) => c + 1);
      }, delay);
    } else {
      setImgLoading(false);
      setImgError(true);
    }
  }, [retryCount]);

  const handleRetryClick = useCallback(() => {
    setRetryCount(0);
    setImgError(false);
    setImgLoading(true);
    requestAnimationFrame(() => setRetryCount(1));
  }, []);

  useEffect(() => {
    if (hasRenderDims) {
      requestAnimationFrame(() => fitToWindowRef.current());
    }
  }, [hasRenderDims, renderW, renderH]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => fitToWindowRef.current());
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const zoomPct = useMemo(() => `${Math.round(transform.scale * 100)}%`, [transform.scale]);

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
          <button onClick={() => zoomTo(transform.scale * ZOOM_STEP)} className="ab-viewer-btn" title="Zoom In">
            <ZoomIn size={14} />
          </button>
          <button onClick={() => zoomTo(transform.scale / ZOOM_STEP)} className="ab-viewer-btn" title="Zoom Out">
            <ZoomOut size={14} />
          </button>
          <button onClick={fitToWindow} className="ab-viewer-btn" title="Fit to Window">
            <Maximize size={14} />
          </button>
          <button onClick={setOneToOne} className="ab-viewer-btn" title="1:1 Pixel">
            <Square size={13} />
          </button>
          <button onClick={resetView} className="ab-viewer-btn" title="Reset View">
            <RotateCcw size={13} />
          </button>
        </div>

        <div className="ab-viewer-toolbar-divider" />

        <div className="ab-viewer-toolbar-group">
          <button
            onClick={() => setCursorMode((m) => (m === "pan" ? "crosshair" : "pan"))}
            className={`ab-viewer-btn ${cursorMode === "crosshair" ? "ab-viewer-btn-active" : ""}`}
            title={cursorMode === "crosshair" ? "Switch to Pan (drag to move)" : "Switch to Crosshair (inspect pixels)"}
          >
            {cursorMode === "crosshair" ? <Crosshair size={14} /> : <Move size={14} />}
          </button>
          {hasComparison && (
            <button
              onClick={() => setCompareMode((v) => !v)}
              className={`ab-viewer-btn ${compareMode ? "ab-viewer-btn-active" : ""}`}
              title="Before / After comparison"
            >
              <SquareSplitHorizontal size={14} />
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
        onWheel={handleWheel}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerLeave={() => {
          handlePointerUp();
          onMouseLeave?.();
        }}
        style={{ cursor: isPanning ? "grabbing" : cursorMode === "pan" ? "grab" : "crosshair" }}
      >
        {imgLoading && !hasRenderDims && !imgError && (
          <div className="ab-viewer-loading-overlay">
            <Loader2 size={24} className="animate-spin" style={{ color: "var(--ab-teal)", opacity: 0.5 }} />
          </div>
        )}

        {imgError && (
          <div className="ab-viewer-error-overlay">
            <ImageOff size={28} strokeWidth={1.5} className="text-zinc-600" />
            <span className="text-xs text-zinc-500 mt-2">Preview failed to load</span>
            <button onClick={handleRetryClick} className="ab-viewer-retry-btn">
              <RefreshCw size={12} />
              Retry
            </button>
          </div>
        )}

        {!imgError && compareMode && original && processed ? (
          <>
            <div style={{ ...imgStyle, zIndex: 1 }}>
              <img
                src={procSrc ?? ""}
                alt={processed.label}
                draggable={false}
                onLoad={handleImageLoad}
                onError={handleImageError}
                style={{ display: "block" }}
              />
              {overlayCanvasRef && (
                <canvas
                  ref={overlayCanvasRef}
                  style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%", pointerEvents: "none", display: "none" }}
                />
              )}
            </div>
            <div
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: `${comparePos}%`,
                height: "100%",
                overflow: "hidden",
                zIndex: 2,
              }}
            >
              <div style={imgStyle}>
                <img
                  src={origSrc ?? ""}
                  alt={original.label}
                  draggable={false}
                  style={{ display: "block" }}
                />
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
        ) : !imgError ? (
          <div style={imgStyle}>
            <img
              src={imgSrc ?? ""}
              alt={activeImage.label}
              draggable={false}
              onLoad={handleImageLoad}
              onError={handleImageError}
              style={{ display: "block" }}
            />
            {overlayCanvasRef && (
              <canvas
                ref={overlayCanvasRef}
                style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%", pointerEvents: "none", display: "none" }}
              />
            )}
          </div>
        ) : null}
      </div>

      {showOverlay && (
        <div className="ab-viewer-statusbar">
          <span className="ab-viewer-status-item">{zoomPct}</span>
          {activeImage.width && activeImage.height && (
            <span className="ab-viewer-status-item">
              {activeImage.width} x {activeImage.height}
            </span>
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
