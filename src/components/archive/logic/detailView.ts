import type { MastError, MastImageEntry, MastImageSet, MastMission, MastMissionId, MastPreviewRef, MastProductList, MastQuickWcs, MastRow } from "../../../shared/types/mast";
import { dateText, exclusiveText, instrumentText, isPublic, levelBadge } from "./cardLabel";
import { errorView } from "./errorCopy";
import { formatBytes } from "./productView";
import { fieldText, orientationText, pixelScaleText } from "./quickWcs";
import { elapsedText } from "./resultsView";

export type PreviewSize = "pane" | "large";
export type PreviewPhase = "pending" | "download" | "decode" | "ready" | "failed";

export interface StripThumb {
  url: string;
  filename: string;
}

export interface DetailHeader {
  badges: string[];
  line: string;
  exclusive: string | null;
}

export interface TabLabels {
  previews: string;
  metadata: string;
  products: string;
}

export const QUICKLOOK_NOTE = "8-bit quicklook made by MAST, not science data";
export const EXCLUSIVE_PREVIEWS_TEXT = "MAST serves no preview of exclusive-access data to anonymous users.";
export const PRODUCTS_SLOW_NOTE = "MAST takes 20 to 30 s for listings it has not seen recently.";
export const PREVIEW_DWELL_MS = 400;
export const STRIP_LIMIT = 24;

const SIZE_EDGE: Record<PreviewSize, number> = { pane: 2048, large: 4096 };
const TOKEN_MISSIONS: readonly MastMissionId[] = ["roman", "roman_cgi"];
const BYTE_UNITS: readonly number[] = [1e12, 1e9, 1e6, 1e3];
const SEP = " · ";

export function previewKey(mission: MastMissionId, datasetId: string, filename: string, size: PreviewSize): string {
  return `${mission}:${datasetId}:${filename}:${size}`;
}

export function stripKey(datasetId: string, filename: string): string {
  return `${datasetId}:${filename}`;
}

export function parseStripKey(key: string): { datasetId: string; filename: string } | null {
  const at = key.indexOf(":");
  if (at <= 0 || at === key.length - 1) return null;
  return { datasetId: key.slice(0, at), filename: key.slice(at + 1) };
}

export function entryFilename(entry: MastImageEntry): string {
  return entry.previewFilename ?? entry.productFilename;
}

export function entryTitle(entry: MastImageEntry): string {
  const label = entry.label.trim();
  const suffix = entry.suffix.trim();
  if (!suffix || label.toLowerCase().endsWith(suffix.toLowerCase())) return label;
  return `${label} ${suffix}`;
}

export function previewsAllowed(row: Pick<MastRow, "access">, mission: MastMission): boolean {
  return mission.capabilities.images && (isPublic(row) || TOKEN_MISSIONS.includes(mission.id));
}

export function detailHeader(row: MastRow, missionLabel: string): DetailHeader {
  const badges = [missionLabel];
  const instrument = row.instrument?.trim();
  if (instrument) badges.push(instrument);
  if (row.expType) badges.push(row.expType);
  const level = levelBadge(row.levelClass);
  if (level) badges.push(level);
  else if (row.filter && row.filters.length <= 1) badges.push(row.filter);
  if (row.access !== null && isPublic(row)) badges.push("PUBLIC");
  const line = [row.program ? `Programme ${row.program}` : null, row.observation ? `Obs ${row.observation}` : null, row.target?.trim() || null].filter(Boolean).join(SEP);
  const exclusive = exclusiveText(row);
  return { badges, line, exclusive: exclusive ? exclusive.replace(/^Exclusive/, "EXCLUSIVE") : null };
}

export function tabLabels(input: { set: MastImageSet | null; products: MastProductList | null; previewsEnabled: boolean }): TabLabels {
  const previews = !input.previewsEnabled ? "Previews (none)" : input.set ? `Previews ${input.set.entries.length}` : "Previews";
  return { previews, metadata: "Metadata", products: input.products ? `Products ${input.products.totalCount}` : "Products" };
}

export function groupsOf(set: MastImageSet): MastImageSet["groups"] {
  const listed = set.groups.filter((g) => g.count > 0);
  if (listed.length) return listed;
  const keys = [...new Set(set.entries.map((e) => e.group))];
  return keys.map((key) => ({ key, label: key, count: set.entries.filter((e) => e.group === key).length }));
}

export function entriesInGroup(set: MastImageSet, group: string): MastImageEntry[] {
  return set.entries.filter((e) => e.group === group);
}

export function stepEntry(set: MastImageSet, currentKey: string, dir: 1 | -1): string | null {
  const current = set.entries.find((e) => e.key === currentKey);
  if (!current) return null;
  const list = entriesInGroup(set, current.group);
  const next = list[list.findIndex((e) => e.key === currentKey) + dir];
  return next ? next.key : null;
}

export function stripPosition(set: MastImageSet, entry: MastImageEntry): { index: number; count: number } {
  const list = entriesInGroup(set, entry.group);
  return { index: list.findIndex((e) => e.key === entry.key) + 1, count: list.length };
}

export function stripThumbKeys(datasetId: string, entries: readonly MastImageEntry[], card: StripThumb | undefined): string[] {
  return entries
    .filter((e): e is MastImageEntry & { thumbnailFilename: string } => !!e.thumbnailFilename && card?.filename !== e.thumbnailFilename)
    .slice(0, STRIP_LIMIT)
    .map((e) => stripKey(datasetId, e.thumbnailFilename));
}

export function stripThumbUrl(datasetId: string, entry: MastImageEntry, refs: Readonly<Record<string, StripThumb | undefined>>, card: StripThumb | undefined): string | null {
  if (!entry.thumbnailFilename) return null;
  const own = refs[stripKey(datasetId, entry.thumbnailFilename)];
  if (own) return own.url;
  return card && card.filename === entry.thumbnailFilename ? card.url : null;
}

export function sourceWcs(ref: Pick<MastPreviewRef, "wcs" | "width" | "source">): MastQuickWcs | null {
  if (!ref.wcs) return null;
  const factor = ref.source.width > 0 ? ref.width / ref.source.width : 1;
  return { ...ref.wcs, pixelScaleArcsec: ref.wcs.pixelScaleArcsec * factor };
}

function resized(ref: Pick<MastPreviewRef, "width" | "height" | "source">): boolean {
  return ref.width !== ref.source.width || ref.height !== ref.source.height;
}

export function largeNeeded(paneRef: Pick<MastPreviewRef, "width" | "height" | "source">): boolean {
  return resized(paneRef);
}

export function previewInfoLines(entry: MastImageEntry, ref: MastPreviewRef): string[] {
  const title = entryTitle(entry);
  const src = ref.source;
  const lines: string[] = resized(ref)
    ? [`${title}${SEP}${src.width} x ${src.height} source`, `shown ${ref.width} x ${ref.height}${SEP}${formatBytes(src.bytes)}`]
    : [`${title}${SEP}${ref.width} x ${ref.height}${SEP}${formatBytes(src.bytes)}`];
  const wcs = sourceWcs(ref);
  if (wcs) {
    lines.push(`${pixelScaleText(wcs)}${SEP}${fieldText(wcs)}`);
    const orientation = orientationText(wcs, "pane");
    if (orientation) lines.push(orientation);
  }
  lines.push(QUICKLOOK_NOTE);
  return lines;
}

export function previewAlt(entry: MastImageEntry, ref: MastPreviewRef): string {
  return `${entryTitle(entry)} preview, ${ref.source.width} by ${ref.source.height} pixels, shown at ${Math.max(ref.width, ref.height)}`;
}

function amountIn(received: number, total: number): string {
  const size = BYTE_UNITS.find((u) => total >= u);
  if (!size) return String(Math.max(0, Math.round(received)));
  const value = Math.max(0, received) / size;
  if (size >= 1e9) return value.toFixed(2);
  return value < 10 ? value.toFixed(1) : String(Math.round(value));
}

export function progressText(p: { received: number; total: number | null; startedAt: number }, now: number): string {
  const elapsed = elapsedText(now - p.startedAt);
  if (p.total !== null && p.total > 0) return `${amountIn(p.received, p.total)} of ${formatBytes(p.total)}${SEP}${elapsed}`;
  return `${formatBytes(p.received)}${SEP}${elapsed}`;
}

export function progressFraction(p: { received: number; total: number | null }): number | null {
  if (p.total === null || p.total <= 0) return null;
  return Math.min(1, Math.max(0, p.received / p.total));
}

export function previewStatusText(phase: PreviewPhase, entry: MastImageEntry, size: PreviewSize): string {
  switch (phase) {
    case "pending":
      return `Loading ${entry.label} preview`;
    case "download":
      return `Downloading ${entry.label} preview`;
    case "decode":
      return `Preparing a ${SIZE_EDGE[size]} px copy`;
    default:
      return "";
  }
}

export function listingText(datasetId: string, elapsedMs: number): string {
  return `Listing images for ${datasetId} … ${elapsedText(elapsedMs)}`;
}

export function productsListingText(datasetId: string, elapsedMs: number): string {
  return `Listing products for ${datasetId} … ${elapsedText(elapsedMs)}`;
}

export function previewFailureText(error: MastError, rowPublic: boolean): string {
  if (error.code === "mast_cancelled") return "Preview cancelled.";
  return errorView(error, { surface: "preview", rowPublic }).text;
}

export function listFailureText(error: MastError): string {
  return errorView(error, { surface: "list" }).text;
}

export function productsFailureText(error: MastError): string {
  return errorView(error, { surface: "products" }).text;
}

export function largeTitle(row: MastRow, missionLabel: string): string {
  const what = [missionLabel, instrumentText(row.instrument)].filter(Boolean).join(" ");
  return [row.target?.trim() || null, what, row.datasetId].filter(Boolean).join(SEP);
}

export function datasetPositionText(index: number, count: number): string {
  return `dataset ${index + 1} of ${count}`;
}

export function railImageLines(entry: MastImageEntry, ref: MastPreviewRef): string[] {
  const lines = [[entryTitle(entry), entry.level ? `level ${entry.level}` : null].filter(Boolean).join(SEP)];
  lines.push(`source ${ref.source.width} x ${ref.source.height}`, `shown ${ref.width} x ${ref.height}`, `${formatBytes(ref.source.bytes)} from MAST`);
  const wcs = sourceWcs(ref);
  if (wcs) {
    lines.push(`${pixelScaleText(wcs)}${SEP}${fieldText(wcs).replace(/^field /, "")}`);
    const orientation = orientationText(wcs, "rail");
    if (orientation) lines.push(orientation);
  }
  return lines;
}

export function railObservationLines(row: MastRow): string[] {
  const lines: string[] = [];
  const ids = [row.program ? `Programme ${row.program}` : null, row.observation ? `Obs ${row.observation}` : null].filter(Boolean).join(SEP);
  if (ids) lines.push(ids);
  const when = [dateText(row.dateObs) || null, row.expTimeS !== null && Number.isFinite(row.expTimeS) ? `${row.expTimeS.toFixed(1)} s` : null].filter(Boolean).join(SEP);
  if (when) lines.push(when);
  lines.push(QUICKLOOK_NOTE);
  return lines;
}
