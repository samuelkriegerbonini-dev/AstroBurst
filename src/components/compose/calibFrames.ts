import type { CalibAssignment, CalibFrameSlot, ChannelFile } from "./SmartChannelMapper";

export function toggleCalibFrame(assignment: CalibAssignment, slot: CalibFrameSlot, file: ChannelFile): CalibAssignment {
  const frames = assignment[slot];
  const next = frames.some((f) => f.id === file.id) ? frames.filter((f) => f.id !== file.id) : [...frames, file];
  return { ...assignment, [slot]: next };
}

export function withAllFramesOfKind(assignment: CalibAssignment, slot: CalibFrameSlot, files: readonly ChannelFile[]): CalibAssignment {
  const present = new Set(assignment[slot].map((f) => f.id));
  const missing = files.filter((f) => f.frameKind === slot && !present.has(f.id));
  return missing.length === 0 ? assignment : { ...assignment, [slot]: [...assignment[slot], ...missing] };
}
