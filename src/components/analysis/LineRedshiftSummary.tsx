import { memo, useMemo } from "react";
import { useMeasurementLog } from "../../hooks/useMeasurementLog";
import { lineRedshiftSummary } from "../../utils/lineRedshiftSummary";

interface LineRedshiftSummaryProps {
  fileName: string | null;
  source: string | null;
}

function LineRedshiftSummary({ fileName, source }: LineRedshiftSummaryProps) {
  const entries = useMeasurementLog();
  const summary = useMemo(() => lineRedshiftSummary(entries, fileName, source), [entries, fileName, source]);
  if (!summary) return null;
  return (
    <div data-line-redshift-summary className="text-[10px] font-mono flex flex-col gap-0.5">
      <span className="text-zinc-300">{summary.text}</span>
      <span className="text-zinc-500 break-words">{`lines: ${summary.labels.join(", ")}`}</span>
    </div>
  );
}

export default memo(LineRedshiftSummary);
