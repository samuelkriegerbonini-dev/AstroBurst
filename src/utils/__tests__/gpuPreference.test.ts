import { describe, it, expect } from "vitest";
import { loadGpuPreference, saveGpuPreference, GPU_PREF_KEY } from "../gpuPreference";

function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { map.set(k, v); },
    map,
  };
}

describe("loadGpuPreference", () => {
  it("returns null when nothing is stored", () => {
    expect(loadGpuPreference(memoryStorage())).toBeNull();
  });

  it("round-trips true and false through saveGpuPreference", () => {
    const s = memoryStorage();
    saveGpuPreference(true, s);
    expect(s.map.get(GPU_PREF_KEY)).toBe("true");
    expect(loadGpuPreference(s)).toBe(true);
    saveGpuPreference(false, s);
    expect(loadGpuPreference(s)).toBe(false);
  });

  it("returns null for garbage values", () => {
    expect(loadGpuPreference(memoryStorage({ [GPU_PREF_KEY]: "yes" }))).toBeNull();
    expect(loadGpuPreference(memoryStorage({ [GPU_PREF_KEY]: "" }))).toBeNull();
    expect(loadGpuPreference(memoryStorage({ [GPU_PREF_KEY]: "1" }))).toBeNull();
  });

  it("returns null when the storage throws", () => {
    const throwing = { getItem: () => { throw new Error("blocked"); } };
    expect(loadGpuPreference(throwing)).toBeNull();
  });
});

describe("saveGpuPreference", () => {
  it("does not throw when the storage throws", () => {
    const throwing = { setItem: () => { throw new Error("quota"); } };
    expect(() => saveGpuPreference(true, throwing)).not.toThrow();
  });
});
