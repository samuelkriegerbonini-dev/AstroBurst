import { memo } from "react";

export const SAVED_NOTICE_MS = 6000;

interface ExportNoticeProps {
  saved?: string | null;
  copied?: boolean;
  error?: string | null;
}

function ExportNotice({ saved = null, copied = false, error = null }: ExportNoticeProps) {
  return (
    <>
      {saved && <div data-export-saved className="text-[9px] text-emerald-400/90 break-all">Saved {saved}</div>}
      {copied && <span data-export-copied className="text-[10px] text-zinc-500">copied</span>}
      {error && (
        <p data-export-error role="alert" className="text-[10px] text-red-400/80 break-words">
          {error}
        </p>
      )}
    </>
  );
}

export default memo(ExportNotice);
