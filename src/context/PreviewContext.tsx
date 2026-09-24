/* @refresh reset */
import {
  createContext,
  useContext,
  useState,
  useEffect,
  useRef,
  useMemo,
  useCallback,
} from "react";
import { computeHistogram } from "../services/analysis";
import { getCubeInfo } from "../services/cube";
import { getRawPixelsPreview, getRawRgbPixelsPreview } from "../services/fits";
import { detectNarrowbandFilters } from "../services/header";
import type { NarrowbandDetection, NarrowbandFilterDetection } from "../services/header";
import { fileStore } from "../hooks/useFileStore";
import { useCompositeActions } from "./CompositeContext";
import type {
  ProcessedFile,
  StfParams,
  HistogramData,
  RawPixelData,
  RawRgbPixelData,
  PlaneInfo,
  DqFlagTable,
  DqMaskData,
  DqOverlaySettings,
} from "../shared/types";
import type { FileRenderState, ProcessedResult, ProcessingChain } from "../shared/types/preview";
import type { CubeDims } from "../shared/types";
import {
  COLORMAP_NAMES,
  DEFAULT_DISPLAY_SETTINGS,
  GRID_FRAMES,
  LIMIT_MODES,
  STRETCH_MODES,
  type DisplaySettings,
  type ScaleLimits,
} from "../shared/types/display";
import { computeScaleLimits, getColormapLut, getDqFlagTable, getDqMaskPreview } from "../services/display";
import { GRAY_LUT_RGBA } from "../utils/displayTransfer";
import { clampGridDensity } from "../utils/gridSteps";
import { EMPTY_CHAIN, pruneRecord, putCapped, withVersionParam } from "../utils/processingChain";

export type PaletteSuggestion = NarrowbandDetection["palette"];

export interface RgbChannelMap {
  r: string | null;
  g: string | null;
  b: string | null;
}

interface FileContextValue {
  file: ProcessedFile | null;
}

interface DoneFilesContextValue {
  doneFiles: ProcessedFile[];
}

interface HistContextValue {
  histData: HistogramData | null;
  stfParams: StfParams;
  setStfParams: (p: StfParams) => void;
}

interface CubeContextValue {
  isCube: boolean;
  isSpectralCube: boolean;
  spectralReason: string | null;
  cubeDims: CubeDims | null;
}

interface RgbContextValue {
  rgbChannels: RgbChannelMap | null;
  setRgbChannels: React.Dispatch<React.SetStateAction<RgbChannelMap | null>>;
}

interface RenderContextValue {
  processed: ProcessedResult | null;
  processedVersion: number;
  chain: ProcessingChain;
  stfPreviewUrl: string | null;
}

interface RenderActionsContextValue {
  currentFileKey: () => string | null;
  publishProcessed: (fileKey: string, result: ProcessedResult, chainUpdate?: (c: ProcessingChain) => ProcessingChain) => void;
  setChain: (fileKey: string, update: (c: ProcessingChain) => ProcessingChain) => void;
  resetProcessed: () => void;
  forgetOutputs: (paths: string[]) => void;
  setStfPreviewUrl: (url: string | null) => void;
}

export interface DisplayedImage {
  path: string | null;
  dimensions: [number, number] | null;
  isProcessed: boolean;
  label: string | null;
  previewOnly: boolean;
  previewUrl: string | null;
}

interface StarOverlayContextValue {
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
}

interface RawPixelsContextValue {
  rawPixels: RawPixelData | null;
  rawPixelsLoading: boolean;
  rawPixelsError: string | null;
  loadRawPixels: (force?: boolean) => void;
  clearRawPixels: () => void;
  rgbRawPixels: RawRgbPixelData | null;
  rgbRawPixelsLoading: boolean;
  loadRgbRawPixels: (source: string | null, force?: boolean) => void;
  clearRgbRawPixels: () => void;
}

interface NarrowbandContextValue {
  narrowbandPalette: PaletteSuggestion | null;
  narrowbandFilters: NarrowbandFilterDetection[];
  selectedPalette: string;
  setSelectedPalette: (p: string) => void;
}

interface DisplayContextValue {
  display: DisplaySettings;
  setDisplay: (patch: Partial<DisplaySettings>) => void;
  limits: ScaleLimits | null;
  lut: Uint8Array | null;
  limitsLoading: boolean;
  limitsError: string | null;
}

interface DqContextValue {
  plane: PlaneInfo | null;
  flagTable: DqFlagTable | null;
  overlay: DqOverlaySettings;
  setOverlay: (patch: Partial<DqOverlaySettings>) => void;
  excludeDq: boolean;
  setExcludeDq: (v: boolean) => void;
  dqMask: DqMaskData | null;
  dqMaskLoading: boolean;
  dqMaskError: string | null;
}

const DEFAULT_DQ_OVERLAY: DqOverlaySettings = { enabled: false, mask: 0 };

const DISPLAY_STORAGE_KEY = "astroburst.display.v1";
const LIMITS_DEBOUNCE_MS = 150;

function finiteOr(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function finiteOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function sanitizeDisplaySettings(raw: unknown): DisplaySettings {
  const d = DEFAULT_DISPLAY_SETTINGS;
  if (!raw || typeof raw !== "object") return d;
  const r = raw as Record<string, unknown>;
  const stretch = STRETCH_MODES.find((s) => s === r.stretch) ?? d.stretch;
  const limits = LIMIT_MODES.find((l) => l === r.limits) ?? d.limits;
  const colormap = COLORMAP_NAMES.find((c) => c === r.colormap) ?? d.colormap;
  const gridFrame = GRID_FRAMES.find((f) => f === r.gridFrame) ?? d.gridFrame;
  return {
    stretch,
    limits,
    percentileLow: finiteOr(r.percentileLow, d.percentileLow),
    percentileHigh: finiteOr(r.percentileHigh, d.percentileHigh),
    userLo: finiteOrNull(r.userLo),
    userHi: finiteOrNull(r.userHi),
    zscaleContrast: finiteOr(r.zscaleContrast, d.zscaleContrast),
    asinhA: finiteOr(r.asinhA, d.asinhA),
    power: finiteOr(r.power, d.power),
    colormap,
    invert: r.invert === true,
    grid: r.grid === true,
    gridFrame,
    gridDensity: clampGridDensity(finiteOr(r.gridDensity, d.gridDensity)),
  };
}

function loadDisplaySettings(): DisplaySettings {
  try {
    const text = window.localStorage.getItem(DISPLAY_STORAGE_KEY);
    if (!text) return DEFAULT_DISPLAY_SETTINGS;
    return sanitizeDisplaySettings(JSON.parse(text));
  } catch {
    return DEFAULT_DISPLAY_SETTINGS;
  }
}

function saveDisplaySettings(settings: DisplaySettings): void {
  try {
    window.localStorage.setItem(DISPLAY_STORAGE_KEY, JSON.stringify(settings));
  } catch {
  }
}

const FileCtx = createContext<FileContextValue | null>(null);
const DisplayCtx = createContext<DisplayContextValue | null>(null);
const DoneFilesCtx = createContext<DoneFilesContextValue | null>(null);
const HistCtx = createContext<HistContextValue | null>(null);
const CubeCtx = createContext<CubeContextValue | null>(null);
const RgbCtx = createContext<RgbContextValue | null>(null);
const RenderCtx = createContext<RenderContextValue | null>(null);
const RenderActionsCtx = createContext<RenderActionsContextValue | null>(null);
const StarOverlayCtx = createContext<StarOverlayContextValue | null>(null);
const RawPixelsCtx = createContext<RawPixelsContextValue | null>(null);
const NarrowbandCtx = createContext<NarrowbandContextValue | null>(null);
const DqCtx = createContext<DqContextValue | null>(null);

function useCtx<T>(ctx: React.Context<T | null>, name: string): T {
  const val = useContext(ctx);
  if (!val) throw new Error(`${name} must be used within PreviewProvider`);
  return val;
}

export const useFileContext = () => useCtx(FileCtx, "useFileContext");
export const useDoneFilesContext = () => useCtx(DoneFilesCtx, "useDoneFilesContext");
export const useHistContext = () => useCtx(HistCtx, "useHistContext");
export const useCubeContext = () => useCtx(CubeCtx, "useCubeContext");
export const useRgbContext = () => useCtx(RgbCtx, "useRgbContext");
export const useRenderContext = () => useCtx(RenderCtx, "useRenderContext");
export const useRenderActions = () => useCtx(RenderActionsCtx, "useRenderActions");
export const useStarOverlayContext = () => useCtx(StarOverlayCtx, "useStarOverlayContext");
export const useRawPixelsContext = () => useCtx(RawPixelsCtx, "useRawPixelsContext");
export const useNarrowbandContext = () => useCtx(NarrowbandCtx, "useNarrowbandContext");
export const useDisplayContext = () => useCtx(DisplayCtx, "useDisplayContext");
export const useDqContext = () => useCtx(DqCtx, "useDqContext");

export function useDisplayedImage(): DisplayedImage {
  const { file } = useCtx(FileCtx, "useDisplayedImage");
  const { processed } = useCtx(RenderCtx, "useDisplayedImage");
  const filePath = file?.path ?? null;
  const fileDims = file?.result?.dimensions ?? null;
  const filePreviewUrl = file?.result?.previewUrl ?? null;
  return useMemo<DisplayedImage>(
    () => ({
      path: processed?.fitsPath ?? filePath,
      dimensions: processed?.dimensions ?? fileDims,
      isProcessed: processed !== null,
      label: processed?.label ?? null,
      previewOnly: processed !== null && processed.previewUrl !== null && processed.fitsPath === null,
      previewUrl: processed?.previewUrl ?? filePreviewUrl,
    }),
    [processed, filePath, fileDims, filePreviewUrl],
  );
}

interface Props {
  file: ProcessedFile | null;
  doneFiles: ProcessedFile[];
  children: React.ReactNode;
}

const RENDER_CACHE_MAX = 50;
const renderRecords = new Map<string, FileRenderState>();
let renderVersionSeq = 0;

function nextRenderVersion(): number {
  renderVersionSeq += 1;
  return renderVersionSeq;
}

export function getRenderRecord(fileKey: string): FileRenderState | null {
  return renderRecords.get(fileKey) ?? null;
}

export function listRenderRecords(): FileRenderState[] {
  return [...renderRecords.values()];
}

interface RenderView extends FileRenderState {
  key: string | null;
}

const EMPTY_VIEW: RenderView = { key: null, processed: null, chain: EMPTY_CHAIN, version: 0 };

function viewFor(key: string | null): RenderView {
  const record = key ? renderRecords.get(key) : undefined;
  return record ? { key, ...record } : { ...EMPTY_VIEW, key };
}

interface StfPreviewState {
  key: string | null;
  version: number;
  url: string | null;
}

const NO_STF_PREVIEW: StfPreviewState = { key: null, version: 0, url: null };

const DEFAULT_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };

const PREVIEW_MAX_DIM_CAP = 2048;

function computePreviewMaxDim(): number {
  const dpr = window.devicePixelRatio || 1;
  return Math.min(Math.round(Math.max(window.innerWidth, window.innerHeight) * dpr), PREVIEW_MAX_DIM_CAP);
}

export function fileKeyOf(file: ProcessedFile | null): string | null {
  return file ? `${file.id}|${file.path}` : null;
}

export function PreviewProvider({ file, doneFiles, children }: Props) {
  const composite = useCompositeActions();

  const [histData, setHistData] = useState<HistogramData | null>(null);
  const [stfParams, setStfParams] = useState<StfParams>(DEFAULT_STF);
  const [isCube, setIsCube] = useState(false);
  const [isSpectralCube, setIsSpectralCube] = useState(false);
  const [spectralReason, setSpectralReason] = useState<string | null>(null);
  const [cubeDims, setCubeDims] = useState<CubeDims | null>(null);
  const [rgbChannels, setRgbChannels] = useState<RgbChannelMap | null>(null);
  const [view, setView] = useState<RenderView>(EMPTY_VIEW);
  const [stfPreview, setStfPreview] = useState<StfPreviewState>(NO_STF_PREVIEW);
  const [rawPixels, setRawPixels] = useState<RawPixelData | null>(null);
  const [rawPixelsLoading, setRawPixelsLoading] = useState(false);
  const [rawPixelsError, setRawPixelsError] = useState<string | null>(null);
  const [rgbRawPixels, setRgbRawPixels] = useState<RawRgbPixelData | null>(null);
  const [rgbRawPixelsLoading, setRgbRawPixelsLoading] = useState(false);
  const [narrowbandPalette, setNarrowbandPalette] = useState<PaletteSuggestion | null>(null);
  const [narrowbandFilters, setNarrowbandFilters] = useState<NarrowbandFilterDetection[]>([]);
  const [selectedPalette, setSelectedPaletteRaw] = useState("SHO");
  const [display, setDisplayRaw] = useState<DisplaySettings>(loadDisplaySettings);
  const [limits, setLimits] = useState<ScaleLimits | null>(null);
  const [limitsLoading, setLimitsLoading] = useState(false);
  const [limitsError, setLimitsError] = useState<string | null>(null);
  const [lut, setLut] = useState<Uint8Array | null>(GRAY_LUT_RGBA);
  const [flagTable, setFlagTable] = useState<DqFlagTable | null>(null);
  const [overlay, setOverlayRaw] = useState<DqOverlaySettings>(DEFAULT_DQ_OVERLAY);
  const [excludeDq, setExcludeDq] = useState(false);
  const [dqMask, setDqMask] = useState<DqMaskData | null>(null);
  const [dqMaskLoading, setDqMaskLoading] = useState(false);
  const [dqMaskError, setDqMaskError] = useState<string | null>(null);
  const [dqMaskRefetch, setDqMaskRefetch] = useState(0);

  const fileKey = fileKeyOf(file);
  if (view.key !== fileKey) {
    setView(fileKey ? viewFor(fileKey) : EMPTY_VIEW);
    setStfPreview(NO_STF_PREVIEW);
  }
  const current = view.key === fileKey ? view : EMPTY_VIEW;
  const processed = current.processed;
  const processedVersion = current.version;
  const chain = current.chain;
  const processedPath = processed?.fitsPath ?? null;
  const sourceVersion = processedPath ? processedVersion : 0;
  const processedPreviewOnly = processed !== null && processed.fitsPath === null && processed.previewUrl !== null;
  const stfPreviewUrl =
    !processedPreviewOnly && stfPreview.key === fileKey && stfPreview.version === processedVersion ? stfPreview.url : null;

  const prevFileIdRef = useRef<string | null>(null);
  const histSeqRef = useRef(0);
  const flagTableSeqRef = useRef(0);
  const dqMaskSeqRef = useRef(0);
  const dqMaskKeyRef = useRef("");
  const excludeDqRef = useRef(excludeDq);
  excludeDqRef.current = excludeDq;
  const limitsSeqRef = useRef(0);
  const seqRef = useRef(0);
  const rawPixelsAbortRef = useRef(0);
  const rgbRawPixelsAbortRef = useRef(0);
  const lastFetchMaxDimRef = useRef(0);
  const rgbSourceRef = useRef<string | null>(null);
  const narrowbandKeyRef = useRef("");
  const narrowbandSeqRef = useRef(0);
  const starOverlayRef = useRef<HTMLCanvasElement>(null);

  const rawPixelsRef = useRef(rawPixels);
  rawPixelsRef.current = rawPixels;
  const rawPixelsLoadingRef = useRef(rawPixelsLoading);
  rawPixelsLoadingRef.current = rawPixelsLoading;
  const rgbRawPixelsRef = useRef(rgbRawPixels);
  rgbRawPixelsRef.current = rgbRawPixels;
  const rgbRawPixelsLoadingRef = useRef(rgbRawPixelsLoading);
  rgbRawPixelsLoadingRef.current = rgbRawPixelsLoading;
  const filePathRef = useRef(file?.path);
  filePathRef.current = file?.path;
  const fileKeyRef = useRef(fileKey);
  fileKeyRef.current = fileKey;
  const viewRef = useRef<RenderView>(current);
  viewRef.current = current;
  const processedRef = useRef<ProcessedResult | null>(processed);
  processedRef.current = processed;

  const currentFileKey = useCallback(() => fileKeyRef.current, []);

  const commitView = useCallback((next: RenderView) => {
    viewRef.current = next;
    processedRef.current = next.processed;
    setView(next);
  }, []);

  const publishProcessed = useCallback(
    (key: string, result: ProcessedResult, chainUpdate?: (c: ProcessingChain) => ProcessingChain) => {
      const isCurrent = key === fileKeyRef.current;
      const base = renderRecords.get(key) ?? (isCurrent ? viewRef.current : null);
      const baseChain = base?.chain ?? EMPTY_CHAIN;
      const version = nextRenderVersion();
      const record: FileRenderState = {
        processed: {
          ...result,
          previewUrl: result.previewUrl ? withVersionParam(result.previewUrl, version) : null,
        },
        chain: chainUpdate ? chainUpdate(baseChain) : baseChain,
        version,
      };
      putCapped(renderRecords, key, record, RENDER_CACHE_MAX);
      if (isCurrent) commitView({ key, ...record });
    },
    [commitView],
  );

  const setChain = useCallback(
    (key: string, update: (c: ProcessingChain) => ProcessingChain) => {
      const isCurrent = key === fileKeyRef.current;
      const base = renderRecords.get(key) ?? (isCurrent ? viewRef.current : null);
      const record: FileRenderState = {
        processed: base?.processed ?? null,
        chain: update(base?.chain ?? EMPTY_CHAIN),
        version: base?.version ?? 0,
      };
      putCapped(renderRecords, key, record, RENDER_CACHE_MAX);
      if (isCurrent) commitView({ key, ...record });
    },
    [commitView],
  );

  const resetProcessed = useCallback(() => {
    const key = fileKeyRef.current;
    if (!key) return;
    renderRecords.delete(key);
    commitView({ key, processed: null, chain: EMPTY_CHAIN, version: nextRenderVersion() });
  }, [commitView]);

  const forgetOutputs = useCallback(
    (paths: string[]) => {
      if (paths.length === 0) return;
      for (const [key, record] of [...renderRecords]) {
        const pruned = pruneRecord(record, paths);
        if (pruned === null) renderRecords.delete(key);
        else if (pruned !== record) renderRecords.set(key, pruned);
      }
      const key = fileKeyRef.current;
      if (!key) return;
      const live = viewRef.current;
      const pruned = pruneRecord(live, paths);
      if (pruned === null) {
        resetProcessed();
        return;
      }
      if (pruned !== live) commitView({ ...pruned, key });
    },
    [commitView, resetProcessed],
  );

  const setStfPreviewUrl = useCallback(
    (url: string | null) => {
      if (fileKeyRef.current !== fileKey || viewRef.current.version !== processedVersion) return;
      setStfPreview({ key: fileKey, version: processedVersion, url });
    },
    [fileKey, processedVersion],
  );

  const setOverlay = useCallback((patch: Partial<DqOverlaySettings>) => {
    setOverlayRaw((prev) => ({ ...prev, ...patch }));
  }, []);

  const setSelectedPalette = useCallback((p: string) => {
    setSelectedPaletteRaw(p);
    narrowbandKeyRef.current = "";
  }, []);

  const setDisplay = useCallback((patch: Partial<DisplaySettings>) => {
    setDisplayRaw((prev) => {
      const next = { ...prev, ...patch };
      saveDisplaySettings(next);
      return next;
    });
  }, []);

  const filePath = file?.path ?? null;
  const {
    stretch: displayStretch,
    limits: displayLimits,
    percentileLow,
    percentileHigh,
    zscaleContrast,
    userLo,
    userHi,
    colormap: displayColormap,
  } = display;

  useEffect(() => {
    const seq = ++limitsSeqRef.current;
    if (!filePath || displayStretch === "mtf") {
      setLimitsLoading(false);
      setLimitsError(null);
      return;
    }
    setLimitsLoading(true);
    const timer = window.setTimeout(() => {
      computeScaleLimits(processedPath ?? filePath, {
        ...DEFAULT_DISPLAY_SETTINGS,
        limits: displayLimits,
        percentileLow,
        percentileHigh,
        zscaleContrast,
        userLo,
        userHi,
      })
        .then((res) => {
          if (limitsSeqRef.current !== seq) return;
          setLimits(res);
          setLimitsError(null);
        })
        .catch((err) => {
          if (limitsSeqRef.current !== seq) return;
          console.error("[AstroBurst] Scale limits failed:", err);
          setLimitsError(err instanceof Error ? err.message : String(err));
        })
        .finally(() => {
          if (limitsSeqRef.current !== seq) return;
          setLimitsLoading(false);
        });
    }, LIMITS_DEBOUNCE_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [filePath, processedPath, sourceVersion, displayStretch, displayLimits, percentileLow, percentileHigh, zscaleContrast, userLo, userHi]);

  useEffect(() => {
    let cancelled = false;
    getColormapLut(displayColormap)
      .then((rgba) => {
        if (cancelled) return;
        setLut(rgba);
      })
      .catch((err) => {
        if (cancelled) return;
        console.warn("[AstroBurst] Colormap LUT fetch failed, using gray:", err);
        setLut(GRAY_LUT_RGBA);
      });
    return () => {
      cancelled = true;
    };
  }, [displayColormap]);

  useEffect(() => {
    if (doneFiles.length < 2) return;
    const paths = doneFiles.map((f) => f.path);
    const key = paths.join("|") + "|" + selectedPalette;
    if (key === narrowbandKeyRef.current) return;
    let timer = 0;
    const attempt = () => {
      if (fileStore.getIsProcessing()) {
        timer = window.setTimeout(attempt, 400);
        return;
      }
      const seq = ++narrowbandSeqRef.current;
      detectNarrowbandFilters(paths, selectedPalette)
        .then((result) => {
          if (narrowbandSeqRef.current !== seq) return;
          narrowbandKeyRef.current = key;
          if (result?.palette) setNarrowbandPalette(result.palette);
          if (result?.filters) setNarrowbandFilters(result.filters);
        })
        .catch(() => {});
    };
    timer = window.setTimeout(attempt, 250);
    return () => window.clearTimeout(timer);
  }, [doneFiles, selectedPalette]);

  const loadRawPixels = useCallback((force = false) => {
    const path = processedRef.current?.fitsPath ?? filePathRef.current;
    if (!path) return;
    if (!force && (rawPixelsRef.current || rawPixelsLoadingRef.current)) return;
    setRawPixelsLoading(true);
    setRawPixelsError(null);
    const seq = ++rawPixelsAbortRef.current;
    const maxDim = computePreviewMaxDim();
    lastFetchMaxDimRef.current = maxDim;
    getRawPixelsPreview(path, maxDim)
      .then((result) => {
        if (rawPixelsAbortRef.current !== seq) return;
        setRawPixels({
          data: result.pixels,
          width: result.width,
          height: result.height,
          min: result.dataMin,
          max: result.dataMax,
        });
      })
      .catch((err) => {
        if (rawPixelsAbortRef.current !== seq) return;
        console.error("[AstroBurst] Raw pixels load failed:", err);
        setRawPixelsError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (rawPixelsAbortRef.current !== seq) return;
        setRawPixelsLoading(false);
      });
  }, []);

  const clearRawPixels = useCallback(() => {
    rawPixelsAbortRef.current++;
    setRawPixels(null);
    setRawPixelsLoading(false);
    setRawPixelsError(null);
  }, []);

  const loadRgbRawPixels = useCallback((source: string | null, force = false) => {
    if (!force && (rgbRawPixelsRef.current || rgbRawPixelsLoadingRef.current)) return;
    setRgbRawPixelsLoading(true);
    const seq = ++rgbRawPixelsAbortRef.current;
    const maxDim = computePreviewMaxDim();
    lastFetchMaxDimRef.current = maxDim;
    rgbSourceRef.current = source;
    getRawRgbPixelsPreview(source, maxDim)
      .then((result) => {
        if (rgbRawPixelsAbortRef.current !== seq) return;
        setRgbRawPixels(result);
      })
      .catch((err) => {
        if (rgbRawPixelsAbortRef.current !== seq) return;
        console.error("[AstroBurst] RGB raw pixels load failed:", err);
      })
      .finally(() => {
        if (rgbRawPixelsAbortRef.current !== seq) return;
        setRgbRawPixelsLoading(false);
      });
  }, []);

  const clearRgbRawPixels = useCallback(() => {
    rgbRawPixelsAbortRef.current++;
    setRgbRawPixels(null);
    setRgbRawPixelsLoading(false);
  }, []);

  useEffect(() => {
    let timer = 0;
    const onResize = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const prev = lastFetchMaxDimRef.current;
        if (prev === 0 || prev >= PREVIEW_MAX_DIM_CAP) return;
        const next = computePreviewMaxDim();
        if (next <= prev * 1.25) return;
        if (rawPixelsRef.current) {
          loadRawPixels(true);
          setDqMaskRefetch((c) => c + 1);
        }
        if (rgbRawPixelsRef.current) loadRgbRawPixels(rgbSourceRef.current, true);
      }, 300);
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      window.clearTimeout(timer);
    };
  }, [loadRawPixels, loadRgbRawPixels]);

  useEffect(() => {
    if (!file?.path || fileKey === prevFileIdRef.current) return;
    prevFileIdRef.current = fileKey;

    setHistData(null);
    setFlagTable(null);
    setOverlayRaw(DEFAULT_DQ_OVERLAY);
    setDqMask(null);
    setDqMaskLoading(false);
    setDqMaskError(null);
    dqMaskKeyRef.current = "";
    dqMaskSeqRef.current++;
    flagTableSeqRef.current++;
    histSeqRef.current++;
    setStfParams(DEFAULT_STF);
    setIsCube(false);
    setIsSpectralCube(false);
    setSpectralReason(null);
    setCubeDims(null);
    setRawPixels(null);
    setRawPixelsLoading(false);
    setRawPixelsError(null);
    setRgbRawPixels(null);
    setRgbRawPixelsLoading(false);
    setNarrowbandPalette(null);
    setLimits(null);
    rawPixelsAbortRef.current++;
    rgbRawPixelsAbortRef.current++;

    composite.resetComposite();

    const seq = ++seqRef.current;
    const stale = () => seqRef.current !== seq;

    const isRgbFits = file.result?.is_rgb === true;

    if (isRgbFits) {
      const toStf = (s: StfParams): StfParams => ({ shadow: s.shadow, midtone: s.midtone, highlight: s.highlight });
      if (file.result?.stf_r && file.result?.stf_g && file.result?.stf_b) {
        composite.initRgb(
          file.result.previewUrl ?? null,
          toStf(file.result.stf_r),
          toStf(file.result.stf_g),
          toStf(file.result.stf_b),
        );
      } else if (file.result?.previewUrl) {
        composite.setCompositePreviewUrl(file.result.previewUrl);
      }
    }

    const precomputedAutoStf = file.result?.histogram?.auto_stf;
    if (excludeDqRef.current && !processedPath && precomputedAutoStf) setStfParams(precomputedAutoStf);

    const naxis3 = file.result?.header?.NAXIS3;
    const n3 = naxis3 ? parseInt(naxis3, 10) : 0;
    if (n3 > 1 && !isRgbFits) {
      setIsCube(true);
      getCubeInfo(file.path)
        .then((info) => {
          if (stale()) return;
          setCubeDims(info);
          if (info?.spectral_classification) {
            setIsSpectralCube(info.spectral_classification.is_spectral || false);
            setSpectralReason(info.spectral_classification.reason || null);
          }
        })
        .catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fileKey]);

  const precomputedHist = file?.result?.histogram ?? null;

  useEffect(() => {
    if (!fileKey || !filePath) return;
    const seq = ++histSeqRef.current;
    const precomputed = processedPath || excludeDq ? null : precomputedHist;
    if (precomputed?.bins) {
      setHistData(precomputed);
      if (precomputed.auto_stf) setStfParams(precomputed.auto_stf);
      return;
    }
    computeHistogram(processedPath ?? filePath, excludeDq)
      .then((data) => {
        if (histSeqRef.current !== seq) return;
        setHistData(data);
        if (data.auto_stf) setStfParams(data.auto_stf);
      })
      .catch((err) => {
        if (histSeqRef.current === seq) console.error("[AstroBurst] Histogram fetch failed:", err);
      });
  }, [fileKey, filePath, processedPath, sourceVersion, excludeDq, precomputedHist]);

  const plane = file?.result?.plane ?? null;
  const dqRef = plane?.dq_ref ?? null;

  useEffect(() => {
    if (!filePath || !dqRef) return;
    const seq = ++flagTableSeqRef.current;
    getDqFlagTable(filePath)
      .then((table) => {
        if (flagTableSeqRef.current !== seq) return;
        setFlagTable(table);
        setOverlayRaw({ enabled: false, mask: table.default_mask });
      })
      .catch((err) => {
        if (flagTableSeqRef.current !== seq) return;
        console.error("[AstroBurst] DQ flag table fetch failed:", err);
      });
  }, [fileKey, filePath, dqRef]);

  const overlayEnabled = overlay.enabled;
  const overlayMask = overlay.mask;

  useEffect(() => {
    if (!filePath || !dqRef || !overlayEnabled) return;
    const maxDim = computePreviewMaxDim();
    const key = `${fileKey}|${overlayMask}|${maxDim}`;
    if (dqMaskKeyRef.current === key) return;
    dqMaskKeyRef.current = key;
    const seq = ++dqMaskSeqRef.current;
    setDqMaskLoading(true);
    setDqMask(null);
    getDqMaskPreview(filePath, overlayMask, maxDim)
      .then((mask) => {
        if (dqMaskSeqRef.current !== seq) return;
        setDqMask(mask);
        setDqMaskError(null);
      })
      .catch((err) => {
        if (dqMaskSeqRef.current !== seq) return;
        dqMaskKeyRef.current = "";
        setDqMaskError(err instanceof Error ? err.message : String(err));
        console.error("[AstroBurst] DQ mask fetch failed:", err);
      })
      .finally(() => {
        if (dqMaskSeqRef.current !== seq) return;
        setDqMaskLoading(false);
      });
  }, [fileKey, filePath, dqRef, overlayEnabled, overlayMask, dqMaskRefetch]);

  const fileValue = useMemo<FileContextValue>(
    () => ({ file }),
    [file],
  );

  const doneFilesValue = useMemo<DoneFilesContextValue>(
    () => ({ doneFiles }),
    [doneFiles],
  );

  const histValue = useMemo<HistContextValue>(
    () => ({ histData, stfParams, setStfParams }),
    [histData, stfParams],
  );

  const cubeValue = useMemo<CubeContextValue>(
    () => ({ isCube, isSpectralCube, spectralReason, cubeDims }),
    [isCube, isSpectralCube, spectralReason, cubeDims],
  );

  const rgbValue = useMemo<RgbContextValue>(
    () => ({ rgbChannels, setRgbChannels }),
    [rgbChannels],
  );

  const renderValue = useMemo<RenderContextValue>(
    () => ({ processed, processedVersion, chain, stfPreviewUrl }),
    [processed, processedVersion, chain, stfPreviewUrl],
  );

  const renderActionsValue = useMemo<RenderActionsContextValue>(
    () => ({ currentFileKey, publishProcessed, setChain, resetProcessed, forgetOutputs, setStfPreviewUrl }),
    [currentFileKey, publishProcessed, setChain, resetProcessed, forgetOutputs, setStfPreviewUrl],
  );

  const rawPixelsValue = useMemo<RawPixelsContextValue>(
    () => ({
      rawPixels, rawPixelsLoading, rawPixelsError, loadRawPixels, clearRawPixels,
      rgbRawPixels, rgbRawPixelsLoading, loadRgbRawPixels, clearRgbRawPixels,
    }),
    [
      rawPixels, rawPixelsLoading, rawPixelsError, loadRawPixels, clearRawPixels,
      rgbRawPixels, rgbRawPixelsLoading, loadRgbRawPixels, clearRgbRawPixels,
    ],
  );

  const narrowbandValue = useMemo<NarrowbandContextValue>(
    () => ({ narrowbandPalette, narrowbandFilters, selectedPalette, setSelectedPalette }),
    [narrowbandPalette, narrowbandFilters, selectedPalette, setSelectedPalette],
  );

  const starOverlayValue = useMemo<StarOverlayContextValue>(
    () => ({ starOverlayRef }),
    [],
  );

  const displayValue = useMemo<DisplayContextValue>(
    () => ({ display, setDisplay, limits, lut, limitsLoading, limitsError }),
    [display, setDisplay, limits, lut, limitsLoading, limitsError],
  );

  const dqValue = useMemo<DqContextValue>(
    () => ({ plane, flagTable, overlay, setOverlay, excludeDq, setExcludeDq, dqMask, dqMaskLoading, dqMaskError }),
    [plane, flagTable, overlay, setOverlay, excludeDq, dqMask, dqMaskLoading, dqMaskError],
  );

  return (
    <FileCtx.Provider value={fileValue}>
      <DoneFilesCtx.Provider value={doneFilesValue}>
        <HistCtx.Provider value={histValue}>
          <CubeCtx.Provider value={cubeValue}>
            <RgbCtx.Provider value={rgbValue}>
              <RenderCtx.Provider value={renderValue}>
                <RenderActionsCtx.Provider value={renderActionsValue}>
                <RawPixelsCtx.Provider value={rawPixelsValue}>
                  <NarrowbandCtx.Provider value={narrowbandValue}>
                    <StarOverlayCtx.Provider value={starOverlayValue}>
                      <DisplayCtx.Provider value={displayValue}>
                        <DqCtx.Provider value={dqValue}>
                          {children}
                        </DqCtx.Provider>
                      </DisplayCtx.Provider>
                    </StarOverlayCtx.Provider>
                  </NarrowbandCtx.Provider>
                </RawPixelsCtx.Provider>
                </RenderActionsCtx.Provider>
              </RenderCtx.Provider>
            </RgbCtx.Provider>
          </CubeCtx.Provider>
        </HistCtx.Provider>
      </DoneFilesCtx.Provider>
    </FileCtx.Provider>
  );
}
