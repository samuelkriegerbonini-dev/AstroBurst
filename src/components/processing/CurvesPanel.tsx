import { useCallback, useState } from "react";
import type { CurvePoint, LevelsParams } from "../../services/tone";
import { IDENTITY_CURVE, IDENTITY_LEVELS, TONE_IDENTITY_MESSAGE, applyToneFile, isToneIdentity, type ToneFileResult } from "../../services/toneFile";
import { useRenderContext } from "../../context/PreviewContext";
import { chainHoldsOutput } from "../../utils/processingChain";
import { INPUT_CHANGED_MESSAGE, bustPreviewUrl, useProcessingRun } from "../../hooks/useProcessingRun";
import CurveEditor from "../compose/CurveEditor";
import { Slider, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";

export const RGB_PLANES_SENTENCE = "Levels and the curve apply to all three planes.";
export const COMPOSITE_CURVES_NOTICE = "Curves on the composite live in Compose › Adjust.";

export interface CurvesPanelProps {
  selectedFile: { path: string; result?: { is_rgb?: boolean } | null } | null;
  outputDir?: string;
  onProcessingDone?: (result: ToneFileResult) => void;
  chainedFrom?: string;
  inputPreviewUrl?: string | null;
  inputLabel?: string;
  fileKey?: string | null;
  compositeMode: boolean;
  fileName: string;
  disabledReason?: string | null;
  disabledReasonId?: string;
}

export interface CurvesPanelViewProps extends CurvesPanelProps {
  levels: LevelsParams;
  points: CurvePoint[];
  onLevelsChange: (levels: LevelsParams) => void;
  onPointsChange: (points: CurvePoint[]) => void;
}

interface CurvesRun {
  res: ToneFileResult;
  resultUrl: string | undefined;
  baseUrl: string | null;
  baseLabel: string;
}

const ACCENT = "purple";
const SLIDER_ACCENT = "violet";
const CURVE_COLOR = "#a855f7";

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className="text-purple-400">
    <path d="M3 21 C9 21, 11 3, 21 3" />
    <path d="M3 21 L21 3" strokeOpacity="0.35" />
  </svg>
);

function freshIdentityCurve(): CurvePoint[] {
  return IDENTITY_CURVE.map((p) => ({ ...p }));
}

function appliedParts(res: ToneFileResult): string {
  const parts = [res.levels_applied ? "Levels applied" : null, res.curves_applied ? "Curve applied" : null].filter((p): p is string => p !== null);
  return parts.join(" · ");
}

export function CurvesPanelView({
  selectedFile,
  outputDir = "./output",
  onProcessingDone,
  chainedFrom,
  inputPreviewUrl,
  inputLabel,
  fileKey,
  compositeMode,
  disabledReason,
  disabledReasonId,
  levels,
  points,
  onLevelsChange,
  onPointsChange,
}: CurvesPanelViewProps) {
  const { running, blocked, busyTitle, result: runResult, error, run } = useProcessingRun<CurvesRun>("tone", fileKey ?? null);
  const { chain } = useRenderContext();
  const result = !compositeMode && runResult !== null && chainHoldsOutput(chain, "tone", runResult.res.fits_path) ? runResult : null;
  const isRgb = !!selectedFile?.result?.is_rgb;
  const externalReason = compositeMode || isRgb ? null : disabledReason ?? null;
  const reason = compositeMode ? COMPOSITE_CURVES_NOTICE : externalReason;
  const identity = isToneIdentity({ levels, curve: points });
  const canRun = !!selectedFile?.path && !identity && !reason;
  const title = reason ?? (identity ? TONE_IDENTITY_MESSAGE : busyTitle);
  const disabled = !canRun || blocked;

  const handleRun = useCallback(() => {
    if (!selectedFile?.path || compositeMode) return;
    const path = selectedFile.path;
    const baseUrl = inputPreviewUrl ?? null;
    const baseLabel = inputLabel ?? "Original";
    const options = { levels, curve: points };
    void run(async (ctx) => {
      const res = await applyToneFile(path, outputDir, options);
      if (!ctx.inputUnchanged("tone")) throw new Error(INPUT_CHANGED_MESSAGE);
      onProcessingDone?.(res);
      return { res, resultUrl: bustPreviewUrl(res.previewUrl, Date.now()), baseUrl, baseLabel };
    });
  }, [selectedFile?.path, compositeMode, inputPreviewUrl, inputLabel, levels, points, outputDir, run, onProcessingDone]);

  const setLevel = (key: keyof LevelsParams) => (value: number) => onLevelsChange({ ...levels, [key]: value });

  return (
    <div className="flex flex-col gap-3 p-4">
      <SectionHeader icon={ICON} title="Curves" subtitle="Levels and a tone curve on the image" />
      <ChainBanner chainedFrom={compositeMode ? undefined : chainedFrom} accent={SLIDER_ACCENT} />

      {compositeMode && (
        <div className="text-[10px] text-teal-300 bg-teal-900/20 border border-teal-800/30 rounded-lg px-3 py-1.5">
          {COMPOSITE_CURVES_NOTICE}
        </div>
      )}
      {!compositeMode && !selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to apply curves.</div>
      )}
      {!compositeMode && isRgb && (
        <div className="text-[10px] text-purple-300 bg-purple-900/20 border border-purple-800/30 rounded-lg px-3 py-1.5">
          {RGB_PLANES_SENTENCE}
        </div>
      )}

      <div className="flex flex-col gap-2">
        <Slider label="Black point" value={levels.black} min={0} max={0.5} step={0.001} disabled={running} accent={SLIDER_ACCENT} format={(v) => v.toFixed(3)} onChange={setLevel("black")} />
        <Slider label="Gamma" value={levels.gamma} min={0.2} max={5} step={0.01} scale="log" disabled={running} accent={SLIDER_ACCENT} format={(v) => v.toFixed(2)} onChange={setLevel("gamma")} />
        <Slider label="White point" value={levels.white} min={0.5} max={1} step={0.001} disabled={running} accent={SLIDER_ACCENT} format={(v) => v.toFixed(3)} onChange={setLevel("white")} />
      </div>

      <div className="flex flex-col items-center gap-1.5">
        <CurveEditor points={points} onChange={onPointsChange} color={CURVE_COLOR} width={220} height={180} />
        <div className="flex items-center gap-2 w-full">
          <span className="text-[9px] text-zinc-600 flex-1">Dbl-click add, right-click remove</span>
          <button
            type="button"
            className="text-[10px] text-zinc-500 hover:text-zinc-300 disabled:opacity-40"
            disabled={running}
            onClick={() => {
              onLevelsChange(IDENTITY_LEVELS);
              onPointsChange(freshIdentityCurve());
            }}
          >
            Reset
          </button>
        </div>
      </div>

      <button
        type="button"
        data-testid="curves-run"
        onClick={handleRun}
        disabled={running || disabled}
        title={title}
        aria-describedby={externalReason ? disabledReasonId : undefined}
        className="ab-run-btn"
        data-accent={ACCENT}
        data-running={running}
        data-disabled={disabled}
      >
        <span className="flex items-center justify-center gap-2">{running ? "Applying curves..." : "Apply Curves"}</span>
      </button>
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in">
          <ResultGrid items={[
            { label: "Applied", value: appliedParts(result.res) || null },
            { label: "Time", value: `${(result.res.elapsed_ms / 1000).toFixed(2)}s` },
            { label: "Size", value: `${result.res.dimensions[0]}x${result.res.dimensions[1]}` },
          ]} />
          {result.baseUrl && result.resultUrl && (
            <CompareView originalUrl={result.baseUrl} resultUrl={result.resultUrl} originalLabel={result.baseLabel} resultLabel="Curves" accent={SLIDER_ACCENT} height={180} />
          )}
        </div>
      )}
    </div>
  );
}

export default function CurvesPanel(props: CurvesPanelProps) {
  const [levels, setLevels] = useState<LevelsParams>(IDENTITY_LEVELS);
  const [points, setPoints] = useState<CurvePoint[]>(freshIdentityCurve);
  return <CurvesPanelView {...props} levels={levels} points={points} onLevelsChange={setLevels} onPointsChange={setPoints} />;
}
