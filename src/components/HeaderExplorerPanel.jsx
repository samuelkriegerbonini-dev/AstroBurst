import { useState, useCallback, useEffect, useMemo } from "react";
import {
  FileSearch, ChevronDown, ChevronRight, Search, Telescope,
  Camera, Image, Globe, Cpu, MoreHorizontal, Sparkles, Zap,
  Copy, Check, X,
} from "lucide-react";

const CATEGORY_META = {
  observation: { label: "Observation", icon: Telescope, color: "text-amber-400", bg: "bg-amber-400/10", border: "border-amber-400/20" },
  instrument: { label: "Instrument", icon: Camera, color: "text-cyan-400", bg: "bg-cyan-400/10", border: "border-cyan-400/20" },
  image: { label: "Image", icon: Image, color: "text-pink-400", bg: "bg-pink-400/10", border: "border-pink-400/20" },
  wcs: { label: "WCS / Astrometry", icon: Globe, color: "text-emerald-400", bg: "bg-emerald-400/10", border: "border-emerald-400/20" },
  processing: { label: "Processing", icon: Cpu, color: "text-violet-400", bg: "bg-violet-400/10", border: "border-violet-400/20" },
  other: { label: "Other", icon: MoreHorizontal, color: "text-zinc-400", bg: "bg-zinc-400/10", border: "border-zinc-400/20" },
};

const CHANNEL_COLORS = {
  R: { bg: "bg-red-500/20", text: "text-red-300", border: "border-red-500/40" },
  G: { bg: "bg-green-500/20", text: "text-green-300", border: "border-green-500/40" },
  B: { bg: "bg-blue-500/20", text: "text-blue-300", border: "border-blue-500/40" },
};

const CONFIDENCE_STYLE = {
  High: "bg-emerald-500/20 text-emerald-300",
  Medium: "bg-amber-500/20 text-amber-300",
  Low: "bg-zinc-500/20 text-zinc-400",
};

export default function HeaderExplorerPanel({
  file = null,
  onLoadHeader,
  headerData = null,
  isLoading = false,
  onAssignChannel,
}) {
  const [search, setSearch] = useState("");
  const [expanded, setExpanded] = useState({
    observation: true,
    instrument: true,
    image: false,
    wcs: false,
    processing: false,
    other: false,
  });
  const [copiedKey, setCopiedKey] = useState(null);

  useEffect(() => {
    if (file?.path && onLoadHeader) {
      onLoadHeader(file.path);
    }
  }, [file?.path]);

  const toggleCategory = useCallback((cat) => {
    setExpanded((prev) => ({ ...prev, [cat]: !prev[cat] }));
  }, []);

  const handleCopy = useCallback((key, value) => {
    navigator.clipboard?.writeText(`${key} = ${value}`);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 1500);
  }, []);

  const filteredCategories = useMemo(() => {
    if (!headerData?.categories) return {};
    const q = search.toLowerCase().trim();
    if (!q) return headerData.categories;

    const result = {};
    for (const [cat, entries] of Object.entries(headerData.categories)) {
      const filtered = {};
      for (const [key, value] of Object.entries(entries)) {
        if (
          key.toLowerCase().includes(q) ||
          String(value).toLowerCase().includes(q)
        ) {
          filtered[key] = value;
        }
      }
      if (Object.keys(filtered).length > 0) {
        result[cat] = filtered;
      }
    }
    return result;
  }, [headerData?.categories, search]);

  const totalVisible = useMemo(() => {
    return Object.values(filteredCategories).reduce(
      (sum, entries) => sum + Object.keys(entries).length,
      0,
    );
  }, [filteredCategories]);

  const handleAssign = useCallback((channel) => {
    if (onAssignChannel && file?.path) {
      onAssignChannel(channel, file.path);
    }
  }, [onAssignChannel, file?.path]);

  if (!file) {
    return (
      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-6 flex flex-col items-center justify-center gap-2">
        <FileSearch size={24} className="text-zinc-600" />
        <p className="text-[11px] text-zinc-500">Select a FITS file to explore headers</p>
      </div>
    );
  }

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden flex flex-col max-h-full">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50 flex-shrink-0">
        <div className="flex items-center gap-2">
          <FileSearch size={12} className="text-amber-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            Header Explorer
          </span>
        </div>
        {headerData && (
          <span className="text-[10px] text-zinc-500">
            {totalVisible}/{headerData.total_cards} cards
          </span>
        )}
      </div>

      {isLoading && (
        <div className="px-3 py-4 flex items-center justify-center gap-2 text-[11px] text-zinc-400">
          <div className="w-3 h-3 border border-amber-400/40 border-t-amber-400 rounded-full animate-spin" />
          Reading FITS header…
        </div>
      )}

      {headerData && !isLoading && (
        <>
          <div className="px-3 py-2 border-b border-zinc-800/50 flex-shrink-0 space-y-2">
            <div className="text-[11px] text-zinc-300 font-mono truncate">
              {headerData.file_name}
            </div>

            {headerData.filter_detection && (
              <div className={`flex items-center gap-2 px-2.5 py-1.5 rounded-md border ${CHANNEL_COLORS[headerData.filter_detection.hubble_channel]?.bg || ""} ${CHANNEL_COLORS[headerData.filter_detection.hubble_channel]?.border || "border-zinc-700"}`}>
                <Sparkles size={11} className="text-amber-300" />
                <div className="flex-1 min-w-0">
                  <div className={`text-[11px] font-medium ${CHANNEL_COLORS[headerData.filter_detection.hubble_channel]?.text || "text-zinc-300"}`}>
                    {headerData.filter_detection.filter} → Channel {headerData.filter_detection.hubble_channel}
                  </div>
                  <div className="text-[9px] text-zinc-500 truncate">
                    via {headerData.filter_detection.matched_keyword}: "{headerData.filter_detection.matched_value}"
                  </div>
                </div>
                <span className={`text-[9px] px-1.5 py-0.5 rounded ${CONFIDENCE_STYLE[headerData.filter_detection.confidence] || CONFIDENCE_STYLE.Low}`}>
                  {headerData.filter_detection.confidence}
                </span>
                {onAssignChannel && (
                  <button
                    onClick={() => handleAssign(headerData.filter_detection.hubble_channel)}
                    className="text-[9px] bg-white/10 hover:bg-white/20 text-white rounded px-2 py-0.5 transition-colors"
                  >
                    Assign
                  </button>
                )}
              </div>
            )}

            {!headerData.filter_detection && headerData.filename_hint && (
              <div className="flex items-center gap-2 px-2.5 py-1.5 rounded-md border border-zinc-700/50 bg-zinc-800/30">
                <Zap size={11} className="text-zinc-400" />
                <span className="text-[10px] text-zinc-400">
                  Filename hint: {headerData.filename_hint}
                </span>
              </div>
            )}

            <div className="relative">
              <Search size={12} className="absolute left-2 top-1/2 -translate-y-1/2 text-zinc-500" />
              <input
                type="text"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Search keywords or values…"
                className="w-full bg-zinc-900 border border-zinc-700 rounded pl-7 pr-7 py-1 text-[11px] text-zinc-300 outline-none focus:border-zinc-500 placeholder:text-zinc-600"
              />
              {search && (
                <button
                  onClick={() => setSearch("")}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-zinc-300"
                >
                  <X size={11} />
                </button>
              )}
            </div>
          </div>

          <div className="flex-1 overflow-y-auto min-h-0 custom-scrollbar">
            {Object.entries(filteredCategories).map(([cat, entries]) => {
              const meta = CATEGORY_META[cat] || CATEGORY_META.other;
              const Icon = meta.icon;
              const isOpen = expanded[cat];
              const count = Object.keys(entries).length;

              if (count === 0) return null;

              return (
                <div key={cat} className="border-b border-zinc-800/30 last:border-b-0">
                  <button
                    onClick={() => toggleCategory(cat)}
                    className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-zinc-800/30 transition-colors"
                  >
                    {isOpen ? (
                      <ChevronDown size={10} className="text-zinc-500" />
                    ) : (
                      <ChevronRight size={10} className="text-zinc-500" />
                    )}
                    <Icon size={11} className={meta.color} />
                    <span className={`text-[10px] font-semibold ${meta.color} uppercase tracking-wider`}>
                      {meta.label}
                    </span>
                    <span className={`text-[9px] ${meta.color} opacity-50 ml-auto`}>
                      {count}
                    </span>
                  </button>

                  {isOpen && (
                    <div className="px-1 pb-1">
                      {Object.entries(entries).map(([key, value]) => {
                        const isHighlight = [
                          "FILTER", "EXPTIME", "DATE-OBS", "OBJECT", "TARGNAME",
                          "TELESCOP", "INSTRUME",
                        ].includes(key.toUpperCase());

                        return (
                          <div
                            key={key}
                            className={`group flex items-center gap-2 px-2 py-0.5 rounded hover:bg-zinc-800/40 transition-colors ${isHighlight ? "bg-zinc-800/20" : ""}`}
                          >
                            <span className={`text-[10px] font-mono w-20 flex-shrink-0 truncate ${isHighlight ? "text-amber-300 font-semibold" : "text-zinc-400"}`}>
                              {key}
                            </span>
                            <span className="text-[10px] text-zinc-300 font-mono flex-1 truncate">
                              {String(value).replace(/^'|'$/g, "").trim()}
                            </span>
                            <button
                              onClick={() => handleCopy(key, value)}
                              className="opacity-0 group-hover:opacity-100 transition-opacity text-zinc-500 hover:text-zinc-300"
                            >
                              {copiedKey === key ? (
                                <Check size={10} className="text-emerald-400" />
                              ) : (
                                <Copy size={10} />
                              )}
                            </button>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}

            {totalVisible === 0 && search && (
              <div className="px-3 py-6 text-center text-[11px] text-zinc-500">
                No headers match "{search}"
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
