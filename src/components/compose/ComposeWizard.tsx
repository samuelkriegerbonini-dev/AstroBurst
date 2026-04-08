import { useState, useCallback, useMemo, useEffect, useRef, lazy, Suspense } from "react";
import { Loader2, ChevronRight, Check, ArrowRight, RotateCcw } from "lucide-react";
import { useDoneFilesContext, useRenderContext, useNarrowbandContext, useFileContext, useHistContext } from "../../context/PreviewContext";
import { useCompositeContext } from "../../context/CompositeContext";
import { useComposeWizardContext } from "../../context/ComposeWizardContext";
import { detectNarrowbandFilters } from "../../services/header";
import {
  nextEnabledStep,
  STEPS,
} from "../../utils/wizard";


const ChannelStep = lazy(() => import("./steps/ChannelStep"));
const StackStep = lazy(() => import("./steps/StackStep"));
const AlignStep = lazy(() => import("./steps/AlignStep"));
const CropStep = lazy(() => import("./steps/CropStep"));
const BackgroundStep = lazy(() => import("./steps/BackgroundStep"));
const BlendStep = lazy(() => import("./steps/BlendStep"));
const ColorBalanceStep = lazy(() => import("./steps/ColorBalanceStep"));
const MaskStep = lazy(() => import("./steps/MaskStep"));
const StretchStep = lazy(() => import("./steps/StretchStep"));
const AdjustStep = lazy(() => import("./steps/AdjustStep"));
const ExportStep = lazy(() => import("./steps/ExportStep"));

const COLOR_MAP: Record<string, { tab: string; dot: string }> = {
  violet: { tab: "bg-violet-600/20 text-violet-400 ring-1 ring-violet-500/30", dot: "bg-violet-400" },
  blue: { tab: "bg-blue-600/20 text-blue-400 ring-1 ring-blue-500/30", dot: "bg-blue-400" },
  emerald: { tab: "bg-emerald-600/20 text-emerald-400 ring-1 ring-emerald-500/30", dot: "bg-emerald-400" },
  sky: { tab: "bg-sky-600/20 text-sky-400 ring-1 ring-sky-500/30", dot: "bg-sky-400" },
  amber: { tab: "bg-amber-600/20 text-amber-400 ring-1 ring-amber-500/30", dot: "bg-amber-400" },
  cyan: { tab: "bg-cyan-600/20 text-cyan-400 ring-1 ring-cyan-500/30", dot: "bg-cyan-400" },
  rose: { tab: "bg-rose-600/20 text-rose-400 ring-1 ring-rose-500/30", dot: "bg-rose-400" },
  purple: { tab: "bg-purple-600/20 text-purple-400 ring-1 ring-purple-500/30", dot: "bg-purple-400" },
  teal: { tab: "bg-teal-600/20 text-teal-400 ring-1 ring-teal-500/30", dot: "bg-teal-400" },
};

interface FilterDetection {
  path: string;
  filter: string | null;
  hubble_channel?: string | null;
  confidence?: number;
  matched_keyword?: string;
  matched_value?: string;
}

function MiniInfoBar() {
  const { file } = useFileContext();
  const { histData, stfParams } = useHistContext();

  if (!file) return null;

  return (
    <div
      className="flex items-center gap-3 px-3 py-1 text-[9px] font-mono overflow-x-auto scrollbar-hide shrink-0"
      style={{ borderBottom: "1px solid rgba(20,184,166,0.06)", background: "rgba(20,184,166,0.02)" }}
    >
      {file.result?.dimensions && (
        <span className="text-zinc-500 shrink-0">
          {file.result.dimensions[0]}&times;{file.result.dimensions[1]}
        </span>
      )}
      {histData && (
        <>
          <span className="text-zinc-600 shrink-0">
            mean={histData.mean?.toFixed(2)} median={histData.median?.toFixed(2)} &sigma;={histData.sigma?.toFixed(2)}
          </span>
          <span style={{ color: "rgba(20,184,166,0.2)" }}>|</span>
          <span className="shrink-0">
            <span style={{ color: "rgba(239,68,68,0.5)" }}>S={stfParams.shadow.toFixed(4)}</span>
            {" "}
            <span style={{ color: "rgba(245,158,11,0.5)" }}>M={stfParams.midtone.toFixed(4)}</span>
            {" "}
            <span style={{ color: "rgba(16,185,129,0.5)" }}>H={stfParams.highlight.toFixed(4)}</span>
          </span>
        </>
      )}
      {file.result?.header && (
        <>
          <span style={{ color: "rgba(20,184,166,0.2)" }}>|</span>
          <span className="text-zinc-600 truncate shrink-0">
            {[
              file.result.header.TELESCOP,
              file.result.header.INSTRUME,
              file.result.header.FILTER,
            ].filter(Boolean).join(" ")}
          </span>
        </>
      )}
    </div>
  );
}

export default function ComposeWizard() {
  const { doneFiles } = useDoneFilesContext();
  const {
    setCompositePreviewUrl,
    setCompositeAutoStf,
    setCompositeStf,
  } = useCompositeContext();
  const {
    setActiveImagePath,
  } = useRenderContext();

  const { narrowbandPalette } = useNarrowbandContext();

  const { state, dispatch, activeStep, setActiveStep } = useComposeWizardContext();
  const [filterDetections, setFilterDetections] = useState<FilterDetection[]>([]);
  const [suggestedStep, setSuggestedStep] = useState<string | null>(null);
  const suggestedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const detectionKeyRef = useRef("");

  useEffect(() => {
    if (doneFiles.length < 2) return;
    const paths = doneFiles.map((f) => f.path);
    const key = paths.join("|");
    if (key === detectionKeyRef.current) return;
    detectionKeyRef.current = key;
    detectNarrowbandFilters(paths)
      .then((result: any) => {
        if (result?.filters) {
          setFilterDetections(result.filters);
        }
      })
      .catch(() => {});
  }, [doneFiles]);

  const filledBins = useMemo(() => state.bins.filter((b) => b.files.length > 0), [state.bins]);
  const totalFiles = useMemo(() => state.bins.reduce((a, b) => a + b.files.length, 0), [state.bins]);

  const advanceToNext = useCallback((fromStepId: string) => {
    const next = nextEnabledStep(fromStepId, state);
    if (!next) return;
    if (suggestedTimerRef.current) clearTimeout(suggestedTimerRef.current);
    setSuggestedStep(next);
    suggestedTimerRef.current = setTimeout(() => setSuggestedStep(null), 4000);
  }, [state]);

  const completeStep = useCallback((stepId: string) => {
    dispatch({ type: "COMPLETE_STEP", stepId });
    advanceToNext(stepId);
  }, [advanceToNext, dispatch]);

  const handleStepClick = useCallback((stepId: string) => {
    setActiveStep(stepId);
    if (suggestedTimerRef.current) clearTimeout(suggestedTimerRef.current);
    setSuggestedStep(null);
  }, [setActiveStep]);

  const handleCompositePreview = useCallback((previewUrl: string | null, stfR?: any, stfG?: any, stfB?: any, lumFitsPath?: string | null) => {
    if (previewUrl) {
      setCompositePreviewUrl(previewUrl);
    }
    if (stfR && stfG && stfB) {
      setCompositeAutoStf(stfR, stfG, stfB);
      setCompositeStf(stfR, stfG, stfB);
    }
    if (lumFitsPath) {
      setActiveImagePath(lumFitsPath);
    }
    dispatch({ type: "SET_COMPOSITE_READY", ready: true });
    completeStep("blend");
  }, [setCompositePreviewUrl, setCompositeAutoStf, setCompositeStf, setActiveImagePath, completeStep, dispatch]);

  const handleRestretchPreview = useCallback((previewUrl: string | null, stf?: { r: any; g: any; b: any }) => {
    if (previewUrl) {
      setCompositePreviewUrl(previewUrl);
    }
    if (stf) {
      setCompositeStf(stf.r, stf.g, stf.b);
    }
  }, [setCompositePreviewUrl, setCompositeStf]);

  const handleReset = useCallback(() => {
    dispatch({ type: "RESET" });
    setActiveStep("channels");
  }, [dispatch, setActiveStep]);

  const stepContent = useMemo(() => {
    switch (activeStep) {
      case "channels":
        return (
          <ChannelStep
            state={state}
            doneFiles={doneFiles}
            onBinsChange={(bins) => dispatch({ type: "SET_BINS", bins })}
            narrowbandPalette={narrowbandPalette}
            filterDetections={filterDetections}
          />
        );
      case "stack":
        return (
          <StackStep
            state={state}
            dispatch={dispatch}
            onStacked={(channelId, path) => {
              dispatch({ type: "SET_STACKED", channelId, path });
              completeStep("stack");
            }}
          />
        );
      case "align":
        return (
          <AlignStep
            state={state}
            onAligned={(paths) => {
              dispatch({ type: "SET_ALIGNED", paths });
              completeStep("align");
            }}
          />
        );
      case "crop":
        return (
          <CropStep
            state={state}
            onCropped={(paths) => {
              dispatch({ type: "SET_CROPPED", paths });
              completeStep("crop");
            }}
          />
        );
      case "background":
        return (
          <BackgroundStep
            state={state}
            onBackground={(channelId: string, path: string) => {
              dispatch({ type: "SET_BACKGROUND", channelId, path });
              completeStep("background");
            }}
          />
        );
      case "blend":
        return (
          <BlendStep
            state={state}
            onWeightsChange={(weights, preset) => dispatch({ type: "SET_BLEND_WEIGHTS", weights, preset })}
            onCompositeReady={(url, autoStf) => {
              if (autoStf) {
                handleCompositePreview(url, autoStf, autoStf, autoStf);
              } else {
                handleCompositePreview(url);
              }
            }}
          />
        );
      case "colorbalance":
        return (
          <ColorBalanceStep
            state={state}
            filterDetections={filterDetections}
            onWbChange={(mode, r, g, b) => dispatch({ type: "SET_WB", mode, r, g, b })}
            onScnrChange={(enabled, amount, method, preserveLuminance) =>
              dispatch({ type: "SET_SCNR", enabled, amount, method, preserveLuminance })
            }
            onResult={(url, autoStf) => {
              if (autoStf) {
                handleRestretchPreview(url, { r: autoStf, g: autoStf, b: autoStf });
                setCompositeAutoStf(autoStf, autoStf, autoStf);
              } else {
                handleRestretchPreview(url);
              }
              completeStep("colorbalance");
            }}
          />
        );
      case "mask":
        return (
          <MaskStep
            state={state}
            onMask={(path) => dispatch({ type: "SET_MASK", path })}
            onMaskParams={(growth, protection) => {
              dispatch({ type: "SET_MASK_PARAMS", growth, protection });
            }}
          />
        );
      case "stretch":
        return (
          <StretchStep
            state={state}
            onStretchChange={(mode, factor, target) => dispatch({ type: "SET_STRETCH", mode, factor, target })}
            onResult={(url, stf) => {
              handleRestretchPreview(url, stf);
              completeStep("stretch");
            }}
          />
        );
      case "adjust":
        return (
          <AdjustStep
            state={state}
            onResult={(url) => {
              handleRestretchPreview(url);
              completeStep("adjust");
            }}
          />
        );
      case "export":
        return <ExportStep state={state} />;
      default:
        return null;
    }
  }, [activeStep, state, doneFiles, handleCompositePreview, handleRestretchPreview, setCompositeAutoStf, narrowbandPalette, filterDetections, completeStep, dispatch]);

  return (
    <div className="flex flex-col h-full">
      <MiniInfoBar />

      <div className="flex items-center gap-0.5 px-2 pt-1.5 pb-1 overflow-x-auto scrollbar-hide shrink-0">
        {STEPS.map((step, idx) => {
          const isActive = activeStep === step.id;
          const isEnabled = step.enabled(state);
          const badge = step.badge?.(state);
          const isDone = !!state.completedSteps[step.id];
          const isSuggested = suggestedStep === step.id;
          const colors = COLOR_MAP[step.color] ?? COLOR_MAP.violet;
          return (
            <button
              key={step.id}
              onClick={() => isEnabled && handleStepClick(step.id)}
              disabled={!isEnabled}
              className={`ab-step-pill ${
                isActive ? colors.tab : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"
              } ${isSuggested ? "ab-step-suggested" : ""}`}
              title={step.label}
            >
              <span className="text-[9px] text-zinc-600 font-mono">{idx + 1}</span>
              {step.shortLabel}
              {isDone && !badge && (
                <span className="ab-step-done-badge">
                  <Check size={8} strokeWidth={3} />
                </span>
              )}
              {badge && (
                <span className={`w-4 h-4 flex items-center justify-center rounded-full text-[8px] font-bold ${isDone ? "bg-emerald-500 text-white" : colors.dot + " text-zinc-900"}`}>
                  {isDone && badge === "✓" ? <Check size={8} strokeWidth={3} /> : badge}
                </span>
              )}
              {isSuggested && (
                <ArrowRight size={10} className="ab-step-arrow" />
              )}
            </button>
          );
        })}

        <button
          onClick={handleReset}
          className="ml-auto px-1.5 py-1 rounded text-[9px] text-zinc-600 hover:text-red-400 hover:bg-red-500/10 transition-all"
          title="Reset Wizard"
        >
          <RotateCcw size={10} />
        </button>
      </div>

      {totalFiles > 0 && (
        <div className="flex items-center gap-1 px-3 py-1 text-[9px] font-mono text-zinc-600 border-b border-zinc-800/30 overflow-x-auto shrink-0">
          {filledBins.map((bin, i) => (
            <span key={bin.id} className="flex items-center gap-0.5">
              {i > 0 && <ChevronRight size={8} className="text-zinc-700" />}
              <span className="flex items-center gap-1">
                <span className="w-1.5 h-1.5 rounded-full" style={{ background: bin.color }} />
                <span>{bin.shortLabel}</span>
                <span className="text-zinc-700">({bin.files.length})</span>
              </span>
            </span>
          ))}
          {state.compositeReady && (
            <span className="ml-auto text-emerald-500/70 flex items-center gap-1">
              <Check size={9} /> composite ready
            </span>
          )}
        </div>
      )}

      <div className="flex-1 overflow-y-auto">
        <Suspense fallback={<div className="flex items-center justify-center py-12"><Loader2 size={20} className="animate-spin text-zinc-500" /></div>}>
          {stepContent}
        </Suspense>
      </div>
    </div>
  );
}
