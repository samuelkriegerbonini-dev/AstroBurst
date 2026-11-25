type ResetListener = (restored: boolean) => void;

const listeners = new Set<ResetListener>();

export const compositeChainResets = {
  notify: (restored: boolean): void => {
    for (const listener of listeners) listener(restored);
  },
  subscribe: (listener: ResetListener): (() => void) => {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
};
