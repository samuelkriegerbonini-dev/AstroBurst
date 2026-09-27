import type { CompositeChain } from "../shared/types/compositeChain";
import { EMPTY_COMPOSITE_CHAIN } from "./compositeChain";

let chainState: CompositeChain = EMPTY_COMPOSITE_CHAIN;
const listeners = new Set<() => void>();

function set(next: CompositeChain): void {
  if (next === chainState) return;
  chainState = next;
  for (const listener of listeners) listener();
}

export const compositeChainStore = {
  get: (): CompositeChain => chainState,
  set,
  update: (fn: (chain: CompositeChain) => CompositeChain): void => {
    set(fn(chainState));
  },
  reset: (): void => {
    set(EMPTY_COMPOSITE_CHAIN);
  },
  subscribe: (listener: () => void): (() => void) => {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
};
