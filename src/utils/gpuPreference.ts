export const GPU_PREF_KEY = "astroburst.gpu.v1";

export function loadGpuPreference(storage: Pick<Storage, "getItem"> = localStorage): boolean | null {
  try {
    const raw = storage.getItem(GPU_PREF_KEY);
    if (raw === "true") return true;
    if (raw === "false") return false;
    return null;
  } catch {
    return null;
  }
}

export function saveGpuPreference(value: boolean, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(GPU_PREF_KEY, value ? "true" : "false");
  } catch {
  }
}
