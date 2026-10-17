import type { MastDownloadAction, MastDownloadInput, MastDownloadItem, MastError, MastMission, MastMissionId, MastOpensAs, MastProduct } from "../../../shared/types/mast";
import { isValidFitsFile } from "../../../utils/validation";
import { formatBytes } from "./productView";

export const CONFIRM_ABOVE_BYTES = 1_000_000_000;
export const NOT_VERIFIED_TEXT = "not verified against MAST";
export const ROMAN_DOWNLOADS_TEXT = "Roman downloads are not enabled yet.";
export const HLSP_NAME_TEXT = "Only HLSP files whose MAST URI uses letters, digits and . _ - / are downloaded";

export const ACTION_LABELS: Readonly<Record<MastDownloadAction, string>> = {
  resume: "Resume",
  retry: "Retry",
  replace: "Replace",
  redownload: "Re-download",
};

const OPENS_IN_WORKSPACE: readonly MastOpensAs[] = ["image", "cube", "spectrum", "ramp"];
const HLSP_PREFIX = "mast:HLSP/";
const ROMAN_PREFIX = "roman";
const PRODUCT_URI_MISSIONS: readonly MastMissionId[] = ["jwst", "hst", "tess"];
const DOWNLOADABLE_URI = /^mast:[A-Za-z0-9._/-]+$/;
const FIELD_PREFIX = /^[A-Za-z]+: /;

export interface DownloadCounts {
  downloading: number;
  queued: number;
  done: number;
  failed: number;
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

export function downloadsUnavailable(mission: MastMissionId): string | null {
  return mission.startsWith(ROMAN_PREFIX) ? ROMAN_DOWNLOADS_TEXT : null;
}

type MissionRef = Pick<MastMission, "id" | "label">;

export function productDownloadRefusal(product: Pick<MastProduct, "mastUri">, mission: MissionRef): string | null {
  const unavailable = downloadsUnavailable(mission.id);
  if (unavailable) return unavailable;
  if (product.mastUri.startsWith(HLSP_PREFIX)) return DOWNLOADABLE_URI.test(product.mastUri) ? null : HLSP_NAME_TEXT;
  return PRODUCT_URI_MISSIONS.includes(mission.id) ? null : `${mission.label} downloads are not enabled yet.`;
}

export function listDownloadRefusal(products: readonly Pick<MastProduct, "mastUri">[], mission: MissionRef): string | null {
  const unavailable = downloadsUnavailable(mission.id);
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
  return item.opensAs !== null && OPENS_IN_WORKSPACE.includes(item.opensAs) && isValidFitsFile(item.filename.toLowerCase());
}

export function downloadInput(product: MastProduct, program: string | null, replace = false): MastDownloadInput {
  const trimmed = program?.trim() ?? "";
  return {
    datasetId: product.datasetId,
    filename: product.filename,
    program: trimmed ? trimmed : null,
    sizeBytes: product.sizeBytes,
    uri: product.mastUri.startsWith(HLSP_PREFIX) ? product.mastUri : null,
    opensAs: product.opensAs,
    replace,
  };
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

function failureText(error: MastError): string {
  const message = error.code === "mast_bad_input" ? error.message.replace(FIELD_PREFIX, "") : error.message;
  const text = message.trim().replace(/[.\s]+$/, "");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export function downloadStatusText(item: MastDownloadItem): string {
  switch (item.state) {
    case "queued":
      return "Queued";
    case "downloading": {
      const share = percent(item);
      if (share === null || (item.received === 0 && item.total === null)) return "Starting";
      return `${share}% of ${sizeOf(item)}`;
    }
    case "done":
      if (item.reused) return item.verified ? "Already downloaded, checked against MAST" : `Already downloaded, ${NOT_VERIFIED_TEXT}`;
      return `Downloaded, ${sizeOf(item)}`;
    case "cancelled":
      return "Cancelled";
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
    total: items.length,
  };
}

export function downloadsPillText(counts: DownloadCounts): string {
  if (counts.downloading === 0 && counts.queued === 0) {
    if (counts.failed > 0) return `Downloads: ${counts.failed} failed`;
    return counts.done > 0 ? `Downloads: ${counts.done} done` : "Downloads";
  }
  const queued = counts.queued > 0 ? `, ${counts.queued} queued` : "";
  return `Downloads: ${counts.downloading} running${queued}`;
}
