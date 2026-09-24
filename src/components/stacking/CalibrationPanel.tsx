import { useState, useCallback, useMemo } from "react";
import { CheckCircle2 } from "lucide-react";
import { Slider, ErrorAlert } from "../ui";
import { useProgress } from "../../hooks/useProgress";
import { calibrate } from "../../services/stacking";
import type { CalibrateResult } from "../../shared/types";
import { CALIBRATE_PROGRESS_EVENT } from "../../shared/types/stacking";
import { getOutputDir } from "../../infrastructure/tauri";
import SmartChannelMapper from "../compose/SmartChannelMapper";
import type { ChannelFile, CalibAssignment } from "../compose/SmartChannelMapper";
import type { ProcessedFile } from "../../shared/types";
import type { RunTarget } from "./StackingTab";

export interface CalibrationMasters {
  darkPaths: string[];
  flatPaths: string[];
  biasPaths: string[];
}

interface CalibrationPanelProps {
  files: ProcessedFile[];
  runTarget?: RunTarget | null;
  onCalibrationDone?: (result: CalibrateResult, masters: CalibrationMasters, sciencePath: string, target: RunTarget | null) => void;
}

function toChannelFiles(files: ProcessedFile[]): ChannelFile[] {
  return files.map((f) => ({
    id: f.id ?? f.path,
    path: f.path ?? "",
    name: f.name ?? "Unknown",
    filter: f.result?.header?.FILTER as string | undefined,
    instrument: f.result?.header?.INSTRUME as string | undefined,
    exptime: f.result?.header?.EXPTIME as number | undefined,
    previewUrl: f.result?.previewUrl,
  }));
}

export default function CalibrationPanel({ files = [], runTarget = null, onCalibrationDone }: CalibrationPanelProps) {
  const [darkExposureRatio, setDarkExposureRatio] = useState(1.0);
  const [isCalibrating, setIsCalibrating] = useState(false);
  const [result, setResult] = useState<CalibrateResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastAssignment, setLastAssignment] = useState<CalibAssignment | null>(null);
  const progress = useProgress(CALIBRATE_PROGRESS_EVENT);
  const resetProgress = progress.reset;

  const channelFiles = useMemo(() => toChannelFiles(files), [files]);

  const handleCalibrate = useCallback(async (assignments: CalibAssignment) => {
    if (!assignments.science) return;
    const target = runTarget;
    const sciencePath = assignments.science.path;
    setLastAssignment(assignments);
    setIsCalibrating(true);
    setError(null);
    setResult(null);
    resetProgress();
    const masters: CalibrationMasters = {
      darkPaths: assignments.dark.map((f) => f.path),
      flatPaths: assignments.flat.map((f) => f.path),
      biasPaths: assignments.bias.map((f) => f.path),
    };
    try {
      const res = await calibrate(sciencePath, await getOutputDir(), {
        biasPaths: masters.biasPaths.length > 0 ? masters.biasPaths : undefined,
        darkPaths: masters.darkPaths.length > 0 ? masters.darkPaths : undefined,
        flatPaths: masters.flatPaths.length > 0 ? masters.flatPaths : undefined,
        darkExposureRatio,
      });
      setResult(res);
      onCalibrationDone?.(res, masters, sciencePath, target);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsCalibrating(false);
      resetProgress();
    }
  }, [darkExposureRatio, resetProgress, runTarget, onCalibrationDone]);

  const hasDarks = lastAssignment ? lastAssignment.dark.length > 0 : false;

  return (
    <div className="flex flex-col gap-4 h-full overflow-y-auto">
      <SmartChannelMapper
        mode="calibration"
        files={channelFiles}
        onCalibrate={handleCalibrate}
        isLoading={isCalibrating}
      />

      {hasDarks && (
        <div className="px-4">
          <Slider
            label="Dark Exposure Ratio"
            value={darkExposureRatio}
            min={0.1}
            max={3.0}
            step={0.1}
            accent="sky"
            format={(v) => `${v.toFixed(1)}x`}
            onChange={setDarkExposureRatio}
          />
        </div>
      )}

      {isCalibrating && progress.active && (
        <div className="px-4 flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-sky), #7dd3fc)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{progress.stage}</span>
            <span>{progress.percent}%</span>
          </div>
        </div>
      )}

      <div className="px-4">
        <ErrorAlert message={error} />
      </div>

      {result && (
        <div className="mx-4 flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            Calibration Complete
          </div>
          <div className="text-[10px] font-mono text-zinc-400 space-y-0.5">
            <div>{result.dimensions?.[0]}x{result.dimensions?.[1]}</div>
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
