import { useState, useEffect, useRef, useCallback, memo } from "react";
import { Crosshair, Loader2, Star as StarIcon } from "lucide-react";
import { measurePhotometry } from "../../services/analysis";
import type { PhotometryMeasurement, StarPhotometry } from "../../services/analysis";
import { usePixelClick } from "../../hooks/useMousePixelStore";
import { useDqContext } from "../../context/PreviewContext";
import { Toggle } from "../ui";
import MeasurementBadge from "./MeasurementBadge";

interface PhotometryPanelProps {
  filePath?: string | null;
}

function formatMag(v: number | null | undefined): string {
  return v == null || !isFinite(v) ? "--" : v.toFixed(2);
}

function formatMagWithError(mag: number | null | undefined, err: number | null | undefined): string {
  if (mag == null || !isFinite(mag)) return "--";
  const base = mag.toFixed(3);
  return err != null && isFinite(err) ? `${base} ± ${err.toFixed(3)}` : base;
}

function formatSci(v: number): string {
  return isFinite(v) ? v.toExponential(3) : "--";
}

function formatWithError(value: number, err: number | null | undefined): string {
  const base = formatSci(value);
  return err != null && isFinite(err) && err > 0 ? `${base} ± ${err.toExponential(2)}` : base;
}

function formatJansky(v: number | null | undefined): string {
  if (v == null || !isFinite(v)) return "--";
  const a = Math.abs(v);
  if (a >= 1) return `${v.toFixed(4)} Jy`;
  if (a >= 1e-3) return `${(v * 1e3).toFixed(4)} mJy`;
  if (a >= 1e-6) return `${(v * 1e6).toFixed(4)} µJy`;
  return `${(v * 1e9).toFixed(4)} nJy`;
}

function fluxUnitLabel(bunit: string | null | undefined): string {
  return bunit ? ` ${bunit}` : "";
}

function bestMag(p: StarPhotometry): string {
  return p.mag_ab != null ? `AB ${formatMag(p.mag_ab)}` : `inst ${formatMag(p.mag_inst)}`;
}

function PhotometryPanel({ filePath }: PhotometryPanelProps) {
  const [armed, setArmed] = useState(false);
  const [gaiaMatch, setGaiaMatch] = useState(true);
  const [isMeasuring, setIsMeasuring] = useState(false);
  const [result, setResult] = useState<PhotometryMeasurement | null>(null);
  const [history, setHistory] = useState<PhotometryMeasurement[]>([]);
  const [error, setError] = useState<string | null>(null);
  const click = usePixelClick();
  const { excludeDq } = useDqContext();
  const lastSeqRef = useRef(0);
  const busyRef = useRef(false);
  const requestSeqRef = useRef(0);
  const clickRef = useRef(click);
  clickRef.current = click;

  useEffect(() => {
    requestSeqRef.current++;
    lastSeqRef.current = clickRef.current?.seq ?? lastSeqRef.current;
    setResult(null);
    setHistory([]);
    setError(null);
    setIsMeasuring(false);
    busyRef.current = false;
  }, [filePath]);

  const measure = useCallback(
    async (x: number, y: number) => {
      if (!filePath || busyRef.current) return;
      const seq = ++requestSeqRef.current;
      busyRef.current = true;
      setIsMeasuring(true);
      setError(null);
      try {
        const res = await measurePhotometry(filePath, x, y, { gaiaMatch, excludeDq });
        if (requestSeqRef.current !== seq) return;
        setResult(res);
        setHistory((prev) => [res, ...prev].slice(0, 4));
      } catch (e: unknown) {
        if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
      } finally {
        busyRef.current = false;
        if (requestSeqRef.current === seq) setIsMeasuring(false);
      }
    },
    [filePath, gaiaMatch, excludeDq],
  );

  useEffect(() => {
    if (!armed || !click || click.seq === lastSeqRef.current) return;
    lastSeqRef.current = click.seq;
    measure(click.x, click.y);
  }, [armed, click, measure]);

  const phot = result?.photometry;
  const photcal = result?.photcal ?? null;
  const warnings = result?.warnings ?? [];
  const unit = fluxUnitLabel(photcal?.bunit);

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <StarIcon size={12} className="text-yellow-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            Photometry
          </span>
          <MeasurementBadge />
        </div>
        {isMeasuring && <Loader2 size={12} className="animate-spin text-yellow-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <Toggle label="Measure on image click" checked={armed} accent="amber" onChange={setArmed} />
        <Toggle label="Match Gaia DR3 (online)" checked={gaiaMatch} accent="amber" onChange={setGaiaMatch} />

        {armed && (
          <div className="flex items-center gap-1.5 text-[10px] text-yellow-400/70">
            <Crosshair size={10} />
            <span>Enable crosshair mode in the viewer toolbar, then click a star.</span>
          </div>
        )}

        {error && (
          <div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
            {error}
          </div>
        )}

        {phot && (
          <div className="grid grid-cols-2 gap-1.5 text-[10px]">
            <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2 flex flex-wrap items-center gap-1.5">
              <span className="text-zinc-500">Calibration</span>
              <span className={photcal ? "text-zinc-300" : "text-amber-400/90"}>
                {photcal ? photcal.label : "no calibration in header"}
              </span>
              {phot.err_used && (
                <span className="text-[9px] px-1.5 py-0.5 rounded text-emerald-300 bg-emerald-900/30">ERR plane</span>
              )}
              {phot.n_masked > 0 && (
                <span className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30">
                  {phot.n_masked} masked
                </span>
              )}
              {phot.n_saturated > 0 && (
                <span className="text-[9px] px-1.5 py-0.5 rounded text-red-300 bg-red-900/30">
                  {phot.n_saturated} saturated
                </span>
              )}
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Centroid</div>
              <div className="text-yellow-300 font-mono">
                {phot.x.toFixed(2)}, {phot.y.toFixed(2)}
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Net flux{unit}</div>
              <div className="text-yellow-300 font-mono">{formatWithError(phot.net_flux, phot.flux_err)}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">AB mag</div>
              <div className="text-yellow-300 font-mono">{formatMagWithError(phot.mag_ab, phot.mag_ab_err)}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Flux (Jy)</div>
              <div className="text-yellow-300 font-mono">
                {phot.flux_jy != null ? formatJansky(phot.flux_jy) : "--"}
                {phot.flux_err_jy != null && phot.flux_jy != null && (
                  <span className="text-zinc-500"> ± {formatJansky(phot.flux_err_jy)}</span>
                )}
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Mag (inst){phot.st_mag != null ? " / ST" : ""}</div>
              <div className="text-zinc-300 font-mono">
                {formatMag(phot.mag_inst)}
                {phot.st_mag != null ? ` / ${formatMag(phot.st_mag)}` : ""}
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">SNR</div>
              <div className="text-yellow-300 font-mono">{phot.snr.toFixed(1)}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">FWHM</div>
              <div className="text-yellow-300 font-mono">{phot.fwhm.toFixed(2)} px</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Aperture</div>
              <div className="text-zinc-300 font-mono">
                r={phot.aperture_radius.toFixed(1)} ({phot.aperture_pixels}px)
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
              <div className="text-zinc-500">Background ({phot.bg_pixels}px)</div>
              <div className="text-zinc-300 font-mono">
                {phot.bg_mean.toExponential(2)} ± {phot.bg_sigma.toExponential(2)}
              </div>
            </div>
            {phot.aperture_correction != null && (
              <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
                <div className="text-zinc-500">
                  Aperture correction
                  {phot.plateau_radius != null ? ` (growth curve plateau at r=${phot.plateau_radius.toFixed(0)} px)` : ""}
                </div>
                <div className="text-zinc-300 font-mono">
                  {phot.aperture_correction.toFixed(4)}
                  {phot.flux_total != null ? ` → total ${formatSci(phot.flux_total)}${unit}` : ""}
                  {phot.mag_ab_total != null ? `, AB ${formatMag(phot.mag_ab_total)}` : ""}
                </div>
              </div>
            )}
            {result?.sky && (
              <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
                <div className="text-zinc-500">Sky (RA, Dec)</div>
                <div className="text-zinc-300 font-mono">
                  {result.sky.ra.toFixed(5)}°, {result.sky.dec.toFixed(5)}°
                </div>
              </div>
            )}
            {result?.gaia && (
              <div className="bg-emerald-900/20 border border-emerald-800/20 rounded px-2 py-1.5 col-span-2">
                <div className="text-emerald-500/80">Gaia DR3 match ({result.gaia.separation_arcsec.toFixed(1)}")</div>
                <div className="text-emerald-300 font-mono">
                  G = {formatMag(result.gaia.gmag)}  BP−RP = {result.gaia.bp_rp.toFixed(3)}
                </div>
              </div>
            )}
            {phot.saturated && (
              <div className="text-[10px] text-amber-400/90 bg-amber-900/20 border border-amber-800/30 rounded px-2 py-1.5 col-span-2">
                Star appears saturated (level from {phot.saturation_source}) — flux and magnitude are unreliable.
              </div>
            )}
            {warnings.length > 0 && (
              <div className="text-[9px] text-amber-300/90 bg-amber-900/15 border border-amber-800/30 rounded px-2 py-1 col-span-2 space-y-0.5">
                {warnings.map((w, i) => (
                  <div key={i}>{w}</div>
                ))}
              </div>
            )}
          </div>
        )}

        {history.length > 1 && (
          <div className="flex flex-col gap-0.5">
            <span className="text-[9px] text-zinc-600 uppercase">Previous</span>
            {history.slice(1).map((m, i) => (
              <div key={i} className="flex justify-between text-[9px] font-mono text-zinc-500">
                <span>
                  ({m.photometry.x.toFixed(0)}, {m.photometry.y.toFixed(0)})
                </span>
                <span>{bestMag(m.photometry)}</span>
                <span>SNR {m.photometry.snr.toFixed(0)}</span>
                <span>{m.gaia ? `G ${formatMag(m.gaia.gmag)}` : "--"}</span>
              </div>
            ))}
          </div>
        )}

        {!phot && !error && (
          <div className="text-[10px] text-zinc-600">
            Aperture photometry with a local background annulus and ERR-plane error propagation.
            Zero points are read from the header (JWST, HST, Roman or MAGZPT-style keywords) to
            give AB magnitudes and fluxes in Jy; a Gaia match adds the catalog G magnitude.
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(PhotometryPanel);
