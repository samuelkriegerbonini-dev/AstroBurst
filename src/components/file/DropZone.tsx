import { useState, useEffect, useCallback, useRef } from "react";
import { Upload, AlertTriangle, X } from "lucide-react";
import { isValidFitsFile, isCalibRefAsdf } from "../../utils/validation";
import { AnimatePresence, motion } from "framer-motion";
import { isTauri } from "../../infrastructure/tauri";
import type { AstroFile } from "../../shared/types";

interface DropZoneProps {
  onFilesAdded: (files: AstroFile[]) => void;
  children: React.ReactNode;
}

export default function DropZone({ onFilesAdded, children }: DropZoneProps) {
  const [isDragOver, setIsDragOver] = useState(false);
  const [rejected, setRejected] = useState<{ skipped: number; calib: number } | null>(null);
  const callbackRef = useRef(onFilesAdded);
  const dragCounterRef = useRef(0);
  const rejectedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    callbackRef.current = onFilesAdded;
  }, [onFilesAdded]);

  const reportRejected = useCallback((skipped: number, calib: number) => {
    if (skipped === 0 && calib === 0) return;
    setRejected({ skipped, calib });
    if (rejectedTimerRef.current) clearTimeout(rejectedTimerRef.current);
    rejectedTimerRef.current = setTimeout(() => setRejected(null), 6000);
  }, []);
  const reportRejectedRef = useRef(reportRejected);
  reportRejectedRef.current = reportRejected;

  useEffect(() => {
    return () => {
      if (rejectedTimerRef.current) clearTimeout(rejectedTimerRef.current);
    };
  }, []);

  useEffect(() => {
    if (!isTauri()) return;

    let unlisten: (() => void) | null = null;
    let cancelled = false;

    const setup = async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();

        unlisten = await win.onDragDropEvent((event) => {
          if (cancelled) return;
          const t = event.payload.type;

          if (t === "enter" || t === "over") {
            setIsDragOver(true);
          } else if (t === "drop") {
            setIsDragOver(false);
            const paths: string[] = event.payload.paths || [];
            const fitsFiles: AstroFile[] = paths
              .filter((p: string) => isValidFitsFile(p))
              .map((p: string) => ({
                name: p.split(/[/\\]/).pop() || "file",
                path: p,
                size: 0,
              }));
            const validFiles = fitsFiles.filter((f: AstroFile) => !isCalibRefAsdf(f.name));
            reportRejectedRef.current(paths.length - fitsFiles.length, fitsFiles.length - validFiles.length);
            if (validFiles.length > 0) {
              callbackRef.current(validFiles);
            }
          } else if (t === "leave" || t === "cancel") {
            setIsDragOver(false);
          }
        });
      } catch (err) {
        console.error("[AstroBurst] Drag-drop setup failed:", err);
      }
    };

    setup();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  const handleDragEnter = useCallback((e: DragEvent) => {
    if (isTauri()) return;
    e.preventDefault();
    e.stopPropagation();
    const types = Array.from(e.dataTransfer?.types || []);
    if (!types.includes("Files") && !types.includes("application/x-tauri-file")) return;
    dragCounterRef.current++;
    if (dragCounterRef.current === 1) setIsDragOver(true);
  }, []);

  const handleDragOver = useCallback((e: DragEvent) => {
    if (isTauri()) return;
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const handleDragLeave = useCallback((e: DragEvent) => {
    if (isTauri()) return;
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current--;
    if (dragCounterRef.current <= 0) {
      dragCounterRef.current = 0;
      setIsDragOver(false);
    }
  }, []);

  const handleDrop = useCallback((e: DragEvent) => {
    if (isTauri()) return;
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current = 0;
    setIsDragOver(false);

    const droppedFiles = Array.from(e.dataTransfer?.files || []);
    const fitsFiles = droppedFiles.filter((f) => isValidFitsFile(f.name));
    const accepted = fitsFiles.filter((f) => !isCalibRefAsdf(f.name));
    const validFiles: AstroFile[] = accepted.map((f) => ({
      name: f.name,
      path: f.name,
      size: f.size,
    }));

    reportRejected(droppedFiles.length - fitsFiles.length, fitsFiles.length - accepted.length);
    if (validFiles.length > 0) {
      callbackRef.current(validFiles);
    }
  }, [reportRejected]);

  useEffect(() => {
    if (isTauri()) return;
    const w = window;
    w.addEventListener("dragenter", handleDragEnter);
    w.addEventListener("dragover", handleDragOver);
    w.addEventListener("dragleave", handleDragLeave);
    w.addEventListener("drop", handleDrop);

    return () => {
      w.removeEventListener("dragenter", handleDragEnter);
      w.removeEventListener("dragover", handleDragOver);
      w.removeEventListener("dragleave", handleDragLeave);
      w.removeEventListener("drop", handleDrop);
    };
  }, [handleDragEnter, handleDragOver, handleDragLeave, handleDrop]);

  return (
    <div className="relative w-full h-full">
      {children}

      <AnimatePresence>
        {isDragOver && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            className="fixed inset-0 z-[100] flex items-center justify-center bg-zinc-950/80 backdrop-blur-sm"
          >
            <motion.div
              initial={{ scale: 0.9, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              exit={{ scale: 0.9, opacity: 0 }}
              className="flex flex-col items-center gap-4 border-2 border-dashed border-blue-500 rounded-2xl px-20 py-16 bg-blue-500/5"
            >
              <Upload size={48} className="text-blue-400" />
              <p className="text-xl font-semibold text-zinc-100">
                Drop anywhere
              </p>
              <p className="text-zinc-500 text-sm">
                Release to add .fits / .asdf / .zip files
              </p>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      <AnimatePresence>
        {rejected && (
          <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 10 }}
            transition={{ duration: 0.15 }}
            className="fixed bottom-6 left-1/2 -translate-x-1/2 z-[110] flex items-center gap-2 px-3 py-2 rounded-lg text-xs shadow-lg"
            style={{ background: "rgba(24,17,4,0.96)", border: "1px solid rgba(245,158,11,0.3)", color: "#fbbf24" }}
          >
            <AlertTriangle size={13} className="shrink-0" />
            <span>
              {rejected.skipped > 0 && `${rejected.skipped} file${rejected.skipped > 1 ? "s" : ""} skipped — supported: .fits .fit .fts .asdf .zip`}
              {rejected.skipped > 0 && rejected.calib > 0 && " · "}
              {rejected.calib > 0 && `${rejected.calib} JWST calibration reference .asdf skipped (not a loadable image)`}
            </span>
            <button
              onClick={() => setRejected(null)}
              title="Dismiss"
              aria-label="Dismiss"
              className="ml-1 shrink-0 text-amber-500/70 hover:text-amber-300 transition-colors"
            >
              <X size={12} />
            </button>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
