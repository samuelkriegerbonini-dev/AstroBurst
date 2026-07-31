import {
  createContext,
  useContext,
  useReducer,
  useCallback,
  useMemo,
} from "react";
import { clearCompositeCache } from "../services/compose";
import type { StfParams } from "../shared/types";

export interface ScnrState {
  enabled: boolean;
  method: string;
  amount: number;
}

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
  scnr: ScnrState | null;
}

type CompositeAction =
  | { type: "SET_PREVIEW_URL"; url: string | null }
  | { type: "SET_STF"; r: StfParams; g: StfParams; b: StfParams }
  | { type: "SET_AUTO_STF"; r: StfParams; g: StfParams; b: StfParams }
  | { type: "SET_LINKED"; linked: boolean }
  | { type: "SET_SCNR"; scnr: ScnrState | null }
  | { type: "INIT_RGB"; previewUrl: string | null; stfR: StfParams; stfG: StfParams; stfB: StfParams }
  | { type: "RESET" };

const DEFAULT_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };

const INITIAL_STATE: CompositeState = {
  previewUrl: null,
  stf: { r: DEFAULT_STF, g: DEFAULT_STF, b: DEFAULT_STF },
  autoStf: { r: null, g: null, b: null },
  linked: true,
  scnr: null,
};

function reducer(state: CompositeState, action: CompositeAction): CompositeState {
  switch (action.type) {
    case "SET_PREVIEW_URL":
      return { ...state, previewUrl: action.url };
    case "SET_STF":
      return { ...state, stf: { r: action.r, g: action.g, b: action.b } };
    case "SET_AUTO_STF":
      return { ...state, autoStf: { r: action.r, g: action.g, b: action.b } };
    case "SET_LINKED":
      return { ...state, linked: action.linked };
    case "SET_SCNR":
      return { ...state, scnr: action.scnr };
    case "INIT_RGB":
      return {
        ...state,
        previewUrl: action.previewUrl,
        stf: { r: action.stfR, g: action.stfG, b: action.stfB },
        autoStf: { r: action.stfR, g: action.stfG, b: action.stfB },
      };
    case "RESET":
      return INITIAL_STATE;
  }
}

interface CompositePreviewValue {
  compositePreviewUrl: string | null;
  isShowingComposite: boolean;
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

interface CompositeScnrValue {
  compositeScnr: ScnrState | null;
}

interface CompositeActionsValue {
  setCompositePreviewUrl: (url: string | null) => void;
  clearComposite: () => Promise<void>;
  setCompositeStf: (r: StfParams, g: StfParams, b: StfParams) => void;
  setCompositeStfLinked: (linked: boolean) => void;
  setCompositeAutoStf: (r: StfParams, g: StfParams, b: StfParams) => void;
  setCompositeScnr: (scnr: ScnrState | null) => void;
  initRgb: (previewUrl: string | null, stfR: StfParams, stfG: StfParams, stfB: StfParams) => void;
  resetComposite: () => void;
}

export type CompositeContextValue = CompositePreviewValue & CompositeStfValue & CompositeScnrValue & CompositeActionsValue;

const CompositePreviewCtx = createContext<CompositePreviewValue | null>(null);
const CompositeStfCtx = createContext<CompositeStfValue | null>(null);
const CompositeScnrCtx = createContext<CompositeScnrValue | null>(null);
const CompositeActionsCtx = createContext<CompositeActionsValue | null>(null);

function useCtx<T>(ctx: React.Context<T | null>, name: string): T {
  const val = useContext(ctx);
  if (!val) throw new Error(`${name} must be used within CompositeProvider`);
  return val;
}

export const useCompositePreview = () => useCtx(CompositePreviewCtx, "useCompositePreview");
export const useCompositeStf = () => useCtx(CompositeStfCtx, "useCompositeStf");
export const useCompositeScnr = () => useCtx(CompositeScnrCtx, "useCompositeScnr");
export const useCompositeActions = () => useCtx(CompositeActionsCtx, "useCompositeActions");

export function useCompositeContext(): CompositeContextValue {
  const preview = useCompositePreview();
  const stf = useCompositeStf();
  const scnr = useCompositeScnr();
  const actions = useCompositeActions();
  return useMemo(
    () => ({ ...preview, ...stf, ...scnr, ...actions }),
    [preview, stf, scnr, actions],
  );
}

interface Props {
  children: React.ReactNode;
}

export function CompositeProvider({ children }: Props) {
  const [state, dispatch] = useReducer(reducer, INITIAL_STATE);

  const setCompositePreviewUrl = useCallback((url: string | null) => {
    dispatch({ type: "SET_PREVIEW_URL", url });
  }, []);

  const clearComposite = useCallback(async () => {
    dispatch({ type: "RESET" });
    await clearCompositeCache().catch(() => {});
  }, []);

  const setCompositeStf = useCallback((r: StfParams, g: StfParams, b: StfParams) => {
    dispatch({ type: "SET_STF", r, g, b });
  }, []);

  const setCompositeAutoStf = useCallback((r: StfParams, g: StfParams, b: StfParams) => {
    dispatch({ type: "SET_AUTO_STF", r, g, b });
  }, []);

  const setCompositeStfLinked = useCallback((linked: boolean) => {
    dispatch({ type: "SET_LINKED", linked });
  }, []);

  const setCompositeScnr = useCallback((scnr: ScnrState | null) => {
    dispatch({ type: "SET_SCNR", scnr });
  }, []);

  const initRgb = useCallback((previewUrl: string | null, stfR: StfParams, stfG: StfParams, stfB: StfParams) => {
    dispatch({ type: "INIT_RGB", previewUrl, stfR, stfG, stfB });
  }, []);

  const resetComposite = useCallback(() => {
    dispatch({ type: "RESET" });
  }, []);

  const previewValue = useMemo<CompositePreviewValue>(() => ({
    compositePreviewUrl: state.previewUrl,
    isShowingComposite: state.previewUrl !== null,
  }), [state.previewUrl]);

  const stfValue = useMemo<CompositeStfValue>(() => ({
    compositeStfR: state.stf.r,
    compositeStfG: state.stf.g,
    compositeStfB: state.stf.b,
    compositeStfLinked: state.linked,
    compositeAutoStfR: state.autoStf.r,
    compositeAutoStfG: state.autoStf.g,
    compositeAutoStfB: state.autoStf.b,
  }), [state.stf, state.linked, state.autoStf]);

  const scnrValue = useMemo<CompositeScnrValue>(() => ({
    compositeScnr: state.scnr,
  }), [state.scnr]);

  const actionsValue = useMemo<CompositeActionsValue>(() => ({
    setCompositePreviewUrl,
    clearComposite,
    setCompositeStf,
    setCompositeStfLinked,
    setCompositeAutoStf,
    setCompositeScnr,
    initRgb,
    resetComposite,
  }), [setCompositePreviewUrl, clearComposite, setCompositeStf, setCompositeStfLinked, setCompositeAutoStf, setCompositeScnr, initRgb, resetComposite]);

  return (
    <CompositeActionsCtx.Provider value={actionsValue}>
      <CompositePreviewCtx.Provider value={previewValue}>
        <CompositeStfCtx.Provider value={stfValue}>
          <CompositeScnrCtx.Provider value={scnrValue}>
            {children}
          </CompositeScnrCtx.Provider>
        </CompositeStfCtx.Provider>
      </CompositePreviewCtx.Provider>
    </CompositeActionsCtx.Provider>
  );
}

export type { ScnrState as CompositeScnrState };
