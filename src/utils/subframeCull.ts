import type { PipelineChannel } from "../shared/types/stacking";

export type SubframePanel = "pipeline" | "drizzle";
export interface SubframeSelection { accepted: string[]; rejected: string[]; weights?: Record<string, number>; fileKey: string; allFramesIn?: readonly SubframePanel[] }
export const SUBFRAME_NOTICE_NAMES = 5;

export function fileSetKey(paths: readonly string[]): string {
  return [...new Set(paths)].sort().join("\n");
}

export function liveSubframeSelection(selection: SubframeSelection | null, paths: readonly string[]): SubframeSelection | null {
  if (!selection) return null;
  return selection.fileKey === fileSetKey(paths) ? selection : null;
}

const NOTHING_CULLED: readonly string[] = Object.freeze([]);

export function panelRejectedPaths(selection: SubframeSelection | null, panel: SubframePanel): readonly string[] {
  if (!selection || selection.allFramesIn?.includes(panel)) return NOTHING_CULLED;
  return selection.rejected;
}

export function withAllFramesIn(selection: SubframeSelection | null, panel: SubframePanel): SubframeSelection | null {
  if (!selection || selection.allFramesIn?.includes(panel)) return selection;
  return { ...selection, allFramesIn: [...(selection.allFramesIn ?? []), panel] };
}

function keptPaths(paths: readonly string[], rejected: ReadonlySet<string>, excluded: Set<string>): string[] {
  const kept: string[] = [];
  for (const path of paths) {
    if (rejected.has(path)) excluded.add(path);
    else kept.push(path);
  }
  return kept;
}

export function cullChannelGroups(groups: readonly PipelineChannel[], rejected: readonly string[]): { groups: PipelineChannel[]; excluded: string[] } {
  const rejectedSet = new Set(rejected);
  const excluded = new Set<string>();
  const culled = groups.map((group) => ({ ...group, paths: keptPaths(group.paths, rejectedSet, excluded) }));
  return { groups: culled, excluded: [...excluded] };
}

export function pipelineChannelInputs(groups: readonly PipelineChannel[], rejected: readonly string[]): { inputs: PipelineChannel[]; excluded: string[] } {
  const culled = cullChannelGroups(groups, rejected);
  const inputs = culled.groups
    .filter((group) => group.paths.length > 0)
    .map((group) => ({ label: group.label, paths: group.paths }));
  return { inputs, excluded: culled.excluded };
}

export function cullDrizzleChannels<K extends string>(channels: Readonly<Record<K, string[]>>, rejected: readonly string[]): { channels: Record<K, string[]>; excluded: string[] } {
  const rejectedSet = new Set(rejected);
  const excluded = new Set<string>();
  const culled = {} as Record<K, string[]>;
  for (const key of Object.keys(channels) as K[]) {
    culled[key] = keptPaths(channels[key], rejectedSet, excluded);
  }
  return { channels: culled, excluded: [...excluded] };
}

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

export function subframeExclusionNotice(excluded: readonly string[]): string | null {
  if (excluded.length === 0) return null;
  const names = excluded.slice(0, SUBFRAME_NOTICE_NAMES).map(baseName).join(", ");
  const more = excluded.length - SUBFRAME_NOTICE_NAMES;
  return `${excluded.length} excluded by Subframe Selector: ${names}${more > 0 ? ` and ${more} more` : ""}`;
}
