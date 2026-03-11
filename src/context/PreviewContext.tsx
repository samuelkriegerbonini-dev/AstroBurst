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
import { useBackend } from "../hooks/useBackend";
import type { ProcessedFile, StfParams, HistogramData, RawPixelData } from "../utils/types";

interface ChannelSuggestion {
  file_path: string;
  file_name: string;
  detection: { filter_name: string; method: string; confidence: number } | null;
}

interface PaletteSuggestion {
  r_file: ChannelSuggestion | null;
  g_file: ChannelSuggestion | null;
  b_file: ChannelSuggestion | null;
  unmapped: ChannelSuggestion[];
  is_complete: boolean;
  palette_name: string;
}

interface FileContextValue {
  file: ProcessedFile | null;
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
  cubeDims: any;
}

interface RgbContextValue {
  rgbChannels: { r: string | null; g: string | null; b: string | null } | null;
  setRgbChannels: React.Dispatch<React.SetStateAction<any>>;
}

interface RenderContextValue {
  renderedPreviewUrl: string | null;
  setRenderedPreviewUrl: (url: string | null) => void;
}

interface RawPixelsContextValue {
  rawPixels: RawPixelData | null;
  rawPixelsLoading: boolean;
  loadRawPixels: () => void;
  clearRawPixels: () => void;
}

interface NarrowbandContextValue {
  narrowbandPalette: PaletteSuggestion | null;
}

const FileCtx = createContext<FileContextValue | null>(null);
const HistCtx = createContext<HistContextValue | null>(null);
const CubeCtx = createContext<CubeContextValue | null>(null);
const RgbCtx = createContext<RgbContextValue | null>(null);
const RenderCtx = createContext<RenderContextValue | null>(null);
const RawPixelsCtx = createContext<RawPixelsContextValue | null>(null);
const NarrowbandCtx = createContext<NarrowbandContextValue | null>(null);

export function usePreviewContext() {
  const fileCtx = useContext(FileCtx);
  const histCtx = useContext(HistCtx);
  const cubeCtx = useContext(CubeCtx);
  const rgbCtx = useContext(RgbCtx);
  const renderCtx = useContext(RenderCtx);
  const rawPixelsCtx = useContext(RawPixelsCtx);
  const narrowbandCtx = useContext(NarrowbandCtx);
  if (!fileCtx) throw new Error("usePreviewContext must be used within PreviewProvider");
  return {
    ...fileCtx!,
    ...histCtx!,
    ...cubeCtx!,
    ...rgbCtx!,
    ...renderCtx!,
    ...rawPixelsCtx!,
    ...narrowbandCtx!,
  };
}

export function useFileContext() {
  const ctx = useContext(FileCtx);
  if (!ctx) throw new Error("useFileContext outside PreviewProvider");
  return ctx;
}

export function useHistContext() {
  const ctx = useContext(HistCtx);
  if (!ctx) throw new Error("useHistContext outside PreviewProvider");
  return ctx;
}

export function useCubeContext() {
  const ctx = useContext(CubeCtx);
  if (!ctx) throw new Error("useCubeContext outside PreviewProvider");
  return ctx;
}

export function useRgbContext() {
  const ctx = useContext(RgbCtx);
  if (!ctx) throw new Error("useRgbContext outside PreviewProvider");
  return ctx;
}

export function useRenderContext() {
  const ctx = useContext(RenderCtx);
  if (!ctx) throw new Error("useRenderContext outside PreviewProvider");
  return ctx;
}

export function useRawPixelsContext() {
  const ctx = useContext(RawPixelsCtx);
  if (!ctx) throw new Error("useRawPixelsContext outside PreviewProvider");
  return ctx;
}

export function useNarrowbandContext() {
  const ctx = useContext(NarrowbandCtx);
  if (!ctx) throw new Error("useNarrowbandContext outside PreviewProvider");
  return ctx;
}

interface Props {
  file: ProcessedFile | null;
  allFiles: ProcessedFile[];
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

export function PreviewProvider({ file, allFiles, children }: Props) {
  const { computeHistogram, getCubeInfo, getRawPixelsPreview, detectNarrowbandFilters } = useBackend();

  const [histData, setHistData] = useState<HistogramData | null>(null);
  const [stfParams, setStfParams] = useState<StfParams>({
    shadow: 0,
    midtone: 0.5,
    highlight: 1,
  });
  const [isCube, setIsCube] = useState(false);
  const [isSpectralCube, setIsSpectralCube] = useState(false);
  const [spectralReason, setSpectralReason] = useState<string | null>(null);
  const [cubeDims, setCubeDims] = useState<any>(null);
  const [rgbChannels, setRgbChannels] = useState<any>(null);
  const [renderedPreviewUrl, setRenderedPreviewUrlRaw] = useState<string | null>(null);
  const [rawPixels, setRawPixels] = useState<RawPixelData | null>(null);
  const [rawPixelsLoading, setRawPixelsLoading] = useState(false);
  const [narrowbandPalette, setNarrowbandPalette] = useState<PaletteSuggestion | null>(null);

  const prevFileIdRef = useRef<string | null>(null);
  const histAbortRef = useRef(0);
  const rawPixelsAbortRef = useRef(0);
  const narrowbandDetectedRef = useRef(0);

  const setRenderedPreviewUrl = useCallback(
    (url: string | null) => {
      setRenderedPreviewUrlRaw(url);
      if (url && file?.id) {
        setPreviewCache(file.id, url);
      }
    },
    [file?.id],
  );

  const doneFiles = useMemo(() => {
    if (!allFiles) return [];
    return allFiles.filter((f) => f.status === "done");
  }, [allFiles]);

  useEffect(() => {
    if (doneFiles.length < 2) return;
    const paths = doneFiles.map((f) => f.path);
    const key = paths.length;
    if (key === narrowbandDetectedRef.current) return;
    narrowbandDetectedRef.current = key;
    detectNarrowbandFilters(paths)
      .then((result: any) => {
        if (result?.palette) {
          setNarrowbandPalette(result.palette);
        }
      })
      .catch(() => {});
  }, [doneFiles, detectNarrowbandFilters]);

  const loadRawPixels = useCallback(() => {
    if (!file?.path || rawPixels || rawPixelsLoading) return;
    setRawPixelsLoading(true);
    const seq = ++rawPixelsAbortRef.current;
    const maxDim = Math.min(window.innerWidth, window.innerHeight, 2048);
    getRawPixelsPreview(file.path, maxDim)
      .then((result: any) => {
        if (rawPixelsAbortRef.current !== seq) return;
        setRawPixels({
          data: result.pixels,
          width: result.width,
          height: result.height,
          min: result.dataMin,
          max: result.dataMax,
        });
      })
      .catch((err: any) => {
        if (rawPixelsAbortRef.current !== seq) return;
        console.error("[AstroBurst] Raw pixels load failed:", err);
      })
      .finally(() => {
        if (rawPixelsAbortRef.current !== seq) return;
        setRawPixelsLoading(false);
      });
  }, [file?.path, rawPixels, rawPixelsLoading, getRawPixelsPreview]);

  const clearRawPixels = useCallback(() => {
    rawPixelsAbortRef.current++;
    setRawPixels(null);
    setRawPixelsLoading(false);
  }, []);

  useEffect(() => {
    if (!file || !file.path || file.id === prevFileIdRef.current) return;
    prevFileIdRef.current = file.id;

    setHistData(null);
    setStfParams({ shadow: 0, midtone: 0.5, highlight: 1 });
    setIsCube(false);
    setIsSpectralCube(false);
    setSpectralReason(null);
    setCubeDims(null);
    setRgbChannels(null);
    setRawPixels(null);
    setRawPixelsLoading(false);
    setNarrowbandPalette(null);
    rawPixelsAbortRef.current++;

    const cachedUrl = previewUrlCache.get(file.id);
    setRenderedPreviewUrlRaw(cachedUrl || null);

    const seq = ++histAbortRef.current;

    const precomputedHist = file.result?.histogram;
    if (precomputedHist && precomputedHist.bins) {
      setHistData(precomputedHist);
      if (precomputedHist.auto_stf) setStfParams(precomputedHist.auto_stf);
    } else {
      computeHistogram(file.path)
        .then((data: any) => {
          if (histAbortRef.current !== seq) return;
          setHistData(data);
          if (data.auto_stf) setStfParams(data.auto_stf);
        })
        .catch((err: any) => {
          if (histAbortRef.current !== seq) return;
          console.error("Histogram fetch failed:", err);
        });
    }

    const naxis3 = file.result?.header?.NAXIS3;
    const n3 = naxis3 ? parseInt(naxis3, 10) : 0;
    if (n3 > 1 ) {
      setIsCube(true);
      getCubeInfo(file.path)
        .then((info: any) => {
          if (histAbortRef.current !== seq) return;
          setCubeDims(info);
          if (info?.spectral_classification) {
            setIsSpectralCube(info.spectral_classification.is_spectral || false);
            setSpectralReason(info.spectral_classification.reason || null);
          }
        })
        .catch(() => {});
    }
  }, [file?.id]);

  const fileValue = useMemo<FileContextValue>(
    () => ({ file, doneFiles }),
    [file, doneFiles],
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
    () => ({ renderedPreviewUrl, setRenderedPreviewUrl }),
    [renderedPreviewUrl, setRenderedPreviewUrl],
  );

  const rawPixelsValue = useMemo<RawPixelsContextValue>(
    () => ({ rawPixels, rawPixelsLoading, loadRawPixels, clearRawPixels }),
    [rawPixels, rawPixelsLoading, loadRawPixels, clearRawPixels],
  );

  const narrowbandValue = useMemo<NarrowbandContextValue>(
    () => ({ narrowbandPalette }),
    [narrowbandPalette],
  );

  return (
    <FileCtx.Provider value={fileValue}>
      <HistCtx.Provider value={histValue}>
        <CubeCtx.Provider value={cubeValue}>
          <RgbCtx.Provider value={rgbValue}>
            <RenderCtx.Provider value={renderValue}>
              <RawPixelsCtx.Provider value={rawPixelsValue}>
                <NarrowbandCtx.Provider value={narrowbandValue}>
                  {children}
                </NarrowbandCtx.Provider>
              </RawPixelsCtx.Provider>
            </RenderCtx.Provider>
          </RgbCtx.Provider>
        </CubeCtx.Provider>
      </HistCtx.Provider>
    </FileCtx.Provider>
  );
}

export type { PaletteSuggestion, ChannelSuggestion };
