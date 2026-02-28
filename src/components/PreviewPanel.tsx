import { useState, useEffect, useCallback, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Image, Download, Cpu, Zap } from "lucide-react";

import HeaderTable from "./HeaderTable";
import HistogramPanel from "./HistogramPanel";
import FFTPanel from "./FFTPanel";
import SpectroscopyPanel from "./SpectroscopyPanel";
import GpuRenderer from "./GpuRenderer";
import PlateSolvePanel from "./PlateSolvePanel";
import RgbComposePanel from "./RgbComposePanel";
import ExportPanel from "./ExportPanel";
import HeaderExplorerPanel from "./HeaderExplorerPanel";
import DrizzlePanel from "./DrizzlePanel";
import { useBackend } from "../hooks/useBackend";
import type { ProcessedFile, StfParams, RawPixelData, HeaderData } from "../utils/types";

const fadeIn = {
  initial: { opacity: 0 },
  animate: { opacity: 1 },
  exit: { opacity: 0 },
  transition: { duration: 0.2 },
};

interface PreviewPanelProps {
  file: ProcessedFile | null;
  allFiles: ProcessedFile[];
}

export default function PreviewPanel({ file, allFiles }: PreviewPanelProps) {
  const {
    computeHistogram,
    applyStfRender,
    getRawPixels,
    getCubeSpectrum,
    getCubeInfo,
    detectStars,
    composeRgb,
    exportFits,
    exportFitsRgb,
    computeFftSpectrum,
    getFullHeader,
    drizzleStack,
  } = useBackend();

  const [histData, setHistData] = useState<any>(null);
  const [stfParams, setStfParams] = useState<StfParams>({
    shadow: 0,
    midtone: 0.5,
    highlight: 1,
  });
  const [stfPreviewUrl, setStfPreviewUrl] = useState<string | null>(null);

  const [useGpu, setUseGpu] = useState(false);
  const [rawPixels, setRawPixels] = useState<RawPixelData | null>(null);

  const [spectrum, setSpectrum] = useState<number[]>([]);
  const [specWavelengths, setSpecWavelengths] = useState<number[] | null>(null);
  const [specCoord, setSpecCoord] = useState<{ x: number; y: number } | null>(null);
  const [specLoading, setSpecLoading] = useState(false);
  const [specElapsed, setSpecElapsed] = useState(0);
  const [cubeDims, setCubeDims] = useState<any>(null);
  const [isCube, setIsCube] = useState(false);

  const prevFileIdRef = useRef<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const previewImgRef = useRef<HTMLImageElement>(null);
  const starOverlayRef = useRef<HTMLCanvasElement>(null);

  const [starResult, setStarResult] = useState<any>(null);
  const [starLoading, setStarLoading] = useState(false);

  const [rgbResult, setRgbResult] = useState<any>(null);
  const [rgbLoading, setRgbLoading] = useState(false);
  const [rgbChannels, setRgbChannels] = useState<any>(null);

  const [exportResult, setExportResult] = useState<any>(null);
  const [exportLoading, setExportLoading] = useState(false);

  const [headerData, setHeaderData] = useState<HeaderData | null>(null);
  const [headerLoading, setHeaderLoading] = useState(false);

  const [drizzleResult, setDrizzleResult] = useState<any>(null);
  const [drizzleLoading, setDrizzleLoading] = useState(false);

  useEffect(() => {
    if (!file || !file.path || file.id === prevFileIdRef.current) return;
    prevFileIdRef.current = file.id;

    setHistData(null);
    setStfPreviewUrl(null);
    setStfParams({ shadow: 0, midtone: 0.5, highlight: 1 });
    setRawPixels(null);
    setUseGpu(false);
    setSpectrum([]);
    setSpecWavelengths(null);
    setSpecCoord(null);
    setCubeDims(null);
    setIsCube(false);
    setStarResult(null);
    setRgbResult(null);
    setRgbChannels(null);
    setExportResult(null);
    setHeaderData(null);
    setDrizzleResult(null);

    computeHistogram(file.path)
      .then((data: any) => {
        setHistData(data);
        if (data.auto_stf) {
          setStfParams(data.auto_stf);
        }
      })
      .catch((err: any) => console.error("Histogram fetch failed:", err));

    const naxis3 = file.result?.header?.NAXIS3;
    if (naxis3 && parseInt(naxis3, 10) > 1) {
      setIsCube(true);
      getCubeInfo(file.path)
        .then((info: any) => setCubeDims(info))
        .catch(() => {});
    }
  }, [file?.id]);

  const handleLoadHeader = useCallback(
    async (path: string) => {
      setHeaderLoading(true);
      try {
        const data = await getFullHeader(path);
        setHeaderData(data);
      } catch (e) {
        console.error("Header load failed:", e);
      } finally {
        setHeaderLoading(false);
      }
    },
    [getFullHeader],
  );

  const handleStfChange = useCallback(
    (params: StfParams) => {
      setStfParams(params);
      if (useGpu) return;

      if (debounceRef.current) clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(async () => {
        if (!file?.path) return;
        try {
          const result = await applyStfRender(
            file.path,
            "./output",
            params.shadow,
            params.midtone,
            params.highlight,
          );
          setStfPreviewUrl(result.previewUrl);
        } catch (e) {
          console.error("STF render failed:", e);
        }
      }, 150);
    },
    [file?.path, useGpu, applyStfRender],
  );

  const handleAutoStf = useCallback(() => {
    if (histData?.auto_stf) {
      const params = histData.auto_stf;
      setStfParams(params);
      handleStfChange(params);
    }
  }, [histData, handleStfChange]);

  const handleResetStf = useCallback(() => {
    const params: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };
    setStfParams(params);
    setStfPreviewUrl(null);
  }, []);

  const handleToggleGpu = useCallback(async () => {
    if (useGpu) {
      setUseGpu(false);
      setRawPixels(null);
      return;
    }

    if (!file?.path) return;
    try {
      const result = await getRawPixels(file.path);
      const binary = atob(result.data_b64);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      const f32 = new Float32Array(bytes.buffer);

      setRawPixels({
        data: f32,
        width: result.width,
        height: result.height,
        min: result.data_min,
        max: result.data_max,
      });
      setUseGpu(true);
    } catch (e) {
      console.error("Failed to load raw pixels:", e);
    }
  }, [useGpu, file?.path, getRawPixels]);

  const handleImageClick = useCallback(
    async (e: React.MouseEvent<HTMLImageElement>) => {
      if (!isCube || !file?.path) return;

      const img = e.target as HTMLImageElement;
      const rect = img.getBoundingClientRect();
      const dims = file.result?.dimensions;
      if (!dims) return;

      const relX = (e.clientX - rect.left) / rect.width;
      const relY = (e.clientY - rect.top) / rect.height;
      const pixelX = Math.floor(relX * dims[0]);
      const pixelY = Math.floor(relY * dims[1]);

      setSpecCoord({ x: pixelX, y: pixelY });
      setSpecLoading(true);

      try {
        const result = await getCubeSpectrum(file.path, pixelX, pixelY);
        setSpectrum(result.spectrum || []);
        setSpecWavelengths(result.wavelengths || null);
        setSpecElapsed(result.elapsed_ms || 0);
      } catch (e) {
        console.error("Spectrum fetch failed:", e);
        setSpectrum([]);
      } finally {
        setSpecLoading(false);
      }
    },
    [isCube, file?.path, file?.result?.dimensions, getCubeSpectrum],
  );

  const handleDownloadPng = () => {
    const url = stfPreviewUrl || file?.result?.previewUrl;
    if (!url) return;
    const a = document.createElement("a");
    a.href = url;
    a.download = (file?.name || "image").replace(/\.(fits?|zip)$/i, ".png");
    a.click();
  };

  const handleDetectStars = useCallback(
    async (sigma: number) => {
      if (!file?.path) return;
      setStarLoading(true);
      try {
        const result = await detectStars(file.path, sigma, 200);
        setStarResult(result);
      } catch (e) {
        console.error("Star detection failed:", e);
      } finally {
        setStarLoading(false);
      }
    },
    [file?.path, detectStars],
  );

  const handleComposeRgb = useCallback(
    async (rPath: string, gPath: string, bPath: string, options: any) => {
      setRgbLoading(true);
      try {
        const result = await composeRgb(rPath, gPath, bPath, "./output", options);
        setRgbResult(result);
        setRgbChannels({ r: rPath, g: gPath, b: bPath });
      } catch (e) {
        console.error("RGB compose failed:", e);
      } finally {
        setRgbLoading(false);
      }
    },
    [composeRgb],
  );

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
        const result = await exportFitsRgb(rPath, gPath, bPath, outputPath, options);
        setExportResult(result);
      } catch (e) {
        console.error("RGB FITS export failed:", e);
      } finally {
        setExportLoading(false);
      }
    },
    [exportFitsRgb],
  );

  const handleDrizzle = useCallback(
    async (paths: string[], options: any) => {
      setDrizzleLoading(true);
      try {
        const result = await drizzleStack(paths, "./output", options);
        setDrizzleResult(result);
      } catch (e) {
        console.error("Drizzle stack failed:", e);
      } finally {
        setDrizzleLoading(false);
      }
    },
    [drizzleStack],
  );

  const previewUrl = stfPreviewUrl || file?.result?.previewUrl;
  const doneFiles = allFiles?.filter((f) => f.status === "done") || [];

  return (
    <div className="flex flex-col h-full bg-zinc-900 border border-zinc-800 rounded-xl overflow-hidden">
      <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
        <h3 className="text-sm font-semibold text-zinc-300">Preview</h3>
        <div className="flex items-center gap-2">
          {file && (
            <button
              onClick={handleToggleGpu}
              className={`flex items-center gap-1 text-[10px] px-2 py-1 rounded transition-colors ${
                useGpu
                  ? "bg-purple-500/20 text-purple-300 border border-purple-500/30"
                  : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800"
              }`}
              title={useGpu ? "GPU rendering active" : "Enable GPU (shader) rendering"}
            >
              {useGpu ? <Zap size={10} /> : <Cpu size={10} />}
              {useGpu ? "GPU" : "CPU"}
            </button>
          )}
          {file && (
            <span className="text-xs font-mono text-zinc-500 truncate max-w-[180px]">
              {file.name}
            </span>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-4">
        <AnimatePresence mode="wait">
          {!file ? (
            <motion.div
              key="empty"
              {...fadeIn}
              className="flex flex-col items-center justify-center h-full gap-3 text-zinc-600"
            >
              <Image size={48} strokeWidth={1} />
              <p className="text-sm">Select a processed file</p>
            </motion.div>
          ) : (
            <motion.div key={file.id} {...fadeIn} className="flex flex-col gap-4">
              {useGpu && rawPixels ? (
                <div className="relative bg-zinc-950 rounded-lg overflow-hidden border border-zinc-800">
                  <GpuRenderer
                    rawData={rawPixels.data}
                    width={rawPixels.width}
                    height={rawPixels.height}
                    dataMin={rawPixels.min}
                    dataMax={rawPixels.max}
                    shadow={stfParams.shadow}
                    midtone={stfParams.midtone}
                    highlight={stfParams.highlight}
                    className="w-full"
                  />
                </div>
              ) : previewUrl ? (
                <div className="relative bg-zinc-950 rounded-lg overflow-hidden border border-zinc-800">
                  <img
                    ref={previewImgRef}
                    src={previewUrl}
                    alt={file.name}
                    className={`w-full h-auto object-contain max-h-[400px] ${
                      isCube ? "cursor-crosshair" : ""
                    }`}
                    onClick={handleImageClick}
                    onError={(e) => {
                      const img = e.target as HTMLImageElement;
                      const src = img.src;
                      if (!src.includes("retry=1")) {
                        setTimeout(() => {
                          img.src = src.includes("?")
                            ? `${src}&retry=1&t=${Date.now()}`
                            : `${src}?retry=1&t=${Date.now()}`;
                        }, 500);
                      }
                    }}
                  />
                  <canvas
                    ref={starOverlayRef}
                    className="absolute inset-0 w-full h-full pointer-events-none"
                    style={{ display: "none" }}
                  />
                  {isCube && (
                    <div className="absolute bottom-2 right-2 bg-black/60 backdrop-blur-sm text-[10px] text-purple-300 px-2 py-1 rounded">
                      Click to extract spectrum
                    </div>
                  )}
                </div>
              ) : null}

              {histData && (
                <HistogramPanel
                  bins={histData.bins}
                  dataMin={histData.data_min}
                  dataMax={histData.data_max}
                  autoStf={histData.auto_stf}
                  shadow={stfParams.shadow}
                  midtone={stfParams.midtone}
                  highlight={stfParams.highlight}
                  onChange={handleStfChange}
                  onAutoStf={handleAutoStf}
                  onReset={handleResetStf}
                  stats={{
                    median: histData.median,
                    mean: histData.mean,
                    sigma: histData.sigma,
                  }}
                />
              )}

              {file?.path && (
                <FFTPanel filePath={file.path} computeFftSpectrum={computeFftSpectrum} />
              )}

              {isCube && (
                <SpectroscopyPanel
                  spectrum={spectrum}
                  wavelengths={specWavelengths}
                  pixelCoord={specCoord}
                  isLoading={specLoading}
                  cubeDims={cubeDims}
                  elapsed={specElapsed}
                />
              )}

              <HeaderExplorerPanel
                file={file}
                onLoadHeader={handleLoadHeader}
                headerData={headerData}
                isLoading={headerLoading}
              />

              <PlateSolvePanel
                stars={starResult?.stars || []}
                count={starResult?.count || 0}
                isLoading={starLoading}
                onDetect={handleDetectStars}
                backgroundMedian={starResult?.background_median}
                backgroundSigma={starResult?.background_sigma}
                imageWidth={starResult?.image_width || file.result?.dimensions?.[0]}
                imageHeight={starResult?.image_height || file.result?.dimensions?.[1]}
                elapsed={starResult?.elapsed_ms || 0}
                overlayCanvasRef={starOverlayRef}
              />

              {doneFiles.length >= 2 && (
                <RgbComposePanel
                  files={doneFiles}
                  onCompose={handleComposeRgb}
                  result={rgbResult}
                  isLoading={rgbLoading}
                />
              )}

              {doneFiles.length >= 2 && (
                <DrizzlePanel
                  files={doneFiles}
                  onDrizzle={(paths: string[], opts: any) => handleDrizzle(paths, opts)}
                  result={drizzleResult}
                  isLoading={drizzleLoading}
                />
              )}

              <ExportPanel
                filePath={file?.path}
                stfParams={stfParams}
                onExport={handleExportFits}
                onExportRgb={handleExportFitsRgb}
                rgbChannels={rgbChannels}
                isLoading={exportLoading}
                lastResult={exportResult}
              />

              {file.result?.header && (
                <div className="bg-zinc-950/50 rounded-lg p-4 border border-zinc-800/50">
                  <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-3">
                    FITS Header (Summary)
                  </h4>
                  <HeaderTable header={file.result.header} />
                </div>
              )}

              <button
                onClick={handleDownloadPng}
                className="flex items-center justify-center gap-2 bg-zinc-800 hover:bg-zinc-700 text-zinc-200 rounded-lg px-4 py-2.5 font-medium transition-colors text-sm w-full"
              >
                <Download size={16} />
                Download PNG
              </button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
}
