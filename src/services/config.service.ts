import { safeInvoke } from "../infrastructure/tauri";

export function getConfig() {
  return safeInvoke("get_config");
}

export function updateConfig(field: string, value: any) {
  return safeInvoke("update_config", { field, value });
}

export function saveApiKey(key: string, service?: string) {
  return safeInvoke("save_api_key", { key, service });
}

export function getApiKey() {
  return safeInvoke("get_api_key");
}
