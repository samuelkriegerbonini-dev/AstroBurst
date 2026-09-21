export const TASK_PANIC_PREFIX = "Task join failed";

export function shouldRetryWithoutFullAnalysis(message: string): boolean {
  return message.trimStart().startsWith(TASK_PANIC_PREFIX);
}

export function combineAttemptErrors(firstMessage: string, retryMessage: string): string {
  if (retryMessage === firstMessage) return firstMessage;
  return `${firstMessage} (retry without full analysis also failed: ${retryMessage})`;
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
