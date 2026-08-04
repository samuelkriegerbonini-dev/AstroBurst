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
import type { NarrowbandFilterDetection } from "../services/header";
import { fileStore } from "../hooks/useFileStore";
import { useCompositeActions } from "./CompositeContext";
import { clearCompositeCache } from "../services/compose";
import type { ProcessedFile, StfParams, HistogramData, RawPixelData, RawRgbPixelData } from "../shared/types";
import type { CubeDims } from "../shared/types/cube";

export interface ChannelSuggestion {
  file_path: string;
  file_name: string;
  detection: { filter_name: string; method: string; confidence: number } | null;
}

export interface PaletteSuggestion {
  r_file: ChannelSuggestion | null;
  g_file: ChannelSuggestion | null;
  b_file: ChannelSuggestion | null;
  unmapped: ChannelSuggestion[];
  is_complete: boolean;
  palette_name: string;
}

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
  lastAlignMethod: string | null;
  setLastAlignMethod: (method: string | null) => void;
}

interface RenderContextValue {
  renderedPreviewUrl: string | null;
  setRenderedPreviewUrl: (url: string | null) => void;
  activeImagePath: string | null;
  setActiveImagePath: (path: string | null) => void;
}

interface RenderActionsContextValue {
  setRenderedPreviewUrl: (url: string | null) => void;
  setActiveImagePath: (path: string | null) => void;
}

interface StarOverlayContextValue {
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
}

interface RawPixelsContextValue {
  rawPixels: RawPixelData | null;
  rawPixelsLoading: boolean;
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

const FileCtx = createContext<FileContextValue | null>(null);
const DoneFilesCtx = createContext<DoneFilesContextValue | null>(null);
const HistCtx = createContext<HistContextValue | null>(null);
const CubeCtx = createContext<CubeContextValue | null>(null);
const RgbCtx = createContext<RgbContextValue | null>(null);
const RenderCtx = createContext<RenderContextValue | null>(null);
const RenderActionsCtx = createContext<RenderActionsContextValue | null>(null);
const StarOverlayCtx = createContext<StarOverlayContextValue | null>(null);
const RawPixelsCtx = createContext<RawPixelsContextValue | null>(null);
const NarrowbandCtx = createContext<NarrowbandContextValue | null>(null);

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

interface Props {
  file: ProcessedFile | null;
  doneFiles: ProcessedFile[];
  children: React.ReactNode;
}

const PREVIEW_CACHE_MAX = 50;
const previewUrlCache = new Map<string, string>();

function setPreviewCache(key: string, value: string) {
  if (previewUrlCache.size >= PREVIEW_CACHE_MAX) {
    const first = previewUrlCache.keys().next().value;
    if (first !== undefined) previewUrlCache.delete(first);
  }
  previewUrlCache.set(key, value);
}

const DEFAULT_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };

const PREVIEW_MAX_DIM_CAP = 2048;

function computePreviewMaxDim(): number {
  const dpr = window.devicePixelRatio || 1;
  return Math.min(Math.round(Math.max(window.innerWidth, window.innerHeight) * dpr), PREVIEW_MAX_DIM_CAP);
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
  const [lastAlignMethod, setLastAlignMethod] = useState<string | null>(null);
  const [renderedPreviewUrl, setRenderedPreviewUrlRaw] = useState<string | null>(null);
  const [activeImagePath, setActiveImagePathRaw] = useState<string | null>(null);
  const [rawPixels, setRawPixels] = useState<RawPixelData | null>(null);
  const [rawPixelsLoading, setRawPixelsLoading] = useState(false);
  const [rgbRawPixels, setRgbRawPixels] = useState<RawRgbPixelData | null>(null);
  const [rgbRawPixelsLoading, setRgbRawPixelsLoading] = useState(false);
  const [narrowbandPalette, setNarrowbandPalette] = useState<PaletteSuggestion | null>(null);
  const [narrowbandFilters, setNarrowbandFilters] = useState<NarrowbandFilterDetection[]>([]);
  const [selectedPalette, setSelectedPaletteRaw] = useState("SHO");

  const prevFileIdRef = useRef<string | null>(null);
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

  const setRenderedPreviewUrl = useCallback(
    (url: string | null) => {
      const creatingId = file?.id ?? null;
      if (prevFileIdRef.current !== creatingId) return;
      setRenderedPreviewUrlRaw(url);
      if (url && creatingId && !url.includes("cube_frame_")) setPreviewCache(creatingId, url);
    },
    [file?.id],
  );

  const setActiveImagePath = useCallback((path: string | null) => {
    setActiveImagePathRaw(path);
  }, []);

  const setSelectedPalette = useCallback((p: string) => {
    setSelectedPaletteRaw(p);
    narrowbandKeyRef.current = "";
  }, []);

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
    const path = filePathRef.current;
    if (!path) return;
    if (!force && (rawPixelsRef.current || rawPixelsLoadingRef.current)) return;
    setRawPixelsLoading(true);
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
        if (rawPixelsRef.current) loadRawPixels(true);
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
    if (!file?.path || file.id === prevFileIdRef.current) return;
    prevFileIdRef.current = file.id;

    setHistData(null);
    setStfParams(DEFAULT_STF);
    setIsCube(false);
    setIsSpectralCube(false);
    setSpectralReason(null);
    setCubeDims(null);
    setRgbChannels(null);
    setLastAlignMethod(null);
    setRawPixels(null);
    setRawPixelsLoading(false);
    setRgbRawPixels(null);
    setRgbRawPixelsLoading(false);
    setNarrowbandPalette(null);
    setActiveImagePathRaw(null);
    rawPixelsAbortRef.current++;
    rgbRawPixelsAbortRef.current++;

    composite.resetComposite();

    if (!file.result?.is_rgb) {
      clearCompositeCache().catch(() => {});
    }

    setRenderedPreviewUrlRaw(previewUrlCache.get(file.id) ?? null);

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

    const precomputedHist = file.result?.histogram;
    if (precomputedHist?.bins) {
      setHistData(precomputedHist);
      if (precomputedHist.auto_stf) setStfParams(precomputedHist.auto_stf);
    } else {
      computeHistogram(file.path)
        .then((data) => {
          if (stale()) return;
          setHistData(data);
          if (data.auto_stf) setStfParams(data.auto_stf);
        })
        .catch((err) => {
          if (!stale()) console.error("Histogram fetch failed:", err);
        });
    }

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
  }, [file?.id]);

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
    () => ({ rgbChannels, setRgbChannels, lastAlignMethod, setLastAlignMethod }),
    [rgbChannels, lastAlignMethod],
  );

  const renderValue = useMemo<RenderContextValue>(
    () => ({
      renderedPreviewUrl, setRenderedPreviewUrl,
      activeImagePath, setActiveImagePath,
    }),
    [renderedPreviewUrl, setRenderedPreviewUrl, activeImagePath, setActiveImagePath],
  );

  const renderActionsValue = useMemo<RenderActionsContextValue>(
    () => ({ setRenderedPreviewUrl, setActiveImagePath }),
    [setRenderedPreviewUrl, setActiveImagePath],
  );

  const rawPixelsValue = useMemo<RawPixelsContextValue>(
    () => ({
      rawPixels, rawPixelsLoading, loadRawPixels, clearRawPixels,
      rgbRawPixels, rgbRawPixelsLoading, loadRgbRawPixels, clearRgbRawPixels,
    }),
    [
      rawPixels, rawPixelsLoading, loadRawPixels, clearRawPixels,
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
                      {children}
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


