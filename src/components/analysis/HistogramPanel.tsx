import { useEffect, useRef, useCallback, useMemo, memo, useState } from "react";
import { Wand2, RotateCcw, SlidersHorizontal, Check } from "lucide-react";
import type { StfParams } from "../../shared/types";
import { constrainStf, dragStfMarker, pickStfMarker, stfMarkerPositions } from "../../utils/histogramWindow";
import type { HistogramRange } from "../../utils/histogramWindow";
import { MEAN_LABEL, SIGMA_MAD_LABEL, SIGMA_MAD_TITLE, skyRangeLabel } from "../../utils/analysisLabels";

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
  disabled?: boolean;
  disabledHint?: string;
  badge?: React.ReactNode;
  binsWindow?: HistogramRange | null;
  skyZoom?: boolean;
  skyAvailable?: boolean;
  skyLoading?: boolean;
  onSkyZoomChange?: (on: boolean) => void;
}

const DISABLED_HINT = "STF applies only to the mtf stretch";
const SKY_TITLE = "Zoom on the sky: median − 5 σ(MAD) to median + 50 σ(MAD); markers outside it sit on the edge";

function pinnedLabel(label: string, pos: { x: number; pinned: boolean }): string {
  if (!pos.pinned) return label;
  return pos.x === 0 ? `◂${label}` : `${label}▸`;
}

function HistogramPanel({
                          bins = [],
                          dataMin = 0,
                          dataMax = 1,
                          shadow = 0,
                          midtone = 0.5,
                          highlight = 1,
                          onChange,
                          onAutoStf,
                          onReset,
                          stats,
                          disabled = false,
                          disabledHint = DISABLED_HINT,
                          badge,
                          binsWindow = null,
                          skyZoom = false,
                          skyAvailable = false,
                          skyLoading = false,
                          onSkyZoomChange,
                        }: HistogramPanelProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef<"shadow" | "midtone" | "highlight" | null>(null);
  const rafRef = useRef<number | null>(null);
  const overlayRafRef = useRef<number | null>(null);
  const frame = useMemo(() => ({ min: dataMin, max: dataMax }), [dataMin, dataMax]);
  const stateRef = useRef({ shadow, midtone, highlight, frame, binsWindow });
  stateRef.current = { shadow, midtone, highlight, frame, binsWindow };

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
    const typed = {
      shadow: parseFloat(draft.shadow) || 0,
      midtone: parseFloat(draft.midtone) || 0.5,
      highlight: parseFloat(draft.highlight) || 1,
    };
    onChange(constrainStf(typed, frame, binsWindow));
    setManualMode(false);
  }, [draft, onChange, frame, binsWindow]);

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

      const { frame: f, binsWindow: win, ...stf } = stateRef.current;
      const { shadow: shadowPos, midtone: midPos, highlight: highlightPos } = stfMarkerPositions(stf, f, win);
      const shadowX = shadowPos.x * W;
      const highlightX = highlightPos.x * W;
      const midX = midPos.x * W;

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
      const drawLabel = (text: string, x: number, y: number, side: "left" | "right") => {
        const width = ctx.measureText(text).width;
        const left = x - 3 - width;
        const right = x + 3;
        const useLeft = side === "left" ? left >= 0 : right + width > W;
        ctx.fillText(text, useLeft ? left : right, y);
      };
      ctx.fillStyle = "#ef4444";
      drawLabel(pinnedLabel("S", shadowPos), shadowX, 11, "right");
      ctx.fillStyle = "#eab308";
      drawLabel(pinnedLabel("M", midPos), midX, midPos.pinned ? 23 : 11, "right");
      ctx.fillStyle = "#22c55e";
      drawLabel(pinnedLabel("H", highlightPos), highlightX, 11, "left");
    });
  }, []);

  const drawOverlayRef = useRef(drawOverlay);
  drawOverlayRef.current = drawOverlay;

  useEffect(() => {
    drawBars();
    drawOverlayRef.current();
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
      if (overlayRafRef.current) cancelAnimationFrame(overlayRafRef.current);
    };
  }, [drawBars]);

  useEffect(() => { drawOverlay(); }, [shadow, midtone, highlight, frame, binsWindow, drawOverlay]);

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
      if (disabled) return;
      const { frame: f, binsWindow: win, ...stf } = stateRef.current;
      const marker = pickStfMarker(getMouseNorm(e.nativeEvent), stf, f, win, DRAG_THRESHOLD);
      if (!marker) return;

      draggingRef.current = marker;
      e.preventDefault();

      const onMove = (ev: MouseEvent) => {
        const dragging = draggingRef.current;
        if (!dragging || !onChange) return;
        const { frame: cf, binsWindow: cw, ...current } = stateRef.current;
        const next = dragStfMarker(dragging, getMouseNorm(ev), current, cf, cw);
        if (next) onChange(next);
      };

      const onUp = () => {
        draggingRef.current = null;
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };

      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [getMouseNorm, onChange, disabled],
  );

  useEffect(() => {
    if (disabled) setManualMode(false);
  }, [disabled]);

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
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">
            Histogram / STF
          </span>
          {badge}
        </div>
        <div className="flex items-center gap-0.5">
          {onSkyZoomChange && (
            <div className="flex items-center mr-1 rounded-md overflow-hidden" style={{ border: "1px solid rgba(63,63,70,0.4)" }}>
              <button
                type="button"
                onClick={() => onSkyZoomChange(false)}
                aria-pressed={!skyZoom}
                title="Full range: bins over the whole data range"
                className="text-[9px] px-1.5 py-0.5 transition-colors"
                style={{ color: skyZoom ? "#71717a" : "var(--ab-blue)", background: skyZoom ? undefined : "rgba(59,130,246,0.1)" }}
              >
                Full
              </button>
              <button
                type="button"
                onClick={() => onSkyZoomChange(true)}
                disabled={!skyAvailable}
                aria-pressed={skyZoom}
                title={skyAvailable ? SKY_TITLE : "No sky window: σ(MAD) is zero or the data range is empty"}
                className="flex items-center gap-1 text-[9px] px-1.5 py-0.5 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                style={{ color: skyZoom ? "var(--ab-blue)" : "#71717a", background: skyZoom ? "rgba(59,130,246,0.1)" : undefined }}
              >
                Sky
                {skyLoading && <span className="w-2 h-2 rounded-full animate-pulse" style={{ background: "var(--ab-blue)" }} />}
              </button>
            </div>
          )}
          <ToolbarBtn onClick={onAutoStf} title={disabled ? disabledHint : "Auto Stretch (STF)"} active={false} color="var(--ab-blue)" disabled={disabled}>
            <Wand2 size={11} />
            <span>Auto</span>
          </ToolbarBtn>
          <ToolbarBtn onClick={openManual} title={disabled ? disabledHint : "Manual input"} active={manualMode} color="var(--ab-teal)" disabled={disabled}>
            <SlidersHorizontal size={11} />
          </ToolbarBtn>
          <ToolbarBtn onClick={onReset} title={disabled ? disabledHint : "Reset to linear"} active={false} color="#71717a" disabled={disabled}>
            <RotateCcw size={11} />
          </ToolbarBtn>
        </div>
      </div>

      <div ref={containerRef} className="relative" style={{ height: CANVAS_H, margin: "6px 6px 0", opacity: disabled ? 0.55 : 1 }}>
        <canvas
          ref={canvasRef}
          height={CANVAS_H}
          className="w-full rounded-md absolute inset-0"
          style={{ height: CANVAS_H }}
        />
        <canvas
          ref={overlayRef}
          height={CANVAS_H}
          className={`w-full rounded-md absolute inset-0 ${disabled ? "cursor-not-allowed" : "cursor-crosshair"}`}
          style={{ height: CANVAS_H }}
          onMouseDown={handleMouseDown}
          title={disabled ? disabledHint : undefined}
        />
      </div>

      {disabled && (
        <div className="px-3 py-1 text-[9px] text-zinc-500" style={{ borderTop: "1px solid rgba(63,63,70,0.12)" }}>
          {disabledHint}
        </div>
      )}

      {manualMode ? (
        <div
          className="flex items-center gap-1.5 px-3 py-2 animate-fade-in"
          style={{ borderTop: "1px solid var(--ab-border)" }}
        >
          <ManualInput label="S" name="S shadow clipping point (0-1)" color="#ef4444" value={draft.shadow} min={0} max={1}
                       onChange={(v) => setDraft((d) => ({ ...d, shadow: v }))} />
          <ManualInput label="M" name="M midtone balance (0-1)" color="#eab308" value={draft.midtone} min={0.001} max={0.999}
                       onChange={(v) => setDraft((d) => ({ ...d, midtone: v }))} />
          <ManualInput label="H" name="H highlight clipping point (0-1)" color="#22c55e" value={draft.highlight} min={0} max={1}
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
          <span>{`${MEAN_LABEL}=${stats.mean?.toFixed(1)}`}</span>
          <span>med={stats.median?.toFixed(1)}</span>
          <span title={SIGMA_MAD_TITLE}>{`${SIGMA_MAD_LABEL}=${stats.sigma?.toFixed(1)}`}</span>
          {binsWindow && (
            <span className="ml-auto text-zinc-600" title={SKY_TITLE}>
              {skyRangeLabel(binsWindow)}
            </span>
          )}
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
                      disabled = false,
                      children,
                    }: {
  onClick?: () => void;
  title: string;
  active: boolean;
  color: string;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      disabled={disabled}
      className="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-md transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
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
                       name,
                       color,
                       value,
                       min,
                       max,
                       onChange,
                     }: {
  label: string;
  name: string;
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
        aria-label={name}
        value={value}
        step={0.001}
        min={min}
        max={max}
        onChange={(e) => onChange(e.target.value)}
        className="w-[72px] text-[10px] font-mono rounded-md px-1.5 py-0.5 text-zinc-200 transition-colors"
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

export default memo(HistogramPanel);
