import { useSyncExternalStore } from "react";
import { cancelForcedPhotometry, forcedPhotometry } from "../services/forcedPhotometry";
import type { ForcedPhotometryRequest, ForcedPhotometryResult, ForcedSkyPosition } from "../shared/types/forcedPhotometry";
import {
  DEFAULT_FORCED_FIELDS,
  bandChoice,
  forcedRequest,
  positionsCsvText,
  requestKey,
  type ForcedBandChoice,
  type ForcedFields,
} from "../utils/forcedPhotometryRequest";

export interface ForcedState {
  fields: ForcedFields;
  running: boolean;
  error: string | null;
  result: ForcedPhotometryResult | null;
  resultKey: string | null;
  selected: number | null;
}

export const EMPTY_FORCED: ForcedState = Object.freeze({
  fields: DEFAULT_FORCED_FIELDS,
  running: false,
  error: null,
  result: null,
  resultKey: null,
  selected: null,
});

type Listener = () => void;
type RunRequest = (request: ForcedPhotometryRequest) => Promise<ForcedPhotometryResult>;
type CancelRequest = () => Promise<boolean>;

function sameMembers<T extends object>(current: T, patch: Partial<T>): boolean {
  return (Object.keys(patch) as (keyof T)[]).every((key) => Object.is(patch[key], current[key]));
}

export class ForcedPhotometryStore {
  private value: ForcedState = EMPTY_FORCED;
  private listeners = new Set<Listener>();
  private seq = 0;
  private readonly runRequest: RunRequest;
  private readonly cancelRequest: CancelRequest;

  constructor(run: RunRequest = forcedPhotometry, cancel: CancelRequest = cancelForcedPhotometry) {
    this.runRequest = run;
    this.cancelRequest = cancel;
  }

  getSnapshot = (): ForcedState => this.value;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private emit(next: ForcedState) {
    this.value = next;
    this.listeners.forEach((l) => l());
  }

  private setBands(bands: readonly ForcedBandChoice[]) {
    this.emit({ ...this.value, fields: { ...this.value.fields, bands } });
  }

  setFields(patch: Partial<Omit<ForcedFields, "bands">>): void {
    const fields = this.value.fields;
    if (sameMembers(fields, patch)) return;
    this.emit({ ...this.value, fields: { ...fields, ...patch } });
  }

  addBand(file: { name: string; path: string }): void {
    const bands = this.value.fields.bands;
    if (bands.some((b) => b.path === file.path)) return;
    this.setBands([...bands, bandChoice(file)]);
  }

  removeBand(path: string): void {
    const bands = this.value.fields.bands;
    if (!bands.some((b) => b.path === path)) return;
    this.setBands(bands.filter((b) => b.path !== path));
  }

  updateBand(path: string, patch: Partial<Omit<ForcedBandChoice, "path" | "fileName">>): void {
    const bands = this.value.fields.bands;
    const band = bands.find((b) => b.path === path);
    if (band === undefined || sameMembers(band, patch)) return;
    this.setBands(bands.map((b) => (b === band ? { ...b, ...patch } : b)));
  }

  loadPositions(positions: readonly ForcedSkyPosition[]): void {
    this.setFields({ positionsText: positionsCsvText(positions) });
  }

  async run(): Promise<void> {
    if (this.value.running) return;
    const plan = forcedRequest(this.value.fields);
    const request = plan.request;
    if (request === null) {
      this.emit({ ...this.value, error: plan.blockers[0] });
      return;
    }
    const seq = ++this.seq;
    this.emit({ ...this.value, running: true, error: null });
    try {
      const result = await this.runRequest(request);
      if (seq !== this.seq) return;
      this.emit({ ...this.value, result, resultKey: requestKey(request), selected: null, running: false });
    } catch (e: unknown) {
      if (seq !== this.seq) return;
      this.emit({ ...this.value, error: e instanceof Error ? e.message : String(e), running: false });
    }
  }

  async cancel(): Promise<void> {
    if (!this.value.running) return;
    await this.cancelRequest().catch(() => false);
  }

  select(index: number | null): void {
    const rows = this.value.result?.sources.length ?? 0;
    const next = index !== null && Number.isInteger(index) && index >= 0 && index < rows ? index : null;
    if (Object.is(next, this.value.selected)) return;
    this.emit({ ...this.value, selected: next });
  }

  reset(): void {
    this.seq++;
    if (this.value === EMPTY_FORCED) return;
    this.emit(EMPTY_FORCED);
  }
}

export const forcedPhotometryStore = new ForcedPhotometryStore();

export function useForcedPhotometry(): ForcedState {
  return useSyncExternalStore(forcedPhotometryStore.subscribe, forcedPhotometryStore.getSnapshot, forcedPhotometryStore.getSnapshot);
}
