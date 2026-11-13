import { memo, useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { Download, Loader2, Shapes, Aperture, X } from "lucide-react";
import { measurePhotometryBatch, photometryGainModel } from "../../services/analysis";
import type { BatchPhotometryResult, GainModel, UserZeroPoint } from "../../services/analysis";
import type { PhotometryStatus } from "../../shared/types/photometryStatus";
import { worldToPixel } from "../../services/astrometry";
import { useDqContext } from "../../context/PreviewContext";
import { useMeasurementProvenance } from "../../hooks/useMeasurementLog";
import { usePhotometryStatus } from "../../hooks/usePhotometryStatus";
import { useImageDepth } from "../../hooks/usePhotometryLimit";
import { useTimedNotice } from "../../hooks/useTimedNotice";
import { dockStore } from "../../hooks/useDockLayout";
import { batchPhotometryEntry, measurementLog, type BatchLogOptions } from "../../utils/measurementLog";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { overlayStore } from "../../utils/overlayStore";
import { regionStore } from "../../utils/regionStore";
import { DEFAULT_REGION_PROPS } from "../../utils/regionPersistence";
import { generateId } from "../../utils/format";
import {
  APERTURE_RANGE_HINT,
  BATCH_SEMANTICS_TEXT,
  FALLBACK_APERTURE_RADIUS_PX,
  MAX_SKY_OUTER_RADIUS_PX,
  NO_DETECTED_STARS_HINT,
  PEAK_SEARCH_RADIUS_PX,
  STARS_ELSEWHERE_NOTICE,
  abCellText,
  apertureRadiusInRange,
  flagDuplicates,
  fluxCellText,
  medianSnr,
  parsePositions,
  photometryCsvFileName,
  photometryTableCsv,
  sortRows,
  summedFluxUnit,
  tableUnitsCaption,
  type PhotometryColumnKey,
  type PhotometryCsvContext,
  type PhotometryTableRow,
  type SortDirection,
} from "../../utils/photometryTable";
import {
  ZERO_POINT_CAPTION,
  ZERO_POINT_FIELD,
  ZERO_POINT_TITLE,
  appliedDepthFactor,
  batchWarnings,
  calibratedMagnitude,
  calibrationText,
  convertedRadiusText,
  depthNoiseText,
  depthRadiusText,
  detectedStarsLabel,
  magnitudeColumnHeader,
  photometryLimit,
  userZeroPoint,
} from "../../utils/photometryPanel";
import { noiseFactorOffer } from "../../utils/forcedPhotometryRequest";
import { DEFAULT_LIMIT_SIGMA } from "../../utils/upperLimits";
import { RADIUS_UNITS, radiiRequest, radiusInArcsec, radiusInPixels, type RadiusUnit } from "../../utils/photometryUnits";
import { parseSkyCsv } from "../../utils/skyList";
import { ANALYSIS_SECTION } from "../../utils/analysisSections";
import { GAIN_TITLE, gainCaption, nextGainText } from "../../utils/photometryGain";
import { ZERO_BASED_PIXEL_TITLE } from "../../utils/regionGeometry";
import { APERTURE_LAYER_ID, APERTURE_LAYER_KIND, createAperturePainter, type ApertureMarker } from "../viewer/painters/aperturePainter";
import { CopyCsvButton, DecimalField, ErrorAlert, RunButton, SMALL_BUTTON_CLASS, Toggle, WarningList } from "../ui";
import { decimalFieldState, firstFieldError, type DecimalFieldOptions } from "../../utils/decimalText";
import MeasurementBadge from "./MeasurementBadge";
import PhotometryStatusLine from "./PhotometryStatusLine";
import type { Star } from "./PlateSolvePanel";

interface PhotometryTablePanelProps {
  filePath: string | null;
  overlayKey: string | null;
  stars: Star[];
  starsElsewhere?: boolean;
  measureKey?: string | null;
  detectedTotal?: number | null;
}

type PositionSource = "stars" | "regions" | "pasted" | "sky";

interface BatchRun {
  result: BatchPhotometryResult;
  labels: string[];
  log: BatchLogOptions;
  status: PhotometryStatus | null;
  fCorr: number | null;
  fCorrRadius: string | null;
}

interface TableColumn {
  key: PhotometryColumnKey;
  header: string;
  note?: string;
}

const DEFAULT_APERTURE_RADIUS_PX = "5";
const TABLE_ROW_LIMIT = 200;
const DUPLICATE_TOLERANCE_PX = 1;
const SAVED_NOTICE_MS = 6000;
const PASTE_ROWS = 4;
const ANNULUS_NEEDS_BOTH = "sky annulus needs both an inner and an outer radius";
const APERTURE_DEFAULTS_CAPTION = `blank: r_ap = 1.5 × FWHM (${FALLBACK_APERTURE_RADIUS_PX} px if the FWHM is not measured), sky 2–3 × r_ap`;
const APERTURE_TITLE = `Aperture radius in pixels; blank uses 1.5 × FWHM of each star, or ${FALLBACK_APERTURE_RADIUS_PX} px for a star whose FWHM is not measured`;
const SKY_IN_TITLE = "Inner sky radius in pixels; blank uses 2 × r_ap";
const SKY_OUT_TITLE = "Outer sky radius in pixels; blank uses 3 × r_ap";
const ARCSEC_APERTURE_TITLE = APERTURE_TITLE.replace("in pixels", "in arcsec");
const ARCSEC_SKY_IN_TITLE = SKY_IN_TITLE.replace("in pixels", "in arcsec");
const ARCSEC_SKY_OUT_TITLE = SKY_OUT_TITLE.replace("in pixels", "in arcsec");
const NO_ARCSEC_TITLE = "No celestial WCS: radii in arcsec are not available for this image";
const PIXEL_SCALE_PENDING = "Reading the pixel scale of this image...";
const SKY_PLACEHOLDER = "53.1301 -28.1002 a\n03:32:31.2 -28:06:00.7 b";
const SKY_POSITIONS_NOTE = `Each sky position is a starting point: it snaps to the brightest pixel within ${PEAK_SEARCH_RADIUS_PX} px and is recentred, like every other source of positions. For positions that must not move, use the ${ANALYSIS_SECTION.forced.label} section.`;
const NO_CELESTIAL_WCS = "This image has no celestial WCS: sky positions cannot be placed on it.";
const ZERO_BASED_CAPTION = "x, y: 0-based pixel centres; DS9 and .reg files are 1-based";
const USE_DEPTH_FACTOR_LABEL = "Use the Depth noise factor for this aperture";
const REMOVE_DEPTH_FACTOR_LABEL = "Do not use the Depth noise factor";
const NO_STARS: Star[] = [];

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const UNIT_SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono";

const TABLE_COLUMNS: TableColumn[] = [
  { key: "label", header: "label" },
  { key: "x", header: "x", note: ZERO_BASED_PIXEL_TITLE },
  { key: "y", header: "y", note: ZERO_BASED_PIXEL_TITLE },
  { key: "net_flux", header: "flux" },
  { key: "snr", header: "SNR" },
  { key: "mag_ab", header: "AB" },
  { key: "mag_inst", header: "inst" },
  { key: "fwhm", header: "FWHM" },
  { key: "ee50_radius", header: "EE50" },
  { key: "aperture_correction", header: "corr" },
];

const APERTURE_FIELD: DecimalFieldOptions = { optional: true };
const SKY_IN_FIELD: DecimalFieldOptions = { optional: true, min: 1 };
const SKY_OUT_FIELD: DecimalFieldOptions = { optional: true, min: 1, max: MAX_SKY_OUTER_RADIUS_PX };
const GAIN_FIELD: DecimalFieldOptions = { optional: true, min: 0.01 };
const ARCSEC_FIELD: DecimalFieldOptions = { optional: true };

function fmt(v: number | null | undefined, digits: number): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "--";
  return Math.abs(v) >= 1e6 ? v.toExponential(2) : v.toFixed(digits);
}

function cellText(row: PhotometryTableRow, column: TableColumn): string {
  if (column.key === "label") return row.label;
  const p = row.photometry;
  if (!p) return "--";
  switch (column.key) {
    case "x":
      return fmt(p.x, 1);
    case "y":
      return fmt(p.y, 1);
    case "net_flux":
      return fluxCellText(p.net_flux);
    case "snr":
      return fmt(p.snr, 1);
    case "mag_ab":
      return row.limit?.isLimit && row.limit.limitAb !== null
        ? `> ${row.limit.limitAb.toFixed(2)}`
        : abCellText(row.magnitude?.mag, row.magnitude?.magErr);
    case "mag_inst":
      return fmt(p.mag_inst, 2);
    case "fwhm":
      return fmt(p.fwhm, 2);
    case "ee50_radius":
      return fmt(p.ee50_radius, 2);
    case "aperture_correction":
      return fmt(p.aperture_correction, 3);
    default:
      return "--";
  }
}

function radiusEcho(aperture: number | null, unit: RadiusUnit, arcsecPerPx: number | null): string | null {
  if (aperture === null) return null;
  if (unit === "px") {
    const arcsec = radiusInArcsec(aperture, arcsecPerPx);
    return arcsec === null ? null : `r_ap ${aperture} px = ${arcsec.toFixed(3)} arcsec`;
  }
  const px = radiusInPixels(aperture, "arcsec", arcsecPerPx);
  return px === null ? null : `r_ap ${aperture} arcsec = ${px.toFixed(2)} px`;
}

function parseErrorLines(errors: string[]): ReactNode {
  if (errors.length === 0) return null;
  return (
    <div className="text-[9px] text-red-400 space-y-0.5">
      {errors.slice(0, 5).map((msg) => (
        <div key={msg}>{msg}</div>
      ))}
      {errors.length > 5 && <div>{errors.length - 5} more</div>}
    </div>
  );
}

function rowFlags(row: PhotometryTableRow, duplicate: boolean): string {
  if (row.error) return row.error;
  const p = row.photometry;
  const flags: string[] = [];
  if (duplicate) flags.push("dup");
  if (p?.saturated) flags.push("sat");
  if (row.limit?.isLimit) flags.push(`below ${row.limit.sigma} sigma`);
  if (p && p.n_masked > 0) flags.push(`${p.n_masked} masked`);
  if (p && p.n_nonfinite > 0) flags.push(`${p.n_nonfinite} no data`);
  if (p && p.aperture_correction == null) flags.push("no plateau");
  return flags.join(", ");
}

function limitCaptionText(limits: number, measured: number, fCorr: number | null): string {
  const caption = `${limits} of ${measured} measured rows are below ${DEFAULT_LIMIT_SIGMA} sigma: a magnitude cell with > is a ${DEFAULT_LIMIT_SIGMA} sigma limit; the CSV keeps the measured flux and adds the limit columns`;
  return fCorr === null ? caption : `${caption} Errors multiplied by f_corr ${fCorr.toFixed(2)} from the Depth section where an error layer was used.`;
}

function PhotometryTablePanel({
  filePath,
  overlayKey,
  stars: detectedStars,
  starsElsewhere = false,
  measureKey = null,
  detectedTotal = null,
}: PhotometryTablePanelProps) {
  const sourceId = useId();
  const apertureId = useId();
  const skyInId = useId();
  const skyOutId = useId();
  const gainId = useId();
  const zeroPointId = useId();
  const pasteId = useId();
  const skyId = useId();
  const [source, setSource] = useState<PositionSource>("stars");
  const [apertureText, setApertureText] = useState(DEFAULT_APERTURE_RADIUS_PX);
  const [skyInText, setSkyInText] = useState("");
  const [skyOutText, setSkyOutText] = useState("");
  const [radiusUnit, setRadiusUnit] = useState<RadiusUnit>("px");
  const [gainText, setGainText] = useState("");
  const [gainModel, setGainModel] = useState<GainModel | null>(null);
  const [zeroPointText, setZeroPointText] = useState("");
  const [zeroPointSystem, setZeroPointSystem] = useState<UserZeroPoint["system"]>("ab");
  const [zeroPointPerSecond, setZeroPointPerSecond] = useState(false);
  const [pastedText, setPastedText] = useState("");
  const [skyText, setSkyText] = useState("");
  const [placementNote, setPlacementNote] = useState<string | null>(null);
  const [run, setRun] = useState<BatchRun | null>(null);
  const [acceptedFCorr, setAcceptedFCorr] = useState<number | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, showSavedPath, clearSavedPath] = useTimedNotice<string>(SAVED_NOTICE_MS);
  const [sortKey, setSortKey] = useState<PhotometryColumnKey>("index");
  const [sortDir, setSortDir] = useState<SortDirection>("asc");
  const [highlightIndex, setHighlightIndex] = useState<number | null>(null);
  const [showApertures, setShowApertures] = useState(true);
  const { excludeDq } = useDqContext();
  const status = usePhotometryStatus(filePath, excludeDq, measureKey ?? null);
  const arcsecPerPx = status.status?.pixel_scale_arcsec ?? null;
  const depth = useImageDepth(filePath ?? null);
  const provenance = useMeasurementProvenance();
  const regionDoc = useRegionDoc(overlayKey);
  const requestSeqRef = useRef(0);
  const gainTextRef = useRef(gainText);
  gainTextRef.current = gainText;
  const gainPrefillRef = useRef("");
  const gainSeqRef = useRef(0);
  const stars = starsElsewhere ? NO_STARS : detectedStars;

  useEffect(() => {
    requestSeqRef.current++;
    setRun(null);
    setError(null);
    setRunning(false);
    setHighlightIndex(null);
    setPlacementNote(null);
    clearSavedPath();
  }, [filePath, measureKey, clearSavedPath]);

  useEffect(() => {
    const seq = ++gainSeqRef.current;
    setGainModel(null);
    const cleared = nextGainText(gainTextRef.current, gainPrefillRef.current, null);
    gainPrefillRef.current = cleared.prefill;
    gainTextRef.current = cleared.text;
    setGainText(cleared.text);
    if (!filePath) return;
    const apply = (model: GainModel | null) => {
      if (gainSeqRef.current !== seq) return;
      const next = nextGainText(gainTextRef.current, gainPrefillRef.current, model);
      gainPrefillRef.current = next.prefill;
      setGainModel(model);
      setGainText(next.text);
    };
    photometryGainModel(filePath).then(apply, () => apply(null));
  }, [filePath]);

  const pointRegions = useMemo(() => regionDoc.regions.filter((r) => r.shape.shape === "point"), [regionDoc]);
  const pasted = useMemo(() => parsePositions(pastedText), [pastedText]);
  const skyList = useMemo(() => parseSkyCsv(skyText, true), [skyText]);

  const input = useMemo((): { points: [number, number][]; labels: string[] } => {
    if (source === "stars") {
      return { points: stars.map((s) => [s.x, s.y]), labels: stars.map((_, i) => `star ${i + 1}`) };
    }
    if (source === "regions") {
      const points: [number, number][] = [];
      const labels: string[] = [];
      for (const region of pointRegions) {
        if (region.shape.shape !== "point") continue;
        points.push([region.shape.x, region.shape.y]);
        labels.push(region.props.text || `point ${labels.length + 1}`);
      }
      return { points, labels };
    }
    if (source === "sky") {
      return {
        points: skyList.targets.map((t): [number, number] => [t.lon, t.lat]),
        labels: skyList.targets.map((t, i) => t.label || `s${i + 1}`),
      };
    }
    return { points: pasted.points, labels: pasted.points.map((_, i) => `p${i + 1}`) };
  }, [source, stars, pointRegions, pasted, skyList]);

  const inArcsec = radiusUnit === "arcsec";
  const apertureOptions = inArcsec ? ARCSEC_FIELD : APERTURE_FIELD;
  const skyInOptions = inArcsec ? ARCSEC_FIELD : SKY_IN_FIELD;
  const skyOutOptions = inArcsec ? ARCSEC_FIELD : SKY_OUT_FIELD;
  const apertureField = decimalFieldState(apertureText, apertureOptions);
  const skyInField = decimalFieldState(skyInText, skyInOptions);
  const skyOutField = decimalFieldState(skyOutText, skyOutOptions);
  const gainField = decimalFieldState(gainText, GAIN_FIELD);
  const typedRadii = useMemo(
    () => ({ aperture: apertureField.value ?? undefined, skyInner: skyInField.value ?? undefined, skyOuter: skyOutField.value ?? undefined }),
    [apertureField.value, skyInField.value, skyOutField.value],
  );
  const request = radiiRequest(typedRadii, radiusUnit, arcsecPerPx);
  const { apertureRadius, annulusInner, annulusOuter } = request.radii;
  const gain = gainField.value ?? undefined;
  const offersZeroPoint = status.status !== null && status.status.calibration_source !== "header";
  const zeroPointInput = offersZeroPoint ? zeroPointText : "";
  const zeroPointField = decimalFieldState(zeroPointInput, ZERO_POINT_FIELD);
  const zeroPoint = useMemo(
    () => userZeroPoint(zeroPointInput, zeroPointSystem, zeroPointPerSecond),
    [zeroPointInput, zeroPointSystem, zeroPointPerSecond],
  );
  const fieldError = firstFieldError([
    [`r_ap ${radiusUnit}`, apertureField],
    [`sky in ${radiusUnit}`, skyInField],
    [`sky out ${radiusUnit}`, skyOutField],
    ["gain e-/ADU", gainField],
    ["zero point", zeroPointField],
  ]);
  const statusPending = status.error !== null ? `Status not available: ${status.error}` : PIXEL_SCALE_PENDING;
  const radiusError = inArcsec && status.status === null ? statusPending : request.error;
  const echo = radiusEcho(apertureField.value, radiusUnit, arcsecPerPx);
  const noCelestialWcs = status.status !== null && arcsecPerPx === null;
  const skyUnplaceable = source === "sky" && noCelestialWcs;
  const annulusHalfFilled = (annulusInner === undefined) !== (annulusOuter === undefined);
  const apertureOutOfRange = !apertureRadiusInRange(apertureRadius);
  const offer = useMemo(() => noiseFactorOffer(depth, apertureField.value ?? null, radiusUnit), [depth, apertureField.value, radiusUnit]);
  const fCorr = appliedDepthFactor(offer, acceptedFCorr);
  const noiseText = depthNoiseText(offer, acceptedFCorr);

  useEffect(() => {
    if (acceptedFCorr !== null && appliedDepthFactor(offer, acceptedFCorr) === null) setAcceptedFCorr(null);
  }, [offer, acceptedFCorr]);

  const measure = useCallback(async () => {
    if (!filePath || input.points.length === 0 || apertureOutOfRange || fieldError !== null || radiusError !== null || skyUnplaceable) return;
    const seq = ++requestSeqRef.current;
    const measuredStatus = status.status ?? null;
    const fCorrRadius = fCorr === null ? null : depthRadiusText(offer);
    let { points, labels } = input;
    let placement: BatchLogOptions["placement"] = null;
    setRunning(true);
    setError(null);
    setHighlightIndex(null);
    setPlacementNote(null);
    try {
      if (source === "sky") {
        const placed = await worldToPixel(filePath, points, "icrs");
        if (requestSeqRef.current !== seq) return;
        const kept = labels.flatMap((label, i) => {
          const point = placed.points[i];
          return placed.on_image[i] && point ? [{ point, label }] : [];
        });
        if (kept.length === 0) {
          setError(`None of the ${points.length} sky positions falls on this image.`);
          return;
        }
        const left = points.length - kept.length;
        if (left > 0) setPlacementNote(`${left} of ${points.length} sky positions are outside the image and were left out`);
        placement = { requested: points.length, onImage: kept.length };
        points = kept.map((k) => k.point);
        labels = kept.map((k) => k.label);
      }
      const result = await measurePhotometryBatch(filePath, points, {
        apertureRadius,
        annulusInner,
        annulusOuter,
        gain,
        excludeDq,
        zeroPoint,
      });
      if (requestSeqRef.current !== seq) return;
      const log: BatchLogOptions = {
        apertureRadius,
        annulusInner,
        annulusOuter,
        gain,
        excludeDq,
        from: source,
        radiusUnit,
        typedRadii,
        pixelScaleArcsec: arcsecPerPx,
        zeroPoint: zeroPoint ?? null,
        placement,
        limitSigma: DEFAULT_LIMIT_SIGMA,
        fCorr,
        nLimits: result.rows.filter((r) => r.photometry !== null && photometryLimit(r.photometry, fCorr).isLimit).length,
      };
      setRun({ result, labels, log, status: measuredStatus, fCorr, fCorrRadius });
      measurementLog.append(batchPhotometryEntry(provenance, result, log));
    } catch (e: unknown) {
      if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (requestSeqRef.current === seq) setRunning(false);
    }
  }, [
    filePath,
    input,
    apertureOutOfRange,
    fieldError,
    radiusError,
    skyUnplaceable,
    apertureRadius,
    annulusInner,
    annulusOuter,
    gain,
    zeroPoint,
    excludeDq,
    provenance,
    source,
    radiusUnit,
    typedRadii,
    arcsecPerPx,
    status.status,
    fCorr,
    offer,
  ]);

  const rows = useMemo(
    (): PhotometryTableRow[] =>
      run
        ? run.result.rows.map((r) => ({
            index: r.index,
            label: run.labels[r.index] ?? String(r.index),
            photometry: r.photometry,
            sky: r.sky,
            error: r.error,
            magnitude: r.photometry ? calibratedMagnitude(r.photometry, run.result.photcal) : undefined,
            limit: r.photometry ? photometryLimit(r.photometry, run.fCorr) : undefined,
          }))
        : [],
    [run],
  );
  const limitCaption = useMemo(() => {
    if (!run) return null;
    const measured = rows.filter((row) => row.photometry !== null);
    const limits = measured.filter((row) => row.limit?.isLimit).length;
    return limits > 0 ? limitCaptionText(limits, measured.length, run.fCorr) : null;
  }, [run, rows]);
  const duplicates = useMemo(() => flagDuplicates(rows, DUPLICATE_TOLERANCE_PX), [rows]);
  const sorted = useMemo(() => sortRows(rows, sortKey, sortDir), [rows, sortKey, sortDir]);
  const shown = useMemo(() => sorted.slice(0, TABLE_ROW_LIMIT), [sorted]);
  const median = useMemo(() => medianSnr(rows), [rows]);

  useEffect(() => {
    if (!overlayKey || rows.length === 0 || !showApertures) return;
    const apertures: ApertureMarker[] = [];
    const indices: number[] = [];
    for (const row of rows) {
      const p = row.photometry;
      if (!p) continue;
      apertures.push({ x: p.x, y: p.y, rAp: p.aperture_radius, skyIn: p.sky_inner, skyOut: p.sky_outer, label: row.label });
      indices.push(row.index);
    }
    const highlight = highlightIndex === null ? -1 : indices.indexOf(highlightIndex);
    overlayStore.add(overlayKey, {
      id: APERTURE_LAYER_ID,
      kind: APERTURE_LAYER_KIND,
      visible: true,
      paint: createAperturePainter({ apertures, highlightIndex: highlight >= 0 ? highlight : null }),
    });
    return () => overlayStore.remove(overlayKey, APERTURE_LAYER_ID);
  }, [overlayKey, rows, highlightIndex, showApertures]);

  const clearTable = useCallback(() => {
    requestSeqRef.current++;
    setRun(null);
    setError(null);
    setRunning(false);
    setHighlightIndex(null);
    setPlacementNote(null);
    clearSavedPath();
  }, [clearSavedPath]);

  const toggleSort = useCallback(
    (key: PhotometryColumnKey) => {
      if (key === sortKey) setSortDir((d) => (d === "asc" ? "desc" : "asc"));
      else {
        setSortKey(key);
        setSortDir("asc");
      }
    },
    [sortKey],
  );

  const csvContext = useCallback(
    (batch: BatchRun): PhotometryCsvContext => ({
      exportedUtc: new Date().toISOString(),
      fileName: provenance.file,
      image: provenance.image,
      positions: batch.log.from,
      bunit: batch.result.bunit ?? batch.result.photcal?.bunit ?? null,
      summedUnit: batch.result.summed_unit ?? summedFluxUnit(batch.result.photcal),
      calibration: calibrationText(batch.result.photcal),
      calibrationNotes: batch.result.photcal?.notes ?? [],
      magSystem: batch.result.photcal ? (batch.result.photcal.mag_system ?? "ab") : null,
      zeroPoint: batch.log.zeroPoint ?? null,
      filter: batch.status?.filter ?? null,
      pixelScaleArcsec: batch.log.pixelScaleArcsec ?? null,
      radiusUnit: batch.log.radiusUnit ?? "px",
      typedRadii: batch.log.typedRadii ?? {},
      gain: batch.result.gain_used ?? null,
      excludeDq: batch.log.excludeDq,
      masked: batch.result.masked,
      errLayer: batch.status?.err?.origin ?? null,
      dqLayer: batch.status?.dq?.source ?? null,
      warnings: batchWarnings(batch.result),
      limitSigma: DEFAULT_LIMIT_SIGMA,
      fCorr: batch.fCorr,
      fCorrRadius: batch.fCorrRadius,
    }),
    [provenance],
  );

  const copyText = useCallback(() => (run && rows.length > 0 ? photometryTableCsv(rows, csvContext(run)) : null), [run, rows, csvContext]);

  const saveCsv = useCallback(async () => {
    if (!filePath || !run || rows.length === 0) return;
    setError(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: photometryCsvFileName(filePath),
        filters: [{ name: "CSV", extensions: ["csv"] }],
        title: "Save photometry table",
      });
      if (!target) return;
      const { writeTextFile } = await import("@tauri-apps/plugin-fs");
      await writeTextFile(target, photometryTableCsv(rows, csvContext(run)));
      showSavedPath(target);
      measurementLog.append(batchPhotometryEntry(provenance, run.result, { ...run.log, saved: { path: target, rows: rows.length } }));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [filePath, run, rows, showSavedPath, provenance, csvContext]);

  const addAsRegions = useCallback(() => {
    if (!overlayKey) return;
    for (const row of rows) {
      const p = row.photometry;
      if (!p) continue;
      regionStore.add(overlayKey, {
        id: generateId(),
        shape: { shape: "point", x: p.x, y: p.y },
        props: { ...DEFAULT_REGION_PROPS, text: row.label },
        backgroundId: null,
      });
    }
  }, [overlayKey, rows]);

  const changeRadiusUnit = (next: RadiusUnit) => {
    const clears = radiusUnit === "arcsec" && next === "px" && arcsecPerPx === null;
    const convert = (text: string) => (clears ? "" : convertedRadiusText(text, radiusUnit, next, arcsecPerPx));
    setApertureText(convert);
    setSkyInText(convert);
    setSkyOutText(convert);
    setRadiusUnit(next);
  };

  const result = run?.result ?? null;
  const calibration = result ? (calibrationText(result.photcal) ?? "uncalibrated") : null;
  const magnitudeHeader = magnitudeColumnHeader(result?.photcal?.mag_system);
  const unitsCaption = result ? tableUnitsCaption(result.summed_unit ?? summedFluxUnit(result.photcal)) : null;
  const canMeasure =
    !!filePath && input.points.length > 0 && !annulusHalfFilled && !apertureOutOfRange && fieldError === null && radiusError === null && !skyUnplaceable;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Aperture size={12} className="text-amber-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Photometry table</span>
          <MeasurementBadge />
        </div>
        {running && <Loader2 size={12} className="animate-spin text-amber-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <PhotometryStatusLine filePath={filePath ?? null} measureKey={measureKey ?? null} excludeDq={excludeDq} zeroPoint={zeroPoint ?? null} />
        <div className="flex flex-col gap-0.5">
          <label htmlFor={sourceId} className="text-[9px] text-zinc-500 uppercase">
            Positions
          </label>
          <select id={sourceId} value={source} onChange={(e) => setSource(e.target.value as PositionSource)} className={SELECT_CLASS}>
            <option value="stars">{detectedStarsLabel(stars.length, starsElsewhere ? null : detectedTotal)}</option>
            <option value="regions">Point regions ({pointRegions.length})</option>
            <option value="pasted">Pasted positions</option>
            <option value="sky">Sky positions (RA Dec)</option>
          </select>
          {starsElsewhere && source === "stars" && <div className="text-[9px] text-amber-400/90">{STARS_ELSEWHERE_NOTICE}</div>}
          {!starsElsewhere && source === "stars" && stars.length === 0 && (
            <div className="flex flex-wrap items-center gap-1.5">
              <div className="text-[9px] text-zinc-500">{NO_DETECTED_STARS_HINT}</div>
              <button type="button" onClick={() => dockStore.dispatch({ type: "open", tool: "astrometry" })} className={SMALL_BUTTON_CLASS}>
                Open Astrometry
              </button>
            </div>
          )}
        </div>

        {source === "pasted" && (
          <div className="flex flex-col gap-0.5">
            <label htmlFor={pasteId} className="text-[9px] text-zinc-500 uppercase">
              x y per line (0-based pixels, # comments)
            </label>
            <textarea
              id={pasteId}
              rows={PASTE_ROWS}
              value={pastedText}
              placeholder={"120.5 88.2\n301, 240"}
              onChange={(e) => setPastedText(e.target.value)}
              className={INPUT_CLASS}
              spellCheck={false}
            />
            {parseErrorLines(pasted.errors)}
          </div>
        )}

        {source === "sky" && (
          <div className="flex flex-col gap-0.5">
            <label htmlFor={skyId} className="text-[9px] text-zinc-500 uppercase">
              RA Dec [label] per line, or CSV with ra/dec headers (ICRS, # comments)
            </label>
            <textarea
              id={skyId}
              rows={PASTE_ROWS}
              value={skyText}
              placeholder={SKY_PLACEHOLDER}
              onChange={(e) => setSkyText(e.target.value)}
              className={INPUT_CLASS}
              spellCheck={false}
            />
            {parseErrorLines(skyList.errors)}
            {noCelestialWcs && <div className="text-[9px] text-amber-400/90">{NO_CELESTIAL_WCS}</div>}
            <div className="text-[9px] text-zinc-500">{SKY_POSITIONS_NOTE}</div>
          </div>
        )}

        <div className="flex justify-end">
          <select
            aria-label="Radius unit"
            value={radiusUnit}
            onChange={(e) => changeRadiusUnit(e.target.value as RadiusUnit)}
            className={UNIT_SELECT_CLASS}
          >
            {RADIUS_UNITS.map((unit) => {
              const unavailable = unit === "arcsec" && arcsecPerPx === null;
              return (
                <option key={unit} value={unit} disabled={unavailable} title={unavailable ? NO_ARCSEC_TITLE : undefined}>
                  {unit}
                </option>
              );
            })}
          </select>
        </div>

        <div className="grid grid-cols-4 gap-1.5">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={apertureId} className="text-[9px] text-zinc-500">
              r_ap {radiusUnit}
            </label>
            <DecimalField
              id={apertureId}
              {...apertureOptions}
              value={apertureText}
              placeholder="auto"
              title={inArcsec ? ARCSEC_APERTURE_TITLE : APERTURE_TITLE}
              onChange={setApertureText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={skyInId} className="text-[9px] text-zinc-500">
              sky in {radiusUnit}
            </label>
            <DecimalField
              id={skyInId}
              {...skyInOptions}
              value={skyInText}
              placeholder="2×r"
              title={inArcsec ? ARCSEC_SKY_IN_TITLE : SKY_IN_TITLE}
              onChange={setSkyInText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={skyOutId} className="text-[9px] text-zinc-500">
              sky out {radiusUnit}
            </label>
            <DecimalField
              id={skyOutId}
              {...skyOutOptions}
              value={skyOutText}
              placeholder="3×r"
              title={inArcsec ? ARCSEC_SKY_OUT_TITLE : SKY_OUT_TITLE}
              onChange={setSkyOutText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={gainId} className="text-[9px] text-zinc-500">
              gain e-/ADU
            </label>
            <DecimalField
              id={gainId}
              {...GAIN_FIELD}
              value={gainText}
              placeholder="none"
              title={GAIN_TITLE}
              data-gain-source={gainModel?.source ?? ""}
              onChange={setGainText}
              className={INPUT_CLASS}
            />
          </div>
        </div>
        {echo && (
          <div className="text-[9px] text-zinc-500" data-radius-echo>
            {echo}
          </div>
        )}
        <div className="text-[9px] text-zinc-600">{APERTURE_DEFAULTS_CAPTION}</div>
        <div className="text-[9px] text-zinc-500" data-gain-caption>
          {gainCaption(gainModel, gainText)}
        </div>
        {offersZeroPoint && (
          <div className="flex flex-col gap-0.5">
            <label htmlFor={zeroPointId} className="text-[9px] text-zinc-500">
              zero point mag
            </label>
            <div className="flex items-center gap-1.5">
              <DecimalField
                id={zeroPointId}
                {...ZERO_POINT_FIELD}
                value={zeroPointText}
                placeholder="none"
                title={ZERO_POINT_TITLE}
                onChange={setZeroPointText}
                className={INPUT_CLASS}
              />
              <select
                aria-label="Zero point system"
                value={zeroPointSystem}
                onChange={(e) => setZeroPointSystem(e.target.value as UserZeroPoint["system"])}
                className={UNIT_SELECT_CLASS}
              >
                <option value="ab">AB</option>
                <option value="vega">Vega</option>
              </select>
            </div>
            <Toggle label="Zero point is per second" checked={zeroPointPerSecond} accent="amber" onChange={setZeroPointPerSecond} />
            <div className="text-[9px] text-zinc-500">{ZERO_POINT_CAPTION}</div>
          </div>
        )}
        {annulusHalfFilled && <div className="text-[9px] text-amber-400/90">{ANNULUS_NEEDS_BOTH}</div>}
        {apertureOutOfRange && <div className="text-[9px] text-amber-400/90">{APERTURE_RANGE_HINT}</div>}
        {fieldError && <div className="text-[9px] text-amber-400/90" data-field-error>{fieldError}</div>}
        {radiusError && (
          <div className="text-[9px] text-amber-400/90" data-radius-error>
            {radiusError}
          </div>
        )}
        {noiseText && (
          <div data-depth-noise-offer={offer.kind} className="text-[9px] text-zinc-500">
            {noiseText}{" "}
            {offer.kind === "usable" &&
              (acceptedFCorr === offer.fCorr ? (
                <button
                  type="button"
                  data-action="remove-depth-fcorr"
                  aria-label={REMOVE_DEPTH_FACTOR_LABEL}
                  onClick={() => setAcceptedFCorr(null)}
                  className="text-zinc-500 hover:text-zinc-200 transition-colors"
                >
                  Remove
                </button>
              ) : (
                <button
                  type="button"
                  data-action="use-depth-fcorr"
                  aria-label={USE_DEPTH_FACTOR_LABEL}
                  onClick={() => setAcceptedFCorr(offer.fCorr)}
                  className="text-zinc-500 hover:text-zinc-200 transition-colors"
                >
                  Use
                </button>
              ))}
          </div>
        )}

        <RunButton
          label={`Measure ${input.points.length}`}
          runningLabel="Measuring..."
          running={running}
          disabled={!canMeasure}
          accent="amber"
          icon={<Aperture size={12} />}
          onClick={() => void measure()}
        />

        {placementNote && (
          <div className="text-[9px] text-amber-400/90" data-placement-note>
            {placementNote}
          </div>
        )}

        <ErrorAlert message={error} />

        {result && (
          <div className="grid grid-cols-2 gap-1.5 text-[10px]">
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Measured / failed</div>
              <div className="text-amber-300 font-mono">
                {result.n_measured} / {result.n_failed}
                <span className="text-zinc-500"> in {result.elapsed_ms} ms</span>
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Calibration</div>
              <div className={`font-mono ${result.photcal ? "text-zinc-300" : "text-amber-400/90"}`}>{calibration}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Median SNR</div>
              <div className="text-amber-300 font-mono">{fmt(median, 1)}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Duplicates (within {DUPLICATE_TOLERANCE_PX} px)</div>
              <div className={`font-mono ${duplicates.size > 0 ? "text-amber-400/90" : "text-zinc-300"}`}>{duplicates.size}</div>
            </div>
          </div>
        )}

        <WarningList warnings={result ? batchWarnings(result) : null} />

        {shown.length > 0 && (
          <div className="max-h-56 overflow-auto border border-zinc-800/60 rounded">
            <table className="w-full text-[9px] font-mono">
              <thead className="sticky top-0 bg-zinc-900 text-zinc-500">
                <tr>
                  {TABLE_COLUMNS.map((column) => {
                    const sorted = sortKey === column.key;
                    const header = column.key === "mag_ab" ? magnitudeHeader : column.header;
                    return (
                      <th
                        key={column.key}
                        aria-sort={sorted ? (sortDir === "asc" ? "ascending" : "descending") : "none"}
                        className={`${column.key === "label" ? "text-left" : "text-right"} px-1.5 py-0.5`}
                      >
                        <button
                          type="button"
                          onClick={() => toggleSort(column.key)}
                          className="cursor-pointer select-none hover:text-zinc-300"
                          title={column.note ? `Sort by ${header}\n${column.note}` : `Sort by ${header}`}
                        >
                          {header}
                          {sorted && <span aria-hidden="true" className="ml-0.5">{sortDir === "asc" ? "▲" : "▼"}</span>}
                        </button>
                      </th>
                    );
                  })}
                  <th className="text-left px-1.5 py-0.5">flags</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((row) => {
                  const duplicate = duplicates.has(row.index);
                  const tone = row.error ? "text-red-300" : duplicate ? "text-amber-200" : "text-zinc-300";
                  return (
                    <tr
                      key={row.index}
                      onMouseEnter={() => setHighlightIndex(row.index)}
                      onMouseLeave={() => setHighlightIndex(null)}
                      className={`${tone} ${highlightIndex === row.index ? "bg-zinc-800/80" : ""}`}
                    >
                      {TABLE_COLUMNS.map((column) => (
                        <td
                          key={column.key}
                          data-column={column.key}
                          className={`${column.key === "label" ? "text-left truncate max-w-[90px]" : "text-right whitespace-nowrap"} px-1.5 py-0.5`}
                          title={column.key === "label" ? row.label : undefined}
                        >
                          {cellText(row, column)}
                        </td>
                      ))}
                      <td className="text-left px-1.5 py-0.5 truncate max-w-[140px]" title={rowFlags(row, duplicate)}>
                        {rowFlags(row, duplicate)}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {rows.length > TABLE_ROW_LIMIT && (
              <div className="text-[9px] text-zinc-600 px-1.5 py-0.5">
                showing {TABLE_ROW_LIMIT} of {rows.length}; the CSV holds every row
              </div>
            )}
            <div className="text-[9px] text-zinc-600 px-1.5 py-0.5">{unitsCaption}</div>
            {limitCaption && (
              <div className="text-[9px] text-zinc-600 px-1.5 py-0.5" data-limit-caption>
                {limitCaption}
              </div>
            )}
            <div className="text-[9px] text-zinc-600 px-1.5 py-0.5" title={ZERO_BASED_PIXEL_TITLE}>
              {ZERO_BASED_CAPTION}
            </div>
          </div>
        )}

        {rows.length > 0 && (
          <Toggle label="Show apertures" checked={showApertures} disabled={!overlayKey} accent="amber" onChange={setShowApertures} />
        )}

        {rows.length > 0 && (
          <div className="flex flex-wrap items-center gap-1.5">
            <CopyCsvButton text={copyText} onError={setError} />
            <button type="button" onClick={() => void saveCsv()} disabled={!filePath} className={SMALL_BUTTON_CLASS}>
              <Download size={10} />
              Save CSV
            </button>
            <button
              type="button"
              onClick={addAsRegions}
              disabled={!overlayKey || (result?.n_measured ?? 0) === 0}
              className={SMALL_BUTTON_CLASS}
              title="Add every measured centroid as a Point region named by its label"
            >
              <Shapes size={10} />
              Add as Point regions
            </button>
            <button
              type="button"
              onClick={clearTable}
              className={SMALL_BUTTON_CLASS}
              title="Discard this table and remove its apertures and labels from the image; point regions already added stay"
            >
              <X size={10} />
              Clear
            </button>
          </div>
        )}

        {savedPath && (
          <div role="status" className="text-[9px] text-emerald-400/90 break-all">
            Saved {savedPath}
          </div>
        )}

        {!result && !error && <div className="text-[10px] text-zinc-600">{BATCH_SEMANTICS_TEXT}</div>}
      </div>
    </div>
  );
}

export default memo(PhotometryTablePanel);
