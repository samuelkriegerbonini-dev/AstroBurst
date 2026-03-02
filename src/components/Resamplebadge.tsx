import { Scaling } from "lucide-react";

interface ResampleBadgeProps {
  originalDims: [number, number];
  resampledDims: [number, number];
}

export default function ResampleBadge({ originalDims, resampledDims }: ResampleBadgeProps) {
  return (
    <div
      className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md bg-amber-500/10 border border-amber-500/20 text-amber-400 text-[10px] font-medium cursor-default"
      title={`Resampled from ${originalDims[0]}×${originalDims[1]} to ${resampledDims[0]}×${resampledDims[1]}`}
    >
      <Scaling size={11} />
      <span>
        {originalDims[0]}×{originalDims[1]} → {resampledDims[0]}×{resampledDims[1]}
      </span>
    </div>
  );
}
