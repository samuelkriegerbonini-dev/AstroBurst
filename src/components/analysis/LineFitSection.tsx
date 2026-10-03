import { memo, useId } from "react";
import { Activity } from "lucide-react";
import type { LineFitComponents, LineFitPlane, LineFitSpaxel } from "../../shared/types/cube";
import { LINE_FIT_COMPONENTS } from "../../shared/types/cube";
import LineFitInspectPlot from "./LineFitInspectPlot";
import {
  RESOLVING_POWER_HINT,
  lineFitSpaxelNotes,
  lineFitSpaxelRows,
  lineFitSummary,
  planeLabel,
  planeUnit,
  sigmaCaption,
  type LineFitRun,
} from "../../utils/lineFit";

interface LineFitSectionProps {
  ready: boolean;
  readyHint: string | null;
  loading: boolean;
  error: string | null;
  errAvailable: boolean | null;
  dqAvailable: boolean | null;
  useErr: boolean;
  onUseErr(v: boolean): void;
  useDq: boolean;
  onUseDq(v: boolean): void;
  emissionOnly: boolean;
  onEmissionOnly(v: boolean): void;
  snrText: string;
  onSnrText(v: string): void;
  resolvingPowerText: string;
  onResolvingPowerText(v: string): void;
  components: LineFitComponents;
  onComponents(v: LineFitComponents): void;
  onFit(): void;
  run: LineFitRun | null;
  shownPlane: LineFitPlane | null;
  onShowPlane(plane: LineFitPlane): void;
  inspect: LineFitSpaxel | null;
  inspectLoading: boolean;
  inspectError: string | null;
  frameNote: string;
}

type LineFitStatus = "idle" | "running" | "done" | "error";

const ACCENT = "var(--ab-emerald)";
const INSPECT_HINT = "click a spaxel on the displayed map to see its fit";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] font-mono text-zinc-200 focus:border-violet-500/50 w-14 disabled:opacity-40";
const SELECT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 focus:border-violet-500/50 disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const ERROR_CLASS = "text-[10px] text-red-400/80 px-2 py-1 rounded bg-red-900/15 border border-red-800/20";

function FitBtn({ loading, disabled, onClick }: { loading: boolean; disabled: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      aria-label="Fit lines"
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
      Fit lines
    </button>
  );
}

function availabilityText(name: string, available: boolean | null): string | null {
  if (available === null) return null;
  return available ? `${name} found` : `no ${name} cube`;
}

function lineFitStatus(loading: boolean, error: string | null, run: LineFitRun | null): LineFitStatus {
  if (loading) return "running";
  if (error) return "error";
  return run ? "done" : "idle";
}

function LineFitSection({
  ready,
  readyHint,
  loading,
  error,
  errAvailable,
  dqAvailable,
  useErr,
  onUseErr,
  useDq,
  onUseDq,
  emissionOnly,
  onEmissionOnly,
  snrText,
  onSnrText,
  resolvingPowerText,
  onResolvingPowerText,
  components,
  onComponents,
  onFit,
  run,
  shownPlane,
  onShowPlane,
  inspect,
  inspectLoading,
  inspectError,
  frameNote,
}: LineFitSectionProps) {
  const snrId = useId();
  const resolvingPowerId = useId();
  const componentsId = useId();
  const result = run?.result ?? null;
  const rows = inspect && result ? lineFitSpaxelRows(inspect, result.units, result.resolving_power) : [];
  const spaxelNotes = inspect && result ? lineFitSpaxelNotes(inspect, result.notes) : [];
  const errText = availabilityText("ERR", errAvailable);
  const dqText = availabilityText("DQ", dqAvailable);

  return (
    <div
      data-linefit-section
      data-linefit-status={lineFitStatus(loading, error, run)}
      className="px-3 pb-2 flex flex-col gap-2"
      style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}
    >
      <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
        <span className="flex items-center gap-1.5">
          <Activity size={12} style={{ color: ACCENT }} />
          <span className="text-[11px] font-semibold text-zinc-400 uppercase tracking-wider">Line fit maps</span>
        </span>
        <label className="flex items-center gap-1 text-zinc-500 cursor-pointer">
          <input type="checkbox" aria-label="Weight by ERR" checked={useErr} onChange={(e) => onUseErr(e.target.checked)} disabled={loading} />
          Weight by ERR
        </label>
        {errText && <span className="text-zinc-600">{errText}</span>}
        <label className="flex items-center gap-1 text-zinc-500 cursor-pointer">
          <input type="checkbox" aria-label="Mask DQ channels" checked={useDq} onChange={(e) => onUseDq(e.target.checked)} disabled={loading} />
          Mask DQ channels
        </label>
        {dqText && <span className="text-zinc-600">{dqText}</span>}
        <label className="flex items-center gap-1 text-zinc-500 cursor-pointer">
          <input
            type="checkbox"
            aria-label="Emission only"
            checked={emissionOnly}
            onChange={(e) => onEmissionOnly(e.target.checked)}
            disabled={loading}
          />
          Emission only
        </label>
      </div>

      <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
        <label htmlFor={snrId} className={LABEL_CLASS}>
          S/N
        </label>
        <input
          id={snrId}
          type="number"
          min={0}
          step={0.5}
          aria-label="Line-fit S/N threshold"
          value={snrText}
          onChange={(e) => onSnrText(e.target.value)}
          className={INPUT_CLASS}
          disabled={loading}
        />
        <label htmlFor={resolvingPowerId} className={LABEL_CLASS} title={RESOLVING_POWER_HINT}>
          R
        </label>
        <input
          id={resolvingPowerId}
          type="text"
          inputMode="decimal"
          aria-label="Resolving power R"
          title={RESOLVING_POWER_HINT}
          placeholder="none"
          value={resolvingPowerText}
          onChange={(e) => onResolvingPowerText(e.target.value)}
          className={INPUT_CLASS}
          disabled={loading}
        />
        <label htmlFor={componentsId} className={LABEL_CLASS}>
          Components
        </label>
        <select
          id={componentsId}
          aria-label="Line-fit components"
          value={components}
          onChange={(e) => onComponents(e.target.value as LineFitComponents)}
          className={SELECT_CLASS}
          disabled={loading}
        >
          {LINE_FIT_COMPONENTS.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        <FitBtn loading={loading} disabled={loading || !ready} onClick={onFit} />
        {!ready && !loading && readyHint && <span className="text-zinc-600">{readyHint}</span>}
      </div>

      {error && (
        <p data-linefit-error className={ERROR_CLASS}>
          {error}
        </p>
      )}

      {run && result && (
        <>
          <div className="flex items-center gap-1 flex-wrap text-[10px] font-mono">
            {result.plane_order.map((plane) => {
              const pressed = plane === shownPlane;
              const unit = planeUnit(result, plane);
              return (
                <button
                  key={plane}
                  data-linefit-plane={plane}
                  aria-pressed={pressed}
                  onClick={() => onShowPlane(plane)}
                  disabled={!result.planes[plane]?.previewUrl}
                  title={unit ? `${plane} (${unit})` : plane}
                  className="px-2 py-0.5 rounded border transition-colors disabled:opacity-40"
                  style={{
                    borderColor: pressed ? ACCENT : "var(--ab-border)",
                    color: pressed ? ACCENT : "#71717a",
                  }}
                >
                  {planeLabel(plane)}
                </button>
              );
            })}
          </div>
          <div className="text-[10px] font-mono text-zinc-500 flex flex-col gap-0.5">
            <span
              data-linefit-summary
              data-n-fit={result.n_fit}
              data-n-masked={result.n_masked}
              data-elapsed-ms={result.elapsed_ms}
              data-median-chi2={result.median_chi2_red ?? ""}
              data-weighting={result.weighting}
              data-n-two={result.n_two_components}
            >
              {lineFitSummary(result)}
            </span>
            <span>
              flux in {result.units.flux}, ch {result.z0}-{result.z1}, continuum {result.continuum_windows[0][0]}-
              {result.continuum_windows[0][1]} and {result.continuum_windows[1][0]}-{result.continuum_windows[1][1]}, weights{" "}
              {result.weighting}
            </span>
            <span data-linefit-sigma>{sigmaCaption(result)}</span>
            <span data-linefit-frame>{frameNote}</span>
          </div>
          {result.notes.length > 0 && (
            <details data-linefit-notes className="text-[10px] font-mono text-zinc-600">
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
          <div
            data-linefit-inspect
            data-x={inspect?.x}
            data-y={inspect?.y}
            data-ncomp={inspect?.ncomp}
            data-converged={inspect ? String(inspect.converged) : undefined}
            className="flex flex-col gap-1.5"
          >
            {inspect && (
              <>
                <div className="text-[10px] font-mono text-zinc-500">
                  spaxel ({inspect.x}, {inspect.y}), ch {inspect.z0}-{inspect.z1}
                </div>
                <LineFitInspectPlot spaxel={inspect} />
                <div className="grid grid-cols-2 gap-1.5">
                  {rows.map((row) => (
                    <div key={row.label} className="bg-zinc-900/80 rounded px-2 py-1.5">
                      <div className="text-[9px] text-zinc-500 uppercase tracking-wider">{row.label}</div>
                      <div className="text-[10px] font-mono text-zinc-200 break-words">{row.value}</div>
                    </div>
                  ))}
                </div>
                {spaxelNotes.length > 0 && (
                  <details data-linefit-inspect-notes open className="text-[10px] font-mono text-zinc-500">
                    <summary className="cursor-pointer select-none">spaxel notes ({spaxelNotes.length})</summary>
                    <ul className="mt-0.5 flex flex-col gap-0.5">
                      {spaxelNotes.map((note, i) => (
                        <li key={i} className="break-words">
                          {note}
                        </li>
                      ))}
                    </ul>
                  </details>
                )}
              </>
            )}
            {inspectLoading && <p className="text-[10px] font-mono text-zinc-500">fitting the spaxel…</p>}
            {inspectError && (
              <p data-linefit-inspect-error className={ERROR_CLASS}>
                {inspectError}
              </p>
            )}
            {!inspect && !inspectLoading && !inspectError && (
              <p data-linefit-inspect-hint className="text-[10px] font-mono text-zinc-600">
                {INSPECT_HINT}
              </p>
            )}
          </div>
        </>
      )}
    </div>
  );
}

export default memo(LineFitSection);
