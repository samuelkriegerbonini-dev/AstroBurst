import { useState, useRef, useEffect, useCallback } from "react";
import { useBackend } from "../../hooks/useBackend";

interface PsfResult {
  kernel: number[][];
  kernel_size: number;
  average_fwhm: number;
  average_ellipticity: number;
  stars_used: { x: number; y: number; fwhm: number; snr: number; ellipticity: number; peak: number; flux: number }[];
  stars_rejected: number;
  spread_pixels: number;
}

interface PsfPanelProps {
  selectedFile: { path: string; result?: any } | null;
  onPsfReady?: (kernel: number[][]) => void;
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onProcessingDone?: (result: any) => void;
  chainedFrom?: string;
}

export default function PsfPanel({ selectedFile, onPsfReady }: PsfPanelProps) {
  const { estimatePsf } = useBackend();
  const [result, setResult] = useState<PsfResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const [numStars, setNumStars] = useState(3);
  const [cutoutRadius, setCutoutRadius] = useState(15);
  const [maxEllipticity, setMaxEllipticity] = useState(0.3);
  const [satThreshold, setSatThreshold] = useState(0.95);

  const handleEstimate = useCallback(async () => {
    if (!selectedFile?.path) return;
    setLoading(true);
    setError(null);
    try {
      const res = await estimatePsf(selectedFile.path, {
        numStars,
        cutoutRadius,
        maxEllipticity,
        saturationThreshold: satThreshold,
      }) as PsfResult;
      setResult(res);
      if (res && onPsfReady) {
        onPsfReady(res.kernel);
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, [selectedFile, numStars, cutoutRadius, maxEllipticity, satThreshold, estimatePsf, onPsfReady]);

  useEffect(() => {
    if (!result || !canvasRef.current) return;
    const canvas = canvasRef.current;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const size = result.kernel_size;
    canvas.width = size;
    canvas.height = size;

    const flat = result.kernel.flat();
    const maxVal = Math.max(...flat);
    if (maxVal === 0) return;

    const imgData = ctx.createImageData(size, size);
    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        const v = Math.floor((result.kernel[y][x] / maxVal) * 255);
        const idx = (y * size + x) * 4;
        imgData.data[idx] = v;
        imgData.data[idx + 1] = v;
        imgData.data[idx + 2] = v;
        imgData.data[idx + 3] = 255;
      }
    }
    ctx.putImageData(imgData, 0, 0);
  }, [result]);

  return (
    <div className="flex flex-col gap-3 p-4">
      <div className="flex items-center gap-2 mb-1">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-violet-400">
          <circle cx="12" cy="12" r="3" />
          <circle cx="12" cy="12" r="8" opacity="0.3" />
          <circle cx="12" cy="12" r="11" opacity="0.1" />
        </svg>
        <span className="text-sm font-semibold text-zinc-200 tracking-wide">
          PSF Estimation
        </span>
        <span className="text-[10px] text-zinc-500 ml-auto">Empirical</span>
      </div>

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">
          Select a FITS file to estimate PSF.
        </div>
      )}

      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-1">
          <div className="flex justify-between items-center">
            <label className="text-xs text-zinc-400">Stars to sample</label>
            <span className="text-xs font-mono text-zinc-300 bg-zinc-800 px-1.5 py-0.5 rounded">{numStars}</span>
          </div>
          <input
            type="range" min={1} max={10} step={1} value={numStars}
            onChange={(e) => setNumStars(Number(e.target.value))}
            disabled={loading}
            className="w-full h-1.5 bg-zinc-700 rounded-full appearance-none cursor-pointer accent-violet-500
              [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3
              [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-violet-400
              [&::-webkit-slider-thumb]:appearance-none"
          />
        </div>

        <div className="flex flex-col gap-1">
          <div className="flex justify-between items-center">
            <label className="text-xs text-zinc-400">Cutout radius</label>
            <span className="text-xs font-mono text-zinc-300 bg-zinc-800 px-1.5 py-0.5 rounded">{cutoutRadius}px</span>
          </div>
          <input
            type="range" min={5} max={50} step={1} value={cutoutRadius}
            onChange={(e) => setCutoutRadius(Number(e.target.value))}
            disabled={loading}
            className="w-full h-1.5 bg-zinc-700 rounded-full appearance-none cursor-pointer accent-violet-500
              [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3
              [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-violet-400
              [&::-webkit-slider-thumb]:appearance-none"
          />
        </div>

        <div className="flex flex-col gap-1">
          <div className="flex justify-between items-center">
            <label className="text-xs text-zinc-400">Max ellipticity</label>
            <span className="text-xs font-mono text-zinc-300 bg-zinc-800 px-1.5 py-0.5 rounded">{maxEllipticity.toFixed(2)}</span>
          </div>
          <input
            type="range" min={0.05} max={1.0} step={0.05} value={maxEllipticity}
            onChange={(e) => setMaxEllipticity(Number(e.target.value))}
            disabled={loading}
            className="w-full h-1.5 bg-zinc-700 rounded-full appearance-none cursor-pointer accent-violet-500
              [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3
              [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-violet-400
              [&::-webkit-slider-thumb]:appearance-none"
          />
        </div>

        <div className="flex flex-col gap-1">
          <div className="flex justify-between items-center">
            <label className="text-xs text-zinc-400">Saturation threshold</label>
            <span className="text-xs font-mono text-zinc-300 bg-zinc-800 px-1.5 py-0.5 rounded">{satThreshold.toFixed(2)}</span>
          </div>
          <input
            type="range" min={0.5} max={1.0} step={0.05} value={satThreshold}
            onChange={(e) => setSatThreshold(Number(e.target.value))}
            disabled={loading}
            className="w-full h-1.5 bg-zinc-700 rounded-full appearance-none cursor-pointer accent-violet-500
              [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3
              [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-violet-400
              [&::-webkit-slider-thumb]:appearance-none"
          />
        </div>
      </div>

      <button
        onClick={handleEstimate}
        disabled={loading || !selectedFile}
        className={`w-full py-2 rounded-lg text-sm font-medium transition-all duration-200
          ${loading
            ? "bg-zinc-700 text-zinc-400 cursor-wait"
            : selectedFile
              ? "bg-violet-600 hover:bg-violet-500 text-white shadow-lg shadow-violet-900/30 active:scale-[0.98]"
              : "bg-zinc-700 text-zinc-500 cursor-not-allowed"
          }`}
      >
        {loading ? (
          <span className="flex items-center justify-center gap-2">
            <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            Estimating...
          </span>
        ) : "Estimate PSF"}
      </button>

      {error && (
        <div className="text-xs text-red-400 bg-red-900/20 border border-red-800/30 rounded-lg px-3 py-2">
          {error}
        </div>
      )}

      {result && (
        <div className="flex flex-col gap-3">
          <div className="flex items-center gap-3 bg-zinc-800/40 rounded-lg p-3">
            <canvas
              ref={canvasRef}
              className="rounded border border-zinc-700 flex-shrink-0"
              style={{ width: 100, height: 100, imageRendering: "pixelated" }}
            />
            <div className="flex flex-col gap-1.5">
              <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
                <div className="text-zinc-500">FWHM</div>
                <div className="text-zinc-200 font-mono">{result.average_fwhm.toFixed(2)} px</div>
                <div className="text-zinc-500">Ellipticity</div>
                <div className="text-zinc-200 font-mono">{result.average_ellipticity.toFixed(3)}</div>
                <div className="text-zinc-500">Spread</div>
                <div className="text-zinc-200 font-mono">{result.spread_pixels.toFixed(2)} px</div>
                <div className="text-zinc-500">Stars</div>
                <div className="text-zinc-200 font-mono">{result.stars_used.length} / {result.stars_used.length + result.stars_rejected}</div>
              </div>
            </div>
          </div>

          <div className="flex flex-col gap-0.5">
            {result.stars_used.map((s, i) => (
              <div key={i} className="flex items-center gap-2 text-[10px] text-zinc-500 font-mono px-1">
                <span className="text-violet-400/60">#{i + 1}</span>
                <span>({s.x.toFixed(0)},{s.y.toFixed(0)})</span>
                <span>FWHM={s.fwhm.toFixed(1)}</span>
                <span>SNR={s.snr.toFixed(0)}</span>
                <span>e={s.ellipticity.toFixed(2)}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
