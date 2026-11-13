import { memo, useCallback, type ReactNode } from "react";
import { ClipboardCopy } from "lucide-react";
import { useTimedFlag } from "../../hooks/useTimedNotice";
import { SMALL_BUTTON_CLASS } from "./uiClasses";

export const COPY_CONFIRM_MS = 1500;
export const COPY_CONFIRM_LABEL = "copied";
export const COPY_STATUS_TEXT = "Copied";

export interface CopyCsvButtonProps {
  text: () => string | null;
  disabled?: boolean;
  title?: string;
  label?: string;
  className?: string;
  icon?: ReactNode;
  onError?: (message: string) => void;
}

function CopyCsvButton({
  text,
  disabled,
  title,
  label = "Copy CSV",
  className = SMALL_BUTTON_CLASS,
  icon = <ClipboardCopy size={10} />,
  onError,
}: CopyCsvButtonProps) {
  const [copied, flagCopied, clearCopied] = useTimedFlag(COPY_CONFIRM_MS);

  const handleClick = useCallback(() => {
    const csv = text();
    if (csv === null) return;
    navigator.clipboard.writeText(csv).then(flagCopied, (e: unknown) => {
      clearCopied();
      onError?.(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    });
  }, [text, flagCopied, clearCopied, onError]);

  return (
    <>
      <button type="button" className={className} disabled={disabled} title={title} onClick={handleClick}>
        {icon}
        {copied ? COPY_CONFIRM_LABEL : label}
      </button>
      <span role="status" className="sr-only" data-copy-status="">
        {copied ? COPY_STATUS_TEXT : ""}
      </span>
    </>
  );
}

export default memo(CopyCsvButton);
