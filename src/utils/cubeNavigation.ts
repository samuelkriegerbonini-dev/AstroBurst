import { formatAxisValue } from "./spectralAxis";
import { nearestChannel, pixelToAxisValue, type PlotMapping } from "./spectrumRange";

export const FRAME_STEP = 1;
export const FRAME_STEP_LARGE = 10;
export const FRAME_KEY_HINT = "ArrowLeft / ArrowRight step the displayed channel, Shift steps by 10, double-click jumps to a channel";

export type PlaybackSpeed = "fast" | "normal" | "slow";

export const PLAYBACK_SPEEDS: readonly { id: PlaybackSpeed; label: string; ms: number }[] = [
  { id: "fast", label: "Fast", ms: 50 },
  { id: "normal", label: "Normal", ms: 150 },
  { id: "slow", label: "Slow", ms: 400 },
];

export function playbackIntervalMs(speed: PlaybackSpeed): number {
  return PLAYBACK_SPEEDS.find((s) => s.id === speed)?.ms ?? PLAYBACK_SPEEDS[1].ms;
}

export function isPlaybackSpeed(value: string): value is PlaybackSpeed {
  return PLAYBACK_SPEEDS.some((s) => s.id === value);
}

export function clampFrame(idx: number, total: number): number {
  if (!Number.isFinite(total) || total <= 0) return 0;
  if (!Number.isFinite(idx)) return 0;
  return Math.min(Math.floor(total) - 1, Math.max(0, Math.round(idx)));
}

export function stepFrame(current: number, direction: -1 | 1, total: number, large: boolean): number {
  const step = large ? FRAME_STEP_LARGE : FRAME_STEP;
  return clampFrame(clampFrame(current, total) + direction * step, total);
}

export function frameFromKey(key: string, shiftKey: boolean, current: number, total: number): number | null {
  if (total <= 1) return null;
  if (key === "ArrowLeft") return stepFrame(current, -1, total, shiftKey);
  if (key === "ArrowRight") return stepFrame(current, 1, total, shiftKey);
  return null;
}

export function channelFromPlotPixel(px: number, m: PlotMapping, total: number): number | null {
  if (!Number.isFinite(px) || total <= 0) return null;
  const channel = nearestChannel(pixelToAxisValue(px, m), m);
  if (channel === null) return null;
  return clampFrame(channel, total);
}

export function frameAxisValue(idx: number, values: number[] | null | undefined): number | null {
  if (!values || idx < 0 || idx >= values.length) return null;
  const v = values[idx];
  return Number.isFinite(v) ? v : null;
}

export function formatFrameLabel(idx: number, total: number, axisValue: number | null, unit: string): string {
  const channel = `Channel ${idx + 1}/${total}`;
  if (axisValue === null || !Number.isFinite(axisValue) || !unit || unit === "ch") return channel;
  return `${channel} - ${formatAxisValue(axisValue, unit)} ${unit}`;
}

export function formatFrameDelta(hovered: number, displayed: number): string {
  const delta = hovered - displayed;
  if (delta === 0) return "displayed";
  return `${delta > 0 ? "+" : ""}${delta} from displayed`;
}

export function nextPlaybackFrame(current: number, total: number, loop: boolean): number | null {
  if (!Number.isFinite(total) || total <= 1) return null;
  const next = clampFrame(current, total) + 1;
  if (next < total) return next;
  return loop ? 0 : null;
}

export function playbackStartFrame(current: number, total: number): number | null {
  return nextPlaybackFrame(current, total, true);
}

export interface FrameLoadRequest {
  idx: number;
  withFits: boolean;
}

export function mergeFrameRequest(pending: FrameLoadRequest | null, next: FrameLoadRequest): FrameLoadRequest {
  if (pending && pending.idx === next.idx) return { idx: next.idx, withFits: pending.withFits || next.withFits };
  return next;
}

function pathHash(path: string): string {
  let hash = 5381;
  for (let i = 0; i < path.length; i++) hash = ((hash << 5) + hash + path.charCodeAt(i)) | 0;
  return (hash >>> 0).toString(36);
}

export function cubeFrameOutputPaths(filePath: string, idx: number): { png: string; fits: string } {
  const stem = `./output/cube_frame_${pathHash(filePath)}_${idx}`;
  return { png: `${stem}.png`, fits: `${stem}.fits` };
}
