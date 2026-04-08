import {
  createContext,
  useContext,
  useReducer,
  useCallback,
  useMemo,
  useState,
} from "react";
import type { Dispatch } from "react";
import {
  BlendWeight,
  FrequencyBin,
  INITIAL_STATE,
  invalidateDownstream,
  STEP_ORDER,
  WizardState,
} from "../utils/wizard";
import type { SubframeAnalysisResult } from "../utils/wizard";

export type WizardAction =
  | { type: "SET_BINS"; bins: FrequencyBin[] }
  | { type: "SET_BLEND_WEIGHTS"; weights: BlendWeight[]; preset: string }
  | { type: "SET_WB"; mode: WizardState["wbMode"]; r?: number; g?: number; b?: number }
  | { type: "SET_STACKED"; channelId: string; path: string }
  | { type: "SET_BACKGROUND"; channelId: string; path: string }
  | { type: "SET_ALIGNED"; paths: Record<string, string> }
  | { type: "SET_CROPPED"; paths: Record<string, string> }
  | { type: "SET_MASK"; path: string | null }
  | { type: "SET_MASK_PARAMS"; growth: number; protection: number }
  | { type: "SET_SEGM"; path: string | null }
  | { type: "SET_STRETCH"; mode: WizardState["stretchMode"]; factor?: number; target?: number }
  | { type: "SET_SCNR"; enabled: boolean; amount?: number; method?: string; preserveLuminance?: boolean }
  | { type: "SET_RESULT"; png: string | null; fits: string | null }
  | { type: "SET_COMPOSITE_READY"; ready: boolean }
  | { type: "SET_SUBFRAME_RESULT"; binId: string; result: SubframeAnalysisResult }
  | { type: "SET_EXCLUDED_FILES"; binId: string; files: string[] }
  | { type: "UPDATE"; partial: Partial<WizardState> }
  | { type: "COMPLETE_STEP"; stepId: string }
  | { type: "INVALIDATE_FROM"; stepId: string }
  | { type: "RESET" };

function reducer(state: WizardState, action: WizardAction): WizardState {
  switch (action.type) {
    case "SET_BINS": {
      const hasFiles = action.bins.some((b) => b.files.length > 0);
      const completed: Record<string, boolean> = hasFiles
        ? { channels: true }
        : {};
      return { ...state, bins: action.bins, completedSteps: completed };
    }
    case "SET_BLEND_WEIGHTS":
      return { ...state, blendWeights: action.weights, blendPreset: action.preset };
    case "SET_WB":
      return { ...state, wbMode: action.mode, wbR: action.r ?? state.wbR, wbG: action.g ?? state.wbG, wbB: action.b ?? state.wbB };
    case "SET_STACKED": {
      return { ...state, stackedPaths: { ...state.stackedPaths, [action.channelId]: action.path } };
    }
    case "SET_BACKGROUND": {
      return { ...state, backgroundPaths: { ...state.backgroundPaths, [action.channelId]: action.path } };
    }
    case "SET_ALIGNED": {
      const downstream = invalidateDownstream(state, "align");
      return { ...state, ...downstream, alignedPaths: { ...state.alignedPaths, ...action.paths } };
    }
    case "SET_CROPPED": {
      const downstream = invalidateDownstream(state, "crop");
      return { ...state, ...downstream, croppedPaths: { ...state.croppedPaths, ...action.paths } };
    }
    case "SET_MASK":
      return { ...state, starMaskPath: action.path };
    case "SET_MASK_PARAMS":
      return { ...state, maskGrowth: action.growth, maskProtection: action.protection };
    case "SET_SEGM":
      return { ...state, segmPath: action.path };
    case "SET_STRETCH":
      return { ...state, stretchMode: action.mode, stretchFactor: action.factor ?? state.stretchFactor, targetBackground: action.target ?? state.targetBackground };
    case "SET_SCNR":
      return {
        ...state,
        scnrEnabled: action.enabled,
        scnrAmount: action.amount ?? state.scnrAmount,
        scnrMethod: (action.method as WizardState["scnrMethod"]) ?? state.scnrMethod,
        scnrPreserveLuminance: action.preserveLuminance ?? state.scnrPreserveLuminance,
      };
    case "SET_RESULT":
      return { ...state, resultPng: action.png, resultFits: action.fits };
    case "SET_COMPOSITE_READY":
      return { ...state, compositeReady: action.ready };
    case "SET_SUBFRAME_RESULT":
      return { ...state, subframeResults: { ...state.subframeResults, [action.binId]: action.result } };
    case "SET_EXCLUDED_FILES":
      return { ...state, excludedFiles: { ...state.excludedFiles, [action.binId]: action.files } };
    case "COMPLETE_STEP": {
      const completed = { ...state.completedSteps, [action.stepId]: true };
      const idx = STEP_ORDER.indexOf(action.stepId);
      for (let i = idx + 1; i < STEP_ORDER.length; i++) {
        delete completed[STEP_ORDER[i]];
      }
      return { ...state, completedSteps: completed };
    }
    case "INVALIDATE_FROM": {
      const downstream = invalidateDownstream(state, action.stepId);
      return { ...state, ...downstream };
    }
    case "UPDATE":
      return { ...state, ...action.partial };
    case "RESET":
      return { ...INITIAL_STATE, bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: [] })) };
    default:
      return state;
  }
}

interface ComposeWizardContextValue {
  state: WizardState;
  dispatch: Dispatch<WizardAction>;
  activeStep: string;
  setActiveStep: (step: string) => void;
}

const ComposeWizardCtx = createContext<ComposeWizardContextValue | null>(null);

export function useComposeWizardContext(): ComposeWizardContextValue {
  const val = useContext(ComposeWizardCtx);
  if (!val) throw new Error("useComposeWizardContext must be used within ComposeWizardProvider");
  return val;
}

interface Props {
  children: React.ReactNode;
}

export function ComposeWizardProvider({ children }: Props) {
  const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
  const [activeStep, setActiveStepRaw] = useState("channels");

  const setActiveStep = useCallback((step: string) => {
    setActiveStepRaw(step);
  }, []);

  const value = useMemo<ComposeWizardContextValue>(() => ({
    state,
    dispatch,
    activeStep,
    setActiveStep,
  }), [state, dispatch, activeStep, setActiveStep]);

  return (
    <ComposeWizardCtx.Provider value={value}>
      {children}
    </ComposeWizardCtx.Provider>
  );
}
