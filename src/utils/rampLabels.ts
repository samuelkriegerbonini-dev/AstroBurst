import { DEFAULT_QUICK_SLOPE_PARAMS } from "../shared/types/ramp";
import type {
  CompareBin,
  FlagConfusion,
  Histogram,
  QuickSlopeParams,
  RampInfo,
  RampPixelFit,
  RampSource,
  RateComparison,
  RefCorrection,
  ShiftCheck,
} from "../shared/types/ramp";
import { parseImageRef } from "./imageRef";
import { createFramePublishGate, type FramePublishGate } from "./cubeNavigation";

export interface FramePosition {
  group: number;
  integration: number;
}

function groupCount(ngroups: number): number {
  return Number.isFinite(ngroups) && ngroups >= 1 ? Math.floor(ngroups) : 1;
}

export function frameToPosition(z: number, ngroups: number): FramePosition {
  const n = groupCount(ngroups);
  return { group: z % n, integration: Math.floor(z / n) };
}

export function positionToFrame(pos: FramePosition, ngroups: number): number {
  return pos.integration * groupCount(ngroups) + pos.group;
}

export function frameForIntegration(z: number, integration: number, ngroups: number): number {
  return positionToFrame({ group: frameToPosition(z, ngroups).group, integration }, ngroups);
}

export function formatRampFrameLabel(z: number, ramp: RampInfo): string {
  const { group, integration } = frameToPosition(z, ramp.ngroups);
  if (ramp.kind === "roman_resultants") return `resultant ${group + 1} of ${ramp.ngroups}`;
  return `group ${group + 1} of ${ramp.ngroups}, integration ${integration + 1} of ${ramp.nints}`;
}

export function rampKeyHint(ramp: RampInfo): string {
  if (ramp.kind === "roman_resultants") return "ArrowLeft / ArrowRight step the resultant, Shift steps by 10";
  return "ArrowLeft / ArrowRight step the group, Shift steps by 10; integration from the Ramp panel";
}

export function formatGroupTime(z: number, ramp: RampInfo): string | null {
  const t = ramp.group_times_s?.[frameToPosition(z, ramp.ngroups).group];
  return t !== undefined && Number.isFinite(t) ? `${t.toFixed(2)} s` : null;
}

const GROUP_TIME_UNKNOWN = "group time unknown: slope disabled";

export function tgroupSourceNote(ramp: RampInfo): string {
  if (ramp.kind === "roman_resultants") return ramp.group_times_s ? "resultant times from read_pattern" : GROUP_TIME_UNKNOWN;
  if (ramp.tgroup_source === "tgroup") return "TGROUP from header";
  if (ramp.tgroup_source === "tframe_product") return "TFRAME x (NFRAMES + GROUPGAP)";
  return GROUP_TIME_UNKNOWN;
}

export function rampIdentity(ramp: RampInfo): string {
  return [ramp.instrument, ramp.detector, ramp.readpatt, ramp.exp_type].filter((v): v is string => Boolean(v)).join(" ");
}

export function rampShape(ramp: RampInfo): string {
  if (ramp.kind === "roman_resultants") return `${ramp.ngroups} resultants`;
  return `${ramp.ngroups} x ${ramp.nints}`;
}

export function groupTimeSummary(ramp: RampInfo): string {
  const note = tgroupSourceNote(ramp);
  return ramp.tgroup_s !== null && ramp.tgroup_s > 0 ? `${note} ${ramp.tgroup_s} s` : note;
}

const REVERSED_IRS2_DETECTOR = "NRS2";

export function irs2Badge(ramp: RampInfo): string | null {
  if (!ramp.irs2) return null;
  const reversed = ramp.detector?.trim().toUpperCase() === REVERSED_IRS2_DETECTOR;
  return `IRS2 ${ramp.irs2.nrs_norm}/${ramp.irs2.nrs_ref}${reversed ? " reversed" : ""}`;
}

const TABLE_SIGNIFICANT_DIGITS = 12;

export function formatTableCell(v: number | null): string {
  if (v === null) return "null";
  if (Number.isInteger(v) || !Number.isFinite(v)) return String(v);
  return String(Number(v.toPrecision(TABLE_SIGNIFICANT_DIGITS)));
}

export interface QuickSlopeDraft {
  sat_dn: string;
  jump_k: number;
  scale_floor_dn: string;
  min_groups_ols: string;
  ref_correction: RefCorrection;
  ref_window_rows: string;
  whole_band: boolean;
}

const DEFAULT_WINDOW_TEXT = String(DEFAULT_QUICK_SLOPE_PARAMS.ref_window_rows);

export function draftFromParams(p: QuickSlopeParams): QuickSlopeDraft {
  return {
    sat_dn: String(p.sat_dn),
    jump_k: p.jump_k,
    scale_floor_dn: String(p.scale_floor_dn),
    min_groups_ols: String(p.min_groups_ols),
    ref_correction: p.ref_correction,
    ref_window_rows: p.ref_window_rows === null ? DEFAULT_WINDOW_TEXT : String(p.ref_window_rows),
    whole_band: p.ref_window_rows === null,
  };
}

function parseDraftNumber(text: string): number {
  const t = text.trim();
  return t === "" ? Number.NaN : Number(t);
}

export function paramsFromDraft(d: QuickSlopeDraft): QuickSlopeParams {
  return {
    sat_dn: parseDraftNumber(d.sat_dn),
    jump_k: d.jump_k,
    scale_floor_dn: parseDraftNumber(d.scale_floor_dn),
    min_groups_ols: parseDraftNumber(d.min_groups_ols),
    ref_correction: d.ref_correction,
    ref_window_rows: d.whole_band ? null : parseDraftNumber(d.ref_window_rows),
  };
}

export interface IntegrationOption {
  value: number;
  label: string;
}

export function integrationOptions(ramp: RampInfo): IntegrationOption[] {
  return Array.from({ length: Math.max(ramp.nints, 0) }, (_, i) => ({ value: i, label: `integration ${i + 1} of ${ramp.nints}` }));
}

const COMPRESSION_EXTENSIONS = ["fz", "gz"];
const FITS_EXTENSIONS = ["fits", "fit", "fts"];
const DEFAULT_STEM = "output";
const UNCAL_SUFFIX_RE = /_uncal$/i;

function splitExtension(name: string): [string, string] | null {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? [name.slice(0, dot), name.slice(dot + 1).toLowerCase()] : null;
}

function sourceStem(path: string): string {
  const name = parseImageRef(path).path.split(/[/\\]/).pop() ?? "";
  const outer = splitExtension(name);
  let stem = name;
  if (outer) {
    const inner = COMPRESSION_EXTENSIONS.includes(outer[1]) ? splitExtension(outer[0]) : null;
    stem = inner && FITS_EXTENSIONS.includes(inner[1]) ? inner[0] : outer[0];
  }
  return stem.length > 0 ? stem : DEFAULT_STEM;
}

export function rampSourceStem(inputName: string): string {
  return sourceStem(inputName).replace(UNCAL_SUFFIX_RE, "");
}

export function qslopeOutputName(inputName: string, nints: number, integration: number): string {
  const stem = rampSourceStem(inputName);
  if (nints <= 1) return `${stem}_qslope.fits`;
  return `${stem}_int${String(integration + 1).padStart(3, "0")}_qslope.fits`;
}

export function siblingRateName(inputName: string): string {
  return `${rampSourceStem(inputName)}_rate.fits`;
}

const QSLOPE_PRODUCT_RE = /_qslope\.fits$/i;

export function isRampProductName(name: string): boolean {
  return QSLOPE_PRODUCT_RE.test(name);
}

export type RampOpenPlan = "cube" | "asdf_ramp" | "none";

const ASDF_RE = /\.asdf$/i;

export function rampOpenPlan(path: string, naxis3: number, isRgb: boolean): RampOpenPlan {
  if (isRgb || !(naxis3 > 1)) return "none";
  return ASDF_RE.test(parseImageRef(path).path) ? "asdf_ramp" : "cube";
}

export function quickSlopeUnavailableReason(ramp: RampInfo, source: RampSource, cubeReaderRefused = false): string | null {
  if (source === "asdf") return "quick slope reads FITS uncal ramps only";
  if (cubeReaderRefused) return "the cube reader refused this file: quick slope and the per-pixel fit need it";
  if (ramp.kind === "roman_resultants") return "quick slope needs one group time: Roman resultants average uneven reads";
  if (ramp.tgroup_s === null || !(ramp.tgroup_s > 0)) return GROUP_TIME_UNKNOWN;
  return null;
}

export const MIN_REF_WINDOW_ROWS = 40;
export const MAX_SAT_DN = 65535;

export function validateQuickSlopeParams(p: QuickSlopeParams): string[] {
  const errors: string[] = [];
  if (!Number.isFinite(p.sat_dn) || p.sat_dn <= 0 || p.sat_dn > MAX_SAT_DN) {
    errors.push(`saturation must be a number in (0, ${MAX_SAT_DN}] DN`);
  }
  if (!Number.isFinite(p.jump_k) || p.jump_k < 1) errors.push("jump k must be at least 1");
  if (!Number.isFinite(p.scale_floor_dn) || p.scale_floor_dn < 0) errors.push("scale floor must be >= 0 DN");
  if (!Number.isInteger(p.min_groups_ols) || p.min_groups_ols < 2) errors.push("min groups for OLS must be a whole number >= 2");
  if (p.ref_window_rows !== null && (!Number.isInteger(p.ref_window_rows) || p.ref_window_rows < MIN_REF_WINDOW_ROWS)) {
    errors.push(`reference window must be a whole number >= ${MIN_REF_WINDOW_ROWS} rows, or the whole band`);
  }
  return errors;
}

export interface TimeAxis {
  t: number[];
  label: string;
}

export function rampTimeAxis(groupTimes: number[] | null, n: number): TimeAxis {
  if (groupTimes && groupTimes.length === n) return { t: groupTimes, label: "time (s)" };
  return { t: Array.from({ length: n }, (_, g) => g), label: "group (0-based)" };
}

export interface RampInspectorSeries {
  t: number[];
  raw: (number | null)[];
  corrected: (number | null)[];
  flagged: (number | null)[];
  excluded: (number | null)[];
}

export function rampInspectorSeries(fit: RampPixelFit): RampInspectorSeries {
  const n = fit.raw.length;
  const usable = fit.fit.n_usable;
  const flaggedGroups = new Set(fit.flagged_diffs.map((d) => d + 1));
  return {
    t: rampTimeAxis(fit.group_times_s, n).t,
    raw: [...fit.raw],
    corrected: fit.corrected.map((v, g) => (g < usable ? v : null)),
    flagged: fit.corrected.map((v, g) => (g < usable && flaggedGroups.has(g) ? v : null)),
    excluded: fit.raw.map((v, g) => (g >= usable ? v : null)),
  };
}

function mean(values: number[]): number {
  return values.reduce((a, b) => a + b, 0) / values.length;
}

export function fittedLinePoints(fit: RampPixelFit): { t: number; value: number }[] {
  const slope = fit.fit.slope;
  const n = Math.min(fit.fit.n_usable, fit.corrected.length);
  if (slope === null || !Number.isFinite(slope) || n < 2) return [];
  const t = rampTimeAxis(fit.group_times_s, fit.corrected.length).t.slice(0, n);
  const intercept = mean(fit.corrected.slice(0, n)) - slope * mean(t);
  return [t[0], t[n - 1]].map((tt) => ({ t: tt, value: intercept + slope * tt }));
}

export interface CompareVerdict {
  passed: boolean;
  reasons: string[];
}

export const ACCEPTANCE = {
  faintMaxAbsDelta: 0.03,
  brightRelLow: -0.01,
  brightRelHigh: 0.03,
  brightMinN: 50,
  zeroShiftMinCorr: 0.99,
} as const;

const FAINT_BIN_MAX_HI = 0.3;
const BRIGHT_BIN_MIN_LO = 1;
const BRIGHT_BIN_MAX_HI = 30;

function isFaintBin(bin: CompareBin): boolean {
  return bin.hi !== null && bin.hi <= FAINT_BIN_MAX_HI;
}

function isBrightBin(bin: CompareBin): boolean {
  return bin.lo !== null && bin.lo >= BRIGHT_BIN_MIN_LO && bin.hi !== null && bin.hi <= BRIGHT_BIN_MAX_HI;
}

function signed(v: number, digits: number): string {
  return `${v >= 0 ? "+" : ""}${v.toFixed(digits)}`;
}

function signedInt(v: number): string {
  return v > 0 ? `+${v}` : String(v);
}

function signedPercent(v: number, digits: number): string {
  return `${signed(v * 100, digits)} %`;
}

function fixedOrNa(v: number | null, digits: number): string {
  return v === null || !Number.isFinite(v) ? "n/a" : v.toFixed(digits);
}

function percentOrNa(v: number | null): string {
  return v === null || !Number.isFinite(v) ? "n/a" : `${(v * 100).toFixed(1)} %`;
}

function faintReasons(bins: CompareBin[], scope: string): string[] {
  return bins
    .filter((b) => isFaintBin(b) && !(Math.abs(b.median_delta) <= ACCEPTANCE.faintMaxAbsDelta))
    .map(
      (b) =>
        `G2: ${scope} bin ${formatBinRange(b)}: faint delta ${signed(b.median_delta, 4)} DN/s exceeds +/-${ACCEPTANCE.faintMaxAbsDelta}`,
    );
}

export function compareVerdict(cmp: RateComparison): CompareVerdict {
  const reasons: string[] = [];
  const s = cmp.zero_shift;
  const aligned = s.passed && s.best_dy === 0 && s.corr_at_zero !== null && s.corr_at_zero >= ACCEPTANCE.zeroShiftMinCorr;
  if (!aligned) {
    reasons.push(`G1: zero shift not aligned (best dy ${signedInt(s.best_dy)}, corr at dy 0 ${fixedOrNa(s.corr_at_zero, 4)})`);
  }

  if (cmp.amp_bins.length > 0) {
    if (!cmp.amp_bins.some((a) => a.bins.some(isFaintBin))) reasons.push("G2: no faint bin to evaluate");
    for (const amp of cmp.amp_bins) reasons.push(...faintReasons(amp.bins, `amplifier ${amp.amplifier}`));
  } else if (!cmp.bins.some(isFaintBin)) {
    reasons.push("G2: no faint bin to evaluate");
  } else {
    reasons.push(...faintReasons(cmp.bins, "full frame"));
  }

  const bright = cmp.bins.filter((b) => isBrightBin(b) && b.n >= ACCEPTANCE.brightMinN);
  if (bright.length === 0) reasons.push(`G3: no bright bin with n >= ${ACCEPTANCE.brightMinN}`);
  for (const b of bright) {
    const rel = b.median_rel;
    if (rel === null || !Number.isFinite(rel)) {
      reasons.push(`G3: bin ${formatBinRange(b)} has no delta/rate`);
    } else if (rel < ACCEPTANCE.brightRelLow || rel > ACCEPTANCE.brightRelHigh) {
      reasons.push(
        `G3: bin ${formatBinRange(b)} delta/rate ${signedPercent(rel, 2)} outside ${signedPercent(ACCEPTANCE.brightRelLow, 0)}..${signedPercent(ACCEPTANCE.brightRelHigh, 0)}`,
      );
    }
  }
  return { passed: reasons.length === 0, reasons };
}

function formatEdge(v: number | null, infinite: string): string {
  return v === null || !Number.isFinite(v) ? infinite : String(v);
}

export function formatBinRange(bin: CompareBin): string {
  return `[${formatEdge(bin.lo, "-inf")},${formatEdge(bin.hi, "inf")})`;
}

export function formatCompareBin(bin: CompareBin): string {
  const rel = bin.median_rel !== null && Number.isFinite(bin.median_rel) ? ` (${signedPercent(bin.median_rel, 1)})` : "";
  return (
    `${formatBinRange(bin)} n=${bin.n} rate ${bin.median_rate.toFixed(2)} err ${fixedOrNa(bin.median_err, 2)} ` +
    `delta ${signed(bin.median_delta, 3)}${rel} p16..p84 ${signed(bin.p16_delta, 2)}..${signed(bin.p84_delta, 2)}`
  );
}

export function formatShiftCheck(s: ShiftCheck): string {
  const next = `(next dy ${signedInt(s.second_dy)}, ${fixedOrNa(s.second_corr, 3)})`;
  if (s.passed) return `aligned: best dy ${signedInt(s.best_dy)}, corr ${fixedOrNa(s.corr_at_zero, 4)} ${next}`;
  return `NOT aligned: best dy ${signedInt(s.best_dy)}, corr ${fixedOrNa(s.corr_best, 4)}, corr at dy 0 ${fixedOrNa(s.corr_at_zero, 4)} ${next}`;
}

export function formatConfusion(c: FlagConfusion): string {
  return `tp ${c.tp}, fp ${c.fp}, fn ${c.fn_}, recall ${percentOrNa(c.recall)}, precision ${percentOrNa(c.precision)}`;
}

export interface AmpFaintRow {
  amplifier: number;
  rows: string;
  deltas: (number | null)[];
}

const FAINT_BIN_UPPER_EDGES = [0.05, 0.3];

export function ampFaintRows(cmp: RateComparison): AmpFaintRow[] {
  return cmp.amp_bins.map((amp) => ({
    amplifier: amp.amplifier,
    rows: `${amp.science_rows[0]}..${amp.science_rows[1]}`,
    deltas: FAINT_BIN_UPPER_EDGES.map((hi) => amp.bins.find((b) => b.hi === hi)?.median_delta ?? null),
  }));
}

export function quickFlaggedShare(cmp: RateComparison): string {
  if (cmp.good_pixels <= 0) return "n/a";
  return `${((cmp.quick_flagged_in_good / cmp.good_pixels) * 100).toFixed(2)} %`;
}

export function histogramSeries(h: Histogram): { x: number[]; y: number[] } {
  const n = Math.min(h.counts.length, Math.max(h.edges.length - 1, 0));
  return {
    x: Array.from({ length: n }, (_, i) => (h.edges[i] + h.edges[i + 1]) / 2),
    y: h.counts.slice(0, n),
  };
}

export interface UncalRowsInput {
  rows: [number, number] | null;
  error: string | null;
}

const WHOLE_NUMBER_RE = /^\d+$/;

export function parseUncalRows(y0: string, y1: string): UncalRowsInput {
  const a = y0.trim();
  const b = y1.trim();
  if (a === "" && b === "") return { rows: null, error: null };
  if (a === "" || b === "") return { rows: null, error: "enter both rows of the uncal window or neither" };
  if (!WHOLE_NUMBER_RE.test(a) || !WHOLE_NUMBER_RE.test(b)) return { rows: null, error: "rows must be whole numbers >= 0" };
  const lo = Number(a);
  const hi = Number(b);
  if (lo >= hi) return { rows: null, error: "y0 must be below y1" };
  return { rows: [lo, hi], error: null };
}

const IRS2_SUPPORTED_OUTPUTS = 5;

interface Irs2Rows {
  nFast: number;
  outputLen: number;
  period: number;
  half: number;
  nrsNorm: number;
  nrsRef: number;
  reversed: boolean;
  scienceRows: number;
}

function isPositiveInteger(v: number): boolean {
  return Number.isInteger(v) && v > 0;
}

function irs2Rows(ramp: RampInfo): Irs2Rows | null {
  const irs2 = ramp.irs2;
  const detector = ramp.detector?.trim().toUpperCase();
  if (!irs2 || !detector || irs2.noutputs !== IRS2_SUPPORTED_OUTPUTS) return null;
  const { nrs_norm: nrsNorm, nrs_ref: nrsRef, noutputs } = irs2;
  const nFast = ramp.frame_height;
  if (!isPositiveInteger(nrsNorm) || !isPositiveInteger(nrsRef) || nrsNorm % 2 !== 0 || !isPositiveInteger(nFast)) return null;
  if (nFast % noutputs !== 0) return null;
  const outputLen = nFast / noutputs;
  const period = nrsNorm + nrsRef;
  if (outputLen % period !== 0) return null;
  return {
    nFast,
    outputLen,
    period,
    half: nrsNorm / 2,
    nrsNorm,
    nrsRef,
    reversed: detector === REVERSED_IRS2_DETECTOR,
    scienceRows: ((noutputs - 1) * outputLen * nrsNorm) / period,
  };
}

export function irs2ScienceRowCount(ramp: RampInfo): number | null {
  return irs2Rows(ramp)?.scienceRows ?? null;
}

export function uncalRowOfScienceRow(r: number, ramp: RampInfo): number | null {
  const layout = irs2Rows(ramp);
  if (!layout || !Number.isInteger(r) || r < 0 || r >= layout.scienceRows) return null;
  const k = layout.reversed ? layout.scienceRows - 1 - r : r;
  const offset = k % layout.nrsNorm;
  const d = layout.outputLen + Math.floor(k / layout.nrsNorm) * layout.period + (offset < layout.half ? offset : offset + layout.nrsRef);
  return layout.reversed ? layout.nFast - 1 - d : d;
}

export function scienceRowOfUncalRow(y: number, ramp: RampInfo): number | null {
  const layout = irs2Rows(ramp);
  if (!layout || !Number.isInteger(y) || y < 0 || y >= layout.nFast) return null;
  const d = layout.reversed ? layout.nFast - 1 - y : y;
  if (d < layout.outputLen) return null;
  const p = (d - layout.outputLen) % layout.period;
  if (p >= layout.half && p < layout.half + layout.nrsRef) return null;
  const k = Math.floor((d - layout.outputLen) / layout.period) * layout.nrsNorm + (p < layout.half ? p : p - layout.nrsRef);
  return layout.reversed ? layout.scienceRows - 1 - k : k;
}

export type RampDisplayMapping = "frame" | "stripped" | "unmappable";

export function rampDisplayMapping(ramp: RampInfo, displayed: [number, number] | null): RampDisplayMapping {
  if (!displayed) return "frame";
  const [w, h] = displayed;
  if (w === ramp.frame_width && h === ramp.frame_height) return "frame";
  if (w === ramp.frame_width && h === irs2ScienceRowCount(ramp)) return "stripped";
  return "unmappable";
}

function unmappableText(ramp: RampInfo, displayed: [number, number]): string {
  return `the displayed image (${displayed[0]} x ${displayed[1]}) does not map onto the ${ramp.frame_width} x ${ramp.frame_height} ramp frame: inspect pixels on a ramp frame (Back to file)`;
}

export function rampDisplayNote(ramp: RampInfo, displayed: [number, number] | null): string | null {
  const mapping = rampDisplayMapping(ramp, displayed);
  if (mapping === "stripped") return "quick-slope rows are IRS2-stripped: a click inspects the matching uncal row";
  if (mapping === "unmappable" && displayed) return unmappableText(ramp, displayed);
  return null;
}

export type RampPixelTarget = { ok: true; x: number; y: number; stripped: boolean } | { ok: false; reason: string };

function outsideText(x: number, y: number, w: number, h: number): string {
  return `pixel (${x}, ${y}) is outside the ${w} x ${h} image`;
}

export function rampPixelTarget(x: number, y: number, ramp: RampInfo, displayed: [number, number] | null): RampPixelTarget {
  const mapping = rampDisplayMapping(ramp, displayed);
  if (mapping === "unmappable" && displayed) return { ok: false, reason: unmappableText(ramp, displayed) };
  const w = ramp.frame_width;
  const h = mapping === "stripped" && displayed ? displayed[1] : ramp.frame_height;
  if (!Number.isInteger(x) || !Number.isInteger(y) || x < 0 || y < 0 || x >= w || y >= h) {
    return { ok: false, reason: outsideText(x, y, w, h) };
  }
  if (mapping !== "stripped") return { ok: true, x, y, stripped: false };
  const uncalY = uncalRowOfScienceRow(y, ramp);
  return uncalY === null ? { ok: false, reason: outsideText(x, y, w, h) } : { ok: true, x, y: uncalY, stripped: true };
}

export function rampInspectLabel(coord: { x: number; y: number }, ramp: RampInfo, integration: number): string {
  const integrationText = ramp.nints > 1 ? `, integration ${integration + 1} of ${ramp.nints}` : "";
  if (irs2ScienceRowCount(ramp) === null) return `pixel (${coord.x}, ${coord.y})${integrationText}`;
  const scienceRow = scienceRowOfUncalRow(coord.y, ramp);
  const product = scienceRow === null ? "IRS2 reference row: no quick-slope pixel" : `quick-slope pixel (${coord.x}, ${scienceRow})`;
  return `uncal pixel (${coord.x}, ${coord.y}), ${product}${integrationText}`;
}

export function rampCompareKey(runId: number, qslopePath: string, ratePath: string): string {
  return `${runId}|${qslopePath}|${ratePath}`;
}

export interface FramePublishers<R> {
  frame: (record: R) => void;
  result: (record: R) => void;
}

export function framePublishers<R>(gate: FramePublishGate, publish: (record: R) => void): FramePublishers<R> {
  return {
    frame: publish,
    result: (record: R) => {
      gate.supersede();
      publish(record);
    },
  };
}

export interface CubePanelGates {
  spectrum: FramePublishGate;
  ramp: FramePublishGate;
}

export function cubePanelGates(): CubePanelGates {
  const gate = createFramePublishGate();
  return { spectrum: gate, ramp: gate };
}

export function compareVerdictTitle(cmp: RateComparison, verdict: CompareVerdict): string {
  if (cmp.science_rows === null) {
    return verdict.passed ? "Within the published tolerances (G1-G3)" : "Outside the published tolerances";
  }
  const [s0, s1] = cmp.science_rows;
  return `Science rows ${s0}..${s1} ${verdict.passed ? "within" : "outside"} the full-frame tolerances (G1-G3 are full-frame gates)`;
}

export function histogramSummary(h: Histogram, label: string): string {
  const { x, y } = histogramSeries(h);
  if (x.length === 0) return `${label} histogram: no data`;
  const peak = y.reduce((best, v, i) => (v > y[best] ? i : best), 0);
  return `${label} histogram: ${x.length} bins from ${h.edges[0]} to ${h.edges[x.length]}, peak bin centred at ${Number(x[peak].toPrecision(6))} with ${y[peak]} pixels`;
}

export interface RampFrameMarker {
  channel: number;
  text: string;
}

function isPerIntegrationSeries(n: number, ramp: RampInfo): boolean {
  return ramp.nints > 1 && n === ramp.ngroups;
}

export function rampFrameMarker(shownFrame: number | null, n: number, ramp: RampInfo | null): RampFrameMarker | null {
  if (shownFrame === null) return null;
  if (!ramp) return shownFrame < n ? { channel: shownFrame, text: `ch ${shownFrame}` } : null;
  const { group, integration } = frameToPosition(shownFrame, ramp.ngroups);
  const channel = isPerIntegrationSeries(n, ramp) ? group : shownFrame;
  if (channel >= n) return null;
  if (ramp.kind === "roman_resultants") return { channel, text: `resultant ${group + 1}` };
  const flattened = ramp.nints > 1 && !isPerIntegrationSeries(n, ramp);
  return { channel, text: flattened ? `group ${group + 1}, integration ${integration + 1}` : `group ${group + 1}` };
}

export function rampFrameFromPlotChannel(channel: number, n: number, ramp: RampInfo | null, integration: number): number {
  if (!ramp || !isPerIntegrationSeries(n, ramp)) return channel;
  return positionToFrame({ group: channel, integration }, ramp.ngroups);
}

export const QSLOPE_ACCURACY_NOTE =
  "Decision aid, not a rate: no superbias, linearity, dark, FFT refpix or weighted fit. Measured against the official rates of 8 program-1266 NIRSpec IRS2 exposures: faint additive offset within 0.011 DN/s full frame and 0.021 DN/s per amplifier; +1.2 to +2.7 % at 1-30 DN/s (AstroBurst 0.6.6 kernel, release build, 2026-10-03).";
