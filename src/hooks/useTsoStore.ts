import { useCallback, useSyncExternalStore } from "react";
import { tsoLightCurves, tsoOpen } from "../services/tso";
import type { LightCurveDoc } from "../shared/types/lightCurve";
import type {
  SeriesKey,
  TsoCurveMode,
  TsoLightCurves,
  TsoLightCurvesRequest,
  TsoOpenResult,
  TsoSeriesSummary,
} from "../shared/types/tso";
import { DEFAULT_REJECT_SIGMA, lightCurveDocsFromTso } from "../utils/tsoLightCurves";
import { lightCurveStore } from "./useLightCurveStore";

export const MAX_TSO_SETS = 8;

export interface TsoSetState {
  paths: readonly string[];
  loading: boolean;
  result: TsoOpenResult | null;
  error: string | null;
}

export interface TsoCurvesDraft {
  mode: TsoCurveMode;
  minText: string;
  maxText: string;
  binsText: string;
  extraBins: readonly (readonly [number, number])[];
  excludeText: string;
  rejectOn: boolean;
  rejectSigmaText: string;
}

export interface TsoCurvesState {
  loading: boolean;
  result: TsoLightCurves | null;
  error: string | null;
}

export interface TsoSectionProps {
  fileKey: string;
  paths: readonly string[];
  set: TsoOpenResult;
  series: TsoSeriesSummary;
  group: string;
}

const NO_PATHS: readonly string[] = Object.freeze([]);
const NO_DOCS: readonly LightCurveDoc[] = Object.freeze([]);

export const EMPTY_TSO_SET: TsoSetState = Object.freeze({ paths: NO_PATHS, loading: false, result: null, error: null });

export const EMPTY_TSO_CURVES: TsoCurvesState = Object.freeze({ loading: false, result: null, error: null });

export const DEFAULT_TSO_DRAFT: TsoCurvesDraft = Object.freeze({
  mode: "pipeline",
  minText: "",
  maxText: "",
  binsText: "",
  extraBins: Object.freeze([]),
  excludeText: "",
  rejectOn: true,
  rejectSigmaText: String(DEFAULT_REJECT_SIGMA),
});

type Listener = () => void;

interface ComputedCurves {
  paths: readonly string[];
  request: TsoLightCurvesRequest;
}

function errorText(reason: unknown): string {
  if (typeof reason === "object" && reason !== null && "message" in reason) return String(reason.message);
  return String(reason);
}

function samePaths(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((path, i) => path === b[i]);
}

export class TsoStore {
  private sets = new Map<string, TsoSetState>();
  private chosen = new Map<string, readonly string[]>();
  private seriesIndex = new Map<string, number>();
  private drafts = new Map<string, TsoCurvesDraft>();
  private curves = new Map<string, TsoCurvesState>();
  private computed = new Map<string, ComputedCurves>();
  private extraDocs = new Map<string, readonly LightCurveDoc[]>();
  private openSeq = new Map<string, number>();
  private curvesSeq = new Map<string, number>();
  private sequence = 0;
  private listeners = new Set<Listener>();

  constructor(
    private readonly read: typeof tsoOpen = tsoOpen,
    private readonly readCurves: typeof tsoLightCurves = tsoLightCurves,
  ) {}

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private notify(): void {
    this.listeners.forEach((l) => l());
  }

  private nextSequence(): number {
    this.sequence += 1;
    return this.sequence;
  }

  private dropSet(fileKey: string): void {
    this.sets.delete(fileKey);
    this.chosen.delete(fileKey);
    this.seriesIndex.delete(fileKey);
    this.openSeq.delete(fileKey);
  }

  private writeSet(fileKey: string, state: TsoSetState): void {
    this.sets.delete(fileKey);
    this.sets.set(fileKey, state);
    for (const [candidate, held] of this.sets) {
      if (this.sets.size <= MAX_TSO_SETS) break;
      if (candidate !== fileKey && !held.loading) this.dropSet(candidate);
    }
    this.notify();
  }

  getSet(fileKey: string): TsoSetState {
    return this.sets.get(fileKey) ?? EMPTY_TSO_SET;
  }

  async open(fileKey: string, paths: readonly string[]): Promise<void> {
    const current = this.sets.get(fileKey);
    if (current && samePaths(current.paths, paths) && (current.loading || current.result !== null)) return;
    const seq = this.nextSequence();
    this.openSeq.set(fileKey, seq);
    const asked = Object.freeze([...paths]);
    this.writeSet(fileKey, { paths: asked, loading: true, result: null, error: null });
    try {
      const result = await this.read([...asked]);
      if (this.openSeq.get(fileKey) === seq) this.writeSet(fileKey, { paths: asked, loading: false, result, error: null });
    } catch (e) {
      if (this.openSeq.get(fileKey) === seq) this.writeSet(fileKey, { paths: asked, loading: false, result: null, error: errorText(e) });
    }
  }

  getChosen(fileKey: string): readonly string[] {
    return this.chosen.get(fileKey) ?? NO_PATHS;
  }

  setChosen(fileKey: string, paths: readonly string[]): void {
    this.chosen.set(fileKey, Object.freeze([...paths]));
    this.notify();
  }

  getSeriesIndex(fileKey: string): number {
    return this.seriesIndex.get(fileKey) ?? 0;
  }

  setSeriesIndex(fileKey: string, index: number): void {
    this.seriesIndex.set(fileKey, Number.isFinite(index) ? Math.max(0, Math.trunc(index)) : 0);
    this.notify();
  }

  getDraft(group: string): TsoCurvesDraft {
    return this.drafts.get(group) ?? DEFAULT_TSO_DRAFT;
  }

  patchDraft(group: string, patch: Partial<TsoCurvesDraft>): void {
    this.drafts.set(group, Object.freeze({ ...this.getDraft(group), ...patch }));
    this.notify();
  }

  getCurves(group: string): TsoCurvesState {
    return this.curves.get(group) ?? EMPTY_TSO_CURVES;
  }

  getRequest(group: string): TsoLightCurvesRequest | null {
    return this.computed.get(group)?.request ?? null;
  }

  private writeDocs(group: string): void {
    const result = this.getCurves(group).result;
    const computed = this.computed.get(group);
    if (result === null || computed === undefined) return;
    lightCurveStore.put(group, [...lightCurveDocsFromTso(result, computed.paths), ...(this.extraDocs.get(group) ?? NO_DOCS)]);
  }

  async compute(group: string, paths: readonly string[], series: SeriesKey, request: TsoLightCurvesRequest): Promise<void> {
    const seq = this.nextSequence();
    this.curvesSeq.set(group, seq);
    this.curves.set(group, { loading: true, result: this.getCurves(group).result, error: null });
    this.notify();
    try {
      const result = await this.readCurves([...paths], series, request);
      if (this.curvesSeq.get(group) !== seq) return;
      this.curves.set(group, { loading: false, result, error: null });
      this.computed.set(group, { paths: [...paths], request });
      this.writeDocs(group);
    } catch (e) {
      if (this.curvesSeq.get(group) !== seq) return;
      this.curves.set(group, { loading: false, result: this.getCurves(group).result, error: errorText(e) });
    }
    this.notify();
  }

  setExtraDocs(group: string, docs: readonly LightCurveDoc[]): void {
    this.extraDocs.set(group, [...docs]);
    this.writeDocs(group);
  }
}

export const tsoStore = new TsoStore();

export function useTsoSet(fileKey: string | null): TsoSetState {
  const getSnapshot = useCallback(() => (fileKey === null ? EMPTY_TSO_SET : tsoStore.getSet(fileKey)), [fileKey]);
  return useSyncExternalStore(tsoStore.subscribe, getSnapshot, getSnapshot);
}

export function useTsoChosen(fileKey: string | null): readonly string[] {
  const getSnapshot = useCallback(() => (fileKey === null ? NO_PATHS : tsoStore.getChosen(fileKey)), [fileKey]);
  return useSyncExternalStore(tsoStore.subscribe, getSnapshot, getSnapshot);
}

export function useTsoSeriesIndex(fileKey: string | null): number {
  const getSnapshot = useCallback(() => (fileKey === null ? 0 : tsoStore.getSeriesIndex(fileKey)), [fileKey]);
  return useSyncExternalStore(tsoStore.subscribe, getSnapshot, getSnapshot);
}

export function useTsoDraft(group: string | null): TsoCurvesDraft {
  const getSnapshot = useCallback(() => (group === null ? DEFAULT_TSO_DRAFT : tsoStore.getDraft(group)), [group]);
  return useSyncExternalStore(tsoStore.subscribe, getSnapshot, getSnapshot);
}

export function useTsoCurves(group: string | null): TsoCurvesState {
  const getSnapshot = useCallback(() => (group === null ? EMPTY_TSO_CURVES : tsoStore.getCurves(group)), [group]);
  return useSyncExternalStore(tsoStore.subscribe, getSnapshot, getSnapshot);
}
