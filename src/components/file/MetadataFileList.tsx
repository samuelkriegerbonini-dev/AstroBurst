import {
  useState,
  useCallback,
  useRef,
  useEffect,
  useMemo,
  memo,
} from "react";
import {
  FolderOpen,
  PanelLeftClose,
  PanelLeftOpen,
  Download,
  Loader2,
  CheckCircle2,
  XCircle,
  Clock,
  ImageOff,
  GripVertical,
  Filter,
  Timer,
  Aperture,
  ChevronRight,
  Search,
  SlidersHorizontal,
  Pin,
  X,
} from "lucide-react";

export interface FileMetadata {
  filter?: string;
  exptime?: number;
  instrument?: string;
  detector?: string;
  bitpix?: number;
  naxis?: number[];
  dateObs?: string;
}

export interface MetadataFile {
  id: string;
  name: string;
  path: string;
  size: number;
  status: "queued" | "processing" | "done" | "error";
  error?: string;
  metadata?: FileMetadata;
  previewUrl?: string;
  dimensions?: [number, number];
  elapsed_ms?: number;
}

import type { FilterMode } from "../../hooks/useProductFilter";

interface MetadataFileListProps {
  files: MetadataFile[];
  totalFiles?: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onExportZip?: () => void;
  collapsed?: boolean;
  onToggle?: () => void;
  isExporting?: boolean;
  zipProgress?: number;
  downloaded?: boolean;
  groupByInstrument?: boolean;
  productTypes?: string[];
  customChips?: string[];
  activeFilters?: string[];
  filterMode?: FilterMode;
  onToggleFilter?: (filter: string) => void;
  onToggleMode?: () => void;
  onClearFilters?: () => void;
  onAddCustomChip?: (text: string) => void;
  onRemoveCustomChip?: (text: string) => void;
}

const STATUS_CONFIG = {
  queued: { icon: Clock, color: "text-zinc-500", accentColor: "" },
  processing: { icon: Loader2, color: "", accentColor: "var(--ab-teal)" },
  done: { icon: CheckCircle2, color: "", accentColor: "var(--ab-green)" },
  error: { icon: XCircle, color: "text-red-400", accentColor: "" },
};

const FILTER_COLORS: Record<string, string> = {
  F070W: "#dc2626", F090W: "#ea580c", F115W: "#d97706",
  F140M: "#ca8a04", F150W: "#84cc16", F162M: "#22c55e",
  F164N: "#16a34a", F182M: "#14b8a6", F187N: "#0d9488",
  F200W: "#0ea5e9", F210M: "#2563eb", F212N: "#4f46e5",
  F250M: "#7c3aed", F277W: "#9333ea", F300M: "#a855f7",
  F322W2: "#c026d3", F335M: "#db2777", F356W: "#e11d48",
  F360M: "#f43f5e", F405N: "#ef4444", F410M: "#dc2626",
  F430M: "#b91c1c", F444W: "#991b1b", F460M: "#7f1d1d",
  F466N: "#78350f", F470N: "#713f12", F480M: "#365314",
};

function getFilterColor(filter?: string): string {
  if (!filter) return "#71717a";
  return FILTER_COLORS[filter.toUpperCase().trim()] ?? "#71717a";
}

function shortName(fullName: string): string {
  const parts = fullName.replace(/\.[^.]+$/, "").split(/[_-]/);
  if (parts.length <= 3) return fullName;
  const filterPart = parts.find((p) => /^f\d{3}[wmnWMN]/i.test(p));
  const last = parts[parts.length - 1];
  if (filterPart) return `...${filterPart}_${last}`;
  return parts.slice(-3).join("_");
}

interface MetadataFileItemProps {
  file: MetadataFile;
  isSelected: boolean;
  onSelect: (id: string) => void;
}

function MetadataFileItem({ file, isSelected, onSelect }: MetadataFileItemProps) {
  const [thumbError, setThumbError] = useState(false);
  const [thumbLoaded, setThumbLoaded] = useState(false);

  const status = file.status;
  const config = STATUS_CONFIG[status];
  const Icon = config.icon;
  const isClickable = status === "done";
  const meta = file.metadata;

  useEffect(() => {
    setThumbError(false);
    setThumbLoaded(false);
  }, [file.previewUrl]);

  const handleClick = useCallback(() => {
    if (isClickable) onSelect(file.id);
  }, [isClickable, onSelect, file.id]);

  const handleDragStart = useCallback(
    (e: React.DragEvent) => {
      e.dataTransfer.setData(
        "application/astroburst-file",
        JSON.stringify({
          id: file.id,
          path: file.path,
          name: file.name,
          filter: meta?.filter,
          instrument: meta?.instrument,
          exptime: meta?.exptime,
          previewUrl: file.previewUrl,
        }),
      );
      e.dataTransfer.effectAllowed = "copy";
    },
    [file, meta],
  );

  return (
    <div
      className={`ab-mfl-item ${isSelected ? "ab-mfl-item-selected" : ""} ${isClickable ? "ab-mfl-item-clickable" : ""}`}
      onClick={handleClick}
      draggable={status === "done"}
      onDragStart={handleDragStart}
    >
      <div className="ab-mfl-grip">
        {status === "done" ? (
          <GripVertical size={12} className="text-zinc-700 group-hover:text-zinc-500" />
        ) : (
          <div className="w-3" />
        )}
      </div>

      <div className="ab-mfl-thumb">
        {status === "done" && file.previewUrl && !thumbError ? (
          <img
            src={file.previewUrl}
            alt=""
            loading="lazy"
            decoding="async"
            className={`ab-mfl-thumb-img ${thumbLoaded ? "opacity-100" : "opacity-0"}`}
            onLoad={() => setThumbLoaded(true)}
            onError={() => setThumbError(true)}
          />
        ) : (
          <div className="ab-mfl-thumb-placeholder">
            <Icon
              size={14}
              className={`${config.color} ${status === "processing" ? "animate-spin" : ""}`}
              style={config.accentColor ? { color: config.accentColor } : undefined}
            />
          </div>
        )}
      </div>

      <div className="ab-mfl-content">
        <div className="ab-mfl-row-primary">
          <span className="ab-mfl-filename" title={file.name}>
            {shortName(file.name)}
          </span>
          <div className="ab-mfl-status-dot" data-status={status} />
        </div>

        <div className="ab-mfl-row-meta">
          {status === "done" && (
            <>
              {meta?.filter && (
                <span
                  className="ab-mfl-chip"
                  style={{
                    color: getFilterColor(meta.filter),
                    background: `${getFilterColor(meta.filter)}15`,
                    borderColor: `${getFilterColor(meta.filter)}30`,
                  }}
                >
                  <Filter size={8} />
                  {meta.filter}
                </span>
              )}
              {meta?.exptime != null && (
                <span className="ab-mfl-chip ab-mfl-chip-neutral">
                  <Timer size={8} />
                  {meta.exptime}s
                </span>
              )}
              {meta?.instrument && (
                <span className="ab-mfl-chip ab-mfl-chip-neutral">
                  <Aperture size={8} />
                  {meta.instrument}
                </span>
              )}
              {file.dimensions && (
                <span className="ab-mfl-chip ab-mfl-chip-dim">
                  {file.dimensions[0]}x{file.dimensions[1]}
                </span>
              )}
            </>
          )}
          {status === "processing" && (
            <span className="ab-mfl-processing-text">Processing...</span>
          )}
          {status === "queued" && (
            <span className="ab-mfl-queued-text">Queued</span>
          )}
          {status === "error" && (
            <span className="ab-mfl-error-text" title={file.error}>{file.error}</span>
          )}
        </div>
      </div>

      {isSelected && status === "done" && (
        <div className="ab-mfl-active-bar" />
      )}
    </div>
  );
}

const MemoFileItem = memo(MetadataFileItem, (prev, next) =>
  prev.file.id === next.file.id
  && prev.isSelected === next.isSelected
  && prev.file.status === next.file.status
  && prev.file.previewUrl === next.file.previewUrl,
);

const ITEM_HEIGHT = 58;
const OVERSCAN = 4;

function MetadataFileList({
                            files,
                            totalFiles,
                            selectedId,
                            onSelect,
                            onExportZip,
                            collapsed = false,
                            onToggle,
                            isExporting = false,
                            zipProgress = 0,
                            downloaded = false,
                            groupByInstrument = false,
                            productTypes = [],
                            customChips = [],
                            activeFilters = [],
                            filterMode = "or",
                            onToggleFilter,
                            onToggleMode,
                            onClearFilters,
                            onAddCustomChip,
                            onRemoveCustomChip,
                          }: MetadataFileListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewHeight, setViewHeight] = useState(600);
  const [searchQuery, setSearchQuery] = useState("");
  const rafRef = useRef<number | null>(null);

  const doneCount = useMemo(() => files.filter((f) => f.status === "done").length, [files]);

  const filteredFiles = useMemo(() => {
    if (!searchQuery.trim()) return files;
    const q = searchQuery.toLowerCase();
    return files.filter(
      (f) =>
        f.name.toLowerCase().includes(q)
        || f.metadata?.filter?.toLowerCase().includes(q)
        || f.metadata?.instrument?.toLowerCase().includes(q),
    );
  }, [files, searchQuery]);

  const groupedFiles = useMemo(() => {
    if (!groupByInstrument) return null;
    const groups: Record<string, MetadataFile[]> = {};
    for (const f of filteredFiles) {
      const key = f.metadata?.instrument ?? "Unknown";
      (groups[key] ??= []).push(f);
    }
    return groups;
  }, [filteredFiles, groupByInstrument]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setViewHeight(entry.contentRect.height));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const handleScroll = useCallback(() => {
    if (rafRef.current) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      if (scrollRef.current) setScrollTop(scrollRef.current.scrollTop);
    });
  }, []);

  useEffect(() => {
    return () => { if (rafRef.current) cancelAnimationFrame(rafRef.current); };
  }, []);

  const startIdx = Math.max(0, Math.floor(scrollTop / ITEM_HEIGHT) - OVERSCAN);
  const endIdx = Math.min(filteredFiles.length, Math.ceil((scrollTop + viewHeight) / ITEM_HEIGHT) + OVERSCAN);
  const visibleFiles = useMemo(
    () => filteredFiles.slice(startIdx, endIdx),
    [filteredFiles, startIdx, endIdx],
  );

  const isFiltered = activeFilters.length > 0;
  const total = totalFiles ?? files.length;

  const handlePinSearch = useCallback(() => {
    const q = searchQuery.trim();
    if (!q) return;
    onAddCustomChip?.(q);
    setSearchQuery("");
  }, [searchQuery, onAddCustomChip]);

  const handleSearchKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handlePinSearch();
    }
  }, [handlePinSearch]);

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
          <span
            className="text-[9px] font-mono text-zinc-600"
            style={{ writingMode: "vertical-rl", textOrientation: "mixed" }}
          >
            {files.length} files
          </span>
        </div>
      </div>
    );
  }

  return (
    <div className="ab-mfl-root">
      <div className="ab-mfl-header">
        <div className="flex items-center gap-2">
          <FolderOpen size={13} style={{ color: "var(--ab-teal)", opacity: 0.6 }} />
          <h3 className="text-[11px] font-semibold text-zinc-400 uppercase tracking-wider">Files</h3>
          <span
            className="text-[10px] font-mono px-1.5 py-0.5 rounded"
            style={{ color: "var(--ab-teal)", background: "rgba(20,184,166,0.08)" }}
          >
            {isFiltered ? `${files.length}/${total}` : files.length}
          </span>
        </div>
        <button
          onClick={onToggle}
          className="p-1 rounded hover:bg-zinc-800 transition-colors text-zinc-600 hover:text-zinc-300"
        >
          <PanelLeftClose size={14} />
        </button>
      </div>

      {(productTypes.length > 1 || customChips.length > 0) && (
        <div className="ab-mfl-product-filter">
          <SlidersHorizontal size={10} className="text-zinc-600 shrink-0" />
          {productTypes.map((pt) => (
            <button
              key={pt}
              onClick={() => onToggleFilter?.(pt)}
              className={`ab-mfl-product-chip ${activeFilters.includes(pt) ? "ab-mfl-product-chip-active" : ""}`}
            >
              {pt}
            </button>
          ))}
          {customChips.map((chip) => (
            <span key={chip} className="ab-mfl-custom-chip-wrapper">
              <button
                onClick={() => onToggleFilter?.(chip)}
                className={`ab-mfl-product-chip ab-mfl-product-chip-custom ${activeFilters.includes(chip) ? "ab-mfl-product-chip-active" : ""}`}
              >
                {chip}
              </button>
              <button
                onClick={() => onRemoveCustomChip?.(chip)}
                className="ab-mfl-chip-remove"
                title={`Remove "${chip}" filter`}
              >
                <X size={8} />
              </button>
            </span>
          ))}
          {activeFilters.length >= 2 && (
            <button
              onClick={onToggleMode}
              className="ab-mfl-mode-toggle"
              title={filterMode === "or" ? "OR: show files matching ANY filter. Click to switch to AND." : "AND: show files matching ALL filters. Click to switch to OR."}
            >
              <span className={`ab-mfl-mode-opt ${filterMode === "and" ? "ab-mfl-mode-opt-active" : ""}`}>AND</span>
              <span className={`ab-mfl-mode-opt ${filterMode === "or" ? "ab-mfl-mode-opt-active" : ""}`}>OR</span>
            </button>
          )}
          {activeFilters.length > 0 && (
            <button
              onClick={() => onClearFilters?.()}
              className="ab-mfl-product-chip ab-mfl-product-chip-clear"
            >
              ✕
            </button>
          )}
        </div>
      )}

      <div className="ab-mfl-search-row">
        <div className="ab-mfl-search">
          <Search size={12} className="ab-mfl-search-icon" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={handleSearchKeyDown}
            placeholder="Search files..."
            className="ab-mfl-search-input"
          />
          {searchQuery.trim() && (
            <button
              onClick={() => setSearchQuery("")}
              className="ab-mfl-search-clear"
              title="Clear search"
            >
              <X size={10} />
            </button>
          )}
        </div>
        {searchQuery.trim() && (
          <button
            onClick={handlePinSearch}
            className="ab-mfl-pin-btn"
            title={`Pin "${searchQuery.trim()}" as global filter (Enter)`}
          >
            <Pin size={10} />
          </button>
        )}
      </div>

      <div
        ref={scrollRef}
        className="ab-mfl-scroll"
        onScroll={handleScroll}
      >
        <div style={{ height: filteredFiles.length * ITEM_HEIGHT, position: "relative" }}>
          <div style={{ position: "absolute", top: startIdx * ITEM_HEIGHT, left: 0, right: 0 }}>
            {visibleFiles.map((file) => (
              <MemoFileItem
                key={file.id}
                file={file}
                isSelected={file.id === selectedId}
                onSelect={onSelect}
              />
            ))}
          </div>
        </div>
      </div>

      {doneCount > 0 && (
        <div className="ab-mfl-footer">
          <button
            onClick={onExportZip}
            disabled={doneCount === 0 || isExporting}
            className="ab-mfl-export-btn"
            data-ready={doneCount > 0 && !isExporting && !downloaded ? "true" : "false"}
          >
            {downloaded ? (
              <><Download size={12} /> Downloaded</>
            ) : isExporting ? (
              <><Loader2 size={12} className="animate-spin" /> Exporting {Math.round(zipProgress)}%</>
            ) : (
              <><Download size={12} /> Download ZIP ({doneCount})</>
            )}
          </button>
        </div>
      )}
    </div>
  );
}

export default memo(MetadataFileList);
