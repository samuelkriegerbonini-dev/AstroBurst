import { memo, useCallback, useId } from "react";
import { useTimedFlag } from "../../hooks/useTimedNotice";
import { Activity, Copy } from "lucide-react";
import type { LineMeasurement, LineModel } from "../../shared/types/spectral";
import { LINE_MODEL_LABELS, LINE_MODELS } from "../../shared/types/spectral";
import { lineMeasurementCsv, lineMeasurementRows, spectrumSourceLabel } from "../../utils/lineMeasure";

export interface LineErrControls {
  useErr: boolean;
  onUseErr: (useErr: boolean) => void;
  useDq: boolean;
  onUseDq: (useDq: boolean) => void;
  errAvailable: boolean;
  dqAvailable: boolean;
}

interface LineMeasurementSectionProps {
  result: LineMeasurement | null;
  loading: boolean;
  error: string | null;
  model: LineModel;
  onModelChange: (model: LineModel) => void;
  onMeasure: () => void;
  disabled: boolean;
  disabledHint: string | null;
  errControls?: LineErrControls | null;
}

const ACCENT = "var(--ab-violet)";
const COPY_FEEDBACK_MS = 1500;
const SELECT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 focus:border-violet-500/50 disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";

function MeasureBtn({
  label,
  loading,
  disabled,
  onClick,
}: {
  label: string;
  loading: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all disabled:opacity-40"
      style={{
        background: `color-mix(in srgb, ${ACCENT} 8%, transparent)`,
        border: `1px solid color-mix(in srgb, ${ACCENT} 20%, transparent)`,
        color: ACCENT,
      }}
    >
      {loading ? (
        <div className="w-2.5 h-2.5 rounded-full animate-spin" style={{ border: "1.5px solid transparent", borderTopColor: ACCENT }} />
      ) : (
        <Activity size={10} />
      )}
      {label}
    </button>
  );
}

function LineMeasurementSection({
  result,
  loading,
  error,
  model,
  onModelChange,
  onMeasure,
  disabled,
  disabledHint,
  errControls = null,
}: LineMeasurementSectionProps) {
  const modelId = useId();
  const [copied, flagCopied, clearCopied] = useTimedFlag(COPY_FEEDBACK_MS);

  const handleCopy = useCallback(async () => {
    if (!result) return;
    try {
      await navigator.clipboard.writeText(lineMeasurementCsv(result));
      flagCopied();
    } catch {
      clearCopied();
    }
  }, [result, flagCopied, clearCopied]);

  const rows = result ? lineMeasurementRows(result) : [];
  const fit = result?.fit ?? null;

  return (
    <div className="px-3 pb-2 flex flex-col gap-2" style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}>
      <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
        <span className="flex items-center gap-1.5">
          <Activity size={12} style={{ color: ACCENT }} />
          <span className="text-[11px] font-semibold text-zinc-400 uppercase tracking-wider">Line</span>
        </span>
        <label htmlFor={modelId} className={LABEL_CLASS}>
          Model
        </label>
        <select
          id={modelId}
          value={model}
          onChange={(e) => onModelChange(e.target.value as LineModel)}
          className={SELECT_CLASS}
          disabled={loading}
        >
          {LINE_MODELS.map((m) => (
            <option key={m} value={m}>
              {LINE_MODEL_LABELS[m]}
            </option>
          ))}
        </select>
        <MeasureBtn label="Measure line" loading={loading} disabled={disabled} onClick={onMeasure} />
        {errControls && (
          <>
            <label className="flex items-center gap-1 text-zinc-500 cursor-pointer">
              <input
                type="checkbox"
                aria-label="Weight by ERR"
                checked={errControls.useErr}
                onChange={(e) => errControls.onUseErr(e.target.checked)}
                disabled={loading}
              />
              Weight by ERR
            </label>
            {!errControls.errAvailable && <span className="text-zinc-600">no ERR in this file</span>}
            <label className="flex items-center gap-1 text-zinc-500 cursor-pointer">
              <input
                type="checkbox"
                aria-label="Mask DQ channels"
                checked={errControls.useDq}
                onChange={(e) => errControls.onUseDq(e.target.checked)}
                disabled={loading}
              />
              Mask DQ channels
            </label>
            {!errControls.dqAvailable && <span className="text-zinc-600">no DQ in this file</span>}
          </>
        )}
        {disabled && !loading && disabledHint && <span className="text-zinc-600">{disabledHint}</span>}
        {result && (
          <button
            onClick={handleCopy}
            className="ml-auto flex items-center gap-1 text-zinc-500 hover:text-zinc-300"
            title="Copy the measurement as one CSV row"
          >
            <Copy size={10} />
            {copied ? "copied" : "Copy CSV"}
          </button>
        )}
      </div>

      {error && (
        <p className="text-[10px] text-red-400/80 px-2 py-1 rounded bg-red-900/15 border border-red-800/20">{error}</p>
      )}

      {result && (
        <>
          <div className="grid grid-cols-2 gap-1.5">
            {rows.map((row) => (
              <div key={row.label} className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-[9px] text-zinc-500 uppercase tracking-wider">{row.label}</div>
                <div className="text-[10px] font-mono text-zinc-200 break-words">{row.value}</div>
              </div>
            ))}
            {fit && (
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-[9px] text-zinc-500 uppercase tracking-wider">Fit status</div>
                <div className="text-[10px] font-mono" style={{ color: fit.converged ? "#34d399" : "var(--ab-amber)" }}>
                  {fit.converged ? "converged" : "did not converge"}
                </div>
              </div>
            )}
          </div>
          <div className="text-[10px] font-mono text-zinc-500 flex items-center gap-2 flex-wrap">
            <span>
              {spectrumSourceLabel(result.source)}, ch {result.z0}-{result.z1}, {result.n_channels} channels, continuum{" "}
              {result.continuum_windows[0][0]}-{result.continuum_windows[0][1]} and {result.continuum_windows[1][0]}-
              {result.continuum_windows[1][1]}
            </span>
            <span className="ml-auto">{result.elapsed_ms}ms</span>
          </div>
          {result.notes.length > 0 && (
            <details className="text-[10px] font-mono text-zinc-600">
              <summary className="cursor-pointer select-none">notes ({result.notes.length})</summary>
              <ul className="mt-0.5 flex flex-col gap-0.5">
                {result.notes.map((note, i) => (
                  <li key={i} className="break-words">
                    {note}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </>
      )}
    </div>
  );
}

export default memo(LineMeasurementSection);
