import { useState, useCallback, useEffect } from "react";
import { Crosshair, Star as StarIcon, Loader2, Eye, EyeOff, Globe, Compass } from "lucide-react";
import { plateSolve, getWcsInfo } from "../../services/astrometry.service";
import { getApiKey } from "../../services/config.service";
import type { Star, WcsInfo } from "../../shared/types";

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

interface SolveResult {
  success: boolean;
  ra_center?: number;
  dec_center?: number;
  pixel_scale?: number;
  field_w_arcmin?: number;
  field_h_arcmin?: number;
  orientation?: number;
}

interface PlateSolvePanelProps {
  stars?: Star[];
  count?: number;
  isLoading?: boolean;
  onDetect?: (sigma: number) => void;
  backgroundMedian?: number | null;
  backgroundSigma?: number | null;
  imageWidth?: number;
  imageHeight?: number;
  elapsed?: number;
  overlayCanvasRef?: React.RefObject<HTMLCanvasElement | null>;
  filePath?: string | null;
}

export default function PlateSolvePanel({
  stars = [],
  count = 0,
  isLoading = false,
  onDetect,
  backgroundMedian,
  backgroundSigma,
  imageWidth,
  imageHeight,
  elapsed = 0,
  overlayCanvasRef,
  filePath,
}: PlateSolvePanelProps) {

  const [sigma, setSigma] = useState(5.0);
  const [showOverlay, setShowOverlay] = useState(true);
  const [selectedStar, setSelectedStar] = useState<number | null>(null);

  const [solveLoading, setSolveLoading] = useState(false);
  const [solveResult, setSolveResult] = useState<SolveResult | null>(null);
  const [solveError, setSolveError] = useState<string | null>(null);
  const [hasApiKey, setHasApiKey] = useState(false);
  const [scaleLow, setScaleLow] = useState(0.1);
  const [scaleHigh, setScaleHigh] = useState(10.0);
  const [wcsInfo, setWcsInfo] = useState<WcsInfo | null>(null);

  useEffect(() => {
    getApiKey()
      .then((r: any) => setHasApiKey(!!r?.key || !!r?.apiKey))
      .catch(() => setHasApiKey(false));
  }, []);

  useEffect(() => {
    if (!filePath) {
      setWcsInfo(null);
      setSolveResult(null);
      return;
    }
    getWcsInfo(filePath)
      .then((info: WcsInfo) => setWcsInfo(info))
      .catch(() => setWcsInfo(null));
  }, [filePath]);

  useEffect(() => {
    const canvas = overlayCanvasRef?.current;
    if (!canvas || stars.length === 0) return;

    const parent = canvas.parentElement;
    if (!parent) return;

    const W = parent.clientWidth;
    const H = parent.clientHeight;
    canvas.width = W;
    canvas.height = H;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, W, H);

    if (!showOverlay) return;

    const scaleX = W / (imageWidth || 1);
    const scaleY = H / (imageHeight || 1);

    const maxFlux = stars.length > 0 ? stars[0].flux : 1;

    stars.forEach((star, i) => {
      const sx = star.x * scaleX;
      const sy = star.y * scaleY;
      const radius = Math.max(3, (star.fwhm || 3) * scaleX * 1.5);
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
      const sx = s.x * scaleX;
      const sy = s.y * scaleY;
      const radius = Math.max(6, (s.fwhm || 3) * scaleX * 2);

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
  }, [stars, showOverlay, selectedStar, imageWidth, imageHeight, overlayCanvasRef]);

  useEffect(() => {
    const canvas = overlayCanvasRef?.current;
    if (!canvas) return;
    canvas.style.display = showOverlay && stars.length > 0 ? "block" : "none";
  }, [showOverlay, stars.length, overlayCanvasRef]);

  const handleDetect = useCallback(() => {
    if (onDetect) onDetect(sigma);
  }, [onDetect, sigma]);

  const handleSolve = useCallback(async () => {
    if (!filePath) return;
    setSolveLoading(true);
    setSolveError(null);
    setSolveResult(null);
    try {
      const result = await plateSolve(filePath, {
        scaleLower: scaleLow,
        scaleUpper: scaleHigh,
        scaleUnits: "arcsecperpix",
      }) as SolveResult;
      setSolveResult(result);
      if (result.success) {
        getWcsInfo(filePath)
          .then((info: WcsInfo) => setWcsInfo(info))
          .catch(() => {});
      }
    } catch (e: unknown) {
      setSolveError(e instanceof Error ? e.message : String(e));
    } finally {
      setSolveLoading(false);
    }
  }, [filePath, scaleLow, scaleHigh]);

  const medianFwhm = stars.length > 0
    ? (stars.reduce((s, st) => s + st.fwhm, 0) / stars.length).toFixed(2)
    : null;

  const activeWcs = solveResult?.success ? solveResult : wcsInfo;

  return (
    <div className="flex flex-col gap-3">
      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
        <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
          <div className="flex items-center gap-2">
            <Crosshair size={12} className="text-cyan-400" />
            <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
              Star Detection
            </span>
          </div>
          <div className="flex items-center gap-2">
            {stars.length > 0 && (
              <button
                onClick={() => setShowOverlay(!showOverlay)}
                className="text-zinc-500 hover:text-zinc-300 transition-colors"
                title={showOverlay ? "Hide overlay" : "Show overlay"}
              >
                {showOverlay ? <Eye size={12} /> : <EyeOff size={12} />}
              </button>
            )}
          </div>
        </div>

        <div className="px-3 py-2 space-y-2">
          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-12">σ thresh</label>
            <input
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

          {stars.length > 0 && (
            <div className="space-y-1.5">
              <div className="grid grid-cols-3 gap-1 text-[10px]">
                <div className="bg-zinc-900/80 rounded px-2 py-1">
                  <div className="text-zinc-500">Stars</div>
                  <div className="text-cyan-300 font-mono">{count}</div>
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
                      <th className="text-right px-1">X</th>
                      <th className="text-right px-1">Y</th>
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

      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
        <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
          <div className="flex items-center gap-2">
            <Compass size={12} className="text-emerald-400" />
            <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
              Plate Solve
            </span>
          </div>
          {activeWcs && (
            <Globe size={12} className="text-emerald-400/60" />
          )}
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
              <label className="text-[9px] text-zinc-500 uppercase">Scale low ("/px)</label>
              <input
                type="number"
                min={0.01}
                max={100}
                step={0.1}
                value={scaleLow}
                onChange={(e) => setScaleLow(parseFloat(e.target.value) || 0.1)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono outline-none focus:border-emerald-500/50 w-full"
              />
            </div>
            <div className="flex-1 flex flex-col gap-0.5">
              <label className="text-[9px] text-zinc-500 uppercase">Scale high ("/px)</label>
              <input
                type="number"
                min={0.01}
                max={100}
                step={0.1}
                value={scaleHigh}
                onChange={(e) => setScaleHigh(parseFloat(e.target.value) || 10.0)}
                className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono outline-none focus:border-emerald-500/50 w-full"
              />
            </div>
          </div>

          <button
            onClick={handleSolve}
            disabled={solveLoading || !filePath}
            className="w-full flex items-center justify-center gap-2 bg-emerald-600/20 hover:bg-emerald-600/30 text-emerald-300 border border-emerald-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
          >
            {solveLoading ? (
              <><Loader2 size={12} className="animate-spin" /> Solving...</>
            ) : (
              <><Compass size={12} /> Plate Solve</>
            )}
          </button>

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

          {solveResult?.success && (
            <div className="grid grid-cols-2 gap-1.5 text-[10px]">
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Center RA</div>
                <div className="text-emerald-300 font-mono">{formatRA(solveResult.ra_center!)}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Center Dec</div>
                <div className="text-emerald-300 font-mono">{formatDec(solveResult.dec_center!)}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Pixel Scale</div>
                <div className="text-emerald-300 font-mono">{solveResult.pixel_scale?.toFixed(3)}"/px</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">FOV</div>
                <div className="text-emerald-300 font-mono">
                  {solveResult.field_w_arcmin?.toFixed(1)}' x {solveResult.field_h_arcmin?.toFixed(1)}'
                </div>
              </div>
              {solveResult.orientation !== undefined && (
                <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
                  <div className="text-zinc-500">Orientation</div>
                  <div className="text-emerald-300 font-mono">{solveResult.orientation?.toFixed(2)}°</div>
                </div>
              )}
            </div>
          )}

          {!solveResult && activeWcs && (activeWcs as WcsInfo).center_ra !== undefined && (() => {
            const wcs = activeWcs as WcsInfo;
            return (
              <div className="grid grid-cols-2 gap-1.5 text-[10px]">
                <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                  <div className="text-zinc-500">Center RA</div>
                  <div className="text-zinc-300 font-mono">{formatRA(wcs.center_ra)}</div>
                </div>
                <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                  <div className="text-zinc-500">Center Dec</div>
                  <div className="text-zinc-300 font-mono">{formatDec(wcs.center_dec)}</div>
                </div>
                {wcs.pixel_scale_arcsec && (
                  <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                    <div className="text-zinc-500">Pixel Scale</div>
                    <div className="text-zinc-300 font-mono">{wcs.pixel_scale_arcsec.toFixed(3)}"/px</div>
                  </div>
                )}
                {wcs.fov_arcmin && (
                  <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                    <div className="text-zinc-500">FOV</div>
                    <div className="text-zinc-300 font-mono">
                      {wcs.fov_arcmin[0].toFixed(1)}' x {wcs.fov_arcmin[1].toFixed(1)}'
                    </div>
                  </div>
                )}
              </div>
            );
          })()}
        </div>
      </div>
    </div>
  );
}
