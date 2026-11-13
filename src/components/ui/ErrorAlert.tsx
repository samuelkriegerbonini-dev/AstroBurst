import { memo } from "react";

interface ErrorAlertProps {
  message: string | null;
  compact?: boolean;
  className?: string;
}

function ErrorAlert({ message, compact = false, className }: ErrorAlertProps) {
  if (!message) return null;

  const classes = `ab-error-alert${compact ? " ab-error-alert-compact" : ""}${className ? ` ${className}` : ""}`;

  return (
    <div className={classes} role="alert">
      {message}
    </div>
  );
}

export default memo(ErrorAlert);
