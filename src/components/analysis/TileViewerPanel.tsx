import { useState, useCallback, useEffect, memo } from "react";
import { Grid3X3, X, Maximize2 } from "lucide-react";
import { Slider } from "../ui";
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
      <div className="ab-panel overflow-hidden">
        <div className="ab-panel-header">
          <div className="flex items-center gap-1.5">
            <Grid3X3 size={12} style={{ color: "var(--ab-rose)" }} />
            <span className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">
              Deep Zoom
            </span>
          </div>
          <span className="text-[10px] font-mono text-zinc-600">
            {imageWidth}\u00d7{imageHeight}
          </span>
        </div>

        <div className="px-3 py-3 flex flex-col gap-3">
          <p className="text-[10px] text-zinc-500">
            Pan and zoom at full resolution on large images.
          </p>

          <Slider
            label="Tile Size"
            value={tileSize}
            min={128}
            max={512}
            step={64}
            accent="teal"
            format={(v) => `${v}px`}
            onChange={setTileSize}
          />

          <button
            onClick={handleOpen}
            className="ab-run-btn"
            data-accent="teal"
            style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8 }}
          >
            <Maximize2 size={13} />
            Open Deep Zoom
          </button>
        </div>
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
            className="absolute top-4 left-4 z-50 w-9 h-9 flex items-center justify-center rounded-lg text-zinc-400 hover:text-white transition-all duration-150 active:scale-95"
            style={{
              background: "rgba(24,24,32,0.8)",
              backdropFilter: "blur(8px)",
              border: "1px solid rgba(63,63,70,0.4)",
            }}
            title="Close viewer (Esc)"
          >
            <X size={16} strokeWidth={2} />
          </button>

          <div
            className="absolute top-4 left-16 z-50 text-[11px] text-zinc-500 rounded-md px-3 py-1.5 select-none pointer-events-none"
            style={{
              background: "rgba(24,24,32,0.7)",
              backdropFilter: "blur(8px)",
              border: "1px solid rgba(63,63,70,0.2)",
            }}
          >
            Scroll to zoom | Double-click to zoom in | Drag to pan
          </div>
        </div>
      )}
    </>
  );
}

export default memo(TileViewerPanelInner);
