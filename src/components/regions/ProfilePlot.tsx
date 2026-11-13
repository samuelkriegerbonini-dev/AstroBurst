import { memo, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTimedFlag } from "../../hooks/useTimedNotice";
import {
  niceTicks,
  linearScale,
  finiteExtent,
  fitLegendLabel,
  formatLogTickLabel,
  formatTickLabels,
  legendRows,
  tickLabels,
  LEGEND_CHAR_PX,
  LEGEND_ROW_PX,
} from "../../utils/plotScale";
import { visibleBands } from "../../utils/plotBands";
import {
  clampDomain,
  emptyPlotMessage,
  errorBarSpan,
  logDomain,
  logHiddenCount,
  logTicks,
  nearestHit,
  panDomain,
  plotPngScale,
  plotSummary,
  PLOT_PNG_BACKGROUND,
  savePngWith,
  seriesToCsv,
  seriesYExtent,
  zoomDomain,
  zoomScopeKey,
  type Domain,
} from "../../utils/plotInteraction";
import { wheelGestureZooms, type WheelZoomMode } from "../../utils/viewerZoom";

export type SeriesMode = "line" | "points" | "both";

export interface ProfileSeries {
  x: number[];
  y: (number | null)[];
  color: string;
  label: string;
  yErr?: (number | null)[];
  mode?: SeriesMode;
  dashed?: boolean;
}

export interface PlotReferenceLine {
  axis: "x" | "y";
  value: number;
  label?: string;
  color?: string;
  dashed?: boolean;
}

export interface PlotBand {
  x0: number;
  x1: number;
  color?: string;
  label?: string;
}

export type PlotLegend = "inline" | "wrap" | "none";

export interface PlotHit {
  seriesIndex: number;
  index: number;
  x: number;
  y: number;
}

export interface ProfilePlotProps {
  series: ProfileSeries[];
  xLabel: string;
  yLabel: string;
  height?: number;
  logY?: boolean;
  invertY?: boolean;
  referenceLines?: PlotReferenceLine[];
  xDomain?: [number, number] | null;
  onDomainChange?: (d: [number, number] | null) => void;
  onHover?: (hit: PlotHit | null) => void;
  onSelect?: (hit: PlotHit) => void;
  selected?: { seriesIndex: number; index: number } | null;
  toolbar?: boolean;
  csvName?: string;
  xTickFormat?: XTickFormat;
  logToggle?: boolean;
  bands?: PlotBand[];
  legend?: PlotLegend;
  wheelZoom?: WheelZoomMode;
}

export type XTickFormat = (v: number, step: number) => string;

const MARGIN = { top: 8, right: 10, bottom: 26, left: 52 };
const AXIS_COLOR = "rgba(161,161,170,0.35)";
const GRID_COLOR = "rgba(161,161,170,0.12)";
const TEXT_COLOR = "#a1a1aa";
const HALO_COLOR = "rgba(0,0,0,0.8)";
const TOOLTIP_BG = "rgba(0,0,0,0.75)";
const TOOLTIP_TEXT = "#fafafa";
const CROSSHAIR_COLOR = "rgba(255,255,255,0.3)";
const SELECTED_RING_COLOR = "#ffffff";
const ZOOM_FACTOR = 1.15;
const HIT_DISTANCE_PX = 12;
const POINT_RADIUS_PX = 2.5;
const ERROR_CAP_PX = 3;
const TOOLBAR_H = 24;
const DRAG_THRESHOLD_PX = 3;
const DASH_PATTERN = [4, 3];
const REFERENCE_DASH = [3, 3];
const X_TICK_COUNT = 6;
const Y_TICK_COUNT = 5;
const AXIS_FONT = "9px 'JetBrains Mono', monospace";
const HALO_FONT = "10px 'JetBrains Mono', monospace";
const TOOLTIP_LINE_H = 13;
const TOOLTIP_PAD = 4;
const TOOLTIP_OFFSET = 10;
const COPIED_FEEDBACK_MS = 1500;
const DEFAULT_CSV_NAME = "plot";
const DEFAULT_HEIGHT = 160;
const MIN_PLOT_PX = 10;
const DECADE_TOLERANCE = 1e-9;
const Y_TITLE_CLEARANCE_PX = 22;
const LEGEND_INSET_PX = 6;
const LEGEND_TEXT_OFFSET_PX = 11;
const LEGEND_SWATCH_DROP_PX = 3;
const LEGEND_INLINE_TEXT_Y = 7;
const LEGEND_WRAP_TEXT_Y = 1;
const BAND_LABEL_INSET_PX = 2;
const BAND_LABEL_UNDER_LEGEND_PX = 13;
const BAND_LABEL_GAP_PX = 4;
const BAND_LABEL_ROWS = 3;
const WHEEL_MODIFIER_HINT = "Ctrl+wheel zooms, drag pans, double-click resets";
const TOOLBAR_BUTTON_CLASS =
  "px-1.5 min-h-6 min-w-6 inline-flex items-center justify-center rounded text-[9px] font-mono text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/60 disabled:opacity-40 disabled:cursor-not-allowed";
const TOOLBAR_ACTIVE_CLASS = "px-1.5 min-h-6 min-w-6 inline-flex items-center justify-center rounded text-[9px] font-mono text-cyan-300 bg-zinc-800/80";

function fmtValue(v: number): string {
  const a = Math.abs(v);
  if (a === 0) return "0";
  if (a >= 1e5 || a < 1e-3) return v.toExponential(3);
  return String(Number(v.toPrecision(5)));
}

function isFiniteValue(v: number | null | undefined): v is number {
  return v !== null && v !== undefined && Number.isFinite(v);
}

function isDecade(v: number): boolean {
  const l = Math.log10(v);
  return Math.abs(l - Math.round(l)) < DECADE_TOLERANCE;
}

function drawsLine(s: ProfileSeries): boolean {
  return s.mode !== "points";
}

function drawsPoints(s: ProfileSeries): boolean {
  return s.mode === "points" || s.mode === "both";
}

function nearestIndexByX(s: ProfileSeries, x: number, logY: boolean): number {
  let best = -1;
  let bestDist = Infinity;
  for (let i = 0; i < s.x.length; i++) {
    const v = s.y[i];
    if (!isFiniteValue(v) || (logY && v <= 0)) continue;
    const d = Math.abs(s.x[i] - x);
    if (d < bestDist) {
      bestDist = d;
      best = i;
    }
  }
  return best;
}

function haloText(ctx: CanvasRenderingContext2D, text: string, x: number, y: number) {
  ctx.save();
  ctx.font = HALO_FONT;
  ctx.lineWidth = 3;
  ctx.lineJoin = "round";
  ctx.strokeStyle = HALO_COLOR;
  ctx.strokeText(text, x, y);
  ctx.fillText(text, x, y);
  ctx.restore();
}

let measureContext: CanvasRenderingContext2D | null | undefined;

function axisTextWidth(text: string): number {
  if (typeof document === "undefined") return text.length * LEGEND_CHAR_PX;
  if (measureContext === undefined) {
    measureContext = document.createElement("canvas").getContext("2d");
    if (measureContext) measureContext.font = AXIS_FONT;
  }
  return measureContext ? measureContext.measureText(text).width : text.length * LEGEND_CHAR_PX;
}

interface LegendEntry {
  x: number;
  y: number;
  text: string;
}

function legendSlots(series: ProfileSeries[], startX: number, maxX: number) {
  const rowPx = maxX - startX - LEGEND_TEXT_OFFSET_PX;
  const texts: string[] = [];
  const widths: number[] = [];
  for (const s of series) {
    const full = axisTextWidth(s.label);
    const text = full > rowPx ? fitLegendLabel(s.label, Math.floor((rowPx * s.label.length) / full)) : s.label;
    texts.push(text);
    widths.push(text === s.label ? full : axisTextWidth(text));
  }
  return legendRows(widths, startX, maxX).map((slot, i) => ({ ...slot, text: texts[i] }));
}

interface PlotLayout {
  left: number;
  top: number;
  plotW: number;
  plotH: number;
  xDomain: Domain;
  xExtent: Domain;
  xTicks: number[];
  yTicks: number[];
  yText: (string | null)[];
  legend: LegendEntry[];
  bandLabelY: number;
  logY: boolean;
  sx: (v: number) => number;
  sy: (v: number) => number;
  xOf: (px: number) => number;
  yFloorPx: number;
}

function computeLayout(
  series: ProfileSeries[],
  width: number,
  height: number,
  logY: boolean,
  invertY: boolean,
  zoom: Domain | null,
  legend: PlotLegend,
): PlotLayout | null {
  const xExt = finiteExtent(series.flatMap((s) => s.x));
  if (!xExt) return null;
  const fullTicks = niceTicks(xExt[0], xExt[1], X_TICK_COUNT);
  const xExtent: Domain = [fullTicks[0] ?? xExt[0], fullTicks[fullTicks.length - 1] ?? xExt[1]];
  const clamped = zoom !== null ? clampDomain(zoom, xExtent) : null;
  const zoomed = clamped !== null && (clamped[0] !== xExtent[0] || clamped[1] !== xExtent[1]);
  const xDomain: Domain = zoomed ? clamped : xExtent;
  const xTicks = zoomed
    ? niceTicks(xDomain[0], xDomain[1], X_TICK_COUNT).filter((t) => t >= xDomain[0] && t <= xDomain[1])
    : fullTicks;

  const yExt = seriesYExtent(series, logY, zoomed ? xDomain : null) ?? seriesYExtent(series, logY, null);
  if (!yExt) return null;
  let yTicks: number[];
  let yDomainT: Domain;
  if (logY) {
    const pos = logDomain(yExt);
    if (!pos) return null;
    const lo = Math.floor(Math.log10(pos[0]));
    const hi = Math.ceil(Math.log10(pos[1]));
    yDomainT = lo === hi ? [lo - 1, hi + 1] : [lo, hi];
    yTicks = logTicks(10 ** yDomainT[0], 10 ** yDomainT[1]);
  } else {
    const ticks = niceTicks(yExt[0], yExt[1], Y_TICK_COUNT);
    yDomainT = yExt[0] === yExt[1] ? [yExt[0] - 1, yExt[1] + 1] : [ticks[0] ?? yExt[0], ticks[ticks.length - 1] ?? yExt[1]];
    yTicks = yExt[0] === yExt[1] ? niceTicks(yDomainT[0], yDomainT[1], Y_TICK_COUNT) : ticks;
  }
  const decadesOnly = logY && yTicks.length > Y_TICK_COUNT * 2;
  const tickText = logY ? yTicks.map(formatLogTickLabel) : formatTickLabels(yTicks);
  const yText = yTicks.map((t, i) => (decadesOnly && !isDecade(t) ? null : tickText[i]));
  const widest = Math.max(0, ...yText.map((t) => (t === null ? 0 : axisTextWidth(t))));
  const left = Math.max(MARGIN.left, Math.ceil(Y_TITLE_CLEARANCE_PX + widest));
  const plotW = width - left - MARGIN.right;
  if (plotW <= MIN_PLOT_PX) return null;

  const slots = legend === "none" ? [] : legendSlots(series, left + LEGEND_INSET_PX, width);
  const rows = slots.reduce((n, slot) => Math.max(n, slot.row + 1), 0);
  const top =
    legend === "wrap" ? MARGIN.top + LEGEND_ROW_PX * rows : MARGIN.top + LEGEND_ROW_PX * Math.max(0, rows - 1);
  const plotH = height - top - MARGIN.bottom;
  if (plotH <= MIN_PLOT_PX) return null;
  const textTop = legend === "wrap" ? LEGEND_WRAP_TEXT_Y : LEGEND_INLINE_TEXT_Y;
  const legendEntries = slots.map((slot) => ({ x: slot.x, y: textTop + LEGEND_ROW_PX * slot.row, text: slot.text }));
  const bandLabelY = top + (legend === "inline" && rows > 0 ? BAND_LABEL_UNDER_LEGEND_PX : BAND_LABEL_INSET_PX);

  const sx = linearScale(xDomain, [left, left + plotW]);
  const syRaw = linearScale(yDomainT, invertY ? [top, top + plotH] : [top + plotH, top]);
  const sy = logY ? (v: number) => (v > 0 ? syRaw(Math.log10(v)) : NaN) : syRaw;
  const xOf = (px: number) => xDomain[0] + ((px - left) / plotW) * (xDomain[1] - xDomain[0]);
  return {
    left,
    top,
    plotW,
    plotH,
    xDomain,
    xExtent,
    xTicks,
    yTicks,
    yText,
    legend: legendEntries,
    bandLabelY,
    logY,
    sx,
    sy,
    xOf,
    yFloorPx: syRaw(yDomainT[0]),
  };
}

function drawBase(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  series: ProfileSeries[],
  xLabel: string,
  yLabel: string,
  layout: PlotLayout | null,
  referenceLines: PlotReferenceLine[],
  bands: PlotBand[],
  xTickFormat: XTickFormat | undefined,
  emptyMessage: string,
  background: string | null = null,
) {
  if (background === null) {
    ctx.clearRect(0, 0, width, height);
  } else {
    ctx.fillStyle = background;
    ctx.fillRect(0, 0, width, height);
  }
  ctx.font = AXIS_FONT;
  if (!layout) {
    ctx.fillStyle = TEXT_COLOR;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(emptyMessage, width / 2, height / 2);
    return;
  }
  const { left, top, plotW, plotH, xTicks, yTicks, yText, sx, sy, logY, yFloorPx } = layout;
  const right = left + plotW;
  const bottom = top + plotH;

  ctx.strokeStyle = GRID_COLOR;
  ctx.lineWidth = 1;
  for (const t of yTicks) {
    const y = Math.round(sy(t)) + 0.5;
    if (!Number.isFinite(y)) continue;
    ctx.beginPath();
    ctx.moveTo(left, y);
    ctx.lineTo(right, y);
    ctx.stroke();
  }
  ctx.strokeStyle = AXIS_COLOR;
  ctx.beginPath();
  ctx.moveTo(left + 0.5, top);
  ctx.lineTo(left + 0.5, bottom + 0.5);
  ctx.lineTo(right, bottom + 0.5);
  ctx.stroke();

  ctx.fillStyle = TEXT_COLOR;
  ctx.textAlign = "right";
  ctx.textBaseline = "middle";
  yTicks.forEach((t, i) => {
    const text = yText[i];
    if (text === null) return;
    const y = sy(t);
    if (Number.isFinite(y)) ctx.fillText(text, left - 4, y);
  });
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  const xText = tickLabels(xTicks, xTickFormat);
  xTicks.forEach((t, i) => ctx.fillText(xText[i], sx(t), bottom + 3));
  ctx.fillText(xLabel, left + plotW / 2, height - 11);
  ctx.save();
  ctx.translate(9, top + plotH / 2);
  ctx.rotate(-Math.PI / 2);
  ctx.fillText(yLabel, 0, 0);
  ctx.restore();

  ctx.save();
  ctx.beginPath();
  ctx.rect(left, top, plotW, plotH);
  ctx.clip();
  const spans = visibleBands(bands, layout.xDomain);
  for (const band of spans) {
    ctx.fillStyle = band.color;
    ctx.fillRect(sx(band.x0), top, sx(band.x1) - sx(band.x0), plotH);
  }
  for (const s of series) {
    const line = drawsLine(s);
    const points = drawsPoints(s);
    if (s.yErr) {
      ctx.strokeStyle = s.color;
      ctx.lineWidth = 1;
      ctx.setLineDash([]);
      for (let i = 0; i < s.x.length; i++) {
        const v = s.y[i];
        const e = s.yErr[i];
        if (!isFiniteValue(v) || !isFiniteValue(e)) continue;
        const span = errorBarSpan(v, e, logY);
        if (!span) continue;
        const px = sx(s.x[i]);
        const y0 = span.lower === null ? yFloorPx : sy(span.lower);
        const y1 = sy(span.upper);
        if (!Number.isFinite(px) || !Number.isFinite(y0) || !Number.isFinite(y1)) continue;
        ctx.beginPath();
        ctx.moveTo(px, y0);
        ctx.lineTo(px, y1);
        if (span.lower !== null) {
          ctx.moveTo(px - ERROR_CAP_PX, y0);
          ctx.lineTo(px + ERROR_CAP_PX, y0);
        }
        ctx.moveTo(px - ERROR_CAP_PX, y1);
        ctx.lineTo(px + ERROR_CAP_PX, y1);
        ctx.stroke();
      }
    }
    if (line) {
      ctx.strokeStyle = s.color;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = "round";
      ctx.setLineDash(s.dashed ? DASH_PATTERN : []);
      ctx.beginPath();
      let pen = false;
      for (let i = 0; i < s.x.length; i++) {
        const v = s.y[i];
        if (!isFiniteValue(v) || (logY && v <= 0)) {
          pen = false;
          continue;
        }
        const px = sx(s.x[i]);
        const py = sy(v);
        if (!Number.isFinite(px) || !Number.isFinite(py)) {
          pen = false;
          continue;
        }
        if (pen) ctx.lineTo(px, py);
        else ctx.moveTo(px, py);
        pen = true;
      }
      ctx.stroke();
      ctx.setLineDash([]);
    }
    if (points) {
      ctx.fillStyle = s.color;
      for (let i = 0; i < s.x.length; i++) {
        const v = s.y[i];
        if (!isFiniteValue(v)) continue;
        const px = sx(s.x[i]);
        const py = sy(v);
        if (!Number.isFinite(px) || !Number.isFinite(py)) continue;
        ctx.beginPath();
        ctx.arc(px, py, POINT_RADIUS_PX, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }
  const labelRows: [number, number][][] = [];
  for (const band of spans) {
    if (band.label === null) continue;
    const cx = (sx(band.x0) + sx(band.x1)) / 2;
    const half = ctx.measureText(band.label).width / 2;
    const span: [number, number] = [cx - half, cx + half];
    const clear = (row: [number, number][]) => row.every(([a, b]) => span[1] + BAND_LABEL_GAP_PX <= a || b + BAND_LABEL_GAP_PX <= span[0]);
    const found = labelRows.findIndex(clear);
    const row = found >= 0 ? found : labelRows.length < BAND_LABEL_ROWS ? labelRows.length : BAND_LABEL_ROWS - 1;
    if (row === labelRows.length) labelRows.push([]);
    labelRows[row].push(span);
    ctx.fillStyle = TEXT_COLOR;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    ctx.fillText(band.label, cx, layout.bandLabelY + row * LEGEND_ROW_PX);
  }

  for (const ref of referenceLines) {
    if (!Number.isFinite(ref.value)) continue;
    const color = ref.color ?? TEXT_COLOR;
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.setLineDash(ref.dashed === false ? [] : REFERENCE_DASH);
    ctx.beginPath();
    if (ref.axis === "x") {
      const px = Math.round(sx(ref.value)) + 0.5;
      if (!Number.isFinite(px)) continue;
      ctx.moveTo(px, top);
      ctx.lineTo(px, bottom);
      ctx.stroke();
      if (ref.label) {
        ctx.fillStyle = color;
        ctx.font = HALO_FONT;
        const fits = px + 3 + ctx.measureText(ref.label).width <= right;
        ctx.textAlign = fits ? "left" : "right";
        ctx.textBaseline = "bottom";
        haloText(ctx, ref.label, fits ? px + 3 : px - 3, bottom - 2);
      }
    } else {
      const py = Math.round(sy(ref.value)) + 0.5;
      if (!Number.isFinite(py)) continue;
      ctx.moveTo(left, py);
      ctx.lineTo(right, py);
      ctx.stroke();
      if (ref.label) {
        ctx.fillStyle = color;
        ctx.textAlign = "right";
        ctx.textBaseline = "bottom";
        haloText(ctx, ref.label, right - 3, py - 2);
      }
    }
    ctx.setLineDash([]);
  }
  ctx.restore();

  ctx.font = AXIS_FONT;
  ctx.textAlign = "left";
  ctx.textBaseline = "top";
  layout.legend.forEach((entry, i) => {
    const s = series[i];
    const swatchY = entry.y + LEGEND_SWATCH_DROP_PX;
    ctx.fillStyle = s.color;
    if (s.dashed) {
      ctx.strokeStyle = s.color;
      ctx.lineWidth = 2;
      ctx.setLineDash(DASH_PATTERN);
      ctx.beginPath();
      ctx.moveTo(entry.x, swatchY + 1);
      ctx.lineTo(entry.x + 8, swatchY + 1);
      ctx.stroke();
      ctx.setLineDash([]);
    } else {
      ctx.fillRect(entry.x, swatchY, 8, 2);
    }
    ctx.fillText(entry.text, entry.x + LEGEND_TEXT_OFFSET_PX, entry.y);
  });
}

function tooltipLines(series: ProfileSeries[], hit: PlotHit, logY: boolean, xTickFormat: XTickFormat | undefined): string[] {
  const hitSeries = series[hit.seriesIndex];
  if (!hitSeries) return [];
  const lines = [`x ${xTickFormat ? xTickFormat(hit.x, NaN) : fmtValue(hit.x)}`];
  if (hitSeries.mode === "points") {
    const e = hitSeries.yErr?.[hit.index];
    lines.push(`${hitSeries.label} ${fmtValue(hit.y)}${isFiniteValue(e) ? ` ± ${fmtValue(Math.abs(e))}` : ""}`);
    return lines;
  }
  for (const s of series) {
    if (s.mode === "points") continue;
    const i = nearestIndexByX(s, hit.x, logY);
    if (i < 0) continue;
    const v = s.y[i];
    if (!isFiniteValue(v)) continue;
    const e = s.yErr?.[i];
    lines.push(`${s.label} ${fmtValue(v)}${isFiniteValue(e) ? ` ± ${fmtValue(Math.abs(e))}` : ""}`);
  }
  return lines;
}

function drawOverlay(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  series: ProfileSeries[],
  layout: PlotLayout | null,
  hover: PlotHit | null,
  selected: { seriesIndex: number; index: number } | null,
  xTickFormat: XTickFormat | undefined,
) {
  ctx.clearRect(0, 0, width, height);
  if (!layout) return;
  const { left, top, plotW, plotH, sx, sy } = layout;
  const right = left + plotW;
  const bottom = top + plotH;

  if (selected) {
    const s = series[selected.seriesIndex];
    const v = s?.y[selected.index];
    const x = s?.x[selected.index];
    if (s && isFiniteValue(v) && isFiniteValue(x)) {
      const px = sx(x);
      const py = sy(v);
      if (Number.isFinite(px) && Number.isFinite(py) && px >= left && px <= right && py >= top && py <= bottom) {
        ctx.strokeStyle = SELECTED_RING_COLOR;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.arc(px, py, POINT_RADIUS_PX + 2.5, 0, Math.PI * 2);
        ctx.stroke();
      }
    }
  }

  if (!hover) return;
  const cx = sx(hover.x);
  const cy = sy(hover.y);
  if (!Number.isFinite(cx) || !Number.isFinite(cy)) return;

  ctx.strokeStyle = CROSSHAIR_COLOR;
  ctx.lineWidth = 0.5;
  ctx.setLineDash(REFERENCE_DASH);
  ctx.beginPath();
  ctx.moveTo(cx, top);
  ctx.lineTo(cx, bottom);
  ctx.moveTo(left, cy);
  ctx.lineTo(right, cy);
  ctx.stroke();
  ctx.setLineDash([]);

  const hitSeries = series[hover.seriesIndex];
  ctx.fillStyle = hitSeries?.color ?? TOOLTIP_TEXT;
  ctx.beginPath();
  ctx.arc(cx, cy, POINT_RADIUS_PX + 1.5, 0, Math.PI * 2);
  ctx.fill();

  const lines = tooltipLines(series, hover, layout.logY, xTickFormat);
  if (lines.length === 0) return;
  ctx.font = HALO_FONT;
  const tw = Math.max(...lines.map((l) => ctx.measureText(l).width));
  const boxW = tw + TOOLTIP_PAD * 2;
  const boxH = lines.length * TOOLTIP_LINE_H + TOOLTIP_PAD;
  let bx = cx + TOOLTIP_OFFSET;
  if (bx + boxW > width - 2) bx = cx - TOOLTIP_OFFSET - boxW;
  bx = Math.max(2, bx);
  let by = cy - boxH / 2;
  by = Math.min(Math.max(2, by), Math.max(2, height - boxH - 2));
  ctx.fillStyle = TOOLTIP_BG;
  ctx.fillRect(bx, by, boxW, boxH);
  ctx.fillStyle = TOOLTIP_TEXT;
  ctx.textAlign = "left";
  ctx.textBaseline = "top";
  lines.forEach((l, i) => ctx.fillText(l, bx + TOOLTIP_PAD, by + TOOLTIP_PAD / 2 + i * TOOLTIP_LINE_H));
}

function sizeCanvas(canvas: HTMLCanvasElement, width: number, height: number): CanvasRenderingContext2D | null {
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(width * dpr);
  canvas.height = Math.round(height * dpr);
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return ctx;
}

interface DragState {
  startPx: number;
  startDomain: Domain;
  moved: boolean;
}

function ProfilePlot(props: ProfilePlotProps) {
  const {
    series,
    xLabel,
    yLabel,
    height = DEFAULT_HEIGHT,
    invertY = false,
    referenceLines,
    xDomain: xDomainProp,
    onDomainChange,
    onHover,
    onSelect,
    selected = null,
    toolbar = false,
    csvName = DEFAULT_CSV_NAME,
    xTickFormat,
    logToggle = true,
    bands,
    legend = "inline",
    wheelZoom = "always",
  } = props;
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  const summaryId = useId();
  const [width, setWidth] = useState(0);
  const [ownDomain, setOwnDomain] = useState<Domain | null>(null);
  const zoomScope = useMemo(() => zoomScopeKey(series, xLabel), [series, xLabel]);
  const [ownDomainScope, setOwnDomainScope] = useState(zoomScope);
  if (ownDomainScope !== zoomScope) {
    setOwnDomainScope(zoomScope);
    setOwnDomain(null);
  }
  const [logOverride, setLogOverride] = useState<boolean | null>(null);
  const [hover, setHover] = useState<PlotHit | null>(null);
  const [copied, flagCopied] = useTimedFlag(COPIED_FEEDBACK_MS);
  const [saving, setSaving] = useState(false);
  const [pngNotice, setPngNotice] = useState<{ ok: boolean; text: string } | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const hoverRef = useRef<PlotHit | null>(null);

  const interactive = Boolean(onHover || onSelect || toolbar);
  const logOffered = toolbar && logToggle;
  const logY = (logOffered ? logOverride : null) ?? props.logY ?? false;
  const controlled = xDomainProp !== undefined;
  const zoom = controlled ? xDomainProp : ownDomain;

  const layout = useMemo(
    () => (width > 0 ? computeLayout(series, width, height, logY, invertY, zoom ?? null, legend) : null),
    [series, width, height, logY, invertY, zoom, legend],
  );
  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  const emptyMessage = useMemo(() => emptyPlotMessage(series, logY), [series, logY]);
  const logHidden = useMemo(() => (logY ? logHiddenCount(series) : null), [series, logY]);
  const summary = useMemo(() => plotSummary(series, xLabel, yLabel, selected), [series, xLabel, yLabel, selected]);

  const setDomain = useCallback(
    (d: Domain | null) => {
      if (!controlled) setOwnDomain(d);
      onDomainChange?.(d);
    },
    [controlled, onDomainChange],
  );
  const setDomainRef = useRef(setDomain);
  setDomainRef.current = setDomain;
  const wheelZoomRef = useRef(wheelZoom);
  wheelZoomRef.current = wheelZoom;

  useLayoutEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const measure = () => setWidth(el.clientWidth);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || width <= 0) return;
    const ctx = sizeCanvas(canvas, width, height);
    if (!ctx) return;
    drawBase(ctx, width, height, series, xLabel, yLabel, layout, referenceLines ?? [], bands ?? [], xTickFormat, emptyMessage);
  }, [series, xLabel, yLabel, width, height, layout, referenceLines, bands, xTickFormat, emptyMessage]);

  useEffect(() => {
    const canvas = overlayRef.current;
    if (!canvas || width <= 0) return;
    const ctx = sizeCanvas(canvas, width, height);
    if (!ctx) return;
    drawOverlay(ctx, width, height, series, layout, hover, selected, xTickFormat);
  }, [series, width, height, layout, hover, selected, xTickFormat]);

  useEffect(() => {
    hoverRef.current = null;
    setHover(null);
  }, [series, logY]);

  useEffect(() => {
    if (!interactive) return;
    const canvas = overlayRef.current;
    if (!canvas) return;
    const onWheel = (e: WheelEvent) => {
      if (!wheelGestureZooms(wheelZoomRef.current, e)) return;
      const l = layoutRef.current;
      if (!l || e.deltaY === 0) return;
      const rect = canvas.getBoundingClientRect();
      const px = e.clientX - rect.left;
      const anchor = l.xOf(Math.min(Math.max(px, l.left), l.left + l.plotW));
      const factor = e.deltaY > 0 ? ZOOM_FACTOR : 1 / ZOOM_FACTOR;
      const next = zoomDomain(l.xDomain, l.xExtent, anchor, factor);
      if (next[0] === l.xDomain[0] && next[1] === l.xDomain[1]) return;
      e.preventDefault();
      const isExtent = next[0] === l.xExtent[0] && next[1] === l.xExtent[1];
      setDomainRef.current(isExtent ? null : next);
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [interactive]);

  const hitAt = useCallback(
    (e: { clientX: number; clientY: number }): PlotHit | null => {
      const canvas = overlayRef.current;
      const l = layoutRef.current;
      if (!canvas || !l) return null;
      const rect = canvas.getBoundingClientRect();
      const px = e.clientX - rect.left;
      const py = e.clientY - rect.top;
      const pointsMode = series.map((s) => s.mode === "points");
      return nearestHit(series, l.sx, l.sy, px, py, HIT_DISTANCE_PX, pointsMode, l.xDomain);
    },
    [series],
  );

  const updateHover = useCallback(
    (hit: PlotHit | null) => {
      const prev = hoverRef.current;
      if (prev === hit) return;
      if (prev && hit && prev.seriesIndex === hit.seriesIndex && prev.index === hit.index) return;
      hoverRef.current = hit;
      setHover(hit);
      onHover?.(hit);
    },
    [onHover],
  );

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const drag = dragRef.current;
      const l = layoutRef.current;
      if (drag && l) {
        const rect = e.currentTarget.getBoundingClientRect();
        const dx = e.clientX - rect.left - drag.startPx;
        if (!drag.moved && Math.abs(dx) < DRAG_THRESHOLD_PX) return;
        drag.moved = true;
        const deltaValue = (-dx / l.plotW) * (drag.startDomain[1] - drag.startDomain[0]);
        const next = panDomain(drag.startDomain, l.xExtent, deltaValue);
        const isExtent = next[0] === l.xExtent[0] && next[1] === l.xExtent[1];
        setDomain(isExtent ? null : next);
        updateHover(null);
        return;
      }
      updateHover(hitAt(e));
    },
    [hitAt, updateHover, setDomain],
  );

  const handleMouseLeave = useCallback(() => {
    dragRef.current = null;
    updateHover(null);
  }, [updateHover]);

  const handleMouseDown = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const l = layoutRef.current;
    if (!l || e.button !== 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    dragRef.current = { startPx: e.clientX - rect.left, startDomain: l.xDomain, moved: false };
  }, []);

  const handleMouseUp = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const drag = dragRef.current;
      dragRef.current = null;
      if (drag && !drag.moved && onSelect) {
        const hit = hitAt(e);
        if (hit) onSelect(hit);
      }
    },
    [hitAt, onSelect],
  );

  const handleDoubleClick = useCallback(() => {
    dragRef.current = null;
    setDomain(null);
  }, [setDomain]);

  const handleCopy = useCallback(() => {
    navigator.clipboard?.writeText(seriesToCsv(series, xLabel));
    flagCopied();
  }, [series, xLabel, flagCopied]);

  const handleSavePng = useCallback(async () => {
    if (saving || width <= 0) return;
    setPngNotice(null);
    setSaving(true);
    const outcome = await savePngWith(
      async () => {
        const { save } = await import("@tauri-apps/plugin-dialog");
        return save({
          defaultPath: `${csvName}.png`,
          filters: [{ name: "PNG image", extensions: ["png"] }],
          title: "Save plot as PNG",
        });
      },
      async () => {
        const scale = plotPngScale(window.devicePixelRatio);
        const canvas = document.createElement("canvas");
        canvas.width = Math.round(width * scale);
        canvas.height = Math.round(height * scale);
        const ctx = canvas.getContext("2d");
        if (!ctx) return null;
        ctx.setTransform(scale, 0, 0, scale, 0, 0);
        drawBase(
          ctx,
          width,
          height,
          series,
          xLabel,
          yLabel,
          layout,
          referenceLines ?? [],
          bands ?? [],
          xTickFormat,
          emptyMessage,
          PLOT_PNG_BACKGROUND,
        );
        const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
        return blob ? new Uint8Array(await blob.arrayBuffer()) : null;
      },
      async (path, bytes) => {
        const { writeFile } = await import("@tauri-apps/plugin-fs");
        await writeFile(path, bytes);
      },
    );
    setSaving(false);
    if (outcome.kind === "saved") {
      setPngNotice({ ok: true, text: outcome.path });
      window.setTimeout(() => setPngNotice((n) => (n?.ok ? null : n)), COPIED_FEEDBACK_MS);
    } else if (outcome.kind === "failed") {
      setPngNotice({ ok: false, text: outcome.message });
    }
  }, [csvName, saving, width, height, series, xLabel, yLabel, layout, referenceLines, bands, xTickFormat, emptyMessage]);

  return (
    <div ref={wrapRef} className="w-full" style={{ height: height + (toolbar ? TOOLBAR_H : 0) }}>
      <div className="relative w-full" style={{ height }}>
        <canvas
          ref={canvasRef}
          role="img"
          aria-label={
            series.length > 0
              ? `Plot of ${yLabel} against ${xLabel}: ${series.map((s) => s.label).join(", ")}`
              : `Plot of ${yLabel} against ${xLabel}`
          }
          aria-describedby={summaryId}
          style={{ width: "100%", height, display: "block" }}
        />
        <canvas
          ref={overlayRef}
          title={wheelZoom === "modifier" ? WHEEL_MODIFIER_HINT : undefined}
          onMouseMove={interactive ? handleMouseMove : undefined}
          onMouseLeave={interactive ? handleMouseLeave : undefined}
          onMouseDown={interactive ? handleMouseDown : undefined}
          onMouseUp={interactive ? handleMouseUp : undefined}
          onDoubleClick={interactive ? handleDoubleClick : undefined}
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: "100%",
            height,
            display: "block",
            pointerEvents: interactive ? "auto" : "none",
            cursor: interactive ? "crosshair" : "default",
          }}
        />
        <p id={summaryId} className="sr-only" data-plot-summary>
          {summary}
        </p>
      </div>
      {toolbar && (
        <div className="flex items-center gap-1 px-1 min-w-0" style={{ height: TOOLBAR_H }}>
          {logOffered && (
            <button
              type="button"
              onClick={() => setLogOverride(!logY)}
              className={logY ? TOOLBAR_ACTIVE_CLASS : TOOLBAR_BUTTON_CLASS}
              aria-pressed={logY}
              title="Toggle a logarithmic y axis"
            >
              log y
            </button>
          )}
          <button
            type="button"
            onClick={() => setDomain(null)}
            disabled={zoom === null || zoom === undefined}
            className={TOOLBAR_BUTTON_CLASS}
            title="Reset the zoom"
          >
            reset
          </button>
          <button type="button" onClick={handleCopy} className={TOOLBAR_BUTTON_CLASS} title="Copy the plotted series as CSV">
            {copied ? "copied" : "CSV"}
          </button>
          <button
            type="button"
            onClick={() => void handleSavePng()}
            disabled={saving}
            className={TOOLBAR_BUTTON_CLASS}
            title={pngNotice?.ok ? `Saved ${pngNotice.text}` : "Save the plot as a PNG image"}
          >
            {pngNotice?.ok ? "saved" : "PNG"}
          </button>
          {pngNotice && !pngNotice.ok && (
            <span className="text-[9px] font-mono text-red-400 truncate min-w-0" title={pngNotice.text}>
              PNG not saved: {pngNotice.text}
            </span>
          )}
          {logHidden && logHidden.hidden > 0 && (
            <span className="text-[9px] font-mono text-zinc-500 truncate min-w-0">
              {`${logHidden.hidden} of ${logHidden.total} values <= 0 hidden`}
            </span>
          )}
        </div>
      )}
    </div>
  );
}

export default memo(ProfilePlot);
