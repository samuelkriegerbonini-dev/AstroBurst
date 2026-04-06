import { typedInvoke, getOutputDirTiles } from "../infrastructure/tauri";
import type { TileResult } from "../shared/types/tiles";

export async function generateTiles(path: string, outputDir?: string, tileSize = 256): Promise<TileResult> {
  const dir = outputDir || await getOutputDirTiles();
  return typedInvoke<TileResult>("generate_tiles", { path, outputDir: dir, tileSize });
}

export async function generateTilesRgb(outputDir?: string, tileSize = 256): Promise<TileResult> {
  const dir = outputDir || await getOutputDirTiles();
  return typedInvoke<TileResult>("generate_tiles_rgb", { outputDir: dir, tileSize });
}
