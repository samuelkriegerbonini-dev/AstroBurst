import { safeInvoke, getOutputDirTiles } from "../infrastructure/tauri";

export async function generateTiles(path: string, outputDir?: string, tileSize = 256) {
  const dir = outputDir || await getOutputDirTiles();
  return safeInvoke("generate_tiles", { path, outputDir: dir, tileSize });
}
