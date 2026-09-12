import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { niceTicks, linearScale, finiteExtent } from "../../utils/plotScale";

export interface ProfileSeries {
  x: number[];
  y: (number | null)[];
  color: string;
  label: string;
}

interface ProfilePlotProps {
  series: ProfileSeries[];
  xLabel: string;
  yLabel: string;
  height?: number;
}

const MARGIN = { top: 8, right: 10, bottom: 26, left: 52 };
const AXIS_COLOR = "rgba(161,161,170,0.35)";
const GRID_COLOR = "rgba(161,161,170,0.12)";
const TEXT_COLOR = "#a1a1aa";

function fmtTick(v: number): string {
  const a = Math.abs(v);
  if (a === 0) return "0";
  if (a >= 1e5 || a < 1e-3) return v.toExponential(1);
  if (a >= 100) return v.toFixed(0);
  if (a >= 10) return v.toFixed(1);
  return v.toFixed(2);
}

function ProfilePlot({ series, xLabel, yLabel, height = 160 }: ProfilePlotProps) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(0);

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
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    ctx.font = "9px 'JetBrains Mono', monospace";

    const xExt = finiteExtent(series.flatMap((s) => s.x));
    const yExt = finiteExtent(series.flatMap((s) => s.y));
    const plotW = width - MARGIN.left - MARGIN.right;
    const plotH = height - MARGIN.top - MARGIN.bottom;
    if (!xExt || !yExt || plotW <= 10 || plotH <= 10) {
      ctx.fillStyle = TEXT_COLOR;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText("no data", width / 2, height / 2);
      return;
    }
    const xTicks = niceTicks(xExt[0], xExt[1], 6);
    const yTicks = niceTicks(yExt[0], yExt[1], 5);
    const xDomain: [number, number] = [xTicks[0] ?? xExt[0], xTicks[xTicks.length - 1] ?? xExt[1]];
    const yDomain: [number, number] =
      yExt[0] === yExt[1] ? [yExt[0] - 1, yExt[1] + 1] : [yTicks[0] ?? yExt[0], yTicks[yTicks.length - 1] ?? yExt[1]];
    const sx = linearScale(xDomain, [MARGIN.left, MARGIN.left + plotW]);
    const sy = linearScale(yDomain, [MARGIN.top + plotH, MARGIN.top]);

    ctx.strokeStyle = GRID_COLOR;
    ctx.lineWidth = 1;
    for (const t of yTicks) {
      const y = Math.round(sy(t)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(MARGIN.left, y);
      ctx.lineTo(MARGIN.left + plotW, y);
      ctx.stroke();
    }
    ctx.strokeStyle = AXIS_COLOR;
    ctx.beginPath();
    ctx.moveTo(MARGIN.left + 0.5, MARGIN.top);
    ctx.lineTo(MARGIN.left + 0.5, MARGIN.top + plotH + 0.5);
    ctx.lineTo(MARGIN.left + plotW, MARGIN.top + plotH + 0.5);
    ctx.stroke();

    ctx.fillStyle = TEXT_COLOR;
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (const t of yTicks) ctx.fillText(fmtTick(t), MARGIN.left - 4, sy(t));
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    for (const t of xTicks) ctx.fillText(fmtTick(t), sx(t), MARGIN.top + plotH + 3);
    ctx.fillText(xLabel, MARGIN.left + plotW / 2, height - 11);
    ctx.save();
    ctx.translate(9, MARGIN.top + plotH / 2);
    ctx.rotate(-Math.PI / 2);
    ctx.fillText(yLabel, 0, 0);
    ctx.restore();

    ctx.save();
    ctx.beginPath();
    ctx.rect(MARGIN.left, MARGIN.top, plotW, plotH);
    ctx.clip();
    for (const s of series) {
      ctx.strokeStyle = s.color;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = "round";
      ctx.beginPath();
      let pen = false;
      for (let i = 0; i < s.x.length; i++) {
        const v = s.y[i];
        if (v === null || v === undefined || !Number.isFinite(v)) {
          pen = false;
          continue;
        }
        const px = sx(s.x[i]);
        const py = sy(v);
        if (pen) ctx.lineTo(px, py);
        else ctx.moveTo(px, py);
        pen = true;
      }
      ctx.stroke();
    }
    ctx.restore();

    let lx = MARGIN.left + 6;
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    for (const s of series) {
      ctx.fillStyle = s.color;
      ctx.fillRect(lx, MARGIN.top + 2, 8, 2);
      ctx.fillText(s.label, lx + 11, MARGIN.top - 1);
      lx += 11 + ctx.measureText(s.label).width + 10;
    }
  }, [series, xLabel, yLabel, width, height]);

  return (
    <div ref={wrapRef} className="w-full" style={{ height }}>
      <canvas ref={canvasRef} style={{ width: "100%", height, display: "block" }} />
    </div>
  );
}

export default memo(ProfilePlot);
