import { describe, expect, it } from "vitest";
import { computeMaxLevel, createTileSlotPool, readTilePyramid, tileSlotDir, tileUrl } from "../deepZoomTiles";

const convertFileSrc = (path: string) => `http://asset.localhost/${encodeURIComponent(path)}`;

describe("readTilePyramid", () => {
  it("uses the directory the backend reports, not the one requested", () => {
    const info = readTilePyramid(
      { base_dir: "C:\\Users\\me\\AppData\\Roaming\\com.astroburst.desktop\\output/tiles", original_width: 8000, original_height: 6000, levels: [{}, {}, {}] },
      "./output/tiles",
      1,
      1,
    );
    expect(info.dir).toBe("C:\\Users\\me\\AppData\\Roaming\\com.astroburst.desktop\\output/tiles");
    expect(info.width).toBe(8000);
    expect(info.height).toBe(6000);
    expect(info.levelCount).toBe(3);
  });

  it("accepts an output_dir field", () => {
    expect(readTilePyramid({ output_dir: "/data/tiles" }, "/req", 10, 20).dir).toBe("/data/tiles");
  });

  it("falls back to the request and the known dimensions", () => {
    expect(readTilePyramid(null, "/req/tiles", 5000, 4000)).toEqual({ dir: "/req/tiles", width: 5000, height: 4000, levelCount: null });
  });
});

describe("tileUrl", () => {
  it("matches an asset URL built from the full tile path", () => {
    const dir = "C:\\Users\\me\\AppData\\Roaming\\com.astroburst.desktop\\output/tiles";
    expect(tileUrl(convertFileSrc(dir), 3, 1, 2, 7)).toBe(`${convertFileSrc(`${dir}/3/1_2.png`)}?v=7`);
  });

  it("changes with the generation so a new pyramid is not served from cache", () => {
    const base = convertFileSrc("/tiles");
    expect(tileUrl(base, 0, 0, 0, 1)).not.toBe(tileUrl(base, 0, 0, 0, 2));
  });
});

describe("tile slot pool", () => {
  it("gives a new generation its own directory while a cancelled job is still writing", () => {
    const pool = createTileSlotPool();
    const cancelled = pool.acquire();
    pool.hold(cancelled);
    pool.release(cancelled);
    const next = pool.acquire();
    expect(next).not.toBe(cancelled);
    expect(tileSlotDir("/out/tiles", next)).not.toBe(tileSlotDir("/out/tiles", cancelled));
  });

  it("keeps a slot while its pyramid is on screen even after the job finished", () => {
    const pool = createTileSlotPool();
    const shown = pool.acquire();
    pool.hold(shown);
    pool.release(shown);
    expect(pool.acquire()).not.toBe(shown);
  });

  it("reuses a slot once its job and its viewer have both released it, so disk use stays bounded", () => {
    const pool = createTileSlotPool();
    for (let i = 0; i < 5; i++) {
      const slot = pool.acquire();
      pool.hold(slot);
      pool.release(slot);
      pool.release(slot);
      expect(slot).toBe(0);
    }
  });
});

describe("computeMaxLevel", () => {
  it("matches the backend level count", () => {
    expect(computeMaxLevel(8000, 6000, 256)).toBe(5);
    expect(computeMaxLevel(512, 100, 256)).toBe(1);
    expect(computeMaxLevel(200, 100, 256)).toBe(0);
  });
});
