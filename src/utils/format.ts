export function formatTime(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const remainder = s % 60;
  if (m === 0) return `${s}s`;
  return `${m}m ${remainder}s`;
}

export function formatThroughput(bytes: number, ms: number): string {
  if (ms <= 0 || bytes <= 0) return "—";
  const mbps = (bytes / 1024 / 1024) / (ms / 1000);
  return `${mbps.toFixed(1)} MB/s`;
}

export function generateId(): string {
  return `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

export function formatDecimalBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(2)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} MB`;
  if (bytes >= 1e3) return `${(bytes / 1e3).toFixed(1)} kB`;
  return `${bytes} B`;
}
