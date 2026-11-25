import type { MastDownloadAction, MastDownloadInput, MastDownloadItem, MastError, MastMission, MastOpensAs, MastProduct } from "../../../shared/types/mast";
import { isTextTableFile, isValidFitsFile } from "../../../utils/validation";
import { cubeClickOpensCube } from "../../../utils/previewShell";
import type { DockLayout, DockToolId } from "../../../utils/dockLayout";
import { calibrationOf, formatBytes } from "./productView";

export const CONFIRM_ABOVE_BYTES = 1_000_000_000;
export const NOT_VERIFIED_TEXT = "not verified against MAST";
export const NO_URI_TEXT = "MAST gives no download address for this file";

export const ACTION_LABELS: Readonly<Record<MastDownloadAction, string>> = {
  resume: "Resume",
  retry: "Retry",
  replace: "Replace",
  redownload: "Re-download",
};

const OPENS_IN_WORKSPACE: readonly MastOpensAs[] = ["image", "cube", "spectrum", "ramp"];
const OPENS_SPECTRA_TOOL: readonly MastOpensAs[] = ["cube", "spectrum", "ramp"];
const MAST_URI_PREFIX = "mast:";
const DEFAULT_PRODUCT_URI = /^mast:[A-Za-z_]+\/product\/[^/]+$/;
const FIELD_PREFIX = /^[A-Za-z]+: /;
const MAST_FOLDER = /^(.*?[\\/]mast)[\\/]/i;

export interface DownloadCounts {
  downloading: number;
  queued: number;
  done: number;
  failed: number;
  stopped: number;
  total: number;
}

export function needsConfirmation(totalBytes: number): boolean {
  return totalBytes > CONFIRM_ABOVE_BYTES;
}

export function selectionBytes(products: readonly Pick<MastProduct, "sizeBytes">[]): number {
  return products.reduce((sum, product) => sum + product.sizeBytes, 0);
}

export function selectionKey(product: Pick<MastProduct, "datasetId" | "filename">): string {
  return `${product.datasetId}:${product.filename}`;
}

type MissionRef = Pick<MastMission, "id" | "label" | "capabilities"> & { note?: string | null; downloadsNote?: string | null };
type Capabilities = MastMission["capabilities"] & { downloads?: boolean };

export function downloadsOffered(mission: Pick<MastMission, "capabilities">): boolean {
  const caps = mission.capabilities as Capabilities;
  return typeof caps.downloads === "boolean" ? caps.downloads : caps.products;
}

export function downloadsUnavailable(mission: MissionRef): string | null {
  if (downloadsOffered(mission)) return null;
  const note = (mission.downloadsNote ?? mission.note)?.trim();
  return note ? `${mission.label} downloads are not offered: ${note.replace(/[.\s]+$/, "")}.` : `${mission.label} downloads are not offered by AstroBurst yet.`;
}

export function productDownloadRefusal(product: Pick<MastProduct, "mastUri">, mission: MissionRef): string | null {
  const unavailable = downloadsUnavailable(mission);
  if (unavailable) return unavailable;
  return product.mastUri.startsWith(MAST_URI_PREFIX) ? null : NO_URI_TEXT;
}

export function listDownloadRefusal(products: readonly Pick<MastProduct, "mastUri">[], mission: MissionRef): string | null {
  const unavailable = downloadsUnavailable(mission);
  if (unavailable || products.length === 0) return unavailable;
  const refusals = products.map((product) => productDownloadRefusal(product, mission));
  return refusals.every((refusal) => refusal !== null && refusal === refusals[0]) ? refusals[0] : null;
}

export interface PickedProducts {
  key: string;
  ids: ReadonlySet<string>;
}

export function picksAfterSend(current: PickedProducts, sentKey: string): PickedProducts {
  return current.key === sentKey ? { key: sentKey, ids: new Set() } : current;
}

export function openableDownload(item: Pick<MastDownloadItem, "opensAs" | "filename">): boolean {
  const filename = item.filename.toLowerCase();
  if (item.opensAs === "catalog") return isTextTableFile(filename);
  return item.opensAs !== null && OPENS_IN_WORKSPACE.includes(item.opensAs) && isValidFitsFile(filename);
}

export function archiveOpenTool(opensAs: MastOpensAs | null, layout: DockLayout): DockToolId | null {
  return opensAs !== null && OPENS_SPECTRA_TOOL.includes(opensAs) && cubeClickOpensCube(layout) ? "cube" : null;
}

export function downloadInput(product: MastProduct, program: string | null, replace = false): MastDownloadInput {
  const trimmed = program?.trim() ?? "";
  const calibration = calibrationOf(product);
  const input: MastDownloadInput = {
    datasetId: product.datasetId,
    filename: product.filename,
    program: trimmed ? trimmed : null,
    sizeBytes: product.sizeBytes,
    uri: product.mastUri.startsWith(MAST_URI_PREFIX) && !DEFAULT_PRODUCT_URI.test(product.mastUri) ? product.mastUri : null,
    opensAs: product.opensAs,
    replace,
  };
  return calibration ? ({ ...input, calibration } as MastDownloadInput) : input;
}

export function recoveryInput(item: MastDownloadItem): MastDownloadInput {
  return {
    datasetId: item.datasetId,
    filename: item.filename,
    program: item.program,
    sizeBytes: item.sizeBytes,
    uri: item.uri,
    opensAs: item.opensAs,
    replace: item.replace || item.action === "replace" || item.action === "redownload",
  };
}

function percent(item: MastDownloadItem): number | null {
  const total = item.total ?? (item.sizeBytes > 0 ? item.sizeBytes : null);
  if (!total) return null;
  return Math.min(100, Math.floor((item.received / total) * 100));
}

export function progressPercent(item: MastDownloadItem): number | null {
  return item.state === "downloading" ? percent(item) : null;
}

function sizeOf(item: MastDownloadItem): string {
  return formatBytes(item.total ?? item.sizeBytes);
}

export function partBytesOf(item: MastDownloadItem): number {
  const value = (item as MastDownloadItem & { partBytes?: unknown }).partBytes;
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : 0;
}

export function discardable(item: MastDownloadItem): boolean {
  return item.state !== "queued" && item.state !== "downloading" && item.state !== "done" && partBytesOf(item) > 0;
}

export function partialBytes(items: readonly MastDownloadItem[]): number {
  return items.filter(discardable).reduce((sum, item) => sum + partBytesOf(item), 0);
}

export function partialText(items: readonly MastDownloadItem[]): string | null {
  const parts = items.filter(discardable);
  if (parts.length === 0) return null;
  const files = parts.length === 1 ? "1 partial file" : `${parts.length} partial files`;
  return `${files}, ${formatBytes(partialBytes(items))} on disk, counted in the MAST download quota until resumed or discarded`;
}

export function downloadRoot(items: readonly Pick<MastDownloadItem, "path">[]): string | null {
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const m = MAST_FOLDER.exec(items[i].path);
    if (m) return m[1];
  }
  return null;
}

export function folderOf(path: string): string {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut > 0 ? path.slice(0, cut) : path;
}

function durationText(seconds: number): string {
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))} s left`;
  if (seconds < 3600) return `${Math.round(seconds / 60)} min left`;
  return `${(seconds / 3600).toFixed(1)} h left`;
}

export function rateText(item: MastDownloadItem, bytesPerSec: number | null): string | null {
  if (item.state !== "downloading" || bytesPerSec === null || !Number.isFinite(bytesPerSec) || bytesPerSec <= 0) return null;
  const speed = `${formatBytes(bytesPerSec)}/s`;
  const total = item.total ?? (item.sizeBytes > 0 ? item.sizeBytes : null);
  if (total === null || total <= item.received) return speed;
  return `${speed}, ${durationText((total - item.received) / bytesPerSec)}`;
}

function stoppedText(item: MastDownloadItem): string {
  const kept = partBytesOf(item) || item.received;
  const total = item.total ?? item.sizeBytes;
  if (kept <= 0 || item.action !== "resume") return "Cancelled";
  const share = total > 0 ? `${Math.min(99, Math.floor((kept / total) * 100))}% of ${formatBytes(total)}` : formatBytes(kept);
  return partBytesOf(item) > 0 ? `Interrupted at ${share}, the partial file is kept` : `Paused at ${share}`;
}

function failureText(error: MastError): string {
  const message = error.code === "mast_bad_input" ? error.message.replace(FIELD_PREFIX, "") : error.message;
  const text = message.trim().replace(/[.\s]+$/, "");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export function downloadStatusText(item: MastDownloadItem, bytesPerSec: number | null = null): string {
  switch (item.state) {
    case "queued":
      return "Queued";
    case "downloading": {
      const share = percent(item);
      if (share === null || (item.received === 0 && item.total === null)) return "Starting";
      const rate = rateText(item, bytesPerSec);
      return rate ? `${share}% of ${sizeOf(item)}, ${rate}` : `${share}% of ${sizeOf(item)}`;
    }
    case "done":
      if (item.reused) return item.verified ? "Already downloaded, checked against MAST" : `Already downloaded, ${NOT_VERIFIED_TEXT}`;
      return `Downloaded, ${sizeOf(item)}`;
    case "cancelled":
      return stoppedText(item);
    case "error":
      return item.error ? failureText(item.error) || "Failed" : "Failed";
  }
}

export function downloadCountsOf(items: readonly MastDownloadItem[]): DownloadCounts {
  const count = (test: (item: MastDownloadItem) => boolean) => items.filter(test).length;
  return {
    downloading: count((i) => i.state === "downloading"),
    queued: count((i) => i.state === "queued"),
    done: count((i) => i.state === "done"),
    failed: count((i) => i.state === "error"),
    stopped: count((i) => i.state === "cancelled" && i.action === "resume"),
    total: items.length,
  };
}

export function downloadsPillText(counts: DownloadCounts): string {
  if (counts.downloading === 0 && counts.queued === 0) {
    if (counts.failed > 0) return `Downloads: ${counts.failed} failed`;
    if (counts.stopped > 0) return `Downloads: ${counts.stopped} to resume`;
    return counts.done > 0 ? `Downloads: ${counts.done} done` : "Downloads";
  }
  const queued = counts.queued > 0 ? `, ${counts.queued} queued` : "";
  return `Downloads: ${counts.downloading} running${queued}`;
}
