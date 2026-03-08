import { useState, useCallback, useEffect, memo } from "react";
import { Grid3X3, X, Maximize2 } from "lucide-react";
import DeepZoomViewer from "../render/DeepZoomviewer";

interface TileViewerPanelProps {
  filePath: string | null;
  imageWidth?: number;
  imageHeight?: number;
}

function TileViewerPanelInner({ filePath, imageWidth, imageHeight }: TileViewerPanelProps) {
  const [tileSize, setTileSize] = useState(256);
  const [isOpen, setIsOpen] = useState(false);

  const isLargeImage = (imageWidth || 0) > 4096 || (imageHeight || 0) > 4096;

  const handleOpen = useCallback(() => {
    if (!filePath) return;
    setIsOpen(true);
  }, [filePath]);

  const handleClose = useCallback(() => {
    setIsOpen(false);
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setIsOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen]);

  if (!filePath || !isLargeImage) return null;

  return (
    <>
      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4">
        <div className="flex items-center justify-between mb-2">
          <h4 className="text-xs font-semibold text-rose-400 uppercase tracking-wider flex items-center gap-1.5">
            <Grid3X3 size={12} />
            Deep Zoom
          </h4>
          <span className="text-[10px] font-mono text-zinc-600">
            {imageWidth}x{imageHeight}
          </span>
        </div>
        <p className="text-[10px] text-zinc-500 mb-3">
          Pan and zoom at full resolution on large images.
        </p>

        <div className="mb-3">
          <div className="flex items-center justify-between mb-1">
            <label className="text-[10px] text-zinc-400">Tile Size</label>
            <span className="text-[10px] font-mono text-zinc-500">{tileSize}px</span>
          </div>
          <input
            type="range"
            min={128}
            max={512}
            step={64}
            value={tileSize}
            onChange={(e) => setTileSize(parseInt(e.target.value))}
            className="w-full accent-rose-500"
          />
        </div>

        <button
          onClick={handleOpen}
          className="flex items-center justify-center gap-2 w-full rounded-md px-3 py-2 text-xs font-medium transition-all"
          style={{
            background: "rgba(244,63,94,0.12)",
            color: "#fb7185",
            border: "1px solid rgba(244,63,94,0.2)",
          }}
        >
          <Maximize2 size={12} />
          Open Deep Zoom
        </button>
      </div>

      {isOpen && filePath && (
        <div className="fixed inset-0 z-50 bg-zinc-950">
          <DeepZoomViewer
            filePath={filePath}
            imageWidth={imageWidth || 0}
            imageHeight={imageHeight || 0}
            tileSize={tileSize}
            className="w-full h-full"
          />

          <button
            onClick={handleClose}
            className="absolute top-4 left-4 z-50
              w-9 h-9 flex items-center justify-center rounded-lg
              bg-zinc-900/80 backdrop-blur-sm border border-zinc-700/50
              text-zinc-400 hover:text-white hover:bg-zinc-800/90
              transition-all duration-150 active:scale-95"
            title="Close viewer"
          >
            <X size={16} strokeWidth={2} />
          </button>

          <div className="absolute top-4 left-16 z-50
            text-[11px] text-zinc-500
            bg-zinc-900/70 backdrop-blur-sm rounded-md px-3 py-1.5
            border border-zinc-800/30 select-none pointer-events-none"
          >
            Scroll to zoom | Double-click to zoom in | Drag to pan
          </div>
        </div>
      )}
    </>
  );
}

export default memo(TileViewerPanelInner);
