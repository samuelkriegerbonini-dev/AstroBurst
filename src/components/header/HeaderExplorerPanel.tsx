import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import type { LucideIcon } from "lucide-react";
import {
  FileSearch, ChevronDown, ChevronRight, Search, Telescope,
  Camera, Image, Globe, Cpu, MoreHorizontal, Sparkles, Zap,
  Copy, Check, X, ClipboardCopy,
} from "lucide-react";
import type { ProcessedFile, HeaderData } from "../../shared/types";

interface CategoryMeta {
  label: string;
  icon: LucideIcon;
  color: string;
  glow: string;
}

const CATEGORY_META: Record<string, CategoryMeta> = {
  observation: { label: "Observation", icon: Telescope, color: "var(--ab-amber)", glow: "rgba(245,158,11,0.08)" },
  instrument: { label: "Instrument", icon: Camera, color: "var(--ab-cyan)", glow: "rgba(34,211,238,0.08)" },
  image: { label: "Image", icon: Image, color: "var(--ab-rose)", glow: "rgba(244,63,94,0.08)" },
  wcs: { label: "WCS / Astrometry", icon: Globe, color: "var(--ab-emerald)", glow: "rgba(16,185,129,0.08)" },
  processing: { label: "Processing", icon: Cpu, color: "var(--ab-violet)", glow: "rgba(139,92,246,0.08)" },
  other: { label: "Other", icon: MoreHorizontal, color: "var(--ab-sky)", glow: "rgba(14,165,233,0.06)" },
};

const CHANNEL_STYLES: Record<string, { bg: string; border: string; text: string; glow: string }> = {
  R: { bg: "rgba(239,68,68,0.08)", border: "rgba(239,68,68,0.25)", text: "#fca5a5", glow: "rgba(239,68,68,0.12)" },
  G: { bg: "rgba(34,197,94,0.08)", border: "rgba(34,197,94,0.25)", text: "#86efac", glow: "rgba(34,197,94,0.12)" },
  B: { bg: "rgba(59,130,246,0.08)", border: "rgba(59,130,246,0.25)", text: "#93c5fd", glow: "rgba(59,130,246,0.12)" },
};

const CONFIDENCE_STYLES: Record<string, { bg: string; text: string }> = {
  High: { bg: "rgba(16,185,129,0.15)", text: "#6ee7b7" },
  Medium: { bg: "rgba(245,158,11,0.15)", text: "#fcd34d" },
  Low: { bg: "rgba(113,113,122,0.15)", text: "#a1a1aa" },
};

const IMPORTANT_KEYS = new Set([
  "FILTER", "EXPTIME", "DATE-OBS", "OBJECT", "TARGNAME",
  "TELESCOP", "INSTRUME", "NAXIS1", "NAXIS2", "BITPIX",
]);

interface HeaderExplorerPanelProps {
  file: ProcessedFile | null;
  onLoadHeader: (path: string) => Promise<void> | void;
  headerData: HeaderData | null;
  isLoading?: boolean;
  onAssignChannel?: (channel: string, path: string) => void;
}

export default function HeaderExplorerPanel({
                                              file,
                                              onLoadHeader,
                                              headerData,
                                              isLoading = false,
                                              onAssignChannel,
                                            }: HeaderExplorerPanelProps) {
  const [search, setSearch] = useState("");
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    observation: true,
    instrument: true,
    image: false,
    wcs: false,
    processing: false,
    other: false,
  });
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [copiedAll, setCopiedAll] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const onLoadHeaderRef = useRef(onLoadHeader);
  onLoadHeaderRef.current = onLoadHeader;
  const prevPathRef = useRef<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!file?.path || file.path === prevPathRef.current) return;
    prevPathRef.current = file.path;
    setLoadError(false);
    setSearch("");
    const result = onLoadHeaderRef.current?.(file.path);
    if (result && typeof (result as Promise<void>).catch === "function") {
      (result as Promise<void>).catch(() => setLoadError(true));
    }
  }, [file?.path]);

  const toggleCategory = useCallback((cat: string) => {
    setExpanded((prev) => ({ ...prev, [cat]: !prev[cat] }));
  }, []);

  const expandAll = useCallback(() => {
    setExpanded((prev) => {
      const allExpanded = Object.values(prev).every(Boolean);
      const next: Record<string, boolean> = {};
      for (const key of Object.keys(prev)) next[key] = !allExpanded;
      return next;
    });
  }, []);

  const handleCopy = useCallback((key: string, value: string) => {
    navigator.clipboard?.writeText(`${key} = ${value}`);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 1500);
  }, []);

  const handleCopyAll = useCallback(() => {
    if (!headerData?.cards) return;
    const text = headerData.cards
      .map((c) => `${c.key.padEnd(8)} = ${c.value}`)
      .join("\n");
    navigator.clipboard?.writeText(text);
    setCopiedAll(true);
    setTimeout(() => setCopiedAll(false), 2000);
  }, [headerData?.cards]);

  const filteredCategories = useMemo(() => {
    if (!headerData?.categories) return {} as Record<string, Record<string, string>>;
    const q = search.toLowerCase().trim();
    if (!q) return headerData.categories;

    const result: Record<string, Record<string, string>> = {};
    for (const [cat, entries] of Object.entries(headerData.categories)) {
      const filtered: Record<string, string> = {};
      for (const [key, value] of Object.entries(entries)) {
        if (key.toLowerCase().includes(q) || String(value).toLowerCase().includes(q)) {
          filtered[key] = value;
        }
      }
      if (Object.keys(filtered).length > 0) result[cat] = filtered;
    }
    return result;
  }, [headerData?.categories, search]);

  const totalVisible = useMemo(() => {
    return Object.values(filteredCategories).reduce(
      (sum, entries) => sum + Object.keys(entries).length, 0,
    );
  }, [filteredCategories]);

  const handleAssign = useCallback((channel: string) => {
    if (onAssignChannel && file?.path) onAssignChannel(channel, file.path);
  }, [onAssignChannel, file?.path]);

  if (!file) {
    return (
      <div className="ab-panel p-8 flex flex-col items-center justify-center gap-3 animate-fade-in">
        <div
          className="w-10 h-10 rounded-xl flex items-center justify-center"
          style={{ background: "rgba(20,184,166,0.06)", border: "1px solid rgba(20,184,166,0.1)" }}
        >
          <FileSearch size={18} style={{ color: "var(--ab-teal)", opacity: 0.5 }} />
        </div>
        <p className="text-[11px] text-zinc-500">Select a FITS file to explore headers</p>
      </div>
    );
  }

  return (
    <div className="ab-panel overflow-hidden flex flex-col" style={{ maxHeight: "100%" }}>
      <div className="ab-panel-header">
        <div className="flex items-center gap-2">
          <FileSearch size={12} style={{ color: "var(--ab-amber)" }} />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            Header Explorer
          </span>
        </div>
        <div className="flex items-center gap-2">
          {headerData && (
            <span className="text-[10px] font-mono text-zinc-500">
              {totalVisible}/{headerData.total_cards}
            </span>
          )}
          {headerData?.cards && headerData.cards.length > 0 && (
            <button
              onClick={handleCopyAll}
              className="p-1 rounded hover:bg-zinc-800/60 transition-colors"
              title="Copy all headers"
            >
              {copiedAll
                ? <Check size={11} style={{ color: "var(--ab-emerald)" }} />
                : <ClipboardCopy size={11} className="text-zinc-500 hover:text-zinc-300" />}
            </button>
          )}
        </div>
      </div>

      {isLoading && (
        <div className="px-4 py-6 flex flex-col items-center gap-2">
          <div className="relative w-6 h-6">
            <div
              className="absolute inset-0 rounded-full animate-spin"
              style={{ border: "2px solid transparent", borderTopColor: "var(--ab-amber)", borderRightColor: "rgba(245,158,11,0.3)" }}
            />
          </div>
          <span className="text-[11px] text-zinc-500">Reading FITS header...</span>
        </div>
      )}

      {loadError && !isLoading && !headerData && (
        <div className="px-4 py-6 flex flex-col items-center gap-3">
          <div
            className="w-8 h-8 rounded-lg flex items-center justify-center"
            style={{ background: "rgba(239,68,68,0.08)", border: "1px solid rgba(239,68,68,0.15)" }}
          >
            <X size={14} style={{ color: "#f87171" }} />
          </div>
          <p className="text-[11px] text-zinc-400">Failed to read header</p>
          <button
            onClick={() => { setLoadError(false); onLoadHeader?.(file.path); }}
            className="text-[10px] font-medium px-4 py-1.5 rounded-md transition-all"
            style={{ background: "rgba(20,184,166,0.1)", border: "1px solid rgba(20,184,166,0.2)", color: "var(--ab-teal)" }}
          >
            Retry
          </button>
        </div>
      )}

      {headerData && !isLoading && (
        <>
          <div className="px-3 py-2 flex-shrink-0 space-y-2" style={{ borderBottom: "1px solid var(--ab-border)" }}>
            <div className="flex items-center gap-2">
              <span className="text-[11px] text-zinc-300 font-mono truncate flex-1">
                {headerData.file_name}
              </span>
              <button
                onClick={expandAll}
                className="text-[9px] font-medium px-2 py-0.5 rounded transition-colors text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/40"
              >
                {Object.values(expanded).every(Boolean) ? "Collapse all" : "Expand all"}
              </button>
            </div>

            {headerData.filter_detection && (
              <FilterDetectionBadge
                detection={headerData.filter_detection}
                onAssign={onAssignChannel ? handleAssign : undefined}
              />
            )}

            {!headerData.filter_detection && headerData.filename_hint && (
              <div
                className="flex items-center gap-2 px-3 py-2 rounded-md"
                style={{ background: "rgba(113,113,122,0.06)", border: "1px solid rgba(113,113,122,0.12)" }}
              >
                <Zap size={11} className="text-zinc-500 shrink-0" />
                <span className="text-[10px] text-zinc-400">
                  Filename hint: <span className="font-mono text-zinc-300">{headerData.filename_hint}</span>
                </span>
              </div>
            )}

            <div className="relative">
              <Search size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-zinc-500 pointer-events-none" />
              <input
                ref={searchRef}
                type="text"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Search keywords or values..."
                className="w-full pl-8 pr-8 py-1.5 text-[11px] text-zinc-300 rounded-md outline-none placeholder:text-zinc-600 transition-colors"
                style={{
                  background: "rgba(24,24,32,0.8)",
                  border: search ? "1px solid rgba(20,184,166,0.25)" : "1px solid rgba(63,63,70,0.4)",
                }}
              />
              {search && (
                <button
                  onClick={() => { setSearch(""); searchRef.current?.focus(); }}
                  className="absolute right-2.5 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-zinc-300 transition-colors"
                >
                  <X size={12} />
                </button>
              )}
            </div>

            {search && (
              <div className="text-[10px] text-zinc-500 px-1">
                {totalVisible === 0
                  ? <span style={{ color: "#f87171" }}>No matches</span>
                  : <>{totalVisible} {totalVisible === 1 ? "match" : "matches"} found</>}
              </div>
            )}
          </div>

          <div className="flex-1 overflow-y-auto min-h-0">
            {Object.entries(filteredCategories).map(([cat, entries]) => {
              const meta = CATEGORY_META[cat] || CATEGORY_META.other;
              const Icon = meta.icon;
              const isOpen = search ? true : expanded[cat];
              const count = Object.keys(entries).length;
              if (count === 0) return null;

              return (
                <div key={cat} style={{ borderBottom: "1px solid rgba(63,63,70,0.15)" }}>
                  <button
                    onClick={() => !search && toggleCategory(cat)}
                    className="w-full flex items-center gap-2 px-3 py-2 transition-colors"
                    style={{
                      background: isOpen ? meta.glow : "transparent",
                      cursor: search ? "default" : "pointer",
                    }}
                  >
                    <span className="transition-transform" style={{ transform: isOpen ? "rotate(90deg)" : "rotate(0deg)" }}>
                      <ChevronRight size={10} className="text-zinc-500" />
                    </span>
                    <Icon size={12} style={{ color: meta.color }} />
                    <span className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: meta.color }}>
                      {meta.label}
                    </span>
                    <span className="ml-auto text-[10px] font-mono" style={{ color: meta.color, opacity: 0.5 }}>
                      {count}
                    </span>
                  </button>

                  {isOpen && (
                    <div className="pb-1">
                      {Object.entries(entries).map(([key, value]) => {
                        const isHighlight = IMPORTANT_KEYS.has(key.toUpperCase());
                        const displayValue = String(value).replace(/^'|'$/g, "").trim();

                        return (
                          <div
                            key={key}
                            className="group flex items-center gap-2 mx-1 px-2 py-[3px] rounded transition-colors"
                            style={{ background: isHighlight ? "rgba(245,158,11,0.04)" : undefined }}
                            onMouseOver={(e) => (e.currentTarget.style.background = "rgba(63,63,70,0.2)")}
                            onMouseOut={(e) => (e.currentTarget.style.background = isHighlight ? "rgba(245,158,11,0.04)" : "")}
                          >
                            <span
                              className="text-[10px] font-mono w-[88px] shrink-0 truncate"
                              style={{
                                color: isHighlight ? "var(--ab-amber)" : "#71717a",
                                fontWeight: isHighlight ? 600 : 400,
                              }}
                              title={key}
                            >
                              {key}
                            </span>
                            <span
                              className="text-[10px] font-mono flex-1 truncate select-text"
                              style={{ color: "#d4d4d8" }}
                              title={displayValue}
                            >
                              {displayValue}
                            </span>
                            <button
                              onClick={() => handleCopy(key, displayValue)}
                              className="opacity-0 group-hover:opacity-100 p-0.5 rounded transition-all hover:bg-zinc-700/40"
                              title={`Copy ${key}`}
                            >
                              {copiedKey === key
                                ? <Check size={10} style={{ color: "var(--ab-emerald)" }} />
                                : <Copy size={10} className="text-zinc-500" />}
                            </button>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}

            {totalVisible === 0 && !search && !isLoading && (
              <div className="px-4 py-8 text-center text-[11px] text-zinc-600">
                No header data available
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}

function FilterDetectionBadge({
                                detection,
                                onAssign,
                              }: {
  detection: NonNullable<HeaderData["filter_detection"]>;
  onAssign?: (channel: string) => void;
}) {
  const ch = detection.hubble_channel;
  const style = CHANNEL_STYLES[ch] || CHANNEL_STYLES.B;
  const conf = CONFIDENCE_STYLES[detection.confidence] || CONFIDENCE_STYLES.Low;

  return (
    <div
      className="flex items-center gap-2.5 px-3 py-2 rounded-lg animate-fade-in"
      style={{ background: style.bg, border: `1px solid ${style.border}`, boxShadow: `0 0 20px ${style.glow}` }}
    >
      <Sparkles size={12} style={{ color: style.text, flexShrink: 0 }} />
      <div className="flex-1 min-w-0">
        <div className="text-[11px] font-medium" style={{ color: style.text }}>
          {detection.filter} → Channel {ch}
        </div>
        <div className="text-[9px] text-zinc-500 font-mono truncate">
          {detection.matched_keyword}: {detection.matched_value}
        </div>
      </div>
      <span
        className="text-[9px] font-medium px-2 py-0.5 rounded-full shrink-0"
        style={{ background: conf.bg, color: conf.text }}
      >
        {detection.confidence}
      </span>
      {onAssign && (
        <button
          onClick={() => onAssign(ch)}
          className="text-[10px] font-medium px-3 py-1 rounded-md shrink-0 transition-all"
          style={{
            background: `${style.border}`,
            color: style.text,
            border: `1px solid ${style.border}`,
          }}
          onMouseOver={(e) => { e.currentTarget.style.background = style.bg; e.currentTarget.style.boxShadow = `0 0 12px ${style.glow}`; }}
          onMouseOut={(e) => { e.currentTarget.style.background = style.border; e.currentTarget.style.boxShadow = "none"; }}
        >
          Assign {ch}
        </button>
      )}
    </div>
  );
}
