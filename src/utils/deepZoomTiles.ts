export interface TilePyramidInfo {
  dir: string;
  width: number;
  height: number;
  levelCount: number | null;
}

function positiveInt(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : null;
}

export function readTilePyramid(
  result: unknown,
  requestedDir: string,
  fallbackWidth: number,
  fallbackHeight: number,
): TilePyramidInfo {
  const r = result !== null && typeof result === "object" ? (result as Record<string, unknown>) : {};
  const dir = [r.base_dir, r.output_dir].find((d): d is string => typeof d === "string" && d.length > 0) ?? requestedDir;
  return {
    dir,
    width: positiveInt(r.original_width) ?? fallbackWidth,
    height: positiveInt(r.original_height) ?? fallbackHeight,
    levelCount: Array.isArray(r.levels) && r.levels.length > 0 ? r.levels.length : null,
  };
}

export function computeMaxLevel(w: number, h: number, tileSize: number): number {
  let maxDim = Math.max(w, h);
  let level = 0;
  while (maxDim > tileSize) {
    maxDim = Math.ceil(maxDim / 2);
    level++;
  }
  return level;
}

export function tileUrl(dirUrl: string, level: number, x: number, y: number, version: number): string {
  return `${dirUrl}${encodeURIComponent(`/${level}/${x}_${y}.png`)}?v=${version}`;
}

export interface TileSlotPool {
  acquire: () => number;
  hold: (slot: number) => void;
  release: (slot: number) => void;
}

export function createTileSlotPool(): TileSlotPool {
  const holds = new Map<number, number>();
  return {
    acquire() {
      let slot = 0;
      while ((holds.get(slot) ?? 0) > 0) slot++;
      holds.set(slot, 1);
      return slot;
    },
    hold(slot) {
      holds.set(slot, (holds.get(slot) ?? 0) + 1);
    },
    release(slot) {
      const left = (holds.get(slot) ?? 0) - 1;
      if (left > 0) holds.set(slot, left);
      else holds.delete(slot);
    },
  };
}

export function tileSlotDir(tilesRoot: string, slot: number): string {
  return `${tilesRoot}/slot${slot}`;
}
