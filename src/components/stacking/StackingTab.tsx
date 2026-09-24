import { lazy, Suspense, memo, useState, useCallback, useEffect, useMemo } from "react";
import { Loader2 } from "lucide-react";
import { fileKeyOf, getRenderRecord, useDoneFilesContext, useFileContext, useRenderActions } from "../../context/PreviewContext";
import { fileStore } from "../../hooks/useFileStore";
import { getOutputDir, getPreviewUrl } from "../../infrastructure/tauri";
import { DEFAULT_STACK_SETTINGS, type StackSettings } from "../../utils/stackingRejection";
import { framesLabel, resultsForRecipients, showsOutput, sourceLabel, toDims } from "../../utils/stackingOutputs";
import type { CalibrateResult } from "../../shared/types";
import type { DrizzleRgbResult, StackResult } from "../../shared/types/stacking";
import type { CosmeticBatchResult, CosmeticResult } from "../../shared/types/cosmetic";
import type { ProcessedResult } from "../../shared/types/preview";
import type { CalibrationMasters } from "./CalibrationPanel";

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

const SECTION_ACTIVE_CLASS: Record<string, string> = {
  violet: "bg-violet-600/20 text-violet-400 ring-1 ring-violet-500/30",
  fuchsia: "bg-fuchsia-600/20 text-fuchsia-400 ring-1 ring-fuchsia-500/30",
  teal: "bg-teal-600/20 text-teal-400 ring-1 ring-teal-500/30",
  amber: "bg-amber-600/20 text-amber-400 ring-1 ring-amber-500/30",
  cyan: "bg-cyan-600/20 text-cyan-400 ring-1 ring-cyan-500/30",
  rose: "bg-rose-600/20 text-rose-400 ring-1 ring-rose-500/30",
};

export type StackConfig = StackSettings;

export interface CalibrationState {
  calibratedPath: string | null;
  calibratedFitsPath: string | null;
  hasBias: boolean;
  hasDark: boolean;
  hasFlat: boolean;
  darkPaths: string[];
  flatPaths: string[];
  biasPaths: string[];
}

export interface RunTarget {
  key: string;
  path: string;
}

const COSMETIC_LABEL = "Cosmetic";

function filesShowing(output: Pick<ProcessedResult, "fitsPath" | "previewUrl">): RunTarget[] {
  const shown: RunTarget[] = [];
  for (const f of fileStore.getFiles()) {
    const key = fileKeyOf(f);
    if (key && showsOutput(getRenderRecord(key)?.processed, output)) shown.push({ key, path: f.path });
  }
  return shown;
}

function StackingTabInner() {
  const { doneFiles } = useDoneFilesContext();
  const { publishProcessed } = useRenderActions();
  const { file } = useFileContext();
  const [active, setActive] = useState<StackSection>("calibrate");
  const [resolvedDir, setResolvedDir] = useState("./output");
  useEffect(() => { getOutputDir().then(setResolvedDir); }, []);

  const [calibration, setCalibration] = useState<CalibrationState>({
    calibratedPath: null,
    calibratedFitsPath: null,
    hasBias: false,
    hasDark: false,
    hasFlat: false,
    darkPaths: [],
    flatPaths: [],
    biasPaths: [],
  });

  const [stackConfig, setStackConfig] = useState<StackConfig>(DEFAULT_STACK_SETTINGS);
  const [injectedPaths, setInjectedPaths] = useState<string[]>([]);
  const [rejectedPaths, setRejectedPaths] = useState<string[]>([]);
  const [acceptedPaths, setAcceptedPaths] = useState<string[] | undefined>(undefined);
  const [subframeWeights, setSubframeWeights] = useState<Record<string, number> | undefined>(undefined);

  const handleSubframeSelection = useCallback(
    (accepted: string[], rejected: string[], weights?: Record<string, number>) => {
      setAcceptedPaths(accepted);
      setRejectedPaths(rejected);
      setSubframeWeights(weights);
      setActive("stack");
    },
    [],
  );

  const filePath = file?.path ?? null;
  const fileKey = fileKeyOf(file);
  const runTarget = useMemo<RunTarget | null>(
    () => (fileKey && filePath ? { key: fileKey, path: filePath } : null),
    [fileKey, filePath],
  );

  const publishOutput = useCallback(
    (target: RunTarget, output: Omit<ProcessedResult, "label">, labelFor: (recipientPath: string) => string) => {
      if (!output.fitsPath && !output.previewUrl) return;
      for (const { key, result } of resultsForRecipients(target, filesShowing(output), output, labelFor)) {
        publishProcessed(key, result);
      }
    },
    [publishProcessed],
  );

  const handleCalibrationDone = useCallback(
    (result: CalibrateResult, masters: CalibrationMasters, sciencePath: string, target: RunTarget | null) => {
      if (target) {
        publishOutput(target, {
          fitsPath: result.fits_path ?? null,
          previewUrl: result.previewUrl ?? null,
          dimensions: toDims(result.dimensions),
          kind: "stacking",
          inputPath: sciencePath,
        }, (path) => sourceLabel("Calibrated", sciencePath, path));
      }
      if (result?.fits_path) {
        const fitsPath = result.fits_path;
        setCalibration({
          calibratedPath: result.previewUrl || null,
          calibratedFitsPath: fitsPath,
          hasBias: result.has_bias || false,
          hasDark: result.has_dark || false,
          hasFlat: result.has_flat || false,
          darkPaths: masters.darkPaths,
          flatPaths: masters.flatPaths,
          biasPaths: masters.biasPaths,
        });
        setInjectedPaths((prev) => {
          if (prev.includes(fitsPath)) return prev;
          return [...prev, fitsPath];
        });
      }
    },
    [publishOutput],
  );

  const handleCosmeticDone = useCallback(
    (result: CosmeticResult, target: RunTarget | null) => {
      if (!target) return;
      publishOutput(target, {
        fitsPath: result.fits_path ?? null,
        previewUrl: result.previewUrl ?? null,
        dimensions: toDims(result.dimensions),
        kind: "stacking",
        inputPath: result.path,
      }, (path) => sourceLabel(COSMETIC_LABEL, result.path, path));
    },
    [publishOutput],
  );

  const handleCosmeticBatchDone = useCallback(
    async (batch: CosmeticBatchResult) => {
      for (const item of batch.results) {
        const fitsPath = item.fits_path;
        if (item.error || !fitsPath) continue;
        const previewUrl = item.png_path ? await getPreviewUrl(item.png_path) : null;
        for (const f of filesShowing({ fitsPath, previewUrl: null })) {
          publishProcessed(f.key, {
            fitsPath,
            previewUrl: previewUrl || null,
            dimensions: toDims(item.dimensions),
            label: sourceLabel(COSMETIC_LABEL, item.path, f.path),
            kind: "stacking",
            inputPath: item.path,
          });
        }
      }
    },
    [publishProcessed],
  );

  const handleStackResult = useCallback(
    (result: StackResult, inputs: string[], target: RunTarget | null) => {
      if (!target) return;
      publishOutput(target, {
        fitsPath: result.fits_path ?? null,
        previewUrl: result.previewUrl ?? null,
        dimensions: toDims(result.dimensions),
        kind: "stacking",
        inputPath: inputs[0] ?? target.path,
      }, () => framesLabel("Stack", result.frame_count ?? inputs.length));
    },
    [publishOutput],
  );

  const handleDrizzleResult = useCallback(
    (result: DrizzleRgbResult, inputs: string[], target: RunTarget | null) => {
      if (!target) return;
      const frames = result.frame_count_r + result.frame_count_g + result.frame_count_b;
      publishOutput(target, {
        fitsPath: null,
        previewUrl: result.previewUrl ?? null,
        dimensions: null,
        kind: "stacking",
        inputPath: inputs[0] ?? target.path,
      }, () => framesLabel("Drizzle RGB", frames || inputs.length));
    },
    [publishOutput],
  );

  const handleStackConfigChange = useCallback((config: Partial<StackConfig>) => {
    setStackConfig((prev) => ({ ...prev, ...config }));
  }, []);

  return (
    <div className="flex flex-col h-full">
      <div className="flex gap-1 flex-wrap px-4 pt-3 pb-1">
        {SECTIONS.map((s) => {
          const isActive = active === s.id;
          const hasCalibrated = s.id === "stack" && calibration.calibratedFitsPath;
          return (
            <button
              key={s.id}
              onClick={() => setActive(s.id)}
              className={`ab-processing-pill whitespace-nowrap shrink-0 ${
                isActive ? SECTION_ACTIVE_CLASS[s.color] : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"
              }`}
            >
              {s.label}
              {hasCalibrated && (
                <span className="ab-processing-pill-dot bg-emerald-400" />
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
              runTarget={runTarget}
              onCalibrationDone={handleCalibrationDone}
            />
          </div>
          <div style={{ display: active === "cosmetic" ? "block" : "none" }}>
            <CosmeticPanel
              selectedFile={file}
              outputDir={resolvedDir}
              runTarget={runTarget}
              onProcessingDone={handleCosmeticDone}
              onBatchDone={handleCosmeticBatchDone}
            />
          </div>
          <div style={{ display: active === "subframe" ? "block" : "none" }}>
            <SubframeSelectorPanel files={doneFiles.map(f => f.path)} onSelectionChange={handleSubframeSelection} />
          </div>
          <div style={{ display: active === "stack" ? "block" : "none" }}>
            <StackingPanel
              files={doneFiles}
              runTarget={runTarget}
              onResult={handleStackResult}
              injectedPaths={injectedPaths}
              acceptedPaths={acceptedPaths}
              stackConfig={stackConfig}
              onStackConfigChange={handleStackConfigChange}
              rejectedPaths={rejectedPaths}
              subframeWeights={subframeWeights}
            />
          </div>
          <div style={{ display: active === "pipeline" ? "block" : "none" }}>
            <PipelinePanel
              files={doneFiles}
              calibration={calibration}
              stackConfig={stackConfig}
            />
          </div>
          <div style={{ display: active === "drizzle_rgb" ? "block" : "none" }}>
            <DrizzleRgbPanel files={doneFiles} runTarget={runTarget} onResult={handleDrizzleResult} />
          </div>
        </div>
      </Suspense>
    </div>
  );
}

export default memo(StackingTabInner);
