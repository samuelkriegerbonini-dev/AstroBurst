import { useState, useCallback, useMemo, useRef, useEffect, useId } from "react";
import { createPortal } from "react-dom";
import { Wand2, FolderOpen, ChevronDown, X, Sparkles, AlertTriangle } from "lucide-react";
import type { ProcessedFile } from "../../../shared/types";
import { ingestFiles } from "../../../hooks/useFileIngest";
import {
  BIN_MENU_MAX_HEIGHT,
  BIN_MENU_WIDTH,
  binMenuKeyAction,
  binMenuPlacement,
  DEFAULT_BINS,
  type FrequencyBin,
  type MenuPlacement,
  type WizardState,
} from "../../../utils/wizard";
import {
  UnmappedFile,
  assignedPaths,
  detectChannel,
  displayFilterValue,
  exclusivelyAssignedPaths,
  runAutoMap,
  shortName,
} from "../../../utils/channelMapping";
import { usePointingOverlap } from "../../../hooks/usePointingOverlap";
import type { NarrowbandFilterDetection } from "../../../services/header";

interface NarrowbandPalette {
  palette_name: string;
  is_complete: boolean;
  r_file?: { file_path: string; file_name: string; detection?: unknown } | null;
  g_file?: { file_path: string; file_name: string; detection?: unknown } | null;
  b_file?: { file_path: string; file_name: string; detection?: unknown } | null;
  unmapped?: { file_path: string; file_name: string; detection?: unknown }[];
}

type FilterDetection = NarrowbandFilterDetection;

interface ChannelStepProps {
  state: WizardState;
  doneFiles: ProcessedFile[];
  onBinsChange: (bins: FrequencyBin[]) => void;
  narrowbandPalette?: NarrowbandPalette | null;
  filterDetections?: FilterDetection[];
}

function getFilterInfo(file: ProcessedFile): string | null {
  return displayFilterValue(file);
}

interface BinDropdownProps {
  bin: FrequencyBin;
  files: ProcessedFile[];
  assignedSet: Set<string>;
  onSelect: (binId: string, filePath: string) => void;
}

function BinDropdown({ bin, files, assignedSet, onSelect }: BinDropdownProps) {
  const [placement, setPlacement] = useState<MenuPlacement | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const open = placement !== null;

  const closeToButton = useCallback(() => {
    setPlacement(null);
    buttonRef.current?.focus({ preventScroll: true });
  }, []);

  const menuItems = useCallback(
    () => Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>("[role=menuitemcheckbox]") ?? []),
    [],
  );

  useEffect(() => {
    if (!open) return;
    const target = menuItems()[0] ?? menuRef.current;
    target?.focus({ preventScroll: true });
  }, [open, menuItems]);

  const onMenuKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>) => {
    const items = menuItems();
    const current = items.findIndex((item) => item === document.activeElement);
    const action = binMenuKeyAction(e.key, current, items.length);
    if (!action) return;
    e.stopPropagation();
    if (action.type === "close") {
      if (action.preventDefault) e.preventDefault();
      closeToButton();
      return;
    }
    e.preventDefault();
    items[action.index]?.focus();
  }, [menuItems, closeToButton]);

  const onMenuBlur = useCallback((e: React.FocusEvent<HTMLDivElement>) => {
    const next = e.relatedTarget;
    if (!(next instanceof Node)) return;
    if (menuRef.current?.contains(next) || buttonRef.current?.contains(next)) return;
    setPlacement(null);
  }, []);

  const toggle = useCallback(() => {
    if (placement) {
      setPlacement(null);
      return;
    }
    const rect = buttonRef.current?.getBoundingClientRect();
    if (rect) setPlacement(binMenuPlacement(rect, { width: window.innerWidth, height: window.innerHeight }));
  }, [placement]);

  useEffect(() => {
    if (!open) return;
    const insideMenu = (target: EventTarget | null) => target instanceof Node && !!menuRef.current?.contains(target);
    const close = () => {
      const focusInside = insideMenu(document.activeElement);
      setPlacement(null);
      if (focusInside) buttonRef.current?.focus({ preventScroll: true });
    };
    const onPointerDown = (e: MouseEvent) => {
      if (!insideMenu(e.target) && !(e.target instanceof Node && buttonRef.current?.contains(e.target))) close();
    };
    const onScroll = (e: Event) => {
      if (!insideMenu(e.target)) close();
    };
    document.addEventListener("mousedown", onPointerDown);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", close);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", close);
    };
  }, [open]);

  const available = bin.id === "l"
    ? files
    : files.filter((f) => !assignedSet.has(f.path) || bin.files.includes(f.path));

  return (
    <div className="relative">
      <button
        ref={buttonRef}
        onClick={toggle}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={`Select files for ${bin.shortLabel}`}
        className="flex items-center gap-1 text-[9px] text-zinc-500 hover:text-zinc-300 transition-colors"
      >
        <ChevronDown size={10} className={open ? "rotate-180 transition-transform" : "transition-transform"} />
        Select
      </button>
      {placement && createPortal(
        <div
          ref={menuRef}
          id={menuId}
          role="menu"
          aria-label={`Files for ${bin.shortLabel}`}
          tabIndex={-1}
          onKeyDown={onMenuKeyDown}
          onBlur={onMenuBlur}
          className="fixed z-50 overflow-y-auto bg-zinc-900 border border-zinc-700 rounded-lg shadow-xl outline-none"
          style={{
            left: placement.left,
            top: placement.top ?? undefined,
            bottom: placement.bottom ?? undefined,
            width: BIN_MENU_WIDTH,
            maxHeight: BIN_MENU_MAX_HEIGHT,
          }}
        >
          {available.length === 0 ? (
            <div className="px-3 py-2 text-[10px] text-zinc-600">No files available</div>
          ) : (
            available.map((f) => {
              const isInBin = bin.files.includes(f.path);
              const filterInfo = getFilterInfo(f);
              return (
                <button
                  key={f.path}
                  role="menuitemcheckbox"
                  aria-checked={isInBin}
                  tabIndex={-1}
                  onClick={() => {
                    onSelect(bin.id, f.path);
                    closeToButton();
                  }}
                  title={f.name || f.path}
                  className={`w-full flex items-center gap-2 px-3 py-1.5 text-left text-[10px] transition-colors outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-violet-400/70 ${
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
                  {isInBin && <span aria-hidden="true" className="text-[8px] text-emerald-400">✓</span>}
                </button>
              );
            })
          )}
        </div>,
        document.body,
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
  const [unmappedFiles, setUnmappedFiles] = useState<UnmappedFile[]>([]);
  const [armedAction, setArmedAction] = useState<string | null>(null);
  const armedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => {
    if (armedTimerRef.current) clearTimeout(armedTimerRef.current);
  }, []);

  const armAction = useCallback((key: string) => {
    if (armedTimerRef.current) clearTimeout(armedTimerRef.current);
    setArmedAction(key);
    armedTimerRef.current = setTimeout(() => setArmedAction(null), 4000);
  }, []);

  const disarm = useCallback(() => {
    if (armedTimerRef.current) clearTimeout(armedTimerRef.current);
    setArmedAction(null);
  }, []);

  const assignedSet = useMemo(() => assignedPaths(state.bins), [state.bins]);

  const colorAssignedSet = useMemo(() => exclusivelyAssignedPaths(state.bins), [state.bins]);

  const unassigned = useMemo(
    () => doneFiles.filter((f) => !assignedSet.has(f.path)),
    [doneFiles, assignedSet],
  );

  const overlapPaths = useMemo(() => {
    const filled = state.bins.filter((b) => b.files.length > 0);
    if (filled.length < 2) return [];
    return Array.from(new Set(filled.flatMap((b) => b.files))).sort();
  }, [state.bins]);
  const { disjointPairs } = usePointingOverlap(overlapPaths);

  const handleAutoMap = useCallback(() => {
    const result = runAutoMap({
      bins: state.bins,
      files: doneFiles,
      assigned: assignedSet,
      palette: narrowbandPalette,
      detections: filterDetections,
    });
    setUnmappedFiles(result.unmapped);
    if (result.mappedCount === 0) {
      setAutoMapSource(null);
      return;
    }
    onBinsChange(result.bins);
    setAutoMapSource(result.sources.join(" + "));
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
    const key = `file:${binId}:${filePath}`;
    if (armedAction !== key) {
      armAction(key);
      return;
    }
    disarm();
    onBinsChange(state.bins.map((b) =>
      b.id === binId ? { ...b, files: b.files.filter((f) => f !== filePath) } : b
    ));
  }, [state.bins, onBinsChange, armedAction, armAction, disarm]);

  const handleClearAll = useCallback(() => {
    if (armedAction !== "clear") {
      armAction("clear");
      return;
    }
    disarm();
    onBinsChange(state.bins.map((b) => ({ ...b, files: [] })));
    setAutoMapSource(null);
    setUnmappedFiles([]);
  }, [state.bins, onBinsChange, armedAction, armAction, disarm]);

  const handleOpenFolder = useCallback(async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: true,
        filters: [{ name: "FITS", extensions: ["fits", "fit", "fts", "fz", "FITS", "FIT", "FTS", "FZ", "asdf"] }],
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
    const bin = state.bins.find((b) => b.id === binId);
    const key = `bin:${binId}`;
    if (bin && bin.files.length > 0 && armedAction !== key) {
      armAction(key);
      return;
    }
    disarm();
    onBinsChange(state.bins.filter((b) => b.id !== binId));
  }, [state.bins, onBinsChange, armedAction, armAction, disarm]);

  const assignedFileCount = useMemo(
    () => state.bins.reduce((acc, b) => acc + b.files.length, 0),
    [state.bins],
  );

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
          <button onClick={handleClearAll} onBlur={() => { if (armedAction === "clear") disarm(); }}
                  disabled={assignedFileCount === 0}
                  className={`px-2 py-1 rounded text-[10px] transition-all disabled:opacity-30 ${
                    armedAction === "clear"
                      ? "text-amber-300 bg-amber-500/15 ring-1 ring-amber-500/30"
                      : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"
                  }`}>
            {armedAction === "clear"
              ? `Confirm — clears ${assignedFileCount} assignment${assignedFileCount === 1 ? "" : "s"}?`
              : "Clear"}
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
                    <button onClick={() => handleRemoveBin(bin.id)}
                            onBlur={() => { if (armedAction === `bin:${bin.id}`) disarm(); }}
                            aria-label={armedAction === `bin:${bin.id}`
                              ? `Confirm removing channel ${bin.shortLabel} and its ${bin.files.length} frame(s)`
                              : `Remove channel ${bin.shortLabel}`}
                            title={armedAction === `bin:${bin.id}`
                              ? `Click again to remove ${bin.shortLabel} and unassign ${bin.files.length} frame(s)`
                              : `Remove channel ${bin.shortLabel}`}
                            className={`p-0.5 transition-colors ${
                              armedAction === `bin:${bin.id}`
                                ? "text-amber-300 bg-amber-500/15 rounded"
                                : "text-zinc-600 hover:text-red-400"
                            }`}>
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
                              onBlur={() => { if (armedAction === `file:${bin.id}:${fp}`) disarm(); }}
                              aria-label={armedAction === `file:${bin.id}:${fp}`
                                ? `Confirm removing ${shortName(fp)} from ${bin.shortLabel}`
                                : `Remove ${shortName(fp)} from ${bin.shortLabel}`}
                              title={armedAction === `file:${bin.id}:${fp}`
                                ? `Click again to remove ${shortName(fp)}`
                                : `Remove ${shortName(fp)}`}
                              className={`transition-opacity shrink-0 ${
                                armedAction === `file:${bin.id}:${fp}`
                                  ? "text-amber-300 opacity-100"
                                  : "text-zinc-700 hover:text-red-400 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                              }`}>
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

      {disjointPairs.length > 0 && (
        <div className="flex items-start gap-1.5 text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25 rounded px-2 py-1.5">
          <AlertTriangle size={12} className="shrink-0 mt-px" />
          <span>
            {disjointPairs.slice(0, 3).map((p) => `${shortName(p.a)} × ${shortName(p.b)}`).join(", ")}
            {disjointPairs.length > 3 ? ` and ${disjointPairs.length - 3} more pair(s)` : ""}
            {" "}point at different sky regions
            {disjointPairs[0].separation_arcmin != null ? ` (~${disjointPairs[0].separation_arcmin.toFixed(1)}′ apart)` : ""}
            {" "}— their WCS footprints do not overlap, so blending them produces a patchwork with no common signal. Check the channel assignment.
          </span>
        </div>
      )}

      {unmappedFiles.length > 0 && (
        <div className="flex items-start gap-1.5 text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25 rounded px-2 py-1.5">
          <AlertTriangle size={12} className="shrink-0 mt-px" />
          <span>
            Auto Map could not resolve {unmappedFiles.length} file(s):{" "}
            {unmappedFiles.slice(0, 4).map((u) => `${u.name}${u.filter ? ` [${u.filter}]` : " [no FILTER keyword]"}`).join(", ")}
            {unmappedFiles.length > 4 ? ` and ${unmappedFiles.length - 4} more` : ""}
            {" "}— their filter is not in the wavelength table, so they stay unassigned. Drop them into a channel manually.
          </span>
        </div>
      )}

      <div className="flex items-center gap-1.5 pt-1 border-t border-zinc-800/30">
        <input value={customLabel} onChange={(e) => setCustomLabel(e.target.value)}
               aria-label="Custom channel name"
               placeholder="Custom channel..."
               className="flex-1 text-[10px] bg-zinc-800/40 border border-zinc-700/50 rounded px-2 py-1 text-zinc-300 placeholder:text-zinc-700" />
        <input value={customWl} onChange={(e) => setCustomWl(e.target.value)}
               aria-label="Custom channel wavelength (nm)"
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
