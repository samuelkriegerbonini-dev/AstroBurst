const PREFIX = "ab.layout.";

export function loadLayout(key: string, fallback: number, min: number, max: number): number {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    if (raw === null) return fallback;
    const n = Number(raw);
    if (!Number.isFinite(n)) return fallback;
    return Math.max(min, Math.min(max, n));
  } catch {
    return fallback;
  }
}

export function saveLayout(key: string, value: number): void {
  try {
    localStorage.setItem(PREFIX + key, String(Math.round(value)));
  } catch {
    /* storage unavailable (private mode) — layout just won't persist */
  }
}
