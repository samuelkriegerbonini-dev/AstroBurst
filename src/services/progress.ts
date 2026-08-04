import { typedInvoke } from "../infrastructure/tauri";

export function cancelProgress(event: string): Promise<boolean> {
  return typedInvoke<boolean>("cancel_progress_cmd", { event });
}
