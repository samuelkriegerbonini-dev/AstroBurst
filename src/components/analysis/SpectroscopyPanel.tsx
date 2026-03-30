import { useState, useRef, useCallback, useEffect, useMemo } from "react";
import { Activity, Crosshair, Layers } from "lucide-react";
import CubeFrameNav from "../CubeFrameNav";
import { processCube, processCubeLazy } from "../../services/cube";
import { getOutputDir } from "../../infrastructure/tauri";

interface SpectroscopyPanelProps {
  spectrum?: number[];
  wavelengths?: number[] | null;
  pixelCoord?: { x: number; y: number } | null;
  isLoading?: boolean;
  cubeDims?: { naxis1: number; naxis2: number; naxis3: number; width?: number; height?: number; frames?: number } | null;
  elapsed?: number;
  filePath?: string;
  onFramePreview?: (previewUrl: string, frameIndex: number) => void;
  onCollapsePreview?: (previewUrl: string) => void;
}

function arrayMinMax(arr: number[]): [number, number] {
  let min = Infinity;
  let max = -Infinity;
  for (let i = 0; i < arr.length; i++) {
    const v = arr[i];
    if (Number.isFinite(v)) {
      if (v < min) min = v;
      if (v > max) max = v;
    }
  }
  return [min === Infinity ? 0 : min, max === -Infinity ? 1 : max];
}

const CANVAS_H = 180;
const PAD = { top: 10, bottom: 24, left: 50, right: 12 } as const;
const N_GRID_Y = 4;

export default function SpectroscopyPanel({
                                            spectrum = [],
                                            wavelengths = null,
                                            pixelCoord = null,
                                            isLoading = false,
                                            cubeDims = null,
                                            elapsed = 0,
                                            filePath,
                                            onFramePreview,
                                            onCollapsePreview,
                                          }: SpectroscopyPanelProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [hoveredIdx, setHoveredIdx] = useState<number | null>(null);
  const [collapseLoading, setCollapseLoading] = useState(false);
  const [collapseResult, setCollapseResult] = useState<any>(null);
  const [collapseMode, setCollapseMode] = useState<"sum" | "median">("sum");

  const wlUnit = useMemo(() => {
    const raw = (cubeDims as any)?.spectral_classification?.axis_unit as string | null;
    if (!raw) return null;
    return raw.trim().toUpperCase();
  }, [cubeDims]);

  const wlConversion = useMemo<{ factor: number; label: string }>(() => {
    if (!wlUnit) return { factor: 1, label: "Channel" };
    if (wlUnit === "M") return { factor: 1e6, label: "\u03bcm" };
    if (wlUnit === "CM") return { factor: 1e4, label: "\u03bcm" };
    if (wlUnit === "MM") return { factor: 1e3, label: "\u03bcm" };
    if (wlUnit === "UM") return { factor: 1, label: "\u03bcm" };
    if (wlUnit === "NM") return { factor: 1, label: "nm" };
    if (wlUnit === "ANGSTROM" || wlUnit === "A") return { factor: 0.1, label: "nm" };
    if (wlUnit === "HZ") return { factor: 1e-9, label: "GHz" };
    if (wlUnit === "KHZ") return { factor: 1e-6, label: "GHz" };
    if (wlUnit === "MHZ") return { factor: 1e-3, label: "GHz" };
    if (wlUnit === "GHZ") return { factor: 1, label: "GHz" };
    if (wlUnit === "M/S" || wlUnit === "KM/S") return { factor: 1, label: wlUnit.toLowerCase() };
    return { factor: 1, label: wlUnit.toLowerCase() };
  }, [wlUnit]);

  const plotParams = useMemo(() => {
    if (!spectrum || spectrum.length === 0)
      return { xMin: 0, xMax: 1, yMin: 0, yMax: 1, xLabel: "Channel", hasWl: false };

    const n = spectrum.length;
    const hasWl = !!(wavelengths && wavelengths.length === n);

    let xMin: number, xMax: number;
    if (hasWl) {
      const converted = wavelengths!.map((w) => w * wlConversion.factor);
      [xMin, xMax] = arrayMinMax(converted);
    } else {
      xMin = 0;
      xMax = n - 1;
    }

    const [rawYMin, rawYMax] = arrayMinMax(spectrum);
    const yPad = (rawYMax - rawYMin) * 0.05;

    return {
      xMin,
      xMax,
      yMin: rawYMin - yPad,
      yMax: rawYMax + yPad,
      xLabel: hasWl ? `Wavelength (${wlConversion.label})` : "Channel",
      hasWl,
    };
  }, [spectrum, wavelengths, wlConversion]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || spectrum.length === 0) return;

    const parent = canvas.parentElement;
    if (!parent) return;
    const W = Math.floor(parent.getBoundingClientRect().width);
    const H = CANVAS_H;
    canvas.width = W;
    canvas.height = H;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.fillStyle = "#0a0a0f";
    ctx.fillRect(0, 0, W, H);

    const plotW = W - PAD.left - PAD.right;
    const plotH = H - PAD.top - PAD.bottom;
    const { xMin, xMax, yMin, yMax, xLabel, hasWl } = plotParams;
    const xRange = Math.max(xMax - xMin, 1e-10);
    const yRange = Math.max(yMax - yMin, 1e-10);

    const toX = (xi: number) => PAD.left + ((xi - xMin) / xRange) * plotW;
    const toY = (yi: number) => PAD.top + plotH - ((yi - yMin) / yRange) * plotH;

    ctx.strokeStyle = "#1f1f28";
    ctx.lineWidth = 0.5;
    for (let i = 0; i <= N_GRID_Y; i++) {
      const y = PAD.top + (i / N_GRID_Y) * plotH;
      ctx.beginPath();
      ctx.moveTo(PAD.left, y);
      ctx.lineTo(W - PAD.right, y);
      ctx.stroke();
    }

    const n = spectrum.length;
    const cf = wlConversion.factor;

    ctx.strokeStyle = "#a855f7";
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    let started = false;
    for (let i = 0; i < n; i++) {
      const x = hasWl ? wavelengths![i] * cf : i;
      const y = spectrum[i];
      if (!Number.isFinite(y)) continue;
      const cx = toX(x);
      const cy = toY(y);
      if (!started) { ctx.moveTo(cx, cy); started = true; }
      else ctx.lineTo(cx, cy);
    }
    ctx.stroke();

    if (hoveredIdx !== null && hoveredIdx >= 0 && hoveredIdx < n) {
      const x = hasWl ? wavelengths![hoveredIdx] * cf : hoveredIdx;
      const y = spectrum[hoveredIdx];
      if (Number.isFinite(y)) {
        const cx = toX(x);
        const cy = toY(y);

        ctx.strokeStyle = "rgba(255,255,255,0.3)";
        ctx.lineWidth = 0.5;
        ctx.setLineDash([3, 3]);
        ctx.beginPath();
        ctx.moveTo(cx, PAD.top);
        ctx.lineTo(cx, H - PAD.bottom);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(PAD.left, cy);
        ctx.lineTo(W - PAD.right, cy);
        ctx.stroke();
        ctx.setLineDash([]);

        ctx.fillStyle = "#eab308";
        ctx.beginPath();
        ctx.arc(cx, cy, 4, 0, Math.PI * 2);
        ctx.fill();

        ctx.font = "10px 'JetBrains Mono', monospace";
        const label = hasWl
          ? `${x.toFixed(4)} ${wlConversion.label} \u2192 ${y.toFixed(2)}`
          : `ch ${x} \u2192 ${y.toFixed(2)}`;
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
    for (let i = 0; i <= N_GRID_Y; i++) {
      const val = yMax - (i / N_GRID_Y) * yRange;
      const y = PAD.top + (i / N_GRID_Y) * plotH;
      ctx.fillText(val.toFixed(1), PAD.left - 4, y + 3);
    }

    ctx.textAlign = "center";
    ctx.fillStyle = "#52525b";
    ctx.fillText(xLabel, W / 2, H - 4);
  }, [spectrum, wavelengths, plotParams, hoveredIdx, wlConversion]);

  useEffect(() => { draw(); }, [draw]);

  useEffect(() => {
    const c = containerRef.current;
    if (!c) return;
    const ro = new ResizeObserver(() => draw());
    ro.observe(c);
    return () => ro.disconnect();
  }, [draw]);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      if (!canvas || spectrum.length === 0) return;
      const rect = canvas.getBoundingClientRect();
      const plotW = rect.width - PAD.left - PAD.right;
      const relX = e.clientX - rect.left - PAD.left;
      const frac = relX / plotW;
      const idx = Math.round(frac * (spectrum.length - 1));
      setHoveredIdx(Math.max(0, Math.min(spectrum.length - 1, idx)));
    },
    [spectrum.length],
  );

  const handleMouseLeave = useCallback(() => setHoveredIdx(null), []);

  const handleCollapse = useCallback(async (mode: "sum" | "median") => {
    if (!filePath) return;
    setCollapseLoading(true);
    setCollapseMode(mode);
    setCollapseResult(null);
    try {
      const dir = await getOutputDir();
      const result = mode === "sum"
        ? await processCube(filePath, dir, 1)
        : await processCubeLazy(filePath, dir, 1);
      setCollapseResult(result);
      const url = result.collapsedPreviewUrl || result.collapsedMedianPreviewUrl;
      if (url && onCollapsePreview) onCollapsePreview(url);
    } catch (e) {
      console.error("Cube collapse failed:", e);
    } finally {
      setCollapseLoading(false);
    }
  }, [filePath, onCollapsePreview]);

  if (spectrum.length === 0 && !isLoading) {
    return (
      <div className="ab-panel p-6 flex flex-col items-center gap-3">
        <div
          className="w-10 h-10 rounded-xl flex items-center justify-center"
          style={{ background: "rgba(168,85,247,0.06)", border: "1px solid rgba(168,85,247,0.1)" }}
        >
          <Activity size={18} style={{ color: "var(--ab-violet)", opacity: 0.5 }} />
        </div>
        <p className="text-[11px] text-zinc-500">Click on the preview image to extract a spectrum</p>
      </div>
    );
  }

  const totalFrames = cubeDims ? (cubeDims.naxis3 ?? cubeDims.frames ?? 0) : 0;

  return (
    <div className="ab-panel overflow-hidden animate-fade-in">
      <div className="ab-panel-header">
        <div className="flex items-center gap-1.5">
          <Activity size={12} style={{ color: "var(--ab-violet)" }} />
          <span className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">
            Spectroscopy
          </span>
        </div>
        <div className="flex items-center gap-2 text-[10px] font-mono text-zinc-500">
          {isLoading && (
            <div
              className="w-3 h-3 rounded-full animate-spin"
              style={{ border: "1.5px solid transparent", borderTopColor: "var(--ab-violet)" }}
            />
          )}
          {pixelCoord && (
            <span className="flex items-center gap-1">
              <Crosshair size={10} />
              ({pixelCoord.x}, {pixelCoord.y})
            </span>
          )}
          {elapsed > 0 && <span>{elapsed}ms</span>}
        </div>
      </div>

      <div ref={containerRef} className="p-2">
        {isLoading ? (
          <div
            className="flex items-center justify-center rounded-md"
            style={{ height: CANVAS_H, background: "rgba(9,9,11,0.8)" }}
          >
            <div
              className="w-5 h-5 rounded-full animate-spin"
              style={{ border: "2px solid transparent", borderTopColor: "var(--ab-violet)" }}
            />
          </div>
        ) : (
          <canvas
            ref={canvasRef}
            height={CANVAS_H}
            className="w-full rounded-md cursor-crosshair"
            style={{ height: CANVAS_H }}
            onMouseMove={handleMouseMove}
            onMouseLeave={handleMouseLeave}
          />
        )}
      </div>

      {cubeDims && (
        <div className="px-3 pb-1 text-[10px] font-mono text-zinc-600">
          Cube: {cubeDims.naxis1 ?? cubeDims.width} \u00d7 {cubeDims.naxis2 ?? cubeDims.height} \u00d7 {cubeDims.naxis3 ?? cubeDims.frames}
          {spectrum.length > 0 ? ` \u2014 ${spectrum.length} channels` : ""}
        </div>
      )}

      {filePath && cubeDims && totalFrames > 1 && (
        <div className="px-3 pb-2 flex items-center gap-2" style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}>
          <CollapseBtn
            label="Collapse Mean"
            loading={collapseLoading && collapseMode === "sum"}
            disabled={collapseLoading}
            color="var(--ab-violet)"
            onClick={() => handleCollapse("sum")}
          />
          <CollapseBtn
            label="Collapse Median"
            loading={collapseLoading && collapseMode === "median"}
            disabled={collapseLoading}
            color="var(--ab-amber)"
            onClick={() => handleCollapse("median")}
          />
          {collapseResult && !collapseLoading && (
            <span className="text-[10px] font-mono text-zinc-500 ml-auto">
              {collapseResult.elapsed_ms ?? collapseResult.elapsed}ms
            </span>
          )}
        </div>
      )}

      {filePath && cubeDims && totalFrames > 1 && (
        <div className="px-2 pb-2">
          <CubeFrameNav
            filePath={filePath}
            totalFrames={totalFrames}
            onFrameChange={onFramePreview}
          />
        </div>
      )}
    </div>
  );
}

function CollapseBtn({
                       label,
                       loading,
                       disabled,
                       color,
                       onClick,
                     }: {
  label: string;
  loading: boolean;
  disabled: boolean;
  color: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all disabled:opacity-40"
      style={{
        background: `color-mix(in srgb, ${color} 8%, transparent)`,
        border: `1px solid color-mix(in srgb, ${color} 20%, transparent)`,
        color,
      }}
    >
      {loading ? (
        <div
          className="w-2.5 h-2.5 rounded-full animate-spin"
          style={{ border: "1.5px solid transparent", borderTopColor: color }}
        />
      ) : (
        <Layers size={10} />
      )}
      {label}
    </button>
  );
}
