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
