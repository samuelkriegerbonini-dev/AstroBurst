import { useState, useCallback } from "react";
import { Sun, Moon, Aperture, CheckCircle2 } from "lucide-react";
import { Slider, RunButton, ErrorAlert, SectionHeader } from "../ui";
import { calibrate } from "../../services/stacking.service";
import type { ProcessedFile } from "../../shared/types";

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

const HEADER_ICON = <Aperture size={14} className="text-violet-400" />;

export default function CalibrationPanel({ files = [], onPreviewUpdate, onCalibrationDone }: CalibrationPanelProps) {
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
  }, [sciencePath, frames, darkExposureRatio, onPreviewUpdate, onCalibrationDone]);

  const hasFrames = frames.bias.length > 0 || frames.dark.length > 0 || frames.flat.length > 0;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={HEADER_ICON} title="Calibration" />

      <div className="flex flex-col gap-1.5">
        <span className="text-xs font-semibold text-violet-400 uppercase tracking-wider">Science Frame</span>
        <select
          value={sciencePath || ""}
          onChange={(e) => setSciencePath(e.target.value || null)}
          className="ab-select w-full"
        >
          <option value="">Select science frame...</option>
          {files.map((f) => (
            <option key={f.id} value={f.path}>{f.name}</option>
          ))}
        </select>
      </div>

      {(["bias", "dark", "flat"] as FrameType[]).map((type) => {
        const meta = FRAME_META[type];
        const Icon = meta.icon;
        const selected = frames[type];
        return (
          <div key={type} className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs font-semibold uppercase tracking-wider flex items-center gap-1.5" style={{ color: meta.color }}>
                <Icon size={12} />
                {meta.label} Frames
                {selected.length > 0 && (
                  <span className="ml-1 text-zinc-500">({selected.length})</span>
                )}
              </span>
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
                <span className="text-[10px] text-zinc-600 py-2 text-center">No processed files available</span>
              )}
            </div>
          </div>
        );
      })}

      {frames.dark.length > 0 && (
        <Slider
          label="Dark Exposure Ratio"
          value={darkExposureRatio}
          min={0.1}
          max={3.0}
          step={0.1}
          accent="sky"
          format={(v) => `${v.toFixed(1)}×`}
          onChange={setDarkExposureRatio}
        />
      )}

      <RunButton
        label="Calibrate"
        runningLabel="Calibrating..."
        running={isCalibrating}
        disabled={!sciencePath || !hasFrames}
        accent="violet"
        onClick={handleCalibrate}
      />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            Calibration Complete
          </div>
          <div className="text-[10px] font-mono text-zinc-400 space-y-0.5">
            <div>{result.dimensions?.[0]}×{result.dimensions?.[1]}</div>
            {result.has_bias && <div>Bias subtracted</div>}
            {result.has_dark && <div>Dark subtracted</div>}
            {result.has_flat && <div>Flat divided</div>}
            {result.fits_path && (
              <div className="text-emerald-400/70 mt-1">Output auto-injected into Stack tab</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
