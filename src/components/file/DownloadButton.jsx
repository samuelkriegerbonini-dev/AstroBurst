import { memo } from "react";
import { Download, Check, Loader2 } from "lucide-react";
import ProgressBar from "./ProgressBar";

function DownloadButton({
                          files,
                          onExport,
                          isExporting,
                          progress,
                          downloaded,
                          doneCount,
                          isComplete,
                        }) {
  const disabled = doneCount === 0 || isExporting;

  return (
    <div className="relative">
      <button
        onClick={() => onExport(files)}
        disabled={disabled}
        className={`
          flex items-center gap-1.5 rounded-md px-3 py-1.5 font-medium transition-all duration-150 text-xs
          ${
          isComplete && !isExporting && !downloaded
            ? "bg-green-500 hover:bg-green-600 text-white animate-pulse-glow"
            : disabled
              ? "bg-zinc-800 text-zinc-600 cursor-not-allowed"
              : "bg-blue-500 hover:bg-blue-600 text-white"
        }
        `}
      >
        {downloaded ? (
          <span className="flex items-center gap-1.5 animate-fade-in">
            <Check size={13} />
            Downloaded
          </span>
        ) : isExporting ? (
          <span className="flex items-center gap-1.5">
            <Loader2 size={13} className="animate-spin" />
            Exporting...
          </span>
        ) : (
          <span className="flex items-center gap-1.5">
            <Download size={13} />
            Download ZIP{doneCount > 0 ? ` (${doneCount})` : ""}
          </span>
        )}
      </button>

      {isExporting && (
        <div className="absolute -bottom-2 left-0 right-0">
          <ProgressBar value={progress} variant="blue" height="h-0.5" />
        </div>
      )}
    </div>
  );
}

export default memo(DownloadButton);
