import { lazy, Suspense, memo, useState, useCallback } from "react";
import { Loader2 } from "lucide-react";
import { usePreviewContext } from "../../context/PreviewContext";

const CalibrationPanel = lazy(() => import("./CalibrationPanel"));
const StackingPanel = lazy(() => import("./StackingPanel"));
const PipelinePanel = lazy(() => import("./PipelinePanel"));

type StackSection = "calibrate" | "stack" | "pipeline";

const SECTIONS: { id: StackSection; label: string; color: string }[] = [
  { id: "calibrate", label: "Calibrate", color: "violet" },
  { id: "stack", label: "Stack", color: "amber" },
  { id: "pipeline", label: "Pipeline", color: "cyan" },
];

export interface StackConfig {
  sigmaLow: number;
  sigmaHigh: number;
  maxIterations: number;
  align: boolean;
}

export interface CalibrationState {
  calibratedPath: string | null;
  calibratedFitsPath: string | null;
  hasBias: boolean;
  hasDark: boolean;
  hasFlat: boolean;
}

const DEFAULT_STACK_CONFIG: StackConfig = {
  sigmaLow: 3.0,
  sigmaHigh: 3.0,
  maxIterations: 5,
  align: true,
};

function StackingTabInner() {
  const { doneFiles, setRenderedPreviewUrl } = usePreviewContext();
  const [active, setActive] = useState<StackSection>("calibrate");

  const [calibration, setCalibration] = useState<CalibrationState>({
    calibratedPath: null,
    calibratedFitsPath: null,
    hasBias: false,
    hasDark: false,
    hasFlat: false,
  });

  const [stackConfig, setStackConfig] = useState<StackConfig>(DEFAULT_STACK_CONFIG);
  const [injectedPaths, setInjectedPaths] = useState<string[]>([]);

  const handlePreviewUpdate = useCallback(
    (url: string | null | undefined) => {
      if (!url) return;
      const bust = `${url}${url.includes("?") ? "&" : "?"}t=${Date.now()}`;
      setRenderedPreviewUrl(bust);
    },
    [setRenderedPreviewUrl],
  );

  const handleCalibrationDone = useCallback(
    (result: any) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.fits_path) {
        setCalibration({
          calibratedPath: result.previewUrl || null,
          calibratedFitsPath: result.fits_path,
          hasBias: result.has_bias || false,
          hasDark: result.has_dark || false,
          hasFlat: result.has_flat || false,
        });
        setInjectedPaths((prev) => {
          if (prev.includes(result.fits_path)) return prev;
          return [...prev, result.fits_path];
        });
      }
    },
    [handlePreviewUpdate],
  );

  const handleStackResult = useCallback(
    (result: any) => {
      handlePreviewUpdate(result?.previewUrl);
    },
    [handlePreviewUpdate],
  );

  const handleStackConfigChange = useCallback((config: Partial<StackConfig>) => {
    setStackConfig((prev) => ({ ...prev, ...config }));
  }, []);

  return (
    <div className="flex flex-col h-full">
      <div className="flex gap-1 px-4 pt-3 pb-1">
        {SECTIONS.map((s) => {
          const isActive = active === s.id;
          const hasCalibrated = s.id === "stack" && calibration.calibratedFitsPath;
          return (
            <button
              key={s.id}
              onClick={() => setActive(s.id)}
              className={`px-3 py-1.5 rounded-md text-xs font-medium transition-all duration-150 relative ${
                isActive
                  ? s.color === "violet"
                    ? "bg-violet-600/20 text-violet-400 ring-1 ring-violet-500/30"
                    : s.color === "amber"
                      ? "bg-amber-600/20 text-amber-400 ring-1 ring-amber-500/30"
                      : "bg-cyan-600/20 text-cyan-400 ring-1 ring-cyan-500/30"
                  : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"
              }`}
            >
              {s.label}
              {hasCalibrated && (
                <span className="absolute -top-1 -right-1 w-2 h-2 rounded-full bg-emerald-400" />
              )}
            </button>
          );
        })}
      </div>

      <Suspense
        fallback={
          <div className="flex items-center justify-center py-12">
            <Loader2 size={20} className="animate-spin text-zinc-500" />
          </div>
        }
      >
        <div className="flex-1 overflow-y-auto p-3">
          <div style={{ display: active === "calibrate" ? "block" : "none" }}>
            <CalibrationPanel
              files={doneFiles}
              onPreviewUpdate={handlePreviewUpdate}
              onCalibrationDone={handleCalibrationDone}
            />
          </div>
          <div style={{ display: active === "stack" ? "block" : "none" }}>
            <StackingPanel
              files={doneFiles}
              onResult={handleStackResult}
              injectedPaths={injectedPaths}
              stackConfig={stackConfig}
              onStackConfigChange={handleStackConfigChange}
            />
          </div>
          <div style={{ display: active === "pipeline" ? "block" : "none" }}>
            <PipelinePanel
              files={doneFiles}
              onPreviewUpdate={handlePreviewUpdate}
              calibration={calibration}
              stackConfig={stackConfig}
            />
          </div>
        </div>
      </Suspense>
    </div>
  );
}

export default memo(StackingTabInner);
