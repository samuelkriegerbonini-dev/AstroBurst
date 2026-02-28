import { useState, useEffect, useRef, useCallback } from "react";
import { motion } from "framer-motion";
import { Activity, Loader2 } from "lucide-react";

export default function FFTPanel({ filePath, computeFftSpectrum }) {
  const canvasRef = useRef(null);
  const containerRef = useRef(null);
  const [fftData, setFftData] = useState(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(null);
  const [hoveredCoord, setHoveredCoord] = useState(null);

  const handleCompute = useCallback(async () => {
    if (!filePath || !computeFftSpectrum) return;
    setLoading(true);
    setError(null);
    try {
      const result = await computeFftSpectrum(filePath);
      setFftData(result);
    } catch (e) {
      console.error("FFT computation failed:", e);
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [filePath, computeFftSpectrum]);

  useEffect(() => {
    setFftData(null);
    setError(null);
  }, [filePath]);

  useEffect(() => {
    if (!fftData || !canvasRef.current) return;

    const canvas = canvasRef.current;
    const { width, height, pixels_b64 } = fftData;

    const binary = atob(pixels_b64);
    const pixels = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      pixels[i] = binary.charCodeAt(i);
    }

    const displayW = Math.min(width, 512);
    const displayH = Math.min(height, 512);
    canvas.width = displayW;
    canvas.height = displayH;

    const ctx = canvas.getContext("2d");
    const imgData = ctx.createImageData(displayW, displayH);

    const scaleX = width / displayW;
    const scaleY = height / displayH;

    for (let dy = 0; dy < displayH; dy++) {
      const sy = Math.floor(dy * scaleY);
      for (let dx = 0; dx < displayW; dx++) {
        const sx = Math.floor(dx * scaleX);
        const srcIdx = sy * width + sx;
        const v = pixels[srcIdx] || 0;

        const dstIdx = (dy * displayW + dx) * 4;
        const r = v < 128 ? 0 : Math.floor(((v - 128) / 127) * 255);
        const g = v < 128 ? Math.floor((v / 128) * 255) : 255 - Math.floor(((v - 128) / 127) * 128);
        const b = v < 128 ? Math.floor((v / 128) * 200) : Math.max(0, 200 - Math.floor(((v - 128) / 127) * 200));

        imgData.data[dstIdx] = r;
        imgData.data[dstIdx + 1] = g;
        imgData.data[dstIdx + 2] = b;
        imgData.data[dstIdx + 3] = 255;
      }
    }

    ctx.putImageData(imgData, 0, 0);
  }, [fftData]);

  const handleMouseMove = useCallback(
      (e) => {
        if (!fftData || !canvasRef.current) return;
        const rect = canvasRef.current.getBoundingClientRect();
        const x = ((e.clientX - rect.left) / rect.width) * fftData.width;
        const y = ((e.clientY - rect.top) / rect.height) * fftData.height;
        const fx = (x - fftData.width / 2) / fftData.width;
        const fy = (y - fftData.height / 2) / fftData.height;
        setHoveredCoord({ fx: fx.toFixed(3), fy: fy.toFixed(3) });
      },
      [fftData]
  );

  const handleMouseLeave = useCallback(() => setHoveredCoord(null), []);

  return (
      <motion.div
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.2 }}
          className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden"
      >
        <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
          <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider flex items-center gap-1.5">
            <Activity size={12} className="text-cyan-400" />
            FFT Power Spectrum
          </h4>
          <button
              onClick={handleCompute}
              disabled={loading || !filePath}
              className="flex items-center gap-1 text-xs text-cyan-400 hover:text-cyan-300 px-2 py-1 rounded hover:bg-zinc-800 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          >
            {loading ? <Loader2 size={12} className="animate-spin" /> : <Activity size={12} />}
            {loading ? "Computing…" : fftData ? "Recompute" : "Analyze"}
          </button>
        </div>

        {error && (
            <div className="px-3 py-2 text-xs text-red-400 bg-red-950/20">{error}</div>
        )}

        {!fftData && !loading && !error && (
            <div className="px-3 py-4 text-center text-xs text-zinc-600">
              <p>Compute 2D FFT to diagnose noise patterns and image quality.</p>
              <p className="mt-1 text-zinc-700">Bright center = DC component · Geometric lines = electronic noise · Spread = fine detail</p>
            </div>
        )}

        {fftData && (
            <>
              <div ref={containerRef} className="px-2 pt-2 flex justify-center">
                <canvas
                    ref={canvasRef}
                    className="rounded border border-zinc-800/50 cursor-crosshair"
                    style={{ maxWidth: "100%", imageRendering: "pixelated" }}
                    onMouseMove={handleMouseMove}
                    onMouseLeave={handleMouseLeave}
                />
              </div>

              <div className="flex items-center justify-between px-3 py-2 text-[10px] font-mono text-zinc-500">
                <span>{fftData.width}×{fftData.height} px</span>
                {hoveredCoord && (
                    <span className="text-cyan-400">
                freq: ({hoveredCoord.fx}, {hoveredCoord.fy})
              </span>
                )}
                <span className="text-zinc-600">{fftData.elapsed_ms} ms</span>
              </div>

              <div className="flex items-center gap-3 px-3 pb-2 text-[10px] font-mono text-zinc-500">
                <span>DC={fftData.dc_magnitude?.toExponential(2)}</span>
                <span>max={fftData.max_magnitude?.toExponential(2)}</span>
              </div>
            </>
        )}
      </motion.div>
  );
}