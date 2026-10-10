import { typedInvoke } from "../infrastructure/tauri";

export interface ChannelLevelStats {
  path: string;
  median: number;
  mad: number;
  sigma: number;
  valid_count: number;
}

export async function measureChannelLevels(paths: string[]): Promise<ChannelLevelStats[]> {
  const res = await typedInvoke<{ levels: ChannelLevelStats[]; elapsed_ms: number }>("measure_channel_levels_cmd", { paths });
  return res.levels;
}
