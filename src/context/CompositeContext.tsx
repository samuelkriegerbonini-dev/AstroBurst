import {
  createContext,
  useContext,
  useReducer,
  useCallback,
  useMemo,
  useRef,
} from "react";
import { clearCompositeCache, restretchComposite } from "../services/compose";
import { getOutputDir, getPreviewUrl } from "../infrastructure/tauri";
import type { StfParams } from "../shared/types";
import type { DisplayStf } from "../shared/types/compositeChain";

export interface CompositeStfTriple {
  r: StfParams;
  g: StfParams;
  b: StfParams;
}

interface CompositeAutoStf {
  r: StfParams | null;
  g: StfParams | null;
  b: StfParams | null;
}

export interface ParkedComposite {
  previewUrl: string;
  stf: CompositeStfTriple;
  autoStf: CompositeStfTriple | null;
  linked: boolean;
}

export interface CompositeState {
  previewUrl: string | null;
  stf: CompositeStfTriple;
  autoStf: CompositeAutoStf;
  linked: boolean;
  version: number;
  parked: ParkedComposite | null;
  rgbFileView: boolean;
}

export type CompositeAction =
  | { type: "SET_PREVIEW_URL"; url: string | null }
  | { type: "SET_STF"; r: StfParams; g: StfParams; b: StfParams }
  | { type: "SET_AUTO_STF"; r: StfParams; g: StfParams; b: StfParams }
  | { type: "SET_LINKED"; linked: boolean }
  | { type: "INIT_RGB"; previewUrl: string | null; stfR: StfParams; stfG: StfParams; stfB: StfParams }
  | { type: "PARK" }
  | { type: "SHOW_PARKED"; parked: ParkedComposite; url: string }
  | { type: "DROP_PARKED"; parked: ParkedComposite }
  | { type: "REPLACE_PARKED"; previewUrl: string; stf: DisplayStf | null }
  | { type: "RESET" }
  | { type: "CLEAR" };

const DEFAULT_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };
const NO_AUTO_STF: CompositeAutoStf = { r: null, g: null, b: null };

export const INITIAL_COMPOSITE_STATE: CompositeState = {
  previewUrl: null,
  stf: { r: DEFAULT_STF, g: DEFAULT_STF, b: DEFAULT_STF },
  autoStf: NO_AUTO_STF,
  linked: true,
  version: 0,
  parked: null,
  rgbFileView: false,
};

function sameStf(a: StfParams, b: StfParams): boolean {
  return a.shadow === b.shadow && a.midtone === b.midtone && a.highlight === b.highlight;
}

function parkedAutoStf(autoStf: CompositeAutoStf): CompositeStfTriple | null {
  const { r, g, b } = autoStf;
  return r && g && b ? { r, g, b } : null;
}

function parkedDisplay(parked: ParkedComposite): Pick<CompositeState, "stf" | "autoStf" | "linked"> {
  return { stf: parked.stf, autoStf: parked.autoStf ?? NO_AUTO_STF, linked: parked.linked };
}

function replacedParked(parked: ParkedComposite, previewUrl: string, stf: DisplayStf | null): ParkedComposite {
  if (!stf) return { ...parked, previewUrl };
  const triple = { r: stf.r, g: stf.g, b: stf.b };
  return { previewUrl, stf: triple, autoStf: triple, linked: stf.linked };
}

function showsNothing(state: CompositeState): boolean {
  return state.previewUrl === null && !state.rgbFileView;
}

function parkedFrom(state: CompositeState): ParkedComposite | null {
  if (state.previewUrl === null || state.rgbFileView) return state.parked;
  return {
    previewUrl: state.previewUrl,
    stf: state.stf,
    autoStf: parkedAutoStf(state.autoStf),
    linked: state.linked,
  };
}

export function compositeReducer(state: CompositeState, action: CompositeAction): CompositeState {
  switch (action.type) {
    case "SET_PREVIEW_URL":
      return {
        ...state,
        previewUrl: action.url,
        parked: action.url === null ? state.parked : null,
        rgbFileView: false,
        version: state.version + 1,
      };
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
        rgbFileView: true,
        version: state.version + 1,
      };
    case "PARK":
      return { ...state, parked: parkedFrom(state), previewUrl: null, rgbFileView: false, version: state.version + 1 };
    case "SHOW_PARKED":
      if (state.parked !== action.parked) return { ...state, version: state.version + 1 };
      return {
        ...state,
        ...parkedDisplay(action.parked),
        previewUrl: action.url,
        parked: null,
        rgbFileView: false,
        version: state.version + 1,
      };
    case "DROP_PARKED":
      return state.parked === action.parked ? { ...state, parked: null } : state;
    case "REPLACE_PARKED": {
      if (state.parked === null) return state;
      const parked = replacedParked(state.parked, action.previewUrl, action.stf);
      return showsNothing(state) ? { ...state, ...parkedDisplay(parked), parked } : { ...state, parked };
    }
    case "RESET":
      return {
        ...INITIAL_COMPOSITE_STATE,
        ...(state.parked ? parkedDisplay(state.parked) : {}),
        parked: state.parked,
        version: state.version + 1,
      };
    case "CLEAR":
      return { ...INITIAL_COMPOSITE_STATE, version: state.version + 1 };
  }
}

export function bumpsCompositeVersion(action: CompositeAction): boolean {
  switch (action.type) {
    case "SET_PREVIEW_URL":
    case "INIT_RGB":
    case "PARK":
    case "SHOW_PARKED":
    case "RESET":
    case "CLEAR":
      return true;
    default:
      return false;
  }
}

export function canShowParkedComposite(state: Pick<CompositeState, "parked" | "previewUrl" | "rgbFileView">): boolean {
  return state.parked !== null && (state.previewUrl === null || state.rgbFileView);
}

const RESTRETCH_PNG_PREFIX = "rgb_composite";

function decodedOrRaw(text: string): string {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

function urlFileName(url: string): string {
  const path = decodedOrRaw(url.split(/[?#]/, 1)[0]);
  return path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1);
}

export function isRestretchCompositeUrl(url: string): boolean {
  const name = urlFileName(url);
  return name.startsWith(RESTRETCH_PNG_PREFIX) && name.endsWith(".png");
}

export function isCompositeGoneError(e: unknown): boolean {
  return (e instanceof Error ? e.message : String(e)).includes("no longer in memory");
}

export async function parkedCompositeUrl(parked: ParkedComposite): Promise<string> {
  if (!isRestretchCompositeUrl(parked.previewUrl)) return parked.previewUrl;
  const dir = await getOutputDir();
  const { stf, linked } = parked;
  const res = await restretchComposite(dir, stf.r, stf.g, stf.b, undefined, false, linked);
  return getPreviewUrl(res.png_path);
}

export async function restoreParkedComposite(
  parked: ParkedComposite,
  stillParked: () => boolean,
  dispatch: (action: CompositeAction) => void,
): Promise<void> {
  try {
    const url = await parkedCompositeUrl(parked);
    if (stillParked()) dispatch({ type: "SHOW_PARKED", parked, url });
  } catch (e) {
    console.error("[AstroBurst] Showing the parked composite failed:", e);
    if (isCompositeGoneError(e)) dispatch({ type: "DROP_PARKED", parked });
  }
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
  canShowParked: boolean;
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
  park: () => void;
  replaceParked: (previewUrl: string, stf: DisplayStf | null) => void;
  showParkedComposite: () => Promise<void>;
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
  const parkedRef = useRef(state.parked);
  parkedRef.current = state.parked;

  const dispatch = useCallback((action: CompositeAction) => {
    noteCompositeAction(action);
    reactDispatch(action);
  }, []);

  const setCompositePreviewUrl = useCallback((url: string | null) => {
    dispatch({ type: "SET_PREVIEW_URL", url });
  }, [dispatch]);

  const clearComposite = useCallback(async () => {
    dispatch({ type: "CLEAR" });
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

  const parkComposite = useCallback(() => {
    dispatch({ type: "PARK" });
  }, [dispatch]);

  const replaceParked = useCallback((previewUrl: string, stf: DisplayStf | null) => {
    dispatch({ type: "REPLACE_PARKED", previewUrl, stf });
  }, [dispatch]);

  const showParkedComposite = useCallback(async () => {
    const parked = parkedRef.current;
    if (!parked) return;
    await restoreParkedComposite(parked, () => parkedRef.current === parked, dispatch);
  }, [dispatch]);

  const canShowParked = canShowParkedComposite(state);

  const previewValue = useMemo<CompositePreviewValue>(() => ({
    compositePreviewUrl: state.previewUrl,
    isShowingComposite: state.previewUrl !== null,
    compositeVersion: state.version,
    canShowParked,
  }), [state.previewUrl, state.version, canShowParked]);

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
    park: parkComposite,
    replaceParked,
    showParkedComposite,
  }), [setCompositePreviewUrl, clearComposite, setCompositeStf, setCompositeStfLinked, setCompositeAutoStf, initRgb, resetComposite, parkComposite, replaceParked, showParkedComposite]);

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
