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

export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}
