import { useEffect, useRef, useState, useCallback, memo } from "react";
import type { OsdViewer } from "openseadragon";
import { ZoomIn, ZoomOut, Home, Loader2, Maximize2, Grid3X3, AlertCircle } from "lucide-react";
import { generateTiles, generateTilesRgb } from "../../services/tiles";
import { getOutputDirTiles, getPreviewUrl } from "../../infrastructure/tauri";
import { computeMaxLevel, createTileSlotPool, readTilePyramid, tileSlotDir, tileUrl } from "../../utils/deepZoomTiles";

interface DeepZoomViewerProps {
  filePath: string | null;
  composite: boolean;
  rgbPath?: string | null;
  imageWidth: number;
  imageHeight: number;
  tileSize?: number;
  sourceLabel?: string | null;
  className?: string;
}

interface TileSet {
  dirUrl: string;
  width: number;
  height: number;
  maxLevel: number;
  version: number;
}

let tileGeneration = Date.now();
const tileSlots = createTileSlotPool();

function DeepZoomViewer({
                          filePath,
                          composite,
                          rgbPath = null,
                          imageWidth,
                          imageHeight,
                          tileSize = 256,
                          sourceLabel = null,
                          className = "",
                        }: DeepZoomViewerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewerRef = useRef<OsdViewer | null>(null);
  const [generating, setGenerating] = useState(false);
  const [tiles, setTiles] = useState<TileSet | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [viewerReady, setViewerReady] = useState(false);
  const [retry, setRetry] = useState(0);

  const sourceKey = composite ? (rgbPath ? `rgb:${rgbPath}` : "composite") : filePath ? `file:${filePath}` : null;
  const genKey = sourceKey ? `${sourceKey}|${tileSize}|${retry}` : null;

  const destroyViewer = useCallback(() => {
    if (viewerRef.current) {
      viewerRef.current.destroy();
      viewerRef.current = null;
    }
    setViewerReady(false);
  }, []);

  useEffect(() => {
    let cancelled = false;
    setTiles(null);
    setError(null);
    if (!genKey) {
      setGenerating(false);
      return;
    }
    setGenerating(true);
    const slot = tileSlots.acquire();
    tileSlots.hold(slot);
    (async () => {
      try {
        const requestedDir = tileSlotDir(await getOutputDirTiles(), slot);
        const result = composite
          ? await generateTilesRgb(requestedDir, tileSize, rgbPath)
          : await generateTiles(filePath as string, requestedDir, tileSize);
        if (cancelled) return;
        const info = readTilePyramid(result, requestedDir, imageWidth, imageHeight);
        const dirUrl = await getPreviewUrl(info.dir);
        if (cancelled) return;
        tileGeneration += 1;
        setTiles({
          dirUrl,
          width: info.width,
          height: info.height,
          maxLevel: info.levelCount !== null ? info.levelCount - 1 : computeMaxLevel(info.width, info.height, tileSize),
          version: tileGeneration,
        });
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      } finally {
        tileSlots.release(slot);
        if (!cancelled) setGenerating(false);
      }
    })();
    return () => {
      cancelled = true;
      tileSlots.release(slot);
    };
  }, [genKey, composite, rgbPath, filePath, tileSize, imageWidth, imageHeight]);

  useEffect(() => {
    if (!tiles || !containerRef.current) return;
    if (tiles.width <= 0 || tiles.height <= 0) return;

    let destroyed = false;

    (async () => {
      let OSD: (options: Record<string, unknown>) => OsdViewer;
      try {
        OSD = (await import("openseadragon")).default;
      } catch {
        if (!destroyed) setError("OpenSeadragon not installed. Run: npm install openseadragon");
        return;
      }
      if (destroyed) return;

      destroyViewer();

      const { dirUrl, version } = tiles;
      const viewer = OSD({
        element: containerRef.current,
        prefixUrl: "",
        tileSources: {
          width: tiles.width,
          height: tiles.height,
          tileSize,
          tileOverlap: 0,
          minLevel: 0,
          maxLevel: tiles.maxLevel,
          getTileUrl(level: number, x: number, y: number): string {
            return tileUrl(dirUrl, level, x, y, version);
          },
        },
        showNavigationControl: false,
        showNavigator: false,
        showZoomControl: false,
        showHomeControl: false,
        showFullPageControl: false,
        showSequenceControl: false,
        animationTime: 0.3,
        blendTime: 0.15,
        springStiffness: 12,
        visibilityRatio: 0.8,
        constrainDuringPan: true,
        minZoomLevel: 0.5,
        maxZoomPixelRatio: 4,
        gestureSettingsMouse: {
          clickToZoom: false,
          dblClickToZoom: true,
          scrollToZoom: true,
        },
        gestureSettingsTouch: {
          pinchToZoom: true,
          dblClickToZoom: true,
        },
        placeholderFillStyle: "#09090b",
        timeout: 30000,
        immediateRender: true,
      });

      viewer.addHandler("open", () => { if (!destroyed) setViewerReady(true); });
      viewer.addHandler("tile-load-failed", (event: { tile?: { url?: string } }) => {
        console.warn("[DeepZoom] Tile load failed:", event.tile?.url);
      });

      viewerRef.current = viewer;
    })();

    return () => {
      destroyed = true;
      destroyViewer();
    };
  }, [tiles, tileSize, destroyViewer]);

  useEffect(() => {
    return () => {
      destroyViewer();
    };
  }, [destroyViewer]);

  const handleZoomIn = useCallback(() => {
    viewerRef.current?.viewport?.zoomBy(1.5);
  }, []);

  const handleZoomOut = useCallback(() => {
    viewerRef.current?.viewport?.zoomBy(0.67);
  }, []);

  const handleHome = useCallback(() => {
    viewerRef.current?.viewport?.goHome();
  }, []);

  const handleFullExtent = useCallback(() => {
    viewerRef.current?.viewport?.goHome();
  }, []);

  if (error) {
    return (
      <div className={`flex flex-col items-center justify-center gap-3 bg-zinc-950 text-zinc-500 ${className}`}>
        <AlertCircle size={24} className="text-red-400/60" />
        <p className="text-xs text-red-300/80 max-w-[300px] text-center">{error}</p>
        <button
          onClick={() => setRetry((r) => r + 1)}
          className="text-[10px] text-cyan-400 hover:text-cyan-300 transition-colors"
        >
          Retry
        </button>
      </div>
    );
  }

  if (generating) {
    return (
      <div className={`flex flex-col items-center justify-center gap-4 bg-zinc-950 ${className}`}>
        <div className="relative">
          <Grid3X3 size={32} className="text-zinc-700" />
          <Loader2 size={16} className="absolute -bottom-1 -right-1 animate-spin text-cyan-400" />
        </div>
        <div className="text-center">
          <p className="text-xs text-zinc-400 font-medium">Generating tile pyramid</p>
          <p className="text-[10px] text-zinc-600 mt-1">
            {imageWidth}x{imageHeight} @ {tileSize}px tiles
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className={`relative bg-zinc-950 ${className}`}>
      <div ref={containerRef} className="absolute inset-0" style={{ background: "#09090b" }} />

      {viewerReady && (
        <div className="absolute top-3 right-3 flex flex-col gap-1.5 z-10">
          {[
            { icon: ZoomIn, action: handleZoomIn, title: "Zoom in" },
            { icon: ZoomOut, action: handleZoomOut, title: "Zoom out" },
            { icon: Home, action: handleHome, title: "Reset view" },
            { icon: Maximize2, action: handleFullExtent, title: "Fit to view" },
          ].map(({ icon: Icon, action, title }) => (
            <button
              key={title}
              onClick={action}
              title={title}
              className="w-8 h-8 flex items-center justify-center rounded-md
                bg-zinc-900/80 backdrop-blur-sm border border-zinc-700/50
                text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800/90 hover:border-zinc-600/50
                transition-all duration-150 active:scale-95"
            >
              <Icon size={14} strokeWidth={1.8} />
            </button>
          ))}
        </div>
      )}

      {viewerReady && tiles && (
        <div className="absolute bottom-3 left-3 z-10
          text-[10px] font-mono text-zinc-600
          bg-zinc-950/70 backdrop-blur-sm rounded px-2 py-1
          border border-zinc-800/30 select-none pointer-events-none"
        >
          {tiles.width}x{tiles.height} | {tileSize}px tiles | {tiles.maxLevel + 1} levels | auto STF, display settings not applied
          {sourceLabel && <> | {sourceLabel}</>}
        </div>
      )}

      {!viewerReady && !generating && tiles && (
        <div className="absolute inset-0 flex items-center justify-center">
          <Loader2 size={20} className="animate-spin text-zinc-600" />
        </div>
      )}
    </div>
  );
}

export default memo(DeepZoomViewer);
