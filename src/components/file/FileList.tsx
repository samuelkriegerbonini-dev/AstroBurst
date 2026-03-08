import { useRef, useEffect, memo } from "react";
import { PanelLeftClose, PanelLeftOpen, FolderOpen, Download, Loader2 } from "lucide-react";
import FileItem from "./FileItem";
import type { ProcessedFile } from "../../utils/types";

interface FileListProps {
  files: ProcessedFile[];
  selected: string | null;
  onSelect: (id: string) => void;
  collapsed?: boolean;
  onToggle?: () => void;
  onExportZip?: (files: ProcessedFile[]) => void;
  isExporting?: boolean;
  zipProgress?: number;
  downloaded?: boolean;
  doneCount?: number;
  isComplete?: boolean;
}

function FileList({
                    files,
                    selected,
                    onSelect,
                    collapsed = false,
                    onToggle,
                    onExportZip,
                    isExporting = false,
                    zipProgress = 0,
                    downloaded = false,
                    doneCount = 0,
                    isComplete = false,
                  }: FileListProps) {
  const listRef = useRef<HTMLDivElement>(null);
  const prevLenRef = useRef(files.length);

  useEffect(() => {
    if (files.length > prevLenRef.current && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
    prevLenRef.current = files.length;
  }, [files.length]);

  if (collapsed) {
    return (
      <div className="flex flex-col items-center h-full py-2 gap-2">
        <button
          onClick={onToggle}
          className="p-2 rounded hover:bg-zinc-800 transition-colors text-zinc-500 hover:text-zinc-300"
          title="Show file panel"
        >
          <PanelLeftOpen size={16} />
        </button>
        <div className="w-px flex-1" style={{ background: "rgba(20,184,166,0.08)" }} />
        <div className="flex flex-col items-center gap-1">
          <FolderOpen size={14} className="text-zinc-600" />
          <span className="text-[9px] font-mono text-zinc-600" style={{ writingMode: "vertical-rl", textOrientation: "mixed" }}>
            {files.length} files
          </span>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full cosmic-sidebar-glow">
      <div className="flex items-center justify-between px-3 py-2 shrink-0" style={{ borderBottom: "1px solid rgba(20,184,166,0.1)" }}>
        <div className="flex items-center gap-2">
          <FolderOpen size={13} style={{ color: "var(--ab-teal)", opacity: 0.6 }} />
          <h3 className="text-[11px] font-semibold text-zinc-400 uppercase tracking-wider">Files</h3>
          <span className="text-[10px] font-mono px-1.5 py-0.5 rounded" style={{ color: "var(--ab-teal)", background: "rgba(20,184,166,0.08)" }}>
            {files.length}
          </span>
        </div>
        <button onClick={onToggle} className="p-1 rounded hover:bg-zinc-800 transition-colors text-zinc-600 hover:text-zinc-300">
          <PanelLeftClose size={14} />
        </button>
      </div>

      <div ref={listRef} className="flex-1 overflow-y-auto py-1">
        {files.map((file, index) => (
          <FileItem key={file.id} file={file} isSelected={file.id === selected} onSelect={onSelect} index={index} />
        ))}
      </div>

      {/* Export + Download ZIP fixed at bottom */}
      {doneCount > 0 && (
        <div className="shrink-0 px-2 py-2" style={{ borderTop: "1px solid rgba(20,184,166,0.1)" }}>
          <button
            onClick={() => onExportZip?.(files)}
            disabled={doneCount === 0 || isExporting}
            className="w-full flex items-center justify-center gap-1.5 rounded-md px-3 py-1.5 font-medium transition-all duration-150 text-xs"
            style={
              isComplete && !isExporting && !downloaded
                ? { background: "rgba(20,184,166,0.2)", color: "var(--ab-teal)", border: "1px solid rgba(20,184,166,0.3)" }
                : doneCount === 0 || isExporting
                  ? { background: "rgba(39,39,42,0.5)", color: "#52525b", border: "1px solid transparent" }
                  : { background: "rgba(20,184,166,0.1)", color: "#a1a1aa", border: "1px solid rgba(20,184,166,0.15)" }
            }
          >
            {downloaded ? (
              <><Download size={12} /> Downloaded</>
            ) : isExporting ? (
              <><Loader2 size={12} className="animate-spin" /> Exporting...</>
            ) : (
              <><Download size={12} /> Download ZIP{doneCount > 0 ? ` (${doneCount})` : ""}</>
            )}
          </button>
        </div>
      )}
    </div>
  );
}

export default memo(FileList);
