import { typedInvoke } from "../infrastructure/tauri";

export type SampleState = "absent" | "partial" | "downloading" | "ready";

export interface SampleSetReport {
  id: string;
  title: string;
  credit: string;
  licence: string;
  demonstrates: string;
  total_bytes: number;
  file_count: number;
  state: SampleState;
  received_bytes: number;
  folder: string;
}

export interface SampleOpen {
  id: string;
  paths: string[];
  credit: string;
}

export interface SampleProgress {
  id: string;
  received_bytes: number;
  total_bytes: number;
  file: string;
}

export const SAMPLE_COMMANDS = ["sample_sets_cmd", "sample_download_cmd", "sample_cancel_cmd", "sample_open_cmd"] as const;

export const SAMPLE_DOWNLOAD_EVENT = "sample:download";

const CANCELLED = "cancelled";
const NOT_DOWNLOADED = "not downloaded";

export function listSampleSets(): Promise<SampleSetReport[]> {
  return typedInvoke<SampleSetReport[]>("sample_sets_cmd", {});
}

export function downloadSampleSet(id: string): Promise<SampleOpen> {
  return typedInvoke<SampleOpen>("sample_download_cmd", { id });
}

export function cancelSampleDownload(id: string): Promise<boolean> {
  return typedInvoke<boolean>("sample_cancel_cmd", { id });
}

export function openSampleSet(id: string): Promise<SampleOpen> {
  return typedInvoke<SampleOpen>("sample_open_cmd", { id });
}

export async function onSampleDownloadProgress(cb: (progress: SampleProgress) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<SampleProgress>(SAMPLE_DOWNLOAD_EVENT, (event) => cb(event.payload));
}

export function sampleErrorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

export function isSampleCancelled(err: unknown): boolean {
  return sampleErrorMessage(err) === CANCELLED;
}

export function isSampleNotDownloaded(err: unknown): boolean {
  return sampleErrorMessage(err) === NOT_DOWNLOADED;
}
