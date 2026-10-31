import { DEFAULT_BINS, type FrequencyBin } from "./wizard";

export const PIPELINE_LANES = ["L", "R", "G", "B", "Ha", "OIII", "SII", "Mono"] as const;

export type PipelineLane = (typeof PIPELINE_LANES)[number];

export const MONO_LANE: PipelineLane = "Mono";

const BIN_OF_LANE: Record<PipelineLane, string> = { L: "l", R: "r", G: "g", B: "b", Ha: "ha", OIII: "oiii", SII: "sii", Mono: "l" };

const LANE_OF_BIN: Record<string, PipelineLane> = { l: "L", r: "R", g: "G", b: "B", ha: "Ha", oiii: "OIII", sii: "SII" };

const CUSTOM_BIN_COLOR = "#a1a1aa";

export interface ComposeMaster {
  label: string;
  fits_path: string;
}

export function laneForBin(binId: string | null): PipelineLane | null {
  return binId !== null && Object.hasOwn(LANE_OF_BIN, binId) ? LANE_OF_BIN[binId] : null;
}

function binIdForLane(label: string): string | null {
  return Object.hasOwn(BIN_OF_LANE, label) ? BIN_OF_LANE[label as PipelineLane] : null;
}

function binCandidates(label: string): string[] {
  const own = label.toLowerCase().replace(/[^a-z0-9_]/g, "");
  return [...new Set([binIdForLane(label), own])].filter((id): id is string => !!id);
}

function binTemplate(id: string, label: string): FrequencyBin {
  return DEFAULT_BINS.find((bin) => bin.id === id) ?? { id, label, shortLabel: label.slice(0, 5), color: CUSTOM_BIN_COLOR, files: [] };
}

export function binsFromMasters(masters: readonly ComposeMaster[], bins: readonly FrequencyBin[]): FrequencyBin[] {
  const next = bins.map((bin) => ({ ...bin, files: [] as string[] }));
  const filled = new Set<string>();
  const monoLast = [...masters.filter((m) => m.label !== MONO_LANE), ...masters.filter((m) => m.label === MONO_LANE)];
  for (const master of monoLast) {
    const id = binCandidates(master.label).find((candidate) => !filled.has(candidate));
    if (!id) continue;
    filled.add(id);
    const bin = next.find((b) => b.id === id);
    if (bin) bin.files = [master.fits_path];
    else next.push({ ...binTemplate(id, master.label), files: [master.fits_path] });
  }
  return next;
}
