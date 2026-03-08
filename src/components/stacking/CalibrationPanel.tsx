import { useState, useCallback } from "react";
import { Loader2, Sun, Moon, Aperture, Play, CheckCircle2, AlertCircle } from "lucide-react";
import { useBackend } from "../../hooks/useBackend";
import type { ProcessedFile } from "../../utils/types";

interface CalibrationPanelProps {
  files: ProcessedFile[];
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onCalibrationDone?: (result: any) => void;
}

type FrameType = "bias" | "dark" | "flat";

interface FrameSelection {
  bias: string[];
  dark: string[];
  flat: string[];
}

const FRAME_META: Record<FrameType, { label: string; icon: typeof Sun; color: string; desc: string }> = {
  bias: { label: "Bias", icon: Aperture, color: "#a78bfa", desc: "Zero-exposure frames" },
  dark: { label: "Dark", icon: Moon, color: "#60a5fa", desc: "Same exposure, cap on" },
  flat: { label: "Flat", icon: Sun, color: "#fbbf24", desc: "Uniform light frames" },
};

export default function CalibrationPanel({ files = [], onPreviewUpdate, onCalibrationDone }: CalibrationPanelProps) {
  const { calibrate } = useBackend();
  const [sciencePath, setSciencePath] = useState<string | null>(null);
  const [frames, setFrames] = useState<FrameSelection>({ bias: [], dark: [], flat: [] });
  const [darkExposureRatio, setDarkExposureRatio] = useState(1.0);
  const [isCalibrating, setIsCalibrating] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);

  const toggleFrame = useCallback((type: FrameType, path: string) => {
    setFrames((prev) => {
      const arr = prev[type];
      const next = arr.includes(path) ? arr.filter((p) => p !== path) : [...arr, path];
      return { ...prev, [type]: next };
    });
  }, []);

  const handleCalibrate = useCallback(async () => {
    if (!sciencePath) return;
    setIsCalibrating(true);
    setError(null);
    setResult(null);
    try {
      const res = await calibrate(sciencePath, "./output", {
        biasPaths: frames.bias.length > 0 ? frames.bias : undefined,
        darkPaths: frames.dark.length > 0 ? frames.dark : undefined,
        flatPaths: frames.flat.length > 0 ? frames.flat : undefined,
        darkExposureRatio,
      });
      setResult(res);
      onPreviewUpdate?.(res?.previewUrl);
      onCalibrationDone?.(res);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setIsCalibrating(false);
    }
  }, [sciencePath, frames, darkExposureRatio, calibrate, onPreviewUpdate, onCalibrationDone]);

  const hasFrames = frames.bias.length > 0 || frames.dark.length > 0 || frames.flat.length > 0;

  return (
    <div className="flex flex-col gap-3">
      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4">
        <h4 className="text-xs font-semibold text-violet-400 uppercase tracking-wider mb-3">
          Science Frame
        </h4>
        <select
          value={sciencePath || ""}
          onChange={(e) => setSciencePath(e.target.value || null)}
          className="w-full bg-zinc-900 border border-zinc-700/50 rounded-md px-3 py-2 text-xs text-zinc-200 outline-none focus:border-violet-500/50"
        >
          <option value="">Select science frame...</option>
          {files.map((f) => (
            <option key={f.id} value={f.path}>
              {f.name}
            </option>
          ))}
        </select>
      </div>

      {(["bias", "dark", "flat"] as FrameType[]).map((type) => {
        const meta = FRAME_META[type];
        const Icon = meta.icon;
        const selected = frames[type];
        return (
          <div key={type} className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4">
            <div className="flex items-center justify-between mb-2">
              <h4 className="text-xs font-semibold uppercase tracking-wider flex items-center gap-1.5" style={{ color: meta.color }}>
                <Icon size={12} />
                {meta.label} Frames
                {selected.length > 0 && (
                  <span className="ml-1 text-zinc-500">({selected.length})</span>
                )}
              </h4>
              <span className="text-[10px] text-zinc-600">{meta.desc}</span>
            </div>
            <div className="flex flex-col gap-1 max-h-[120px] overflow-y-auto">
              {files.map((f) => {
                const isSelected = selected.includes(f.path);
                return (
                  <button
                    key={f.id}
                    onClick={() => toggleFrame(type, f.path)}
                    className={`flex items-center gap-2 px-2.5 py-1.5 rounded text-[11px] transition-all text-left ${
                      isSelected
                        ? "bg-zinc-800/80 text-zinc-200 ring-1"
                        : "text-zinc-500 hover:bg-zinc-800/40 hover:text-zinc-300"
                    }`}
                    style={isSelected ? { ringColor: meta.color + "40" } : undefined}
                  >
                    <span
                      className={`w-3 h-3 rounded-sm border flex items-center justify-center shrink-0 ${
                        isSelected ? "border-transparent" : "border-zinc-600"
                      }`}
                      style={isSelected ? { background: meta.color + "30", borderColor: meta.color } : undefined}
                    >
                      {isSelected && <CheckCircle2 size={10} style={{ color: meta.color }} />}
                    </span>
                    <span className="truncate">{f.name}</span>
                  </button>
                );
              })}
              {files.length === 0 && (
                <span className="text-[10px] text-zinc-600 py-2 text-center">
                  No processed files available
                </span>
              )}
            </div>
          </div>
        );
      })}

      {frames.dark.length > 0 && (
        <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4">
          <label className="text-[10px] text-zinc-400 font-medium block mb-1.5">
            Dark Exposure Ratio
          </label>
          <input
            type="range"
            min={0.1}
            max={3.0}
            step={0.1}
            value={darkExposureRatio}
            onChange={(e) => setDarkExposureRatio(parseFloat(e.target.value))}
            className="w-full accent-blue-500"
          />
          <div className="text-[10px] font-mono text-zinc-500 mt-1">{darkExposureRatio.toFixed(1)}x</div>
        </div>
      )}

      <button
        onClick={handleCalibrate}
        disabled={!sciencePath || !hasFrames || isCalibrating}
        className="flex items-center justify-center gap-2 rounded-lg px-4 py-2.5 text-sm font-medium transition-all disabled:opacity-30 disabled:cursor-not-allowed"
        style={{
          background: isCalibrating ? "rgba(168,85,247,0.1)" : "rgba(168,85,247,0.15)",
          color: "#c4b5fd",
          border: "1px solid rgba(168,85,247,0.25)",
        }}
      >
        {isCalibrating ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} />}
        {isCalibrating ? "Calibrating..." : "Calibrate"}
      </button>

      {error && (
        <div className="flex items-start gap-2 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 text-xs text-red-300">
          <AlertCircle size={14} className="shrink-0 mt-0.5" />
          {error}
        </div>
      )}

      {result && (
        <div className="bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium mb-1.5">
            <CheckCircle2 size={12} />
            Calibration Complete
          </div>
          <div className="text-[10px] font-mono text-zinc-400 space-y-0.5">
            <div>{result.dimensions?.[0]}x{result.dimensions?.[1]}</div>
            {result.has_bias && <div>Bias subtracted</div>}
            {result.has_dark && <div>Dark subtracted</div>}
            {result.has_flat && <div>Flat divided</div>}
            {result.fits_path && (
              <div className="text-emerald-400/70 mt-1">
                Output auto-injected into Stack tab
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
