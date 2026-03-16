import { useState, useRef, useEffect, useCallback } from "react";
import { estimatePsf } from "../../services/processing.service";
import { Slider, RunButton, ErrorAlert, SectionHeader } from "../ui";

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

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-violet-400">
    <circle cx="12" cy="12" r="3" />
    <circle cx="12" cy="12" r="8" opacity="0.3" />
    <circle cx="12" cy="12" r="11" opacity="0.1" />
  </svg>
);

export default function PsfPanel({ selectedFile, onPsfReady }: PsfPanelProps) {
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
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [selectedFile, numStars, cutoutRadius, maxEllipticity, satThreshold, onPsfReady]);

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
      <SectionHeader icon={ICON} title="PSF Estimation" subtitle="Empirical" />

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to estimate PSF.</div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Stars to sample" value={numStars} min={1} max={10} step={1} disabled={loading} accent="violet" onChange={setNumStars} />
        <Slider label="Cutout radius" value={cutoutRadius} min={5} max={50} step={1} disabled={loading} accent="violet" format={(v) => `${v}px`} onChange={setCutoutRadius} />
        <Slider label="Max ellipticity" value={maxEllipticity} min={0.05} max={1} step={0.05} disabled={loading} accent="violet" format={(v) => v.toFixed(2)} onChange={setMaxEllipticity} />
        <Slider label="Saturation threshold" value={satThreshold} min={0.5} max={1} step={0.05} disabled={loading} accent="violet" format={(v) => v.toFixed(2)} onChange={setSatThreshold} />
      </div>

      <RunButton label="Estimate PSF" runningLabel="Estimating..." running={loading} disabled={!selectedFile} accent="violet" onClick={handleEstimate} />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          <div className="flex items-center gap-3 ab-metric-card p-3">
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
