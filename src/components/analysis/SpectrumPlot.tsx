import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type { ContinuumWindows, LineFitSpaxel } from "../../shared/types/cube";
import type { SpectrumViewState } from "../../hooks/useSpectrumViewStore";
import type { LineMeasurement } from "../../shared/types/spectral";
import { devicePixelRatioOrOne, sizeCanvas } from "../../utils/canvasDpr";
import { COMPONENT_COLORS, lineFitOverlayPolylines } from "../../utils/lineFit";
import { LINE_PICK_TOLERANCE_PX, nearestBandMark, nearestLineMark, placeLineMarks, type BandMark, type LineMark } from "../../utils/lineList";
import { lineOverlayPolylines, type PlotPoint } from "../../utils/lineMeasure";
import { panDomain, zoomDomain } from "../../utils/plotInteraction";
import { errorBandRuns, type ErrorBandRun } from "../../utils/regionErrors";
import {
  SPECTRUM_PLOT_HEIGHT,
  SPECTRUM_PLOT_PAD,
  plottedCount,
  spectrumPlotMapping,
  spectrumPlottedMapping,
  spectrumReadout,
  spectrumYScale,
  type SpectrumPlotBins,
  type SpectrumPlotModel,
} from "../../utils/spectrumPlotModel";
import {
  axisValueToPixel,
  channelAxisValue,
  channelRangeFromDrag,
  nearestChannel,
  pixelToAxisValue,
  rangePixelSpan,
  spectrumXTicks,
  type ChannelRange,
  type PlotMapping,
} from "../../utils/spectrumRange";
import { themeColor } from "../../utils/themeColor";

export type SpectrumBrush = "range" | "left" | "right";

export interface SpectrumPlotProps {
  model: SpectrumPlotModel;
  height?: number;
  range?: ChannelRange | null;
  windows?: ContinuumWindows | null;
  activeBrush?: SpectrumBrush;
  lineOverlay?: { result: LineMeasurement; measurementAxis: number[] } | null;
  fitOverlay?: LineFitSpaxel | null;
  marks?: readonly LineMark[];
  bands?: readonly BandMark[];
  pickedMarkId?: string | null;
  frameMarker?: { channel: number; text: string } | null;
  referenceChannel?: number | null;
  onBrush?: (range: ChannelRange) => void;
  onPickMark?: (mark: LineMark) => void;
  onPickBand?: (band: BandMark) => void;
  onPlotDoubleClick?: (px: number, mapping: PlotMapping) => void;
  onPress?: () => void;
  cursorKeys?: boolean;
  status?: string | null;
  keyHint?: string | null;
  onViewChange?: (partial: Partial<SpectrumViewState>) => void;
  doubleClick?: "frame" | "reset";
}

const PAD = SPECTRUM_PLOT_PAD;
const WHEEL_ZOOM_FACTOR = 1.15;
const X_TICK_LENGTH = 3;
const X_TICK_BASELINE = 10;
const LINE_CONTINUUM_COLOR = "rgba(245,158,11,0.9)";
const LINE_MODEL_COLOR = "rgba(250,250,250,0.9)";
const LINE_CONTINUUM_DASH = [5, 3];
const LINE_MODEL_DASH = [2, 2];
const FRAME_MARKER_COLOR = "rgba(167,139,250,0.95)";
const FRAME_MARKER_HALO = "rgba(9,9,11,0.8)";
const FRAME_MARKER_LABEL_PAD = 3;
const LINE_LIST_COLOR = "rgba(34,211,238,0.6)";
const LINE_LIST_PICKED_COLOR = "rgba(34,211,238,1)";
const LINE_LIST_DASH = [2, 3];
const ERROR_BAND_FILL = "rgba(34,211,238,0.16)";
const ERROR_BAND_SCALED_STROKE = "rgba(34,211,238,0.55)";
const ERROR_BAND_SCALED_DASH = [3, 3];
const UNDER_BINS_ALPHA = 0.3;
const BIN_LINE_WIDTH = 1.5;
const BIN_DOT_RADIUS = 1.5;
const DQ_TICK_COLOR = "rgba(244,63,94,0.7)";
const DQ_TICK_LENGTH = 4;
const LINE_LIST_FONT = "9px 'JetBrains Mono', monospace";
const LINE_LIST_LABEL_H = 12;
const BRUSH_COLORS: Record<SpectrumBrush, string> = {
  range: "rgba(168,85,247,0.22)",
  left: "rgba(245,158,11,0.18)",
  right: "rgba(245,158,11,0.18)",
};
const NO_MARKS: readonly LineMark[] = [];
const NO_BANDS: readonly BandMark[] = [];
const BAND_LABEL_BASELINE = 9;
const BAND_LABEL_INSET = 3;
const BAND_LABEL_GAP = 4;
const BAND_COLORS: Record<BandMark["kind"], { fill: string; label: string }> = {
  molecular_band: { fill: "rgba(52,211,153,0.12)", label: "rgba(52,211,153,0.9)" },
  atomic_line: { fill: "rgba(250,204,21,0.14)", label: "rgba(250,204,21,0.9)" },
  instrument: { fill: "rgba(161,161,170,0.16)", label: "rgba(161,161,170,0.9)" },
};

function sameDomain(a: [number, number], b: [number, number]): boolean {
  return a[0] === b[0] && a[1] === b[1];
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

function traceBand(ctx: CanvasRenderingContext2D, run: ErrorBandRun, toPx: (x: number) => number, toY: (y: number) => number) {
  ctx.moveTo(toPx(run.x[0]), toY(run.upper[0]));
  for (let i = 1; i < run.x.length; i++) ctx.lineTo(toPx(run.x[i]), toY(run.upper[i]));
  for (let i = run.x.length - 1; i >= 0; i--) ctx.lineTo(toPx(run.x[i]), toY(run.lower[i]));
  ctx.closePath();
}

function fillBand(
  ctx: CanvasRenderingContext2D,
  runs: ErrorBandRun[],
  toPx: (x: number) => number,
  toY: (y: number) => number,
  color: string,
) {
  ctx.fillStyle = color;
  ctx.strokeStyle = color;
  ctx.lineWidth = 1;
  ctx.setLineDash([]);
  for (const run of runs) {
    ctx.beginPath();
    traceBand(ctx, run, toPx, toY);
    if (run.x.length > 1) ctx.fill();
    else ctx.stroke();
  }
}

function outlineBand(
  ctx: CanvasRenderingContext2D,
  runs: ErrorBandRun[],
  toPx: (x: number) => number,
  toY: (y: number) => number,
  color: string,
  dash: number[],
) {
  ctx.strokeStyle = color;
  ctx.lineWidth = 1;
  ctx.setLineDash(dash);
  for (const run of runs) {
    for (const edge of [run.upper, run.lower]) {
      ctx.beginPath();
      ctx.moveTo(toPx(run.x[0]), toY(edge[0]));
      for (let i = 1; i < run.x.length; i++) ctx.lineTo(toPx(run.x[i]), toY(edge[i]));
      ctx.stroke();
    }
  }
  ctx.setLineDash([]);
}

function strokeBins(
  ctx: CanvasRenderingContext2D,
  bins: SpectrumPlotBins,
  toPx: (x: number) => number,
  toY: (y: number) => number,
  model: Pick<SpectrumPlotModel, "color" | "logY">,
) {
  const drawn: PlotPoint[] = [];
  ctx.strokeStyle = model.color;
  ctx.lineWidth = BIN_LINE_WIDTH;
  ctx.beginPath();
  let started = false;
  for (let k = 0; k < bins.y.length; k++) {
    const x = bins.x[k];
    const y = bins.y[k];
    if (!Number.isFinite(y) || !Number.isFinite(x) || (model.logY && y <= 0)) {
      started = false;
      continue;
    }
    const point = { x: toPx(x), y: toY(y) };
    if (started) ctx.lineTo(point.x, point.y);
    else ctx.moveTo(point.x, point.y);
    started = true;
    drawn.push(point);
  }
  ctx.stroke();
  ctx.fillStyle = model.color;
  for (const point of drawn) {
    ctx.beginPath();
    ctx.arc(point.x, point.y, BIN_DOT_RADIUS, 0, Math.PI * 2);
    ctx.fill();
  }
}

function SpectrumPlot({
  model,
  height = SPECTRUM_PLOT_HEIGHT,
  range = null,
  windows = null,
  activeBrush = "range",
  lineOverlay = null,
  fitOverlay = null,
  marks = NO_MARKS,
  bands = NO_BANDS,
  pickedMarkId = null,
  frameMarker = null,
  referenceChannel = null,
  onBrush,
  onPickMark,
  onPickBand,
  onPlotDoubleClick,
  onPress,
  cursorKeys = false,
  status = null,
  keyHint = null,
  onViewChange,
  doubleClick = "frame",
}: SpectrumPlotProps) {
  const hintId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  const cssWidthRef = useRef(0);
  const [pixelRatio, setPixelRatio] = useState(devicePixelRatioOrOne);
  const [hoveredIdx, setHoveredIdx] = useState<number | null>(null);
  const [drag, setDrag] = useState<{ start: number; current: number } | null>(null);
  const panRef = useRef<{ startPx: number; startDomain: [number, number] } | null>(null);
  const drawnMarks = useMemo(() => marks.map((mark) => ({ ...mark, value: mark.value * model.xScale })), [marks, model.xScale]);
  const drawnBands = useMemo(
    () => bands.map((band) => ({ ...band, lo: band.lo * model.xScale, hi: band.hi * model.xScale })),
    [bands, model.xScale],
  );

  const drawPlot = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || model.n === 0) return;
    const parent = canvas.parentElement;
    if (!parent) return;
    const W = Math.floor(parent.getBoundingClientRect().width);
    const H = height;
    cssWidthRef.current = W;
    const ctx = sizeCanvas(canvas, W, H, pixelRatio);
    const overlay = overlayRef.current;
    if (overlay) sizeCanvas(overlay, W, H, pixelRatio);
    if (!ctx) return;

    ctx.fillStyle = "#0a0a0f";
    ctx.fillRect(0, 0, W, H);

    const { plotTop, plotBottom, plotHeight, toY, ticks } = spectrumYScale(model, H);
    const m = spectrumPlotMapping(model, W);

    ctx.strokeStyle = "#1f1f28";
    ctx.lineWidth = 0.5;
    for (const tick of ticks) {
      const y = toY(tick.value);
      ctx.beginPath();
      ctx.moveTo(PAD.left, y);
      ctx.lineTo(W - PAD.right, y);
      ctx.stroke();
    }

    ctx.save();
    ctx.beginPath();
    ctx.rect(PAD.left, plotTop, W - PAD.left - PAD.right, plotHeight);
    ctx.clip();

    const bins = model.bins;
    const errors = model.errors;
    const toPx = (x: number) => axisValueToPixel(x, m);
    if (bins !== null) {
      if (bins.err) fillBand(ctx, errorBandRuns(bins.x, bins.y, bins.err), toPx, toY, ERROR_BAND_FILL);
    } else if (errors?.err) {
      fillBand(ctx, errorBandRuns(model.xValues, model.series, errors.err), toPx, toY, ERROR_BAND_FILL);
      if (errors.scaled) {
        const scaledRuns = errorBandRuns(model.xValues, model.series, errors.scaled);
        outlineBand(ctx, scaledRuns, toPx, toY, ERROR_BAND_SCALED_STROKE, ERROR_BAND_SCALED_DASH);
      }
    }

    ctx.strokeStyle = model.color;
    ctx.lineWidth = 1.2;
    if (bins !== null) ctx.globalAlpha = UNDER_BINS_ALPHA;
    ctx.beginPath();
    let started = false;
    for (let i = 0; i < model.n; i++) {
      const x = model.xValues ? model.xValues[i] : i;
      const y = model.series[i];
      if (!Number.isFinite(y) || !Number.isFinite(x) || (model.logY && y <= 0)) {
        started = false;
        continue;
      }
      const cx = axisValueToPixel(x, m);
      const cy = toY(y);
      if (!started) {
        ctx.moveTo(cx, cy);
        started = true;
      } else ctx.lineTo(cx, cy);
    }
    ctx.stroke();
    if (bins !== null) {
      ctx.globalAlpha = 1;
      strokeBins(ctx, bins, toPx, toY, model);
    }

    const dqExcluded = model.dqExcluded;
    if (dqExcluded) {
      ctx.strokeStyle = DQ_TICK_COLOR;
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let i = 0; i < model.n; i++) {
        const x = model.xValues ? model.xValues[i] : i;
        if (!dqExcluded[i] || !Number.isFinite(x)) continue;
        const px = Math.round(axisValueToPixel(x, m)) + 0.5;
        ctx.moveTo(px, plotBottom);
        ctx.lineTo(px, plotBottom - DQ_TICK_LENGTH);
      }
      ctx.stroke();
    }
    ctx.restore();

    ctx.font = "9px 'JetBrains Mono', monospace";
    ctx.fillStyle = themeColor("--ab-text-3", "#94949e");
    ctx.textAlign = "right";
    for (const tick of ticks) ctx.fillText(tick.label, PAD.left - 4, toY(tick.value) + 3);
    if (model.yLabel !== null) {
      ctx.save();
      ctx.translate(9, plotTop + plotHeight / 2);
      ctx.rotate(-Math.PI / 2);
      ctx.textAlign = "center";
      ctx.fillText(model.yLabel, 0, 0);
      ctx.restore();
    }

    const xTicks = spectrumXTicks(m);
    ctx.strokeStyle = themeColor("--ab-text-4", "#8b8b94");
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (const tick of xTicks) {
      const x = Math.round(tick.px) + 0.5;
      ctx.moveTo(x, plotBottom);
      ctx.lineTo(x, plotBottom + X_TICK_LENGTH);
    }
    ctx.stroke();

    ctx.textAlign = "center";
    for (const tick of xTicks) ctx.fillText(tick.label, tick.px, plotBottom + X_TICK_BASELINE);
    ctx.fillStyle = themeColor("--ab-text-4", "#8b8b94");
    ctx.fillText(model.xLabel, W / 2, H - 4);
  }, [model, height, pixelRatio]);

  const drawOverlay = useCallback(() => {
    const overlay = overlayRef.current;
    if (!overlay) return;
    const ctx = overlay.getContext("2d");
    if (!ctx) return;
    const W = cssWidthRef.current || overlay.width / pixelRatio;
    const H = height;
    ctx.clearRect(0, 0, W, H);
    if (model.n === 0 || W === 0) return;

    const m = spectrumPlotMapping(model, W);
    const { plotTop, plotBottom, plotHeight, toY } = spectrumYScale(model, H);

    const shade = (r: ChannelRange, color: string) => {
      const span = rangePixelSpan(r, m);
      ctx.fillStyle = color;
      ctx.fillRect(span.x0, plotTop, Math.max(span.x1 - span.x0, 1), plotHeight);
    };
    if (windows) {
      shade({ z0: windows[0][0], z1: windows[0][1] }, BRUSH_COLORS.left);
      shade({ z0: windows[1][0], z1: windows[1][1] }, BRUSH_COLORS.right);
    }
    if (range) shade(range, BRUSH_COLORS.range);
    if (drag) {
      const live = channelRangeFromDrag(drag.start, drag.current, m);
      if (live) shade(live, activeBrush === "range" ? "rgba(168,85,247,0.35)" : "rgba(245,158,11,0.3)");
    }
    const strips = drawnBands
      .map((band) => {
        const p0 = axisValueToPixel(band.lo, m);
        const p1 = axisValueToPixel(band.hi, m);
        return { band, x0: Math.max(Math.min(p0, p1), PAD.left), x1: Math.min(Math.max(p0, p1), W - PAD.right) };
      })
      .sort((a, b) => a.x0 - b.x0);
    ctx.font = LINE_LIST_FONT;
    let labelEnd = -Infinity;
    for (const { band, x0, x1 } of strips) {
      const colors = BAND_COLORS[band.kind];
      ctx.fillStyle = colors.fill;
      ctx.fillRect(x0, plotTop, Math.max(x1 - x0, 1), plotHeight);
      const labelWidth = ctx.measureText(band.label).width;
      const labelX = Math.min(x0 + BAND_LABEL_INSET, W - PAD.right - labelWidth);
      if (labelX < labelEnd + BAND_LABEL_GAP) continue;
      ctx.fillStyle = colors.label;
      ctx.fillText(band.label, labelX, plotTop + BAND_LABEL_BASELINE);
      labelEnd = labelX + labelWidth;
    }
    const fileUnitScale = model.logY ? null : model.fluxState.scale;
    const frame =
      fileUnitScale === null ? null : { top: plotTop, height: plotHeight, yMin: model.yMin / fileUnitScale, yMax: model.yMax / fileUnitScale };
    if (lineOverlay && frame) {
      const curves = lineOverlayPolylines(lineOverlay.result, lineOverlay.measurementAxis, m, frame);
      strokeDashed(ctx, curves.continuum, LINE_CONTINUUM_COLOR, LINE_CONTINUUM_DASH);
      strokeDashed(ctx, curves.model, LINE_MODEL_COLOR, LINE_MODEL_DASH);
    }
    if (fitOverlay && frame) {
      const curves = lineFitOverlayPolylines(fitOverlay, m, frame);
      strokeDashed(ctx, curves.continuum, LINE_CONTINUUM_COLOR, LINE_CONTINUUM_DASH);
      strokeDashed(ctx, curves.model, LINE_MODEL_COLOR, []);
      if (fitOverlay.ncomp === 2) curves.components.forEach((c, i) => strokeDashed(ctx, c, COMPONENT_COLORS[i], LINE_MODEL_DASH));
    }
    for (const mark of placeLineMarks(drawnMarks, m)) {
      if (mark.px < PAD.left || mark.px > W - PAD.right) continue;
      const pickedMark = mark.id === pickedMarkId;
      const color = pickedMark ? LINE_LIST_PICKED_COLOR : LINE_LIST_COLOR;
      ctx.strokeStyle = color;
      ctx.lineWidth = pickedMark ? 1.5 : 1;
      ctx.setLineDash(pickedMark ? [] : LINE_LIST_DASH);
      ctx.beginPath();
      ctx.moveTo(mark.px, plotTop);
      ctx.lineTo(mark.px, plotBottom);
      ctx.stroke();
      ctx.setLineDash([]);
      if (!mark.labelShown) continue;
      ctx.fillStyle = FRAME_MARKER_HALO;
      ctx.fillRect(mark.labelX - 2, plotBottom - LINE_LIST_LABEL_H, mark.labelWidth + 4, LINE_LIST_LABEL_H);
      ctx.font = LINE_LIST_FONT;
      ctx.fillStyle = color;
      ctx.fillText(mark.label, mark.labelX, plotBottom - 3);
    }
    if (frameMarker) {
      const fx = axisValueToPixel(channelAxisValue(frameMarker.channel, m), m);
      if (Number.isFinite(fx) && fx >= PAD.left && fx <= W - PAD.right) {
        ctx.strokeStyle = FRAME_MARKER_COLOR;
        ctx.lineWidth = 1.5;
        ctx.setLineDash([]);
        ctx.beginPath();
        ctx.moveTo(fx, plotTop);
        ctx.lineTo(fx, plotBottom);
        ctx.stroke();
        ctx.font = "9px 'JetBrains Mono', monospace";
        const markerText = frameMarker.text;
        const mw = ctx.measureText(markerText).width;
        const mx = Math.min(Math.max(fx - mw / 2, PAD.left), W - PAD.right - mw);
        ctx.fillStyle = FRAME_MARKER_HALO;
        ctx.fillRect(mx - FRAME_MARKER_LABEL_PAD, plotTop, mw + FRAME_MARKER_LABEL_PAD * 2, 12);
        ctx.fillStyle = FRAME_MARKER_COLOR;
        ctx.fillText(markerText, mx, plotTop + 9);
      }
    }

    if (hoveredIdx === null || hoveredIdx < 0 || hoveredIdx >= plottedCount(model)) return;
    const label = spectrumReadout(model, hoveredIdx, referenceChannel);
    if (label === null) return;
    const plotted = spectrumPlottedMapping(model, W);
    const cx = axisValueToPixel(channelAxisValue(hoveredIdx, plotted), plotted);
    const cy = toY(model.bins === null ? model.series[hoveredIdx] : model.bins.y[hoveredIdx]);

    ctx.strokeStyle = "rgba(255,255,255,0.3)";
    ctx.lineWidth = 0.5;
    ctx.setLineDash([3, 3]);
    ctx.beginPath();
    ctx.moveTo(cx, plotTop);
    ctx.lineTo(cx, plotBottom);
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
    const tw = ctx.measureText(label).width;
    const tx = Math.min(cx + 8, W - tw - 8);
    const ty = Math.max(cy - 8, 16);
    ctx.fillStyle = "rgba(0,0,0,0.75)";
    ctx.fillRect(tx - 3, ty - 11, tw + 6, 14);
    ctx.fillStyle = "#fafafa";
    ctx.fillText(label, tx, ty);
  }, [model, height, pixelRatio, hoveredIdx, range, windows, drag, activeBrush, lineOverlay, fitOverlay, drawnBands, drawnMarks, pickedMarkId, frameMarker, referenceChannel]);

  useEffect(() => {
    drawPlot();
  }, [drawPlot]);

  useEffect(() => {
    drawOverlay();
  }, [drawOverlay]);

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const ro = new ResizeObserver(() => {
      drawPlot();
      drawOverlay();
    });
    ro.observe(root);
    return () => ro.disconnect();
  }, [drawPlot, drawOverlay]);

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const query = window.matchMedia(`(resolution: ${pixelRatio}dppx)`);
    const follow = () => setPixelRatio(devicePixelRatioOrOne());
    query.addEventListener("change", follow);
    return () => query.removeEventListener("change", follow);
  }, [pixelRatio]);

  const plotWidth = useCallback((canvas: HTMLCanvasElement) => cssWidthRef.current || canvas.getBoundingClientRect().width, []);

  const canvasX = useCallback(
    (e: { clientX: number }): number | null => {
      const canvas = canvasRef.current;
      if (!canvas) return null;
      const rect = canvas.getBoundingClientRect();
      if (rect.width <= 0) return null;
      return ((e.clientX - rect.left) / rect.width) * plotWidth(canvas);
    },
    [plotWidth],
  );

  useEffect(() => {
    const root = rootRef.current;
    if (!root || !onViewChange || model.n === 0) return;
    const onWheel = (e: WheelEvent) => {
      const canvas = canvasRef.current;
      const px = canvasX(e);
      if (!canvas || px === null || e.deltaY === 0) return;
      const width = plotWidth(canvas);
      const m = spectrumPlotMapping(model, width);
      const anchor = pixelToAxisValue(Math.min(Math.max(px, PAD.left), width - PAD.right), m);
      const current: [number, number] = [model.xMin, model.xMax];
      const next = zoomDomain(current, model.xExtent, anchor, e.deltaY > 0 ? WHEEL_ZOOM_FACTOR : 1 / WHEEL_ZOOM_FACTOR);
      if (sameDomain(next, current)) return;
      e.preventDefault();
      onViewChange({ xDomain: sameDomain(next, model.xExtent) ? null : next });
    };
    root.addEventListener("wheel", onWheel, { passive: false });
    return () => root.removeEventListener("wheel", onWheel);
  }, [model, onViewChange, canvasX, plotWidth]);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      const px = canvasX(e);
      if (!canvas || px === null || model.n === 0) return;
      const width = plotWidth(canvas);
      const plotted = spectrumPlottedMapping(model, width);
      setHoveredIdx(nearestChannel(pixelToAxisValue(px, plotted), plotted));
      const pan = panRef.current;
      if (pan && onViewChange) {
        const [d0, d1] = pan.startDomain;
        const next = panDomain(pan.startDomain, model.xExtent, (-(px - pan.startPx) / (width - PAD.left - PAD.right)) * (d1 - d0));
        onViewChange({ xDomain: sameDomain(next, model.xExtent) ? null : next });
        return;
      }
      if (drag) setDrag({ start: drag.start, current: px });
    },
    [canvasX, plotWidth, model, drag, onViewChange],
  );

  const handleMouseDown = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const px = canvasX(e);
      if (px === null || model.n === 0 || e.button !== 0) return;
      e.preventDefault();
      onPress?.();
      if (e.shiftKey && onViewChange) {
        panRef.current = { startPx: px, startDomain: [model.xMin, model.xMax] };
        return;
      }
      setDrag({ start: px, current: px });
    },
    [canvasX, model.n, model.xMin, model.xMax, onPress, onViewChange],
  );

  const handleDoubleClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      const px = canvasX(e);
      if (!canvas || px === null || model.n === 0) return;
      if (doubleClick === "reset") {
        onViewChange?.({ xDomain: null });
        return;
      }
      onPlotDoubleClick?.(px, spectrumPlotMapping(model, plotWidth(canvas)));
    },
    [canvasX, plotWidth, model, onPlotDoubleClick, doubleClick, onViewChange],
  );

  const handleMouseUp = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      if (panRef.current) {
        panRef.current = null;
        return;
      }
      const canvas = canvasRef.current;
      const px = canvasX(e);
      if (!drag || !canvas) {
        setDrag(null);
        return;
      }
      const end = px ?? drag.current;
      const m = spectrumPlotMapping(model, plotWidth(canvas));
      const selected = channelRangeFromDrag(drag.start, end, m);
      setDrag(null);
      if (Math.abs(end - drag.start) >= 2) {
        if (selected) onBrush?.(selected);
        return;
      }
      if (onPickMark) {
        const hit = nearestLineMark(drawnMarks, m, end, LINE_PICK_TOLERANCE_PX);
        if (hit) {
          onPickMark(marks[drawnMarks.indexOf(hit)]);
          return;
        }
      }
      if (!onPickBand) return;
      const bandHit = nearestBandMark(drawnBands, m, end);
      if (bandHit) onPickBand(bands[drawnBands.indexOf(bandHit)]);
    },
    [canvasX, plotWidth, drag, model, onBrush, onPickMark, onPickBand, drawnMarks, marks, drawnBands, bands],
  );

  const handleMouseLeave = useCallback(() => {
    panRef.current = null;
    setHoveredIdx(null);
    setDrag(null);
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      const count = plottedCount(model);
      if (!cursorKeys || count === 0) return;
      const last = count - 1;
      const step = e.shiftKey ? 10 : 1;
      let next: number | null;
      if (e.key === "ArrowRight") next = hoveredIdx === null ? 0 : Math.min(hoveredIdx + step, last);
      else if (e.key === "ArrowLeft") next = hoveredIdx === null ? last : Math.max(Math.min(hoveredIdx, last) - step, 0);
      else if (e.key === "Home") next = 0;
      else if (e.key === "End") next = last;
      else if (e.key === "Escape") next = null;
      else return;
      e.preventDefault();
      setHoveredIdx(next);
    },
    [cursorKeys, model, hoveredIdx],
  );

  const readout =
    hoveredIdx !== null && hoveredIdx >= 0 && hoveredIdx < plottedCount(model) ? spectrumReadout(model, hoveredIdx, referenceChannel) : null;
  const binsLabel = model.bins === null ? "" : ` in ${model.bins.y.length} ${model.bins.y.length === 1 ? "bin" : "bins"}`;

  return (
    <>
      <div
        ref={rootRef}
        role="img"
        aria-label={`Spectrum plot: ${model.yLabel ?? "flux"} against ${model.xLabel}, ${model.n} channels${binsLabel}`}
        aria-describedby={keyHint ? hintId : undefined}
        tabIndex={cursorKeys ? 0 : undefined}
        onKeyDown={handleKeyDown}
        className="relative outline-none focus-visible:ring-1 focus-visible:ring-violet-500/50"
        style={{ height }}
      >
        <canvas
          ref={canvasRef}
          height={height}
          data-error-band={model.errorBand}
          className="w-full rounded-md cursor-crosshair"
          style={{ height }}
          onMouseMove={handleMouseMove}
          onMouseDown={handleMouseDown}
          onMouseUp={handleMouseUp}
          onMouseLeave={handleMouseLeave}
          onDoubleClick={handleDoubleClick}
        />
        <canvas ref={overlayRef} height={height} className="absolute inset-0 w-full pointer-events-none" style={{ height }} />
      </div>
      {keyHint && (
        <span id={hintId} className="sr-only">
          {keyHint}
        </span>
      )}
      <p role="status" aria-live="polite" className="sr-only" data-spectrum-status="">
        {status ?? readout ?? ""}
      </p>
    </>
  );
}

export default memo(SpectrumPlot);
