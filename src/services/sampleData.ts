import { typedInvoke } from "../infrastructure/tauri";

export interface SampleDataPaths {
  paths: string[];
  credit: string;
}

export function sampleDataPaths(): Promise<SampleDataPaths> {
  return typedInvoke<SampleDataPaths>("sample_data_paths_cmd");
}
