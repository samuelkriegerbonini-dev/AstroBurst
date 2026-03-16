import { memo } from "react";

interface ErrorAlertProps {
  message: string | null;
}

function ErrorAlert({ message }: ErrorAlertProps) {
  if (!message) return null;

  return (
    <div className="ab-error-alert">
      {message}
    </div>
  );
}

export default memo(ErrorAlert);
