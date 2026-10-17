export const NO_WORKSPACE_TEXT = "Open an image in the workspace to draw footprints on it";

export interface ToggleAvailability {
  blocked: string | null;
  workspaceName: string | null;
  fileName: string | null;
  checking: boolean;
}

export function toggleDisabledReason({ blocked, workspaceName, fileName, checking }: ToggleAvailability): string | null {
  if (blocked) return blocked;
  if (!workspaceName) return NO_WORKSPACE_TEXT;
  if (!fileName) return checking ? `Reading the WCS of ${workspaceName}…` : `${workspaceName} has no celestial WCS`;
  return null;
}

export function sRegionNote(summary: string | null, availability: ToggleAvailability): string | null {
  return summary ?? toggleDisabledReason(availability);
}

export function toggleLabel(fileName: string | null): string {
  return `Show on ${fileName ?? "image"}`;
}

export function shownText(count: number, fileName: string): string {
  return `${count} ${count === 1 ? "footprint" : "footprints"} on ${fileName}`;
}

const FITS_SUFFIX = /\.(fits|fit|fts)(\.gz|\.fz)?$/i;

function stem(name: string): string {
  const base = FITS_SUFFIX.test(name) ? name.replace(FITS_SUFFIX, "") : name.replace(/\.[^.]+$/, "");
  return base || "image";
}

export function imageRegionFileName(imageName: string): string {
  return `${stem(imageName)}_mast_footprints.reg`;
}

export function skyRegionFileName(datasetId: string): string {
  return `${datasetId}_footprint.reg`;
}
