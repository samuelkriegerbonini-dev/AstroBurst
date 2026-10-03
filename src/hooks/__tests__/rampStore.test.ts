import { describe, it, expect } from "vitest";
import { RampStore } from "../useRampStore";

describe("RampStore", () => {
  it("starts at the first integration and notifies only on a change", () => {
    const store = new RampStore();
    let calls = 0;
    const off = store.subscribe(() => {
      calls += 1;
    });
    expect(store.getSnapshot()).toBe(0);
    store.setIntegration(2);
    store.setIntegration(2);
    expect(store.getSnapshot()).toBe(2);
    expect(calls).toBe(1);
    store.reset();
    expect(store.getSnapshot()).toBe(0);
    expect(calls).toBe(2);
    off();
  });

  it("refuses negative and fractional integrations", () => {
    const store = new RampStore();
    store.setIntegration(3);
    store.setIntegration(-1);
    expect(store.getSnapshot()).toBe(0);
    store.setIntegration(1.5);
    expect(store.getSnapshot()).toBe(0);
  });
});
