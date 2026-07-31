import { typedInvoke, getPreviewUrl } from "./client";
import { getOutputDir } from "./output";

export { typedInvoke, isTauri, getPreviewUrl } from "./client";
export { getOutputDir, getOutputDirTiles, getExportDir } from "./output";
export { parseRawPixelBuffer, toUint8Array, parseFftBuffer } from "./parsers";

async function resolveDir(explicit?: string): Promise<string> {
  if (explicit && explicit !== "./output") return explicit;
  return getOutputDir();
}

async function resolvePreview(
  res: Record<string, unknown>,
  key: string = "png_path",
  urlKey: string = "previewUrl",
): Promise<Record<string, string>> {
  const path = res[key];
  if (typeof path === "string" && path) return { [urlKey]: await getPreviewUrl(path) };
  return {};
}

async function withDirInvoke<T>(
  cmd: string,
  outputDir: string | undefined,
  args: Record<string, unknown> = {},
): Promise<T> {
  const dir = await resolveDir(outputDir);
  return typedInvoke<T>(cmd, { outputDir: dir, ...args });
}

export async function withPreview<T extends object>(
  cmd: string,
  outputDir: string | undefined,
  args: Record<string, unknown> = {},
  previews: [string, string][] = [["png_path", "previewUrl"]],
): Promise<T> {
  const res = await withDirInvoke<T & Record<string, unknown>>(cmd, outputDir, args);
  // Resolve all preview URLs in parallel and return a NEW object instead of
  // mutating the backend response in place (mutation breaks identity-based
  // memoization and can expose partially-populated objects between awaits).
  const resolved = await Promise.all(
    previews.map(([key, urlKey]) => resolvePreview(res, key, urlKey)),
  );
  return Object.assign({}, res, ...resolved) as T;
}
