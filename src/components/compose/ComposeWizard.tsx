import { useState, useCallback, useMemo, useReducer, useEffect, useRef, lazy, Suspense } from "react";
import { Loader2, ChevronRight, Check, ArrowRight } from "lucide-react";
import { useDoneFilesContext, useRenderContext, useNarrowbandContext, useFileContext, useHistContext } from "../../context/PreviewContext";
import { detectNarrowbandFilters } from "../../services/header";
import { STEPS, INITIAL_STATE, STEP_ORDER, nextEnabledStep } from "./wizard.types";
import type { WizardState, FrequencyBin, BlendWeight } from "./wizard.types";

const ChannelStep = lazy(() => import("./steps/ChannelStep"));
const StackStep = lazy(() => import("./steps/StackStep"));
const BackgroundStep = lazy(() => import("./steps/BackgroundStep"));
const AlignStep = lazy(() => import("./steps/AlignStep"));
const BlendStep = lazy(() => import("./steps/BlendStep"));
const CalibrateStep = lazy(() => import("./steps/CalibrateStep"));
const MaskStep = lazy(() => import("./steps/MaskStep"));
const StretchStep = lazy(() => import("./steps/StretchStep"));
const ColorStep = lazy(() => import("./steps/ColorStep"));
const ExportStep = lazy(() => import("./steps/ExportStep"));

type Action =
  | { type: "SET_BINS"; bins: FrequencyBin[] }
  | { type: "SET_BLEND_WEIGHTS"; weights: BlendWeight[]; preset: string }
  | { type: "SET_WB"; mode: WizardState["wbMode"]; r?: number; g?: number; b?: number }
  | { type: "SET_STACKED"; channelId: string; path: string }
  | { type: "SET_BACKGROUND"; channelId: string; path: string }
  | { type: "SET_ALIGNED"; paths: Record<string, string> }
  | { type: "SET_MASK"; path: string | null }
  | { type: "SET_MASK_PARAMS"; growth: number; protection: number }
  | { type: "SET_SEGM"; path: string | null }
  | { type: "SET_STRETCH"; mode: WizardState["stretchMode"]; factor?: number; target?: number }
  | { type: "SET_SCNR"; enabled: boolean; amount?: number }
  | { type: "SET_RESULT"; png: string | null; fits: string | null }
  | { type: "SET_COMPOSITE_READY"; ready: boolean }
  | { type: "UPDATE"; partial: Partial<WizardState> }
  | { type: "COMPLETE_STEP"; stepId: string }
  | { type: "RESET" };

function reducer(state: WizardState, action: Action): WizardState {
  switch (action.type) {
    case "SET_BINS": {
      const hasFiles = action.bins.some((b) => b.files.length > 0);
      const completed = hasFiles
        ? { channels: true }
        : {};
      return { ...state, bins: action.bins, completedSteps: completed };
    }
    case "SET_BLEND_WEIGHTS":
      return { ...state, blendWeights: action.weights, blendPreset: action.preset };
    case "SET_WB":
      return { ...state, wbMode: action.mode, wbR: action.r ?? state.wbR, wbG: action.g ?? state.wbG, wbB: action.b ?? state.wbB };
    case "SET_STACKED": {
      const next = { ...state, stackedPaths: { ...state.stackedPaths, [action.channelId]: action.path } };
      return next;
    }
    case "SET_BACKGROUND": {
      const next = { ...state, backgroundPaths: { ...state.backgroundPaths, [action.channelId]: action.path } };
      return next;
    }
    case "SET_ALIGNED":
      return { ...state, alignedPaths: { ...state.alignedPaths, ...action.paths } };
    case "SET_MASK":
      return { ...state, starMaskPath: action.path };
    case "SET_MASK_PARAMS":
      return { ...state, maskGrowth: action.growth, maskProtection: action.protection };
    case "SET_SEGM":
      return { ...state, segmPath: action.path };
    case "SET_STRETCH":
      return { ...state, stretchMode: action.mode, stretchFactor: action.factor ?? state.stretchFactor, targetBackground: action.target ?? state.targetBackground };
    case "SET_SCNR":
      return { ...state, scnrEnabled: action.enabled, scnrAmount: action.amount ?? state.scnrAmount };
    case "SET_RESULT":
      return { ...state, resultPng: action.png, resultFits: action.fits };
    case "SET_COMPOSITE_READY":
      return { ...state, compositeReady: action.ready };
    case "COMPLETE_STEP": {
      const completed = { ...state.completedSteps, [action.stepId]: true };
      const idx = STEP_ORDER.indexOf(action.stepId);
      for (let i = idx + 1; i < STEP_ORDER.length; i++) {
        delete completed[STEP_ORDER[i]];
      }
      return { ...state, completedSteps: completed };
    }
    case "UPDATE":
      return { ...state, ...action.partial };
    case "RESET":
      return { ...INITIAL_STATE, bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: [] })) };
    default:
      return state;
  }
}

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
    setActiveImagePath,
  } = useRenderContext();

  const { narrowbandPalette } = useNarrowbandContext();

  const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
  const [activeStep, setActiveStep] = useState("channels");
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
  }, [advanceToNext]);

  const handleStepClick = useCallback((stepId: string) => {
    setActiveStep(stepId);
    if (suggestedTimerRef.current) clearTimeout(suggestedTimerRef.current);
    setSuggestedStep(null);
  }, []);

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
  }, [setCompositePreviewUrl, setCompositeAutoStf, setCompositeStf, setActiveImagePath, completeStep]);

  const handleRestretchPreview = useCallback((previewUrl: string | null, stf?: { r: any; g: any; b: any }) => {
    if (previewUrl) {
      setCompositePreviewUrl(previewUrl);
    }
    if (stf) {
      setCompositeStf(stf.r, stf.g, stf.b);
    }
  }, [setCompositePreviewUrl, setCompositeStf]);

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
            onStacked={(channelId, path) => {
              dispatch({ type: "SET_STACKED", channelId, path });
              completeStep("stack");
            }}
          />
        );
      case "background":
        return (
          <BackgroundStep
            state={state}
            onBackground={(channelId, path) => {
              dispatch({ type: "SET_BACKGROUND", channelId, path });
              completeStep("background");
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
      case "blend":
        return (
          <BlendStep
            state={state}
            onWeightsChange={(weights, preset) => dispatch({ type: "SET_BLEND_WEIGHTS", weights, preset })}
            onCompositeReady={handleCompositePreview}
          />
        );
      case "calibrate":
        return (
          <CalibrateStep
            state={state}
            onWbChange={(mode, r, g, b) => dispatch({ type: "SET_WB", mode, r, g, b })}
            onResult={(url) => {
              handleRestretchPreview(url);
              completeStep("calibrate");
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
      case "color":
        return (
          <ColorStep
            state={state}
            onScnrChange={(enabled, amount) => dispatch({ type: "SET_SCNR", enabled, amount })}
            onResult={(url) => {
              handleRestretchPreview(url);
              completeStep("color");
            }}
          />
        );
      case "export":
        return <ExportStep state={state} />;
      default:
        return null;
    }
  }, [activeStep, state, doneFiles, handleCompositePreview, handleRestretchPreview, narrowbandPalette, filterDetections, completeStep]);

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
