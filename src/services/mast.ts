import { typedInvoke } from "../infrastructure/tauri";
import { TauriCommandError } from "../shared/types/errors";
import type {
  MastCacheInfo,
  MastDownloadInput,
  MastDownloadItem,
  MastError,
  MastErrorCode,
  MastErrorContext,
  MastFootprintInput,
  MastFootprintRegions,
  MastFootprintSet,
  MastImageRef,
  MastImageSet,
  MastLevelClass,
  MastMission,
  MastMissionId,
  MastPeek,
  MastPreset,
  MastPreviewRef,
  MastProductList,
  MastProgressEvent,
  MastQuery,
  MastResolved,
  MastSearchPage,
  MastTokenStatus,
} from "../shared/types/mast";

export const MAST_COMMANDS = [
  "mast_missions_cmd",
  "mast_probe_mission_cmd",
  "mast_resolve_cmd",
  "mast_search_cmd",
  "mast_image_list_cmd",
  "mast_thumbnail_cmd",
  "mast_preview_cmd",
  "mast_products_cmd",
  "mast_cancel_cmd",
  "mast_token_status_cmd",
  "mast_token_save_cmd",
  "mast_token_clear_cmd",
  "mast_cache_info_cmd",
  "mast_cache_clear_cmd",
  "mast_download_cmd",
  "mast_download_cancel_cmd",
  "mast_downloads_cmd",
  "mast_presets_cmd",
  "mast_footprints_cmd",
  "mast_footprints_reg_cmd",
  "mast_peek_cmd",
] as const;

export type MastCommand = (typeof MAST_COMMANDS)[number];

export const MAST_PROGRESS_EVENT = "mast:progress";

export const MAST_DOWNLOAD_EVENT = "mast:download";

export type MastCacheKind = "images" | "lists" | "names" | "all";

export function mastMissions(): Promise<MastMission[]> {
  return typedInvoke<MastMission[]>("mast_missions_cmd", {});
}

export function mastProbeMission(mission: MastMissionId): Promise<MastMission> {
  return typedInvoke<MastMission>("mast_probe_mission_cmd", { mission });
}

export function mastResolve(requestId: string, name: string): Promise<MastResolved | null> {
  return typedInvoke<MastResolved | null>("mast_resolve_cmd", { requestId, name });
}

export function mastSearch(requestId: string, query: MastQuery, refresh = false): Promise<MastSearchPage> {
  return typedInvoke<MastSearchPage>("mast_search_cmd", { requestId, query, refresh });
}

export function mastImageList(requestId: string, mission: MastMissionId, datasetId: string): Promise<MastImageSet> {
  return typedInvoke<MastImageSet>("mast_image_list_cmd", { requestId, mission, datasetId });
}

export function mastThumbnail(
  requestId: string,
  mission: MastMissionId,
  datasetId: string,
  levelClass: MastLevelClass,
  filename: string | null,
): Promise<MastImageRef> {
  return typedInvoke<MastImageRef>("mast_thumbnail_cmd", { requestId, mission, datasetId, levelClass, filename });
}

export function mastPreview(
  requestId: string,
  mission: MastMissionId,
  datasetId: string,
  filename: string,
  size: "pane" | "large",
): Promise<MastPreviewRef> {
  return typedInvoke<MastPreviewRef>("mast_preview_cmd", { requestId, mission, datasetId, filename, size });
}

export function mastProducts(requestId: string, mission: MastMissionId, datasetIds: string[], refresh = false): Promise<MastProductList> {
  return typedInvoke<MastProductList>("mast_products_cmd", { requestId, mission, datasetIds: [...datasetIds], refresh });
}

export function mastCancel(target: { requestIds?: string[]; prefix?: string }): Promise<number> {
  return typedInvoke<number>("mast_cancel_cmd", {
    requestIds: target.requestIds ? [...target.requestIds] : null,
    prefix: target.prefix ?? null,
  });
}

export function mastTokenStatus(requestId: string, check: boolean): Promise<MastTokenStatus> {
  return typedInvoke<MastTokenStatus>("mast_token_status_cmd", { requestId, check });
}

export function mastTokenSave(requestId: string, token: string): Promise<MastTokenStatus> {
  return typedInvoke<MastTokenStatus>("mast_token_save_cmd", { requestId, token });
}

export function mastTokenClear(): Promise<MastTokenStatus> {
  return typedInvoke<MastTokenStatus>("mast_token_clear_cmd", {});
}

export function mastCacheInfo(): Promise<MastCacheInfo> {
  return typedInvoke<MastCacheInfo>("mast_cache_info_cmd", {});
}

export function mastCacheClear(kind: MastCacheKind): Promise<MastCacheInfo> {
  return typedInvoke<MastCacheInfo>("mast_cache_clear_cmd", { kind });
}

export function mastDownload(requestId: string, mission: MastMissionId, items: readonly MastDownloadInput[], open: boolean): Promise<MastDownloadItem[]> {
  return typedInvoke<MastDownloadItem[]>("mast_download_cmd", { requestId, mission, items: items.map((item) => ({ ...item })), open });
}

export function mastDownloadCancel(requestId: string): Promise<number> {
  return typedInvoke<number>("mast_download_cancel_cmd", { requestId });
}

export function mastDownloads(): Promise<MastDownloadItem[]> {
  return typedInvoke<MastDownloadItem[]>("mast_downloads_cmd", {});
}

export function mastPresets(): Promise<MastPreset[]> {
  return typedInvoke<MastPreset[]>("mast_presets_cmd", {});
}

export function mastFootprints(path: string, items: readonly MastFootprintInput[]): Promise<MastFootprintSet> {
  return typedInvoke<MastFootprintSet>("mast_footprints_cmd", { path, items: items.map((item) => ({ ...item })) });
}

export function mastFootprintsReg(items: readonly MastFootprintInput[]): Promise<MastFootprintRegions> {
  return typedInvoke<MastFootprintRegions>("mast_footprints_reg_cmd", { items: items.map((item) => ({ ...item })) });
}

export function mastPeek(requestId: string, mission: MastMissionId, datasetId: string, filename: string, uri: string | null): Promise<MastPeek> {
  return typedInvoke<MastPeek>("mast_peek_cmd", { requestId, mission, datasetId, filename, uri });
}

export async function onMastDownload(cb: (item: MastDownloadItem) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<MastDownloadItem>(MAST_DOWNLOAD_EVENT, (event) => cb(event.payload));
}

export async function onMastProgress(cb: (event: MastProgressEvent) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<MastProgressEvent>(MAST_PROGRESS_EVENT, (event) => cb(event.payload));
}

export const MAST_ERROR_CODES: readonly MastErrorCode[] = [
  "mast_network",
  "mast_timeout",
  "mast_cancelled",
  "mast_rate_limited",
  "mast_server",
  "mast_rejected",
  "mast_unauthorized",
  "mast_no_image",
  "mast_not_deployed",
  "mast_decode",
  "mast_too_large",
  "mast_not_image",
  "mast_bad_input",
  "mast_token_rejected",
  "mast_cache",
  "mast_size_mismatch",
  "mast_busy",
  "mast_stale",
  "mast_quota",
  "mast_uri_refused",
  "mast_peek_incomplete",
];

const KNOWN_CODES: ReadonlySet<string> = new Set<string>(MAST_ERROR_CODES);

const NUMBER_KEYS = ["status", "retryAfterSec", "timeoutSec", "bytes", "consecutive", "attempts", "elapsedMs"] as const;
const STRING_KEYS = ["detail", "mission", "operation", "host"] as const;

function pickContext(raw: unknown): MastErrorContext {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return {};
  const src = raw as Record<string, unknown>;
  const out: Record<string, number | string> = {};
  for (const key of NUMBER_KEYS) {
    const v = src[key];
    if (typeof v === "number" && Number.isFinite(v)) out[key] = v;
  }
  for (const key of STRING_KEYS) {
    const v = src[key];
    if (typeof v === "string") out[key] = v;
  }
  return out as MastErrorContext;
}

function isMastErrorCode(code: unknown): code is MastErrorCode {
  return typeof code === "string" && KNOWN_CODES.has(code);
}

export function mastErrorOf(err: unknown): MastError {
  if (err instanceof TauriCommandError) {
    if (isMastErrorCode(err.code)) return { code: err.code, message: err.message, context: pickContext(err.context) };
    return { code: "mast_decode", message: err.message, context: {} };
  }
  if (typeof err === "object" && err !== null && !(err instanceof Error)) {
    const obj = err as Record<string, unknown>;
    const message = typeof obj.message === "string" ? obj.message : JSON.stringify(err);
    if (isMastErrorCode(obj.code)) return { code: obj.code, message, context: pickContext(obj.context) };
    return { code: "mast_decode", message, context: {} };
  }
  if (err instanceof Error) return { code: "mast_decode", message: err.message, context: {} };
  return { code: "mast_decode", message: String(err), context: {} };
}
