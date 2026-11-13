import { useState, useCallback, useMemo, useRef, useLayoutEffect, lazy, Suspense, memo } from "react";
import { Loader2, Box, Film, X } from "lucide-react";
import { exportFits, exportFitsRgb } from "../../services/export";
import type { ExportResult, ExportFitsOptions, ExportFitsRgbOptions } from "../../services/export";
import { getCubeFrame } from "../../services/cube";
import { useFileContext, useHistContext, useRgbContext, useCubeContext, useDisplayedImage, useRenderContext, useDisplayContext } from "../../context/PreviewContext";
import { useCompositePreview, useCompositeStf } from "../../context/CompositeContext";
import { getExportDir } from "../../infrastructure/tauri";
import { cubeFrameExportName, exportSourceLabel, exportTimestamp, resolveRgbExportSource } from "../../utils/exportSources";
import { RGB_PLANES, rgbProductOf } from "../../utils/rgbProduct";

const ExportPanel = lazy(() => import("./ExportPanel"));

interface CubeExportResult {
  path: string;
  exported: number;
  total: number;
  fits?: boolean;
  dir?: string;
  stopped?: boolean;
  error?: string;
}

function ExportTabInner() {
  const { file } = useFileContext();
  const { stfParams, histData } = useHistContext();
  const { display } = useDisplayContext();
  const { rgbChannels } = useRgbContext();
  const { compositePreviewUrl } = useCompositePreview();
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeStf();
  const displayed = useDisplayedImage();
  const { processed } = useRenderContext();
  const { isCube, cubeDims } = useCubeContext();

  const [exportResult, setExportResult] = useState<ExportResult | null>(null);
  const [exportLoading, setExportLoading] = useState(false);
  const [cubeExporting, setCubeExporting] = useState(false);
  const [cubeExportProgress, setCubeExportProgress] = useState(0);
  const [cubeExportResult, setCubeExportResult] = useState<CubeExportResult | null>(null);
  const [cubeExportFits, setCubeExportFits] = useState(false);
  const [cubeStopping, setCubeStopping] = useState(false);
  const cubeCancelRef = useRef(false);
  const exportFramesButtonRef = useRef<HTMLButtonElement>(null);
  const stopButtonRef = useRef<HTMLButtonElement>(null);
  const refocusExportFramesRef = useRef(false);

  const handleExportFits = useCallback(
    async (path: string, outputPath: string, options: ExportFitsOptions) => {
      setExportLoading(true);
      try {
        const result = await exportFits(path, outputPath, options);
        setExportResult(result);
      } catch (e) {
        console.error("FITS export failed:", e);
        throw e;
      } finally {
        setExportLoading(false);
      }
    },
    [],
  );

  const handleExportFitsRgb = useCallback(
    async (
      rPath: string | null,
      gPath: string | null,
      bPath: string | null,
      outputPath: string,
      options: ExportFitsRgbOptions,
    ) => {
      setExportLoading(true);
      try {
        const result = await exportFitsRgb(rPath, gPath, bPath, outputPath, {
          copyWcs: options?.copyWcs ?? true,
          copyMetadata: options?.copyMetadata ?? true,
          bitpix: options?.bitpix,
          history: options?.history,
          compress: options?.compress,
          quantizeLevel: options?.quantizeLevel,
        });
        setExportResult(result);
      } catch (e) {
        console.error("RGB FITS export failed:", e);
        throw e;
      } finally {
        setExportLoading(false);
      }
    },
    [],
  );

  const handleExportCubeFrames = useCallback(async () => {
    if (!file?.path || !cubeDims) return;
    const path = file.path;
    const totalFrames = cubeDims.frames ?? 0;
    if (totalFrames <= 0) return;

    cubeCancelRef.current = false;
    setCubeStopping(false);
    setCubeExporting(true);
    setCubeExportProgress(0);
    setCubeExportResult(null);

    const stamp = exportTimestamp(new Date());
    const dir = await getExportDir();
    let exported = 0;

    try {
      for (let i = 0; i < totalFrames && !cubeCancelRef.current; i++) {
        const outputPath = `${dir}/${cubeFrameExportName(path, i, "png", stamp)}`;
        const fitsPath = cubeExportFits ? `${dir}/${cubeFrameExportName(path, i, "fits", stamp)}` : undefined;
        await getCubeFrame(path, i, outputPath, fitsPath);
        exported++;
        setCubeExportProgress(Math.round((exported / totalFrames) * 100));
      }
      setCubeExportResult({ path, exported, total: totalFrames, fits: cubeExportFits, dir, stopped: exported < totalFrames });
    } catch (e) {
      console.error("Cube frame export failed:", e);
      setCubeExportResult({ path, exported, total: totalFrames, error: e instanceof Error ? e.message : String(e) });
    } finally {
      const stop = stopButtonRef.current;
      refocusExportFramesRef.current = stop !== null && stop.ownerDocument.activeElement === stop;
      setCubeExporting(false);
    }
  }, [file?.path, cubeDims, cubeExportFits]);

  const stopCubeExport = useCallback(() => {
    cubeCancelRef.current = true;
    setCubeStopping(true);
  }, []);

  useLayoutEffect(() => {
    if (cubeExporting || !refocusExportFramesRef.current) return;
    refocusExportFramesRef.current = false;
    exportFramesButtonRef.current?.focus();
  }, [cubeExporting]);

  const totalFrames = cubeDims ? (cubeDims.frames ?? 0) : 0;
  const shownCubeResult = cubeExportResult && cubeExportResult.path === file?.path ? cubeExportResult : null;

  const rgbProduct = useMemo(
    () =>
      rgbProductOf({
        compositePreviewUrl,
        fileIsRgb: file?.result?.is_rgb === true,
        filePath: file?.path ?? null,
        filePreviewUrl: file?.result?.previewUrl ?? null,
        processed,
      }),
    [compositePreviewUrl, file?.result?.is_rgb, file?.path, file?.result?.previewUrl, processed],
  );

  const rgbSource = useMemo(() => resolveRgbExportSource({ rgbChannels, product: rgbProduct }), [rgbChannels, rgbProduct]);

  const rgbResult = useMemo(
    () => (rgbProduct?.kind === "result" ? { label: rgbProduct.label, displayReferred: rgbProduct.displayReferred } : null),
    [rgbProduct],
  );

  const compositeStf = useMemo(() => {
    const stfOnScreen = rgbProduct?.kind === "file" || rgbProduct?.kind === "composite";
    if (!compositePreviewUrl || !stfOnScreen) return null;
    return { r: compositeStfR, g: compositeStfG, b: compositeStfB, linked: compositeStfLinked };
  }, [compositePreviewUrl, rgbProduct, compositeStfR, compositeStfG, compositeStfB, compositeStfLinked]);

  const effectiveRgbChannels = useMemo(() => {
    if (rgbSource?.kind === "channels") return rgbSource.channels;
    if (rgbSource?.kind === "composite") return { r: null, g: null, b: null };
    return null;
  }, [rgbSource]);

  const sourceLabel = exportSourceLabel(displayed);
  const pngSourceRgb = processed?.fitsPath ? processed.planes === RGB_PLANES : file?.result?.is_rgb === true;

  return (
    <Suspense
      fallback={
        <div className="flex items-center justify-center py-12">
          <Loader2 size={20} className="animate-spin text-zinc-500" />
        </div>
      }
    >
      <div className="flex flex-col gap-4 p-3">
        <ExportPanel
          filePath={file?.path ?? null}
          exportPath={displayed.path}
          sourceLabel={sourceLabel}
          stfParams={stfParams}
          autoStf={histData?.auto_stf ?? null}
          display={display}
          pngSourceRgb={pngSourceRgb}
          onExport={handleExportFits}
          onExportRgb={handleExportFitsRgb}
          rgbChannels={effectiveRgbChannels}
          headerChannels={rgbChannels}
          rgbFilePath={rgbSource?.kind === "file" ? rgbSource.path : null}
          rgbResult={rgbResult}
          compositeStf={compositeStf}
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
                ref={exportFramesButtonRef}
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
                <>
                  <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
                    <div
                      className="h-full bg-purple-500 rounded-full transition-all duration-300"
                      style={{ width: `${cubeExportProgress}%` }}
                    />
                  </div>
                  <button
                    ref={stopButtonRef}
                    type="button"
                    onClick={stopCubeExport}
                    aria-disabled={cubeStopping || undefined}
                    title="Stop after the frame being exported"
                    className="w-full flex items-center justify-center gap-1 rounded border border-zinc-700/60 px-3 py-1.5 text-xs text-zinc-300 hover:text-zinc-100 hover:border-zinc-500 transition-colors aria-disabled:opacity-50 aria-disabled:cursor-default"
                  >
                    <X size={12} /> {cubeStopping ? "Stopping..." : "Stop"}
                  </button>
                </>
              )}

              {(cubeExporting || shownCubeResult) && (
                <div role="status">
                  {shownCubeResult && (
                    <div className={`text-[10px] font-mono px-2 py-1 rounded ${
                      shownCubeResult.error || shownCubeResult.stopped ? "text-amber-300 bg-amber-900/20" : "text-emerald-300 bg-emerald-900/20"
                    }`}>
                      {shownCubeResult.exported}/{shownCubeResult.total} frames exported
                      {shownCubeResult.fits && " (PNG + FITS)"}
                      {shownCubeResult.stopped && " (stopped)"}
                      {shownCubeResult.dir && !shownCubeResult.error && (
                        <span className="block text-emerald-400/90 mt-0.5 truncate">
                          {shownCubeResult.dir}
                        </span>
                      )}
                      {shownCubeResult.error && ` (stopped: ${shownCubeResult.error})`}
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </Suspense>
  );
}

export default memo(ExportTabInner);
