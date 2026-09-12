import { describe, it, expect } from "vitest";
import {
  STORAGE_PREFIX,
  serializeRegions,
  parseRegions,
  loadRegions,
  saveRegions,
  DEFAULT_REGION_PROPS,
} from "../regionPersistence";
import type { Region } from "../../shared/types/regions";

class MemoryStorage {
  data = new Map<string, string>();
  getItem(k: string): string | null {
    return this.data.has(k) ? (this.data.get(k) as string) : null;
  }
  setItem(k: string, v: string): void {
    this.data.set(k, v);
  }
}

const regions: Region[] = [
  { id: "a", shape: { shape: "circle", x: 1, y: 2, r: 3 }, props: { ...DEFAULT_REGION_PROPS, color: "red" }, backgroundId: "b" },
  { id: "b", shape: { shape: "annulus", x: 1, y: 2, r_inner: 4, r_outer: 6 }, props: { ...DEFAULT_REGION_PROPS }, backgroundId: null },
];

describe("regionPersistence", () => {
  it("round-trips through an in-memory storage under the prefixed key", () => {
    const storage = new MemoryStorage();
    saveRegions("file.fits", regions, storage);
    expect(storage.data.has(`${STORAGE_PREFIX}file.fits`)).toBe(true);
    expect(loadRegions("file.fits", storage)).toEqual(regions);
    expect(loadRegions("other.fits", storage)).toEqual([]);
  });

  it("returns [] for corrupt JSON and non-arrays", () => {
    expect(parseRegions("{not json")).toEqual([]);
    expect(parseRegions('{"regions":[]}')).toEqual([]);
    expect(parseRegions(null)).toEqual([]);
  });

  it("drops entries with invalid shapes or ids and defaults missing fields", () => {
    const text = JSON.stringify([
      { id: "ok", shape: { shape: "circle", x: 1, y: 1, r: 1 } },
      { id: "bad", shape: { shape: "circle", x: 1, y: 1 } },
      { id: "", shape: { shape: "circle", x: 1, y: 1, r: 1 } },
      { shape: { shape: "circle", x: 1, y: 1, r: 1 } },
      { id: "dangling", shape: { shape: "point", x: 0, y: 0 }, props: { color: 5, width: 2, include: false }, backgroundId: "missing" },
      { id: "ok", shape: { shape: "point", x: 9, y: 9 } },
    ]);
    const parsed = parseRegions(text);
    expect(parsed.map((r) => r.id)).toEqual(["ok", "dangling"]);
    expect(parsed[0].props).toEqual(DEFAULT_REGION_PROPS);
    expect(parsed[0].backgroundId).toBeNull();
    expect(parsed[1].props).toEqual({ color: null, width: 2, text: null, dash: null, include: false });
    expect(parsed[1].backgroundId).toBeNull();
  });

  it("never throws when the storage throws", () => {
    const throwing = {
      getItem: () => {
        throw new Error("quota");
      },
      setItem: () => {
        throw new Error("quota");
      },
    };
    expect(() => saveRegions("f", regions, throwing)).not.toThrow();
    expect(loadRegions("f", throwing)).toEqual([]);
    expect(() => saveRegions("f", regions, null)).not.toThrow();
    expect(JSON.parse(serializeRegions(regions))).toHaveLength(2);
  });
});
