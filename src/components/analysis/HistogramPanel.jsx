import { useEffect, useRef, useCallback, useMemo, memo, useState } from "react";
import { Wand2, RotateCcw, SlidersHorizontal } from "lucide-react";

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
                        }) {
  const canvasRef = useRef(null);
  const containerRef = useRef(null);
  const draggingRef = useRef(null);
  const rafRef = useRef(null);
  const stateRef = useRef({ shadow, midtone, highlight });
  stateRef.current = { shadow, midtone, highlight };
  const CANVAS_H = 110;

  const [manualMode, setManualMode] = useState(false);
  const [draft, setDraft] = useState({ shadow: "", midtone: "", highlight: "" });

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

  const drawHistogram = useCallback(() => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      const canvas = canvasRef.current;
      if (!canvas || bins.length === 0) return;

      const rect = canvas.parentElement.getBoundingClientRect();
      const W = Math.floor(rect.width);
      const H = CANVAS_H;

      if (canvas.width !== W || canvas.height !== H) {
        canvas.width = W;
        canvas.height = H;
      }

      const ctx = canvas.getContext("2d", { alpha: false });
      ctx.fillStyle = "#0a0a0f";
      ctx.fillRect(0, 0, W, H);

      const maxBin = bins.reduce((a, b) => (a > b ? a : b), 1);
      const logMax = Math.log10(maxBin + 1);

      const barW = W / bins.length;
      ctx.fillStyle = "#3b82f6";
      for (let i = 0; i < bins.length; i++) {
        const logVal = Math.log10(bins[i] + 1);
        const h = (logVal / logMax) * (H - 4);
        ctx.fillRect(i * barW, H - h, Math.max(1, barW - 0.5), h);
      }

      const { shadow: s, midtone: m, highlight: hi } = stateRef.current;

      const shadowX = s * W;
      ctx.fillStyle = "rgba(0, 0, 0, 0.6)";
      ctx.fillRect(0, 0, shadowX, H);

      const highlightX = hi * W;
      ctx.fillStyle = "rgba(0, 0, 0, 0.6)";
      ctx.fillRect(highlightX, 0, W - highlightX, H);

      ctx.strokeStyle = "#ef4444";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(shadowX, 0);
      ctx.lineTo(shadowX, H);
      ctx.stroke();

      ctx.strokeStyle = "#22c55e";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(highlightX, 0);
      ctx.lineTo(highlightX, H);
      ctx.stroke();

      const midX = shadowX + m * (highlightX - shadowX);
      ctx.strokeStyle = "#eab308";
      ctx.lineWidth = 1.5;
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
  }, [bins]);

  useEffect(() => {
    drawHistogram();
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [drawHistogram, shadow, midtone, highlight]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const ro = new ResizeObserver(() => drawHistogram());
    ro.observe(container);
    return () => ro.disconnect();
  }, [drawHistogram]);

  const getMouseNorm = useCallback((e) => {
    const canvas = canvasRef.current;
    if (!canvas) return 0;
    const rect = canvas.getBoundingClientRect();
    return Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  }, []);

  const handleMouseDown = useCallback(
    (e) => {
      const norm = getMouseNorm(e);
      const { shadow: s, midtone: m, highlight: hi } = stateRef.current;
      const midX = s + m * (hi - s);

      const distS = Math.abs(norm - s);
      const distM = Math.abs(norm - midX);
      const distH = Math.abs(norm - hi);

      const threshold = 0.03;
      const minDist = Math.min(distS, distM, distH);
      if (minDist > threshold) return;

      if (minDist === distS) draggingRef.current = "shadow";
      else if (minDist === distH) draggingRef.current = "highlight";
      else draggingRef.current = "midtone";

      e.preventDefault();

      const canvas = canvasRef.current;
      if (canvas) canvas.style.willChange = "transform";

      const onMove = (ev) => {
        if (!draggingRef.current || !onChange) return;
        const n = getMouseNorm(ev);
        const { shadow: cs, midtone: cm, highlight: ch } = stateRef.current;

        if (draggingRef.current === "shadow") {
          const ns = Math.max(0, Math.min(n, ch - 0.01));
          onChange({ shadow: ns, midtone: cm, highlight: ch });
        } else if (draggingRef.current === "highlight") {
          const nh = Math.min(1, Math.max(n, cs + 0.01));
          onChange({ shadow: cs, midtone: cm, highlight: nh });
        } else if (draggingRef.current === "midtone") {
          const range = ch - cs;
          if (range > 0) {
            const nm = Math.max(0.001, Math.min(0.999, (n - cs) / range));
            onChange({ shadow: cs, midtone: nm, highlight: ch });
          }
        }
      };

      const onUp = () => {
        draggingRef.current = null;
        if (canvas) canvas.style.willChange = "";
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
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
      <div className="flex items-center justify-between px-3 py-1.5 border-b border-zinc-800/50">
        <h4 className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">
          Histogram / STF
        </h4>
        <div className="flex items-center gap-1">
          <button
            onClick={onAutoStf}
            className="flex items-center gap-1 text-[10px] text-blue-400 hover:text-blue-300 px-1.5 py-0.5 rounded hover:bg-zinc-800 transition-colors"
            title="Auto Stretch (STF)"
          >
            <Wand2 size={11} />
            Auto
          </button>
          <button
            onClick={openManual}
            className={`flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded hover:bg-zinc-800 transition-colors ${
              manualMode ? "text-teal-400" : "text-zinc-500 hover:text-zinc-300"
            }`}
            title="Manual input"
          >
            <SlidersHorizontal size={11} />
          </button>
          <button
            onClick={onReset}
            className="flex items-center gap-1 text-[10px] text-zinc-500 hover:text-zinc-300 px-1.5 py-0.5 rounded hover:bg-zinc-800 transition-colors"
            title="Reset to linear"
          >
            <RotateCcw size={11} />
          </button>
        </div>
      </div>

      <div ref={containerRef} className="px-1.5 pt-1.5">
        <canvas
          ref={canvasRef}
          height={CANVAS_H}
          className="w-full rounded cursor-crosshair"
          style={{ height: CANVAS_H }}
          onMouseDown={handleMouseDown}
        />
      </div>

      {manualMode ? (
        <div className="flex items-center gap-1.5 px-3 py-1.5">
          <label className="text-[10px] text-red-400 font-mono w-4">S</label>
          <input
            type="number"
            value={draft.shadow}
            step={0.001}
            min={0}
            max={1}
            onChange={(e) => setDraft((d) => ({ ...d, shadow: e.target.value }))}
            className="w-20 text-[10px] font-mono bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-zinc-200 focus:border-red-500 focus:outline-none"
          />
          <label className="text-[10px] text-yellow-400 font-mono w-4 ml-1">M</label>
          <input
            type="number"
            value={draft.midtone}
            step={0.001}
            min={0.001}
            max={0.999}
            onChange={(e) => setDraft((d) => ({ ...d, midtone: e.target.value }))}
            className="w-20 text-[10px] font-mono bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-zinc-200 focus:border-yellow-500 focus:outline-none"
          />
          <label className="text-[10px] text-green-400 font-mono w-4 ml-1">H</label>
          <input
            type="number"
            value={draft.highlight}
            step={0.001}
            min={0}
            max={1}
            onChange={(e) => setDraft((d) => ({ ...d, highlight: e.target.value }))}
            className="w-20 text-[10px] font-mono bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-zinc-200 focus:border-green-500 focus:outline-none"
          />
          <button
            onClick={applyManual}
            className="ml-auto text-[10px] px-2 py-0.5 rounded bg-teal-900/40 text-teal-400 border border-teal-800/50 hover:bg-teal-800/40 transition-colors"
          >
            Apply
          </button>
          <button
            onClick={() => setManualMode(false)}
            className="text-[10px] px-2 py-0.5 rounded text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800 transition-colors"
          >
            Cancel
          </button>
        </div>
      ) : (
        <div className="flex items-center justify-between px-3 py-1.5 text-[10px] font-mono">
          <span className="text-red-400">
            S: {shadow.toFixed(4)} ({shadowVal})
          </span>
          <span className="text-yellow-400">M: {midtone.toFixed(4)}</span>
          <span className="text-green-400">
            H: {highlight.toFixed(4)} ({highlightVal})
          </span>
        </div>
      )}

      {stats && (
        <div className="flex items-center gap-3 px-3 pb-1.5 text-[10px] font-mono text-zinc-500">
          <span>u={stats.mean?.toFixed(1)}</span>
          <span>med={stats.median?.toFixed(1)}</span>
          <span>o={stats.sigma?.toFixed(1)}</span>
        </div>
      )}
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
