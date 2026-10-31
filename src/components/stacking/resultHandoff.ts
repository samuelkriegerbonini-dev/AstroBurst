import { ingestUnlessLoaded, showWhenLoaded } from "../../hooks/useFileIngest";
import { dockStore } from "../../hooks/useDockLayout";
import { binsFromMasters, type ComposeMaster } from "../../utils/pipelineToCompose";
import type { WizardAction } from "../../context/ComposeWizardContext";
import type { WizardState } from "../../utils/wizard";

export interface ComposeWizardHandle {
  getState: () => Pick<WizardState, "bins">;
  dispatch: (action: WizardAction) => void;
  setActiveStep: (step: string) => void;
}

export function openInFiles(paths: readonly string[]): void {
  const [first, ...rest] = paths;
  if (!first) return;
  showWhenLoaded(first);
  for (const path of rest) ingestUnlessLoaded(path);
}

export function composeMasters(masters: readonly ComposeMaster[], wizard: ComposeWizardHandle): void {
  openInFiles(masters.map((master) => master.fits_path));
  wizard.dispatch({ type: "SET_BINS", bins: binsFromMasters(masters, wizard.getState().bins) });
  wizard.setActiveStep("channels");
  dockStore.dispatch({ type: "open", tool: "compose" });
}
