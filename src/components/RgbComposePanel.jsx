import { useState, useCallback } from "react";
import { Palette, Upload, Loader2, RefreshCw, Link2 } from "lucide-react";

/**
 * RgbComposePanel — assign R/G/B channels from separate FITS files.
 *
 * Props:
 *  - files: Array of available files from the queue
 *  - onCompose: (rPath, gPath, bPath, options) => Promise
 *  - result: compose result object (or null)
 *  - isLoading: boolean
 */
export default function RgbComposePanel({
  files = [],
  onCompose,
  result = null,
  isLoading = false,
}) {
  const [rFile, setRFile] = useState("");
  const [gFile, setGFile] = useState("");
  const [bFile, setBFile] = useState("");
  const [autoStretch, setAutoStretch] = useState(true);
  const [linkedStf, setLinkedStf] = useState(false);
  const [align, setAlign] = useState(true);
  const [wbMode, setWbMode] = useState("auto");

  const assignedCount = [rFile, gFile, bFile].filter(Boolean).length;
  const canCompose = assignedCount >= 2;

  const handleCompose = useCallback(() => {
    if (!canCompose || !onCompose) return;
    onCompose(rFile || null, gFile || null, bFile || null, {
      autoStretch,
      linkedStf,
      align,
      wbMode,
    });
  }, [rFile, gFile, bFile, autoStretch, linkedStf, align, wbMode, onCompose, canCompose]);

  const handleAutoAssign = useCallback(() => {
    
    const remaining = [...files];
    const patterns = {
      r: [/[_-]r[._-]/i, /ha|h.?alpha|red/i, /[_-]R\./],
      g: [/[_-]g[._-]/i, /oiii|o3|green/i, /[_-]G\./],
      b: [/[_-]b[._-]/i, /sii|s2|blue/i, /[_-]B\./],
    };

    let rMatch = "", gMatch = "", bMatch = "";
    for (const f of remaining) {
      const name = f.name || f.path || "";
      if (!rMatch && patterns.r.some((p) => p.test(name))) rMatch = f.path;
      else if (!gMatch && patterns.g.some((p) => p.test(name))) gMatch = f.path;
      else if (!bMatch && patterns.b.some((p) => p.test(name))) bMatch = f.path;
    }

    
    if (!rMatch && !gMatch && !bMatch && remaining.length >= 2) {
      rMatch = remaining[0]?.path || "";
      gMatch = remaining[1]?.path || "";
      bMatch = remaining[2]?.path || "";
    }

    if (rMatch) setRFile(rMatch);
    if (gMatch) setGFile(gMatch);
    if (bMatch) setBFile(bMatch);
  }, [files]);

  const ChannelSelect = ({ label, color, value, onChange }) => (
    <div className="flex items-center gap-2">
      <div
        className="w-3 h-3 rounded-full border"
        style={{
          backgroundColor: color + "33",
          borderColor: color,
        }}
      />
      <span className="text-[10px] text-zinc-400 w-3 font-bold">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="flex-1 bg-zinc-900 border border-zinc-700 rounded px-2 py-1 text-[11px] text-zinc-300 outline-none focus:border-zinc-500"
      >
        <option value="">— none —</option>
        {files.map((f) => (
          <option key={f.path || f.id} value={f.path}>
            {f.name || f.path}
          </option>
        ))}
      </select>
    </div>
  );

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Palette size={12} className="text-pink-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            RGB Compose
          </span>
        </div>
        {files.length >= 2 && (
          <button
            onClick={handleAutoAssign}
            className="text-[10px] text-zinc-500 hover:text-zinc-300 flex items-center gap-1"
            title="Auto-assign channels by filename"
          >
            <RefreshCw size={10} />
            Auto
          </button>
        )}
      </div>

      <div className="px-3 py-2 space-y-2">
        {/* Channel assignment */}
        <ChannelSelect label="R" color="#ef4444" value={rFile} onChange={setRFile} />
        <ChannelSelect label="G" color="#22c55e" value={gFile} onChange={setGFile} />
        <ChannelSelect label="B" color="#3b82f6" value={bFile} onChange={setBFile} />

        {/* Options */}
        <div className="flex flex-wrap gap-x-4 gap-y-1 pt-1">
          <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
            <input
              type="checkbox"
              checked={autoStretch}
              onChange={(e) => setAutoStretch(e.target.checked)}
              className="w-3 h-3 accent-pink-500"
            />
            Auto STF
          </label>
          <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
            <input
              type="checkbox"
              checked={linkedStf}
              onChange={(e) => setLinkedStf(e.target.checked)}
              className="w-3 h-3 accent-pink-500"
            />
            <Link2 size={9} />
            Linked
          </label>
          <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
            <input
              type="checkbox"
              checked={align}
              onChange={(e) => setAlign(e.target.checked)}
              className="w-3 h-3 accent-pink-500"
            />
            Align
          </label>
        </div>

        {/* White balance */}
        <div className="flex items-center gap-2">
          <label className="text-[10px] text-zinc-500">WB</label>
          <select
            value={wbMode}
            onChange={(e) => setWbMode(e.target.value)}
            className="bg-zinc-900 border border-zinc-700 rounded px-2 py-0.5 text-[10px] text-zinc-300 outline-none"
          >
            <option value="auto">Auto (Median)</option>
            <option value="none">None</option>
            <option value="manual">Manual</option>
          </select>
        </div>

        {/* Compose button */}
        <button
          onClick={handleCompose}
          disabled={!canCompose || isLoading}
          className="w-full flex items-center justify-center gap-2 bg-pink-600/20 hover:bg-pink-600/30 text-pink-300 border border-pink-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
        >
          {isLoading ? (
            <>
              <Loader2 size={12} className="animate-spin" />
              Composing...
            </>
          ) : (
            <>
              <Palette size={12} />
              Compose RGB ({assignedCount}/3 channels)
            </>
          )}
        </button>

        {/* Results */}
        {result && (
          <div className="space-y-1.5">
            {result.previewUrl && (
              <img
                src={result.previewUrl}
                alt="RGB composite"
                className="w-full rounded border border-zinc-700"
              />
            )}
            <div className="grid grid-cols-3 gap-1 text-[10px]">
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-red-400">R median</div>
                <div className="text-zinc-300 font-mono">{result.stats_r?.median?.toFixed(0)}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-green-400">G median</div>
                <div className="text-zinc-300 font-mono">{result.stats_g?.median?.toFixed(0)}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-blue-400">B median</div>
                <div className="text-zinc-300 font-mono">{result.stats_b?.median?.toFixed(0)}</div>
              </div>
            </div>
            {(result.offset_g || result.offset_b) && (
              <div className="text-[10px] text-zinc-500">
                Offsets — G: [{result.offset_g?.[0]}, {result.offset_g?.[1]}] B: [{result.offset_b?.[0]}, {result.offset_b?.[1]}]
              </div>
            )}
            <div className="text-[10px] text-zinc-500">{result.elapsed_ms} ms</div>
          </div>
        )}
      </div>
    </div>
  );
}
