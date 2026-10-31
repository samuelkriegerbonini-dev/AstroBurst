import type { HistoryDoc } from "../../shared/types/ops";
import type { MarkMode, MaskCreateResult, MaskDeleteResult, MaskExportResult, MaskKind, MaskSpec } from "../../shared/types/masks";
import type { MaskEntry } from "../../utils/maskStore";
import { opLabel } from "../../utils/historyModel";

export const MASKS_COMPOSITE_NOTE = "Masks apply to the Processing steps. Compose › Stretch and Compose › Adjust run without a mask in this version.";
export const HISTORY_LOADING_NOTE = "Loading this image's history…";
export const REGIONS_ON_COMPOSITE_NOTE = "Regions belong to a file's pixel grid. Show the file to make a regions mask.";
export const NO_REGIONS_REASON = "Draw at least one included region on the image first (Image › Regions).";
export const NO_MASK_FILE_REASON = "Pick a FITS file to read the mask from.";
export const RANGE_ORDER_REASON = "Low must be at most High.";
export const MARK_RANGE_ORDER_REASON = "Mark Low must be at most Mark High.";
export const NO_MASKS_NOTE = "No masks yet. A mask you create is listed here with its coverage.";
export const UNMEASURED_NOTE = "coverage not measured yet";
export const LUMINANCE_HINT =
  "Off: the luminance scaled from its minimum to its maximum. On: the luminance after the automatic screen stretch, so faint signal reaches higher mask values.";
export const RANGE_HINT =
  "Pixels whose luminance lies from Low to High get 1; Fuzz fades the mask to 0 over that many data units outside the range. Low and High are in the image's data units, as Image › Statistics shows them.";
export const STARS_HINT = "Each detected star is masked out to a radius of Growth × its FWHM, then fades to 0 over Softness pixels.";
export const MASK_FILE_HINT = "The file must have this image's size and orientation.";
export const FEATHER_HINT = "Gaussian blur of the mask; the 10 to 90% edge is about 2.6 σ wide";
export const DISPLAY_HINT = "Display only: the tint shows the mask over the image; steps use the mask as created.";
export const NO_MASK_LABEL = "No mask";
export const NO_RUN_MASKS_HINT = "No masks yet. Make one in the Masks tool to run this step on part of the image.";

export const MASK_KIND_LABELS: Record<MaskKind, string> = {
  luminance: "Luminance",
  range: "Range",
  stars: "Stars",
  regions: "Regions",
  file: "File",
};

export const MARK_MODE_LABELS: Record<MarkMode, string> = {
  nonzero: "Nonzero",
  zero: "Zero",
  nan: "NaN",
  nonnan: "Not NaN",
  range: "Range",
};

export const MARK_MODE_HINTS: Record<MarkMode, string> = {
  nonzero: "Marks the file's pixels that are not zero; NaN pixels stay unmarked.",
  zero: "Marks the file's pixels that are exactly zero.",
  nan: "Marks the file's NaN pixels.",
  nonnan: "Marks every pixel of the file that is not NaN.",
  range: "Marks the file's pixels from Mark Low to Mark High, both included.",
};

export function fileNameOf(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

export function formatCoverage(coverage: number): string {
  const percent = coverage * 100;
  if (percent <= 0) return "0%";
  if (percent < 0.01) return "< 0.01%";
  return `${percent.toFixed(2)}%`;
}

export function formatDims([cols, rows]: readonly [number, number]): string {
  return `${cols}×${rows}`;
}

function counted(count: number, singular: string, plural: string): string {
  return `${count} ${count === 1 ? singular : plural}`;
}

export function regionsSummary(included: number, excluded: number, skipped: number): string {
  const listed = `${counted(included, "included region", "included regions")} and ${counted(excluded, "excluded region", "excluded regions")} from Image › Regions. Excluded regions cut holes in the mask.`;
  if (skipped === 0) return listed;
  const left = skipped === 1 ? "1 line or point region has no area and is left out." : `${skipped} line or point regions have no area and are left out.`;
  return `${listed} ${left}`;
}

export function starsText(stars: number): string {
  return counted(stars, "star", "stars");
}

export function maskResultLine(result: MaskCreateResult): string {
  const parts = [result.mask_id, MASK_KIND_LABELS[result.kind], `coverage ${formatCoverage(result.coverage)}`, formatDims(result.dims)];
  if (result.stars !== null) parts.push(starsText(result.stars));
  return parts.join(" · ");
}

function modsSummary(spec: MaskSpec): string[] {
  const parts: string[] = [];
  if (spec.feather_px !== undefined && spec.feather_px > 0) parts.push(`feather ${spec.feather_px} px`);
  if (spec.invert) parts.push("inverted");
  return parts;
}

function recipeParts(spec: MaskSpec): string[] {
  switch (spec.kind) {
    case "luminance":
      return [spec.auto_stf ? "Luminance, auto STF" : "Luminance"];
    case "range":
      return [`Range ${spec.lo} to ${spec.hi}`, ...(spec.fuzz ? [`fuzz ${spec.fuzz}`] : [])];
    case "stars":
      return [`Stars at ${spec.sigma ?? 5}σ`, ...(spec.config?.luminance_protect ? ["luminance protection"] : [])];
    case "regions": {
      const excluded = spec.exclude_shapes?.length ?? 0;
      return [`Regions, ${spec.shapes.length} included`, ...(excluded > 0 ? [`${excluded} excluded`] : [])];
    }
    case "file": {
      const mark = spec.mark ?? "nonzero";
      const marked = mark === "range" ? `range ${spec.lo} to ${spec.hi}` : MARK_MODE_LABELS[mark].toLowerCase();
      return [`File ${fileNameOf(spec.path)}`, marked];
    }
  }
}

export function recipeSummary(spec: MaskSpec): string {
  return [...recipeParts(spec), ...modsSummary(spec)].join(", ");
}

export function maskLabel(entry: Pick<MaskEntry, "id" | "recipe">): string {
  return `${entry.id} · ${recipeSummary(entry.recipe)}`;
}

export function recordedMaskLabel(recipe: MaskSpec): string {
  return `Recorded · ${recipeSummary(recipe)}`;
}

export function unlistedMaskLabel(id: string, recipe: MaskSpec | null, deleted: boolean): string {
  const named = recipe ? maskLabel({ id, recipe }) : id;
  return `${named} (${deleted ? "deleted" : "not listed"})`;
}

export function unlistedMaskReason(id: string, deleted: boolean): string {
  return deleted
    ? `Mask ${id} was deleted. Pick another mask, or make it again in the Masks tool.`
    : `Mask ${id} is no longer listed for this image. Pick a mask again.`;
}

export function rerunKeepsMaskReason(node: string, masked: boolean): string {
  return masked ? `Re-running ${node} keeps the mask it used.` : `Re-running ${node} keeps it without a mask.`;
}

export function entryCoverageLine(entry: MaskEntry): string {
  if (entry.coverage === null) return UNMEASURED_NOTE;
  const parts = [`coverage ${formatCoverage(entry.coverage)}`];
  if (entry.dims) parts.push(formatDims(entry.dims));
  if (entry.stars !== null) parts.push(starsText(entry.stars));
  return parts.join(" · ");
}

function joinedNames(names: string[]): string {
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

function stepName(doc: Pick<HistoryDoc, "nodes"> | null, nodeId: string): string {
  const node = doc?.nodes.find((n) => n.id === nodeId);
  return node ? `${nodeId} (${opLabel(node.op)})` : nodeId;
}

export function deletedNotice(result: MaskDeleteResult, doc: Pick<HistoryDoc, "nodes"> | null): string {
  if (!result.kept_for_replay) return doc ? `Deleted ${result.mask_id}. No History step used it.` : `Deleted ${result.mask_id}.`;
  const users = result.used_by.map((id) => stepName(doc, id));
  const verb = users.length === 1 ? "uses" : "use";
  return `${result.mask_id} is no longer listed. ${joinedNames(users)} still ${verb} it, so its recipe stays in the history for replay.`;
}

export function restoredNotice(deletedId: string, restoredId: string): string {
  return deletedId === restoredId ? `Restored ${deletedId}.` : `Restored ${deletedId} as ${restoredId}.`;
}

export function exportedNotice(result: MaskExportResult): string {
  const kind = result.kind === "label" ? "labels" : "float";
  return `Wrote ${fileNameOf(result.fits_path)} (${kind}, BITPIX ${result.bitpix}, ${formatDims(result.dims)}) to the output folder.`;
}
