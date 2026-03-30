import { safeInvoke } from "../infrastructure/tauri";
import type { AppConfig, ApiKeyResult } from "../shared/types/config.types";

export type { AppConfig, ApiKeyResult } from "../shared/types/config.types";

export async function getConfig(): Promise<AppConfig> {
  return safeInvoke("get_config");
}

export async function updateConfig(field: string, value: unknown): Promise<AppConfig> {
  return safeInvoke("update_config", { field, value });
}

export async function saveApiKey(key: string, service: string = "astrometry"): Promise<{ saved: boolean; service: string }> {
  return safeInvoke("save_api_key", { key, service });
}

export async function getApiKey(service: string = "astrometry"): Promise<ApiKeyResult> {
  return safeInvoke("get_api_key", { service });
}
