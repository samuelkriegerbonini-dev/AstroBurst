import { memo } from "react";
import { useMeasurementSource } from "../../hooks/useAnalysisTarget";
import type { MeasurementTone } from "../../utils/analysisTarget";

const TONE_CLASS: Record<MeasurementTone, string> = {
  processed: "text-emerald-300/90 bg-emerald-600/15",
  original: "text-zinc-300 bg-zinc-700/40",
  composite: "text-amber-300 bg-amber-900/30",
};

function MeasurementBadge({ measuresComposite = false }: { measuresComposite?: boolean }) {
  const source = useMeasurementSource(measuresComposite);
  if (!source) return null;
  return (
    <span
      className={`text-[9px] px-1.5 py-0.5 rounded font-mono truncate max-w-35 normal-case tracking-normal ${TONE_CLASS[source.tone]}`}
      title={source.title}
    >
      {source.text}
    </span>
  );
}

export default memo(MeasurementBadge);
