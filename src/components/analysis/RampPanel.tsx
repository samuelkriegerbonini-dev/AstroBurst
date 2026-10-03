import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Activity, FileText, FolderOpen, X } from "lucide-react";
import CubeFrameNav from "../CubeFrameNav";
import ProfilePlot, { type ProfileSeries } from "../regions/ProfilePlot";
import RampComparePanel from "./RampComparePanel";
import { ErrorAlert, RunButton, Slider, WarningList } from "../ui";
import { getPreviewUrl, isTauri } from "../../infrastructure/tauri";
import { useDisplayedImage } from "../../context/PreviewContext";
import { useToolHost } from "../../context/ToolHostContext";
import { getDqFlagTable } from "../../services/display";
import {
  QSLOPE_PROGRESS_EVENT,
  cancelQuickSlope,
  getRampFrame,
  getRampPixelFit,
  getRampPixelSeries,
  getRampTables,
  runQuickSlope,
} from "../../services/ramp";
import { useProgress } from "../../hooks/useProgress";
import { setIntegration, useRampIntegration } from "../../hooks/useRampStore";
import { useSpectrum } from "../../hooks/useSpectrumStore";
import { decodeDqBits } from "../../utils/dqFlags";
import { createFramePublishGate, type FramePublishGate } from "../../utils/cubeNavigation";
import {
  QSLOPE_ACCURACY_NOTE,
  draftFromParams,
  fittedLinePoints,
  formatGroupTime,
  formatRampFrameLabel,
  formatTableCell,
  framePublishers,
  groupTimeSummary,
  integrationOptions,
  irs2Badge,
  paramsFromDraft,
  quickSlopeUnavailableReason,
  rampCompareKey,
  rampDisplayNote,
  rampIdentity,
  rampInspectLabel,
  rampInspectorSeries,
  rampKeyHint,
  rampShape,
  rampTimeAxis,
  validateQuickSlopeParams,
  type QuickSlopeDraft,
} from "../../utils/rampLabels";
import {
  DEFAULT_QUICK_SLOPE_PARAMS,
  REF_CORRECTIONS,
  type QuickSlopeParams,
  type QuickSlopeResult,
  type RampInfo,
  type RampPixelFit,
  type RampPixelSeries,
  type RampSource,
  type RampTables,
  type RefCorrection,
  type TablePreview,
} from "../../shared/types/ramp";
import type { CubeDims } from "../../shared/types/cube";
import type { DqFlag } from "../../shared/types/dq";
import type { CubeResult } from "./SpectroscopyPanel";

interface RampPanelProps {
  filePath: string | undefined;
  ramp: RampInfo;
  rampSource: RampSource | null;
  cubeDims: CubeDims | null;
  onCubeResult?: (result: CubeResult) => void;
  publishGate?: FramePublishGate;
}

interface InspectState {
  fit: RampPixelFit | null;
  series: RampPixelSeries | null;
  fitError: string | null;
}

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 font-mono focus:border-violet-500/50 w-16 disabled:opacity-40";
const SELECT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 focus:border-violet-500/50 disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const BADGE_CLASS = "px-1.5 py-0.5 rounded text-[9px] font-mono border";
const SMALL_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-0.5 rounded text-[9px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40";
const QSLOPE_LABEL = "Quick slope (DN/s)";
const PLOT_HEIGHT = 170;
const RAW_COLOR = "rgba(161,161,170,0.8)";
const CORRECTED_COLOR = "rgba(45,212,191,0.95)";
const FIT_COLOR = "rgba(250,250,250,0.9)";
const FLAGGED_COLOR = "rgba(248,113,113,1)";
const EXCLUDED_COLOR = "rgba(82,82,91,0.9)";
const JUMP_K_MIN = 3;
const JUMP_K_MAX = 10;
const JUMP_K_STEP = 0.5;
const INSPECTOR_PARAMS_NOTE = "fit with the parameters of the last quick slope run (defaults before the first run)";

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function fixedOrNa(v: number | null | undefined, digits: number): string {
  return v === null || v === undefined || !Number.isFinite(v) ? "n/a" : v.toFixed(digits);
}

function inspectorPlot(state: InspectState): { series: ProfileSeries[]; xLabel: string } {
  if (state.fit) {
    const s = rampInspectorSeries(state.fit);
    const line = fittedLinePoints(state.fit);
    const series: ProfileSeries[] = [
      { x: s.t, y: s.raw, color: RAW_COLOR, label: "raw", mode: "both" },
      { x: s.t, y: s.corrected, color: CORRECTED_COLOR, label: "corrected", mode: "both" },
      { x: s.t, y: s.excluded, color: EXCLUDED_COLOR, label: "saturated / not used", mode: "points" },
      { x: s.t, y: s.flagged, color: FLAGGED_COLOR, label: "jump-flagged difference", mode: "points" },
    ];
    if (line.length === 2) {
      series.push({ x: line.map((p) => p.t), y: line.map((p) => p.value), color: FIT_COLOR, label: "fit", mode: "line", dashed: true });
    }
    return { series, xLabel: rampTimeAxis(state.fit.group_times_s, state.fit.raw.length).label };
  }
  if (state.series) {
    const axis = rampTimeAxis(state.series.group_times_s, state.series.values.length);
    return { series: [{ x: axis.t, y: state.series.values, color: RAW_COLOR, label: "raw", mode: "both" }], xLabel: axis.label };
  }
  return { series: [], xLabel: "" };
}

function TablePreviewView({ title, table }: { title: string; table: TablePreview | null }) {
  if (!table) return <div className="text-[9px] text-zinc-600">{title}: not in this file</div>;
  return (
    <details className="text-[9px]">
      <summary className="cursor-pointer text-zinc-400">
        {title} (HDU {table.hdu}) showing {table.shown_rows} of {table.n_rows} rows
      </summary>
      <div className="mt-1 overflow-x-auto max-h-48 overflow-y-auto">
        <table className="font-mono text-zinc-300">
          <thead>
            <tr className="text-zinc-500">
              {table.columns.map((c) => (
                <th key={c.name} className="text-right pr-2 font-normal whitespace-nowrap">
                  {c.name}
                  {c.unit ? ` [${c.unit}]` : ""}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {Array.from({ length: table.shown_rows }, (_, row) => (
              <tr key={row}>
                {table.columns.map((c) => (
                  <td key={c.name} className="text-right pr-2 whitespace-nowrap">
                    {formatTableCell(c.values[row] ?? null)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {table.omitted_columns.length > 0 && (
        <div className="mt-1 text-zinc-500">not shown (string or array columns): {table.omitted_columns.join(", ")}</div>
      )}
    </details>
  );
}

function RampPanel({ filePath, ramp, rampSource, cubeDims, onCubeResult, publishGate }: RampPanelProps) {
  const integrationId = useId();
  const refCorrectionId = useId();
  const slopeBlockerId = useId();
  const paramErrorsId = useId();
  const source: RampSource = rampSource ?? "fits";
  const integration = useRampIntegration();
  const { coord } = useSpectrum();
  const { active } = useToolHost();
  const displayed = useDisplayedImage();
  const progress = useProgress(QSLOPE_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const coordX = coord?.x ?? null;
  const coordY = coord?.y ?? null;

  const [tables, setTables] = useState<RampTables | null>(null);
  const [tablesError, setTablesError] = useState<string | null>(null);
  const [dqFlags, setDqFlags] = useState<DqFlag[] | null>(null);
  const [inspect, setInspect] = useState<InspectState | null>(null);
  const [inspectLoading, setInspectLoading] = useState(false);
  const [inspectError, setInspectError] = useState<string | null>(null);
  const [draft, setDraft] = useState<QuickSlopeDraft>(() => draftFromParams(DEFAULT_QUICK_SLOPE_PARAMS));
  const [running, setRunning] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);
  const [result, setResult] = useState<QuickSlopeResult | null>(null);
  const [runId, setRunId] = useState(0);
  const [pickedRate, setPickedRate] = useState<string | null>(null);
  const [asdfFrame, setAsdfFrame] = useState(0);
  const [asdfRequestSeq, setAsdfRequestSeq] = useState(0);
  const [ownGate] = useState(createFramePublishGate);
  const gate = publishGate ?? ownGate;
  const inspectSeqRef = useRef(0);
  const lastInspectKeyRef = useRef<string | null>(null);
  const runSeqRef = useRef(0);
  const runningRef = useRef(false);
  const asdfPublishSeqRef = useRef(0);

  const params = useMemo(() => paramsFromDraft(draft), [draft]);
  const paramErrors = useMemo(() => validateQuickSlopeParams(params), [params]);
  const slopeBlocker = quickSlopeUnavailableReason(ramp, source, source === "fits" && cubeDims === null);
  const runDescribedBy =
    [slopeBlocker ? slopeBlockerId : null, paramErrors.length > 0 ? paramErrorsId : null].filter((id): id is string => id !== null).join(" ") ||
    undefined;
  const displayNote = rampDisplayNote(ramp, displayed.dimensions);
  const publishers = useMemo(
    () => framePublishers<CubeResult>(gate, (record) => onCubeResult?.(record)),
    [gate, onCubeResult],
  );
  const inspectParams: QuickSlopeParams = result?.params ?? DEFAULT_QUICK_SLOPE_PARAMS;
  const totalFrames = ramp.ngroups * Math.max(ramp.nints, 1);
  const badge = irs2Badge(ramp);
  const ratePath = pickedRate ?? result?.rate_sibling ?? null;

  useEffect(() => {
    if (!filePath || source !== "fits") return;
    let cancelled = false;
    getRampTables(filePath)
      .then((t) => {
        if (!cancelled) setTables(t);
      })
      .catch((e: unknown) => {
        if (!cancelled) setTablesError(errorText(e));
      });
    getDqFlagTable(filePath)
      .then((t) => {
        if (!cancelled) setDqFlags(t.flags);
      })
      .catch(() => {
        if (!cancelled) setDqFlags(null);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath, source]);

  useEffect(() => {
    if (!active || !filePath || coordX === null || coordY === null) return;
    const key = [filePath, coordX, coordY, integration, slopeBlocker ?? "", JSON.stringify(inspectParams)].join("|");
    if (key === lastInspectKeyRef.current) return;
    lastInspectKeyRef.current = key;
    const seq = ++inspectSeqRef.current;
    const x = coordX;
    const y = coordY;
    const seriesOnly = (fitError: string | null) =>
      getRampPixelSeries(filePath, x, y, integration).then((series): InspectState => ({ fit: null, series, fitError }));
    const load =
      slopeBlocker === null
        ? getRampPixelFit(filePath, x, y, integration, inspectParams)
            .then((fit): InspectState => ({ fit, series: null, fitError: null }))
            .catch((e: unknown) => seriesOnly(errorText(e)))
        : seriesOnly(slopeBlocker);
    setInspectLoading(true);
    setInspectError(null);
    load
      .then((state) => {
        if (seq === inspectSeqRef.current) setInspect(state);
      })
      .catch((e: unknown) => {
        if (seq === inspectSeqRef.current) setInspectError(errorText(e));
      })
      .finally(() => {
        if (seq === inspectSeqRef.current) setInspectLoading(false);
      });
  }, [active, filePath, coordX, coordY, integration, inspectParams, slopeBlocker]);

  useEffect(() => {
    const inspectSeq = inspectSeqRef;
    const runSeq = runSeqRef;
    const publishSeq = asdfPublishSeqRef;
    return () => {
      inspectSeq.current++;
      publishSeq.current++;
      runSeq.current++;
      if (runningRef.current) cancelQuickSlope().catch(() => {});
    };
  }, []);

  const plot = useMemo(() => (inspect ? inspectorPlot(inspect) : null), [inspect]);
  const dqNames = inspect?.fit && dqFlags ? decodeDqBits(inspect.fit.fit.dq, dqFlags) : null;

  const patchDraft = useCallback((patch: Partial<QuickSlopeDraft>) => setDraft((d) => ({ ...d, ...patch })), []);

  const handleRun = useCallback(async () => {
    if (!filePath || slopeBlocker !== null || paramErrors.length > 0) return;
    const seq = ++runSeqRef.current;
    runningRef.current = true;
    setRunning(true);
    setRunError(null);
    resetProgress();
    try {
      const res = await runQuickSlope(filePath, undefined, integration, params);
      if (seq !== runSeqRef.current) return;
      setResult(res);
      setRunId((n) => n + 1);
      setPickedRate(null);
      if (res.previewUrl) {
        publishers.result({ label: QSLOPE_LABEL, previewUrl: res.previewUrl, fitsPath: res.fits_path, dimensions: res.dimensions });
      }
    } catch (e) {
      if (seq === runSeqRef.current) setRunError(errorText(e));
    } finally {
      if (seq === runSeqRef.current) {
        runningRef.current = false;
        setRunning(false);
        resetProgress();
      }
    }
  }, [filePath, slopeBlocker, paramErrors.length, integration, params, publishers, resetProgress]);

  const handleCancel = useCallback(() => {
    cancelQuickSlope().catch(() => {});
  }, []);

  const pickRate = useCallback(async () => {
    if (!isTauri()) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: false, title: "Official rate file", filters: [{ name: "FITS", extensions: ["fits"] }] });
      if (typeof picked === "string") setPickedRate(picked);
    } catch (e) {
      console.error("[AstroBurst] Rate file dialog error:", e);
    }
  }, []);

  const requestAsdfFrame = useCallback(
    (idx: number) => {
      setAsdfFrame(Math.max(0, Math.min(totalFrames - 1, idx)));
      setAsdfRequestSeq((s) => s + 1);
    },
    [totalFrames],
  );

  const publishAsdfFrame = useCallback(
    async (outputPath: string, frameIndex: number, fitsPath?: string) => {
      const seq = ++asdfPublishSeqRef.current;
      try {
        const url = await getPreviewUrl(outputPath);
        if (seq !== asdfPublishSeqRef.current) return;
        publishers.frame({
          label: formatRampFrameLabel(frameIndex, ramp),
          previewUrl: url,
          fitsPath: fitsPath ?? null,
          dimensions: fitsPath ? [ramp.frame_width, ramp.frame_height] : null,
          frameIndex,
        });
      } catch (e) {
        console.error("Ramp frame preview failed:", e);
      }
    },
    [publishers, ramp],
  );

  const labelFor = useCallback((idx: number) => formatRampFrameLabel(idx, ramp), [ramp]);
  const frameSize = cubeDims ? `${cubeDims.width} x ${cubeDims.height}` : `${ramp.frame_width} x ${ramp.frame_height}`;
  const groupTime = formatGroupTime(asdfFrame, ramp);

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Activity size={12} className="text-teal-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Ramp</span>
        </div>
        <span className={`${BADGE_CLASS} text-zinc-400 border-zinc-700/60`}>{source === "asdf" ? "ASDF" : "FITS"}</span>
      </div>

      <div className="px-3 py-2 space-y-3">
        <div className="space-y-1">
          <div className="text-[10px] font-mono text-zinc-200">{rampIdentity(ramp) || "unknown instrument"}</div>
          <div className="flex flex-wrap items-center gap-1.5">
            <span className={`${BADGE_CLASS} text-zinc-300 border-zinc-700/60`} title="groups x integrations">
              {rampShape(ramp)}
            </span>
            <span className={`${BADGE_CLASS} text-zinc-300 border-zinc-700/60`}>{frameSize}</span>
            {badge && <span className={`${BADGE_CLASS} text-violet-300 border-violet-700/40`}>{badge}</span>}
          </div>
          <div className="text-[9px] font-mono text-zinc-500">{groupTimeSummary(ramp)}</div>
          <div className="text-[9px] text-zinc-600">{rampKeyHint(ramp)}</div>
        </div>

        {source === "asdf" && filePath && totalFrames > 1 && (
          <div className="space-y-1">
            <CubeFrameNav
              filePath={filePath}
              totalFrames={totalFrames}
              frame={asdfFrame}
              requestSeq={asdfRequestSeq}
              onFrameRequest={requestAsdfFrame}
              frameLabel={labelFor(asdfFrame)}
              labelFor={labelFor}
              frameLoader={getRampFrame}
              onFrameChange={publishAsdfFrame}
              publishGate={gate}
            />
            {groupTime && <div className="text-[9px] font-mono text-zinc-500">resultant time {groupTime}</div>}
          </div>
        )}

        {ramp.nints > 1 && (
          <label htmlFor={integrationId} className="flex items-center gap-2">
            <span className={LABEL_CLASS}>Integration</span>
            <select
              id={integrationId}
              value={Math.min(integration, ramp.nints - 1)}
              onChange={(e) => setIntegration(Number(e.target.value))}
              className={SELECT_CLASS}
            >
              {integrationOptions(ramp).map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
        )}

        {source === "fits" && (
          <div className="space-y-1">
            <div className="flex items-center gap-1.5">
              <FileText size={10} className="text-zinc-500" />
              <span className={LABEL_CLASS}>Ramp tables</span>
            </div>
            {tablesError && <div className="text-[9px] text-amber-300">{tablesError}</div>}
            {tables && (
              <>
                <TablePreviewView title="GROUP" table={tables.group} />
                <TablePreviewView title="INT_TIMES" table={tables.int_times} />
              </>
            )}
          </div>
        )}

        <div className="space-y-1">
          <div className={LABEL_CLASS}>Ramp inspector</div>
          {!coord && <div className="text-[9px] text-zinc-500">click a pixel in the viewer to plot its ramp</div>}
          {displayNote && <div className="text-[9px] text-amber-300/80">{displayNote}</div>}
          {coord && (
            <div className="text-[9px] font-mono text-zinc-500">
              {rampInspectLabel(coord, ramp, integration)}
              {inspectLoading ? ", loading..." : ""}
            </div>
          )}
          {inspectError && <div className="text-[9px] text-red-400">{inspectError}</div>}
          {plot && plot.series.length > 0 && <ProfilePlot series={plot.series} xLabel={plot.xLabel} yLabel="DN" height={PLOT_HEIGHT} logToggle={false} />}
          {inspect?.fit && (
            <div className="text-[9px] font-mono text-zinc-400 space-y-0.5">
              <div>
                slope {fixedOrNa(inspect.fit.fit.slope, 4)} DN/s, noise {fixedOrNa(inspect.fit.fit.noise, 4)} DN/s, ngood {inspect.fit.fit.ngood}
                {" "}of {inspect.fit.raw.length}, usable {inspect.fit.fit.n_usable}
                {inspect.fit.fit.first_saturated !== null ? `, saturated from group ${inspect.fit.fit.first_saturated + 1}` : ""}
              </div>
              <div>
                band scale {inspect.fit.band_scale_dn.toFixed(1)} DN, DQ {inspect.fit.fit.dq}
                {dqNames && dqNames.length > 0 ? ` (${dqNames.join(" | ")})` : ""}
                {inspect.fit.amplifier !== null ? `, amplifier ${inspect.fit.amplifier}` : ""}
                {inspect.fit.science_row !== null ? `, science row ${inspect.fit.science_row}` : ""}
              </div>
              <div className="text-zinc-600">{INSPECTOR_PARAMS_NOTE}</div>
            </div>
          )}
          {inspect?.series && !inspect.fit && (
            <div className="text-[9px] text-zinc-500">
              raw group values only ({inspect.series.unit}): no per-pixel fit
              {inspect.fitError ? <span className="text-amber-300/80">: {inspect.fitError}</span> : null}
            </div>
          )}
        </div>

        <div className="space-y-2">
          <div className={LABEL_CLASS}>Quick slope</div>
          {slopeBlocker && (
            <div id={slopeBlockerId} className="text-[9px] text-amber-300">
              {slopeBlocker}
            </div>
          )}
          <details className="text-[10px]">
            <summary className="cursor-pointer text-zinc-400">Parameters</summary>
            <div className="mt-1.5 space-y-1.5">
              <label className="flex items-center gap-1">
                <span className={LABEL_CLASS}>saturation</span>
                <input
                  type="text"
                  inputMode="decimal"
                  value={draft.sat_dn}
                  onChange={(e) => patchDraft({ sat_dn: e.target.value })}
                  className={INPUT_CLASS}
                />
                <span className={LABEL_CLASS}>DN</span>
              </label>
              <Slider
                label="Jump k"
                value={draft.jump_k}
                min={JUMP_K_MIN}
                max={JUMP_K_MAX}
                step={JUMP_K_STEP}
                onChange={(v) => patchDraft({ jump_k: v })}
                hint="|d - med| > k x scale"
              />
              <label htmlFor={refCorrectionId} className="flex items-center gap-1">
                <span className={LABEL_CLASS}>reference correction</span>
                <select
                  id={refCorrectionId}
                  value={draft.ref_correction}
                  onChange={(e) => patchDraft({ ref_correction: e.target.value as RefCorrection })}
                  className={SELECT_CLASS}
                >
                  {REF_CORRECTIONS.map((r) => (
                    <option key={r} value={r}>
                      {r}
                    </option>
                  ))}
                </select>
              </label>
              <details>
                <summary className="cursor-pointer text-zinc-500">Advanced</summary>
                <div className="mt-1 space-y-1">
                  <label className="flex items-center gap-1">
                    <span className={LABEL_CLASS}>scale floor</span>
                    <input
                      type="text"
                      inputMode="decimal"
                      value={draft.scale_floor_dn}
                      onChange={(e) => patchDraft({ scale_floor_dn: e.target.value })}
                      className={INPUT_CLASS}
                    />
                    <span className={LABEL_CLASS}>DN</span>
                  </label>
                  <label className="flex items-center gap-1">
                    <span className={LABEL_CLASS}>min groups for OLS</span>
                    <input
                      type="text"
                      inputMode="numeric"
                      value={draft.min_groups_ols}
                      onChange={(e) => patchDraft({ min_groups_ols: e.target.value })}
                      className={INPUT_CLASS}
                    />
                  </label>
                  <div className="flex items-center gap-2">
                    <label className="flex items-center gap-1">
                      <span className={LABEL_CLASS}>reference window</span>
                      <input
                        type="text"
                        inputMode="numeric"
                        value={draft.ref_window_rows}
                        disabled={draft.whole_band}
                        onChange={(e) => patchDraft({ ref_window_rows: e.target.value })}
                        className={INPUT_CLASS}
                      />
                      <span className={LABEL_CLASS}>rows</span>
                    </label>
                    <label className="flex items-center gap-1">
                      <input type="checkbox" checked={draft.whole_band} onChange={(e) => patchDraft({ whole_band: e.target.checked })} />
                      <span className={LABEL_CLASS}>whole band</span>
                    </label>
                  </div>
                </div>
              </details>
            </div>
          </details>
          {paramErrors.length > 0 && (
            <ul id={paramErrorsId} className="text-[9px] text-amber-300 list-disc pl-4">
              {paramErrors.map((m) => (
                <li key={m}>{m}</li>
              ))}
            </ul>
          )}
          <RunButton
            label="Run quick slope"
            runningLabel="Fitting ramps..."
            running={running}
            disabled={!filePath || slopeBlocker !== null || paramErrors.length > 0}
            small
            describedBy={runDescribedBy}
            onClick={handleRun}
          />
          {running && progress.active && (
            <div className="flex items-center gap-2">
              <div
                role="progressbar"
                aria-label="Quick slope progress"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(progress.percent)}
                aria-valuetext={progress.stage ? `${Math.round(progress.percent)} %, ${progress.stage}` : undefined}
                className="flex-1 h-1 rounded bg-zinc-800 overflow-hidden"
              >
                <div className="h-full bg-teal-500/70" style={{ width: `${Math.round(progress.percent)}%` }} />
              </div>
              <span className="text-[9px] font-mono text-zinc-500">{progress.stage}</span>
              <button type="button" onClick={handleCancel} className={SMALL_BUTTON_CLASS} aria-label="Cancel quick slope">
                <X size={9} />
                Cancel
              </button>
            </div>
          )}
          <p className="text-[9px] leading-snug text-zinc-500">{QSLOPE_ACCURACY_NOTE}</p>
          <ErrorAlert message={runError} />
          {result && (
            <div className="space-y-1.5">
              <WarningList warnings={result.warnings} />
              <div className="text-[9px] font-mono text-zinc-400 space-y-0.5">
                <div className="break-all" title={result.fits_path}>
                  {result.fits_path}
                </div>
                <div>
                  {result.dimensions[0]} x {result.dimensions[1]}, integration {result.integration + 1} of {result.nints},{" "}
                  {result.ref_corrected ? "reference-corrected" : "no reference correction"}
                  {result.stripped ? ", IRS2 rows stripped" : ""}, {result.elapsed_ms} ms
                </div>
                <div>
                  saturated {result.counts.saturated}, jump {result.counts.jump_det}, do not use {result.counts.do_not_use}
                </div>
                <div>band scales {result.band_scales_dn.map((s) => s.toFixed(1)).join(", ")} DN</div>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <button type="button" onClick={pickRate} className={SMALL_BUTTON_CLASS} disabled={!isTauri()}>
                  <FolderOpen size={9} />
                  {ratePath ? "Use another rate file" : "Pick the official rate file"}
                </button>
                {!ratePath && <span className="text-[9px] text-zinc-600">no sibling _rate.fits next to the uncal</span>}
              </div>
              {ratePath && (
                <RampComparePanel
                  key={rampCompareKey(runId, result.fits_path, ratePath)}
                  qslopePath={result.fits_path}
                  ratePath={ratePath}
                  dimensions={result.dimensions}
                  onCubeResult={onCubeResult ? publishers.result : undefined}
                />
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default memo(RampPanel);
