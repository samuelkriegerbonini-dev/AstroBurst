import { useState, useCallback, lazy, Suspense, memo } from "react";
import { Download, Loader2, Box, Film } from "lucide-react";
import { exportFits, exportFitsRgb } from "../../services/export.service";
import { getCubeFrame } from "../../services/cube.service";
import { useFileContext, useHistContext, useRgbContext, useRenderContext, useCubeContext } from "../../context/PreviewContext";
import { getOutputDir } from "../../infrastructure/tauri";

const ExportPanel = lazy(() => import("./ExportPanel"));

function ExportTabInner() {
  const { file } = useFileContext();
  const { stfParams } = useHistContext();
  const { rgbChannels } = useRgbContext();
  const { renderedPreviewUrl } = useRenderContext();
  const { isCube, cubeDims } = useCubeContext();

  const [exportResult, setExportResult] = useState<any>(null);
  const [exportLoading, setExportLoading] = useState(false);
  const [cubeExporting, setCubeExporting] = useState(false);
  const [cubeExportProgress, setCubeExportProgress] = useState(0);
  const [cubeExportResult, setCubeExportResult] = useState<any>(null);
  const [cubeExportFits, setCubeExportFits] = useState(false);

  const handleExportFits = useCallback(
    async (path: string, outputPath: string, options: any) => {
      setExportLoading(true);
      try {
        const result = await exportFits(path, outputPath, options);
        setExportResult(result);
      } catch (e) {
        console.error("FITS export failed:", e);
      } finally {
        setExportLoading(false);
      }
    },
    [exportFits],
  );

  const handleExportFitsRgb = useCallback(
    async (
      rPath: string | null,
      gPath: string | null,
      bPath: string | null,
      outputPath: string,
      options: any,
    ) => {
      setExportLoading(true);
      try {
        const result = await exportFitsRgb(rPath, gPath, bPath, outputPath, {
          copyWcs: options?.copyWcs ?? true,
          copyMetadata: options?.copyMetadata ?? true,
        });
        setExportResult(result);
      } catch (e) {
        console.error("RGB FITS export failed:", e);
      } finally {
        setExportLoading(false);
      }
    },
    [exportFitsRgb],
  );

  const handleDownloadPng = useCallback(() => {
    const url = renderedPreviewUrl || file?.result?.previewUrl;
    if (!url) return;
    const a = document.createElement("a");
    a.href = url;
    a.download = (file?.name || "image").replace(/\.(fits?|zip)$/i, ".png");
    a.click();
  }, [file, renderedPreviewUrl]);

  const handleExportCubeFrames = useCallback(async () => {
    if (!file?.path || !cubeDims) return;
    const totalFrames = cubeDims.naxis3 ?? cubeDims.frames ?? 0;
    if (totalFrames <= 0) return;

    setCubeExporting(true);
    setCubeExportProgress(0);
    setCubeExportResult(null);

    const stem = (file.name || "cube").replace(/\.(fits?|asdf|zip)$/i, "");
    const dir = await getOutputDir();
    let exported = 0;

    try {
      for (let i = 0; i < totalFrames; i++) {
        const pad = String(i).padStart(4, "0");
        const outputPath = `${dir}/${stem}_frame_${pad}.png`;
        const fitsPath = cubeExportFits ? `${dir}/${stem}_frame_${pad}.fits` : undefined;
        await getCubeFrame(file.path, i, outputPath, fitsPath);
        exported++;
        setCubeExportProgress(Math.round((exported / totalFrames) * 100));
      }
      setCubeExportResult({ exported, total: totalFrames, fits: cubeExportFits });
    } catch (e) {
      console.error("Cube frame export failed:", e);
      setCubeExportResult({ exported, total: totalFrames, error: String(e) });
    } finally {
      setCubeExporting(false);
    }
  }, [file?.path, file?.name, cubeDims, getCubeFrame, cubeExportFits]);

  const totalFrames = cubeDims ? (cubeDims.naxis3 ?? cubeDims.frames ?? 0) : 0;

  return (
    <Suspense
      fallback={
        <div className="flex items-center justify-center py-12">
          <Loader2 size={20} className="animate-spin text-zinc-500" />
        </div>
      }
    >
      <div className="flex flex-col gap-4">
        <ExportPanel
          filePath={file?.path}
          stfParams={stfParams}
          onExport={handleExportFits}
          onExportRgb={handleExportFitsRgb}
          rgbChannels={rgbChannels}
          isLoading={exportLoading}
          lastResult={exportResult}
        />

        {isCube && totalFrames > 1 && (
          <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
            <div className="flex items-center gap-2 px-3 py-2 border-b border-zinc-800/50">
              <Box size={12} className="text-purple-400" />
              <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
                Cube Export
              </span>
              <span className="text-[10px] font-mono text-zinc-600 ml-auto">
                {totalFrames} frames
              </span>
            </div>
            <div className="px-3 py-2 space-y-2">
              <p className="text-[10px] text-zinc-500">
                Export all cube frames as individual images.
              </p>
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={cubeExportFits}
                  onChange={(e) => setCubeExportFits(e.target.checked)}
                  disabled={cubeExporting}
                  className="accent-purple-500"
                />
                <span className="text-[11px] text-zinc-300">Also export as FITS</span>
              </label>
              <button
                onClick={handleExportCubeFrames}
                disabled={cubeExporting}
                className="w-full flex items-center justify-center gap-2 bg-purple-600/20 hover:bg-purple-600/30 text-purple-300 border border-purple-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
              >
                {cubeExporting ? (
                  <>
                    <Loader2 size={12} className="animate-spin" />
                    Exporting {cubeExportProgress}%
                  </>
                ) : (
                  <>
                    <Film size={12} />
                    Export All Frames{cubeExportFits ? " (PNG + FITS)" : " (PNG)"}
                  </>
                )}
              </button>

              {cubeExporting && (
                <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-purple-500 rounded-full transition-all duration-300"
                    style={{ width: `${cubeExportProgress}%` }}
                  />
                </div>
              )}

              {cubeExportResult && (
                <div className={`text-[10px] font-mono px-2 py-1 rounded ${
                  cubeExportResult.error ? "text-amber-300 bg-amber-900/20" : "text-emerald-300 bg-emerald-900/20"
                }`}>
                  {cubeExportResult.exported}/{cubeExportResult.total} frames exported
                  {cubeExportResult.fits && " (PNG + FITS)"}
                  {cubeExportResult.error && ` (stopped: ${cubeExportResult.error})`}
                </div>
              )}
            </div>
          </div>
        )}

        <button
          onClick={handleDownloadPng}
          className="flex items-center justify-center gap-2 bg-zinc-800 hover:bg-zinc-700 text-zinc-200 rounded-lg px-4 py-2.5 font-medium transition-colors text-sm w-full"
        >
          <Download size={16} />
          Download PNG
        </button>
      </div>
    </Suspense>
  );
}

export default memo(ExportTabInner);
