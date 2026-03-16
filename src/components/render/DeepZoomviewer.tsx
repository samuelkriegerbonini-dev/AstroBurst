import { useEffect, useRef, useState, useCallback, memo } from "react";
import { ZoomIn, ZoomOut, Home, Loader2, Maximize2, Grid3X3, AlertCircle } from "lucide-react";
import { generateTiles } from "../../services/tiles.service";
import { useFileContext, useRenderContext } from "../../context/PreviewContext";

interface DeepZoomViewerProps {
  filePath?: string;
  imageWidth: number;
  imageHeight: number;
  tileSize?: number;
  outputDir?: string;
  className?: string;
}

let _convertFileSrc: ((path: string) => string) | null = null;

async function ensureConvertFileSrc(): Promise<(path: string) => string> {
  if (_convertFileSrc) return _convertFileSrc;
  const { convertFileSrc } = await import("@tauri-apps/api/core");
  _convertFileSrc = convertFileSrc;
  return convertFileSrc;
}

function computeMaxLevel(w: number, h: number, ts: number): number {
  let maxDim = Math.max(w, h);
  let level = 0;
  while (maxDim > ts) {
    maxDim = Math.ceil(maxDim / 2);
    level++;
  }
  return level;
}

type ViewerMode = "tiles" | "image";

function DeepZoomViewer({
                          filePath: filePathProp,
                          imageWidth,
                          imageHeight,
                          tileSize = 256,
                          outputDir = "./output/tiles",
                          className = "",
                        }: DeepZoomViewerProps) {
  const { file } = useFileContext();
  const { activeImagePath, renderedPreviewUrl } = useRenderContext();

  const rawPath = activeImagePath || filePathProp || file?.path || "";

  const containerRef = useRef<HTMLDivElement>(null);
  const viewerRef = useRef<any>(null);
  const convertRef = useRef<((path: string) => string) | null>(null);
  const [generating, setGenerating] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [viewerReady, setViewerReady] = useState(false);
  const generatedPathRef = useRef<string | null>(null);
  const renderedUrlRef = useRef<string | null>(null);
  const modeRef = useRef<ViewerMode>("tiles");

  const hasRendered = !!renderedPreviewUrl;
  const effectiveKey = hasRendered ? `rendered:${renderedPreviewUrl}` : `tiles:${rawPath}`;

  const destroyViewer = useCallback(() => {
    if (viewerRef.current) {
      viewerRef.current.destroy();
      viewerRef.current = null;
    }
    setViewerReady(false);
  }, []);

  const runGenerate = useCallback(async () => {
    if (!rawPath || generatedPathRef.current === rawPath) return;
    setGenerating(true);
    setError(null);
    setReady(false);
    setViewerReady(false);

    try {
      const convert = await ensureConvertFileSrc();
      convertRef.current = convert;
      await generateTiles(rawPath, outputDir, tileSize);
      generatedPathRef.current = rawPath;
      modeRef.current = "tiles";
      setReady(true);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setGenerating(false);
    }
  }, [rawPath, outputDir, tileSize, generateTiles]);

  const setupRenderedImage = useCallback(async () => {
    if (!renderedPreviewUrl || renderedUrlRef.current === renderedPreviewUrl) return;
    setGenerating(false);
    setError(null);
    setReady(false);
    setViewerReady(false);

    try {
      const convert = await ensureConvertFileSrc();
      convertRef.current = convert;
      renderedUrlRef.current = renderedPreviewUrl;
      modeRef.current = "image";
      setReady(true);
    } catch (e: any) {
      setError(e?.message || String(e));
    }
  }, [renderedPreviewUrl]);

  useEffect(() => {
    if (hasRendered) {
      if (renderedUrlRef.current !== renderedPreviewUrl) {
        setupRenderedImage();
      }
    } else if (rawPath && rawPath !== generatedPathRef.current) {
      renderedUrlRef.current = null;
      runGenerate();
    }
  }, [hasRendered, renderedPreviewUrl, rawPath, setupRenderedImage, runGenerate]);

  useEffect(() => {
    if (!ready || !containerRef.current) return;
    if (!imageWidth || !imageHeight || imageWidth <= 0 || imageHeight <= 0) return;

    let destroyed = false;

    (async () => {
      let OSD: any;
      try {
        OSD = (await import("openseadragon")).default;
      } catch {
        if (!destroyed) setError("OpenSeadragon not installed. Run: npm install openseadragon");
        return;
      }
      if (destroyed) return;

      destroyViewer();

      let tileSources: any;

      if (modeRef.current === "image" && renderedPreviewUrl) {
        tileSources = {
          type: "image",
          url: renderedPreviewUrl,
          buildPyramid: true,
        };
      } else {
        const convert = convertRef.current!;
        const ts = tileSize;
        const maxLevel = computeMaxLevel(imageWidth, imageHeight, ts);

        tileSources = {
          width: imageWidth,
          height: imageHeight,
          tileSize: ts,
          tileOverlap: 0,
          minLevel: 0,
          maxLevel,
          getTileUrl(level: number, x: number, y: number): string {
            const localPath = `${outputDir}/${level}/${x}_${y}.png`;
            return convert(localPath);
          },
        };
      }

      const viewer = OSD({
        element: containerRef.current,
        prefixUrl: "",
        tileSources,
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
      viewer.addHandler("tile-load-failed", (event: any) => {
        console.warn("[DeepZoom] Tile load failed:", event.tile?.url);
      });

      viewerRef.current = viewer;
    })();

    return () => {
      destroyed = true;
      destroyViewer();
    };
  }, [ready, imageWidth, imageHeight, tileSize, outputDir, renderedPreviewUrl, destroyViewer]);

  useEffect(() => {
    return () => {
      destroyViewer();
      generatedPathRef.current = null;
      renderedUrlRef.current = null;
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
    const v = viewerRef.current;
    if (!v?.viewport || !imageWidth || !imageHeight) return;
    v.viewport.goHome();
  }, [imageWidth, imageHeight]);

  if (error) {
    return (
      <div className={`flex flex-col items-center justify-center gap-3 bg-zinc-950 text-zinc-500 ${className}`}>
        <AlertCircle size={24} className="text-red-400/60" />
        <p className="text-xs text-red-300/80 max-w-[300px] text-center">{error}</p>
        <button
          onClick={() => {
            generatedPathRef.current = null;
            renderedUrlRef.current = null;
            if (hasRendered) setupRenderedImage();
            else runGenerate();
          }}
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

  const modeLabel = modeRef.current === "image" ? "rendered" : "tiled";

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

      {viewerReady && (
        <div className="absolute bottom-3 left-3 z-10
          text-[10px] font-mono text-zinc-600
          bg-zinc-950/70 backdrop-blur-sm rounded px-2 py-1
          border border-zinc-800/30 select-none pointer-events-none"
        >
          {imageWidth}x{imageHeight}
          {modeRef.current === "tiles" && <> | {tileSize}px tiles | {computeMaxLevel(imageWidth, imageHeight, tileSize) + 1} levels</>}
          {modeRef.current === "image" && <> | {modeLabel}</>}
        </div>
      )}

      {!viewerReady && !generating && ready && (
        <div className="absolute inset-0 flex items-center justify-center">
          <Loader2 size={20} className="animate-spin text-zinc-600" />
        </div>
      )}
    </div>
  );
}

export default memo(DeepZoomViewer);
