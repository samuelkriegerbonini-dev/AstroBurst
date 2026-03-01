import { useState, useEffect, useRef } from "react";
import { motion } from "framer-motion";
import { CheckCircle2, XCircle, Clock, Loader2, ImageOff } from "lucide-react";
import { FILE_STATUS } from "../utils/constants";
import type { ProcessedFile } from "../utils/types";
import { convertFileSrc } from "@tauri-apps/api/core";

const statusConfig = {
  [FILE_STATUS.QUEUED]: {
    icon: Clock,
    color: "text-zinc-500",
    bgHover: "hover:bg-zinc-800/50",
  },
  [FILE_STATUS.PROCESSING]: {
    icon: Loader2,
    color: "text-blue-400",
    bgHover: "",
  },
  [FILE_STATUS.DONE]: {
    icon: CheckCircle2,
    color: "text-green-400",
    bgHover: "hover:bg-zinc-800/50 cursor-pointer",
  },
  [FILE_STATUS.ERROR]: {
    icon: XCircle,
    color: "text-red-400",
    bgHover: "",
  },
};

const slideIn = {
  initial: { x: -20, opacity: 0 },
  animate: { x: 0, opacity: 1 },
  exit: { x: -20, opacity: 0 },
};

interface FileItemProps {
  file: ProcessedFile;
  isSelected: boolean;
  onSelect: (id: string) => void;
  index: number;
}

export default function FileItem({ file, isSelected, onSelect, index }: FileItemProps) {
  const config = statusConfig[file.status] || statusConfig[FILE_STATUS.QUEUED];
  const Icon = config.icon;
  const isClickable = file.status === FILE_STATUS.DONE;

  const [thumbError, setThumbError] = useState(false);
  const [thumbLoaded, setThumbLoaded] = useState(false);
  const imgRef = useRef<HTMLImageElement>(null);

  const previewUrl = file.result?.previewUrl
      ? convertFileSrc(file.result.previewUrl)
      : "";

  useEffect(() => {
    setThumbError(false);
    setThumbLoaded(false);
  }, [previewUrl]);

  useEffect(() => {
    if (!previewUrl || file.status !== FILE_STATUS.DONE) return;

    const img = new Image();
    img.onload = () => setThumbLoaded(true);
    img.onerror = () => setThumbError(true);
    img.src = previewUrl;
  }, [previewUrl, file.status]);

  return (
      <motion.div
          {...slideIn}
          transition={{ delay: Math.min(index * 0.03, 0.5), duration: 0.2 }}
          onClick={() => isClickable && onSelect(file.id)}
          className={`
        flex items-center gap-3 px-3 py-2.5 rounded-lg transition-colors
        ${config.bgHover}
        ${isSelected ? "bg-zinc-800 ring-1 ring-blue-500/30" : ""}
        ${isClickable ? "cursor-pointer" : "cursor-default"}
      `}
      >
        <div className={`flex-shrink-0 ${config.color}`}>
          <Icon
              size={18}
              className={file.status === FILE_STATUS.PROCESSING ? "animate-spin" : ""}
          />
        </div>

        <div className="flex-1 min-w-0">
          <p
              className={`text-sm font-medium truncate ${
                  file.status === FILE_STATUS.QUEUED
                      ? "text-zinc-500"
                      : file.status === FILE_STATUS.ERROR
                          ? "text-red-400"
                          : file.status === FILE_STATUS.DONE
                              ? "text-green-50"
                              : "text-zinc-100"
              }`}
          >
            {file.name}
          </p>
          <p className="text-xs text-zinc-600 mt-0.5">
            {file.status === FILE_STATUS.DONE && file.result && (
                <span className="font-mono">
              {file.result.dimensions?.[0]}×{file.result.dimensions?.[1]}
                  {"  "}
                  <span className="text-zinc-500">
                {(file.result.elapsed_ms / 1000).toFixed(2)}s
              </span>
            </span>
            )}
            {file.status === FILE_STATUS.PROCESSING && (
                <span className="text-blue-400/70">processing...</span>
            )}
            {file.status === FILE_STATUS.QUEUED && <span className="text-zinc-600">queued</span>}
            {file.status === FILE_STATUS.ERROR && (
                <span className="text-red-400/70 truncate block max-w-[180px]" title={file.error || ""}>
              {file.error}
            </span>
            )}
          </p>
        </div>

        {file.status === FILE_STATUS.DONE && previewUrl && (
            <div className="flex-shrink-0 w-10 h-10 rounded border border-white/10 overflow-hidden bg-zinc-900 shadow-inner">
              {thumbError ? (
                  <div className="w-full h-full flex items-center justify-center">
                    <ImageOff size={14} className="text-zinc-600" />
                  </div>
              ) : (
                  <img
                      ref={imgRef}
                      src={previewUrl}
                      alt=""
                      className={`w-full h-full object-cover transition-opacity duration-200 ${
                          thumbLoaded ? "opacity-100" : "opacity-0"
                      }`}
                      onError={() => setThumbError(true)}
                      loading="lazy"
                  />
              )}
            </div>
        )}
      </motion.div>
  );
}