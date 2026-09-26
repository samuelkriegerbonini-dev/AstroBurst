import { useState, useCallback, useEffect, useId, useMemo, useRef, memo } from "react";
import { Crosshair, Star as StarIcon, Loader2, Eye, EyeOff, Globe, Compass, Tag } from "lucide-react";
import { plateSolve, getWcsInfo } from "../../services/astrometry";
import type { WcsInfo } from "../../services/astrometry";
import { getApiKey, getConfig } from "../../services/config";
import { fitImageToCanvas, fitsPixelToCanvas, imagePointToCanvas } from "../../utils/starOverlay";
import { withDeadline } from "../../utils/deadline";
import {
  DEFAULT_SCALE_HIGH_TEXT,
  DEFAULT_SCALE_LOW_TEXT,
  parseScaleRange,
  scaleHintFromPixelScale,
} from "../../utils/plateSolveScale";
import { starCountLabel } from "../../utils/analysisLabels";
import { ZERO_BASED_PIXEL_TITLE } from "../../utils/regionGeometry";

export interface Star {
  x: number;
  y: number;
  flux: number;
  fwhm: number;
  snr: number;
}

function formatRA(ra: number): string {
  const h = ra / 15;
  const hours = Math.floor(h);
  const minutes = Math.floor((h - hours) * 60);
  const seconds = ((h - hours) * 60 - minutes) * 60;
  return `${hours}h ${minutes}m ${seconds.toFixed(2)}s`;
}

function formatDec(dec: number): string {
  const sign = dec >= 0 ? "+" : "-";
  const abs = Math.abs(dec);
  const degrees = Math.floor(abs);
  const arcmin = Math.floor((abs - degrees) * 60);
  const arcsec = ((abs - degrees) * 60 - arcmin) * 60;
  return `${sign}${degrees}° ${arcmin}' ${arcsec.toFixed(1)}"`;
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
}

const EMPTY_ANNOTATIONS: FieldAnnotation[] = [];

const SOLVE_TICK_MS = 250;
const DEFAULT_SOLVE_TIMEOUT_SECS = 120;

function parseAngle(text: string, limit: number): number | null {
  const value = parseFloat(text);
  return Number.isFinite(value) && Math.abs(value) <= limit ? value : null;
}

function parseRadius(text: string): number | null {
  const value = parseFloat(text);
  return Number.isFinite(value) && value > 0 && value <= 180 ? value : null;
}

function hintRadiusDeg(info: WcsInfo): number | null {
  const fov = info.fov_arcmin;
  if (!fov || !Number.isFinite(fov[0]) || !Number.isFinite(fov[1])) return null;
  const radius = Math.hypot(fov[0], fov[1]) / 2 / 60;
  return radius > 0 ? radius : null;
}

function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
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
  detectError?: string | null;
  sourceBadge?: React.ReactNode;
  detectedTotal?: number | null;
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
                                          detectError = null,
                                          sourceBadge,
                                          detectedTotal = null,
                                        }: PlateSolvePanelProps) {
  const overlayCanvas = useLiveCanvas(overlayCanvasRef);
  const overlayHostSize = useElementSize(overlayCanvas?.parentElement ?? null);

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
  const [scaleLowText, setScaleLowText] = useState(DEFAULT_SCALE_LOW_TEXT);
  const [scaleHighText, setScaleHighText] = useState(DEFAULT_SCALE_HIGH_TEXT);
  const [scaleUnits, setScaleUnits] = useState<"arcsecperpix" | "arcminwidth" | "degwidth">("arcsecperpix");
  const [downsample, setDownsample] = useState(0);
  const [wcsInfo, setWcsInfo] = useState<WcsInfo | null>(null);
  const [centerRaText, setCenterRaText] = useState("");
  const [centerDecText, setCenterDecText] = useState("");
  const [searchRadiusText, setSearchRadiusText] = useState("");
  const [solveElapsedMs, setSolveElapsedMs] = useState(0);
  const [timeoutSecs, setTimeoutSecs] = useState(DEFAULT_SOLVE_TIMEOUT_SECS);
  const solveSeqRef = useRef(0);

  useEffect(() => {
    getApiKey("astrometry")
      .then((r) => setHasApiKey(!!r?.key))
      .catch(() => setHasApiKey(false));
    getConfig()
      .then((cfg) => setTimeoutSecs(cfg.plate_solve_timeout_secs || DEFAULT_SOLVE_TIMEOUT_SECS))
      .catch(() => setTimeoutSecs(DEFAULT_SOLVE_TIMEOUT_SECS));
  }, []);

  const applyWcsHints = useCallback((info: WcsInfo) => {
    const scaleHint = scaleHintFromPixelScale(info.pixel_scale_arcsec);
    if (scaleHint) {
      setScaleLowText(scaleHint.low);
      setScaleHighText(scaleHint.high);
      setScaleUnits("arcsecperpix");
    }
    if (!Number.isFinite(info.center_ra) || !Number.isFinite(info.center_dec)) return;
    setCenterRaText(info.center_ra.toFixed(6));
    setCenterDecText(info.center_dec.toFixed(6));
    const radius = hintRadiusDeg(info);
    if (radius !== null) setSearchRadiusText(radius.toFixed(4));
  }, []);

  useEffect(() => {
    solveSeqRef.current++;
    setWcsInfo(null);
    setSolveResult(null);
    setSolveError(null);
    setSolveLoading(false);
    setSelectedStar(null);
    setCenterRaText("");
    setCenterDecText("");
    setSearchRadiusText("");
    setScaleLowText(DEFAULT_SCALE_LOW_TEXT);
    setScaleHighText(DEFAULT_SCALE_HIGH_TEXT);
    setScaleUnits("arcsecperpix");
    if (!filePath) return;
    let cancelled = false;
    getWcsInfo(filePath)
      .then((info) => {
        if (cancelled) return;
        setWcsInfo(info);
        applyWcsHints(info);
      })
      .catch(() => {
        if (!cancelled) setWcsInfo(null);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath, applyWcsHints]);

  useEffect(() => {
    if (!solveLoading) return;
    const started = Date.now();
    setSolveElapsedMs(0);
    const id = window.setInterval(() => setSolveElapsedMs(Date.now() - started), SOLVE_TICK_MS);
    return () => window.clearInterval(id);
  }, [solveLoading]);

  const annotations = solveResult?.annotations ?? EMPTY_ANNOTATIONS;

  useEffect(() => {
    const canvas = overlayCanvas;
    if (!canvas) return;

    const hide = () => {
      const context = canvas.getContext("2d");
      context?.clearRect(0, 0, canvas.width, canvas.height);
      canvas.style.display = "none";
    };

    const drawStars = showOverlay && stars.length > 0;
    const drawAnnotations = showAnnotations && annotations.length > 0;

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

    canvas.width = W;
    canvas.height = H;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, W, H);

    const fit = fitImageToCanvas(W, H, imageWidth || 1, imageHeight || 1);
    const scale = fit.scale;

    if (drawStars) {
      const maxFlux = stars[0].flux || 1;

      stars.forEach((star, i) => {
        const { x: sx, y: sy } = imagePointToCanvas(star.x, star.y, fit);
        const radius = Math.max(3, (star.fwhm || 3) * scale * 1.5);
        const brightness = Math.min(1, 0.3 + (star.flux / maxFlux) * 0.7);

        let color: string;
        if (star.snr > 50) color = `rgba(0, 255, 100, ${brightness})`;
        else if (star.snr > 20) color = `rgba(255, 255, 0, ${brightness})`;
        else color = `rgba(255, 100, 0, ${brightness})`;

        ctx.strokeStyle = color;
        ctx.lineWidth = 1.2;
        ctx.beginPath();
        ctx.arc(sx, sy, radius, 0, Math.PI * 2);
        ctx.stroke();

        if (i < 20) {
          ctx.fillStyle = color;
          ctx.font = "9px monospace";
          ctx.fillText(`${i + 1}`, sx + radius + 2, sy - 2);
        }
      });

      if (selectedStar !== null && selectedStar < stars.length) {
        const s = stars[selectedStar];
        const { x: sx, y: sy } = imagePointToCanvas(s.x, s.y, fit);
        const radius = Math.max(6, (s.fwhm || 3) * scale * 2);

        ctx.strokeStyle = "rgba(100, 200, 255, 1)";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(sx, sy, radius, 0, Math.PI * 2);
        ctx.stroke();

        ctx.strokeStyle = "rgba(100, 200, 255, 0.5)";
        ctx.lineWidth = 0.5;
        ctx.beginPath();
        ctx.moveTo(sx - radius * 2, sy);
        ctx.lineTo(sx + radius * 2, sy);
        ctx.moveTo(sx, sy - radius * 2);
        ctx.lineTo(sx, sy + radius * 2);
        ctx.stroke();
      }
    }

    if (drawAnnotations) {
      ctx.lineWidth = 1.2;
      ctx.font = "10px monospace";

      for (const ann of annotations) {
        const { x: ax, y: ay } = fitsPixelToCanvas(ann.pixelx, ann.pixely, fit);
        if (ax < -20 || ay < -20 || ax > W + 20 || ay > H + 20) continue;

        const r = Math.max(10, (ann.radius ?? 12) * scale);

        ctx.strokeStyle = "rgba(167, 139, 250, 0.85)";
        ctx.beginPath();
        ctx.arc(ax, ay, r, 0, Math.PI * 2);
        ctx.stroke();

        const label = ann.names?.length ? ann.names.slice(0, 2).join(", ") : ann.type;
        if (label) {
          ctx.fillStyle = "rgba(0, 0, 0, 0.55)";
          const tw = ctx.measureText(label).width;
          ctx.fillRect(ax + r + 2, ay - 8, tw + 6, 13);
          ctx.fillStyle = "rgba(196, 181, 253, 1)";
          ctx.fillText(label, ax + r + 5, ay + 2);
        }
      }
    }

    return hide;
  }, [stars, showOverlay, showAnnotations, annotations, selectedStar, imageWidth, imageHeight, overlayCanvas, overlayHostSize, filePath]);

  const overlayUnavailable = overlayCanvas === null && (stars.length > 0 || annotations.length > 0);

  const handleDetect = useCallback(() => {
    if (onDetect) onDetect(sigma);
  }, [onDetect, sigma]);

  const scaleRange = parseScaleRange(scaleLowText, scaleHighText);

  const handleSolve = useCallback(async () => {
    if (!filePath) return;
    const scale = parseScaleRange(scaleLowText, scaleHighText);
    if (scale.error !== null) return;
    const seq = ++solveSeqRef.current;
    const centerRa = parseAngle(centerRaText, 360);
    const centerDec = parseAngle(centerDecText, 90);
    const radius = parseRadius(searchRadiusText);
    const positionHint = centerRa !== null && centerDec !== null;
    setSolveLoading(true);
    setSolveError(null);
    setSolveResult(null);
    try {
      const cfg = await getConfig().catch(() => null);
      const limitSecs = cfg?.plate_solve_timeout_secs || DEFAULT_SOLVE_TIMEOUT_SECS;
      if (solveSeqRef.current !== seq) return;
      setTimeoutSecs(limitSecs);
      const result = await withDeadline(
        plateSolve(filePath, {
          scaleLower: scale.low,
          scaleUpper: scale.high,
          scaleUnits,
          downsampleFactor: downsample > 1 ? downsample : undefined,
          centerRa: positionHint ? centerRa : undefined,
          centerDec: positionHint ? centerDec : undefined,
          radius: positionHint && radius !== null ? radius : undefined,
        }),
        limitSecs * 1000,
        `Plate solve gave up after ${formatDuration(limitSecs * 1000)} (Settings > Timeout). astrometry.net may still finish the job; try again later.`,
      ) as SolveResult;
      if (solveSeqRef.current !== seq) return;
      setSolveResult(result);
      getWcsInfo(filePath)
        .then((info) => {
          if (solveSeqRef.current === seq) setWcsInfo(info);
        })
        .catch(() => {});
    } catch (e: unknown) {
      if (solveSeqRef.current === seq) setSolveError(e instanceof Error ? e.message : String(e));
    } finally {
      if (solveSeqRef.current === seq) setSolveLoading(false);
    }
  }, [filePath, scaleLowText, scaleHighText, scaleUnits, downsample, centerRaText, centerDecText, searchRadiusText]);

  const medianFwhm = useMemo(() => {
    if (stars.length === 0) return null;
    const sorted = stars.map((s) => s.fwhm).sort((a, b) => a - b);
    const mid = Math.floor(sorted.length / 2);
    return (sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2).toFixed(2);
  }, [stars]);

  const activeWcs = solveResult ? solveResult : wcsInfo;
  const solveHintReady = parseAngle(centerRaText, 360) !== null && parseAngle(centerDecText, 90) !== null;

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
                  <span>BG: {backgroundMedian.toFixed(1)}</span>
                  <span>σ: {backgroundSigma.toFixed(2)}</span>
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
                      <td className="text-zinc-300 text-right px-1 font-mono">{s.flux.toFixed(0)}</td>
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
          </div>
          <div className="flex items-center gap-2">
            {annotations.length > 0 && (
              <button
                onClick={() => setShowAnnotations(!showAnnotations)}
                className={`transition-colors ${showAnnotations ? "text-violet-400" : "text-zinc-600 hover:text-zinc-400"}`}
                title={showAnnotations ? "Hide object labels" : "Show object labels"}
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
                value={scaleLowText}
                aria-invalid={scaleRange.error !== null}
                onChange={(e) => setScaleLowText(e.target.value)}
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
                value={scaleHighText}
                aria-invalid={scaleRange.error !== null}
                onChange={(e) => setScaleHighText(e.target.value)}
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
                value={scaleUnits}
                onChange={(e) => setScaleUnits(e.target.value as typeof scaleUnits)}
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
                type="number"
                min={0}
                max={360}
                step={0.0001}
                value={centerRaText}
                placeholder="blind"
                title="Right ascension of the field centre in degrees; speeds the solve up enormously"
                onChange={(e) => setCenterRaText(e.target.value)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-emerald-500/50 w-full"
              />
            </div>
            <div className="flex-1 flex flex-col gap-0.5">
              <label htmlFor={centerDecId} className="text-[9px] text-zinc-500 uppercase">
                Centre Dec (deg)
              </label>
              <input
                id={centerDecId}
                type="number"
                min={-90}
                max={90}
                step={0.0001}
                value={centerDecText}
                placeholder="blind"
                title="Declination of the field centre in degrees; both centre fields are needed for a hinted solve"
                onChange={(e) => setCenterDecText(e.target.value)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-emerald-500/50 w-full"
              />
            </div>
            <div className="flex-1 flex flex-col gap-0.5">
              <label htmlFor={searchRadiusId} className="text-[9px] text-zinc-500 uppercase">
                Radius (deg)
              </label>
              <input
                id={searchRadiusId}
                type="number"
                min={0.01}
                max={180}
                step={0.1}
                value={searchRadiusText}
                placeholder="10"
                title="Search radius around the centre in degrees (default 10 when left empty)"
                onChange={(e) => setSearchRadiusText(e.target.value)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-emerald-500/50 w-full"
              />
            </div>
          </div>

          <div className="flex items-center justify-between text-[9px] text-zinc-500">
            <span>
              {solveHintReady ? "Hinted solve around the field centre" : "Blind solve: fill both centre fields to hint it"}
            </span>
            {wcsInfo && (
              <button
                type="button"
                onClick={() => applyWcsHints(wcsInfo)}
                className="px-1.5 py-0.5 rounded border border-zinc-700/60 text-zinc-400 hover:text-zinc-200"
                title="Refill the hints from the WCS in the header"
              >
                From header
              </button>
            )}
          </div>

          <button
            onClick={handleSolve}
            disabled={solveLoading || !filePath || scaleRange.error !== null}
            className="w-full flex items-center justify-center gap-2 bg-emerald-600/20 hover:bg-emerald-600/30 text-emerald-300 border border-emerald-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
          >
            {solveLoading ? (
              <>
                <Loader2 size={12} className="animate-spin" /> Solving... {formatDuration(solveElapsedMs)} /{" "}
                {formatDuration(timeoutSecs * 1000)}
              </>
            ) : (
              <><Compass size={12} /> Plate Solve</>
            )}
          </button>

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
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Center RA</div>
                <div className="text-emerald-300 font-mono">{formatRA(solveResult.center_ra)}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Center Dec</div>
                <div className="text-emerald-300 font-mono">{formatDec(solveResult.center_dec)}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Pixel Scale</div>
                <div className="text-emerald-300 font-mono">{solveResult.pixel_scale_arcsec?.toFixed(3)}"/px</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">FOV</div>
                <div className="text-emerald-300 font-mono">
                  {solveResult.field_of_view_w_arcmin?.toFixed(1)}' x {solveResult.field_of_view_h_arcmin?.toFixed(1)}'
                </div>
              </div>
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

          {!solveResult && wcsInfo && wcsInfo.center_ra !== undefined && (
            <div className="grid grid-cols-2 gap-1.5 text-[10px]">
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Center RA</div>
                <div className="text-zinc-300 font-mono">{formatRA(wcsInfo.center_ra)}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Center Dec</div>
                <div className="text-zinc-300 font-mono">{formatDec(wcsInfo.center_dec)}</div>
              </div>
              {wcsInfo.pixel_scale_arcsec != null && (
                <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                  <div className="text-zinc-500">Pixel Scale</div>
                  <div className="text-zinc-300 font-mono">{wcsInfo.pixel_scale_arcsec.toFixed(3)}"/px</div>
                </div>
              )}
              {wcsInfo.fov_arcmin && (
                <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                  <div className="text-zinc-500">FOV</div>
                  <div className="text-zinc-300 font-mono">
                    {wcsInfo.fov_arcmin[0].toFixed(1)}' x {wcsInfo.fov_arcmin[1].toFixed(1)}'
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default memo(PlateSolvePanel);
