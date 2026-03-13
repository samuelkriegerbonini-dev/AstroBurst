import { useState, useEffect, memo, useCallback, useRef } from "react";
import { CheckCircle2, XCircle, Clock, Loader2, ImageOff } from "lucide-react";
import { FILE_STATUS } from "../../utils/constants";
import { useFileEntry } from "../../hooks/useFileStore";
import type { ProcessedFile } from "../../utils/types";

const statusConfig = {
  [FILE_STATUS.QUEUED]: { icon: Clock, color: "text-zinc-500", bgHover: "hover:bg-zinc-800/40" },
  [FILE_STATUS.PROCESSING]: { icon: Loader2, color: "", bgHover: "", inlineColor: "var(--ab-teal)" },
  [FILE_STATUS.DONE]: { icon: CheckCircle2, color: "", bgHover: "hover:bg-zinc-800/40 cursor-pointer", inlineColor: "var(--ab-green)" },
  [FILE_STATUS.ERROR]: { icon: XCircle, color: "text-red-400", bgHover: "" },
};

interface FileItemProps {
  fileId: string;
  isSelected: boolean;
  onSelect: (id: string) => void;
}

function FileItem({ fileId, isSelected, onSelect }: FileItemProps) {
  const file = useFileEntry(fileId);
  const [thumbError, setThumbError] = useState(false);
  const [thumbLoaded, setThumbLoaded] = useState(false);
  const [visible, setVisible] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  if (!file) return null;

  const config = statusConfig[file.status] || statusConfig[FILE_STATUS.QUEUED];
  const Icon = config.icon;
  const isClickable = file.status === FILE_STATUS.DONE;
  const previewUrl = file.result?.previewUrl || "";

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "100px" },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    setThumbError(false);
    setThumbLoaded(false);
  }, [previewUrl]);

  useEffect(() => {
    if (!visible || !previewUrl || file.status !== FILE_STATUS.DONE) return;
    const img = new window.Image();
    img.onload = () => setThumbLoaded(true);
    img.onerror = () => setThumbError(true);
    img.src = previewUrl;
  }, [visible, previewUrl, file.status]);

  const handleClick = useCallback(() => {
    if (isClickable) onSelect(fileId);
  }, [isClickable, onSelect, fileId]);

  const showThumb = visible && file.status === FILE_STATUS.DONE && previewUrl;

  return (
    <div
      ref={rootRef}
      onClick={handleClick}
      className={`group flex items-center gap-2 mx-1 px-2 py-1.5 rounded-md transition-all duration-150 border ${config.bgHover} ${isClickable ? "cursor-pointer" : "cursor-default"}`}
      style={{
        height: 44,
        ...(isSelected
          ? { background: "rgba(20,184,166,0.08)", borderColor: "rgba(20,184,166,0.25)", boxShadow: "0 0 12px rgba(20,184,166,0.06)" }
          : { borderColor: "transparent" }),
      }}
    >
      {showThumb ? (
        <div className="flex-shrink-0 w-8 h-8 rounded overflow-hidden bg-zinc-900" style={{ border: "1px solid rgba(20,184,166,0.15)" }}>
          {thumbError ? (
            <div className="w-full h-full flex items-center justify-center"><ImageOff size={12} className="text-zinc-600" /></div>
          ) : (
            <img
              src={previewUrl}
              alt=""
              loading="lazy"
              decoding="async"
              className={`w-full h-full object-cover transition-opacity duration-150 ${thumbLoaded ? "opacity-100" : "opacity-0"}`}
              onError={() => setThumbError(true)}
            />
          )}
        </div>
      ) : (
        <div
          className="flex-shrink-0 w-8 h-8 rounded flex items-center justify-center"
          style={file.status === FILE_STATUS.PROCESSING ? { background: "rgba(20,184,166,0.08)" } : { background: "rgba(39,39,42,0.4)" }}
        >
          <Icon
            size={14}
            className={`${config.color} ${file.status === FILE_STATUS.PROCESSING ? "animate-spin" : ""}`}
            style={config.inlineColor ? { color: config.inlineColor } : undefined}
          />
        </div>
      )}

      <div className="flex-1 min-w-0">
        <p
          className={`text-[11px] font-medium truncate leading-tight ${
            file.status === FILE_STATUS.QUEUED ? "text-zinc-500"
              : file.status === FILE_STATUS.ERROR ? "text-red-400"
                : file.status === FILE_STATUS.DONE ? (isSelected ? "text-zinc-100" : "text-zinc-300")
                  : "text-zinc-100"
          }`}
          title={file.name}
        >{file.name}</p>
        <p className="text-[10px] text-zinc-600 leading-tight mt-0.5">
          {file.status === FILE_STATUS.DONE && file.result && (
            <span className="font-mono">{file.result.dimensions?.[0]}x{file.result.dimensions?.[1]} <span className="text-zinc-500">{(file.result.elapsed_ms / 1000).toFixed(2)}s</span></span>
          )}
          {file.status === FILE_STATUS.PROCESSING && <span style={{ color: "rgba(20,184,166,0.6)" }}>processing...</span>}
          {file.status === FILE_STATUS.QUEUED && <span>queued</span>}
          {file.status === FILE_STATUS.ERROR && <span className="text-red-400/70 truncate block" title={file.error || ""}>{file.error}</span>}
        </p>
      </div>

      {isSelected && file.status === FILE_STATUS.DONE && (
        <div className="flex-shrink-0 w-1 h-5 rounded-full" style={{ background: "var(--ab-teal)" }} />
      )}
    </div>
  );
}

export default memo(FileItem, (prev, next) =>
  prev.fileId === next.fileId && prev.isSelected === next.isSelected
);
