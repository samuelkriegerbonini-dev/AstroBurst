export function wizardStepStaleAfterCompositeWrite(wizardCompositeReady: boolean): "stretch" | null {
  return wizardCompositeReady ? "stretch" : null;
}
