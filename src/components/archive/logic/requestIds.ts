export const TOKEN_SAVE_ID = "token:save";
export const TOKEN_CHECK_ID = "token:check";

const REQUEST_ID = /^[A-Za-z0-9:._-]{1,512}$/;

export function generationPrefix(generation: number): string {
  return `g${generation}:`;
}

export function nonThumbPrefixes(generation: number): string[] {
  const g = generationPrefix(generation);
  return [`${g}search`, `${g}resolve`, `${g}page:`, `${g}list:`, `${g}preview:`, `${g}products:`];
}

export function searchId(generation: number): string {
  return `g${generation}:search`;
}

export function resolveId(generation: number): string {
  return `g${generation}:resolve`;
}

export function pageId(generation: number, offset: number): string {
  return `g${generation}:page:${offset}`;
}

export function listId(generation: number, datasetId: string, attempt: number): string {
  return `g${generation}:list:${datasetId}:a${attempt}`;
}

export function thumbId(generation: number, datasetId: string, filename: string | null): string {
  return filename ? `g${generation}:thumb:${datasetId}:${filename}` : `g${generation}:thumb:${datasetId}`;
}

export function previewId(generation: number, datasetId: string, filename: string, size: "pane" | "large", attempt: number): string {
  return `g${generation}:preview:${datasetId}:${filename}:${size}:a${attempt}`;
}

export function productsId(generation: number, datasetId: string): string {
  return `g${generation}:products:${datasetId}`;
}

export function generationOf(requestId: string): number | null {
  const m = /^g(\d+):/.exec(requestId);
  return m ? Number(m[1]) : null;
}

export function isValidRequestId(requestId: string): boolean {
  return REQUEST_ID.test(requestId);
}
