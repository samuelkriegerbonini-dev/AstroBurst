import { useMemo, useSyncExternalStore } from "react";
import { imageLabelOf, measurementLog, type MeasurementLogEntry, type MeasurementProvenance } from "../utils/measurementLog";
import { useAnalysisTarget, useMeasurementSource } from "./useAnalysisTarget";

export function useMeasurementLog(): readonly MeasurementLogEntry[] {
  return useSyncExternalStore(measurementLog.subscribe, measurementLog.getSnapshot, measurementLog.getSnapshot);
}

export function useMeasurementProvenance(measuresComposite = false): MeasurementProvenance {
  const { fileName } = useAnalysisTarget();
  const source = useMeasurementSource(measuresComposite);
  return useMemo(() => ({ file: fileName, image: imageLabelOf(source) }), [fileName, source]);
}
