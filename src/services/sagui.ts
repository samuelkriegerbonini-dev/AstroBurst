import { listen } from "@tauri-apps/api/event";
import { typedInvoke } from "../infrastructure/tauri";
import { cancelProgress } from "./progress";
import type { SaguiConfig, SaguiInputInfo, SaguiInstallation, SaguiProgress, SaguiResult, SaguiRuntimeInfo, SaguiSavedResult } from "../shared/types/sagui";

export function loadSaguiResult(manifestPath: string): Promise<SaguiSavedResult> {
  return typedInvoke("load_sagui_result_cmd", { manifestPath });
}

export function inspectSaguiInput(path: string): Promise<SaguiInputInfo> {
  return typedInvoke("inspect_sagui_input_cmd", { path });
}

export function inspectSaguiRuntime(rscriptPath: string, libraryPath: string): Promise<SaguiRuntimeInfo> {
  return typedInvoke("inspect_sagui_runtime_cmd", { rscriptPath, libraryPath });
}

export function runSagui(config: SaguiConfig, runId: string): Promise<SaguiResult> {
  return typedInvoke("run_sagui_cmd", { config, runId });
}

export function onSaguiProgress(runId: string, callback: (progress: SaguiProgress) => void): Promise<() => void> {
  return listen<SaguiProgress>(`sagui-progress-${runId}`, (event) => callback(event.payload));
}

export function cancelSagui(runId: string): Promise<boolean> {
  return cancelProgress(`sagui-progress-${runId}`);
}

export function installSaguiRuntime(rscriptPath: string, outputDir: string, runId: string): Promise<SaguiInstallation> {
  return typedInvoke("install_sagui_runtime_cmd", { rscriptPath, outputDir, runId });
}

export function onSaguiInstallProgress(runId: string, callback: (progress: SaguiProgress) => void): Promise<() => void> {
  return listen<SaguiProgress>(`sagui-install-progress-${runId}`, (event) => callback(event.payload));
}

export function cancelSaguiInstall(runId: string): Promise<boolean> {
  return cancelProgress(`sagui-install-progress-${runId}`);
}
