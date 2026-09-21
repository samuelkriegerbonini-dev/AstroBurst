import { useEffect, useId, useMemo, useRef, useState, memo } from "react";
import { Globe, Loader2 } from "lucide-react";
import { getRadialVelocityCorrection, isCorrectionError } from "../../services/spectral";
import type {
  CorrectionFrame,
  RadialVelocityCorrectionResponse,
  RadialVelocityCorrectionResult,
  SpectralAxisInfo,
  SpectralAxisMode,
  VelocityConvention,
} from "../../shared/types/spectral";
import {
  CORRECTION_FRAMES,
  VELOCITY_CONVENTIONS,
  availableModes,
  conventionLabel,
  defaultRestUm,
  modeLabel,
  parseRestInput,
} from "../../utils/spectralAxis";

export interface SpectralAxisControlsProps {
  filePath: string | null;
  axis: SpectralAxisInfo | null;
  mode: SpectralAxisMode;
  onModeChange: (mode: SpectralAxisMode) => void;
  restValue: number | null;
  onRestChange: (restUm: number | null) => void;
  convention: VelocityConvention;
  onConventionChange: (convention: VelocityConvention) => void;
  correction: CorrectionFrame;
  onCorrectionChange: (frame: CorrectionFrame) => void;
  onCorrectionLoaded?: (correction: RadialVelocityCorrectionResult | null) => void;
}

const SELECT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 focus:border-violet-500/50 w-full disabled:opacity-40";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs font-mono text-zinc-200 focus:border-violet-500/50 w-full disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";

function frameLabel(frame: CorrectionFrame): string {
  return frame === "none" ? "None" : frame.charAt(0).toUpperCase() + frame.slice(1);
}

function signed(value: number, decimals: number): string {
  if (!Number.isFinite(value)) return "—";
  return `${value >= 0 ? "+" : ""}${value.toFixed(decimals)}`;
}

function SpectralAxisControls({
  filePath,
  axis,
  mode,
  onModeChange,
  restValue,
  onRestChange,
  convention,
  onConventionChange,
  correction,
  onCorrectionChange,
  onCorrectionLoaded,
}: SpectralAxisControlsProps) {
  const axisId = useId();
  const restId = useId();
  const conventionId = useId();
  const correctionId = useId();
  const modes = useMemo(() => availableModes(axis), [axis]);
  const [restText, setRestText] = useState(() => (restValue === null ? "" : String(restValue)));
  const [response, setResponse] = useState<RadialVelocityCorrectionResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const prefilledAxis = useRef<SpectralAxisInfo | null>(restValue === null ? null : axis);
  const loadedRef = useRef(onCorrectionLoaded);
  loadedRef.current = onCorrectionLoaded;

  useEffect(() => {
    if (axis === prefilledAxis.current) return;
    prefilledAxis.current = axis;
    onRestChange(defaultRestUm(axis));
  }, [axis, onRestChange]);

  useEffect(() => {
    setRestText((current) =>
      parseRestInput(current) === restValue ? current : restValue === null ? "" : String(restValue),
    );
  }, [restValue]);

  useEffect(() => {
    if (modes.length > 0 && !modes.includes(mode)) onModeChange(modes[0]);
  }, [modes, mode, onModeChange]);

  useEffect(() => {
    if (!filePath) {
      setResponse(null);
      setLoading(false);
      loadedRef.current?.(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setResponse(null);
    loadedRef.current?.(null);
    getRadialVelocityCorrection(filePath)
      .then((result) => {
        if (cancelled) return;
        setResponse(result);
        loadedRef.current?.(isCorrectionError(result) ? null : result);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setResponse({ error: e instanceof Error ? e.message : String(e) });
        loadedRef.current?.(null);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath]);

  const result = response && !isCorrectionError(response) ? response : null;
  const failure = response && isCorrectionError(response) ? response.error : null;

  return (
    <div className="flex flex-col gap-2 px-3 pb-2" style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}>
      <div className="flex gap-2">
        <div className="flex-1 flex flex-col gap-0.5">
          <label htmlFor={axisId} className={LABEL_CLASS}>
            Axis
          </label>
          <select
            id={axisId}
            value={mode}
            disabled={modes.length === 0}
            onChange={(e) => onModeChange(e.target.value as SpectralAxisMode)}
            className={SELECT_CLASS}
          >
            {modes.length === 0 && <option value={mode}>No spectral axis</option>}
            {modes.map((m) => (
              <option key={m} value={m}>
                {modeLabel(m)}
              </option>
            ))}
          </select>
        </div>
        <div className="flex-1 flex flex-col gap-0.5">
          <label htmlFor={restId} className={LABEL_CLASS}>
            {"Rest wavelength, vacuum (μm)"}
          </label>
          <input
            id={restId}
            type="number"
            min={0}
            step={0.0001}
            value={restText}
            placeholder="e.g. 0.65646 (H-alpha, vacuum)"
            title="Vacuum rest wavelength in micrometres; convert air line-list values first (H-alpha air 0.65628 = vacuum 0.65646)"
            disabled={modes.length === 0}
            onChange={(e) => {
              setRestText(e.target.value);
              onRestChange(parseRestInput(e.target.value));
            }}
            className={INPUT_CLASS}
          />
        </div>
      </div>

      {mode === "velocity" && (
        <div className="flex gap-2">
          <div className="flex-1 flex flex-col gap-0.5">
            <label htmlFor={conventionId} className={LABEL_CLASS}>
              Convention
            </label>
            <select
              id={conventionId}
              value={convention}
              onChange={(e) => onConventionChange(e.target.value as VelocityConvention)}
              className={SELECT_CLASS}
            >
              {VELOCITY_CONVENTIONS.map((c) => (
                <option key={c} value={c}>
                  {conventionLabel(c)}
                </option>
              ))}
            </select>
          </div>
          <div className="flex-1 flex flex-col gap-0.5">
            <label htmlFor={correctionId} className={LABEL_CLASS}>
              Apply correction
            </label>
            <select
              id={correctionId}
              value={correction}
              disabled={result === null}
              onChange={(e) => onCorrectionChange(e.target.value as CorrectionFrame)}
              className={SELECT_CLASS}
            >
              {CORRECTION_FRAMES.map((f) => (
                <option key={f} value={f}>
                  {frameLabel(f)}
                </option>
              ))}
            </select>
          </div>
        </div>
      )}

      <div
        className="rounded-md px-2 py-1.5 text-[10px] font-mono"
        style={{ background: "rgba(9,9,11,0.6)", border: "1px solid var(--ab-border)" }}
      >
        <div className="flex items-center gap-1.5 text-zinc-400">
          <Globe size={10} style={{ color: "var(--ab-violet)" }} />
          <span className="font-semibold uppercase tracking-wider text-[9px]">Correction</span>
          {loading && <Loader2 size={10} className="animate-spin text-zinc-500" />}
          {axis?.specsys && <span className="ml-auto text-zinc-600">SPECSYS {axis.specsys}</span>}
        </div>
        {!filePath && <p className="mt-1 text-zinc-600">Load a spectrum or cube to compute the correction.</p>}
        {failure && <p className="mt-1 text-red-400/80 break-words">{failure}</p>}
        {result && (
          <div className="mt-1 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-zinc-300">
            <span className="text-zinc-500">Barycentric</span>
            <span>{signed(result.barycentric_kms, 4)} km/s</span>
            <span className="text-zinc-500">Heliocentric</span>
            <span>{signed(result.heliocentric_kms, 4)} km/s</span>
            <span className="text-zinc-500">Method</span>
            <span className="break-words">
              {result.method}
              {result.accuracy_kms > 0 ? ` (±${result.accuracy_kms} km/s)` : ""}
            </span>
            <span className="text-zinc-500">Target</span>
            <span>
              {result.ra_deg.toFixed(5)} {signed(result.dec_deg, 5)} ({result.coordinate_source})
            </span>
            {Number.isFinite(result.jd_mid) && (
              <>
                <span className="text-zinc-500">JD mid</span>
                <span>{result.jd_mid.toFixed(6)}</span>
              </>
            )}
          </div>
        )}
        {result && result.notes.length > 0 && (
          <details className="mt-1 text-zinc-600">
            <summary className="cursor-pointer select-none">notes ({result.notes.length})</summary>
            <ul className="mt-0.5 flex flex-col gap-0.5 text-zinc-500">
              {result.notes.map((note, i) => (
                <li key={i} className="break-words">
                  {note}
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>

      {axis && axis.notes.length > 0 && (
        <details className="text-[10px] font-mono text-zinc-600">
          <summary className="cursor-pointer select-none">axis notes ({axis.notes.length})</summary>
          <ul className="mt-0.5 flex flex-col gap-0.5 text-zinc-500">
            {axis.notes.map((note, i) => (
              <li key={i} className="break-words">
                {note}
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

export default memo(SpectralAxisControls);
