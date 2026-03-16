import { safeInvoke, getPreviewUrl } from "./client";
import { getOutputDir } from "./output";

export { safeInvoke, getPreviewUrl, isTauri } from "./client";
export { getOutputDir, getOutputDirTiles } from "./output";
export { parseRawPixelBuffer, toUint8Array, parseFftBuffer } from "./parsers";

async function resolveDir(explicit?: string): Promise<string> {
  if (explicit && explicit !== "./output") return explicit;
  return getOutputDir();
}

async function resolvePreview(res: any, key = "png_path", urlKey = "previewUrl"): Promise<any> {
  if (res[key]) res[urlKey] = await getPreviewUrl(res[key]);
  return res;
}

export async function withDirInvoke(
  cmd: string,
  outputDir: string | undefined,
  args: Record<string, any> = {},
): Promise<any> {
  const dir = await resolveDir(outputDir);
  return safeInvoke(cmd, { outputDir: dir, ...args });
}

export async function withPreview(
  cmd: string,
  outputDir: string | undefined,
  args: Record<string, any> = {},
  previews: [string, string][] = [["png_path", "previewUrl"]],
): Promise<any> {
  const res = await withDirInvoke(cmd, outputDir, args);
  for (const [key, urlKey] of previews) {
    await resolvePreview(res, key, urlKey);
  }
  return res;
}
