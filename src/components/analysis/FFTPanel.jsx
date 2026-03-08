import { useState, useEffect, useRef, useCallback } from "react";
import { Activity, Loader2 } from "lucide-react";

export default function FFTPanel({ filePath, computeFftSpectrum }) {
  const containerRef = useRef(null);
  const canvasRef = useRef(null);
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
      const px = result.pixels;
      let nonZero = 0;
      let maxVal = 0;
      for (let i = 0; i < px.length; i++) {
        if (px[i] > 0) nonZero++;
        if (px[i] > maxVal) maxVal = px[i];
      }
      const centerIdx = Math.floor(result.height / 2) * result.width + Math.floor(result.width / 2);
      console.log("FFT diag:", {
        w: result.width,
        h: result.height,
        pixLen: px.length,
        nonZero,
        maxVal,
        center: px[centerIdx],
        dc: result.dc_magnitude,
        maxMag: result.max_magnitude,
        elapsed: result.elapsed_ms,
      });
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
    if (!fftData?.pixels || !canvasRef.current) return;

    const { width, height, pixels } = fftData;
    const canvas = canvasRef.current;
    canvas.width = width;
    canvas.height = height;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const imageData = ctx.createImageData(width, height);
    const rgba = imageData.data;
    const len = width * height;

    for (let i = 0; i < len; i++) {
      const v = pixels[i];
      const off = i << 2;
      rgba[off] = v;
      rgba[off | 1] = v;
      rgba[off | 2] = v;
      rgba[off | 3] = 255;
    }

    ctx.putImageData(imageData, 0, 0);
  }, [fftData]);

  const handleMouseMove = useCallback(
    (e) => {
      if (!fftData) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const x = ((e.clientX - rect.left) / rect.width) * fftData.width;
      const y = ((e.clientY - rect.top) / rect.height) * fftData.height;
      const fx = (x - fftData.width / 2) / fftData.width;
      const fy = (y - fftData.height / 2) / fftData.height;
      setHoveredCoord({ fx: fx.toFixed(3), fy: fy.toFixed(3) });
    },
    [fftData]
  );

  const handleMouseLeave = useCallback(() => setHoveredCoord(null), []);

  const hasPixels = fftData?.pixels && fftData.width > 0 && fftData.height > 0;

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden animate-fade-in">
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
          {loading ? "Computing..." : fftData ? "Recompute" : "Analyze"}
        </button>
      </div>

      {error && (
        <div className="px-3 py-2 text-xs text-red-400 bg-red-950/20">{error}</div>
      )}

      {!fftData && !loading && !error && (
        <div className="px-3 py-4 text-center text-xs text-zinc-600">
          <p>Compute 2D FFT to diagnose noise patterns and image quality.</p>
          <p className="mt-1 text-zinc-700">
            Bright center = DC component | Geometric lines = electronic noise | Spread = fine
            detail
          </p>
        </div>
      )}

      {fftData && hasPixels && (
        <>
          <div ref={containerRef} className="px-2 pt-2 flex justify-center">
            <canvas
              ref={canvasRef}
              className="rounded border border-zinc-800/50 cursor-crosshair"
              style={{ maxWidth: "100%", maxHeight: 512, imageRendering: "pixelated" }}
              onMouseMove={handleMouseMove}
              onMouseLeave={handleMouseLeave}
            />
          </div>

          <div className="flex items-center justify-between px-3 py-2 text-[10px] font-mono text-zinc-500">
            <span>
              {fftData.width}x{fftData.height} px
            </span>
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
    </div>
  );
}
