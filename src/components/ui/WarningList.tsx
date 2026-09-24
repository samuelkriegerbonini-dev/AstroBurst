import { memo } from "react";
import { AlertTriangle } from "lucide-react";

interface WarningListProps {
  warnings: readonly string[] | null | undefined;
}

function WarningList({ warnings }: WarningListProps) {
  if (!warnings || warnings.length === 0) return null;

  return (
    <>
      {warnings.map((warning) => (
        <div key={warning} className="flex items-start gap-2 text-[10px] text-amber-300 bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2">
          <AlertTriangle size={12} className="shrink-0 mt-0.5" />
          <span>{warning}</span>
        </div>
      ))}
    </>
  );
}

export default memo(WarningList);
