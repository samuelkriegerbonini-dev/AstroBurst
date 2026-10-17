import { CheckCircle2 } from "lucide-react";
import { ResultGrid } from "../ui";
import type { CfaExtractResult } from "../../services/processing";
import { planeRows } from "./debayerModes";

interface CfaExtractResultViewProps {
  result: CfaExtractResult;
}

export function CfaExtractResultView({ result }: CfaExtractResultViewProps) {
  const scale = result.oiii_scale;
  return (
    <div className="flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
      <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
        <CheckCircle2 size={12} />
        Extraction Complete ({result.pattern}, {result.mode})
      </div>
      <ResultGrid columns={2} items={[
        { label: "Output", value: result.dimensions ? `${result.dimensions[0]}×${result.dimensions[1]}` : "--" },
        { label: "Time", value: `${result.elapsed_ms}ms` },
      ]} />
      <div className="flex flex-col gap-1">
        {planeRows(result).map((plane) => (
          <div key={plane.path} className="flex items-center gap-2 text-[10px]">
            <span className="font-bold w-8 text-orange-300">{plane.tag}</span>
            <span className="text-zinc-500 truncate" title={plane.path}>{plane.name}</span>
          </div>
        ))}
      </div>
      {scale && (
        <div className="text-[9px] text-zinc-500">
          OIII blue rescale: k = {scale.scale.toFixed(3)}, c = {scale.offset.toFixed(2)}
        </div>
      )}
      {result.trimmed && (
        <div className="text-[9px] text-amber-400/90">
          The frame has an odd number of rows or columns; the odd last row or column was trimmed.
        </div>
      )}
    </div>
  );
}
