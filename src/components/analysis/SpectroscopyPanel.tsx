import { useState, useRef, useCallback, useEffect, useId, useMemo, memo } from "react";
import { Activity, BarChart3, CircleDot, Crosshair, Layers } from "lucide-react";
import CubeFrameNav from "../CubeFrameNav";
import LineMeasurementSection from "./LineMeasurementSection";
import SpectralAxisControls from "./SpectralAxisControls";
import {
  collapseCubeRange,
  computeMomentMaps,
  getCubeSpectrumRegion,
} from "../../services/cube";
import { getSpectralAxis, measureSpectralLine } from "../../services/spectral";
import { getOutputDir } from "../../infrastructure/tauri";
import { useRenderContext } from "../../context/PreviewContext";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useRegionKey } from "../../hooks/useRegionKey";
import {
  beginRegionSpectrum,
  clearRegionSpectrum,
  commitRegionSpectrum,
  failRegionSpectrum,
  useSpectrum,
} from "../../hooks/useSpectrumStore";
import type {
  CollapseRangeMode,
  ContinuumWindows,
  CubeDims,
  MomentKind,
  MomentMapsResult,
} from "../../shared/types/cube";
import { COLLAPSE_RANGE_MODES, MOMENT_KINDS } from "../../shared/types/cube";
import type { Region, RegionShape } from "../../shared/types/regions";
import type {
  CorrectionFrame,
  LineMeasurement,
  LineModel,
  RadialVelocityCorrectionResult,
  SpectralAxisInfo,
  SpectralAxisMode,
  SpectrumSource,
  VelocityConvention,
} from "../../shared/types/spectral";
import {
  lineOverlayPolylines,
  measurementAxisValues,
  momentVelocityFrameNote,
  runForSource,
  spectrumSourceKey,
  type PlotPoint,
} from "../../utils/lineMeasure";
import { applyCorrectionKms, formatAxis, formatAxisValue } from "../../utils/spectralAxis";
import {
  FRAME_KEY_HINT,
  channelFromPlotPixel,
  channelRequestNeeded,
  clampFrame,
  createFramePublishGate,
  displayedChannel,
  formatFrameDelta,
  formatFrameLabel,
  frameAxisValue,
  frameFromKey,
} from "../../utils/cubeNavigation";
import { formatAxisTick } from "../../utils/plotScale";
import {
  axisValueToPixel,
  channelAxisValue,
  channelRangeFromDrag,
  defaultContinuumWindows,
  formatRangeLabel,
  fullCubeCollapse,
  nearestChannel,
  parseChannelInput,
  pixelToAxisValue,
  rangePixelSpan,
  windowsAreValid,
  type ChannelRange,
  type FullCollapseMode,
  type PlotMapping,
} from "../../utils/spectrumRange";

export interface CubeResult {
  label: string;
  previewUrl: string;
  fitsPath: string | null;
  dimensions: [number, number] | null;
  frameIndex?: number;
}

interface SpectroscopyPanelProps {
  spectrum?: number[];
  wavelengths?: number[] | null;
  pixelCoord?: { x: number; y: number } | null;
  isLoading?: boolean;
  cubeDims?: CubeDims | null;
  elapsed?: number;
  error?: string | null;
  filePath?: string;
  onFramePreview?: (previewUrl: string, frameIndex: number, fitsPath?: string) => void;
  onCubeResult?: (result: CubeResult) => void;
}

type BrushTarget = "range" | "left" | "right";
type RegionView = "sum" | "mean" | "jy";

interface AxisX {
  values: number[] | null;
  label: string;
  unit: string;
}

interface LineRun {
  key: string;
  measurementAxis: number[];
  result: LineMeasurement;
}

function strokeDashed(ctx: CanvasRenderingContext2D, points: PlotPoint[], color: string, dash: number[]) {
  if (points.length < 2) return;
  ctx.strokeStyle = color;
  ctx.lineWidth = 1;
  ctx.setLineDash(dash);
  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) ctx.lineTo(points[i].x, points[i].y);
  ctx.stroke();
  ctx.setLineDash([]);
}

function arrayMinMax(arr: number[]): [number, number] {
  let min = Infinity;
  let max = -Infinity;
  for (let i = 0; i < arr.length; i++) {
    const v = arr[i];
    if (Number.isFinite(v)) {
      if (v < min) min = v;
      if (v > max) max = v;
    }
  }
  return [min === Infinity ? 0 : min, max === -Infinity ? 1 : max];
}

function legacyConversion(rawUnit: string | null | undefined): { factor: number; label: string } {
  const unit = rawUnit ? rawUnit.trim().toUpperCase() : null;
  if (!unit) return { factor: 1, label: "Channel" };
  if (unit === "M") return { factor: 1e6, label: "μm" };
  if (unit === "CM") return { factor: 1e4, label: "μm" };
  if (unit === "MM") return { factor: 1e3, label: "μm" };
  if (unit === "UM") return { factor: 1, label: "μm" };
  if (unit === "NM") return { factor: 1, label: "nm" };
  if (unit === "ANGSTROM" || unit === "A") return { factor: 0.1, label: "nm" };
  if (unit === "HZ") return { factor: 1e-9, label: "GHz" };
  if (unit === "KHZ") return { factor: 1e-6, label: "GHz" };
  if (unit === "MHZ") return { factor: 1e-3, label: "GHz" };
  if (unit === "GHZ") return { factor: 1, label: "GHz" };
  return { factor: 1, label: unit.toLowerCase() };
}

function hasArea(shape: RegionShape): boolean {
  return shape.shape === "circle" || shape.shape === "box" || shape.shape === "ellipse" || shape.shape === "polygon";
}

function pickRegion(regions: Region[], selectedId: string | null): { target: Region | null; background: Region | null } {
  const selected = regions.find((r) => r.id === selectedId) ?? null;
  const target = selected && hasArea(selected.shape) ? selected : (regions.find((r) => hasArea(r.shape)) ?? null);
  if (!target) return { target: null, background: null };
  const linked = target.backgroundId ? (regions.find((r) => r.id === target.backgroundId) ?? null) : null;
  const background = linked && linked.shape.shape === "annulus" ? linked : null;
  return { target, background };
}

const CANVAS_H = 180;
const PAD = { top: 10, bottom: 24, left: 50, right: 12 } as const;
const N_GRID_Y = 4;
const DEFAULT_SNR = 3;
const LABEL_MAPPING_WIDTH = 400;
const MEASURED_SERIES_VIEW: RegionView = "sum";
const LINE_CONTINUUM_COLOR = "rgba(245,158,11,0.9)";
const LINE_MODEL_COLOR = "rgba(250,250,250,0.9)";
const LINE_CONTINUUM_DASH = [5, 3];
const LINE_MODEL_DASH = [2, 2];
const FRAME_MARKER_COLOR = "rgba(167,139,250,0.95)";
const FRAME_MARKER_HALO = "rgba(9,9,11,0.8)";
const FRAME_MARKER_LABEL_PAD = 3;
const BRUSH_COLORS: Record<BrushTarget, string> = {
  range: "rgba(168,85,247,0.22)",
  left: "rgba(245,158,11,0.18)",
  right: "rgba(245,158,11,0.18)",
};
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] font-mono text-zinc-200 focus:border-violet-500/50 w-14 disabled:opacity-40";
const SELECT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 focus:border-violet-500/50 disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";

function SpectroscopyPanel({
  spectrum = [],
  wavelengths = null,
  pixelCoord = null,
  isLoading = false,
  cubeDims = null,
  elapsed = 0,
  error = null,
  filePath,
  onFramePreview,
  onCubeResult,
}: SpectroscopyPanelProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [hoveredIdx, setHoveredIdx] = useState<number | null>(null);
  const [collapseLoading, setCollapseLoading] = useState(false);
  const [collapseResult, setCollapseResult] = useState<{ elapsed_ms: number } | null>(null);
  const [collapseError, setCollapseError] = useState<string | null>(null);
  const [collapseMode, setCollapseMode] = useState<FullCollapseMode>("mean");
  const regionSeqRef = useRef(0);
  const collapseSeqRef = useRef(0);
  const rangeSeqRef = useRef(0);
  const momentSeqRef = useRef(0);
  const lineSeqRef = useRef(0);
  const previousFileRef = useRef<string | undefined>(filePath);

  const { region, regionSource, regionLoading, regionError } = useSpectrum();
  const { processed } = useRenderContext();
  const shownFrame = displayedChannel(processed, filePath);
  const regionKey = useRegionKey();
  const regionDoc = useRegionDoc(regionKey);
  const picked = useMemo(() => pickRegion(regionDoc.regions, regionDoc.selectedId), [regionDoc]);
  const [regionView, setRegionView] = useState<RegionView>("sum");

  const headerAxis = cubeDims?.spectral_axis ?? null;
  const [headerFallbackAxis, setHeaderFallbackAxis] = useState<SpectralAxisInfo | null>(null);
  const [axisError, setAxisError] = useState<string | null>(null);
  const axis = headerAxis ?? headerFallbackAxis;
  const [mode, setMode] = useState<SpectralAxisMode>("wavelength_vac");
  const [restUm, setRestUm] = useState<number | null>(null);
  const [convention, setConvention] = useState<VelocityConvention>("optical");
  const [correction, setCorrection] = useState<CorrectionFrame>("none");
  const [correctionResult, setCorrectionResult] = useState<RadialVelocityCorrectionResult | null>(null);

  const brushTargetId = useId();
  const snrId = useId();
  const [brushTarget, setBrushTarget] = useState<BrushTarget>("range");
  const [range, setRange] = useState<ChannelRange | null>(null);
  const [windows, setWindows] = useState<ContinuumWindows | null>(null);
  const [drag, setDrag] = useState<{ start: number; current: number } | null>(null);
  const [rangeMode, setRangeMode] = useState<CollapseRangeMode>("sum");
  const [rangeLoading, setRangeLoading] = useState(false);
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [snrText, setSnrText] = useState(String(DEFAULT_SNR));
  const [maskBelow, setMaskBelow] = useState(true);
  const [momentLoading, setMomentLoading] = useState(false);
  const [momentError, setMomentError] = useState<string | null>(null);
  const [momentResult, setMomentResult] = useState<MomentMapsResult | null>(null);
  const [momentKind, setMomentKind] = useState<MomentKind>("m0");
  const [lineModel, setLineModel] = useState<LineModel>("gaussian");
  const [lineResult, setLineResult] = useState<LineRun | null>(null);
  const [lineLoading, setLineLoading] = useState(false);
  const [lineError, setLineError] = useState<string | null>(null);
  const [frame, setFrame] = useState(() => shownFrame ?? 0);
  const [frameFile, setFrameFile] = useState(filePath);
  const [seenRecord, setSeenRecord] = useState(processed);
  const [requestSeq, setRequestSeq] = useState(0);
  const [framePublishGate] = useState(createFramePublishGate);
  const [loopPlayback, setLoopPlayback] = useState(false);
  if (frameFile !== filePath) {
    setFrameFile(filePath);
    setFrame(shownFrame ?? 0);
    setSeenRecord(processed);
  } else if (seenRecord !== processed) {
    setSeenRecord(processed);
    if (processed === null) setFrame(0);
  }

  useEffect(() => {
    if (processed === null) framePublishGate.supersede();
  }, [processed, framePublishGate]);

  useEffect(() => {
    regionSeqRef.current++;
    collapseSeqRef.current++;
    rangeSeqRef.current++;
    momentSeqRef.current++;
    lineSeqRef.current++;
    setRange(null);
    setWindows(null);
    setMomentResult(null);
    setRangeError(null);
    setMomentError(null);
    setCollapseResult(null);
    setCollapseError(null);
    setCollapseLoading(false);
    setRangeLoading(false);
    setMomentLoading(false);
    setLineResult(null);
    setLineError(null);
    setLineLoading(false);
    if (previousFileRef.current !== filePath) {
      previousFileRef.current = filePath;
      setRegionView("sum");
      clearRegionSpectrum();
    }
  }, [filePath]);

  useEffect(() => {
    setHeaderFallbackAxis(null);
    setAxisError(null);
    if (!filePath || headerAxis) return;
    let cancelled = false;
    getSpectralAxis(filePath)
      .then((info) => {
        if (!cancelled) setHeaderFallbackAxis(info);
      })
      .catch((e: unknown) => {
        if (!cancelled) setAxisError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [filePath, headerAxis]);

  const series = useMemo<number[]>(() => {
    if (region) {
      if (regionView === "jy" && region.flux_jy) return region.flux_jy;
      return regionView === "mean" ? region.mean : region.sum;
    }
    return spectrum;
  }, [region, regionView, spectrum]);
  const n = series.length;
  const totalFrames = cubeDims ? (cubeDims.frames ?? 0) : 0;
  const channelCount = totalFrames > 0 ? totalFrames : n;
  const frameLinked = totalFrames > 1;

  const formatted = useMemo(() => formatAxis(axis, mode, restUm, convention), [axis, mode, restUm, convention]);

  const axisX = useMemo<AxisX>(() => {
    if (formatted && n > 0 && formatted.values.length === n) {
      const values =
        mode === "velocity" ? applyCorrectionKms(formatted.values, correctionResult, correction) : formatted.values;
      const label =
        mode === "velocity" && correction !== "none" && correctionResult ? `${formatted.label}, ${correction}` : formatted.label;
      return { values, label, unit: formatted.unit };
    }
    if (wavelengths && n > 0 && wavelengths.length === n) {
      const conv = legacyConversion(cubeDims?.spectral_classification?.axis_unit);
      return { values: wavelengths.map((w) => w * conv.factor), label: `Wavelength (${conv.label})`, unit: conv.label };
    }
    return { values: null, label: "Channel", unit: "ch" };
  }, [formatted, n, mode, correction, correctionResult, wavelengths, cubeDims]);

  const frameLabel = useMemo(
    () => (frameLinked ? formatFrameLabel(frame, totalFrames, frameAxisValue(frame, axisX.values), axisX.unit) : null),
    [frameLinked, frame, totalFrames, axisX],
  );

  const shownPngOnly = processed !== null && processed.fitsPath === null;
  const requestFrame = useCallback(
    (idx: number) => {
      const target = clampFrame(idx, totalFrames);
      if (!channelRequestNeeded(target, frame, shownFrame, shownPngOnly)) return;
      setFrame(target);
      setRequestSeq((seq) => seq + 1);
    },
    [totalFrames, frame, shownFrame, shownPngOnly],
  );

  const publishCubeResult = useCallback(
    (result: CubeResult) => {
      framePublishGate.supersede();
      onCubeResult?.(result);
    },
    [framePublishGate, onCubeResult],
  );

  const plotParams = useMemo(() => {
    if (n === 0) return { xMin: 0, xMax: 1, yMin: 0, yMax: 1 };
    const [xMin, xMax] = axisX.values ? arrayMinMax(axisX.values) : [0, n - 1];
    const [rawYMin, rawYMax] = arrayMinMax(series);
    const span = rawYMax - rawYMin;
    const yPad = span > 0 ? span * 0.05 : Math.max(Math.abs(rawYMax), 1) * 0.05;
    return { xMin, xMax, yMin: rawYMin - yPad, yMax: rawYMax + yPad };
  }, [series, axisX, n]);

  const currentSource = useMemo<SpectrumSource | null>(() => {
    if (region) return regionSource;
    return pixelCoord ? { kind: "pixel", x: pixelCoord.x, y: pixelCoord.y } : null;
  }, [region, regionSource, pixelCoord]);
  const lineOverlayVisible = runForSource(lineResult, currentSource, region ? regionView : MEASURED_SERIES_VIEW) !== null;
  const visibleLineResult = runForSource(lineResult, currentSource, MEASURED_SERIES_VIEW)?.result ?? null;
  const lineShiftKms = useMemo(
    () => (correction !== "none" && correctionResult ? applyCorrectionKms([0], correctionResult, correction)[0] : null),
    [correction, correctionResult],
  );

  const mappingFor = useCallback(
    (width: number): PlotMapping => ({
      width,
      padLeft: PAD.left,
      padRight: PAD.right,
      xMin: plotParams.xMin,
      xMax: plotParams.xMax,
      xValues: axisX.values,
      n,
    }),
    [plotParams, axisX, n],
  );

  const drawPlot = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || n === 0) return;
    const parent = canvas.parentElement;
    if (!parent) return;
    const W = Math.floor(parent.getBoundingClientRect().width);
    const H = CANVAS_H;
    canvas.width = W;
    canvas.height = H;
    const overlay = overlayRef.current;
    if (overlay) {
      overlay.width = W;
      overlay.height = H;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.fillStyle = "#0a0a0f";
    ctx.fillRect(0, 0, W, H);

    const plotH = H - PAD.top - PAD.bottom;
    const { yMin, yMax } = plotParams;
    const yRange = Math.max(yMax - yMin, 1e-10);
    const m = mappingFor(W);
    const toY = (yi: number) => PAD.top + plotH - ((yi - yMin) / yRange) * plotH;

    ctx.strokeStyle = "#1f1f28";
    ctx.lineWidth = 0.5;
    for (let i = 0; i <= N_GRID_Y; i++) {
      const y = PAD.top + (i / N_GRID_Y) * plotH;
      ctx.beginPath();
      ctx.moveTo(PAD.left, y);
      ctx.lineTo(W - PAD.right, y);
      ctx.stroke();
    }

    ctx.strokeStyle = region ? "#22d3ee" : "#a855f7";
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    let started = false;
    for (let i = 0; i < n; i++) {
      const x = axisX.values ? axisX.values[i] : i;
      const y = series[i];
      if (!Number.isFinite(y) || !Number.isFinite(x)) continue;
      const cx = axisValueToPixel(x, m);
      const cy = toY(y);
      if (!started) {
        ctx.moveTo(cx, cy);
        started = true;
      } else ctx.lineTo(cx, cy);
    }
    ctx.stroke();

    ctx.font = "9px 'JetBrains Mono', monospace";
    ctx.fillStyle = "#71717a";
    ctx.textAlign = "right";
    for (let i = 0; i <= N_GRID_Y; i++) {
      const val = yMax - (i / N_GRID_Y) * yRange;
      const y = PAD.top + (i / N_GRID_Y) * plotH;
      ctx.fillText(formatAxisTick(val, yRange), PAD.left - 4, y + 3);
    }

    ctx.textAlign = "center";
    ctx.fillStyle = "#52525b";
    ctx.fillText(axisX.label, W / 2, H - 4);
  }, [series, n, axisX, plotParams, mappingFor, region]);

  const drawOverlay = useCallback(() => {
    const overlay = overlayRef.current;
    if (!overlay) return;
    const ctx = overlay.getContext("2d");
    if (!ctx) return;
    const W = overlay.width;
    const H = overlay.height;
    ctx.clearRect(0, 0, W, H);
    if (n === 0 || W === 0) return;

    const m = mappingFor(W);
    const plotTop = PAD.top;
    const plotBottom = H - PAD.bottom;

    const shade = (r: ChannelRange, color: string) => {
      const span = rangePixelSpan(r, m);
      ctx.fillStyle = color;
      ctx.fillRect(span.x0, plotTop, Math.max(span.x1 - span.x0, 1), plotBottom - plotTop);
    };
    if (windows) {
      shade({ z0: windows[0][0], z1: windows[0][1] }, BRUSH_COLORS.left);
      shade({ z0: windows[1][0], z1: windows[1][1] }, BRUSH_COLORS.right);
    }
    if (range) shade(range, BRUSH_COLORS.range);
    if (drag) {
      const live = channelRangeFromDrag(drag.start, drag.current, m);
      if (live) shade(live, brushTarget === "range" ? "rgba(168,85,247,0.35)" : "rgba(245,158,11,0.3)");
    }
    if (lineResult && lineOverlayVisible) {
      const frame = { top: plotTop, height: plotBottom - plotTop, yMin: plotParams.yMin, yMax: plotParams.yMax };
      const curves = lineOverlayPolylines(lineResult.result, lineResult.measurementAxis, m, frame);
      strokeDashed(ctx, curves.continuum, LINE_CONTINUUM_COLOR, LINE_CONTINUUM_DASH);
      strokeDashed(ctx, curves.model, LINE_MODEL_COLOR, LINE_MODEL_DASH);
    }
    if (frameLinked && shownFrame !== null && shownFrame < n) {
      const fx = axisValueToPixel(channelAxisValue(shownFrame, m), m);
      if (Number.isFinite(fx) && fx >= PAD.left && fx <= W - PAD.right) {
        ctx.strokeStyle = FRAME_MARKER_COLOR;
        ctx.lineWidth = 1.5;
        ctx.setLineDash([]);
        ctx.beginPath();
        ctx.moveTo(fx, plotTop);
        ctx.lineTo(fx, plotBottom);
        ctx.stroke();
        ctx.font = "9px 'JetBrains Mono', monospace";
        const markerText = `ch ${shownFrame}`;
        const mw = ctx.measureText(markerText).width;
        const mx = Math.min(Math.max(fx - mw / 2, PAD.left), W - PAD.right - mw);
        ctx.fillStyle = FRAME_MARKER_HALO;
        ctx.fillRect(mx - FRAME_MARKER_LABEL_PAD, plotTop, mw + FRAME_MARKER_LABEL_PAD * 2, 12);
        ctx.fillStyle = FRAME_MARKER_COLOR;
        ctx.fillText(markerText, mx, plotTop + 9);
      }
    }

    if (hoveredIdx === null || hoveredIdx < 0 || hoveredIdx >= n) return;
    const plotH = H - PAD.top - PAD.bottom;
    const { yMin, yMax } = plotParams;
    const yRange = Math.max(yMax - yMin, 1e-10);
    const x = axisX.values ? axisX.values[hoveredIdx] : hoveredIdx;
    const y = series[hoveredIdx];
    if (!Number.isFinite(y) || !Number.isFinite(x)) return;
    const cx = axisValueToPixel(x, m);
    const cy = PAD.top + plotH - ((y - yMin) / yRange) * plotH;

    ctx.strokeStyle = "rgba(255,255,255,0.3)";
    ctx.lineWidth = 0.5;
    ctx.setLineDash([3, 3]);
    ctx.beginPath();
    ctx.moveTo(cx, PAD.top);
    ctx.lineTo(cx, H - PAD.bottom);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(PAD.left, cy);
    ctx.lineTo(W - PAD.right, cy);
    ctx.stroke();
    ctx.setLineDash([]);

    ctx.fillStyle = "#eab308";
    ctx.beginPath();
    ctx.arc(cx, cy, 4, 0, Math.PI * 2);
    ctx.fill();

    ctx.font = "10px 'JetBrains Mono', monospace";
    const delta = frameLinked && shownFrame !== null ? `, ${formatFrameDelta(hoveredIdx, shownFrame)}` : "";
    const label = axisX.values
      ? `ch ${hoveredIdx}${delta} · ${formatAxisValue(x, axisX.unit)} ${axisX.unit} → ${y.toFixed(3)}`
      : `ch ${hoveredIdx}${delta} → ${y.toFixed(3)}`;
    const tw = ctx.measureText(label).width;
    const tx = Math.min(cx + 8, W - tw - 8);
    const ty = Math.max(cy - 8, 16);
    ctx.fillStyle = "rgba(0,0,0,0.75)";
    ctx.fillRect(tx - 3, ty - 11, tw + 6, 14);
    ctx.fillStyle = "#fafafa";
    ctx.fillText(label, tx, ty);
  }, [series, n, axisX, plotParams, mappingFor, hoveredIdx, range, windows, drag, brushTarget, lineResult, lineOverlayVisible, frameLinked, shownFrame]);

  useEffect(() => {
    drawPlot();
  }, [drawPlot]);

  useEffect(() => {
    drawOverlay();
  }, [drawOverlay]);

  useEffect(() => {
    const c = containerRef.current;
    if (!c) return;
    const ro = new ResizeObserver(() => {
      drawPlot();
      drawOverlay();
    });
    ro.observe(c);
    return () => ro.disconnect();
  }, [drawPlot, drawOverlay]);

  const canvasX = useCallback((e: React.MouseEvent<HTMLCanvasElement>): number | null => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0) return null;
    return ((e.clientX - rect.left) / rect.width) * canvas.width;
  }, []);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      const px = canvasX(e);
      if (!canvas || px === null || n === 0) return;
      const m = mappingFor(canvas.width);
      setHoveredIdx(nearestChannel(pixelToAxisValue(px, m), m));
      if (drag) setDrag({ start: drag.start, current: px });
    },
    [canvasX, n, mappingFor, drag],
  );

  const handleMouseDown = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const px = canvasX(e);
      if (px === null || n === 0 || e.button !== 0) return;
      e.preventDefault();
      containerRef.current?.focus({ preventScroll: true });
      setDrag({ start: px, current: px });
    },
    [canvasX, n],
  );

  const handleDoubleClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      const px = canvasX(e);
      if (!canvas || px === null || n === 0 || !frameLinked) return;
      const channel = channelFromPlotPixel(px, mappingFor(canvas.width), totalFrames);
      if (channel !== null) requestFrame(channel);
    },
    [canvasX, n, frameLinked, mappingFor, totalFrames, requestFrame],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (!frameLinked) return;
      const next = frameFromKey(e.key, e.shiftKey, frame, totalFrames);
      if (next === null) return;
      e.preventDefault();
      requestFrame(next);
    },
    [frameLinked, frame, totalFrames, requestFrame],
  );

  const commitBrush = useCallback(
    (selected: ChannelRange) => {
      if (brushTarget === "range") {
        setRange(selected);
        return;
      }
      const window: [number, number] = [selected.z0, selected.z1];
      setWindows((current) => {
        const other: [number, number] = current ? (brushTarget === "left" ? current[1] : current[0]) : window;
        return brushTarget === "left" ? [window, other] : [other, window];
      });
    },
    [brushTarget],
  );

  const handleMouseUp = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      const px = canvasX(e);
      if (!drag || !canvas) {
        setDrag(null);
        return;
      }
      const end = px ?? drag.current;
      const selected = channelRangeFromDrag(drag.start, end, mappingFor(canvas.width));
      setDrag(null);
      if (selected && Math.abs(end - drag.start) >= 2) commitBrush(selected);
    },
    [canvasX, drag, mappingFor, commitBrush],
  );

  const handleMouseLeave = useCallback(() => {
    setHoveredIdx(null);
    setDrag(null);
  }, []);

  const handleCollapse = useCallback(
    async (mode: FullCollapseMode) => {
      const request = fullCubeCollapse(totalFrames, mode);
      if (!filePath || !request) return;
      const seq = ++collapseSeqRef.current;
      setCollapseLoading(true);
      setCollapseMode(mode);
      setCollapseResult(null);
      setCollapseError(null);
      try {
        const dir = await getOutputDir();
        const result = await collapseCubeRange(filePath, dir, request.z0, request.z1, request.mode);
        if (collapseSeqRef.current !== seq) return;
        setCollapseResult(result);
        if (result.previewUrl) {
          publishCubeResult({
            label: mode === "median" ? "Collapse median" : "Collapse mean",
            previewUrl: result.previewUrl,
            fitsPath: result.fits_path || null,
            dimensions: result.dimensions ?? null,
          });
        }
      } catch (e) {
        if (collapseSeqRef.current === seq) setCollapseError(e instanceof Error ? e.message : String(e));
      } finally {
        setCollapseLoading(false);
      }
    },
    [filePath, totalFrames, publishCubeResult],
  );

  const handleRegionSpectrum = useCallback(async () => {
    if (!filePath || !picked.target) return;
    const seq = ++regionSeqRef.current;
    const source: SpectrumSource = { kind: "region", shape: picked.target.shape, background: picked.background?.shape ?? null };
    beginRegionSpectrum();
    try {
      const result = await getCubeSpectrumRegion(filePath, source.shape, source.background);
      if (regionSeqRef.current !== seq) return;
      commitRegionSpectrum(result, source);
      setRegionView((view) => (view === "jy" && !result.flux_jy ? "sum" : view));
    } catch (e) {
      if (regionSeqRef.current === seq) failRegionSpectrum(e instanceof Error ? e.message : String(e));
    }
  }, [filePath, picked]);

  const handleCollapseRange = useCallback(async () => {
    if (!filePath || !range) return;
    const seq = ++rangeSeqRef.current;
    setRangeLoading(true);
    setRangeError(null);
    try {
      const dir = await getOutputDir();
      const result = await collapseCubeRange(filePath, dir, range.z0, range.z1, rangeMode);
      if (rangeSeqRef.current !== seq) return;
      if (result.previewUrl) {
        publishCubeResult({
          label: `Collapse ${result.mode} · ch ${result.z0}-${result.z1}`,
          previewUrl: result.previewUrl,
          fitsPath: result.fits_path || null,
          dimensions: result.dimensions ?? null,
        });
      }
    } catch (e) {
      if (rangeSeqRef.current === seq) setRangeError(e instanceof Error ? e.message : String(e));
    } finally {
      setRangeLoading(false);
    }
  }, [filePath, range, rangeMode, publishCubeResult]);

  const showMoment = useCallback(
    (result: MomentMapsResult, kind: MomentKind) => {
      setMomentKind(kind);
      const map = result[kind];
      if (map.previewUrl) {
        publishCubeResult({
          label: `Moment ${kind} · ${result.n_channels} ch`,
          previewUrl: map.previewUrl,
          fitsPath: map.fits_path || null,
          dimensions: result.dimensions ?? null,
        });
      }
    },
    [publishCubeResult],
  );

  const handleMoments = useCallback(async () => {
    if (!filePath || !range) return;
    const snr = Number(snrText);
    if (!Number.isFinite(snr) || snr < 0) {
      setMomentError("SNR threshold must be a non-negative number");
      return;
    }
    if (windows && !windowsAreValid(windows, channelCount)) {
      setMomentError(`continuum windows must be channel ranges inside 0–${Math.max(channelCount - 1, 0)}`);
      return;
    }
    const seq = ++momentSeqRef.current;
    setMomentLoading(true);
    setMomentError(null);
    try {
      const dir = await getOutputDir();
      const result = await computeMomentMaps(filePath, dir, {
        z0: range.z0,
        z1: range.z1,
        rest_um: restUm,
        convention,
        continuum: windows,
        snr_threshold: snr,
        mask_below_threshold: maskBelow,
      });
      if (momentSeqRef.current !== seq) return;
      setMomentResult(result);
      showMoment(result, "m0");
    } catch (e) {
      if (momentSeqRef.current === seq) setMomentError(e instanceof Error ? e.message : String(e));
    } finally {
      if (momentSeqRef.current === seq) setMomentLoading(false);
    }
  }, [filePath, range, snrText, restUm, convention, windows, channelCount, maskBelow, showMoment]);

  const handleMeasureLine = useCallback(async () => {
    if (!filePath || !range || !currentSource) return;
    if (windows && !windowsAreValid(windows, channelCount)) {
      setLineError(`continuum windows must be channel ranges inside 0–${Math.max(channelCount - 1, 0)}`);
      return;
    }
    const measurementAxis = measurementAxisValues(axis);
    if (!measurementAxis) {
      setLineError("no spectral axis for this cube: the line measurement needs a wavelength, frequency or velocity axis");
      return;
    }
    const seq = ++lineSeqRef.current;
    setLineLoading(true);
    setLineError(null);
    try {
      const result = await measureSpectralLine(filePath, currentSource, {
        z0: range.z0,
        z1: range.z1,
        continuum: windows,
        restUm,
        convention,
        model: lineModel,
        velocityShiftKms: lineShiftKms,
      });
      if (lineSeqRef.current !== seq) return;
      setLineResult({ key: spectrumSourceKey(currentSource, MEASURED_SERIES_VIEW), measurementAxis, result });
    } catch (e) {
      if (lineSeqRef.current === seq) setLineError(e instanceof Error ? e.message : String(e));
    } finally {
      if (lineSeqRef.current === seq) setLineLoading(false);
    }
  }, [filePath, range, currentSource, windows, channelCount, axis, lineShiftKms, restUm, convention, lineModel]);

  const updateRangeEdge = useCallback(
    (edge: "z0" | "z1", text: string) => {
      const value = parseChannelInput(text, channelCount);
      if (value === null) return;
      setRange((current) => {
        const base = current ?? { z0: 0, z1: Math.max(channelCount - 1, 0) };
        const next = { ...base, [edge]: value };
        return { z0: Math.min(next.z0, next.z1), z1: Math.max(next.z0, next.z1) };
      });
    },
    [channelCount],
  );

  const updateWindowEdge = useCallback(
    (side: 0 | 1, edge: 0 | 1, text: string) => {
      const value = parseChannelInput(text, channelCount);
      if (value === null) return;
      setWindows((current) => {
        const seeded = current ?? (range ? defaultContinuumWindows(range, channelCount) : null);
        const base: ContinuumWindows = seeded ?? [
          [value, value],
          [value, value],
        ];
        const next: ContinuumWindows = [[...base[0]], [...base[1]]] as ContinuumWindows;
        next[side][edge] = value;
        if (edge === 0 && next[side][1] < value) next[side][1] = value;
        if (edge === 1 && next[side][0] > value) next[side][0] = value;
        return next;
      });
    },
    [channelCount, range],
  );

  const labelMapping = useMemo(() => mappingFor(LABEL_MAPPING_WIDTH), [mappingFor]);
  const rangeLabel = range ? formatRangeLabel(range, labelMapping, axisX.unit) : "drag on the plot to select channels";
  const regionAvailable = !!filePath && !!picked.target && totalFrames > 1;
  const velocityReady = mode === "velocity" || axis?.kind === "vrad" || axis?.kind === "vopt" || axis?.kind === "velo";
  const momentsReady = !!range && (restUm !== null || (axis?.rest_wavelength_um ?? null) !== null || velocityReady);
  const lineHint = !range
    ? "brush a line range first"
    : !axis
      ? "no spectral axis for this cube"
      : !currentSource
        ? "click the preview or pick a region first"
        : null;
  const lineDisabled = lineLoading || lineHint !== null || !filePath;

  const regionButton = regionAvailable && (
    <button
      onClick={handleRegionSpectrum}
      disabled={regionLoading}
      className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all disabled:opacity-40"
      style={{
        background: "rgba(34,211,238,0.08)",
        border: "1px solid rgba(34,211,238,0.2)",
        color: "#22d3ee",
      }}
      title={
        picked.background
          ? `${picked.target?.shape.shape} region with annulus background`
          : `${picked.target?.shape.shape} region (link an annulus as background for sky subtraction)`
      }
    >
      <CircleDot size={10} />
      {regionLoading ? "Extracting…" : "Region spectrum"}
    </button>
  );

  if (n === 0 && !isLoading && !regionLoading) {
    return (
      <div className="ab-panel p-6 flex flex-col items-center gap-3">
        <div
          className="w-10 h-10 rounded-xl flex items-center justify-center"
          style={{ background: "rgba(168,85,247,0.06)", border: "1px solid rgba(168,85,247,0.1)" }}
        >
          <Activity size={18} style={{ color: "var(--ab-violet)", opacity: 0.5 }} />
        </div>
        <p className="text-[11px] text-zinc-500">Click on the preview image to extract a spectrum</p>
        {regionButton}
        {error && (
          <p className="text-[10px] text-red-400/80 text-center px-2 py-1 rounded bg-red-900/15 border border-red-800/20">
            Spectrum extraction failed: {error}
          </p>
        )}
        {regionError && (
          <p className="text-[10px] text-red-400/80 text-center px-2 py-1 rounded bg-red-900/15 border border-red-800/20">
            Region spectrum failed: {regionError}
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="ab-panel overflow-hidden animate-fade-in">
      <div className="ab-panel-header">
        <div className="flex items-center gap-1.5">
          <Activity size={12} style={{ color: "var(--ab-violet)" }} />
          <span className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">Spectroscopy</span>
        </div>
        <div className="flex items-center gap-2 text-[10px] font-mono text-zinc-500">
          {(isLoading || regionLoading) && (
            <div
              className="w-3 h-3 rounded-full animate-spin"
              style={{ border: "1.5px solid transparent", borderTopColor: "var(--ab-violet)" }}
            />
          )}
          {region ? (
            <span className="flex items-center gap-1" style={{ color: "#22d3ee" }}>
              <CircleDot size={10} />
              {regionSource?.kind === "region" ? regionSource.shape.shape : "region"} · {region.npix.toFixed(1)} px
              {region.bg_subtracted ? " · sky-subtracted" : ""}
            </span>
          ) : (
            pixelCoord && (
              <span className="flex items-center gap-1">
                <Crosshair size={10} />({pixelCoord.x}, {pixelCoord.y})
              </span>
            )
          )}
          {elapsed > 0 && <span>{elapsed}ms</span>}
        </div>
      </div>

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

      {!axis && axisError && (
        <p className="mx-3 mb-2 text-[10px] text-amber-300/80 px-2 py-1 rounded bg-amber-900/15 border border-amber-800/20">
          No spectral axis for this file: {axisError}
        </p>
      )}

      <div
        ref={containerRef}
        tabIndex={0}
        onKeyDown={handleKeyDown}
        title={frameLinked ? FRAME_KEY_HINT : undefined}
        className="p-2 rounded-md outline-none focus-visible:ring-1 focus-visible:ring-violet-500/50"
      >
        {isLoading ? (
          <div
            className="flex items-center justify-center rounded-md"
            style={{ height: CANVAS_H, background: "rgba(9,9,11,0.8)" }}
          >
            <div
              className="w-5 h-5 rounded-full animate-spin"
              style={{ border: "2px solid transparent", borderTopColor: "var(--ab-violet)" }}
            />
          </div>
        ) : (
          <div className="relative" style={{ height: CANVAS_H }}>
            <canvas
              ref={canvasRef}
              height={CANVAS_H}
              className="w-full rounded-md cursor-crosshair"
              style={{ height: CANVAS_H }}
              onMouseMove={handleMouseMove}
              onMouseDown={handleMouseDown}
              onMouseUp={handleMouseUp}
              onMouseLeave={handleMouseLeave}
              onDoubleClick={handleDoubleClick}
            />
            <canvas
              ref={overlayRef}
              height={CANVAS_H}
              className="absolute inset-0 w-full pointer-events-none"
              style={{ height: CANVAS_H }}
            />
          </div>
        )}
      </div>

      {region && (
        <div className="px-3 pb-2 flex items-center gap-1.5 text-[10px] font-mono">
          <span className="text-zinc-500">Series</span>
          {(["sum", "mean", ...(region.flux_jy ? ["jy"] : [])] as RegionView[]).map((view) => (
            <button
              key={view}
              onClick={() => setRegionView(view)}
              className="px-2 py-0.5 rounded border transition-colors"
              style={{
                borderColor: regionView === view ? "#22d3ee" : "var(--ab-border)",
                color: regionView === view ? "#22d3ee" : "#71717a",
              }}
            >
              {view === "jy" ? "Jy" : view}
            </button>
          ))}
          {region.bg_subtracted && <span className="text-zinc-600">bg {region.n_bg} px</span>}
          <button
            onClick={() => clearRegionSpectrum()}
            className="ml-auto px-2 py-0.5 rounded border transition-colors text-zinc-500"
            style={{ borderColor: "var(--ab-border)" }}
          >
            pixel
          </button>
        </div>
      )}
      {regionError && (
        <p className="mx-3 mb-2 text-[10px] text-red-400/80 px-2 py-1 rounded bg-red-900/15 border border-red-800/20">
          Region spectrum failed: {regionError}
        </p>
      )}

      {cubeDims && (
        <div className="px-3 pb-1 text-[10px] font-mono text-zinc-600">
          Cube: {cubeDims.width} × {cubeDims.height} × {cubeDims.frames}
          {n > 0 ? ` — ${n} channels` : ""}
        </div>
      )}

      {filePath && cubeDims && totalFrames > 1 && (
        <div className="px-3 pb-2 flex items-center gap-2 flex-wrap" style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}>
          <CollapseBtn
            label="Collapse Mean"
            loading={collapseLoading && collapseMode === "mean"}
            disabled={collapseLoading}
            color="var(--ab-violet)"
            onClick={() => handleCollapse("mean")}
          />
          <CollapseBtn
            label="Collapse Median"
            loading={collapseLoading && collapseMode === "median"}
            disabled={collapseLoading}
            color="var(--ab-amber)"
            onClick={() => handleCollapse("median")}
          />
          {regionButton}
          {collapseResult && !collapseLoading && (
            <span className="text-[10px] font-mono text-zinc-500 ml-auto">
              {collapseResult.elapsed_ms}ms
            </span>
          )}
          {collapseError && (
            <p className="w-full text-[10px] text-red-400/80 px-2 py-1 rounded bg-red-900/15 border border-red-800/20">
              Cube collapse failed: {collapseError}
            </p>
          )}
        </div>
      )}

      {filePath && cubeDims && totalFrames > 1 && (
        <div className="px-3 pb-2 flex flex-col gap-2" style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}>
          <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
            <label htmlFor={brushTargetId} className={LABEL_CLASS}>
              Brush
            </label>
            <select id={brushTargetId} value={brushTarget} onChange={(e) => setBrushTarget(e.target.value as BrushTarget)} className={SELECT_CLASS}>
              <option value="range">line range</option>
              <option value="left">continuum A</option>
              <option value="right">continuum B</option>
            </select>
            <span className="text-zinc-500">{rangeLabel}</span>
          </div>

          <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
            <span className={LABEL_CLASS}>Range</span>
            <input
              type="number"
              min={0}
              max={Math.max(channelCount - 1, 0)}
              value={range ? range.z0 : ""}
              aria-label="Range first channel z0"
              placeholder="z0"
              onChange={(e) => updateRangeEdge("z0", e.target.value)}
              className={INPUT_CLASS}
            />
            <input
              type="number"
              min={0}
              max={Math.max(channelCount - 1, 0)}
              value={range ? range.z1 : ""}
              aria-label="Range last channel z1"
              placeholder="z1"
              onChange={(e) => updateRangeEdge("z1", e.target.value)}
              className={INPUT_CLASS}
            />
            <select
              value={rangeMode}
              aria-label="Range collapse mode"
              onChange={(e) => setRangeMode(e.target.value as CollapseRangeMode)}
              className={SELECT_CLASS}
            >
              {COLLAPSE_RANGE_MODES.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
            <CollapseBtn
              label="Collapse range"
              loading={rangeLoading}
              disabled={rangeLoading || !range}
              color="var(--ab-violet)"
              onClick={handleCollapseRange}
            />
            {range && (
              <button onClick={() => setRange(null)} className="text-zinc-500 hover:text-zinc-300">
                clear
              </button>
            )}
          </div>
          {rangeError && (
            <p className="text-[10px] text-red-400/80 px-2 py-1 rounded bg-red-900/15 border border-red-800/20">{rangeError}</p>
          )}

          <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
            <span className={LABEL_CLASS}>Continuum</span>
            {([0, 1] as const).map((side) => (
              <span key={side} className="flex items-center gap-1">
                <span className="text-zinc-600">{side === 0 ? "A" : "B"}</span>
                <input
                  type="number"
                  min={0}
                  max={Math.max(channelCount - 1, 0)}
                  value={windows ? windows[side][0] : ""}
                  aria-label={`Continuum ${side === 0 ? "A" : "B"} first channel`}
                  placeholder="from"
                  onChange={(e) => updateWindowEdge(side, 0, e.target.value)}
                  className={INPUT_CLASS}
                />
                <input
                  type="number"
                  min={0}
                  max={Math.max(channelCount - 1, 0)}
                  value={windows ? windows[side][1] : ""}
                  aria-label={`Continuum ${side === 0 ? "A" : "B"} last channel`}
                  placeholder="to"
                  onChange={(e) => updateWindowEdge(side, 1, e.target.value)}
                  className={INPUT_CLASS}
                />
              </span>
            ))}
            <button
              onClick={() => range && setWindows(defaultContinuumWindows(range, channelCount))}
              disabled={!range}
              className="text-zinc-500 hover:text-zinc-300 disabled:opacity-40"
            >
              auto
            </button>
            {windows && (
              <button onClick={() => setWindows(null)} className="text-zinc-500 hover:text-zinc-300">
                none
              </button>
            )}
          </div>

          <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
            <label htmlFor={snrId} className={LABEL_CLASS}>
              SNR
            </label>
            <input
              id={snrId}
              type="number"
              min={0}
              step={0.5}
              value={snrText}
              onChange={(e) => setSnrText(e.target.value)}
              className={INPUT_CLASS}
            />
            <label className="flex items-center gap-1 text-zinc-500 cursor-pointer">
              <input type="checkbox" checked={maskBelow} onChange={(e) => setMaskBelow(e.target.checked)} />
              mask M0
            </label>
            <CollapseBtn
              label="Moments"
              icon={<BarChart3 size={10} />}
              loading={momentLoading}
              disabled={momentLoading || !momentsReady}
              color="var(--ab-amber)"
              onClick={handleMoments}
            />
            {!momentsReady && range && (
              <span className="text-zinc-600">set a rest wavelength for velocity moments</span>
            )}
            {momentResult && (
              <span className="flex items-center gap-1 ml-auto">
                {MOMENT_KINDS.map((kind) => (
                  <button
                    key={kind}
                    onClick={() => showMoment(momentResult, kind)}
                    className="px-2 py-0.5 rounded border transition-colors uppercase"
                    style={{
                      borderColor: momentKind === kind ? "var(--ab-amber)" : "var(--ab-border)",
                      color: momentKind === kind ? "var(--ab-amber)" : "#71717a",
                    }}
                  >
                    {kind}
                  </button>
                ))}
              </span>
            )}
          </div>
          {momentError && (
            <p className="text-[10px] text-red-400/80 px-2 py-1 rounded bg-red-900/15 border border-red-800/20">{momentError}</p>
          )}
          {momentResult && (
            <div className="text-[10px] font-mono text-zinc-500">
              <span>
                M0 in {momentResult.m0_unit}, M1/M2 in {momentResult.velocity_unit}, {momentResult.n_channels} channels
                {momentResult.noise_per_channel !== null ? `, noise ${momentResult.noise_per_channel.toExponential(2)}` : ""}
                {`, ${momentVelocityFrameNote(axis?.specsys ?? null, correction, lineShiftKms)}`}
              </span>
              {momentResult.notes.length > 0 && (
                <details className="mt-0.5 text-zinc-600">
                  <summary className="cursor-pointer select-none">notes ({momentResult.notes.length})</summary>
                  <ul className="mt-0.5 flex flex-col gap-0.5">
                    {momentResult.notes.map((note, i) => (
                      <li key={i} className="break-words">
                        {note}
                      </li>
                    ))}
                  </ul>
                </details>
              )}
            </div>
          )}
        </div>
      )}

      {filePath && cubeDims && totalFrames > 1 && (
        <LineMeasurementSection
          result={visibleLineResult}
          loading={lineLoading}
          error={lineError}
          model={lineModel}
          onModelChange={setLineModel}
          onMeasure={handleMeasureLine}
          disabled={lineDisabled}
          disabledHint={lineHint}
        />
      )}

      {filePath && cubeDims && totalFrames > 1 && (
        <div className="px-2 pb-2">
          <CubeFrameNav
            filePath={filePath}
            totalFrames={totalFrames}
            frame={frame}
            requestSeq={requestSeq}
            onFrameRequest={requestFrame}
            frameLabel={frameLabel}
            loop={loopPlayback}
            onLoopChange={setLoopPlayback}
            onFrameChange={onFramePreview}
            publishGate={framePublishGate}
          />
        </div>
      )}
    </div>
  );
}

function CollapseBtn({
  label,
  loading,
  disabled,
  color,
  icon,
  onClick,
}: {
  label: string;
  loading: boolean;
  disabled: boolean;
  color: string;
  icon?: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all disabled:opacity-40"
      style={{
        background: `color-mix(in srgb, ${color} 8%, transparent)`,
        border: `1px solid color-mix(in srgb, ${color} 20%, transparent)`,
        color,
      }}
    >
      {loading ? (
        <div className="w-2.5 h-2.5 rounded-full animate-spin" style={{ border: "1.5px solid transparent", borderTopColor: color }} />
      ) : (
        (icon ?? <Layers size={10} />)
      )}
      {label}
    </button>
  );
}

export default memo(SpectroscopyPanel);
