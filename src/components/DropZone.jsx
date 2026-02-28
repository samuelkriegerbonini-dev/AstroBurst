import { useState, useEffect, useCallback, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Upload } from "lucide-react";
import { isValidFitsFile } from "../utils/validation";

const isTauri = () => !!window.__TAURI_INTERNALS__;

export default function DropZone({ onFilesAdded, children }) {
  const [isDragOver, setIsDragOver] = useState(false);
  const callbackRef = useRef(onFilesAdded);
  const dragCounterRef = useRef(0);

  useEffect(() => {
    callbackRef.current = onFilesAdded;
  }, [onFilesAdded]);

  useEffect(() => {
    if (!isTauri()) return;

    let unlisten = null;
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
            const paths = event.payload.paths || [];
            const validFiles = paths
              .filter((p) => isValidFitsFile(p))
              .map((p) => ({
                name: p.split(/[/\\]/).pop() || "file",
                path: p,
                size: 0,
              }));
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

  const handleDragEnter = useCallback((e) => {
    if (isTauri()) return;
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current++;
    if (dragCounterRef.current === 1) setIsDragOver(true);
  }, []);

  const handleDragOver = useCallback((e) => {
    if (isTauri()) return;
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const handleDragLeave = useCallback((e) => {
    if (isTauri()) return;
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current--;
    if (dragCounterRef.current <= 0) {
      dragCounterRef.current = 0;
      setIsDragOver(false);
    }
  }, []);

  const handleDrop = useCallback((e) => {
    if (isTauri()) return;
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current = 0;
    setIsDragOver(false);

    const droppedFiles = Array.from(e.dataTransfer.files);
    const validFiles = droppedFiles
      .filter((f) => isValidFitsFile(f.name))
      .map((f) => ({
        name: f.name,
        path: f.name,
        size: f.size,
      }));

    if (validFiles.length > 0) {
      callbackRef.current(validFiles);
    }
  }, []);

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
                Release to add .fits files
              </p>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
