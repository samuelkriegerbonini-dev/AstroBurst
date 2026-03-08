import { memo } from "react";
import ProgressBar from "./ProgressBar";

function GlobalProgress({ progress, isComplete }) {
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
