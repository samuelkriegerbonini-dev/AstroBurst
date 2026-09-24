import {
  createContext,
  useContext,
  useReducer,
  useCallback,
  useMemo,
} from "react";
import { clearCompositeCache } from "../services/compose";
import type { StfParams } from "../shared/types";

interface CompositeStfTriple {
  r: StfParams;
  g: StfParams;
  b: StfParams;
}

interface CompositeState {
  previewUrl: string | null;
  stf: CompositeStfTriple;
  autoStf: { r: StfParams | null; g: StfParams | null; b: StfParams | null };
  linked: boolean;
  version: number;
}

export type CompositeAction =
  | { type: "SET_PREVIEW_URL"; url: string | null }
  | { type: "SET_STF"; r: StfParams; g: StfParams; b: StfParams }
  | { type: "SET_AUTO_STF"; r: StfParams; g: StfParams; b: StfParams }
  | { type: "SET_LINKED"; linked: boolean }
  | { type: "INIT_RGB"; previewUrl: string | null; stfR: StfParams; stfG: StfParams; stfB: StfParams }
  | { type: "RESET" };

const DEFAULT_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };

export const INITIAL_COMPOSITE_STATE: CompositeState = {
  previewUrl: null,
  stf: { r: DEFAULT_STF, g: DEFAULT_STF, b: DEFAULT_STF },
  autoStf: { r: null, g: null, b: null },
  linked: true,
  version: 0,
};

function sameStf(a: StfParams, b: StfParams): boolean {
  return a.shadow === b.shadow && a.midtone === b.midtone && a.highlight === b.highlight;
}

export function compositeReducer(state: CompositeState, action: CompositeAction): CompositeState {
  switch (action.type) {
    case "SET_PREVIEW_URL":
      return { ...state, previewUrl: action.url, version: state.version + 1 };
    case "SET_STF":
      return { ...state, stf: { r: action.r, g: action.g, b: action.b } };
    case "SET_AUTO_STF":
      return {
        ...state,
        autoStf: { r: action.r, g: action.g, b: action.b },
        linked: sameStf(action.r, action.g) && sameStf(action.g, action.b),
      };
    case "SET_LINKED":
      return { ...state, linked: action.linked };
    case "INIT_RGB":
      return {
        ...state,
        previewUrl: action.previewUrl,
        stf: { r: action.stfR, g: action.stfG, b: action.stfB },
        autoStf: { r: action.stfR, g: action.stfG, b: action.stfB },
        linked: false,
        version: state.version + 1,
      };
    case "RESET":
      return { ...INITIAL_COMPOSITE_STATE, version: state.version + 1 };
  }
}

export function bumpsCompositeVersion(action: CompositeAction): boolean {
  return action.type === "SET_PREVIEW_URL" || action.type === "INIT_RGB" || action.type === "RESET";
}

let liveVersion = 0;

export function liveCompositeVersion(): number {
  return liveVersion;
}

export function noteCompositeAction(action: CompositeAction): void {
  if (bumpsCompositeVersion(action)) liveVersion++;
}

interface CompositePreviewValue {
  compositePreviewUrl: string | null;
  isShowingComposite: boolean;
  compositeVersion: number;
}

interface CompositeStfValue {
  compositeStfR: StfParams;
  compositeStfG: StfParams;
  compositeStfB: StfParams;
  compositeStfLinked: boolean;
  compositeAutoStfR: StfParams | null;
  compositeAutoStfG: StfParams | null;
  compositeAutoStfB: StfParams | null;
}

interface CompositeActionsValue {
  setCompositePreviewUrl: (url: string | null) => void;
  clearComposite: () => Promise<void>;
  setCompositeStf: (r: StfParams, g: StfParams, b: StfParams) => void;
  setCompositeStfLinked: (linked: boolean) => void;
  setCompositeAutoStf: (r: StfParams, g: StfParams, b: StfParams) => void;
  initRgb: (previewUrl: string | null, stfR: StfParams, stfG: StfParams, stfB: StfParams) => void;
  resetComposite: () => void;
}

const CompositePreviewCtx = createContext<CompositePreviewValue | null>(null);
const CompositeStfCtx = createContext<CompositeStfValue | null>(null);
const CompositeActionsCtx = createContext<CompositeActionsValue | null>(null);

function useCtx<T>(ctx: React.Context<T | null>, name: string): T {
  const val = useContext(ctx);
  if (!val) throw new Error(`${name} must be used within CompositeProvider`);
  return val;
}

export const useCompositePreview = () => useCtx(CompositePreviewCtx, "useCompositePreview");
export const useCompositeStf = () => useCtx(CompositeStfCtx, "useCompositeStf");
export const useCompositeActions = () => useCtx(CompositeActionsCtx, "useCompositeActions");

interface Props {
  children: React.ReactNode;
}

export function CompositeProvider({ children }: Props) {
  const [state, reactDispatch] = useReducer(compositeReducer, INITIAL_COMPOSITE_STATE);

  const dispatch = useCallback((action: CompositeAction) => {
    noteCompositeAction(action);
    reactDispatch(action);
  }, []);

  const setCompositePreviewUrl = useCallback((url: string | null) => {
    dispatch({ type: "SET_PREVIEW_URL", url });
  }, [dispatch]);

  const clearComposite = useCallback(async () => {
    dispatch({ type: "RESET" });
    await clearCompositeCache().catch(() => {});
  }, [dispatch]);

  const setCompositeStf = useCallback((r: StfParams, g: StfParams, b: StfParams) => {
    dispatch({ type: "SET_STF", r, g, b });
  }, [dispatch]);

  const setCompositeAutoStf = useCallback((r: StfParams, g: StfParams, b: StfParams) => {
    dispatch({ type: "SET_AUTO_STF", r, g, b });
  }, [dispatch]);

  const setCompositeStfLinked = useCallback((linked: boolean) => {
    dispatch({ type: "SET_LINKED", linked });
  }, [dispatch]);

  const initRgb = useCallback((previewUrl: string | null, stfR: StfParams, stfG: StfParams, stfB: StfParams) => {
    dispatch({ type: "INIT_RGB", previewUrl, stfR, stfG, stfB });
  }, [dispatch]);

  const resetComposite = useCallback(() => {
    dispatch({ type: "RESET" });
  }, [dispatch]);

  const previewValue = useMemo<CompositePreviewValue>(() => ({
    compositePreviewUrl: state.previewUrl,
    isShowingComposite: state.previewUrl !== null,
    compositeVersion: state.version,
  }), [state.previewUrl, state.version]);

  const stfValue = useMemo<CompositeStfValue>(() => ({
    compositeStfR: state.stf.r,
    compositeStfG: state.stf.g,
    compositeStfB: state.stf.b,
    compositeStfLinked: state.linked,
    compositeAutoStfR: state.autoStf.r,
    compositeAutoStfG: state.autoStf.g,
    compositeAutoStfB: state.autoStf.b,
  }), [state.stf, state.linked, state.autoStf]);

  const actionsValue = useMemo<CompositeActionsValue>(() => ({
    setCompositePreviewUrl,
    clearComposite,
    setCompositeStf,
    setCompositeStfLinked,
    setCompositeAutoStf,
    initRgb,
    resetComposite,
  }), [setCompositePreviewUrl, clearComposite, setCompositeStf, setCompositeStfLinked, setCompositeAutoStf, initRgb, resetComposite]);

  return (
    <CompositeActionsCtx.Provider value={actionsValue}>
      <CompositePreviewCtx.Provider value={previewValue}>
        <CompositeStfCtx.Provider value={stfValue}>
          {children}
        </CompositeStfCtx.Provider>
      </CompositePreviewCtx.Provider>
    </CompositeActionsCtx.Provider>
  );
}
