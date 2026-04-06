import { typedInvoke, getPreviewUrl } from "./client";
import { getOutputDir } from "./output";

export { typedInvoke, isTauri, getPreviewUrl } from "./client";
export { getOutputDir, getOutputDirTiles, getExportDir } from "./output";
export { parseRawPixelBuffer, toUint8Array, parseFftBuffer } from "./parsers";

async function resolveDir(explicit?: string): Promise<string> {
  if (explicit && explicit !== "./output") return explicit;
  return getOutputDir();
}

async function resolvePreview<T extends Record<string, any>>(
  res: T,
  key: string = "png_path",
  urlKey: string = "previewUrl",
): Promise<T> {
  if ((res as any)[key]) (res as any)[urlKey] = await getPreviewUrl((res as any)[key]);
  return res;
}

async function withDirInvoke<T>(
  cmd: string,
  outputDir: string | undefined,
  args: Record<string, unknown> = {},
): Promise<T> {
  const dir = await resolveDir(outputDir);
  return typedInvoke<T>(cmd, { outputDir: dir, ...args });
}

export async function withPreview<T extends Record<string, any>>(
  cmd: string,
  outputDir: string | undefined,
  args: Record<string, unknown> = {},
  previews: [string, string][] = [["png_path", "previewUrl"]],
): Promise<T> {
  const res = await withDirInvoke<T>(cmd, outputDir, args);
  for (const [key, urlKey] of previews) {
    await resolvePreview(res, key, urlKey);
  }
  return res;
}
