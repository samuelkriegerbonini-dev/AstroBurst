import { memo, useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { Activity, Plus, Loader2 } from "lucide-react";
import type { RadialProfile, LineCut, RegionShape } from "../../shared/types";
import type { SbProfile } from "../../shared/types/regions";
import { radialProfile, lineCut, sbProfile } from "../../services/regions";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useDqContext } from "../../context/PreviewContext";
import { useMeasurementProvenance } from "../../hooks/useMeasurementLog";
import { lineCutEntry, measurementLog, profileLogReady, radialProfileEntry, sbProfileEntry } from "../../utils/measurementLog";
import { shapeSummary } from "../../utils/regionGeometry";
import {
  SB_X_UNITS,
  SB_Y_MODES,
  firstSurfaceBrightness,
  formatPositionAngles,
  formatRadius,
  profileFetchReducer,
  sbProfileCsv,
  sbReferenceLines,
  sbSeries,
  type ProfileFetchState,
  type SbXUnit,
  type SbYMode,
} from "../../utils/sbProfile";
import ProfilePlot, { type ProfileSeries } from "./ProfilePlot";
import LineSkyReadout from "./LineSkyReadout";
import MeasurementBadge from "../analysis/MeasurementBadge";

interface RegionProfilesPanelProps {
  filePath: string | null;
  measurePath: string | null;
}

const PROFILE_DEBOUNCE_MS = 250;
const MEAN_COLOR = "#7dd3fc";
const MEDIAN_COLOR = "#fbbf24";
const CUT_COLOR = "#a78bfa";
const DEFAULT_BIN_WIDTH_TEXT = "1";
const DEFAULT_BIN_WIDTH = 1;
const MIN_BIN_WIDTH = 0.5;
const MAX_BIN_WIDTH = 64;
const BIN_WIDTH_HINT = `bin width must be between ${MIN_BIN_WIDTH} and ${MAX_BIN_WIDTH} px; using ${DEFAULT_BIN_WIDTH}`;
const COPIED_FEEDBACK_MS = 1500;
const CSV_NAME: Record<ProfileRequest["kind"], string> = { radial: "radial-profile", sb: "sb-profile", cut: "line-cut" };
const MAG_DIGITS = 2;
const ELLIPTICITY_DIGITS = 3;
const MISSING_CALIBRATION_REASON = "no flux calibration in the header";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 font-mono focus:border-sky-500/50 w-14 disabled:opacity-40";
const SEGMENT_CLASS = "px-1.5 py-0.5 rounded text-[9px] font-mono text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/60 disabled:opacity-40 disabled:cursor-not-allowed";
const SEGMENT_ACTIVE_CLASS = "px-1.5 py-0.5 rounded text-[9px] font-mono text-sky-300 bg-zinc-800/80";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const SMALL_BUTTON_CLASS =
  "px-2 py-0.5 rounded text-[9px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40";
const LOG_BUTTON_CLASS = `${SMALL_BUTTON_CLASS} inline-flex items-center gap-1`;

type ProfileMode = "radial" | "sb";
const PROFILE_MODES: readonly ProfileMode[] = ["radial", "sb"];
const PROFILE_MODE_LABEL: Record<ProfileMode, string> = { radial: "radial", sb: "SB" };
const Y_MODE_LABEL: Record<SbYMode, string> = { mean: "mean", mu: "mu", ee: "EE" };

type ProfileRequest =
  | { kind: "radial"; x: number; y: number; maxRadius: number; background: [number, number] | null }
  | { kind: "sb"; shape: RegionShape; background: RegionShape | null; binWidth: number }
  | { kind: "cut"; x1: number; y1: number; x2: number; y2: number };

type ProfileResult = { requestKey: string; excludeDq: boolean } & (
  | { kind: "radial"; data: RadialProfile }
  | { kind: "sb"; data: SbProfile }
  | { kind: "cut"; data: LineCut }
);

const NO_PROFILE: ProfileFetchState<ProfileResult> = { result: null, error: null };

function parseBinWidth(text: string): number | null {
  const v = parseFloat(text);
  return Number.isFinite(v) && v >= MIN_BIN_WIDTH && v <= MAX_BIN_WIDTH ? v : null;
}

function requestFor(
  shape: RegionShape | undefined,
  background: RegionShape | undefined,
  mode: ProfileMode,
  binWidth: number,
): ProfileRequest | null {
  if (!shape) return null;
  if (shape.shape === "ellipse" || (shape.shape === "circle" && mode === "sb")) {
    return { kind: "sb", shape, background: background ?? null, binWidth };
  }
  const bg: [number, number] | null =
    background && background.shape === "annulus" ? [background.r_inner, background.r_outer] : null;
  if (shape.shape === "circle") return { kind: "radial", x: shape.x, y: shape.y, maxRadius: shape.r, background: bg };
  if (shape.shape === "annulus") {
    return { kind: "radial", x: shape.x, y: shape.y, maxRadius: shape.r_outer, background: bg };
  }
  if (shape.shape === "line") return { kind: "cut", x1: shape.x1, y1: shape.y1, x2: shape.x2, y2: shape.y2 };
  return null;
}

function panelTitle(kind: ProfileRequest["kind"]): string {
  if (kind === "radial") return "Radial profile";
  if (kind === "sb") return "Surface brightness";
  return "Line cut";
}

function formatMag(v: number | null): string {
  return v === null || !Number.isFinite(v) ? "--" : v.toFixed(MAG_DIGITS);
}

interface SegmentedProps<T extends string> {
  options: readonly T[];
  value: T;
  labels: (v: T) => string;
  disabledOption?: (v: T) => boolean;
  disabled?: boolean;
  onChange: (v: T) => void;
}

function Segmented<T extends string>({ options, value, labels, disabledOption, disabled, onChange }: SegmentedProps<T>) {
  return (
    <div className="flex items-center gap-0.5">
      {options.map((opt) => (
        <button
          key={opt}
          type="button"
          disabled={disabled || disabledOption?.(opt)}
          onClick={() => onChange(opt)}
          className={opt === value ? SEGMENT_ACTIVE_CLASS : SEGMENT_CLASS}
        >
          {labels(opt)}
        </button>
      ))}
    </div>
  );
}

function Card({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="bg-zinc-900/80 rounded px-2 py-1.5" title={title}>
      <div className="text-[9px] text-zinc-500 uppercase">{label}</div>
      <div className="text-[10px] font-mono text-zinc-200 truncate">{value}</div>
    </div>
  );
}

function RegionProfilesPanel({ filePath, measurePath }: RegionProfilesPanelProps) {
  const doc = useRegionDoc(filePath);
  const { excludeDq } = useDqContext();
  const provenance = useMeasurementProvenance();
  const [{ result, error }, dispatch] = useReducer(profileFetchReducer<ProfileResult>, NO_PROFILE);
  const [loading, setLoading] = useState(false);
  const [mode, setMode] = useState<ProfileMode>("radial");
  const [binWidthText, setBinWidthText] = useState(DEFAULT_BIN_WIDTH_TEXT);
  const [xUnit, setXUnit] = useState<SbXUnit>("px");
  const [yMode, setYMode] = useState<SbYMode>("mean");
  const [copied, setCopied] = useState(false);
  const seqRef = useRef(0);

  const selected = doc.regions.find((r) => r.id === doc.selectedId);
  const background = selected?.backgroundId ? doc.regions.find((r) => r.id === selected.backgroundId) : undefined;
  const binWidth = parseBinWidth(binWidthText);
  const request = useMemo(
    () => requestFor(selected?.shape, background?.shape, mode, binWidth ?? DEFAULT_BIN_WIDTH),
    [selected?.shape, background?.shape, mode, binWidth],
  );
  const requestKey = request ? JSON.stringify(request) : null;

  useEffect(() => {
    seqRef.current++;
    dispatch({ type: "reset" });
    setLoading(false);
  }, [measurePath]);

  useEffect(() => {
    const seq = ++seqRef.current;
    if (!measurePath || !requestKey) {
      dispatch({ type: "reset" });
      setLoading(false);
      return;
    }
    const req = JSON.parse(requestKey) as ProfileRequest;
    const timer = setTimeout(async () => {
      setLoading(true);
      try {
        let res: ProfileResult;
        if (req.kind === "radial") {
          res = {
            kind: "radial",
            requestKey,
            excludeDq,
            data: await radialProfile(measurePath, req.x, req.y, req.maxRadius, { background: req.background, excludeDq }),
          };
        } else if (req.kind === "sb") {
          res = {
            kind: "sb",
            requestKey,
            excludeDq,
            data: await sbProfile(measurePath, req.shape, { binWidth: req.binWidth, background: req.background, excludeDq }),
          };
        } else {
          res = { kind: "cut", requestKey, excludeDq, data: await lineCut(measurePath, req.x1, req.y1, req.x2, req.y2, excludeDq) };
        }
        if (seqRef.current !== seq) return;
        dispatch({ type: "success", result: res });
      } catch (e) {
        if (seqRef.current !== seq) return;
        dispatch({ type: "failure", message: e instanceof Error ? e.message : String(e) });
      } finally {
        if (seqRef.current === seq) setLoading(false);
      }
    }, PROFILE_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [measurePath, requestKey, excludeDq]);

  const sb = result?.kind === "sb" ? result.data : null;
  const arcsecAvailable = sb?.pixel_scale_arcsec != null;
  const muAvailable = sb?.photcal != null;
  const effectiveXUnit: SbXUnit = xUnit === "arcsec" && arcsecAvailable ? "arcsec" : "px";
  const effectiveYMode: SbYMode = yMode === "mu" && !muAvailable ? "mean" : yMode;

  const sbLayout = useMemo(() => (sb ? sbSeries(sb, effectiveYMode, effectiveXUnit) : null), [sb, effectiveYMode, effectiveXUnit]);
  const referenceLines = useMemo(() => (sb ? sbReferenceLines(sb, effectiveXUnit) : []), [sb, effectiveXUnit]);

  const series = useMemo<ProfileSeries[]>(() => {
    if (!result) return [];
    if (result.kind === "sb") return sbLayout?.series ?? [];
    if (result.kind === "radial") {
      const bins = result.data.bins;
      const x = bins.map((b) => b.r + 0.5);
      return [
        { x, y: bins.map((b) => b.mean), color: MEAN_COLOR, label: "mean" },
        { x, y: bins.map((b) => b.median), color: MEDIAN_COLOR, label: "median" },
      ];
    }
    return [{ x: result.data.distance, y: result.data.values, color: CUT_COLOR, label: "value" }];
  }, [result, sbLayout]);

  const handleCopyCsv = useCallback(() => {
    if (!sb) return;
    navigator.clipboard?.writeText(sbProfileCsv(sb));
    setCopied(true);
    window.setTimeout(() => setCopied(false), COPIED_FEEDBACK_MS);
  }, [sb]);

  const logReady = profileLogReady(result, requestKey, excludeDq);
  const handleLog = useCallback(() => {
    if (!logReady || !result || !selected) return;
    const req = JSON.parse(result.requestKey) as ProfileRequest;
    if (result.kind === "radial" && req.kind === "radial") {
      measurementLog.append(radialProfileEntry(provenance, result.data, result.excludeDq, req, selected));
    } else if (result.kind === "sb" && req.kind === "sb") {
      measurementLog.append(sbProfileEntry(provenance, result.data, result.excludeDq, req, selected));
    } else if (result.kind === "cut") {
      measurementLog.append(lineCutEntry(provenance, result.data, result.excludeDq, selected));
    }
  }, [logReady, result, selected, provenance]);

  if (!selected || !request) return null;

  const xLabel =
    result?.kind === "sb" ? (sbLayout?.xLabel ?? "") : request.kind === "radial" ? "radius (px)" : "distance (px)";
  const yLabel =
    result?.kind === "sb"
      ? (sbLayout?.yLabel ?? "")
      : result?.kind === "radial" && result.data.background
        ? "value - bg"
        : "value";
  const showControls = request.kind === "radial" || request.kind === "sb";
  const scale = sb?.pixel_scale_arcsec ?? null;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Activity size={12} className="text-sky-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">{panelTitle(request.kind)}</span>
          <MeasurementBadge />
        </div>
        <div className="flex items-center gap-2">
          {result?.data.masked && (
            <span className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30">DQ masked</span>
          )}
          {loading && <Loader2 size={12} className="animate-spin text-sky-400/70" />}
        </div>
      </div>
      <div className="px-3 py-2 space-y-2">
        <div className="text-[10px] font-mono text-zinc-500 truncate">{shapeSummary(selected.shape)}</div>
        {showControls && (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            {selected.shape.shape === "circle" && (
              <div className="flex items-center gap-1">
                <span className={LABEL_CLASS}>mode</span>
                <Segmented options={PROFILE_MODES} value={mode} labels={(m) => PROFILE_MODE_LABEL[m]} onChange={setMode} />
              </div>
            )}
            <label className="flex items-center gap-1">
              <span className={LABEL_CLASS}>bin</span>
              <input
                type="text"
                inputMode="decimal"
                value={binWidthText}
                disabled={request.kind !== "sb"}
                onChange={(e) => setBinWidthText(e.target.value)}
                className={INPUT_CLASS}
                title="Bin width along the semi-major axis in pixels"
              />
              <span className={LABEL_CLASS}>px</span>
            </label>
            <div className="flex items-center gap-1">
              <span className={LABEL_CLASS}>x</span>
              <Segmented
                options={SB_X_UNITS}
                value={effectiveXUnit}
                labels={(u) => u}
                disabled={request.kind !== "sb"}
                disabledOption={(u) => u === "arcsec" && !arcsecAvailable}
                onChange={setXUnit}
              />
            </div>
            <div className="flex items-center gap-1">
              <span className={LABEL_CLASS}>y</span>
              <Segmented
                options={SB_Y_MODES}
                value={effectiveYMode}
                labels={(m) => Y_MODE_LABEL[m]}
                disabled={request.kind !== "sb"}
                disabledOption={(m) => m === "mu" && !muAvailable}
                onChange={setYMode}
              />
            </div>
          </div>
        )}
        {request.kind === "sb" && binWidth === null && <div className="text-[9px] text-amber-300">{BIN_WIDTH_HINT}</div>}
        {error && (
          <div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
            {error}
          </div>
        )}
        {result && (
          <ProfilePlot
            key={result.kind}
            series={series}
            xLabel={xLabel}
            yLabel={yLabel}
            invertY={sbLayout?.invertY ?? false}
            referenceLines={referenceLines}
            toolbar
            logToggle={!(result.kind === "sb" && effectiveYMode === "mu")}
            csvName={CSV_NAME[result.kind]}
          />
        )}
        {sb && (
          <>
            <div className="grid grid-cols-2 gap-1">
              <Card label="R50" value={formatRadius(sb.r50_px, scale)} />
              <Card label="R80" value={formatRadius(sb.r80_px, scale)} />
              <Card label="R90" value={formatRadius(sb.r90_px, scale)} />
              <Card label="Petrosian" value={formatRadius(sb.petrosian_radius_px, scale)} title="First semi-major axis where I(r) / <I(<r)> falls below 0.2" />
              <Card label="mu_0" value={formatMag(firstSurfaceBrightness(sb))} title="Surface brightness of the innermost bin in mag/arcsec^2" />
              <Card label="total AB" value={formatMag(sb.total_mag_ab)} title="AB magnitude of the flux enclosed by the outermost bin" />
              <Card label="ellipticity" value={sb.ellipticity.toFixed(ELLIPTICITY_DIGITS)} />
              <Card label="position angle" value={formatPositionAngles(sb.angle_deg, sb.sky_pa_deg, sb.ellipticity)} />
            </div>
            <div className="flex flex-wrap items-center justify-between gap-1">
              {sb.photcal ? (
                <span
                  className="text-[9px] px-1.5 py-0.5 rounded text-emerald-300 bg-emerald-900/30 truncate max-w-[220px]"
                  title={[sb.photcal.label, ...sb.calibration_warnings].join("\n")}
                >
                  mag/arcsec^2: {sb.photcal.label}
                </span>
              ) : (
                <span
                  className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30 truncate max-w-[220px]"
                  title={sb.calibration_warnings.join("\n") || MISSING_CALIBRATION_REASON}
                >
                  uncalibrated: {sb.calibration_warnings[0] ?? MISSING_CALIBRATION_REASON}
                </span>
              )}
              <button type="button" onClick={handleCopyCsv} className={SMALL_BUTTON_CLASS} title="Copy every bin as CSV">
                {copied ? "copied" : "Copy CSV"}
              </button>
              <button type="button" onClick={handleLog} disabled={loading || !logReady} className={LOG_BUTTON_CLASS} title="Log the profile summary on screen">
                <Plus size={10} /> Log
              </button>
            </div>
            <div className="flex flex-wrap gap-x-3 text-[9px] font-mono text-zinc-500">
              <span>{sb.bins.length} bins</span>
              <span>bin {sb.bin_width} px</span>
              {sb.background && (
                <span>
                  bg {sb.background.median.toExponential(3)} ± {sb.background.sigma.toExponential(2)}
                </span>
              )}
              <span>{sb.elapsed_ms} ms</span>
            </div>
            {sb.notes.length > 0 && (
              <details className="text-[9px] text-zinc-600">
                <summary className="cursor-pointer">notes ({sb.notes.length})</summary>
                <ul className="list-disc pl-4 mt-0.5 space-y-0.5">
                  {sb.notes.map((n) => (
                    <li key={n}>{n}</li>
                  ))}
                </ul>
              </details>
            )}
          </>
        )}
        {result?.kind === "radial" && (
          <div className="flex flex-wrap gap-x-3 text-[9px] font-mono text-zinc-500">
            <span>{result.data.bins.length} bins</span>
            {result.data.background && (
              <span>
                bg {result.data.background.median.toExponential(3)} ± {result.data.background.sigma.toExponential(2)}
              </span>
            )}
            {result.data.bins.length > 0 && (
              <span>cum {result.data.bins[result.data.bins.length - 1].cumulative_sum.toExponential(3)}</span>
            )}
            <button type="button" onClick={handleLog} disabled={loading || !logReady} className={LOG_BUTTON_CLASS} title="Log the profile summary on screen">
              <Plus size={10} /> Log
            </button>
          </div>
        )}
        {result?.kind === "cut" && (
          <div className="space-y-0.5">
            <div className="flex flex-wrap gap-x-3 text-[9px] font-mono text-zinc-500">
              <span>length {result.data.length.toFixed(1)} px</span>
              <span>{result.data.n_samples} samples</span>
              <button type="button" onClick={handleLog} disabled={loading || !logReady} className={LOG_BUTTON_CLASS} title="Log the profile summary on screen">
                <Plus size={10} /> Log
              </button>
            </div>
            <LineSkyReadout measurePath={measurePath} shape={selected.shape} />
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(RegionProfilesPanel);
