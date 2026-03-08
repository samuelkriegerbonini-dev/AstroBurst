import { useState, useCallback, useEffect } from "react";
import { Loader2, Database, ChevronRight, FileText } from "lucide-react";
import { useBackend } from "../../hooks/useBackend";

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

export default function HduSelectorPanel({ filePath, onSelectHdu }: HduSelectorPanelProps) {
  const { getFitsExtensions, getHeaderByHdu } = useBackend();
  const [extensions, setExtensions] = useState<HduInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);
  const [hduHeader, setHduHeader] = useState<any>(null);
  const [headerLoading, setHeaderLoading] = useState(false);

  useEffect(() => {
    if (!filePath) return;
    setLoading(true);
    setExtensions([]);
    setSelectedIdx(null);
    setHduHeader(null);

    getFitsExtensions(filePath)
      .then((result: any) => {
        const exts = result?.extensions || result || [];
        setExtensions(Array.isArray(exts) ? exts : []);
      })
      .catch((err: any) => {
        console.error("Failed to load FITS extensions:", err);
      })
      .finally(() => setLoading(false));
  }, [filePath, getFitsExtensions]);

  const handleSelectHdu = useCallback(
    async (idx: number) => {
      setSelectedIdx(idx);
      setHeaderLoading(true);
      setHduHeader(null);
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
    [filePath, getHeaderByHdu, onSelectHdu],
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center py-4">
        <Loader2 size={16} className="animate-spin text-zinc-500" />
      </div>
    );
  }

  if (extensions.length <= 1) return null;

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
      <div className="flex items-center gap-1.5 px-3 py-2 border-b border-zinc-800/50">
        <Database size={12} className="text-pink-400" />
        <span className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">
          FITS Extensions
        </span>
        <span className="text-[10px] text-zinc-600">({extensions.length} HDUs)</span>
      </div>

      <div className="max-h-[200px] overflow-y-auto">
        {extensions.map((ext, i) => {
          const isSelected = selectedIdx === (ext.index ?? i);
          const dims = ext.naxis?.length > 0 ? ext.naxis.join("x") : "";
          const typeLabel = ext.type || (i === 0 ? "PRIMARY" : "EXT");
          return (
            <button
              key={i}
              onClick={() => handleSelectHdu(ext.index ?? i)}
              className={`w-full flex items-center gap-2 px-3 py-2 text-left transition-all border-b border-zinc-800/30 last:border-b-0 ${
                isSelected
                  ? "bg-pink-500/10 text-zinc-200"
                  : "text-zinc-500 hover:bg-zinc-800/30 hover:text-zinc-300"
              }`}
            >
              <ChevronRight
                size={10}
                className={`shrink-0 transition-transform ${isSelected ? "rotate-90 text-pink-400" : ""}`}
              />
              <span className="text-[10px] font-mono shrink-0 w-[24px] text-zinc-600">
                #{ext.index ?? i}
              </span>
              <span
                className={`text-[9px] font-bold px-1.5 py-0.5 rounded shrink-0 ${
                  typeLabel === "IMAGE"
                    ? "bg-blue-500/15 text-blue-400"
                    : typeLabel === "TABLE" || typeLabel === "BINTABLE"
                    ? "bg-amber-500/15 text-amber-400"
                    : "bg-zinc-700/30 text-zinc-400"
                }`}
              >
                {typeLabel}
              </span>
              <span className="text-[11px] truncate flex-1">
                {ext.name || "(unnamed)"}
              </span>
              {dims && (
                <span className="text-[10px] font-mono text-zinc-600 shrink-0">
                  {dims}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {headerLoading && (
        <div className="flex items-center justify-center py-3 border-t border-zinc-800/50">
          <Loader2 size={14} className="animate-spin text-zinc-500" />
        </div>
      )}

      {hduHeader && !headerLoading && (
        <div className="border-t border-zinc-800/50 px-3 py-2 max-h-[150px] overflow-y-auto">
          <div className="flex items-center gap-1.5 mb-1.5">
            <FileText size={10} className="text-zinc-500" />
            <span className="text-[10px] text-zinc-500 font-medium">
              HDU #{selectedIdx} Header
            </span>
          </div>
          <div className="text-[10px] font-mono text-zinc-400 space-y-0.5">
            {typeof hduHeader === "object" &&
              Object.entries(hduHeader).slice(0, 30).map(([key, val]) => (
                <div key={key} className="flex gap-2">
                  <span className="text-zinc-500 shrink-0 w-[80px]">{key}</span>
                  <span className="text-zinc-300 truncate">{String(val)}</span>
                </div>
              ))}
          </div>
        </div>
      )}
    </div>
  );
}
