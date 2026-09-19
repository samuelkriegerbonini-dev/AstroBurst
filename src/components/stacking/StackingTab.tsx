import { lazy, Suspense, memo, useState, useCallback, useEffect } from "react";
import { Loader2 } from "lucide-react";
import { useDoneFilesContext, useRenderActions } from "../../context/PreviewContext";
import { useSelectedFile } from "../../hooks/useFileStore";
import { getOutputDir } from "../../infrastructure/tauri";
import { DEFAULT_STACK_SETTINGS, type StackSettings } from "../../utils/stackingRejection";

const CalibrationPanel = lazy(() => import("./CalibrationPanel"));
const CosmeticPanel = lazy(() => import("./CosmeticPanel"));
const StackingPanel = lazy(() => import("./StackingPanel"));
const PipelinePanel = lazy(() => import("./PipelinePanel"));
const SubframeSelectorPanel = lazy(() => import("./SubframeSelectorPanel"));
const DrizzleRgbPanel = lazy(() => import("./DrizzleRgbPanel"));

type StackSection = "calibrate" | "cosmetic" | "subframe" | "stack" | "pipeline" | "drizzle_rgb";

const SECTIONS: { id: StackSection; label: string; color: string }[] = [
  { id: "calibrate", label: "Calibrate", color: "violet" },
  { id: "cosmetic", label: "Cosmetic", color: "fuchsia" },
  { id: "subframe", label: "Subframes", color: "teal" },
  { id: "stack", label: "Stack", color: "amber" },
  { id: "pipeline", label: "Pipeline", color: "cyan" },
  { id: "drizzle_rgb", label: "Drizzle RGB", color: "rose" },
];

export type StackConfig = StackSettings;

export interface CalibrationState {
  calibratedPath: string | null;
  calibratedFitsPath: string | null;
  hasBias: boolean;
  hasDark: boolean;
  hasFlat: boolean;
}

function StackingTabInner() {
  const { doneFiles } = useDoneFilesContext();
  const { setRenderedPreviewUrl } = useRenderActions();
  const selectedFile = useSelectedFile();
  const [active, setActive] = useState<StackSection>("calibrate");
  const [resolvedDir, setResolvedDir] = useState("./output");
  useEffect(() => { getOutputDir().then(setResolvedDir); }, []);

  const [calibration, setCalibration] = useState<CalibrationState>({
    calibratedPath: null,
    calibratedFitsPath: null,
    hasBias: false,
    hasDark: false,
    hasFlat: false,
  });

  const [stackConfig, setStackConfig] = useState<StackConfig>(DEFAULT_STACK_SETTINGS);
  const [injectedPaths, setInjectedPaths] = useState<string[]>([]);
  const [rejectedPaths, setRejectedPaths] = useState<string[]>([]);
  const [subframeWeights, setSubframeWeights] = useState<Record<string, number> | undefined>(undefined);

  const handleSubframeSelection = useCallback(
    (_accepted: string[], rejected: string[], weights?: Record<string, number>) => {
      setRejectedPaths(rejected);
      setSubframeWeights(weights);
      setActive("stack");
    },
    [],
  );

  const handlePreviewUpdate = useCallback(
    (url: string | null | undefined) => {
      if (!url) return;
      const bust = `${url}${url.includes("?") ? "&" : "?"}t=${Date.now()}`;
      setRenderedPreviewUrl(bust);
    },
    [setRenderedPreviewUrl],
  );

  const handleCalibrationDone = useCallback(
    (result: {
      previewUrl?: string | null;
      fits_path?: string;
      has_bias?: boolean;
      has_dark?: boolean;
      has_flat?: boolean;
    }) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.fits_path) {
        const fitsPath = result.fits_path;
        setCalibration({
          calibratedPath: result.previewUrl || null,
          calibratedFitsPath: fitsPath,
          hasBias: result.has_bias || false,
          hasDark: result.has_dark || false,
          hasFlat: result.has_flat || false,
        });
        setInjectedPaths((prev) => {
          if (prev.includes(fitsPath)) return prev;
          return [...prev, fitsPath];
        });
      }
    },
    [handlePreviewUpdate],
  );

  const handleStackResult = useCallback(
    (result: { previewUrl?: string | null }) => {
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
                      : s.color === "teal"
                        ? "bg-teal-600/20 text-teal-400 ring-1 ring-teal-500/30"
                        : s.color === "rose"
                          ? "bg-rose-600/20 text-rose-400 ring-1 ring-rose-500/30"
                          : s.color === "fuchsia"
                            ? "bg-fuchsia-600/20 text-fuchsia-400 ring-1 ring-fuchsia-500/30"
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
          <div style={{ display: active === "cosmetic" ? "block" : "none" }}>
            <CosmeticPanel selectedFile={selectedFile} outputDir={resolvedDir} onPreviewUpdate={handlePreviewUpdate} />
          </div>
          <div style={{ display: active === "subframe" ? "block" : "none" }}>
            <SubframeSelectorPanel files={doneFiles.map(f => f.path)} onSelectionChange={handleSubframeSelection} />
          </div>
          <div style={{ display: active === "stack" ? "block" : "none" }}>
            <StackingPanel
              files={doneFiles}
              onResult={handleStackResult}
              injectedPaths={injectedPaths}
              stackConfig={stackConfig}
              onStackConfigChange={handleStackConfigChange}
              rejectedPaths={rejectedPaths}
              subframeWeights={subframeWeights}
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
          <div style={{ display: active === "drizzle_rgb" ? "block" : "none" }}>
            <DrizzleRgbPanel files={doneFiles} onResult={handleStackResult} />
          </div>
        </div>
      </Suspense>
    </div>
  );
}

export default memo(StackingTabInner);
