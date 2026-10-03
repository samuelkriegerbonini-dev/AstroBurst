import { useEffect, useRef } from "react";
import { useCubeContext } from "../../context/PreviewContext";
import { dockStore } from "../../hooks/useDockLayout";
import { analysisRouteDue, nextAnalysisRoute, rememberAnalysisChoice, type AnalysisRouteTool } from "../../utils/analysisSections";
import type { DockLayout } from "../../utils/dockLayout";

const analysisChoices = new Map<string, AnalysisRouteTool>();

export function useAnalysisRouting(active: DockLayout["active"], fileKey: string | null): void {
  const { isCube, ramp, cubeFileKey } = useCubeContext();
  const seenActive = useRef(active);
  const routedActive = useRef<DockLayout["active"] | null>(null);
  const routedKey = useRef<string | null>(null);

  useEffect(() => {
    if (active !== routedActive.current) rememberAnalysisChoice(analysisChoices, fileKey, seenActive.current, active);
    seenActive.current = active;
  }, [active, fileKey]);

  useEffect(() => {
    if (fileKey === null) {
      routedKey.current = null;
      return;
    }
    if (!analysisRouteDue({ fileKey, flagsKey: cubeFileKey, routedKey: routedKey.current })) return;
    routedKey.current = fileKey;
    const steps = nextAnalysisRoute({ layout: dockStore.get(), fileKey, isCube, isRamp: ramp !== null, memory: analysisChoices });
    if (steps.length === 0) return;
    for (const { tool } of steps) dockStore.dispatch({ type: "open", tool });
    routedActive.current = dockStore.get().active;
  }, [fileKey, cubeFileKey, isCube, ramp]);
}
