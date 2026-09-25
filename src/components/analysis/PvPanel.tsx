import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Activity, Loader2 } from "lucide-react";
import SpectralAxisControls from "./SpectralAxisControls";
import ProfilePlot from "../regions/ProfilePlot";
import { computePvDiagram } from "../../services/pv";
import { measurementLog, pvEntry } from "../../utils/measurementLog";
import { getOutputDir } from "../../infrastructure/tauri";
import { useRenderActions, useRenderContext } from "../../context/PreviewContext";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useRegionKey } from "../../hooks/useRegionKey";
import { shapeSummary } from "../../utils/regionGeometry";
import type { CubeDims } from "../../shared/types/cube";
import type { PvDiagramResult, PvRun, PvRunParams } from "../../shared/types/pv";
import type {
  CorrectionFrame,
  RadialVelocityCorrectionResult,
  SpectralAxisMode,
  VelocityConvention,
} from "../../shared/types/spectral";
import {
  DEFAULT_STEP_TEXT,
  DEFAULT_WIDTH_TEXT,
  isPvRecord,
  lineOf,
  offsetLabel,
  parseChannel,
  parseStep,
  parseWidth,
  pickLineRegion,
  pvCsvFileName,
  pvEffectiveMode,
  pvOnScreen,
  pvRecordLabel,
  pvRidgeCsv,
  pvRunBlocker,
  pvShiftKms,
  pvSpectralLabel,
  ridgeSeries,
} from "../../utils/pvDiagram";

interface PvPanelProps {
  filePath: string | undefined;
  fileKey: string | null;
  cubeDims: CubeDims | null;
}

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 font-mono focus:border-violet-500/50 w-14 disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const SMALL_BUTTON_CLASS =
  "px-2 py-0.5 rounded text-[9px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40";
const RUN_COLOR = "var(--ab-violet)";
const NO_LINE_HINT = "draw a line region on the cube (the selected line is used, else the first one)";
const WIDTH_TITLE = "width averaged across the slit in pixels, one sample per step; width <= step = single sample";
const COPIED_FEEDBACK_MS = 1500;
const PLOT_HEIGHT = 200;
const CSV_NAME = "pv-ridge";
const SIGNIFICANT_DIGITS = 4;
const EM_DASH = "—";

function Card({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="bg-zinc-900/80 rounded px-2 py-1.5" title={title}>
      <div className="text-[9px] text-zinc-500 uppercase">{label}</div>
      <div className="text-[10px] font-mono text-zinc-200 truncate">{value}</div>
    </div>
  );
}

function sig(value: number | null, unit: string): string {
  if (value === null || !Number.isFinite(value)) return EM_DASH;
  return `${value.toPrecision(SIGNIFICANT_DIGITS)} ${unit}`.trim();
}

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function PvPanel({ filePath, fileKey, cubeDims }: PvPanelProps) {
  const regionDoc = useRegionDoc(useRegionKey());
  const { publishProcessed, resetProcessed } = useRenderActions();
  const { processed } = useRenderContext();
  const region = useMemo(() => pickLineRegion(regionDoc.regions, regionDoc.selectedId), [regionDoc]);
  const line = useMemo(() => (region ? lineOf(region) : null), [region]);
  const axis = cubeDims?.spectral_axis ?? null;
  const frames = cubeDims?.frames ?? 0;

  const [mode, setMode] = useState<SpectralAxisMode>("wavelength_vac");
  const [restUm, setRestUm] = useState<number | null>(null);
  const [convention, setConvention] = useState<VelocityConvention>("optical");
  const [correction, setCorrection] = useState<CorrectionFrame>("none");
  const [correctionResult, setCorrectionResult] = useState<RadialVelocityCorrectionResult | null>(null);
  const [stepText, setStepText] = useState(DEFAULT_STEP_TEXT);
  const [widthText, setWidthText] = useState(DEFAULT_WIDTH_TEXT);
  const [z0Text, setZ0Text] = useState("0");
  const [z1Text, setZ1Text] = useState("0");
  const [run, setRun] = useState<PvRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [ioError, setIoError] = useState<string | null>(null);
  const seqRef = useRef(0);

  useEffect(() => {
    seqRef.current++;
    setRun(null);
    setError(null);
    setIoError(null);
    setLoading(false);
    setStepText(DEFAULT_STEP_TEXT);
    setWidthText(DEFAULT_WIDTH_TEXT);
    setZ0Text("0");
    setZ1Text(String(Math.max(frames - 1, 0)));
  }, [filePath, frames]);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), COPIED_FEEDBACK_MS);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const step = parseStep(stepText);
  const width = parseWidth(widthText);
  const z0 = parseChannel(z0Text, frames);
  const z1 = parseChannel(z1Text, frames);
  const runMode = pvEffectiveMode(axis, mode);
  const blocker = pvRunBlocker({ line, step, width, z0, z1, mode: runMode, restUm, axis });
  const disabled = line === null || !filePath;

  const publishPv = useCallback(
    (params: PvRunParams, res: PvDiagramResult) => {
      if (!fileKey || !res.previewUrl) return;
      publishProcessed(fileKey, {
        previewUrl: res.previewUrl,
        fitsPath: res.fits_path,
        dimensions: res.dimensions,
        label: pvRecordLabel(params, res),
        kind: "cube",
        inputPath: params.filePath,
      });
    },
    [fileKey, publishProcessed],
  );

  const handleRun = useCallback(async () => {
    if (!filePath || !region || !line || step === null || width === null || z0 === null || z1 === null) return;
    const seq = ++seqRef.current;
    setLoading(true);
    setError(null);
    const velocityShiftKms = pvShiftKms(correction, correctionResult, runMode);
    const params: PvRunParams = {
      filePath,
      regionId: region.id,
      line,
      stepPx: step,
      widthPx: width,
      z0,
      z1,
      mode: runMode,
      restUm,
      convention,
      correction,
      velocityShiftKms,
    };
    try {
      const dir = await getOutputDir();
      if (seqRef.current !== seq) return;
      const res = await computePvDiagram(filePath, dir, {
        line,
        stepPx: step,
        widthPx: width,
        z0,
        z1,
        mode: runMode,
        restUm,
        convention,
        velocityShiftKms,
      });
      if (seqRef.current !== seq) return;
      const run: PvRun = { params, result: res };
      setRun(run);
      measurementLog.append(pvEntry(run));
      publishPv(params, res);
    } catch (e) {
      if (seqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (seqRef.current === seq) setLoading(false);
    }
  }, [filePath, region, line, step, width, z0, z1, runMode, restUm, convention, correction, correctionResult, publishPv]);

  const handleCopyCsv = useCallback(async () => {
    if (!run) return;
    setIoError(null);
    try {
      await navigator.clipboard.writeText(pvRidgeCsv(run));
      setCopied(true);
    } catch (e) {
      setIoError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [run]);

  const handleSaveCsv = useCallback(async () => {
    if (!run || busy) return;
    setBusy(true);
    setIoError(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: pvCsvFileName(filePath ?? null),
        filters: [{ name: "CSV", extensions: ["csv"] }],
        title: "Save the PV ridge as CSV",
      });
      if (!target) return;
      const { writeTextFile } = await import("@tauri-apps/plugin-fs");
      await writeTextFile(target, pvRidgeCsv(run));
    } catch (e) {
      setIoError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [run, busy, filePath]);

  const pvLabel = processed !== null && isPvRecord(processed) ? processed.label : null;
  const onScreen = pvOnScreen(processed, run);
  const res = run?.result ?? null;
  const summary = res?.summary ?? null;
  const offsetUnit = res ? (res.offset_unit === "arcsec" ? "arcsec" : "px") : "";

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Activity size={12} className="text-violet-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">PV diagram</span>
        </div>
        {loading && <Loader2 size={12} className="animate-spin text-violet-400/70" />}
      </div>
      <div className="px-3 py-2 space-y-2">
        {region ? (
          <div className="text-[10px] font-mono text-zinc-500 truncate">{shapeSummary(region.shape)}</div>
        ) : (
          <div className="text-[10px] text-zinc-500">{NO_LINE_HINT}</div>
        )}
        <SpectralAxisControls
          filePath={filePath ?? null}
          axis={axis}
          mode={mode}
          onModeChange={setMode}
          restValue={restUm}
          onRestChange={setRestUm}
          convention={convention}
          onConventionChange={setConvention}
          correction={correction}
          onCorrectionChange={setCorrection}
          onCorrectionLoaded={setCorrectionResult}
        />
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <label className="flex items-center gap-1">
            <span className={LABEL_CLASS}>step</span>
            <input
              type="text"
              inputMode="decimal"
              value={stepText}
              disabled={disabled}
              onChange={(e) => setStepText(e.target.value)}
              className={INPUT_CLASS}
              title="Sample spacing along the slit in pixels"
            />
            <span className={LABEL_CLASS}>px</span>
          </label>
          <label className="flex items-center gap-1">
            <span className={LABEL_CLASS}>width</span>
            <input
              type="text"
              inputMode="decimal"
              value={widthText}
              disabled={disabled}
              onChange={(e) => setWidthText(e.target.value)}
              className={INPUT_CLASS}
              title={WIDTH_TITLE}
            />
            <span className={LABEL_CLASS}>px</span>
          </label>
          <label className="flex items-center gap-1">
            <span className={LABEL_CLASS}>from ch</span>
            <input
              type="text"
              inputMode="decimal"
              value={z0Text}
              disabled={disabled}
              onChange={(e) => setZ0Text(e.target.value)}
              className={INPUT_CLASS}
              title="First channel of the range, 0-based"
            />
          </label>
          <label className="flex items-center gap-1">
            <span className={LABEL_CLASS}>to ch</span>
            <input
              type="text"
              inputMode="decimal"
              value={z1Text}
              disabled={disabled}
              onChange={(e) => setZ1Text(e.target.value)}
              className={INPUT_CLASS}
              title="Last channel of the range, 0-based, inclusive"
            />
          </label>
          <button
            type="button"
            onClick={handleRun}
            disabled={disabled || blocker !== null || loading}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all disabled:opacity-40"
            style={{
              background: `color-mix(in srgb, ${RUN_COLOR} 8%, transparent)`,
              border: `1px solid color-mix(in srgb, ${RUN_COLOR} 20%, transparent)`,
              color: RUN_COLOR,
            }}
          >
            {loading ? (
              <div className="w-2.5 h-2.5 rounded-full animate-spin" style={{ border: "1.5px solid transparent", borderTopColor: RUN_COLOR }} />
            ) : (
              <Activity size={10} />
            )}
            Run PV
          </button>
        </div>
        {region && blocker && <div className="text-[9px] text-amber-300">{blocker}</div>}
        {error && (
          <div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
            {error}
          </div>
        )}
        {pvLabel !== null && (
          <div className="text-[9px] text-amber-300 bg-amber-900/20 border border-amber-800/30 rounded px-2 py-1 break-words">
            PV image on screen ({pvLabel}): row 0 (top) is the first channel of the range; pixel clicks and regions on
            the viewer refer to the cube&apos;s pixel grid, not to the PV axes. Reset the view (Show cube) before drawing
            regions.
          </div>
        )}
        {(pvLabel !== null || run) && (
          <div className="flex flex-wrap items-center gap-1">
            <button type="button" onClick={resetProcessed} disabled={pvLabel === null} className={SMALL_BUTTON_CLASS}>
              Show cube
            </button>
            {run && (
              <button
                type="button"
                onClick={() => publishPv(run.params, run.result)}
                disabled={onScreen || !run.result.previewUrl}
                className={SMALL_BUTTON_CLASS}
              >
                Show PV
              </button>
            )}
          </div>
        )}
        {run && res && summary && (
          <>
            <ProfilePlot
              series={ridgeSeries(res)}
              xLabel={offsetLabel(res.offset_unit)}
              yLabel={pvSpectralLabel(run)}
              toolbar
              csvName={CSV_NAME}
              height={PLOT_HEIGHT}
            />
            <div className="grid grid-cols-2 gap-1">
              <Card label="gradient" value={sig(summary.ridge_gradient, summary.ridge_gradient_unit)} title="Least-squares slope of the ridge over the offsets" />
              <Card label="ridge span" value={sig(summary.ridge_span, res.spectral_unit)} />
              <Card label="slit length" value={`${sig(summary.slit_length, offsetUnit)} (${summary.slit_length_px.toFixed(1)} px)`} />
              <Card label="slit PA" value={summary.slit_pa_deg === null ? EM_DASH : `${summary.slit_pa_deg.toFixed(1)}° E of N`} />
              <Card label="offset step" value={sig(summary.offset_step, offsetUnit)} />
              <Card
                label="peak"
                value={
                  summary.peak_value === null
                    ? EM_DASH
                    : `${sig(summary.peak_value, summary.bunit ?? "")} at ${sig(summary.peak_offset, offsetUnit)}, ${sig(summary.peak_spectral, res.spectral_unit)}`
                }
              />
              <Card label="valid ridge" value={`${summary.n_ridge_valid}/${summary.n_offsets}`} />
              <Card label="cells" value={`${summary.n_offsets} × ${summary.n_channels}, ${summary.n_across} across`} />
            </div>
            <div className="flex flex-wrap items-center gap-1">
              <button type="button" onClick={handleCopyCsv} className={SMALL_BUTTON_CLASS} title="Copy the ridge table as CSV">
                {copied ? "copied" : "Copy CSV"}
              </button>
              <button type="button" onClick={handleSaveCsv} disabled={busy} className={SMALL_BUTTON_CLASS} title="Save the ridge table as CSV">
                Save CSV
              </button>
            </div>
            {ioError && <div className="text-[9px] text-red-400 break-words">{ioError}</div>}
            {res.notes.length > 0 && (
              <details className="text-[10px] font-mono text-zinc-600">
                <summary className="cursor-pointer select-none">notes ({res.notes.length})</summary>
                <ul className="mt-0.5 flex flex-col gap-0.5 text-zinc-500">
                  {res.notes.map((note, i) => (
                    <li key={i} className="break-words">
                      {note}
                    </li>
                  ))}
                </ul>
              </details>
            )}
            <div className="text-[9px] font-mono text-zinc-600 truncate">
              {res.elapsed_ms} ms · {baseName(res.fits_path)}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

export default memo(PvPanel);
