import { memo } from "react";

interface RunButtonProps {
  label: string;
  runningLabel?: string;
  running: boolean;
  disabled?: boolean;
  accent?: string;
  onClick: () => void;
}

function RunButton({
  label,
  runningLabel,
  running,
  disabled = false,
  accent = "teal",
  onClick,
}: RunButtonProps) {
  return (
    <button
      onClick={onClick}
      disabled={running || disabled}
      className="ab-run-btn"
      data-accent={accent}
      data-running={running}
      data-disabled={disabled}
    >
      {running ? (
        <span className="flex items-center justify-center gap-2">
          <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
          </svg>
          {runningLabel || label}
        </span>
      ) : (
        label
      )}
    </button>
  );
}

export default memo(RunButton);
