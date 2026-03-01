import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { motion } from "framer-motion";
import { Wand2, RotateCcw } from "lucide-react";

/**
 * HistogramPanel — renders a log-scale histogram with draggable
 * shadow / midtone / highlight sliders. Changes are reported in real-time
 * via the `onChange` callback.
 *
 * Props:
 *   bins       - Array<number> (e.g. 512 bins)
 *   dataMin    - absolute min in the data
 *   dataMax    - absolute max in the data
 *   autoStf    - { shadow, midtone, highlight } from the backend
 *   shadow     - current black-point [0..1]
 *   midtone    - current midtone balance (0..1)
 *   highlight  - current white-point [0..1]
 *   onChange   - (params: {shadow, midtone, highlight}) => void
 *   onAutoStf  - () => void  (apply auto-stretch)
 *   onReset    - () => void  (reset to linear)
 *   stats      - { median, mean, sigma }  optional stats overlay
 */
export default function HistogramPanel({
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
  const CANVAS_H = 120;

  
  const drawHistogram = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || bins.length === 0) return;

    const rect = canvas.parentElement.getBoundingClientRect();
    const W = Math.floor(rect.width);
    const H = CANVAS_H;
    canvas.width = W;
    canvas.height = H;

    const ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, W, H);

    
    ctx.fillStyle = "#0a0a0f";
    ctx.fillRect(0, 0, W, H);

    
    const maxBin = Math.max(1, ...bins);
    const logMax = Math.log10(maxBin + 1);

    
    const barW = W / bins.length;
    ctx.fillStyle = "#3b82f6";
    for (let i = 0; i < bins.length; i++) {
      const logVal = Math.log10(bins[i] + 1);
      const h = (logVal / logMax) * (H - 4);
      ctx.fillRect(i * barW, H - h, Math.max(1, barW - 0.5), h);
    }

    
    const shadowX = shadow * W;
    ctx.fillStyle = "rgba(0, 0, 0, 0.6)";
    ctx.fillRect(0, 0, shadowX, H);

    
    const highlightX = highlight * W;
    ctx.fillStyle = "rgba(0, 0, 0, 0.6)";
    ctx.fillRect(highlightX, 0, W - highlightX, H);

    
    ctx.strokeStyle = "#ef4444";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(shadowX, 0);
    ctx.lineTo(shadowX, H);
    ctx.stroke();

    
    ctx.strokeStyle = "#22c55e";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(highlightX, 0);
    ctx.lineTo(highlightX, H);
    ctx.stroke();

    
    const midX = shadow * W + midtone * (highlightX - shadowX);
    ctx.strokeStyle = "#eab308";
    ctx.lineWidth = 2;
    ctx.setLineDash([4, 3]);
    ctx.beginPath();
    ctx.moveTo(midX, 0);
    ctx.lineTo(midX, H);
    ctx.stroke();
    ctx.setLineDash([]);

    
    ctx.font = "10px 'JetBrains Mono', monospace";
    ctx.fillStyle = "#ef4444";
    ctx.fillText("S", shadowX + 4, 12);
    ctx.fillStyle = "#eab308";
    ctx.fillText("M", midX + 4, 12);
    ctx.fillStyle = "#22c55e";
    ctx.fillText("H", highlightX - 14, 12);
  }, [bins, shadow, midtone, highlight]);

  useEffect(() => {
    drawHistogram();
  }, [drawHistogram]);

  
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
      const canvas = canvasRef.current;
      if (!canvas) return;
      const rect = canvas.getBoundingClientRect();
      const W = rect.width;

      const shadowX = shadow;
      const highlightX = highlight;
      const midX = shadow + midtone * (highlight - shadow);

      
      const distS = Math.abs(norm - shadowX);
      const distM = Math.abs(norm - midX);
      const distH = Math.abs(norm - highlightX);

      const threshold = 0.03; 
      const minDist = Math.min(distS, distM, distH);

      if (minDist > threshold) return; 

      if (minDist === distS) draggingRef.current = "shadow";
      else if (minDist === distH) draggingRef.current = "highlight";
      else draggingRef.current = "midtone";

      e.preventDefault();
    },
    [shadow, midtone, highlight, getMouseNorm]
  );

  const handleMouseMove = useCallback(
    (e) => {
      if (!draggingRef.current || !onChange) return;
      const norm = getMouseNorm(e);

      if (draggingRef.current === "shadow") {
        const newShadow = Math.min(norm, highlight - 0.01);
        onChange({ shadow: Math.max(0, newShadow), midtone, highlight });
      } else if (draggingRef.current === "highlight") {
        const newHighlight = Math.max(norm, shadow + 0.01);
        onChange({ shadow, midtone, highlight: Math.min(1, newHighlight) });
      } else if (draggingRef.current === "midtone") {
        
        const range = highlight - shadow;
        if (range > 0) {
          const newMid = Math.max(0.001, Math.min(0.999, (norm - shadow) / range));
          onChange({ shadow, midtone: newMid, highlight });
        }
      }
    },
    [shadow, midtone, highlight, onChange, getMouseNorm]
  );

  const handleMouseUp = useCallback(() => {
    draggingRef.current = null;
  }, []);

  useEffect(() => {
    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, [handleMouseMove, handleMouseUp]);

  
  const shadowVal = useMemo(
    () => (dataMin + shadow * (dataMax - dataMin)).toFixed(1),
    [shadow, dataMin, dataMax]
  );
  const highlightVal = useMemo(
    () => (dataMin + highlight * (dataMax - dataMin)).toFixed(1),
    [highlight, dataMin, dataMax]
  );

  if (bins.length === 0) return null;

  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2 }}
      className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">
          Histogram / STF
        </h4>
        <div className="flex items-center gap-1">
          <button
            onClick={onAutoStf}
            className="flex items-center gap-1 text-xs text-blue-400 hover:text-blue-300 px-2 py-1 rounded hover:bg-zinc-800 transition-colors"
            title="Auto Stretch (STF)"
          >
            <Wand2 size={12} />
            Auto
          </button>
          <button
            onClick={onReset}
            className="flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-300 px-2 py-1 rounded hover:bg-zinc-800 transition-colors"
            title="Reset to linear"
          >
            <RotateCcw size={12} />
          </button>
        </div>
      </div>

      {/* Canvas */}
      <div ref={containerRef} className="px-2 pt-2">
        <canvas
          ref={canvasRef}
          height={CANVAS_H}
          className="w-full rounded cursor-crosshair"
          style={{ height: CANVAS_H }}
          onMouseDown={handleMouseDown}
        />
      </div>

      {/* Slider values */}
      <div className="flex items-center justify-between px-3 py-2 text-[10px] font-mono">
        <span className="text-red-400" title="Shadow (Black Point)">
          S: {shadow.toFixed(4)} ({shadowVal})
        </span>
        <span className="text-yellow-400" title="Midtone Balance">
          M: {midtone.toFixed(4)}
        </span>
        <span className="text-green-400" title="Highlight (White Point)">
          H: {highlight.toFixed(4)} ({highlightVal})
        </span>
      </div>

      {/* Stats */}
      {stats && (
        <div className="flex items-center gap-3 px-3 pb-2 text-[10px] font-mono text-zinc-500">
          <span>μ={stats.mean?.toFixed(1)}</span>
          <span>med={stats.median?.toFixed(1)}</span>
          <span>σ={stats.sigma?.toFixed(1)}</span>
        </div>
      )}
    </motion.div>
  );
}
