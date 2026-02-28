import { useState, useRef, useCallback, useEffect, useMemo } from "react";
import { motion } from "framer-motion";
import { Activity, Crosshair, Loader2 } from "lucide-react";

/**
 * SpectroscopyPanel — plots wavelength vs. flux for a selected pixel
 * in an IFU data cube. The user clicks on the preview image and this
 * panel fetches + renders the spectrum at that pixel.
 *
 * Props:
 *   spectrum     - number[]  flux values per channel
 *   wavelengths  - number[]  wavelength axis (μm) or null for channel index
 *   pixelCoord   - { x, y } or null
 *   isLoading    - boolean
 *   onPixelClick - (x, y) => void — called when user clicks image
 *   cubeDims     - { naxis1, naxis2, naxis3 } or null
 *   elapsed      - ms the fetch took
 */
export default function SpectroscopyPanel({
  spectrum = [],
  wavelengths = null,
  pixelCoord = null,
  isLoading = false,
  cubeDims = null,
  elapsed = 0,
}) {
  const canvasRef = useRef(null);
  const containerRef = useRef(null);
  const [hoveredIdx, setHoveredIdx] = useState(null);

  const CANVAS_H = 180;

  
  const { xMin, xMax, yMin, yMax, xLabel } = useMemo(() => {
    if (!spectrum || spectrum.length === 0)
      return { xMin: 0, xMax: 1, yMin: 0, yMax: 1, xLabel: "Channel" };

    const n = spectrum.length;
    const hasWl = wavelengths && wavelengths.length === n;

    const xs = hasWl ? wavelengths : Array.from({ length: n }, (_, i) => i);
    const xMin = Math.min(...xs);
    const xMax = Math.max(...xs);

    const finite = spectrum.filter((v) => Number.isFinite(v));
    const yMin = finite.length ? Math.min(...finite) : 0;
    const yMax = finite.length ? Math.max(...finite) : 1;

    return {
      xMin,
      xMax,
      yMin: yMin - (yMax - yMin) * 0.05,
      yMax: yMax + (yMax - yMin) * 0.05,
      xLabel: hasWl ? "Wavelength (μm)" : "Channel",
    };
  }, [spectrum, wavelengths]);

  
  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || spectrum.length === 0) return;

    const parent = canvas.parentElement;
    const W = Math.floor(parent.getBoundingClientRect().width);
    const H = CANVAS_H;
    canvas.width = W;
    canvas.height = H;

    const ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, W, H);

    ctx.fillStyle = "#0a0a0f";
    ctx.fillRect(0, 0, W, H);

    const pad = { top: 10, bottom: 24, left: 50, right: 12 };
    const plotW = W - pad.left - pad.right;
    const plotH = H - pad.top - pad.bottom;

    ctx.strokeStyle = "#1f1f28";
    ctx.lineWidth = 0.5;
    const nGridY = 4;
    for (let i = 0; i <= nGridY; i++) {
      const y = pad.top + (i / nGridY) * plotH;
      ctx.beginPath();
      ctx.moveTo(pad.left, y);
      ctx.lineTo(W - pad.right, y);
      ctx.stroke();
    }

    const n = spectrum.length;
    const hasWl = wavelengths && wavelengths.length === n;
    const xRange = Math.max(xMax - xMin, 1e-10);
    const yRange = Math.max(yMax - yMin, 1e-10);

    const toCanvasX = (xi) => pad.left + ((xi - xMin) / xRange) * plotW;
    const toCanvasY = (yi) => pad.top + plotH - ((yi - yMin) / yRange) * plotH;

    ctx.strokeStyle = "#a855f7";
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    let started = false;
    for (let i = 0; i < n; i++) {
      const x = hasWl ? wavelengths[i] : i;
      const y = spectrum[i];
      if (!Number.isFinite(y)) continue;
      const cx = toCanvasX(x);
      const cy = toCanvasY(y);
      if (!started) {
        ctx.moveTo(cx, cy);
        started = true;
      } else {
        ctx.lineTo(cx, cy);
      }
    }
    ctx.stroke();

    
    if (hoveredIdx !== null && hoveredIdx >= 0 && hoveredIdx < n) {
      const x = hasWl ? wavelengths[hoveredIdx] : hoveredIdx;
      const y = spectrum[hoveredIdx];
      if (Number.isFinite(y)) {
        const cx = toCanvasX(x);
        const cy = toCanvasY(y);

        ctx.strokeStyle = "rgba(255,255,255,0.3)";
        ctx.lineWidth = 0.5;
        ctx.setLineDash([3, 3]);
        ctx.beginPath();
        ctx.moveTo(cx, pad.top);
        ctx.lineTo(cx, H - pad.bottom);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(pad.left, cy);
        ctx.lineTo(W - pad.right, cy);
        ctx.stroke();
        ctx.setLineDash([]);

        
        ctx.fillStyle = "#eab308";
        ctx.beginPath();
        ctx.arc(cx, cy, 4, 0, Math.PI * 2);
        ctx.fill();

        
        ctx.font = "10px 'JetBrains Mono', monospace";
        ctx.fillStyle = "#fafafa";
        const label = hasWl
          ? `${x.toFixed(4)} μm → ${y.toFixed(2)}`
          : `ch ${x} → ${y.toFixed(2)}`;
        const tw = ctx.measureText(label).width;
        const tx = Math.min(cx + 8, W - tw - 8);
        const ty = Math.max(cy - 8, 16);
        ctx.fillStyle = "rgba(0,0,0,0.75)";
        ctx.fillRect(tx - 3, ty - 11, tw + 6, 14);
        ctx.fillStyle = "#fafafa";
        ctx.fillText(label, tx, ty);
      }
    }

    
    ctx.font = "9px 'JetBrains Mono', monospace";
    ctx.fillStyle = "#71717a";
    ctx.textAlign = "right";
    for (let i = 0; i <= nGridY; i++) {
      const val = yMax - (i / nGridY) * yRange;
      const y = pad.top + (i / nGridY) * plotH;
      ctx.fillText(val.toFixed(1), pad.left - 4, y + 3);
    }

    
    ctx.textAlign = "center";
    ctx.fillStyle = "#52525b";
    ctx.fillText(xLabel, W / 2, H - 4);
  }, [spectrum, wavelengths, xMin, xMax, yMin, yMax, xLabel, hoveredIdx]);

  useEffect(() => {
    draw();
  }, [draw]);

  
  useEffect(() => {
    const c = containerRef.current;
    if (!c) return;
    const ro = new ResizeObserver(() => draw());
    ro.observe(c);
    return () => ro.disconnect();
  }, [draw]);

  
  const handleMouseMove = useCallback(
    (e) => {
      const canvas = canvasRef.current;
      if (!canvas || spectrum.length === 0) return;
      const rect = canvas.getBoundingClientRect();
      const pad = { left: 50, right: 12 };
      const plotW = rect.width - pad.left - pad.right;
      const relX = e.clientX - rect.left - pad.left;
      const frac = relX / plotW;
      const idx = Math.round(frac * (spectrum.length - 1));
      setHoveredIdx(Math.max(0, Math.min(spectrum.length - 1, idx)));
    },
    [spectrum]
  );

  const handleMouseLeave = useCallback(() => setHoveredIdx(null), []);

  if (spectrum.length === 0 && !isLoading) {
    return (
      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4 flex flex-col items-center gap-2 text-zinc-600">
        <Activity size={24} strokeWidth={1.5} />
        <p className="text-xs">Click on the preview image to extract a spectrum</p>
      </div>
    );
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2 }}
      className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider flex items-center gap-1.5">
          <Activity size={12} />
          Spectroscopy
        </h4>
        <div className="flex items-center gap-2 text-[10px] font-mono text-zinc-500">
          {isLoading && <Loader2 size={12} className="animate-spin text-purple-400" />}
          {pixelCoord && (
            <span className="flex items-center gap-1">
              <Crosshair size={10} />
              ({pixelCoord.x}, {pixelCoord.y})
            </span>
          )}
          {elapsed > 0 && <span>{elapsed}ms</span>}
        </div>
      </div>

      {/* Canvas */}
      <div ref={containerRef} className="px-2 pt-2 pb-2">
        {isLoading ? (
          <div
            className="flex items-center justify-center bg-zinc-950 rounded"
            style={{ height: CANVAS_H }}
          >
            <Loader2 size={24} className="animate-spin text-purple-400" />
          </div>
        ) : (
          <canvas
            ref={canvasRef}
            height={CANVAS_H}
            className="w-full rounded cursor-crosshair"
            style={{ height: CANVAS_H }}
            onMouseMove={handleMouseMove}
            onMouseLeave={handleMouseLeave}
          />
        )}
      </div>

      {/* Info bar */}
      {cubeDims && (
        <div className="px-3 pb-2 text-[10px] font-mono text-zinc-600">
          Cube: {cubeDims.naxis1}×{cubeDims.naxis2}×{cubeDims.naxis3} — {spectrum.length} channels
        </div>
      )}
    </motion.div>
  );
}
