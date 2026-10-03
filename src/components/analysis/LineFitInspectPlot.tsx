import { memo, useEffect, useId, useMemo, useRef, useState } from "react";
import type { LineFitSpaxel } from "../../shared/types/cube";
import {
  COMPONENT_COLORS,
  lineFitInspectPlot,
  lineFitInspectSummary,
  type InspectPoint,
  type LineFitInspectPlot as InspectPlotData,
} from "../../utils/lineFit";
import { linearScale } from "../../utils/plotScale";

const HEIGHT = 150;
const PAD = { top: 26, right: 10, bottom: 24, left: 46 } as const;
const MIN_PLOT_WIDTH = 20;
const BACKGROUND = "#0a0a0f";
const GRID_COLOR = "#1f1f28";
const TICK_COLOR = "#71717a";
const UNIT_COLOR = "#52525b";
const FONT = "9px 'JetBrains Mono', monospace";
const LINE_WINDOW_FILL = "rgba(168,85,247,0.16)";
const CONTINUUM_WINDOW_FILL = "rgba(245,158,11,0.08)";
const CONTINUUM_WINDOW_BAR = "rgba(245,158,11,0.75)";
const CONTINUUM_WINDOW_BAR_H = 2;
const USED_COLOR = "rgba(228,228,231,0.95)";
const USED_ERR_COLOR = "rgba(228,228,231,0.4)";
const MUTED_COLOR = "rgba(113,113,122,0.85)";
const MUTED_ERR_COLOR = "rgba(113,113,122,0.35)";
const CONTINUUM_COLOR = "rgba(161,161,170,0.9)";
const CONTINUUM_DASH = [4, 3];
const TOTAL_COLOR = "rgba(250,250,250,0.9)";
const TOTAL_WIDTH = 1.25;
const COMPONENT_DASH = [3, 2];
const MARKER_DASH = [1, 2];
const DOT_RADIUS = 1.8;
const CROSS_HALF = 2.2;
const MARKER_FIRST_BASELINE = 9;
const MARKER_ROW_H = 10;
const LABEL_MARGIN = 2;
const X_TICK_LEN = 3;
const X_LABEL_OFFSET = 12;

type Scale = (v: number) => number;

function strokeCurve(ctx: CanvasRenderingContext2D, points: InspectPoint[], sx: Scale, sy: Scale, color: string, dash: number[], width: number) {
  if (points.length < 2) return;
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.setLineDash(dash);
  ctx.beginPath();
  points.forEach((p, i) => (i === 0 ? ctx.moveTo(sx(p.x), sy(p.y)) : ctx.lineTo(sx(p.x), sy(p.y))));
  ctx.stroke();
  ctx.setLineDash([]);
}

function clampLabelX(ctx: CanvasRenderingContext2D, text: string, centre: number, width: number): number {
  const w = ctx.measureText(text).width;
  return Math.min(Math.max(centre - w / 2, LABEL_MARGIN), width - LABEL_MARGIN - w);
}

function drawInspectPlot(canvas: HTMLCanvasElement, plot: InspectPlotData, width: number) {
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(width * dpr));
  canvas.height = Math.round(HEIGHT * dpr);
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.fillStyle = BACKGROUND;
  ctx.fillRect(0, 0, width, HEIGHT);
  const left = PAD.left;
  const right = width - PAD.right;
  const top = PAD.top;
  const bottom = HEIGHT - PAD.bottom;
  if (right - left < MIN_PLOT_WIDTH) return;
  const sx = linearScale(plot.xDomain, [left, right]);
  const rawY = linearScale(plot.yDomain, [bottom, top]);
  const sy: Scale = (v) => Math.min(bottom, Math.max(top, rawY(v)));

  const band = ([a, b]: [number, number], fill: string, y: number, h: number) => {
    const x0 = sx(a);
    ctx.fillStyle = fill;
    ctx.fillRect(x0, y, Math.max(sx(b) - x0, 1), h);
  };
  for (const w of plot.continuumWindows) {
    band(w, CONTINUUM_WINDOW_FILL, top, bottom - top);
    band(w, CONTINUUM_WINDOW_BAR, bottom - CONTINUUM_WINDOW_BAR_H, CONTINUUM_WINDOW_BAR_H);
  }
  band(plot.lineWindow, LINE_WINDOW_FILL, top, bottom - top);

  ctx.strokeStyle = GRID_COLOR;
  ctx.lineWidth = 0.5;
  for (const v of [...plot.yTicks, plot.yDomain[0]]) {
    const y = sy(v);
    ctx.beginPath();
    ctx.moveTo(left, y);
    ctx.lineTo(right, y);
    ctx.stroke();
  }

  ctx.lineWidth = 1;
  for (const s of plot.samples) {
    if (s.err === null) continue;
    const x = sx(s.x);
    ctx.strokeStyle = s.state === "used" ? USED_ERR_COLOR : MUTED_ERR_COLOR;
    ctx.beginPath();
    ctx.moveTo(x, sy(s.y - s.err));
    ctx.lineTo(x, sy(s.y + s.err));
    ctx.stroke();
  }
  for (const s of plot.samples) {
    const x = sx(s.x);
    const y = sy(s.y);
    if (s.state === "dropped") {
      ctx.strokeStyle = MUTED_COLOR;
      ctx.beginPath();
      ctx.moveTo(x - CROSS_HALF, y - CROSS_HALF);
      ctx.lineTo(x + CROSS_HALF, y + CROSS_HALF);
      ctx.moveTo(x - CROSS_HALF, y + CROSS_HALF);
      ctx.lineTo(x + CROSS_HALF, y - CROSS_HALF);
      ctx.stroke();
      continue;
    }
    ctx.fillStyle = s.state === "used" ? USED_COLOR : MUTED_COLOR;
    ctx.beginPath();
    ctx.arc(x, y, DOT_RADIUS, 0, Math.PI * 2);
    ctx.fill();
  }

  strokeCurve(ctx, plot.continuum, sx, sy, CONTINUUM_COLOR, CONTINUUM_DASH, 1);
  plot.components.forEach((curve, i) => strokeCurve(ctx, curve, sx, sy, COMPONENT_COLORS[i] ?? TOTAL_COLOR, COMPONENT_DASH, 1));
  strokeCurve(ctx, plot.total, sx, sy, TOTAL_COLOR, [], TOTAL_WIDTH);

  ctx.font = FONT;
  plot.markers.forEach((m, i) => {
    const color = m.component === null ? TOTAL_COLOR : (COMPONENT_COLORS[m.component] ?? TOTAL_COLOR);
    const x = sx(m.x);
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.setLineDash(MARKER_DASH);
    ctx.beginPath();
    ctx.moveTo(x, top);
    ctx.lineTo(x, bottom);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = color;
    ctx.textAlign = "left";
    ctx.fillText(m.label, clampLabelX(ctx, m.label, x, width), MARKER_FIRST_BASELINE + i * MARKER_ROW_H);
  });

  ctx.fillStyle = TICK_COLOR;
  ctx.strokeStyle = TICK_COLOR;
  ctx.textAlign = "left";
  plot.xTicks.forEach((t, i) => {
    const x = sx(t);
    ctx.beginPath();
    ctx.moveTo(x, bottom);
    ctx.lineTo(x, bottom + X_TICK_LEN);
    ctx.stroke();
    const label = plot.xTickLabels[i] ?? "";
    ctx.fillText(label, clampLabelX(ctx, label, x, width), bottom + X_LABEL_OFFSET);
  });
  ctx.textAlign = "right";
  plot.yTicks.forEach((v, i) => ctx.fillText(plot.yTickLabels[i], left - 4, sy(v) + 3));
  if (plot.unit) {
    ctx.fillStyle = UNIT_COLOR;
    ctx.fillText(plot.unit, right, HEIGHT - 3);
  }
}

function LineFitInspectPlot({ spaxel }: { spaxel: LineFitSpaxel }) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const summaryId = useId();
  const [width, setWidth] = useState(0);
  const plot = useMemo(() => lineFitInspectPlot(spaxel), [spaxel]);
  const summary = useMemo(() => lineFitInspectSummary(spaxel, plot), [spaxel, plot]);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const measure = () => setWidth(Math.floor(el.getBoundingClientRect().width));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || width <= 0) return;
    drawInspectPlot(canvas, plot, width);
  }, [plot, width]);

  return (
    <div ref={wrapRef} className="w-full">
      <canvas
        ref={canvasRef}
        data-linefit-inspect-plot
        data-points={plot.samples.length}
        data-components={plot.markers.length}
        role="img"
        aria-label="Fitted spaxel spectrum"
        aria-describedby={summaryId}
        className="block w-full rounded"
        style={{ height: HEIGHT }}
      />
      <p id={summaryId} data-linefit-inspect-plot-summary className="sr-only">
        {summary}
      </p>
    </div>
  );
}

export default memo(LineFitInspectPlot);
