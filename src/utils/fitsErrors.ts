import { tableOnlyFileHint, tableOnlyInventoryHint } from "./x1dCompare";

export const TASK_PANIC_PREFIX = "Task join failed";

export function shouldRetryWithoutFullAnalysis(message: string): boolean {
  return message.trimStart().startsWith(TASK_PANIC_PREFIX);
}

export function combineAttemptErrors(firstMessage: string, retryMessage: string): string {
  if (retryMessage === firstMessage) return firstMessage;
  return `${firstMessage} (retry without full analysis also failed: ${retryMessage})`;
}

export const ONE_D_ARRAY_VERDICT = "1D array, not an image";
export const ONE_D_ARRAY_HINT = "1D spectrum, 1D array";

const NO_IMAGE_HDU_MARKER = "no HDU in this file holds a 2D image";

function ingestVerdict(message: string): string {
  const end = message.indexOf(NO_IMAGE_HDU_MARKER);
  return end === -1 ? message : message.slice(0, end);
}

export function spectrum1dHint(message: string | null | undefined): string | null {
  if (!message) return null;
  const verdict = ingestVerdict(message);
  return (
    tableOnlyFileHint(verdict) ??
    tableOnlyInventoryHint(verdict, message.slice(verdict.length)) ??
    (verdict.includes(ONE_D_ARRAY_VERDICT) ? ONE_D_ARRAY_HINT : null)
  );
}

export function spectrumProbeFailure(ingestMessage: string, probeMessage: string): string {
  const failure = `opening it as a 1D spectrum also failed: ${probeMessage}`;
  const inventoryAt = ingestMessage.indexOf(NO_IMAGE_HDU_MARKER);
  if (inventoryAt === -1) return `${ingestMessage} (${failure})`;
  return `${ingestMessage.slice(0, inventoryAt)}${failure}; ${ingestMessage.slice(inventoryAt)}`;
}

export interface HduShape {
  index: number;
  extname: string | null;
  naxis: number;
  naxis3: number;
  has_data: boolean;
  ref: string;
}

export function selectCubePlaneHdu(extensions: HduShape[]): string | null {
  const cubes = extensions.filter((ext) => ext.has_data && ext.naxis >= 3 && ext.naxis3 > 1);
  if (cubes.length === 0) return null;
  const science = cubes.find((ext) => ext.extname?.trim().toUpperCase() === "SCI");
  return (science ?? cubes[0]).ref;
}

export function declaredPlaneCount(header: Record<string, string> | null | undefined): number {
  const raw = header?.NAXIS3;
  if (!raw) return 0;
  const parsed = parseInt(raw, 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function isCubePlaneResult(
  header: Record<string, string> | null | undefined,
  isRgb: boolean | undefined,
): boolean {
  return !isRgb && declaredPlaneCount(header) > 1;
}
