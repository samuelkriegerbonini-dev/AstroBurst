import { motion, AnimatePresence } from "framer-motion";
import { Download, Check, Loader2 } from "lucide-react";
import ProgressBar from "./ProgressBar";

export default function DownloadButton({
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
      <motion.button
        onClick={() => onExport(files)}
        disabled={disabled}
        className={`
          flex items-center gap-2 rounded-lg px-5 py-2.5 font-medium transition-all text-sm
          ${
            isComplete && !isExporting && !downloaded
              ? "bg-green-500 hover:bg-green-600 text-white animate-pulse-glow"
              : disabled
              ? "bg-zinc-800 text-zinc-600 cursor-not-allowed"
              : "bg-blue-500 hover:bg-blue-600 text-white"
          }
        `}
        whileTap={!disabled ? { scale: 0.97 } : {}}
      >
        <AnimatePresence mode="wait">
          {downloaded ? (
            <motion.span
              key="done"
              initial={{ opacity: 0, scale: 0.5 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.5 }}
              className="flex items-center gap-2"
            >
              <Check size={16} />
              Downloaded!
            </motion.span>
          ) : isExporting ? (
            <motion.span
              key="exporting"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="flex items-center gap-2"
            >
              <Loader2 size={16} className="animate-spin" />
              Exporting...
            </motion.span>
          ) : (
            <motion.span
              key="default"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="flex items-center gap-2"
            >
              <Download size={16} />
              Download ZIP{doneCount > 0 ? ` (${doneCount} files)` : ""}
            </motion.span>
          )}
        </AnimatePresence>
      </motion.button>

      {/* Export progress bar */}
      {isExporting && (
        <div className="absolute -bottom-3 left-0 right-0">
          <ProgressBar value={progress} variant="blue" height="h-1" />
        </div>
      )}
    </div>
  );
}
