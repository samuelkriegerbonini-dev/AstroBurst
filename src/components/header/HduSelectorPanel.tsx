import { useState, useCallback, useEffect, useMemo } from "react";
import { Database, ChevronRight, FileText, Search, X, Copy, Check } from "lucide-react";
import { getFitsExtensions, getHeaderByHdu } from "../../services/header.service";

interface HduSelectorPanelProps {
  filePath: string;
  onSelectHdu?: (hduIndex: number, header: any) => void;
}

interface HduInfo {
  index: number;
  name: string;
  type: string;
  naxis: number[];
  bitpix: number;
}

const TYPE_STYLES: Record<string, { bg: string; border: string; text: string }> = {
  IMAGE: { bg: "rgba(59,130,246,0.1)", border: "rgba(59,130,246,0.25)", text: "#93c5fd" },
  TABLE: { bg: "rgba(245,158,11,0.1)", border: "rgba(245,158,11,0.25)", text: "#fcd34d" },
  BINTABLE: { bg: "rgba(245,158,11,0.1)", border: "rgba(245,158,11,0.25)", text: "#fcd34d" },
  PRIMARY: { bg: "rgba(139,92,246,0.1)", border: "rgba(139,92,246,0.25)", text: "#c4b5fd" },
};

const DEFAULT_TYPE_STYLE = { bg: "rgba(113,113,122,0.1)", border: "rgba(113,113,122,0.2)", text: "#a1a1aa" };

export default function HduSelectorPanel({ filePath, onSelectHdu }: HduSelectorPanelProps) {
  const [extensions, setExtensions] = useState<HduInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);
  const [hduHeader, setHduHeader] = useState<Record<string, any> | null>(null);
  const [headerLoading, setHeaderLoading] = useState(false);
  const [hduSearch, setHduSearch] = useState("");
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  useEffect(() => {
    if (!filePath) return;
    setLoading(true);
    setExtensions([]);
    setSelectedIdx(null);
    setHduHeader(null);
    setHduSearch("");

    getFitsExtensions(filePath)
      .then((result: any) => {
        const exts = result?.extensions || result || [];
        setExtensions(Array.isArray(exts) ? exts : []);
      })
      .catch((err: any) => console.error("Failed to load FITS extensions:", err))
      .finally(() => setLoading(false));
  }, [filePath]);

  const handleSelectHdu = useCallback(
    async (idx: number) => {
      setSelectedIdx(idx);
      setHeaderLoading(true);
      setHduHeader(null);
      setHduSearch("");
      try {
        const header = await getHeaderByHdu(filePath, idx);
        setHduHeader(header);
        onSelectHdu?.(idx, header);
      } catch (err) {
        console.error("Failed to load HDU header:", err);
      } finally {
        setHeaderLoading(false);
      }
    },
    [filePath, onSelectHdu],
  );

  const handleCopyKey = useCallback((key: string, val: string) => {
    navigator.clipboard?.writeText(`${key} = ${val}`);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 1500);
  }, []);

  const filteredHduEntries = useMemo(() => {
    if (!hduHeader || typeof hduHeader !== "object") return [];
    const all = Object.entries(hduHeader);
    const q = hduSearch.toLowerCase().trim();
    if (!q) return all;
    return all.filter(
      ([k, v]) => k.toLowerCase().includes(q) || String(v).toLowerCase().includes(q),
    );
  }, [hduHeader, hduSearch]);

  if (loading) {
    return (
      <div className="ab-panel flex items-center justify-center py-5">
        <div
          className="w-5 h-5 rounded-full animate-spin"
          style={{ border: "2px solid transparent", borderTopColor: "var(--ab-rose)", borderRightColor: "rgba(244,63,94,0.3)" }}
        />
      </div>
    );
  }

  if (extensions.length <= 1) return null;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="ab-panel-header">
        <div className="flex items-center gap-2">
          <Database size={12} style={{ color: "var(--ab-rose)" }} />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            FITS Extensions
          </span>
        </div>
        <span className="text-[10px] font-mono text-zinc-500">
          {extensions.length} HDUs
        </span>
      </div>

      <div className="max-h-[200px] overflow-y-auto">
        {extensions.map((ext, i) => {
          const idx = ext.index ?? i;
          const isSelected = selectedIdx === idx;
          const dims = ext.naxis?.length > 0 ? ext.naxis.join(" × ") : "";
          const typeLabel = ext.type || (i === 0 ? "PRIMARY" : "EXT");
          const ts = TYPE_STYLES[typeLabel] || DEFAULT_TYPE_STYLE;

          return (
            <button
              key={i}
              onClick={() => handleSelectHdu(idx)}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-left transition-all"
              style={{
                background: isSelected ? "rgba(244,63,94,0.06)" : "transparent",
                borderBottom: "1px solid rgba(63,63,70,0.12)",
              }}
              onMouseOver={(e) => { if (!isSelected) e.currentTarget.style.background = "rgba(63,63,70,0.15)"; }}
              onMouseOut={(e) => { if (!isSelected) e.currentTarget.style.background = "transparent"; }}
            >
              <span
                className="transition-transform shrink-0"
                style={{ transform: isSelected ? "rotate(90deg)" : "rotate(0deg)" }}
              >
                <ChevronRight size={10} style={{ color: isSelected ? "var(--ab-rose)" : "#52525b" }} />
              </span>
              <span className="text-[10px] font-mono shrink-0 w-[24px]" style={{ color: "#52525b" }}>
                #{idx}
              </span>
              <span
                className="text-[9px] font-bold px-2 py-0.5 rounded shrink-0"
                style={{ background: ts.bg, border: `1px solid ${ts.border}`, color: ts.text }}
              >
                {typeLabel}
              </span>
              <span
                className="text-[11px] truncate flex-1"
                style={{ color: isSelected ? "#e4e4e7" : "#71717a" }}
              >
                {ext.name || "(unnamed)"}
              </span>
              {dims && (
                <span className="text-[10px] font-mono shrink-0" style={{ color: "#52525b" }}>
                  {dims}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {headerLoading && (
        <div className="flex items-center justify-center py-4" style={{ borderTop: "1px solid var(--ab-border)" }}>
          <div
            className="w-4 h-4 rounded-full animate-spin"
            style={{ border: "2px solid transparent", borderTopColor: "var(--ab-rose)", borderRightColor: "rgba(244,63,94,0.3)" }}
          />
        </div>
      )}

      {hduHeader && !headerLoading && (
        <div style={{ borderTop: "1px solid var(--ab-border)" }}>
          <div className="flex items-center gap-2 px-3 py-1.5" style={{ background: "rgba(244,63,94,0.03)" }}>
            <FileText size={10} style={{ color: "var(--ab-rose)", opacity: 0.6 }} />
            <span className="text-[10px] font-medium text-zinc-400">
              HDU #{selectedIdx}
            </span>
            <span className="text-[10px] font-mono text-zinc-600 ml-auto">
              {filteredHduEntries.length} keys
            </span>
          </div>

          {Object.keys(hduHeader).length > 8 && (
            <div className="px-3 py-1.5" style={{ borderBottom: "1px solid rgba(63,63,70,0.12)" }}>
              <div className="relative">
                <Search size={10} className="absolute left-2 top-1/2 -translate-y-1/2 text-zinc-600 pointer-events-none" />
                <input
                  type="text"
                  value={hduSearch}
                  onChange={(e) => setHduSearch(e.target.value)}
                  placeholder="Filter HDU keys..."
                  className="w-full pl-6 pr-6 py-1 text-[10px] text-zinc-300 rounded outline-none placeholder:text-zinc-600 transition-colors"
                  style={{
                    background: "rgba(24,24,32,0.6)",
                    border: hduSearch ? "1px solid rgba(244,63,94,0.2)" : "1px solid rgba(63,63,70,0.3)",
                  }}
                />
                {hduSearch && (
                  <button
                    onClick={() => setHduSearch("")}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-zinc-300"
                  >
                    <X size={10} />
                  </button>
                )}
              </div>
            </div>
          )}

          <div className="max-h-[220px] overflow-y-auto px-1 py-1">
            {filteredHduEntries.map(([key, val]) => (
              <div
                key={key}
                className="group flex items-center gap-2 px-2 py-[3px] rounded transition-colors"
                onMouseOver={(e) => (e.currentTarget.style.background = "rgba(63,63,70,0.2)")}
                onMouseOut={(e) => (e.currentTarget.style.background = "")}
              >
                <span className="text-[10px] font-mono w-[80px] shrink-0 truncate text-zinc-500" title={key}>
                  {key}
                </span>
                <span className="text-[10px] font-mono text-zinc-300 truncate flex-1 select-text" title={String(val)}>
                  {String(val)}
                </span>
                <button
                  onClick={() => handleCopyKey(key, String(val))}
                  className="opacity-0 group-hover:opacity-100 p-0.5 rounded transition-all hover:bg-zinc-700/40"
                >
                  {copiedKey === key
                    ? <Check size={9} style={{ color: "var(--ab-emerald)" }} />
                    : <Copy size={9} className="text-zinc-600" />}
                </button>
              </div>
            ))}
            {filteredHduEntries.length === 0 && hduSearch && (
              <div className="text-[10px] text-zinc-600 text-center py-3">
                No keys match "{hduSearch}"
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
