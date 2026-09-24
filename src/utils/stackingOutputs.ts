import type { ProcessedResult } from "../shared/types/preview";
import { samePath } from "./processingChain";

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? "";
}

export function frameStem(path: string): string {
  const stem = baseName(path)
    .replace(/\.(gz|fz|bz2)$/i, "")
    .replace(/\.[^.]*$/, "")
    .replace(/[^A-Za-z0-9._-]+/g, "_")
    .replace(/^\.+/, "");
  return stem || "frame";
}

function pad(value: number, width = 2): string {
  return String(value).padStart(width, "0");
}

export function stackOutputName(firstFramePath: string, frameCount: number, at: Date): string {
  const date = `${at.getFullYear()}${pad(at.getMonth() + 1)}${pad(at.getDate())}`;
  const time = `${pad(at.getHours())}${pad(at.getMinutes())}${pad(at.getSeconds())}-${pad(at.getMilliseconds(), 3)}`;
  return `${frameStem(firstFramePath)}_stack${frameCount}_${date}-${time}`;
}

export function framesLabel(prefix: string, frameCount: number): string {
  return `${prefix} · ${frameCount} frame${frameCount === 1 ? "" : "s"}`;
}

export function sourceLabel(prefix: string, sourcePath: string, currentPath: string): string {
  return samePath(sourcePath, currentPath) ? prefix : `${prefix} · ${baseName(sourcePath)}`;
}

export interface OutputRecipient {
  key: string;
  path: string;
}

export function resultsForRecipients(
  target: OutputRecipient,
  alsoShowing: readonly OutputRecipient[],
  output: Omit<ProcessedResult, "label">,
  labelFor: (recipientPath: string) => string,
): { key: string; result: ProcessedResult }[] {
  const recipients = [target, ...alsoShowing.filter((r) => r.key !== target.key)];
  return recipients.map((r) => ({ key: r.key, result: { ...output, label: labelFor(r.path) } }));
}

export function toDims(dims: readonly number[] | null | undefined): [number, number] | null {
  return dims && dims.length === 2 && dims[0] > 0 && dims[1] > 0 ? [dims[0], dims[1]] : null;
}

function withoutQuery(url: string): string {
  const cut = url.search(/[?#]/);
  return cut >= 0 ? url.slice(0, cut) : url;
}

export function showsOutput(
  processed: ProcessedResult | null | undefined,
  output: Pick<ProcessedResult, "fitsPath" | "previewUrl">,
): boolean {
  if (!processed) return false;
  if (processed.fitsPath !== null || output.fitsPath !== null) {
    return processed.fitsPath !== null && output.fitsPath !== null && samePath(processed.fitsPath, output.fitsPath);
  }
  return (
    processed.previewUrl !== null &&
    output.previewUrl !== null &&
    samePath(withoutQuery(processed.previewUrl), withoutQuery(output.previewUrl))
  );
}
