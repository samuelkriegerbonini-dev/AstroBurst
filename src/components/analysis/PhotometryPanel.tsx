import { useState, useEffect, useRef, useCallback, memo } from "react";
import { Crosshair, Loader2, Star as StarIcon } from "lucide-react";
import { measurePhotometry } from "../../services/analysis";
import type { PhotometryMeasurement } from "../../services/analysis";
import { usePixelClick } from "../../hooks/useMousePixelStore";
import { Toggle } from "../ui";

interface PhotometryPanelProps {
  filePath?: string | null;
}

function formatMag(v: number | null | undefined): string {
  return v == null || !isFinite(v) ? "--" : v.toFixed(2);
}

function PhotometryPanel({ filePath }: PhotometryPanelProps) {
  const [armed, setArmed] = useState(false);
  const [gaiaMatch, setGaiaMatch] = useState(true);
  const [isMeasuring, setIsMeasuring] = useState(false);
  const [result, setResult] = useState<PhotometryMeasurement | null>(null);
  const [history, setHistory] = useState<PhotometryMeasurement[]>([]);
  const [error, setError] = useState<string | null>(null);
  const click = usePixelClick();
  const lastSeqRef = useRef(0);
  const busyRef = useRef(false);

  const measure = useCallback(
    async (x: number, y: number) => {
      if (!filePath || busyRef.current) return;
      busyRef.current = true;
      setIsMeasuring(true);
      setError(null);
      try {
        const res = await measurePhotometry(filePath, x, y, { gaiaMatch });
        setResult(res);
        setHistory((prev) => [res, ...prev].slice(0, 4));
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        busyRef.current = false;
        setIsMeasuring(false);
      }
    },
    [filePath, gaiaMatch],
  );

  useEffect(() => {
    if (!armed || !click || click.seq === lastSeqRef.current) return;
    lastSeqRef.current = click.seq;
    measure(click.x, click.y);
  }, [armed, click, measure]);

  const phot = result?.photometry;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <StarIcon size={12} className="text-yellow-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            Photometry
          </span>
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
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Centroid</div>
              <div className="text-yellow-300 font-mono">
                {phot.x.toFixed(2)}, {phot.y.toFixed(2)}
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Net Flux</div>
              <div className="text-yellow-300 font-mono">{phot.net_flux.toExponential(3)}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Mag (inst)</div>
              <div className="text-yellow-300 font-mono">{formatMag(phot.mag_inst)}</div>
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
              <div className="text-zinc-500">Background</div>
              <div className="text-zinc-300 font-mono">
                {phot.bg_mean.toExponential(2)} ± {phot.bg_sigma.toExponential(2)}
              </div>
            </div>
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
                Star appears saturated — flux and magnitude are unreliable.
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
                <span>mag {formatMag(m.photometry.mag_inst)}</span>
                <span>SNR {m.photometry.snr.toFixed(0)}</span>
                <span>{m.gaia ? `G ${formatMag(m.gaia.gmag)}` : "--"}</span>
              </div>
            ))}
          </div>
        )}

        {!phot && !error && (
          <div className="text-[10px] text-zinc-600">
            Aperture photometry with local background annulus. Magnitude is instrumental
            (−2.5·log₁₀ flux); a Gaia match adds the catalog G magnitude for calibration.
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(PhotometryPanel);
