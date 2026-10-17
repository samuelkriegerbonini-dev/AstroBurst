import { typedInvoke } from "../infrastructure/tauri";
import type { AppConfig, ApiKeyResult } from "../shared/types/config";

export type { AppConfig, ApiKeyResult } from "../shared/types/config";

export function getConfig(): Promise<AppConfig> {
  return typedInvoke<AppConfig>("get_config");
}

export function updateConfig(field: string, value: unknown): Promise<AppConfig> {
  return typedInvoke<AppConfig>("update_config", { field, value });
}

export function saveApiKey(key: string, service = "astrometry"): Promise<{ saved: boolean; service: string }> {
  return typedInvoke<{ saved: boolean; service: string }>("save_api_key", { key, service });
}

export function getApiKey(service = "astrometry"): Promise<ApiKeyResult> {
  return typedInvoke<ApiKeyResult>("get_api_key", { service });
}

export interface OutputDirInfo {
  output_dir: string;
  total_size: number;
  max_size?: number;
  file_count: number;
}

export interface CleanupOutputResult {
  cleaned_files: number;
  cleaned_bytes: number;
  cleaned_paths: string[];
  total_size: number;
  file_count: number;
  output_dir: string;
  elapsed_ms: number;
}

export interface CleanupHistoryResult {
  cleaned_files: number;
  cleaned_bytes: number;
  cleaned_paths: string[];
  replayable_nodes: number;
  total_size: number;
  file_count: number;
  history_root: string;
  elapsed_ms: number;
}

export type SweepOutcome<T> = { ok: true; result: T } | { ok: false; error: string };

export interface CleanupStorageResult {
  output: SweepOutcome<CleanupOutputResult>;
  history: SweepOutcome<CleanupHistoryResult>;
}

export function getOutputDirInfo(outputDir: string): Promise<OutputDirInfo> {
  return typedInvoke<OutputDirInfo>("get_output_dir_info", { outputDir });
}

export function cleanupOutput(outputDir: string, keep: readonly string[], maxSizeMb?: number): Promise<CleanupOutputResult> {
  return typedInvoke<CleanupOutputResult>("cleanup_output_cmd", { outputDir, maxSizeMb: maxSizeMb ?? null, keep: [...keep] });
}

export function cleanupHistory(historyRoot: string, maxSizeMb?: number): Promise<CleanupHistoryResult> {
  return typedInvoke<CleanupHistoryResult>("cleanup_history_cmd", { historyRoot, maxSizeMb: maxSizeMb ?? null });
}

async function settle<T>(sweep: () => Promise<T>): Promise<SweepOutcome<T>> {
  try {
    return { ok: true, result: await sweep() };
  } catch (e: unknown) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

export async function cleanupStorage(outputDir: string, historyRoot: string, keep: readonly string[]): Promise<CleanupStorageResult> {
  const output = await settle(() => cleanupOutput(outputDir, keep));
  const history = await settle(() => cleanupHistory(historyRoot));
  return { output, history };
}

export interface CleanupSummary {
  cleanedPaths: string[];
  cleanedFiles: number;
  cleanedBytes: number;
  replayableNodes: number;
  errors: string[];
}

export function summarizeCleanup({ output, history }: CleanupStorageResult): CleanupSummary {
  const done = [output, history].flatMap((sweep) => (sweep.ok ? [sweep.result] : []));
  return {
    cleanedPaths: done.flatMap((result) => result.cleaned_paths ?? []),
    cleanedFiles: done.reduce((sum, result) => sum + result.cleaned_files, 0),
    cleanedBytes: done.reduce((sum, result) => sum + result.cleaned_bytes, 0),
    replayableNodes: history.ok ? history.result.replayable_nodes : 0,
    errors: [
      ...(output.ok ? [] : [`Output cleanup failed: ${output.error}`]),
      ...(history.ok ? [] : [`History cleanup failed: ${history.error}`]),
    ],
  };
}
