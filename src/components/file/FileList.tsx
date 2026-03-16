import { useRef, useEffect, useState, useCallback, memo, useMemo } from "react";
import { PanelLeftClose, PanelLeftOpen, FolderOpen, Download, Loader2 } from "lucide-react";
import FileItem from "./FileItem";
import { useFileIds, useSelectedId, useFileStats, fileStore } from "../../hooks/useFileStore";
import type { ProcessedFile } from "../../shared/types";

const ITEM_HEIGHT = 44;
const OVERSCAN = 5;

interface FileListProps {
  collapsed?: boolean;
  onToggle?: () => void;
  onExportZip?: (files: ProcessedFile[]) => void;
  isExporting?: boolean;
  zipProgress?: number;
  downloaded?: boolean;
}

const VirtualizedItems = memo(function VirtualizedItems({
                                                          fileIds,
                                                          scrollTop,
                                                          viewHeight,
                                                          selectedId,
                                                          onSelect,
                                                        }: {
  fileIds: string[];
  scrollTop: number;
  viewHeight: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const startIdx = Math.max(0, Math.floor(scrollTop / ITEM_HEIGHT) - OVERSCAN);
  const endIdx = Math.min(fileIds.length, Math.ceil((scrollTop + viewHeight) / ITEM_HEIGHT) + OVERSCAN);
  const visibleIds = useMemo(() => fileIds.slice(startIdx, endIdx), [fileIds, startIdx, endIdx]);

  return (
    <div style={{ height: fileIds.length * ITEM_HEIGHT, position: "relative" }}>
      <div style={{ position: "absolute", top: startIdx * ITEM_HEIGHT, left: 0, right: 0 }}>
        {visibleIds.map((id) => (
          <FileItem
            key={id}
            fileId={id}
            isSelected={id === selectedId}
            onSelect={onSelect}
          />
        ))}
      </div>
    </div>
  );
});

function FileList({
                    collapsed = false,
                    onToggle,
                    onExportZip,
                    isExporting = false,
                    zipProgress = 0,
                    downloaded = false,
                  }: FileListProps) {
  const fileIds = useFileIds();
  const selectedId = useSelectedId();
  const { stats, isComplete } = useFileStats();

  const scrollRef = useRef<HTMLDivElement>(null);
  const prevLenRef = useRef(fileIds.length);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewHeight, setViewHeight] = useState(600);
  const rafScrollRef = useRef<number | null>(null);

  const onSelect = useCallback((id: string) => {
    fileStore.selectFile(id);
  }, []);

  const handleExport = useCallback(() => {
    onExportZip?.(fileStore.getFiles());
  }, [onExportZip]);

  useEffect(() => {
    if (fileIds.length > prevLenRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
    prevLenRef.current = fileIds.length;
  }, [fileIds.length]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setViewHeight(entry.contentRect.height));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const handleScroll = useCallback(() => {
    if (rafScrollRef.current) return;
    rafScrollRef.current = requestAnimationFrame(() => {
      rafScrollRef.current = null;
      if (scrollRef.current) setScrollTop(scrollRef.current.scrollTop);
    });
  }, []);

  useEffect(() => {
    return () => { if (rafScrollRef.current) cancelAnimationFrame(rafScrollRef.current); };
  }, []);

  if (collapsed) {
    return (
      <div className="flex flex-col items-center h-full py-2 gap-2">
        <button onClick={onToggle} className="p-2 rounded hover:bg-zinc-800 transition-colors text-zinc-500 hover:text-zinc-300" title="Show file panel">
          <PanelLeftOpen size={16} />
        </button>
        <div className="w-px flex-1" style={{ background: "rgba(20,184,166,0.08)" }} />
        <div className="flex flex-col items-center gap-1">
          <FolderOpen size={14} className="text-zinc-600" />
          <span className="text-[9px] font-mono text-zinc-600" style={{ writingMode: "vertical-rl", textOrientation: "mixed" }}>
            {fileIds.length} files
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
            {fileIds.length}
          </span>
        </div>
        <button onClick={onToggle} className="p-1 rounded hover:bg-zinc-800 transition-colors text-zinc-600 hover:text-zinc-300">
          <PanelLeftClose size={14} />
        </button>
      </div>

      <div ref={scrollRef} className="flex-1 overflow-y-auto py-1" onScroll={handleScroll}>
        <VirtualizedItems
          fileIds={fileIds}
          scrollTop={scrollTop}
          viewHeight={viewHeight}
          selectedId={selectedId}
          onSelect={onSelect}
        />
      </div>

      {stats.done > 0 && (
        <div className="shrink-0 px-2 py-2" style={{ borderTop: "1px solid rgba(20,184,166,0.1)" }}>
          <button
            onClick={handleExport}
            disabled={stats.done === 0 || isExporting}
            className="w-full flex items-center justify-center gap-1.5 rounded-md px-3 py-1.5 font-medium transition-all duration-150 text-xs"
            style={
              isComplete && !isExporting && !downloaded
                ? { background: "rgba(20,184,166,0.2)", color: "var(--ab-teal)", border: "1px solid rgba(20,184,166,0.3)" }
                : stats.done === 0 || isExporting
                  ? { background: "rgba(39,39,42,0.5)", color: "#52525b", border: "1px solid transparent" }
                  : { background: "rgba(20,184,166,0.1)", color: "#a1a1aa", border: "1px solid rgba(20,184,166,0.15)" }
            }
          >
            {downloaded ? (
              <><Download size={12} /> Downloaded</>
            ) : isExporting ? (
              <><Loader2 size={12} className="animate-spin" /> Exporting...</>
            ) : (
              <><Download size={12} /> Download ZIP{stats.done > 0 ? ` (${stats.done})` : ""}</>
            )}
          </button>
        </div>
      )}
    </div>
  );
}

export default memo(FileList);
