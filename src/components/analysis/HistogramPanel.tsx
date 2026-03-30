import { useEffect, useRef, useCallback, useMemo, memo, useState } from "react";
import { Wand2, RotateCcw, SlidersHorizontal, Check } from "lucide-react";
import type { StfParams } from "../../shared/types";

const CANVAS_H = 110;
const DRAG_THRESHOLD = 0.03;
const BAR_COLOR = "#3b82f6";
const BG_COLOR = "#0a0a0f";

interface HistogramStats {
  mean?: number;
  median?: number;
  sigma?: number;
}

interface HistogramPanelProps {
  bins?: number[];
  dataMin?: number;
  dataMax?: number;
  autoStf?: StfParams;
  shadow?: number;
  midtone?: number;
  highlight?: number;
  onChange?: (params: StfParams) => void;
  onAutoStf?: () => void;
  onReset?: () => void;
  stats?: HistogramStats | null;
}

function HistogramPanel({
                          bins = [],
                          dataMin = 0,
                          dataMax = 1,
                          autoStf,
                          shadow = 0,
                          midtone = 0.5,
                          highlight = 1,
                          onChange,
                          onAutoStf,
                          onReset,
                          stats,
                        }: HistogramPanelProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef<"shadow" | "midtone" | "highlight" | null>(null);
  const rafRef = useRef<number | null>(null);
  const overlayRafRef = useRef<number | null>(null);
  const stateRef = useRef({ shadow, midtone, highlight });
  stateRef.current = { shadow, midtone, highlight };

  const [manualMode, setManualMode] = useState(false);
  const [draft, setDraft] = useState({ shadow: "", midtone: "", highlight: "" });

  const logMax = useMemo(() => {
    if (bins.length === 0) return 1;
    let max = 1;
    for (let i = 0; i < bins.length; i++) {
      if (bins[i] > max) max = bins[i];
    }
    return Math.log10(max + 1);
  }, [bins]);

  const openManual = useCallback(() => {
    setDraft({
      shadow: shadow.toFixed(4),
      midtone: midtone.toFixed(4),
      highlight: highlight.toFixed(4),
    });
    setManualMode(true);
  }, [shadow, midtone, highlight]);

  const applyManual = useCallback(() => {
    if (!onChange) return;
    const s = Math.max(0, Math.min(parseFloat(draft.shadow) || 0, 1));
    const h = Math.max(0, Math.min(parseFloat(draft.highlight) || 1, 1));
    const m = Math.max(0.001, Math.min(parseFloat(draft.midtone) || 0.5, 0.999));
    onChange({ shadow: s, midtone: m, highlight: Math.max(s + 0.01, h) });
    setManualMode(false);
  }, [draft, onChange]);

  const drawBars = useCallback(() => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      const canvas = canvasRef.current;
      if (!canvas || bins.length === 0) return;

      const rect = canvas.parentElement!.getBoundingClientRect();
      const W = Math.floor(rect.width);
      const H = CANVAS_H;

      if (canvas.width !== W || canvas.height !== H) {
        canvas.width = W;
        canvas.height = H;
      }

      const ctx = canvas.getContext("2d", { alpha: false });
      if (!ctx) return;
      ctx.fillStyle = BG_COLOR;
      ctx.fillRect(0, 0, W, H);

      const barW = W / bins.length;
      ctx.fillStyle = BAR_COLOR;
      for (let i = 0; i < bins.length; i++) {
        const h = (Math.log10(bins[i] + 1) / logMax) * (H - 4);
        ctx.fillRect(i * barW, H - h, Math.max(1, barW - 0.5), h);
      }
    });
  }, [bins, logMax]);

  const drawOverlay = useCallback(() => {
    if (overlayRafRef.current) cancelAnimationFrame(overlayRafRef.current);
    overlayRafRef.current = requestAnimationFrame(() => {
      const canvas = overlayRef.current;
      const barsCanvas = canvasRef.current;
      if (!canvas || !barsCanvas) return;

      const W = barsCanvas.width;
      const H = barsCanvas.height;

      if (canvas.width !== W || canvas.height !== H) {
        canvas.width = W;
        canvas.height = H;
      }

      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.clearRect(0, 0, W, H);

      const { shadow: s, midtone: m, highlight: hi } = stateRef.current;
      const shadowX = s * W;
      const highlightX = hi * W;
      const midX = shadowX + m * (highlightX - shadowX);

      ctx.fillStyle = "rgba(0, 0, 0, 0.6)";
      ctx.fillRect(0, 0, shadowX, H);
      ctx.fillRect(highlightX, 0, W - highlightX, H);

      ctx.lineWidth = 1.5;

      ctx.strokeStyle = "#ef4444";
      ctx.beginPath();
      ctx.moveTo(shadowX, 0);
      ctx.lineTo(shadowX, H);
      ctx.stroke();

      ctx.strokeStyle = "#22c55e";
      ctx.beginPath();
      ctx.moveTo(highlightX, 0);
      ctx.lineTo(highlightX, H);
      ctx.stroke();

      ctx.strokeStyle = "#eab308";
      ctx.setLineDash([4, 3]);
      ctx.beginPath();
      ctx.moveTo(midX, 0);
      ctx.lineTo(midX, H);
      ctx.stroke();
      ctx.setLineDash([]);

      ctx.font = "10px 'JetBrains Mono', monospace";
      ctx.fillStyle = "#ef4444";
      ctx.fillText("S", shadowX + 3, 11);
      ctx.fillStyle = "#eab308";
      ctx.fillText("M", midX + 3, 11);
      ctx.fillStyle = "#22c55e";
      ctx.fillText("H", highlightX - 13, 11);
    });
  }, []);

  useEffect(() => {
    drawBars();
    drawOverlay();
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
      if (overlayRafRef.current) cancelAnimationFrame(overlayRafRef.current);
    };
  }, [drawBars]);

  useEffect(() => { drawOverlay(); }, [shadow, midtone, highlight, drawOverlay]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const ro = new ResizeObserver(() => { drawBars(); drawOverlay(); });
    ro.observe(container);
    return () => {
      ro.disconnect();
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
      if (overlayRafRef.current) cancelAnimationFrame(overlayRafRef.current);
    };
  }, [drawBars, drawOverlay]);

  const getMouseNorm = useCallback((e: MouseEvent) => {
    const canvas = overlayRef.current || canvasRef.current;
    if (!canvas) return 0;
    const rect = canvas.getBoundingClientRect();
    return Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  }, []);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const norm = getMouseNorm(e.nativeEvent);
      const { shadow: s, midtone: m, highlight: hi } = stateRef.current;
      const midX = s + m * (hi - s);

      const distS = Math.abs(norm - s);
      const distM = Math.abs(norm - midX);
      const distH = Math.abs(norm - hi);
      const minDist = Math.min(distS, distM, distH);

      if (minDist > DRAG_THRESHOLD) return;

      draggingRef.current =
        minDist === distS ? "shadow" : minDist === distH ? "highlight" : "midtone";
      e.preventDefault();

      const onMove = (ev: MouseEvent) => {
        if (!draggingRef.current || !onChange) return;
        const n = getMouseNorm(ev);
        const { shadow: cs, midtone: cm, highlight: ch } = stateRef.current;

        if (draggingRef.current === "shadow") {
          onChange({ shadow: Math.max(0, Math.min(n, ch - 0.01)), midtone: cm, highlight: ch });
        } else if (draggingRef.current === "highlight") {
          onChange({ shadow: cs, midtone: cm, highlight: Math.min(1, Math.max(n, cs + 0.01)) });
        } else {
          const range = ch - cs;
          if (range > 0) {
            onChange({ shadow: cs, midtone: Math.max(0.001, Math.min(0.999, (n - cs) / range)), highlight: ch });
          }
        }
      };

      const onUp = () => {
        draggingRef.current = null;
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };

      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [getMouseNorm, onChange],
  );

  const shadowVal = useMemo(
    () => (dataMin + shadow * (dataMax - dataMin)).toFixed(1),
    [shadow, dataMin, dataMax],
  );
  const highlightVal = useMemo(
    () => (dataMin + highlight * (dataMax - dataMin)).toFixed(1),
    [highlight, dataMin, dataMax],
  );

  if (bins.length === 0) return null;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="ab-panel-header" style={{ padding: "4px 12px" }}>
        <span className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">
          Histogram / STF
        </span>
        <div className="flex items-center gap-0.5">
          <ToolbarBtn onClick={onAutoStf} title="Auto Stretch (STF)" active={false} color="var(--ab-blue)">
            <Wand2 size={11} />
            <span>Auto</span>
          </ToolbarBtn>
          <ToolbarBtn onClick={openManual} title="Manual input" active={manualMode} color="var(--ab-teal)">
            <SlidersHorizontal size={11} />
          </ToolbarBtn>
          <ToolbarBtn onClick={onReset} title="Reset to linear" active={false} color="#71717a">
            <RotateCcw size={11} />
          </ToolbarBtn>
        </div>
      </div>

      <div ref={containerRef} className="relative" style={{ height: CANVAS_H, margin: "6px 6px 0" }}>
        <canvas
          ref={canvasRef}
          height={CANVAS_H}
          className="w-full rounded-md absolute inset-0"
          style={{ height: CANVAS_H }}
        />
        <canvas
          ref={overlayRef}
          height={CANVAS_H}
          className="w-full rounded-md absolute inset-0 cursor-crosshair"
          style={{ height: CANVAS_H }}
          onMouseDown={handleMouseDown}
        />
      </div>

      {manualMode ? (
        <div
          className="flex items-center gap-1.5 px-3 py-2 animate-fade-in"
          style={{ borderTop: "1px solid var(--ab-border)" }}
        >
          <ManualInput label="S" color="#ef4444" value={draft.shadow} min={0} max={1}
                       onChange={(v) => setDraft((d) => ({ ...d, shadow: v }))} />
          <ManualInput label="M" color="#eab308" value={draft.midtone} min={0.001} max={0.999}
                       onChange={(v) => setDraft((d) => ({ ...d, midtone: v }))} />
          <ManualInput label="H" color="#22c55e" value={draft.highlight} min={0} max={1}
                       onChange={(v) => setDraft((d) => ({ ...d, highlight: v }))} />
          <button
            onClick={applyManual}
            className="ml-auto p-1 rounded-md transition-colors"
            style={{ background: "rgba(20,184,166,0.12)", color: "var(--ab-teal)" }}
            title="Apply"
          >
            <Check size={12} />
          </button>
          <button
            onClick={() => setManualMode(false)}
            className="text-[10px] px-2 py-0.5 rounded text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/40 transition-colors"
          >
            Cancel
          </button>
        </div>
      ) : (
        <div className="flex items-center justify-between px-3 py-1.5 text-[10px] font-mono">
          <span style={{ color: "#ef4444" }}>
            S: {shadow.toFixed(4)} <span className="text-zinc-600">({shadowVal})</span>
          </span>
          <span style={{ color: "#eab308" }}>M: {midtone.toFixed(4)}</span>
          <span style={{ color: "#22c55e" }}>
            H: {highlight.toFixed(4)} <span className="text-zinc-600">({highlightVal})</span>
          </span>
        </div>
      )}

      {stats && (
        <div
          className="flex items-center gap-4 px-3 py-1.5 text-[10px] font-mono text-zinc-500"
          style={{ borderTop: "1px solid rgba(63,63,70,0.12)" }}
        >
          <span>\u03bc={stats.mean?.toFixed(1)}</span>
          <span>med={stats.median?.toFixed(1)}</span>
          <span>\u03c3={stats.sigma?.toFixed(1)}</span>
        </div>
      )}
    </div>
  );
}

function ToolbarBtn({
                      onClick,
                      title,
                      active,
                      color,
                      children,
                    }: {
  onClick?: () => void;
  title: string;
  active: boolean;
  color: string;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-md transition-colors"
      style={{
        color: active ? color : "#71717a",
        background: active ? "rgba(20,184,166,0.08)" : undefined,
      }}
      onMouseOver={(e) => { e.currentTarget.style.background = "rgba(63,63,70,0.3)"; }}
      onMouseOut={(e) => { e.currentTarget.style.background = active ? "rgba(20,184,166,0.08)" : ""; }}
    >
      {children}
    </button>
  );
}

function ManualInput({
                       label,
                       color,
                       value,
                       min,
                       max,
                       onChange,
                     }: {
  label: string;
  color: string;
  value: string;
  min: number;
  max: number;
  onChange: (v: string) => void;
}) {
  return (
    <div className="flex items-center gap-1">
      <span className="text-[10px] font-mono w-3 font-semibold" style={{ color }}>{label}</span>
      <input
        type="number"
        value={value}
        step={0.001}
        min={min}
        max={max}
        onChange={(e) => onChange(e.target.value)}
        className="w-[72px] text-[10px] font-mono rounded-md px-1.5 py-0.5 text-zinc-200 outline-none transition-colors"
        style={{
          background: "rgba(24,24,32,0.8)",
          border: `1px solid ${color}33`,
        }}
        onFocus={(e) => { e.currentTarget.style.borderColor = color; }}
        onBlur={(e) => { e.currentTarget.style.borderColor = `${color}33`; }}
      />
    </div>
  );
}

export default memo(HistogramPanel, (prev, next) =>
  prev.shadow === next.shadow &&
  prev.midtone === next.midtone &&
  prev.highlight === next.highlight &&
  prev.bins === next.bins &&
  prev.dataMin === next.dataMin &&
  prev.dataMax === next.dataMax &&
  prev.stats === next.stats
);
