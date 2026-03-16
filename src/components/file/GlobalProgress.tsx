import { memo } from "react";
import ProgressBar from "./ProgressBar";

interface GlobalProgressProps {
  progress: number;
  isComplete: boolean;
}

function GlobalProgress({ progress, isComplete }: GlobalProgressProps) {
  return (
    <ProgressBar
      value={progress}
      variant={isComplete ? "green" : "blue"}
      height="h-1.5"
      className="w-full"
    />
  );
}

export default memo(GlobalProgress);
