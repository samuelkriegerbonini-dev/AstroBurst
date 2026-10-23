import { useState, useCallback, useEffect, useId, useMemo, useRef, memo } from "react";
import { Crosshair, Star as StarIcon, Loader2, Eye, EyeOff, Globe, Compass, Tag, Save, X, Copy, Check } from "lucide-react";
import { cancelPlateSolve, plateSolve, getWcsInfo, writeSolvedWcs } from "../../services/astrometry";
import type { WcsInfo, WriteSolvedWcsResult } from "../../services/astrometry";
import { shouldResetSolve, solvedWcsSummary, wcsRewriteBlocker, wcsWriteBlocker, type SolveIdentity } from "../../utils/solvedWcs";
import OutputNote from "./OutputNote";
import { getApiKey, getConfig } from "../../services/config";
import {
  VIEW_SCALE_ATTRIBUTE,
  fitImageToCanvas,
  fitsPixelToCanvas,
  imagePointToCanvas,
  overlayLayers,
  overlayStrokeScale,
} from "../../utils/starOverlay";
import { formatSignificant } from "../../utils/formatSignificant";
import { withDeadline } from "../../utils/deadline";
import {
  DEFAULT_HINT_TEXTS,
  headerHintTexts,
  mergeHintTexts,
  parsePositionHint,
  parseScaleRange,
  positionHintStatus,
  type HintField,
  type HintTexts,
  type ScaleUnits,
} from "../../utils/plateSolveScale";
import { starCountLabel } from "../../utils/analysisLabels";
import { ZERO_BASED_PIXEL_TITLE } from "../../utils/regionGeometry";
import {
  fieldOfViewText,
  pixelScaleText,
  solveCentreText,
  solveSummaryLine,
  type SolveCentre,
} from "../../utils/coordFormat";

export interface Star {
  x: number;
  y: number;
  flux: number;
  fwhm: number;
  snr: number;
}

interface FieldAnnotation {
  type: string;
  names: string[];
  pixelx: number;
  pixely: number;
  radius?: number | null;
}

interface SolveResult {
  center_ra: number;
  center_dec: number;
  pixel_scale_arcsec: number;
  field_of_view_w_arcmin: number;
  field_of_view_h_arcmin: number;
  fov_arcmin?: [number, number];
  orientation?: number;
  annotations?: FieldAnnotation[];
  wcs_cards?: [string, string][];
}

const EMPTY_ANNOTATIONS: FieldAnnotation[] = [];
const ANNOTATIONS_OFF_VIEW_TITLE = "Object labels are in the solved file's pixel grid, so they are not drawn on the composite on screen";

const SOLVE_TICK_MS = 250;
const DEFAULT_SOLVE_TIMEOUT_SECS = 120;

function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

const CARD_CLASS = "bg-zinc-900/80 rounded px-2 py-1.5";
const CENTRE_TONES = { solve: "text-emerald-300", header: "text-zinc-300" } as const;
const COPY_CENTRE_LABEL = "Copy centre, scale and FOV";
const COPY_CENTRE_TITLE = "Copy the centre in decimal degrees and sexagesimal, the pixel scale and the FOV as one line";
const COPIED_FEEDBACK_MS = 1500;
const COPIED_TEXT = "Copied";

interface SolveCentreCardsProps {
  centre: SolveCentre;
  source: keyof typeof CENTRE_TONES;
}

export function SolveCentreCards({ centre, source }: SolveCentreCardsProps) {
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState<string | null>(null);
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const text = solveCentreText(centre.center_ra, centre.center_dec);
  const scale = pixelScaleText(centre.pixel_scale_arcsec);
  const fov = fieldOfViewText(centre.field_of_view_w_arcmin, centre.field_of_view_h_arcmin);
  const copyable = Number.isFinite(centre.center_ra) && Number.isFinite(centre.center_dec);
  const valueClass = `${CENTRE_TONES[source]} font-mono`;

  useEffect(() => {
    return () => {
      if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current);
    };
  }, []);

  const copy = async () => {
    setCopyError(null);
    try {
      await navigator.clipboard.writeText(solveSummaryLine(centre));
      setCopied(true);
      if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current);
      copiedTimerRef.current = setTimeout(() => setCopied(false), COPIED_FEEDBACK_MS);
    } catch (e: unknown) {
      setCopied(false);
      setCopyError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  return (
    <>
      <div className={CARD_CLASS} data-centre-source={source}>
        <div className="text-zinc-500">Centre RA</div>
        <div className={valueClass} data-centre-ra>{text.ra}</div>
        <div className="text-zinc-500 font-mono" data-centre-ra-deg>{text.raDeg}</div>
      </div>
      <div className={CARD_CLASS}>
        <div className="flex items-center justify-between gap-1">
          <span className="text-zinc-500">Centre Dec</span>
          {copyable && (
            <>
              <button
                type="button"
                onClick={() => void copy()}
                aria-label={COPY_CENTRE_LABEL}
                title={COPY_CENTRE_TITLE}
                className="text-zinc-500 hover:text-zinc-200 transition-colors"
              >
                {copied ? <Check size={10} className="text-emerald-400" /> : <Copy size={10} />}
              </button>
              <span role="status" className="sr-only" data-copy-status>
                {copied ? COPIED_TEXT : ""}
              </span>
            </>
          )}
        </div>
        <div className={valueClass} data-centre-dec>{text.dec}</div>
        <div className="text-zinc-500 font-mono" data-centre-dec-deg>{text.decDeg}</div>
      </div>
      {scale && (
        <div className={CARD_CLASS}>
          <div className="text-zinc-500">Pixel Scale</div>
          <div className={valueClass}>{scale}</div>
        </div>
      )}
      {fov && (
        <div className={CARD_CLASS}>
          <div className="text-zinc-500">FOV</div>
          <div className={valueClass}>{fov}</div>
        </div>
      )}
      {copyError && (
        <div
          role="alert"
          data-copy-error
          className="col-span-2 text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words"
        >
          {copyError}
        </div>
      )}
    </>
  );
}

interface PlateSolvePanelProps {
  stars?: Star[];
  isLoading?: boolean;
  onDetect?: (sigma: number) => void;
  backgroundMedian?: number | null;
  backgroundSigma?: number | null;
  imageWidth?: number;
  imageHeight?: number;
  elapsed?: number;
  overlayCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
  filePath?: string | null;
  solvePath: string | null;
  detectError?: string | null;
  sourceBadge?: React.ReactNode;
  solveBadge?: React.ReactNode;
  detectedTotal?: number | null;
  annotationsOnView?: boolean;
  onWcsWritten?: (result: WriteSolvedWcsResult) => void;
}

interface SolveActionsProps {
  loading: boolean;
  disabled: boolean;
  elapsedMs: number;
  timeoutSecs: number;
  onSolve: () => void;
  onCancel: () => void;
}

export function SolveActions({ loading, disabled, elapsedMs, timeoutSecs, onSolve, onCancel }: SolveActionsProps) {
  const solveButtonRef = useRef<HTMLButtonElement>(null);
  const focusSolveRef = useRef(false);
  useEffect(() => {
    if (loading || !focusSolveRef.current) return;
    focusSolveRef.current = false;
    solveButtonRef.current?.focus();
  }, [loading]);
  const handleCancel = () => {
    focusSolveRef.current = true;
    onCancel();
  };
  return (
    <div className="flex gap-2">
      <button
        ref={solveButtonRef}
        onClick={onSolve}
        disabled={disabled}
        className="flex-1 flex items-center justify-center gap-2 bg-emerald-600/20 hover:bg-emerald-600/30 text-emerald-300 border border-emerald-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
      >
        {loading ? (
          <>
            <Loader2 size={12} className="animate-spin" /> Solving... {formatDuration(elapsedMs)} /{" "}
            {formatDuration(timeoutSecs * 1000)}
          </>
        ) : (
          <><Compass size={12} /> Plate Solve</>
        )}
      </button>
      {loading && (
        <button
          type="button"
          onClick={handleCancel}
          aria-label="Cancel plate solve"
          title="Stop waiting for this solve"
          className="flex items-center justify-center gap-1 rounded border border-zinc-700/60 px-3 py-1.5 text-xs text-zinc-300 hover:text-zinc-100 hover:border-zinc-500 transition-colors"
        >
          <X size={12} /> Cancel
        </button>
      )}
    </div>
  );
}

function useLiveCanvas(ref: React.RefObject<HTMLCanvasElement | null> | undefined): HTMLCanvasElement | null {
  const [canvas, setCanvas] = useState<HTMLCanvasElement | null>(null);
  useEffect(() => {
    if (!ref) {
      setCanvas(null);
      return;
    }
    const sync = () => {
      const el = ref.current;
      setCanvas(el && el.isConnected ? el : null);
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(document.body, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [ref]);
  return canvas;
}

function useViewScaleAttribute(canvas: HTMLCanvasElement | null): string | null {
  const [value, setValue] = useState<string | null>(null);
  useEffect(() => {
    if (!canvas) {
      setValue(null);
      return;
    }
    const read = () => setValue(canvas.getAttribute(VIEW_SCALE_ATTRIBUTE));
    read();
    const observer = new MutationObserver(read);
    observer.observe(canvas, { attributes: true, attributeFilter: [VIEW_SCALE_ATTRIBUTE] });
    return () => observer.disconnect();
  }, [canvas]);
  return value;
}

function useElementSize(el: HTMLElement | null): string {
  const [size, setSize] = useState("");
  useEffect(() => {
    if (!el) {
      setSize("");
      return;
    }
    const measure = () => setSize(`${el.clientWidth}x${el.clientHeight}`);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return size;
}

function PlateSolvePanel({
                                          stars = [],
                                          isLoading = false,
                                          onDetect,
                                          backgroundMedian,
                                          backgroundSigma,
                                          imageWidth,
                                          imageHeight,
                                          elapsed = 0,
                                          overlayCanvasRef,
                                          filePath,
                                          solvePath,
                                          detectError = null,
                                          sourceBadge,
                                          solveBadge,
                                          detectedTotal = null,
                                          annotationsOnView = true,
                                          onWcsWritten,
                                        }: PlateSolvePanelProps) {
  const overlayCanvas = useLiveCanvas(overlayCanvasRef);
  const overlayHostSize = useElementSize(overlayCanvas?.parentElement ?? null);
  const overlayViewScale = useViewScaleAttribute(overlayCanvas);

  const sigmaId = useId();
  const scaleLowId = useId();
  const scaleHighId = useId();
  const scaleUnitsId = useId();
  const downsampleId = useId();
  const centerRaId = useId();
  const centerDecId = useId();
  const searchRadiusId = useId();
  const [sigma, setSigma] = useState(5.0);
  const [showOverlay, setShowOverlay] = useState(true);
  const [showAnnotations, setShowAnnotations] = useState(true);
  const [selectedStar, setSelectedStar] = useState<number | null>(null);

  const [solveLoading, setSolveLoading] = useState(false);
  const [solveResult, setSolveResult] = useState<SolveResult | null>(null);
  const [solveError, setSolveError] = useState<string | null>(null);
  const [hasApiKey, setHasApiKey] = useState(false);
  const [hintTexts, setHintTexts] = useState<HintTexts>(DEFAULT_HINT_TEXTS);
  const touchedHintsRef = useRef<Set<HintField>>(new Set());
  const [downsample, setDownsample] = useState(0);
  const [wcsInfo, setWcsInfo] = useState<WcsInfo | null>(null);
  const [solveElapsedMs, setSolveElapsedMs] = useState(0);
  const [timeoutSecs, setTimeoutSecs] = useState(DEFAULT_SOLVE_TIMEOUT_SECS);
  const solveSeqRef = useRef(0);
  const solveLoadingRef = useRef(false);
  const [wcsWriting, setWcsWriting] = useState(false);
  const [wcsWriteError, setWcsWriteError] = useState<string | null>(null);
  const [wcsWritten, setWcsWritten] = useState<WriteSolvedWcsResult | null>(null);
  const wcsSeqRef = useRef(0);
  const writtenPathRef = useRef<string | null>(null);
  const solveIdentityRef = useRef<SolveIdentity>({ regionKey: filePath ?? null, solvePath });

  useEffect(() => {
    getApiKey("astrometry")
      .then((r) => setHasApiKey(!!r?.key))
      .catch(() => setHasApiKey(false));
    getConfig()
      .then((cfg) => setTimeoutSecs(cfg.plate_solve_timeout_secs || DEFAULT_SOLVE_TIMEOUT_SECS))
      .catch(() => setTimeoutSecs(DEFAULT_SOLVE_TIMEOUT_SECS));
  }, []);

  useEffect(() => {
    solveLoadingRef.current = solveLoading;
  }, [solveLoading]);

  const applyWcsHints = useCallback((info: WcsInfo, force: boolean) => {
    if (force) touchedHintsRef.current = new Set();
    const touched: ReadonlySet<HintField> = new Set(touchedHintsRef.current);
    const hints = headerHintTexts(info);
    setHintTexts((current) => mergeHintTexts(current, hints, touched));
  }, []);

  const editHint = useCallback(<K extends HintField>(field: K, value: HintTexts[K]) => {
    touchedHintsRef.current.add(field);
    setHintTexts((current) => ({ ...current, [field]: value }));
  }, []);

  useEffect(() => {
    solveSeqRef.current++;
    wcsSeqRef.current++;
    setWcsInfo(null);
    setSolveResult(null);
    setSolveError(null);
    setSolveLoading(false);
    setWcsWriting(false);
    setWcsWriteError(null);
    writtenPathRef.current = null;
    setWcsWritten(null);
    setSelectedStar(null);
    touchedHintsRef.current = new Set();
    setHintTexts(DEFAULT_HINT_TEXTS);
  }, [filePath]);

  useEffect(() => {
    const next: SolveIdentity = { regionKey: filePath ?? null, solvePath };
    const reset = shouldResetSolve(solveIdentityRef.current, next, writtenPathRef.current);
    solveIdentityRef.current = next;
    if (reset) {
      if (solveLoadingRef.current) cancelPlateSolve().catch(() => {});
      solveSeqRef.current++;
      wcsSeqRef.current++;
      setSolveResult(null);
      setSolveError(null);
      setSolveLoading(false);
      setWcsWriting(false);
      setWcsWriteError(null);
      writtenPathRef.current = null;
      setWcsWritten(null);
    }
    if (!solvePath) return;
    let cancelled = false;
    getWcsInfo(solvePath)
      .then((info) => {
        if (cancelled) return;
        setWcsInfo(info);
        applyWcsHints(info, false);
      })
      .catch(() => {
        if (!cancelled) setWcsInfo(null);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath, solvePath, applyWcsHints]);

  useEffect(() => {
    if (!solveLoading) return;
    const started = Date.now();
    setSolveElapsedMs(0);
    const id = window.setInterval(() => setSolveElapsedMs(Date.now() - started), SOLVE_TICK_MS);
    return () => window.clearInterval(id);
  }, [solveLoading]);

  const annotations = solveResult?.annotations ?? EMPTY_ANNOTATIONS;
  const { drawStars, drawAnnotations, unavailable: overlayUnavailable } = overlayLayers({
    canvasMounted: overlayCanvas !== null,
    showStars: showOverlay,
    starCount: stars.length,
    showAnnotations,
    annotationCount: annotations.length,
    annotationsOnView,
  });

  useEffect(() => {
    const canvas = overlayCanvas;
    if (!canvas) return;

    const hide = () => {
      const context = canvas.getContext("2d");
      context?.clearRect(0, 0, canvas.width, canvas.height);
      canvas.style.display = "none";
    };

    if (!drawStars && !drawAnnotations) {
      hide();
      return;
    }

    canvas.style.display = "block";

    const parent = canvas.parentElement;
    if (!parent) return;

    const W = Math.min(parent.clientWidth, 8192);
    const H = Math.min(parent.clientHeight, 8192);
    if (W === 0 || H === 0) return;

    if (canvas.width !== W) canvas.width = W;
    if (canvas.height !== H) canvas.height = H;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, W, H);

    const k = overlayStrokeScale(canvas.getBoundingClientRect().width, W, canvas.hasAttribute(VIEW_SCALE_ATTRIBUTE));
    const fit = fitImageToCanvas(W, H, imageWidth || 1, imageHeight || 1);
    const scale = fit.scale;

    if (drawStars) {
      const maxFlux = stars[0].flux || 1;

      stars.forEach((star, i) => {
        const { x: sx, y: sy } = imagePointToCanvas(star.x, star.y, fit);
        const radius = Math.max(3 * k, (star.fwhm || 3) * scale * 1.5);
        const brightness = Math.min(1, 0.3 + (star.flux / maxFlux) * 0.7);

        let color: string;
        if (star.snr > 50) color = `rgba(0, 255, 100, ${brightness})`;
        else if (star.snr > 20) color = `rgba(255, 255, 0, ${brightness})`;
        else color = `rgba(255, 100, 0, ${brightness})`;

        ctx.strokeStyle = color;
        ctx.lineWidth = 1.2 * k;
        ctx.beginPath();
        ctx.arc(sx, sy, radius, 0, Math.PI * 2);
        ctx.stroke();

        if (i < 20) {
          ctx.fillStyle = color;
          ctx.font = `${9 * k}px monospace`;
          ctx.fillText(`${i + 1}`, sx + radius + 2 * k, sy - 2 * k);
        }
      });

      if (selectedStar !== null && selectedStar < stars.length) {
        const s = stars[selectedStar];
        const { x: sx, y: sy } = imagePointToCanvas(s.x, s.y, fit);
        const radius = Math.max(6 * k, (s.fwhm || 3) * scale * 2);

        ctx.strokeStyle = "rgba(100, 200, 255, 1)";
        ctx.lineWidth = 2 * k;
        ctx.beginPath();
        ctx.arc(sx, sy, radius, 0, Math.PI * 2);
        ctx.stroke();

        ctx.strokeStyle = "rgba(100, 200, 255, 0.5)";
        ctx.lineWidth = 0.5 * k;
        ctx.beginPath();
        ctx.moveTo(sx - radius * 2, sy);
        ctx.lineTo(sx + radius * 2, sy);
        ctx.moveTo(sx, sy - radius * 2);
        ctx.lineTo(sx, sy + radius * 2);
        ctx.stroke();
      }
    }

    if (drawAnnotations) {
      ctx.lineWidth = 1.2 * k;
      ctx.font = `${10 * k}px monospace`;
      const margin = 20 * k;

      for (const ann of annotations) {
        const { x: ax, y: ay } = fitsPixelToCanvas(ann.pixelx, ann.pixely, fit);
        if (ax < -margin || ay < -margin || ax > W + margin || ay > H + margin) continue;

        const r = Math.max(10 * k, (ann.radius ?? 12) * scale);

        ctx.strokeStyle = "rgba(167, 139, 250, 0.85)";
        ctx.beginPath();
        ctx.arc(ax, ay, r, 0, Math.PI * 2);
        ctx.stroke();

        const label = ann.names?.length ? ann.names.slice(0, 2).join(", ") : ann.type;
        if (label) {
          ctx.fillStyle = "rgba(0, 0, 0, 0.55)";
          const tw = ctx.measureText(label).width;
          ctx.fillRect(ax + r + 2 * k, ay - 8 * k, tw + 6 * k, 13 * k);
          ctx.fillStyle = "rgba(196, 181, 253, 1)";
          ctx.fillText(label, ax + r + 5 * k, ay + 2 * k);
        }
      }
    }

    return hide;
  }, [stars, drawStars, drawAnnotations, annotations, selectedStar, imageWidth, imageHeight, overlayCanvas, overlayHostSize, overlayViewScale, solvePath]);

  const handleDetect = useCallback(() => {
    if (onDetect) onDetect(sigma);
  }, [onDetect, sigma]);

  const scaleRange = parseScaleRange(hintTexts.scaleLow, hintTexts.scaleHigh);

  const handleSolve = useCallback(async () => {
    if (!solvePath) return;
    const scale = parseScaleRange(hintTexts.scaleLow, hintTexts.scaleHigh);
    if (scale.error !== null) return;
    const hint = parsePositionHint(hintTexts.centerRa, hintTexts.centerDec, hintTexts.radius);
    if (hint.error !== null) return;
    const seq = ++solveSeqRef.current;
    setSolveLoading(true);
    setSolveError(null);
    setSolveResult(null);
    setWcsWriteError(null);
    writtenPathRef.current = null;
    setWcsWritten(null);
    try {
      const cfg = await getConfig().catch(() => null);
      const limitSecs = cfg?.plate_solve_timeout_secs || DEFAULT_SOLVE_TIMEOUT_SECS;
      if (solveSeqRef.current !== seq) return;
      setTimeoutSecs(limitSecs);
      const result = await withDeadline(
        plateSolve(solvePath, {
          scaleLower: scale.low,
          scaleUpper: scale.high,
          scaleUnits: hintTexts.scaleUnits,
          downsampleFactor: downsample > 1 ? downsample : undefined,
          centerRa: hint.centerRa ?? undefined,
          centerDec: hint.centerDec ?? undefined,
          radius: hint.radius ?? undefined,
        }),
        limitSecs * 1000,
        `Plate solve gave up after ${formatDuration(limitSecs * 1000)} (Settings > Timeout). astrometry.net may still finish the job; try again later.`,
      ) as SolveResult;
      if (solveSeqRef.current !== seq) return;
      setSolveResult(result);
      getWcsInfo(solvePath)
        .then((info) => {
          if (solveSeqRef.current === seq) setWcsInfo(info);
        })
        .catch(() => {});
    } catch (e: unknown) {
      if (solveSeqRef.current === seq) setSolveError(e instanceof Error ? e.message : String(e));
    } finally {
      if (solveSeqRef.current === seq) setSolveLoading(false);
    }
  }, [solvePath, hintTexts, downsample]);

  const handleCancelSolve = useCallback(() => {
    solveSeqRef.current++;
    setSolveLoading(false);
    cancelPlateSolve().catch(() => {});
  }, []);

  const wcsBlocker = wcsWriteBlocker(solveResult) ?? wcsRewriteBlocker(wcsWritten?.fits_path ?? null, solvePath);

  const handleWriteWcs = useCallback(async () => {
    if (!solvePath || wcsWriteBlocker(solveResult) !== null) return;
    const cards = solveResult?.wcs_cards ?? [];
    const seq = ++wcsSeqRef.current;
    setWcsWriting(true);
    setWcsWriteError(null);
    writtenPathRef.current = null;
    setWcsWritten(null);
    try {
      const res = await writeSolvedWcs(solvePath, cards);
      if (wcsSeqRef.current !== seq) return;
      writtenPathRef.current = res.fits_path;
      onWcsWritten?.(res);
      setWcsWritten(res);
    } catch (e: unknown) {
      if (wcsSeqRef.current === seq) setWcsWriteError(e instanceof Error ? e.message : String(e));
    } finally {
      if (wcsSeqRef.current === seq) setWcsWriting(false);
    }
  }, [solvePath, solveResult, onWcsWritten]);

  const medianFwhm = useMemo(() => {
    if (stars.length === 0) return null;
    const sorted = stars.map((s) => s.fwhm).sort((a, b) => a - b);
    const mid = Math.floor(sorted.length / 2);
    return (sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2).toFixed(2);
  }, [stars]);

  const activeWcs = solveResult ? solveResult : wcsInfo;
  const positionHint = parsePositionHint(hintTexts.centerRa, hintTexts.centerDec, hintTexts.radius);

  return (
    <div className="flex flex-col gap-3">
      <div className="ab-panel overflow-hidden">
        <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
          <div className="flex items-center gap-2">
            <Crosshair size={12} className="text-cyan-400" />
            <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
              Star Detection
            </span>
            {sourceBadge}
          </div>
          <div className="flex items-center gap-2">
            {stars.length > 0 && (
              <button
                onClick={() => setShowOverlay(!showOverlay)}
                disabled={overlayUnavailable}
                className="text-zinc-500 hover:text-zinc-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                title={
                  overlayUnavailable
                    ? "The current view has no star overlay layer"
                    : showOverlay
                      ? "Hide overlay"
                      : "Show overlay"
                }
              >
                {showOverlay && !overlayUnavailable ? <Eye size={12} /> : <EyeOff size={12} />}
              </button>
            )}
          </div>
        </div>

        <div className="px-3 py-2 space-y-2">
          <div className="flex items-center gap-2">
            <label htmlFor={sigmaId} className="text-[10px] text-zinc-500 w-12">
              σ thresh
            </label>
            <input
              id={sigmaId}
              type="range"
              min="2"
              max="15"
              step="0.5"
              value={sigma}
              onChange={(e) => setSigma(parseFloat(e.target.value))}
              className="flex-1 h-1 accent-cyan-500"
            />
            <span className="text-[10px] text-zinc-400 font-mono w-8 text-right">
              {sigma.toFixed(1)}
            </span>
          </div>

          <button
            onClick={handleDetect}
            disabled={isLoading}
            className="w-full flex items-center justify-center gap-2 bg-cyan-600/20 hover:bg-cyan-600/30 text-cyan-300 border border-cyan-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
          >
            {isLoading ? (
              <><Loader2 size={12} className="animate-spin" /> Detecting...</>
            ) : (
              <><StarIcon size={12} /> Detect Stars</>
            )}
          </button>

          {detectError && (
            <div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
              Star detection failed: {detectError}
            </div>
          )}

          {overlayUnavailable && (
            <div className="text-[10px] text-zinc-500">
              The current view has no overlay layer, so rings and labels are not drawn on it.
            </div>
          )}

          {stars.length > 0 && (
            <div className="space-y-1.5">
              <div className="grid grid-cols-3 gap-1 text-[10px]">
                <div
                  className="bg-zinc-900/80 rounded px-2 py-1"
                  title={
                    detectedTotal != null && detectedTotal > stars.length
                      ? `Detection keeps the ${stars.length} brightest of ${detectedTotal} sources; FWHM is their median`
                      : undefined
                  }
                >
                  <div className="text-zinc-500">Stars</div>
                  <div className="text-cyan-300 font-mono break-words">{starCountLabel(stars.length, detectedTotal)}</div>
                </div>
                <div className="bg-zinc-900/80 rounded px-2 py-1">
                  <div className="text-zinc-500">FWHM</div>
                  <div className="text-cyan-300 font-mono">{medianFwhm} px</div>
                </div>
                <div className="bg-zinc-900/80 rounded px-2 py-1">
                  <div className="text-zinc-500">Time</div>
                  <div className="text-cyan-300 font-mono">{elapsed} ms</div>
                </div>
              </div>

              {backgroundMedian != null && backgroundSigma != null && (
                <div className="flex gap-2 text-[10px] text-zinc-500">
                  <span>BG: {formatSignificant(backgroundMedian)}</span>
                  <span>σ: {formatSignificant(backgroundSigma)}</span>
                </div>
              )}

              <div className="max-h-[120px] overflow-y-auto">
                <table className="w-full text-[10px]">
                  <thead>
                  <tr className="text-zinc-500 border-b border-zinc-800/50">
                    <th className="text-left px-1 py-0.5">#</th>
                    <th className="text-right px-1" title={ZERO_BASED_PIXEL_TITLE}>X (0-based)</th>
                    <th className="text-right px-1" title={ZERO_BASED_PIXEL_TITLE}>Y (0-based)</th>
                    <th className="text-right px-1">Flux</th>
                    <th className="text-right px-1">FWHM</th>
                    <th className="text-right px-1">SNR</th>
                  </tr>
                  </thead>
                  <tbody>
                  {stars.slice(0, 20).map((s, i) => (
                    <tr
                      key={i}
                      className={`cursor-pointer hover:bg-zinc-800/50 transition-colors ${
                        selectedStar === i ? "bg-cyan-900/20" : ""
                      }`}
                      onClick={() => setSelectedStar(selectedStar === i ? null : i)}
                    >
                      <td className="text-zinc-500 px-1 py-0.5">{i + 1}</td>
                      <td className="text-zinc-300 text-right px-1 font-mono">{s.x.toFixed(1)}</td>
                      <td className="text-zinc-300 text-right px-1 font-mono">{s.y.toFixed(1)}</td>
                      <td className="text-zinc-300 text-right px-1 font-mono">{formatSignificant(s.flux)}</td>
                      <td className="text-zinc-300 text-right px-1 font-mono">{s.fwhm.toFixed(1)}</td>
                      <td className="text-zinc-300 text-right px-1 font-mono">{s.snr.toFixed(0)}</td>
                    </tr>
                  ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </div>
      </div>

      <div className="ab-panel overflow-hidden">
        <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
          <div className="flex items-center gap-2">
            <Compass size={12} className="text-emerald-400" />
            <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
              Plate Solve
            </span>
            {solveBadge}
          </div>
          <div className="flex items-center gap-2">
            {annotations.length > 0 && (
              <button
                onClick={() => setShowAnnotations(!showAnnotations)}
                disabled={!annotationsOnView}
                className={`transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${showAnnotations ? "text-violet-400" : "text-zinc-600 hover:text-zinc-400"}`}
                title={!annotationsOnView ? ANNOTATIONS_OFF_VIEW_TITLE : showAnnotations ? "Hide object labels" : "Show object labels"}
              >
                <Tag size={12} />
              </button>
            )}
            {activeWcs && (
              <Globe size={12} className="text-emerald-400/60" />
            )}
          </div>
        </div>

        <div className="px-3 py-2 space-y-2">
          {activeWcs && !solveResult && (
            <div className="flex items-center gap-1.5 text-[10px] text-emerald-400/70">
              <Globe size={10} />
              <span>WCS present in header</span>
            </div>
          )}

          <div className="flex gap-2">
            <div className="flex-1 flex flex-col gap-0.5">
              <label htmlFor={scaleLowId} className="text-[9px] text-zinc-500 uppercase">
                Scale low
              </label>
              <input
                id={scaleLowId}
                type="text"
                inputMode="decimal"
                value={hintTexts.scaleLow}
                aria-invalid={scaleRange.error !== null}
                onChange={(e) => editHint("scaleLow", e.target.value)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-emerald-500/50 w-full"
              />
            </div>
            <div className="flex-1 flex flex-col gap-0.5">
              <label htmlFor={scaleHighId} className="text-[9px] text-zinc-500 uppercase">
                Scale high
              </label>
              <input
                id={scaleHighId}
                type="text"
                inputMode="decimal"
                value={hintTexts.scaleHigh}
                aria-invalid={scaleRange.error !== null}
                onChange={(e) => editHint("scaleHigh", e.target.value)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-emerald-500/50 w-full"
              />
            </div>
          </div>

          {scaleRange.error !== null && (
            <div role="alert" className="text-[10px] text-red-400">
              {scaleRange.error}
            </div>
          )}

          <div className="flex gap-2">
            <div className="flex-1 flex flex-col gap-0.5">
              <label htmlFor={scaleUnitsId} className="text-[9px] text-zinc-500 uppercase">
                Scale units
              </label>
              <select
                id={scaleUnitsId}
                value={hintTexts.scaleUnits}
                onChange={(e) => editHint("scaleUnits", e.target.value as ScaleUnits)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 focus:border-emerald-500/50 w-full"
              >
                <option value="arcsecperpix">arcsec/px</option>
                <option value="arcminwidth">arcmin width</option>
                <option value="degwidth">deg width</option>
              </select>
            </div>
            <div className="flex-1 flex flex-col gap-0.5">
              <label htmlFor={downsampleId} className="text-[9px] text-zinc-500 uppercase">
                Downsample
              </label>
              <select
                id={downsampleId}
                value={downsample}
                onChange={(e) => setDownsample(parseInt(e.target.value, 10))}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 focus:border-emerald-500/50 w-full"
              >
                <option value={0}>Auto</option>
                <option value={2}>2x</option>
                <option value={4}>4x</option>
              </select>
            </div>
          </div>

          <div className="flex gap-2">
            <div className="flex-1 flex flex-col gap-0.5">
              <label htmlFor={centerRaId} className="text-[9px] text-zinc-500 uppercase">
                Centre RA (deg)
              </label>
              <input
                id={centerRaId}
                type="text"
                inputMode="decimal"
                value={hintTexts.centerRa}
                aria-invalid={positionHint.raInvalid}
                placeholder="blind"
                title="Right ascension of the field centre in degrees; speeds the solve up enormously"
                onChange={(e) => editHint("centerRa", e.target.value)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-emerald-500/50 w-full"
              />
            </div>
            <div className="flex-1 flex flex-col gap-0.5">
              <label htmlFor={centerDecId} className="text-[9px] text-zinc-500 uppercase">
                Centre Dec (deg)
              </label>
              <input
                id={centerDecId}
                type="text"
                inputMode="decimal"
                value={hintTexts.centerDec}
                aria-invalid={positionHint.decInvalid}
                placeholder="blind"
                title="Declination of the field centre in degrees; both centre fields are needed for a hinted solve"
                onChange={(e) => editHint("centerDec", e.target.value)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-emerald-500/50 w-full"
              />
            </div>
            <div className="flex-1 flex flex-col gap-0.5">
              <label htmlFor={searchRadiusId} className="text-[9px] text-zinc-500 uppercase">
                Radius (deg)
              </label>
              <input
                id={searchRadiusId}
                type="text"
                inputMode="decimal"
                value={hintTexts.radius}
                aria-invalid={positionHint.radiusInvalid}
                placeholder="10"
                title="Search radius around the centre in degrees (default 10 when left empty)"
                onChange={(e) => editHint("radius", e.target.value)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-emerald-500/50 w-full"
              />
            </div>
          </div>

          {positionHint.error !== null && (
            <div role="alert" className="text-[10px] text-red-400">
              {positionHint.error}
            </div>
          )}

          <div className="flex items-center justify-between text-[9px] text-zinc-500">
            <span>{positionHintStatus(positionHint)}</span>
            {wcsInfo && (
              <button
                type="button"
                onClick={() => applyWcsHints(wcsInfo, true)}
                className="px-1.5 py-0.5 rounded border border-zinc-700/60 text-zinc-400 hover:text-zinc-200"
                title="Refill the hints from the WCS in the header"
              >
                From header
              </button>
            )}
          </div>

          <SolveActions
            loading={solveLoading}
            disabled={solveLoading || !solvePath || scaleRange.error !== null || positionHint.error !== null}
            elapsedMs={solveElapsedMs}
            timeoutSecs={timeoutSecs}
            onSolve={handleSolve}
            onCancel={handleCancelSolve}
          />

          {solveLoading && (
            <div className="text-[9px] text-zinc-500">
              Queued on astrometry.net; the request gives up after {formatDuration(timeoutSecs * 1000)}.
            </div>
          )}

          {!hasApiKey && (
            <div className="text-[10px] text-amber-400/70 bg-amber-900/20 border border-amber-800/20 rounded px-2.5 py-1.5">
              Set your astrometry.net API key in Settings to enable online plate solving.
              Files with existing WCS headers will still display coordinates.
            </div>
          )}

          {solveError && (
            <div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
              {solveError}
            </div>
          )}

          {solveResult && (
            <div className="grid grid-cols-2 gap-1.5 text-[10px]">
              <SolveCentreCards centre={solveResult} source="solve" />
              {solveResult.orientation !== undefined && (
                <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
                  <div className="text-zinc-500">Orientation</div>
                  <div className="text-emerald-300 font-mono">{solveResult.orientation?.toFixed(2)}°</div>
                </div>
              )}
              {annotations.length > 0 && (
                <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
                  <div className="text-zinc-500">Objects Identified</div>
                  <div className="text-violet-300 font-mono">
                    {annotations.length} ({annotations.slice(0, 3).flatMap((a) => a.names?.slice(0, 1) ?? []).join(", ")}{annotations.length > 3 ? ", ..." : ""})
                  </div>
                </div>
              )}
            </div>
          )}

          <button
            type="button"
            onClick={handleWriteWcs}
            disabled={wcsBlocker !== null || wcsWriting || !solvePath}
            title={wcsBlocker ?? undefined}
            className="w-full flex items-center justify-center gap-2 bg-emerald-600/10 hover:bg-emerald-600/20 text-emerald-300 border border-emerald-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {wcsWriting ? (
              <><Loader2 size={12} className="animate-spin" /> Writing WCS...</>
            ) : (
              <><Save size={12} /> Write WCS to image</>
            )}
          </button>

          {wcsWriteError && (
            <div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
              {wcsWriteError}
            </div>
          )}

          {wcsWritten && (
            <div data-wcs-summary className="text-[10px] text-emerald-300 break-words">
              {solvedWcsSummary(wcsWritten)}
            </div>
          )}
          {wcsWritten?.fits_path && <OutputNote path={wcsWritten.fits_path} />}

          {!solveResult && wcsInfo && wcsInfo.center_ra !== undefined && (
            <div className="grid grid-cols-2 gap-1.5 text-[10px]">
              <SolveCentreCards centre={wcsInfo} source="header" />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default memo(PlateSolvePanel);
