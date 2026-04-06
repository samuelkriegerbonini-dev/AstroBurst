import { useRef, useState, useCallback, useEffect, memo } from "react";

export interface CurvePoint {
  x: number;
  y: number;
}

interface CurveEditorProps {
  points: CurvePoint[];
  onChange: (points: CurvePoint[]) => void;
  color?: string;
  label?: string;
  width?: number;
  height?: number;
}

const PAD = 8;
const POINT_R = 5;
const GRID_LINES = 4;

function clamp(v: number, lo = 0, hi = 1) {
  return Math.max(lo, Math.min(hi, v));
}

function catmullRomSpline(pts: CurvePoint[], steps = 64): string {
  if (pts.length < 2) return "";
  const sorted = [...pts].sort((a, b) => a.x - b.x);
  const n = sorted.length;

  const extended = [
    { x: sorted[0].x - (sorted[1].x - sorted[0].x), y: sorted[0].y - (sorted[1].y - sorted[0].y) },
    ...sorted,
    { x: sorted[n - 1].x + (sorted[n - 1].x - sorted[n - 2].x), y: sorted[n - 1].y + (sorted[n - 1].y - sorted[n - 2].y) },
  ];

  const path: string[] = [];
  for (let seg = 1; seg < extended.length - 2; seg++) {
    const p0 = extended[seg - 1];
    const p1 = extended[seg];
    const p2 = extended[seg + 1];
    const p3 = extended[seg + 2];

    for (let s = 0; s <= steps; s++) {
      const t = s / steps;
      const t2 = t * t;
      const t3 = t2 * t;

      const x =
        0.5 * (2 * p1.x + (-p0.x + p2.x) * t + (2 * p0.x - 5 * p1.x + 4 * p2.x - p3.x) * t2 + (-p0.x + 3 * p1.x - 3 * p2.x + p3.x) * t3);
      const y =
        0.5 * (2 * p1.y + (-p0.y + p2.y) * t + (2 * p0.y - 5 * p1.y + 4 * p2.y - p3.y) * t2 + (-p0.y + 3 * p1.y - 3 * p2.y + p3.y) * t3);

      const cx = clamp(x);
      const cy = clamp(y);
      path.push(seg === 1 && s === 0 ? `M ${cx} ${cy}` : `L ${cx} ${cy}`);
    }
  }
  return path.join(" ");
}

function CurveEditorInner({ points, onChange, color = "#14b8a6", label, width = 200, height = 200 }: CurveEditorProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [dragging, setDragging] = useState<number | null>(null);

  const w = width - PAD * 2;
  const h = height - PAD * 2;

  const toSvg = useCallback((p: CurvePoint) => ({
    x: PAD + p.x * w,
    y: PAD + (1 - p.y) * h,
  }), [w, h]);

  const fromSvg = useCallback((clientX: number, clientY: number): CurvePoint => {
    const svg = svgRef.current;
    if (!svg) return { x: 0, y: 0 };
    const rect = svg.getBoundingClientRect();
    const sx = clientX - rect.left;
    const sy = clientY - rect.top;
    return {
      x: clamp((sx - PAD) / w),
      y: clamp(1 - (sy - PAD) / h),
    };
  }, [w, h]);

  const handlePointerDown = useCallback((e: React.PointerEvent, idx: number) => {
    e.preventDefault();
    e.stopPropagation();
    (e.target as SVGElement).setPointerCapture(e.pointerId);
    setDragging(idx);
  }, []);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (dragging === null) return;
    const p = fromSvg(e.clientX, e.clientY);
    const isEndpoint = dragging === 0 || dragging === points.length - 1;
    const next = points.map((pt, i) => {
      if (i !== dragging) return pt;
      return isEndpoint ? { x: pt.x, y: p.y } : p;
    });
    onChange(next);
  }, [dragging, points, fromSvg, onChange]);

  const handlePointerUp = useCallback(() => {
    setDragging(null);
  }, []);

  const handleSvgDoubleClick = useCallback((e: React.MouseEvent) => {
    const p = fromSvg(e.clientX, e.clientY);
    const tooClose = points.some((pt) => Math.abs(pt.x - p.x) < 0.03 && Math.abs(pt.y - p.y) < 0.03);
    if (tooClose) return;
    const next = [...points, p].sort((a, b) => a.x - b.x);
    onChange(next);
  }, [points, fromSvg, onChange]);

  const handleRightClick = useCallback((e: React.MouseEvent, idx: number) => {
    e.preventDefault();
    if (idx === 0 || idx === points.length - 1) return;
    const next = points.filter((_, i) => i !== idx);
    onChange(next);
  }, [points, onChange]);

  const splinePath = catmullRomSpline(points);
  const svgPath = splinePath
    .replace(/M ([\d.]+) ([\d.]+)/g, (_, x, y) => {
      const sv = toSvg({ x: parseFloat(x), y: parseFloat(y) });
      return `M ${sv.x} ${sv.y}`;
    })
    .replace(/L ([\d.]+) ([\d.]+)/g, (_, x, y) => {
      const sv = toSvg({ x: parseFloat(x), y: parseFloat(y) });
      return `L ${sv.x} ${sv.y}`;
    });

  const gridLines = [];
  for (let i = 1; i < GRID_LINES; i++) {
    const frac = i / GRID_LINES;
    const gx = PAD + frac * w;
    const gy = PAD + frac * h;
    gridLines.push(
      <line key={`gx${i}`} x1={gx} y1={PAD} x2={gx} y2={PAD + h} stroke="rgba(63,63,70,0.3)" strokeWidth={0.5} />,
      <line key={`gy${i}`} x1={PAD} y1={gy} x2={PAD + w} y2={gy} stroke="rgba(63,63,70,0.3)" strokeWidth={0.5} />,
    );
  }

  return (
    <div className="flex flex-col gap-1">
      {label && <span className="text-[9px] font-medium" style={{ color }}>{label}</span>}
      <svg
        ref={svgRef}
        width={width}
        height={height}
        className="rounded border border-zinc-800/50 bg-zinc-950/80 cursor-crosshair select-none"
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onDoubleClick={handleSvgDoubleClick}
      >
        <rect x={PAD} y={PAD} width={w} height={h} fill="none" stroke="rgba(63,63,70,0.4)" strokeWidth={0.5} />
        {gridLines}

        <line
          x1={toSvg({ x: 0, y: 0 }).x} y1={toSvg({ x: 0, y: 0 }).y}
          x2={toSvg({ x: 1, y: 1 }).x} y2={toSvg({ x: 1, y: 1 }).y}
          stroke="rgba(63,63,70,0.25)" strokeWidth={0.5} strokeDasharray="3,3"
        />

        {svgPath && (
          <path d={svgPath} fill="none" stroke={color} strokeWidth={1.5} strokeLinecap="round" />
        )}

        {points.map((p, i) => {
          const sv = toSvg(p);
          return (
            <circle
              key={i}
              cx={sv.x}
              cy={sv.y}
              r={dragging === i ? POINT_R + 1 : POINT_R}
              fill={dragging === i ? color : "rgba(9,9,11,0.9)"}
              stroke={color}
              strokeWidth={1.5}
              className="cursor-grab active:cursor-grabbing"
              onPointerDown={(e) => handlePointerDown(e, i)}
              onContextMenu={(e) => handleRightClick(e, i)}
            />
          );
        })}
      </svg>
    </div>
  );
}

export default memo(CurveEditorInner);
