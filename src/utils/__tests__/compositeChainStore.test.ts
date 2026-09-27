import { describe, it, expect, beforeEach } from "vitest";
import { compositeChainStore } from "../compositeChainStore";
import { EMPTY_COMPOSITE_CHAIN } from "../compositeChain";

describe("compositeChainStore", () => {
  beforeEach(() => {
    compositeChainStore.reset();
  });

  it("starts empty and notifies subscribers on every change", () => {
    let notified = 0;
    const unsubscribe = compositeChainStore.subscribe(() => {
      notified += 1;
    });
    expect(compositeChainStore.get()).toBe(EMPTY_COMPOSITE_CHAIN);
    compositeChainStore.update((c) => ({ ...c, psfKernel: [[1]] }));
    expect(compositeChainStore.get().psfKernel).toEqual([[1]]);
    expect(notified).toBe(1);
    compositeChainStore.reset();
    expect(compositeChainStore.get()).toBe(EMPTY_COMPOSITE_CHAIN);
    expect(notified).toBe(2);
    unsubscribe();
    compositeChainStore.update((c) => ({ ...c, psfKernel: [[2]] }));
    expect(notified).toBe(2);
  });

  it("skips notifications when the chain object is unchanged", () => {
    let notified = 0;
    const unsubscribe = compositeChainStore.subscribe(() => {
      notified += 1;
    });
    compositeChainStore.update((c) => c);
    compositeChainStore.set(compositeChainStore.get());
    expect(notified).toBe(0);
    unsubscribe();
  });
});
