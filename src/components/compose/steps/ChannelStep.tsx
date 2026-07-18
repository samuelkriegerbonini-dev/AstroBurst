import { useState, useCallback, useMemo, useRef, useEffect } from "react";
import { Wand2, FolderOpen, ChevronDown, X, Sparkles } from "lucide-react";
import type { ProcessedFile } from "../../../shared/types";
import { ingestFiles } from "../../../hooks/useFileIngest";
import {DEFAULT_BINS, FrequencyBin, WizardState} from "../../../utils/wizard";
import { filterCodeAndWavelengthNm } from "../../../utils/filterWavelengths";

interface NarrowbandPalette {
  palette_name: string;
  is_complete: boolean;
  r_file?: { file_path: string; file_name: string; detection?: unknown } | null;
  g_file?: { file_path: string; file_name: string; detection?: unknown } | null;
  b_file?: { file_path: string; file_name: string; detection?: unknown } | null;
  unmapped?: { file_path: string; file_name: string; detection?: unknown }[];
}

interface FilterDetection {
  path: string;
  filter: string | null;
  hubble_channel?: string | null;
  confidence?: number;
  matched_keyword?: string;
  matched_value?: string;
}

interface ChannelStepProps {
  state: WizardState;
  doneFiles: ProcessedFile[];
  onBinsChange: (bins: FrequencyBin[]) => void;
  narrowbandPalette?: NarrowbandPalette | null;
  filterDetections?: FilterDetection[];
}

const FILTER_TO_BIN: Record<string, string> = {
  "Halpha": "ha", "Ha": "ha", "H_alpha": "ha", "H-alpha": "ha",
  "OIII": "oiii", "O3": "oiii", "[OIII]": "oiii",
  "SII": "sii", "S2": "sii", "[SII]": "sii",
  "NII": "nii",
  "Red": "r", "R": "r",
  "Green": "g", "G": "g", "V": "g",
  "Blue": "b", "B": "b",
  "Luminance": "l", "Lum": "l", "Clear": "l", "CLR": "l", "L": "l",
};

const FILTER_PATTERNS: [string, RegExp][] = [
  ["ha", /(?:H[-_]?(?:alpha|a)|\b656\s*(?:nm)?|H_?α|F656N)/i],
  ["oiii", /(?:O\s*III|\[?OIII\]?|50[012](?:\.\d+)?\s*nm|\b50[12]\b|\b5007\b|O3\b|F50[123]N)/i],
  ["sii", /(?:S\s*II|\[?SII\]?|\b673\s*(?:nm)?|S2\b|F673N)/i],
  ["r", /\b(?:Red|R['_-]?band|Sloan[_-]?r)\b/i],
  ["g", /\b(?:Green|G['_-]?band|Sloan[_-]?g|V[_-]?band)\b/i],
  ["b", /\b(?:Blue|B['_-]?band|Sloan[_-]?b)\b/i],
  ["l", /\b(?:Lum(?:inance)?|L['_-]?band|Clear|CLR)\b/i],
];

const FILENAME_PATTERNS: [string, RegExp][] = [
  ["ha", /(?:[_-]HA[_\-.\s]|[_-]HALPHA|[_-]H_?ALPHA|656)/i],
  ["oiii", /(?:[_-]OIII[_\-.\s]|[_-]O3[_\-.\s]|502)/i],
  ["sii", /(?:[_-]SII[_\-.\s]|[_-]S2[_\-.\s]|673)/i],
  ["r", /(?:[_-]RED[_\-.\s]|[_-]R\.)/i],
  ["g", /(?:[_-]GREEN[_\-.\s]|[_-]G\.)/i],
  ["b", /(?:[_-]BLUE[_\-.\s]|[_-]B\.)/i],
  ["l", /(?:[_-]LUM[_\-.\s]|[_-]L\.|[_-]CLEAR)/i],
];

function detectChannelByHeader(file: ProcessedFile): string | null {
  const header = file.result?.header;
  if (!header) return null;

  const filterVal = (header.FILTER ?? header.FILTER1 ?? header.FILTER2 ?? "").toString().trim();
  if (!filterVal) return null;

  const directMatch = FILTER_TO_BIN[filterVal];
  if (directMatch) return directMatch;

  for (const [binId, pattern] of FILTER_PATTERNS) {
    if (pattern.test(filterVal)) return binId;
  }

  return null;
}

function detectChannelByFilename(file: ProcessedFile): string | null {
  const fname = file.name || file.path || "";
  for (const [binId, pattern] of FILENAME_PATTERNS) {
    if (pattern.test(fname)) return binId;
  }
  return null;
}

function detectChannel(file: ProcessedFile): string | null {
  return detectChannelByHeader(file) ?? detectChannelByFilename(file);
}

function shortName(path: string): string {
  return path.split(/[/\\]/).pop()?.replace(/\.(fits?|asdf)$/i, "") ?? path;
}

function getFilterInfo(file: ProcessedFile): string | null {
  const header = file.result?.header;
  if (!header) return null;
  const f = header.FILTER ?? header.FILTER1 ?? header.FILTER2;
  return f ? String(f).trim() : null;
}

interface BinDropdownProps {
  bin: FrequencyBin;
  files: ProcessedFile[];
  assignedSet: Set<string>;
  onSelect: (binId: string, filePath: string) => void;
}

function BinDropdown({ bin, files, assignedSet, onSelect }: BinDropdownProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const available = bin.id === "l"
    ? files
    : files.filter((f) => !assignedSet.has(f.path) || bin.files.includes(f.path));

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1 text-[9px] text-zinc-500 hover:text-zinc-300 transition-colors"
      >
        <ChevronDown size={10} className={open ? "rotate-180 transition-transform" : "transition-transform"} />
        Select
      </button>
      {open && (
        <div className="absolute z-50 top-full left-0 mt-1 min-w-[200px] max-h-[180px] overflow-y-auto bg-zinc-900 border border-zinc-700 rounded-lg shadow-xl">
          {available.length === 0 ? (
            <div className="px-3 py-2 text-[10px] text-zinc-600">No files available</div>
          ) : (
            available.map((f) => {
              const isInBin = bin.files.includes(f.path);
              const filterInfo = getFilterInfo(f);
              return (
                <button
                  key={f.path}
                  onClick={() => {
                    onSelect(bin.id, f.path);
                    setOpen(false);
                  }}
                  className={`w-full flex items-center gap-2 px-3 py-1.5 text-left text-[10px] transition-colors ${
                    isInBin
                      ? "bg-zinc-800 text-zinc-200"
                      : "text-zinc-400 hover:bg-zinc-800/60 hover:text-zinc-200"
                  }`}
                >
                  <span className="truncate flex-1">{f.name || shortName(f.path)}</span>
                  {filterInfo && (
                    <span
                      className="text-[8px] px-1.5 py-0.5 rounded-full shrink-0"
                      style={{
                        color: bin.color,
                        background: `${bin.color}15`,
                        borderColor: `${bin.color}30`,
                        border: "1px solid",
                      }}
                    >
                      {filterInfo}
                    </span>
                  )}
                  {isInBin && <span className="text-[8px] text-emerald-400">✓</span>}
                </button>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}

export default function ChannelStep({
                                      state,
                                      doneFiles,
                                      onBinsChange,
                                      narrowbandPalette,
                                      filterDetections,
                                    }: ChannelStepProps) {
  const [customLabel, setCustomLabel] = useState("");
  const [customWl, setCustomWl] = useState("");
  const [autoMapSource, setAutoMapSource] = useState<string | null>(null);

  const assignedSet = useMemo(() => {
    const s = new Set<string>();
    for (const bin of state.bins) for (const f of bin.files) s.add(f);
    return s;
  }, [state.bins]);

  const colorAssignedSet = useMemo(() => {
    const s = new Set<string>();
    for (const bin of state.bins) {
      if (bin.id === "l") continue;
      for (const f of bin.files) s.add(f);
    }
    return s;
  }, [state.bins]);

  const unassigned = useMemo(
    () => doneFiles.filter((f) => !assignedSet.has(f.path)),
    [doneFiles, assignedSet],
  );

  const handleAutoMap = useCallback(() => {
    const next = state.bins.map((b) => ({ ...b, files: [...b.files] }));

    if (narrowbandPalette?.is_complete) {
      const paletteMap: Record<string, string | undefined> = {};
      if (narrowbandPalette.r_file?.file_path) paletteMap[narrowbandPalette.r_file.file_path] = "r";
      if (narrowbandPalette.g_file?.file_path) paletteMap[narrowbandPalette.g_file.file_path] = "g";
      if (narrowbandPalette.b_file?.file_path) paletteMap[narrowbandPalette.b_file.file_path] = "b";

      let mapped = 0;
      for (const file of doneFiles) {
        if (assignedSet.has(file.path)) continue;
        const target = paletteMap[file.path];
        if (target) {
          const bin = next.find((b) => b.id === target);
          if (bin && !bin.files.includes(file.path)) {
            bin.files.push(file.path);
            mapped++;
          }
        }
      }
      if (mapped > 0) {
        onBinsChange(next);
        setAutoMapSource(narrowbandPalette.palette_name ?? "Palette");
        return;
      }
    }

    if (filterDetections && filterDetections.length > 0) {
      let mapped = 0;
      for (const det of filterDetections) {
        if (assignedSet.has(det.path)) continue;
        if (!det.filter) continue;

        let targetBin: string | null = null;
        const filterStr = String(det.filter);

        const direct = FILTER_TO_BIN[filterStr];
        if (direct) {
          targetBin = direct;
        } else {
          for (const [binId, pattern] of FILTER_PATTERNS) {
            if (pattern.test(filterStr)) { targetBin = binId; break; }
          }
        }

        if (det.hubble_channel) {
          const hc = String(det.hubble_channel).toLowerCase();
          if (hc === "r" || hc === "red") targetBin = targetBin ?? "r";
          else if (hc === "g" || hc === "green") targetBin = targetBin ?? "g";
          else if (hc === "b" || hc === "blue") targetBin = targetBin ?? "b";
        }

        if (targetBin) {
          const bin = next.find((b) => b.id === targetBin);
          if (bin && !bin.files.includes(det.path)) {
            bin.files.push(det.path);
            mapped++;
          }
        }
      }
      if (mapped > 0) {
        onBinsChange(next);
        setAutoMapSource("FITS Headers (Rust)");
        return;
      }
    }

    let headerMapped = 0;
    const dynamicBins: FrequencyBin[] = [];
    for (const file of doneFiles) {
      if (assignedSet.has(file.path)) continue;

      const ch = detectChannelByHeader(file);
      if (ch) {
        const bin = next.find((b) => b.id === ch);
        if (bin && !bin.files.includes(file.path)) {
          bin.files.push(file.path);
          headerMapped++;
        }
        continue;
      }

      const fv = getFilterInfo(file);
      const info = fv ? filterCodeAndWavelengthNm(fv) : null;
      if (!info) continue;

      const binId = `wl${info.nm}`;
      let bin = next.find((b) => b.id === binId) ?? dynamicBins.find((b) => b.id === binId);
      if (!bin) {
        const hue = Math.round((info.nm * 0.18) % 360);
        bin = {
          id: binId,
          label: `${info.code} (${info.nm}nm)`,
          shortLabel: info.code.slice(0, 5),
          wavelength: info.nm,
          color: `hsl(${hue}, 70%, 55%)`,
          files: [],
        };
        dynamicBins.push(bin);
      }
      if (!bin.files.includes(file.path)) {
        bin.files.push(file.path);
        headerMapped++;
      }
    }
    if (headerMapped > 0) {
      onBinsChange(dynamicBins.length > 0 ? [...next, ...dynamicBins] : next);
      setAutoMapSource(dynamicBins.length > 0 ? "FITS Headers + Wavelength" : "FITS Headers");
      return;
    }

    let fnameMapped = 0;
    for (const file of doneFiles) {
      if (assignedSet.has(file.path)) continue;
      const ch = detectChannelByFilename(file);
      if (ch) {
        const bin = next.find((b) => b.id === ch);
        if (bin && !bin.files.includes(file.path)) {
          bin.files.push(file.path);
          fnameMapped++;
        }
      }
    }
    if (fnameMapped > 0) {
      onBinsChange(next);
      setAutoMapSource("Filename");
      return;
    }

    setAutoMapSource(null);
  }, [state.bins, doneFiles, assignedSet, onBinsChange, narrowbandPalette, filterDetections]);

  const handleDrop = useCallback((binId: string, filePath: string) => {
    const next = state.bins.map((bin) => {
      if (bin.id === binId) {
        return bin.files.includes(filePath) ? bin : { ...bin, files: [...bin.files, filePath] };
      }
      if (bin.id === "l" || binId === "l") return bin;
      return { ...bin, files: bin.files.filter((f) => f !== filePath) };
    });
    onBinsChange(next);
    setAutoMapSource(null);
  }, [state.bins, onBinsChange]);

  const handleSelectFile = useCallback((binId: string, filePath: string) => {
    const next = state.bins.map((bin) => {
      if (bin.id === binId) {
        if (bin.files.includes(filePath)) {
          return { ...bin, files: bin.files.filter((f) => f !== filePath) };
        }
        return { ...bin, files: [...bin.files, filePath] };
      }
      if (bin.id === "l" || binId === "l") return bin;
      return { ...bin, files: bin.files.filter((f) => f !== filePath) };
    });
    onBinsChange(next);
  }, [state.bins, onBinsChange]);

  const handleRemoveFile = useCallback((binId: string, filePath: string) => {
    onBinsChange(state.bins.map((b) =>
      b.id === binId ? { ...b, files: b.files.filter((f) => f !== filePath) } : b
    ));
  }, [state.bins, onBinsChange]);

  const handleClearAll = useCallback(() => {
    onBinsChange(state.bins.map((b) => ({ ...b, files: [] })));
    setAutoMapSource(null);
  }, [state.bins, onBinsChange]);

  const handleOpenFolder = useCallback(async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: true,
        filters: [{ name: "FITS", extensions: ["fits", "fit", "fts", "FITS", "FIT", "FTS", "asdf"] }],
      });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      if (paths.length === 0) return;

      ingestFiles(paths.map((p) => ({ name: p.split(/[/\\]/).pop() || "Unknown", path: p, size: 0 })));
    } catch (err) {
      console.warn("[ChannelStep] Dialog not available:", err);
    }
  }, []);

  const handleAddBin = useCallback(() => {
    if (!customLabel.trim()) return;
    const id = customLabel.toLowerCase().replace(/\s+/g, "_").replace(/[^a-z0-9_]/g, "");
    if (state.bins.some((b) => b.id === id)) return;
    const wl = customWl ? parseInt(customWl) : undefined;
    const hue = (state.bins.length * 47 + 120) % 360;
    onBinsChange([...state.bins, {
      id,
      label: `${customLabel}${wl ? ` (${wl}nm)` : ""}`,
      shortLabel: customLabel.slice(0, 5),
      wavelength: wl,
      color: `hsl(${hue}, 70%, 55%)`,
      files: [],
    }]);
    setCustomLabel("");
    setCustomWl("");
  }, [customLabel, customWl, state.bins, onBinsChange]);

  const handleRemoveBin = useCallback((binId: string) => {
    if (DEFAULT_BINS.some((d) => d.id === binId)) return;
    onBinsChange(state.bins.filter((b) => b.id !== binId));
  }, [state.bins, onBinsChange]);

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-xs text-zinc-400">
            {doneFiles.length} files, {doneFiles.length - unassigned.length} assigned
          </span>
          {autoMapSource && (
            <span className="flex items-center gap-1 text-[9px] text-emerald-400/70 bg-emerald-500/10 px-1.5 py-0.5 rounded-full">
              <Sparkles size={9} />
              {autoMapSource}
            </span>
          )}
        </div>
        <div className="flex gap-1.5">
          <button onClick={handleOpenFolder}
                  className="flex items-center gap-1 px-2 py-1 rounded text-[10px] font-medium bg-zinc-800/70 text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 transition-all"
                  title="Open FITS files from folder"
          >
            <FolderOpen size={10} /> Open
          </button>
          <button onClick={handleAutoMap} disabled={unassigned.length === 0}
                  className="flex items-center gap-1 px-2 py-1 rounded text-[10px] font-medium bg-violet-600/20 text-violet-400 hover:bg-violet-600/30 transition-all disabled:opacity-30">
            <Wand2 size={10} /> Auto Map
          </button>
          <button onClick={handleClearAll} disabled={!state.bins.some((b) => b.files.length > 0)}
                  className="px-2 py-1 rounded text-[10px] text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50 transition-all disabled:opacity-30">
            Clear
          </button>
        </div>
      </div>

      <div className="flex flex-wrap gap-2">
        {state.bins.map((bin) => {
          const isCustom = !DEFAULT_BINS.some((d) => d.id === bin.id);
          return (
            <div key={bin.id}
                 className="flex flex-col gap-1 p-2 rounded-lg border min-w-[140px] flex-1 max-w-[200px] transition-all"
                 style={{
                   borderColor: bin.files.length > 0 ? `${bin.color}40` : "rgba(63,63,70,0.3)",
                   background: bin.files.length > 0 ? `${bin.color}08` : "rgba(24,24,27,0.3)",
                 }}
                 onDragOver={(e) => { e.preventDefault(); e.currentTarget.style.borderColor = bin.color; }}
                 onDragLeave={(e) => { e.currentTarget.style.borderColor = bin.files.length > 0 ? `${bin.color}40` : "rgba(63,63,70,0.3)"; }}
                 onDrop={(e) => {
                   e.preventDefault();
                   const fp = e.dataTransfer.getData("text/plain");
                   if (fp) handleDrop(bin.id, fp);
                   try {
                     const data = JSON.parse(e.dataTransfer.getData("application/astroburst-file"));
                     if (data?.path) handleDrop(bin.id, data.path);
                   } catch {}
                   e.currentTarget.style.borderColor = `${bin.color}40`;
                 }}
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5">
                  <span className="w-2 h-2 rounded-full" style={{ background: bin.color }} />
                  <span className="text-[10px] font-medium text-zinc-300">{bin.shortLabel}</span>
                  {bin.wavelength && <span className="text-[8px] text-zinc-600">{bin.wavelength}nm</span>}
                </div>
                <div className="flex items-center gap-1">
                  <BinDropdown
                    bin={bin}
                    files={doneFiles}
                    assignedSet={colorAssignedSet}
                    onSelect={handleSelectFile}
                  />
                  <span className="text-[9px] font-mono text-zinc-600">{bin.files.length}</span>
                  {isCustom && (
                    <button onClick={() => handleRemoveBin(bin.id)} className="text-zinc-600 hover:text-red-400 p-0.5">
                      <X size={10} />
                    </button>
                  )}
                </div>
              </div>
              <div className="flex flex-col gap-0.5 min-h-[24px]">
                {bin.files.length === 0 && (
                  <span className="text-[9px] text-zinc-700 italic">Drop FITS here or select</span>
                )}
                {bin.files.map((fp) => {
                  const f = doneFiles.find((df) => df.path === fp);
                  const filterInfo = f ? getFilterInfo(f) : null;
                  return (
                    <div key={fp} className="flex items-center justify-between gap-1 group">
                      <span className="text-[9px] font-mono text-zinc-500 truncate">{shortName(fp)}</span>
                      {filterInfo && (
                        <span
                          className="text-[7px] px-1 py-0.5 rounded shrink-0"
                          style={{ color: bin.color, background: `${bin.color}15` }}
                        >
                          {filterInfo}
                        </span>
                      )}
                      <button onClick={() => handleRemoveFile(bin.id, fp)}
                              className="text-zinc-700 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                        <X size={9} />
                      </button>
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>

      <div className="flex items-center gap-1.5 pt-1 border-t border-zinc-800/30">
        <input value={customLabel} onChange={(e) => setCustomLabel(e.target.value)}
               placeholder="Custom channel..."
               className="flex-1 text-[10px] bg-zinc-800/40 border border-zinc-700/50 rounded px-2 py-1 text-zinc-300 placeholder:text-zinc-700" />
        <input value={customWl} onChange={(e) => setCustomWl(e.target.value)}
               placeholder="nm" type="number"
               className="w-14 text-[10px] bg-zinc-800/40 border border-zinc-700/50 rounded px-2 py-1 text-zinc-300 placeholder:text-zinc-700 text-right" />
        <button onClick={handleAddBin} disabled={!customLabel.trim()}
                className="px-2 py-1 rounded text-[10px] bg-zinc-800 text-zinc-400 hover:text-zinc-200 transition-all disabled:opacity-30">
          Add
        </button>
      </div>

      {unassigned.length > 0 && (
        <div className="flex flex-col gap-1 pt-1">
          <span className="text-[9px] text-zinc-600 uppercase tracking-wider">Unassigned ({unassigned.length})</span>
          <div className="flex flex-wrap gap-1">
            {unassigned.map((f) => {
              const filterInfo = getFilterInfo(f);
              const detected = detectChannel(f);
              return (
                <span key={f.path}
                      draggable
                      onDragStart={(e) => {
                        e.dataTransfer.setData("text/plain", f.path);
                        e.dataTransfer.setData("application/astroburst-file", JSON.stringify({
                          id: f.id, path: f.path, name: f.name,
                        }));
                      }}
                      className="flex items-center gap-1 text-[9px] font-mono text-zinc-500 bg-zinc-800/50 rounded px-1.5 py-0.5 cursor-grab hover:text-zinc-300 hover:bg-zinc-800 transition-all"
                      title={`${f.path}${filterInfo ? ` [${filterInfo}]` : ""}${detected ? ` → ${detected}` : ""}`}
                >
                  {shortName(f.path)}
                  {filterInfo && (
                    <span className="text-[7px] text-violet-400/70 bg-violet-500/10 px-1 rounded">
                      {filterInfo}
                    </span>
                  )}
                  {detected && (
                    <span className="text-[7px] text-amber-400/70">→{detected}</span>
                  )}
                </span>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
