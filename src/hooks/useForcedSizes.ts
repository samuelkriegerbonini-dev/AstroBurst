import { useSyncExternalStore } from "react";
import { resolutionCheck } from "../services/resolution";
import type { ForcedPhotometryResult } from "../shared/types/forcedPhotometry";
import type { ResolutionCheck, ResolutionRequest, ResolutionVerdict } from "../shared/types/resolution";

export const FORCED_SIZE_BATCH = 16;

export interface ForcedSizeState {
  result: ForcedPhotometryResult | null;
  band: number | null;
  psfText: string | null;
  verdicts: readonly (ResolutionVerdict | null)[];
  checked: number;
  total: number;
  running: boolean;
  error: string | null;
}

export const EMPTY_FORCED_SIZES: ForcedSizeState = Object.freeze({
  result: null,
  band: null,
  psfText: null,
  verdicts: Object.freeze([]),
  checked: 0,
  total: 0,
  running: false,
  error: null,
});

export interface ForcedSizeRun {
  result: ForcedPhotometryResult;
  band: number;
  psfText: string;
  requests: readonly (ResolutionRequest | null)[];
}

type Listener = () => void;
type CheckRequest = (request: ResolutionRequest) => Promise<ResolutionCheck>;

export class ForcedSizeStore {
  private value: ForcedSizeState = EMPTY_FORCED_SIZES;
  private listeners = new Set<Listener>();
  private seq = 0;
  private readonly check: CheckRequest;

  constructor(check: CheckRequest = resolutionCheck) {
    this.check = check;
  }

  getSnapshot = (): ForcedSizeState => this.value;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private emit(next: ForcedSizeState) {
    this.value = next;
    this.listeners.forEach((l) => l());
  }

  async run(input: ForcedSizeRun): Promise<void> {
    const seq = ++this.seq;
    const pending = input.requests.flatMap((request, index) => (request === null ? [] : [{ index, request }]));
    this.emit({
      result: input.result,
      band: input.band,
      psfText: input.psfText,
      verdicts: input.requests.map(() => null),
      checked: 0,
      total: pending.length,
      running: true,
      error: null,
    });
    try {
      for (let start = 0; start < pending.length; start += FORCED_SIZE_BATCH) {
        const batch = pending.slice(start, start + FORCED_SIZE_BATCH);
        const checks = await Promise.all(batch.map((item) => this.check(item.request)));
        if (seq !== this.seq) return;
        const verdicts = [...this.value.verdicts];
        batch.forEach((item, k) => {
          verdicts[item.index] = checks[k].verdict;
        });
        this.emit({ ...this.value, verdicts, checked: this.value.checked + batch.length });
      }
      this.emit({ ...this.value, running: false });
    } catch (e: unknown) {
      if (seq !== this.seq) return;
      this.emit({ ...this.value, running: false, error: e instanceof Error ? e.message : String(e) });
    }
  }

  clear(): void {
    this.seq++;
    if (this.value === EMPTY_FORCED_SIZES) return;
    this.emit(EMPTY_FORCED_SIZES);
  }
}

export const forcedSizeStore = new ForcedSizeStore();

export function useForcedSizes(): ForcedSizeState {
  return useSyncExternalStore(forcedSizeStore.subscribe, forcedSizeStore.getSnapshot, forcedSizeStore.getSnapshot);
}
