import { useState, useEffect, useRef, useCallback, memo } from "react";
import { Activity } from "lucide-react";
import type { FftData } from "../../shared/types";

interface FftDataExtended extends FftData {
  original_size?: number;
  windowed?: boolean;
}

interface HoveredCoord {
  fx: string;
  fy: string;
}

interface FFTPanelProps {
  filePath: string | null;
  computeFftSpectrum: (path: string) => Promise<FftDataExtended>;
}

function FFTPanel({ filePath, computeFftSpectrum }: FFTPanelProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [fftData, setFftData] = useState<FftDataExtended | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hoveredCoord, setHoveredCoord] = useState<HoveredCoord | null>(null);

  const handleCompute = useCallback(async () => {
    if (!filePath || !computeFftSpectrum) return;
    setLoading(true);
    setError(null);
    try {
      setFftData(await computeFftSpectrum(filePath));
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
    const u32 = new Uint32Array(imageData.data.buffer);
    const len = width * height;

    for (let i = 0; i < len; i++) {
      const v = pixels[i];
      u32[i] = (255 << 24) | (v << 16) | (v << 8) | v;
    }

    ctx.putImageData(imageData, 0, 0);
  }, [fftData]);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      if (!fftData) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const x = ((e.clientX - rect.left) / rect.width) * fftData.width;
      const y = ((e.clientY - rect.top) / rect.height) * fftData.height;
      const fx = (x - fftData.width / 2) / fftData.width;
      const fy = (y - fftData.height / 2) / fftData.height;
      setHoveredCoord({ fx: fx.toFixed(3), fy: fy.toFixed(3) });
    },
    [fftData],
  );

  const handleMouseLeave = useCallback(() => setHoveredCoord(null), []);

  const hasPixels = fftData?.pixels && fftData.width > 0 && fftData.height > 0;

  return (
    <div className="ab-panel overflow-hidden animate-fade-in">
      <div className="ab-panel-header">
        <div className="flex items-center gap-1.5">
          <Activity size={12} style={{ color: "var(--ab-cyan)" }} />
          <span className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">
            FFT Power Spectrum
          </span>
        </div>
        <button
          onClick={handleCompute}
          disabled={loading || !filePath}
          className="flex items-center gap-1 text-[10px] font-medium px-2.5 py-1 rounded-md transition-all disabled:opacity-40 disabled:cursor-not-allowed"
          style={{
            color: "var(--ab-cyan)",
            background: loading ? "transparent" : "rgba(34,211,238,0.08)",
            border: "1px solid rgba(34,211,238,0.15)",
          }}
        >
          {loading ? (
            <div
              className="w-3 h-3 rounded-full animate-spin"
              style={{ border: "1.5px solid transparent", borderTopColor: "var(--ab-cyan)" }}
            />
          ) : (
            <Activity size={10} />
          )}
          {loading ? "Computing..." : fftData ? "Recompute" : "Analyze"}
        </button>
      </div>

      {error && (
        <div className="ab-error-alert mx-3 mt-2">{error}</div>
      )}

      {!fftData && !loading && !error && (
        <div className="px-4 py-5 text-center">
          <p className="text-[11px] text-zinc-500">Compute 2D FFT to diagnose noise patterns and image quality.</p>
          <p className="mt-1.5 text-[10px] text-zinc-600 font-mono">
            Bright center = DC | Lines = electronic noise | Spread = fine detail
          </p>
        </div>
      )}

      {fftData && hasPixels && (
        <>
          <div className="p-2 flex justify-center">
            <canvas
              ref={canvasRef}
              className="rounded-md cursor-crosshair"
              style={{
                maxWidth: "100%",
                maxHeight: 512,
                imageRendering: "pixelated",
                border: "1px solid rgba(63,63,70,0.3)",
              }}
              onMouseMove={handleMouseMove}
              onMouseLeave={handleMouseLeave}
            />
          </div>

          <div
            className="flex items-center justify-between px-3 py-1.5 text-[10px] font-mono text-zinc-500"
            style={{ borderTop: "1px solid var(--ab-border)" }}
          >
            <span>
              {fftData.width}\u00d7{fftData.height}
              {fftData.original_size && fftData.original_size !== fftData.width && (
                <span className="text-zinc-600"> (from {fftData.original_size}\u00d7{fftData.original_size})</span>
              )}
            </span>
            {hoveredCoord && (
              <span style={{ color: "var(--ab-cyan)" }}>
                freq ({hoveredCoord.fx}, {hoveredCoord.fy})
              </span>
            )}
            <span className="text-zinc-600">{fftData.elapsed_ms}ms</span>
          </div>

          <div className="flex items-center gap-3 px-3 pb-2 text-[10px] font-mono text-zinc-600">
            <span>DC={fftData.dc_magnitude?.toExponential(2)}</span>
            <span>max={fftData.max_magnitude?.toExponential(2)}</span>
            {fftData.windowed && <span style={{ color: "rgba(34,211,238,0.5)" }}>Hann</span>}
            <span className="text-zinc-700">ln(1+|F|)</span>
          </div>
        </>
      )}
    </div>
  );
}

export default memo(FFTPanel);
