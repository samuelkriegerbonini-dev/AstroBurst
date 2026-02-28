import ProgressBar from "./ProgressBar";

export default function GlobalProgress({ progress, isComplete }) {
  const variant = isComplete ? "green" : "blue";

  return (
    <ProgressBar
      value={progress}
      variant={variant}
      height="h-2"
      className="w-full"
    />
  );
}
