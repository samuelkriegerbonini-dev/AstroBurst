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
import { computeHistogram } from "../services/analysis.service";
import { getCubeInfo } from "../services/cube.service";
import { getRawPixelsPreview } from "../services/fits.service";
import { detectNarrowbandFilters } from "../services/header.service";
import { clearCompositeCache } from "../services/compose.service";
import type { ProcessedFile, StfParams, HistogramData, RawPixelData } from "../shared/types";

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
}

interface ScnrState {
  enabled: boolean;
  method: string;
  amount: number;
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
  cubeDims: any;
}

interface RgbContextValue {
  rgbChannels: { r: string | null; g: string | null; b: string | null } | null;
  setRgbChannels: React.Dispatch<React.SetStateAction<any>>;
  lastAlignMethod: string | null;
  setLastAlignMethod: (method: string | null) => void;
}

interface RenderContextValue {
  renderedPreviewUrl: string | null;
  setRenderedPreviewUrl: (url: string | null) => void;
  activeImagePath: string | null;
  setActiveImagePath: (path: string | null) => void;
  compositePreviewUrl: string | null;
  setCompositePreviewUrl: (url: string | null) => void;
  isShowingComposite: boolean;
  clearComposite: () => Promise<void>;
  compositeStfR: StfParams;
  compositeStfG: StfParams;
  compositeStfB: StfParams;
  setCompositeStf: (r: StfParams, g: StfParams, b: StfParams) => void;
  compositeStfLinked: boolean;
  setCompositeStfLinked: (linked: boolean) => void;
  compositeAutoStfR: StfParams | null;
  compositeAutoStfG: StfParams | null;
  compositeAutoStfB: StfParams | null;
  setCompositeAutoStf: (r: StfParams, g: StfParams, b: StfParams) => void;
  compositeScnr: ScnrState | null;
  setCompositeScnr: (scnr: ScnrState | null) => void;
}

interface RawPixelsContextValue {
  rawPixels: RawPixelData | null;
  rawPixelsLoading: boolean;
  loadRawPixels: () => void;
  clearRawPixels: () => void;
}

interface NarrowbandContextValue {
  narrowbandPalette: PaletteSuggestion | null;
  selectedPalette: string;
  setSelectedPalette: (p: string) => void;
}

const FileCtx = createContext<FileContextValue | null>(null);
const DoneFilesCtx = createContext<DoneFilesContextValue | null>(null);
const HistCtx = createContext<HistContextValue | null>(null);
const CubeCtx = createContext<CubeContextValue | null>(null);
const RgbCtx = createContext<RgbContextValue | null>(null);
const RenderCtx = createContext<RenderContextValue | null>(null);
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
export const useRawPixelsContext = () => useCtx(RawPixelsCtx, "useRawPixelsContext");
export const useNarrowbandContext = () => useCtx(NarrowbandCtx, "useNarrowbandContext");

export function usePreviewContext() {
  return {
    ...useFileContext(),
    ...useDoneFilesContext(),
    ...useHistContext(),
    ...useCubeContext(),
    ...useRgbContext(),
    ...useRenderContext(),
    ...useRawPixelsContext(),
    ...useNarrowbandContext(),
  };
}

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

export function PreviewProvider({ file, doneFiles, children }: Props) {

  const [histData, setHistData] = useState<HistogramData | null>(null);
  const [stfParams, setStfParams] = useState<StfParams>(DEFAULT_STF);
  const [isCube, setIsCube] = useState(false);
  const [isSpectralCube, setIsSpectralCube] = useState(false);
  const [spectralReason, setSpectralReason] = useState<string | null>(null);
  const [cubeDims, setCubeDims] = useState<any>(null);
  const [rgbChannels, setRgbChannels] = useState<any>(null);
  const [lastAlignMethod, setLastAlignMethod] = useState<string | null>(null);
  const [renderedPreviewUrl, setRenderedPreviewUrlRaw] = useState<string | null>(null);
  const [activeImagePath, setActiveImagePathRaw] = useState<string | null>(null);
  const [compositePreviewUrl, setCompositePreviewUrlRaw] = useState<string | null>(null);
  const [rawPixels, setRawPixels] = useState<RawPixelData | null>(null);
  const [rawPixelsLoading, setRawPixelsLoading] = useState(false);
  const [narrowbandPalette, setNarrowbandPalette] = useState<PaletteSuggestion | null>(null);
  const [selectedPalette, setSelectedPaletteRaw] = useState("SHO");

  const [compositeStfR, setCompositeStfR] = useState<StfParams>(DEFAULT_STF);
  const [compositeStfG, setCompositeStfG] = useState<StfParams>(DEFAULT_STF);
  const [compositeStfB, setCompositeStfB] = useState<StfParams>(DEFAULT_STF);
  const [compositeStfLinked, setCompositeStfLinked] = useState(true);
  const [compositeAutoStfR, setCompositeAutoStfR] = useState<StfParams | null>(null);
  const [compositeAutoStfG, setCompositeAutoStfG] = useState<StfParams | null>(null);
  const [compositeAutoStfB, setCompositeAutoStfB] = useState<StfParams | null>(null);
  const [compositeScnr, setCompositeScnrRaw] = useState<ScnrState | null>(null);

  const prevFileIdRef = useRef<string | null>(null);
  const seqRef = useRef(0);
  const rawPixelsAbortRef = useRef(0);
  const narrowbandKeyRef = useRef("");

  const rawPixelsRef = useRef(rawPixels);
  rawPixelsRef.current = rawPixels;
  const rawPixelsLoadingRef = useRef(rawPixelsLoading);
  rawPixelsLoadingRef.current = rawPixelsLoading;
  const filePathRef = useRef(file?.path);
  filePathRef.current = file?.path;

  const setRenderedPreviewUrl = useCallback(
    (url: string | null) => {
      setRenderedPreviewUrlRaw(url);
      if (url && file?.id) setPreviewCache(file.id, url);
    },
    [file?.id],
  );

  const setActiveImagePath = useCallback(
    (path: string | null) => {
      setActiveImagePathRaw(path);
    },
    [],
  );

  const setCompositePreviewUrl = useCallback(
    (url: string | null) => {
      setCompositePreviewUrlRaw(url);
    },
    [],
  );

  const clearComposite = useCallback(async () => {
    setCompositePreviewUrlRaw(null);
    setCompositeStfR(DEFAULT_STF);
    setCompositeStfG(DEFAULT_STF);
    setCompositeStfB(DEFAULT_STF);
    setCompositeAutoStfR(null);
    setCompositeAutoStfG(null);
    setCompositeAutoStfB(null);
    setCompositeScnrRaw(null);
    await clearCompositeCache().catch(() => {});
  }, []);

  const isShowingComposite = compositePreviewUrl !== null;

  const setCompositeStf = useCallback((r: StfParams, g: StfParams, b: StfParams) => {
    setCompositeStfR(r);
    setCompositeStfG(g);
    setCompositeStfB(b);
  }, []);

  const setCompositeAutoStf = useCallback((r: StfParams, g: StfParams, b: StfParams) => {
    setCompositeAutoStfR(r);
    setCompositeAutoStfG(g);
    setCompositeAutoStfB(b);
  }, []);

  const setCompositeScnr = useCallback((scnr: ScnrState | null) => {
    setCompositeScnrRaw(scnr);
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
    narrowbandKeyRef.current = key;
    detectNarrowbandFilters(paths, selectedPalette)
      .then((result: any) => {
        if (result?.palette) setNarrowbandPalette(result.palette);
      })
      .catch(() => {});
  }, [doneFiles, selectedPalette]);

  const loadRawPixels = useCallback(() => {
    const path = filePathRef.current;
    if (!path || rawPixelsRef.current || rawPixelsLoadingRef.current) return;
    setRawPixelsLoading(true);
    const seq = ++rawPixelsAbortRef.current;
    const maxDim = Math.min(window.innerWidth, window.innerHeight, 2048);
    getRawPixelsPreview(path, maxDim)
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
  }, []);

  const clearRawPixels = useCallback(() => {
    rawPixelsAbortRef.current++;
    setRawPixels(null);
    setRawPixelsLoading(false);
  }, []);

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
    setNarrowbandPalette(null);
    setActiveImagePathRaw(null);
    setCompositePreviewUrlRaw(null);
    setCompositeStfR(DEFAULT_STF);
    setCompositeStfG(DEFAULT_STF);
    setCompositeStfB(DEFAULT_STF);
    setCompositeAutoStfR(null);
    setCompositeAutoStfG(null);
    setCompositeAutoStfB(null);
    setCompositeScnrRaw(null);
    rawPixelsAbortRef.current++;

    if (!file.result?.is_rgb) {
      clearCompositeCache().catch(() => {});
    }

    setRenderedPreviewUrlRaw(previewUrlCache.get(file.id) ?? null);

    const seq = ++seqRef.current;
    const stale = () => seqRef.current !== seq;

    const isRgbFits = file.result?.is_rgb === true;

    if (isRgbFits) {
      if (file.result?.previewUrl) {
        setCompositePreviewUrlRaw(file.result.previewUrl);
      }
      if (file.result?.stf_r && file.result?.stf_g && file.result?.stf_b) {
        const toStf = (s: any) => ({ shadow: s.shadow, midtone: s.midtone, highlight: s.highlight });
        setCompositeAutoStfR(toStf(file.result.stf_r));
        setCompositeAutoStfG(toStf(file.result.stf_g));
        setCompositeAutoStfB(toStf(file.result.stf_b));
        setCompositeStfR(toStf(file.result.stf_r));
        setCompositeStfG(toStf(file.result.stf_g));
        setCompositeStfB(toStf(file.result.stf_b));
      }
    }

    const precomputedHist = file.result?.histogram;
    if (precomputedHist?.bins) {
      setHistData(precomputedHist);
      if (precomputedHist.auto_stf) setStfParams(precomputedHist.auto_stf);
    } else {
      computeHistogram(file.path)
        .then((data: any) => {
          if (stale()) return;
          setHistData(data);
          if (data.auto_stf) setStfParams(data.auto_stf);
        })
        .catch((err: any) => {
          if (!stale()) console.error("Histogram fetch failed:", err);
        });
    }

    const naxis3 = file.result?.header?.NAXIS3;
    const n3 = naxis3 ? parseInt(naxis3, 10) : 0;
    if (n3 > 1 && !isRgbFits) {
      setIsCube(true);
      getCubeInfo(file.path)
        .then((info: any) => {
          if (stale()) return;
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
      renderedPreviewUrl, setRenderedPreviewUrl, activeImagePath, setActiveImagePath,
      compositePreviewUrl, setCompositePreviewUrl, isShowingComposite, clearComposite,
      compositeStfR, compositeStfG, compositeStfB, setCompositeStf,
      compositeStfLinked, setCompositeStfLinked,
      compositeAutoStfR, compositeAutoStfG, compositeAutoStfB, setCompositeAutoStf,
      compositeScnr, setCompositeScnr,
    }),
    [renderedPreviewUrl, setRenderedPreviewUrl, activeImagePath, setActiveImagePath,
     compositePreviewUrl, setCompositePreviewUrl, isShowingComposite, clearComposite,
     compositeStfR, compositeStfG, compositeStfB, setCompositeStf,
     compositeStfLinked, setCompositeStfLinked,
     compositeAutoStfR, compositeAutoStfG, compositeAutoStfB, setCompositeAutoStf,
     compositeScnr, setCompositeScnr],
  );

  const rawPixelsValue = useMemo<RawPixelsContextValue>(
    () => ({ rawPixels, rawPixelsLoading, loadRawPixels, clearRawPixels }),
    [rawPixels, rawPixelsLoading, loadRawPixels, clearRawPixels],
  );

  const narrowbandValue = useMemo<NarrowbandContextValue>(
    () => ({ narrowbandPalette, selectedPalette, setSelectedPalette }),
    [narrowbandPalette, selectedPalette, setSelectedPalette],
  );

  return (
    <FileCtx.Provider value={fileValue}>
      <DoneFilesCtx.Provider value={doneFilesValue}>
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
      </DoneFilesCtx.Provider>
    </FileCtx.Provider>
  );
}

export type { PaletteSuggestion, ChannelSuggestion, ScnrState };
