import { useState, useCallback, useMemo } from "react";
import { CheckCircle2 } from "lucide-react";
import { Slider, ErrorAlert } from "../ui";
import { calibrate } from "../../services/stacking.service";
import SmartChannelMapper from "../compose/SmartChannelMapper";
import type { ChannelFile, CalibAssignment } from "../compose/SmartChannelMapper";
import type { ProcessedFile } from "../../shared/types";

interface CalibrationPanelProps {
  files: ProcessedFile[];
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onCalibrationDone?: (result: any) => void;
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

export default function CalibrationPanel({ files = [], onPreviewUpdate, onCalibrationDone }: CalibrationPanelProps) {
  const [darkExposureRatio, setDarkExposureRatio] = useState(1.0);
  const [isCalibrating, setIsCalibrating] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastAssignment, setLastAssignment] = useState<CalibAssignment | null>(null);

  const channelFiles = useMemo(() => toChannelFiles(files), [files]);

  const handleCalibrate = useCallback(async (assignments: CalibAssignment) => {
    if (!assignments.science) return;
    setLastAssignment(assignments);
    setIsCalibrating(true);
    setError(null);
    setResult(null);
    try {
      const res = await calibrate(assignments.science.path, "./output", {
        biasPaths: assignments.bias.length > 0 ? assignments.bias.map((f) => f.path) : undefined,
        darkPaths: assignments.dark.length > 0 ? assignments.dark.map((f) => f.path) : undefined,
        flatPaths: assignments.flat.length > 0 ? assignments.flat.map((f) => f.path) : undefined,
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
  }, [darkExposureRatio, onPreviewUpdate, onCalibrationDone]);

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
