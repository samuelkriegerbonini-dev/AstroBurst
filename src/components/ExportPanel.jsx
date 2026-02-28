import { useState, useCallback } from "react";
import { Save, FileDown, Loader2, Check } from "lucide-react";

/**
 * ExportPanel — save processed images as FITS files.
 *
 * Props:
 *  - filePath: current file path
 *  - stfParams: current STF params { shadow, midtone, highlight }
 *  - onExport: (path, outputPath, options) => Promise
 *  - onExportRgb: (rPath, gPath, bPath, outputPath, options) => Promise (optional)
 *  - rgbChannels: { r, g, b } paths (optional, for RGB export)
 *  - isLoading: boolean
 *  - lastResult: export result or null
 */
export default function ExportPanel({
  filePath,
  stfParams,
  onExport,
  onExportRgb,
  rgbChannels,
  isLoading = false,
  lastResult = null,
}) {
  const [applyStf, setApplyStf] = useState(false);
  const [copyWcs, setCopyWcs] = useState(true);
  const [copyMetadata, setCopyMetadata] = useState(true);
  const [exportDone, setExportDone] = useState(false);

  const handleExport = useCallback(async () => {
    if (!filePath || !onExport) return;

    const stem = filePath.replace(/\.(fits?|zip)$/i, "");
    const suffix = applyStf ? "_stf" : "_proc";
    const outputPath = `${stem}${suffix}.fits`;

    try {
      await onExport(filePath, outputPath, {
        applyStf,
        shadow: stfParams?.shadow,
        midtone: stfParams?.midtone,
        highlight: stfParams?.highlight,
        copyWcs,
        copyMetadata,
      });
      setExportDone(true);
      setTimeout(() => setExportDone(false), 3000);
    } catch (e) {
      console.error("Export failed:", e);
    }
  }, [filePath, applyStf, stfParams, copyWcs, copyMetadata, onExport]);

  const handleExportRgb = useCallback(async () => {
    if (!rgbChannels || !onExportRgb) return;
    const outputPath = "./output/rgb_composite.fits";
    try {
      await onExportRgb(rgbChannels.r, rgbChannels.g, rgbChannels.b, outputPath, {
        copyWcs,
        copyMetadata,
      });
      setExportDone(true);
      setTimeout(() => setExportDone(false), 3000);
    } catch (e) {
      console.error("RGB FITS export failed:", e);
    }
  }, [rgbChannels, copyWcs, copyMetadata, onExportRgb]);

  const hasRgb = rgbChannels && (rgbChannels.r || rgbChannels.g || rgbChannels.b);

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-zinc-800/50">
        <Save size={12} className="text-amber-400" />
        <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
          Export FITS
        </span>
      </div>

      <div className="px-3 py-2 space-y-2">
        {/* Options */}
        <div className="space-y-1">
          <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
            <input
              type="checkbox"
              checked={applyStf}
              onChange={(e) => setApplyStf(e.target.checked)}
              className="w-3 h-3 accent-amber-500"
            />
            Apply current STF stretch
          </label>
          <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
            <input
              type="checkbox"
              checked={copyWcs}
              onChange={(e) => setCopyWcs(e.target.checked)}
              className="w-3 h-3 accent-amber-500"
            />
            Copy WCS (coordinates)
          </label>
          <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
            <input
              type="checkbox"
              checked={copyMetadata}
              onChange={(e) => setCopyMetadata(e.target.checked)}
              className="w-3 h-3 accent-amber-500"
            />
            Copy observation metadata
          </label>
        </div>

        {/* Export 2D FITS */}
        <button
          onClick={handleExport}
          disabled={!filePath || isLoading}
          className="w-full flex items-center justify-center gap-2 bg-amber-600/20 hover:bg-amber-600/30 text-amber-300 border border-amber-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
        >
          {isLoading ? (
            <>
              <Loader2 size={12} className="animate-spin" />
              Exporting...
            </>
          ) : exportDone ? (
            <>
              <Check size={12} />
              Saved!
            </>
          ) : (
            <>
              <FileDown size={12} />
              Export as FITS
            </>
          )}
        </button>

        {/* Export RGB FITS (only if RGB channels available) */}
        {hasRgb && (
          <button
            onClick={handleExportRgb}
            disabled={isLoading}
            className="w-full flex items-center justify-center gap-2 bg-pink-600/15 hover:bg-pink-600/25 text-pink-300 border border-pink-600/25 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
          >
            <FileDown size={12} />
            Export RGB as FITS cube
          </button>
        )}

        {/* Last result */}
        {lastResult && (
          <div className="text-[10px] space-y-0.5 text-zinc-500">
            <div className="flex justify-between">
              <span>Output:</span>
              <span className="text-zinc-300 font-mono truncate max-w-[160px]">
                {lastResult.output_path?.split("/").pop()}
              </span>
            </div>
            <div className="flex justify-between">
              <span>Size:</span>
              <span className="text-zinc-300 font-mono">
                {(lastResult.file_size_bytes / 1024).toFixed(0)} KB
              </span>
            </div>
            <div className="flex justify-between">
              <span>Time:</span>
              <span className="text-zinc-300 font-mono">{lastResult.elapsed_ms} ms</span>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
