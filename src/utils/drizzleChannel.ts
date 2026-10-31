import { detectChannel, type ChannelSource } from "./channelMapping";
import { frameKindOf, isCalibrationKind, sortByFrameKind } from "./frameKind";

export type DrizzleChannel = "r" | "g" | "b";

export const DRIZZLE_CHANNELS: readonly DrizzleChannel[] = ["r", "g", "b"];

type DrizzleFrame = ChannelSource & { path: string };

type Assignments = Readonly<Record<string, DrizzleChannel>>;

const UNASSIGNED_FILTER_NAMES: readonly (readonly [string, string])[] = [
  ["ha", "Hα"],
  ["oiii", "OIII"],
  ["sii", "SII"],
  ["nii", "NII"],
  ["l", "L"],
];

const MONO_FILTER_LABELS: Readonly<Record<string, string>> = {
  l: "L",
  r: "R",
  g: "G",
  b: "B",
  ha: "Ha",
  oiii: "OIII",
  sii: "SII",
  nii: "NII",
};

function isDrizzleChannel(bin: string | null): bin is DrizzleChannel {
  return bin === "r" || bin === "g" || bin === "b";
}

export function monoDrizzleFilterLabel(frames: readonly ChannelSource[], channel: DrizzleChannel): string {
  const bins = new Set(frames.map((frame) => detectChannel(frame)));
  const [bin] = bins;
  const label = bins.size === 1 && bin ? MONO_FILTER_LABELS[bin] : undefined;
  return label ?? channel.toUpperCase();
}

export function autoDrizzleChannel(file: ChannelSource): DrizzleChannel | null {
  if (isCalibrationKind(frameKindOf(file.result?.header))) return null;
  const bin = detectChannel(file);
  return isDrizzleChannel(bin) ? bin : null;
}

export function withAutoChannels(assignments: Assignments, files: readonly DrizzleFrame[]): Record<string, DrizzleChannel> {
  const next = { ...assignments };
  for (const file of files) {
    if (next[file.path]) continue;
    const channel = autoDrizzleChannel(file);
    if (channel) next[file.path] = channel;
  }
  return next;
}

function joinedWithAnd(parts: readonly string[]): string {
  return parts.length < 2 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

export function unassignedFilterLine(files: readonly DrizzleFrame[], assignments: Assignments): string | null {
  const counts = new Map<string, number>();
  for (const file of sortByFrameKind(files).lights) {
    if (assignments[file.path]) continue;
    const bin = detectChannel(file);
    if (bin !== null) counts.set(bin, (counts.get(bin) ?? 0) + 1);
  }
  const counted = UNASSIGNED_FILTER_NAMES.flatMap(([bin, name]) => {
    const n = counts.get(bin) ?? 0;
    return n > 0 ? [`${n} ${name}`] : [];
  });
  if (counted.length === 0) return null;
  const total = UNASSIGNED_FILTER_NAMES.reduce((sum, [bin]) => sum + (counts.get(bin) ?? 0), 0);
  if (total === 1) return `${counted[0]} frame not assigned: assign it by hand`;
  const advice = counted.length === 1 ? "drizzle them in mono or assign by hand" : "drizzle each filter in mono or assign by hand";
  return `${joinedWithAnd(counted)} frames not assigned: ${advice}`;
}
