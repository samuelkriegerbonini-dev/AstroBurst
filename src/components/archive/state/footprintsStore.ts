import { useSyncExternalStore } from "react";
import type { MastError, MastFootprint, MastFootprintInput } from "../../../shared/types/mast";
import { footprintBlock, footprintKey } from "../../../utils/mastFootprints";
import type { ToggleAvailability } from "../logic/footprintView";

export interface WorkspaceImage {
  path: string;
  name: string;
  projectionPath: string;
}

export type WcsCheck = "checking" | "present" | "absent";

export type ShownFootprint = MastFootprint & { key: string };

export interface FootprintsState {
  workspace: WorkspaceImage | null;
  wcs: WcsCheck;
  target: WorkspaceImage | null;
  items: MastFootprintInput[];
  footprints: ShownFootprint[];
  projectedOn: WorkspaceImage | null;
  pending: boolean;
  error: MastError | null;
  hover: string | null;
  focus: string | null;
  saveError: string | null;
}

export type FootprintsAction =
  | { type: "workspace"; file: WorkspaceImage | null; wcs: WcsCheck }
  | { type: "toggle"; input: MastFootprintInput }
  | { type: "clear" }
  | { type: "hover"; datasetId: string | null }
  | { type: "focus"; datasetId: string | null }
  | { type: "projecting" }
  | { type: "idle" }
  | { type: "projected"; image: WorkspaceImage; footprints: ShownFootprint[] }
  | { type: "failed"; error: MastError }
  | { type: "saveFailed"; error: string | null };

export function createFootprintsState(): FootprintsState {
  return {
    workspace: null,
    wcs: "absent",
    target: null,
    items: [],
    footprints: [],
    projectedOn: null,
    pending: false,
    error: null,
    hover: null,
    focus: null,
    saveError: null,
  };
}

function sameImage(a: WorkspaceImage | null, b: WorkspaceImage | null): boolean {
  return a === b || (a !== null && b !== null && a.path === b.path && a.name === b.name && a.projectionPath === b.projectionPath);
}

function sameProjection(a: WorkspaceImage | null, b: WorkspaceImage | null): boolean {
  return a !== null && b !== null && a.path === b.path && a.projectionPath === b.projectionPath;
}

export function footprintsReducer(state: FootprintsState, action: FootprintsAction): FootprintsState {
  switch (action.type) {
    case "workspace": {
      const target = action.wcs === "present" ? action.file : null;
      if (sameImage(state.workspace, action.file) && sameImage(state.target, target) && state.wcs === action.wcs) return state;
      const moved = !sameProjection(target, state.projectedOn);
      return {
        ...state,
        workspace: action.file,
        wcs: action.wcs,
        target,
        footprints: moved ? [] : state.footprints,
        projectedOn: moved ? null : state.projectedOn,
        error: null,
      };
    }
    case "toggle": {
      const key = footprintKey(action.input);
      if (state.items.some((item) => footprintKey(item) === key)) {
        return {
          ...state,
          items: state.items.filter((item) => footprintKey(item) !== key),
          footprints: state.footprints.filter((f) => f.key !== key),
        };
      }
      return { ...state, items: [...state.items, action.input] };
    }
    case "clear":
      return { ...state, items: [], footprints: [], projectedOn: null, pending: false, error: null };
    case "hover":
      return state.hover === action.datasetId ? state : { ...state, hover: action.datasetId };
    case "focus":
      return state.focus === action.datasetId ? state : { ...state, focus: action.datasetId, saveError: null };
    case "projecting":
      return state.pending ? state : { ...state, pending: true };
    case "idle":
      return state.pending ? { ...state, pending: false } : state;
    case "projected":
      return { ...state, pending: false, error: null, footprints: action.footprints, projectedOn: action.image };
    case "failed":
      return { ...state, pending: false, error: action.error, footprints: [], projectedOn: null };
    case "saveFailed":
      return state.saveError === action.error ? state : { ...state, saveError: action.error };
  }
}

export function isShown(state: FootprintsState, input: MastFootprintInput): boolean {
  const key = footprintKey(input);
  return state.items.some((item) => footprintKey(item) === key);
}

export function shownInput(state: FootprintsState, input: MastFootprintInput): MastFootprintInput {
  return state.items.find((item) => item.datasetId === input.datasetId && item.sRegion === input.sRegion) ?? input;
}

export function drawnFootprints(state: FootprintsState): ShownFootprint[] {
  return sameProjection(state.target, state.projectedOn) ? state.footprints : [];
}

export function pixelFootprints(state: FootprintsState): ShownFootprint[] {
  return drawnFootprints(state).filter((f) => f.polygons.length > 0 && f.onImage);
}

export function highlightOf(state: FootprintsState): string | null {
  const hover = state.hover;
  const drawn = hover !== null && drawnFootprints(state).some((f) => f.datasetId === hover && f.polygons.length > 0);
  return drawn ? hover : state.focus;
}

export function shownCount(state: FootprintsState): number {
  return drawnFootprints(state).filter((f) => f.polygons.length > 0).length;
}

export function footprintFor(state: FootprintsState, input: MastFootprintInput): ShownFootprint | null {
  const key = footprintKey(input);
  return drawnFootprints(state).find((f) => f.key === key) ?? null;
}

export function footprintNoteFor(state: FootprintsState, input: MastFootprintInput): string | null {
  return footprintFor(state, input)?.note ?? null;
}

export function availabilityOf(state: FootprintsState, input: MastFootprintInput): ToggleAvailability {
  return {
    blocked: footprintBlock(input),
    workspaceName: state.workspace?.name ?? null,
    fileName: state.target?.name ?? null,
    checking: state.workspace !== null && state.wcs === "checking",
  };
}

export interface FootprintsStore {
  getState(): FootprintsState;
  dispatch(action: FootprintsAction): void;
  subscribe(listener: () => void): () => void;
}

export function createFootprintsStore(initial: FootprintsState = createFootprintsState()): FootprintsStore {
  let state = initial;
  const listeners = new Set<() => void>();
  return {
    getState: () => state,
    dispatch(action) {
      const next = footprintsReducer(state, action);
      if (next === state) return;
      state = next;
      [...listeners].forEach((listener) => listener());
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

export const footprintsStore = createFootprintsStore();

export function useFootprints(store: FootprintsStore = footprintsStore): FootprintsState {
  return useSyncExternalStore(store.subscribe, store.getState, store.getState);
}
