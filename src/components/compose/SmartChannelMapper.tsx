import { toggleCalibFrame, withAllFramesOfKind } from "./calibFrames";
import {
  useState,
  useCallback,
  useMemo,
  memo,
  useRef,
  useEffect,
} from "react";
import {
  Palette,
  Sparkles,
  Sun,
  ArrowLeftRight,
  X,
  GripVertical,
  Wand2,
  ChevronDown,
  Check,
} from "lucide-react";
import { RunButton, SectionHeader } from "../ui";
import { filterCodeAndWavelengthNm } from "../../utils/filterWavelengths";
import {
  filenameChannelSlot,
  groupByWavelength,
  resolveFilterFromName,
  splitSpectralThirds,
} from "../../utils/channelMapping";
import { FRAME_KIND_NAMES, isCalibrationKind, type FrameKind } from "../../utils/frameKind";

export interface ChannelFile {
  id: string;
  path: string;
  name: string;
  filter?: string;
  instrument?: string;
  exptime?: number;
  previewUrl?: string;
  frameKind?: FrameKind;
}

export type ChannelSlot = "L" | "R" | "G" | "B";
export type CalibSlot = "science" | "bias" | "dark" | "flat" | "flatdark";
export type CalibFrameSlot = Exclude<CalibSlot, "science">;
export type MapperMode = "rgb" | "calibration";

export interface ChannelAssignment {
  L: ChannelFile | null;
  R: ChannelFile | null;
  G: ChannelFile | null;
  B: ChannelFile | null;
}

export interface CalibAssignment {
  science: ChannelFile | null;
  bias: ChannelFile[];
  dark: ChannelFile[];
  flat: ChannelFile[];
  flatdark: ChannelFile[];
}

interface SmartChannelMapperProps {
  mode: MapperMode;
  files: ChannelFile[];
  onComposeRgb?: (
    assignments: ChannelAssignment,
    options: Record<string, unknown>,
  ) => void;
  onCalibrate?: (assignments: CalibAssignment) => void;
  isLoading?: boolean;
  composeOptions?: Record<string, unknown>;
  hideButton?: boolean;
  onAssignmentChange?: (assignments: ChannelAssignment) => void;
  paletteSuggestion?: {
    palette_name: string;
    is_complete: boolean;
    r_file?: { file_path: string } | null;
    g_file?: { file_path: string } | null;
    b_file?: { file_path: string } | null;
  } | null;
  selectedPalette?: string;
  onPaletteChange?: (palette: string) => void;
  calibAssignment?: CalibAssignment;
  onCalibAssignmentChange?: React.Dispatch<React.SetStateAction<CalibAssignment>>;
  note?: React.ReactNode;
}

const NO_CALIB_FRAMES: CalibAssignment = { science: null, bias: [], dark: [], flat: [], flatdark: [] };
const CALIB_FRAME_SLOTS: readonly CalibFrameSlot[] = ["bias", "dark", "flat", "flatdark"];
const VISIBLE_FRAMES = 5;



const CHANNEL_META: Record<ChannelSlot, { color: string; label: string; bg: string; border: string }> = {
  L: { color: "#e4e4e7", label: "Luminance", bg: "rgba(228,228,231,0.06)", border: "rgba(228,228,231,0.2)" },
  R: { color: "#ef4444", label: "Red", bg: "rgba(239,68,68,0.06)", border: "rgba(239,68,68,0.25)" },
  G: { color: "#22c55e", label: "Green", bg: "rgba(34,197,94,0.06)", border: "rgba(34,197,94,0.25)" },
  B: { color: "#3b82f6", label: "Blue", bg: "rgba(59,130,246,0.06)", border: "rgba(59,130,246,0.25)" },
};

const CALIB_META: Record<CalibSlot, { color: string; label: string; bg: string; border: string }> = {
  science: { color: "#f59e0b", label: "Science", bg: "rgba(245,158,11,0.06)", border: "rgba(245,158,11,0.25)" },
  bias: { color: "#a78bfa", label: "Bias", bg: "rgba(167,139,250,0.06)", border: "rgba(167,139,250,0.2)" },
  dark: { color: "#60a5fa", label: "Dark", bg: "rgba(96,165,250,0.06)", border: "rgba(96,165,250,0.2)" },
  flat: { color: "#fbbf24", label: "Flat", bg: "rgba(251,191,36,0.06)", border: "rgba(251,191,36,0.2)" },
  flatdark: { color: "#f472b6", label: "Flat dark", bg: "rgba(244,114,182,0.06)", border: "rgba(244,114,182,0.2)" },
};

const PALETTE_PRESETS = [
  { id: "SHO", label: "SHO (Hubble)", desc: "SII\u2192R  Ha\u2192G  OIII\u2192B", icon: "\u2728" },
  { id: "HOO", label: "HOO", desc: "Ha\u2192R  OIII\u2192G+B", icon: "\ud83c\udf11" },
  { id: "HOS", label: "HOS", desc: "Ha\u2192R  OIII\u2192G  SII\u2192B", icon: "\ud83c\udf0c" },
  { id: "NaturalColor", label: "Natural Color", desc: "Ha\u2192R  OIII\u2192G+B (natural)", icon: "\ud83c\udf0d" },
  { id: "Custom", label: "Custom", desc: "Manual assignment", icon: "\ud83c\udfa8" },
] as const;

function fileFilterInfo(file: ChannelFile): { code: string; nm: number } | null {
  return filterCodeAndWavelengthNm(file.filter) ?? resolveFilterFromName(file.name || file.path || "");
}

function autoMapByMetadata(files: ChannelFile[]): Partial<ChannelAssignment> {
  const entries = files
    .map((file) => ({ item: file, info: fileFilterInfo(file) }))
    .filter((x): x is { item: ChannelFile; info: { code: string; nm: number } } => x.info !== null)
    .map((x) => ({ item: x.item, code: x.info.code, nm: x.info.nm }));

  if (entries.length === 0) return {};

  const spectral = splitSpectralThirds(groupByWavelength(entries));
  const result: Partial<ChannelAssignment> = {};
  if (spectral.r.length > 0) result.R = spectral.r[0];
  if (spectral.g.length > 0) result.G = spectral.g[0];
  if (spectral.b.length > 0) result.B = spectral.b[0];
  return result;
}

function autoMapByFilename(files: ChannelFile[]): Partial<ChannelAssignment> {
  const result: Partial<ChannelAssignment> = {};
  for (const f of files) {
    if (Object.values(result).some((v) => v?.id === f.id)) continue;
    const slot = filenameChannelSlot(f.name || f.path || "");
    if (slot && !result[slot]) result[slot] = f;
  }
  return result;
}

function FilterChip({ filter, color }: { filter: string; color: string }) {
  return (
    <span
      className="ab-filter-chip"
      style={{
        color,
        background: `${color}15`,
        borderColor: `${color}30`,
      }}
    >
      {filter}
    </span>
  );
}

interface DropZoneSlotProps {
  slot: string;
  label: string;
  color: string;
  bg: string;
  border: string;
  file: ChannelFile | null;
  onDrop: (slot: string, file: ChannelFile) => void;
  onClear: (slot: string) => void;
  allFiles: ChannelFile[];
  multi?: boolean;
  count?: number;
  frames?: ChannelFile[];
  onToggle?: (slot: string, file: ChannelFile) => void;
  children?: React.ReactNode;
}

function CalibFrameList({ frames, label, onRemove }: { frames: ChannelFile[]; label: string; onRemove: (file: ChannelFile) => void }) {
  return (
    <ul className="flex flex-col">
      {frames.map((f) => (
        <li key={f.id} className="flex items-center gap-1.5 py-0.5">
          <span className="ab-channel-filename min-w-0 flex-1" title={f.name}>{f.name}</span>
          {f.exptime != null && <span className="ab-channel-exptime shrink-0">{f.exptime}s</span>}
          <button
            type="button"
            className="ab-channel-clear"
            aria-label={`Remove ${f.name} from ${label}`}
            title={`Remove ${f.name}`}
            onClick={() => onRemove(f)}
          >
            <X size={10} />
          </button>
        </li>
      ))}
    </ul>
  );
}

function DropZoneSlot({
  slot,
  label,
  color,
  bg,
  border,
  file,
  onDrop,
  onClear,
  allFiles,
  multi = false,
  count = 0,
  frames = [],
  onToggle,
  children,
}: DropZoneSlotProps) {
  const [isDragOver, setIsDragOver] = useState(false);
  const [showDropdown, setShowDropdown] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const togglesFrames = multi && onToggle !== undefined;
  const rowIds = useMemo(() => new Set(frames.map((f) => f.id)), [frames]);

  const pick = (picked: ChannelFile) => {
    if (togglesFrames && onToggle) {
      onToggle(slot, picked);
      return;
    }
    onDrop(slot, picked);
    setShowDropdown(false);
  };

  const closeOnEscape = (e: React.KeyboardEvent) => {
    if (e.key !== "Escape" || !showDropdown) return;
    e.stopPropagation();
    setShowDropdown(false);
    toggleRef.current?.focus();
  };

  useEffect(() => {
    if (!showDropdown) return;
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setShowDropdown(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [showDropdown]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    setIsDragOver(true);
  }, []);

  const handleDragLeave = useCallback(() => setIsDragOver(false), []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setIsDragOver(false);
      try {
        const data = JSON.parse(e.dataTransfer.getData("application/astroburst-file"));
        if (data?.id) onDrop(slot, data as ChannelFile);
      } catch {}
    },
    [slot, onDrop],
  );

  return (
    <div className="ab-channel-slot-wrapper" ref={dropdownRef} onKeyDown={closeOnEscape}>
      <div className="relative">
        <div
          className={`ab-channel-slot ${isDragOver ? "ab-channel-slot-dragover" : ""} ${file ? "ab-channel-slot-filled" : ""}`}
          style={{
            borderColor: isDragOver ? color : file ? border : "rgba(63,63,70,0.3)",
            background: isDragOver ? `${color}12` : file ? bg : "rgba(24,24,32,0.5)",
          }}
          onDragOver={handleDragOver}
          onDragLeave={handleDragLeave}
          onDrop={handleDrop}
        >
          <div className="ab-channel-indicator" style={{ background: color }} />

          <div className="ab-channel-label">
            <span className="ab-channel-letter" style={{ color }}>{slot}</span>
            <span className="ab-channel-name">{label}</span>
          </div>

          {file ? (
            <div className="ab-channel-file-info">
              {!togglesFrames && file.previewUrl && (
                <img src={file.previewUrl} alt="" className="ab-channel-thumb" />
              )}
              <div className="ab-channel-file-detail">
                {togglesFrames ? (
                  <span className="text-[11px] font-medium text-zinc-300">{`${count} frame${count === 1 ? "" : "s"}`}</span>
                ) : (
                  <>
                    <span className="ab-channel-filename" title={file.name}>{file.name}</span>
                    <div className="ab-channel-meta-row">
                      {file.filter && <FilterChip filter={file.filter} color={color} />}
                      {file.exptime != null && (
                        <span className="ab-channel-exptime">{file.exptime}s</span>
                      )}
                    </div>
                  </>
                )}
              </div>
              {multi && (
                <button
                  ref={toggleRef}
                  className="ab-channel-browse"
                  style={{ flex: "0 0 auto", width: "auto", padding: "2px 6px" }}
                  onClick={(e) => {
                    e.stopPropagation();
                    setShowDropdown((v) => !v);
                  }}
                  title={togglesFrames ? `Pick ${label.toLowerCase()} frames` : "Add another frame"}
                  aria-label={`Select ${label} frames`}
                  aria-expanded={showDropdown}
                >
                  <ChevronDown size={12} />
                  <span>+</span>
                </button>
              )}
              <button
                className="ab-channel-clear"
                onClick={(e) => {
                  e.stopPropagation();
                  onClear(slot);
                }}
                title={multi && count > 1 ? `Clear all ${count} frames` : "Remove"}
                aria-label={multi && count > 1 ? `Clear all ${count} ${label} frames` : `Clear ${label} slot`}
              >
                <X size={12} />
              </button>
            </div>
          ) : (
            <button
              ref={toggleRef}
              className="ab-channel-browse"
              onClick={() => setShowDropdown((v) => !v)}
              aria-label={`Select ${label} ${multi ? "frames" : "file"}`}
              aria-expanded={showDropdown}
            >
              <ChevronDown size={12} />
              <span>Drop file or select</span>
            </button>
          )}
        </div>

        {showDropdown && (
          <div className="ab-channel-dropdown">
            {allFiles.length === 0 ? (
              <div className="ab-channel-dropdown-empty">No files available</div>
            ) : (
              allFiles.map((f) => (
                <button
                  key={f.id}
                  className="ab-channel-dropdown-item"
                  aria-pressed={togglesFrames ? rowIds.has(f.id) : undefined}
                  onClick={() => pick(f)}
                >
                  <span className="truncate">{f.name}</span>
                  {togglesFrames ? (
                    <span className="flex items-center gap-1 shrink-0">
                      {f.filter && <FilterChip filter={f.filter} color={color} />}
                      <Check size={11} aria-hidden="true" className={rowIds.has(f.id) ? "" : "invisible"} style={{ color }} />
                    </span>
                  ) : (
                    f.filter && <FilterChip filter={f.filter} color={color} />
                  )}
                </button>
              ))
            )}
          </div>
        )}
      </div>

      {children}

      {togglesFrames && frames.length > 0 && (
        <div className="mt-1 ml-3 flex flex-col">
          <CalibFrameList frames={frames.slice(0, VISIBLE_FRAMES)} label={label} onRemove={pick} />
          {frames.length > VISIBLE_FRAMES && (
            <details>
              <summary className="cursor-pointer text-[10px] text-zinc-500 hover:text-zinc-300">{`${frames.length - VISIBLE_FRAMES} more`}</summary>
              <CalibFrameList frames={frames.slice(VISIBLE_FRAMES)} label={label} onRemove={pick} />
            </details>
          )}
        </div>
      )}
    </div>
  );
}

function SmartChannelMapper({
  mode,
  files,
  onComposeRgb,
  onCalibrate,
  isLoading = false,
  composeOptions,
  hideButton = false,
  onAssignmentChange,
  paletteSuggestion,
  selectedPalette = "SHO",
  onPaletteChange,
  calibAssignment,
  onCalibAssignmentChange,
  note,
}: SmartChannelMapperProps) {
  const [channels, setChannels] = useState<ChannelAssignment>({
    L: null, R: null, G: null, B: null,
  });
  const [ownCalibFrames, setOwnCalibFrames] = useState<CalibAssignment>(NO_CALIB_FRAMES);
  const calibFrames = calibAssignment ?? ownCalibFrames;
  const setCalibFrames = onCalibAssignmentChange ?? setOwnCalibFrames;
  const [autoMapSource, setAutoMapSource] = useState<"metadata" | "filename" | "palette" | null>(null);

  const onAssignmentChangeRef = useRef(onAssignmentChange);
  onAssignmentChangeRef.current = onAssignmentChange;

  useEffect(() => {
    if (mode === "rgb" && onAssignmentChangeRef.current) onAssignmentChangeRef.current(channels);
  }, [channels, mode]);

  const assignChannel = useCallback((slot: string, file: ChannelFile) => {
    if (mode === "rgb") {
      setChannels((prev) => ({ ...prev, [slot]: file }));
      setAutoMapSource(null);
    } else {
      const s = slot as CalibSlot;
      setCalibFrames((prev) => {
        if (s === "science") return { ...prev, science: file };
        const existing = prev[s] as ChannelFile[];
        if (existing.some((f) => f.id === file.id)) return prev;
        return { ...prev, [s]: [...existing, file] };
      });
    }
  }, [mode, setCalibFrames]);

  const clearChannel = useCallback((slot: string) => {
    if (mode === "rgb") {
      setChannels((prev) => ({ ...prev, [slot]: null }));
    } else {
      const s = slot as CalibSlot;
      setCalibFrames((prev) =>
        s === "science" ? { ...prev, science: null } : { ...prev, [s]: [] },
      );
    }
  }, [mode, setCalibFrames]);

  const toggleCalibSlotFrame = useCallback((slot: string, file: ChannelFile) => {
    setCalibFrames((prev) => toggleCalibFrame(prev, slot as CalibFrameSlot, file));
  }, [setCalibFrames]);

  const addAllFramesOfKind = useCallback((slot: CalibFrameSlot) => {
    setCalibFrames((prev) => withAllFramesOfKind(prev, slot, files));
  }, [files, setCalibFrames]);

  const framesOfKind = useMemo(() => {
    const counts: Record<CalibFrameSlot, number> = { bias: 0, dark: 0, flat: 0, flatdark: 0 };
    for (const f of files) {
      if (f.frameKind && isCalibrationKind(f.frameKind)) counts[f.frameKind] += 1;
    }
    return counts;
  }, [files]);

  const swapChannels = useCallback((a: ChannelSlot, b: ChannelSlot) => {
    setChannels((prev) => ({ ...prev, [a]: prev[b], [b]: prev[a] }));
  }, []);

  const handleAutoMap = useCallback(() => {
    if (paletteSuggestion?.is_complete && paletteSuggestion.r_file && paletteSuggestion.g_file && paletteSuggestion.b_file) {
      const find = (path: string) => files.find((f) => f.path === path) ?? null;
      setChannels((prev) => ({
        ...prev,
        R: find(paletteSuggestion.r_file!.file_path),
        G: find(paletteSuggestion.g_file!.file_path),
        B: find(paletteSuggestion.b_file!.file_path),
      }));
      setAutoMapSource("palette");
      return;
    }

    const metaResult = autoMapByMetadata(files);
    if (metaResult.R || metaResult.G || metaResult.B) {
      setChannels((prev) => ({
        ...prev,
        L: metaResult.L ?? prev.L,
        R: metaResult.R ?? prev.R,
        G: metaResult.G ?? prev.G,
        B: metaResult.B ?? prev.B,
      }));
      setAutoMapSource("metadata");
      return;
    }

    const nameResult = autoMapByFilename(files);
    if (nameResult.R || nameResult.G || nameResult.B) {
      setChannels((prev) => ({
        ...prev,
        L: nameResult.L ?? prev.L,
        R: nameResult.R ?? prev.R,
        G: nameResult.G ?? prev.G,
        B: nameResult.B ?? prev.B,
      }));
      setAutoMapSource("filename");
      return;
    }

    if (files.length >= 2) {
      setChannels({
        L: null,
        R: files[0] ?? null,
        G: files[1] ?? null,
        B: files[2] ?? null,
      });
      setAutoMapSource(null);
    }
  }, [files, paletteSuggestion]);

  const prevPaletteSuggestionRef = useRef(paletteSuggestion);
  useEffect(() => {
    if (paletteSuggestion === prevPaletteSuggestionRef.current) return;
    prevPaletteSuggestionRef.current = paletteSuggestion;
    if (selectedPalette === "Custom") return;
    if (paletteSuggestion?.is_complete && paletteSuggestion.r_file && paletteSuggestion.g_file && paletteSuggestion.b_file) {
      const find = (path: string) => files.find((f) => f.path === path) ?? null;
      setChannels((prev) => ({
        ...prev,
        R: find(paletteSuggestion.r_file!.file_path),
        G: find(paletteSuggestion.g_file!.file_path),
        B: find(paletteSuggestion.b_file!.file_path),
      }));
      setAutoMapSource("palette");
    }
  }, [paletteSuggestion, files, selectedPalette]);

  const handlePaletteChange = useCallback((paletteId: string) => {
    if (onPaletteChange) onPaletteChange(paletteId);
    if (paletteId === "Custom") {
      setAutoMapSource(null);
    }
  }, [onPaletteChange]);

  const assignedRgbCount = useMemo(
    () => [channels.R, channels.G, channels.B].filter(Boolean).length,
    [channels],
  );

  const canCompose = assignedRgbCount >= 2;

  const handleCompose = useCallback(() => {
    if (!onComposeRgb || !canCompose) return;
    onComposeRgb(channels, composeOptions ?? {});
  }, [channels, canCompose, onComposeRgb, composeOptions]);

  const handleCalibrate = useCallback(() => {
    if (!onCalibrate || !calibFrames.science) return;
    onCalibrate(calibFrames);
  }, [calibFrames, onCalibrate]);

  const ICON = mode === "rgb"
    ? <Palette size={14} className="text-pink-400" />
    : <Sun size={14} className="text-violet-400" />;

  const title = mode === "rgb" ? "RGB Channel Mapper" : "Calibration Mapper";

  return (
    <div className="ab-mapper-root">
      <div className="ab-mapper-header">
        <SectionHeader icon={ICON} title={title} />
        {mode === "rgb" && (
          <div className="flex items-center gap-2">
            {autoMapSource && (
              <span className="ab-mapper-source-badge">
                <Sparkles size={10} />
                {autoMapSource === "palette"
                  ? paletteSuggestion?.palette_name ?? "Palette"
                  : autoMapSource === "metadata"
                    ? "FITS Headers"
                    : "Filename"}
              </span>
            )}
            <button
              onClick={handleAutoMap}
              disabled={files.length < 2}
              className="ab-mapper-auto-btn"
              title="Auto-assign channels using FITS metadata, then filename patterns"
            >
              <Wand2 size={12} />
              Auto-Map
            </button>
          </div>
        )}
      </div>

      {note}

      {mode === "rgb" && (
        <div className="ab-palette-selector">
          {PALETTE_PRESETS.map((p) => (
            <button
              key={p.id}
              className={`ab-palette-chip ${selectedPalette === p.id ? "ab-palette-chip-active" : ""}`}
              onClick={() => handlePaletteChange(p.id)}
              title={p.desc}
            >
              <span className="ab-palette-chip-icon">{p.icon}</span>
              <span>{p.label}</span>
            </button>
          ))}
        </div>
      )}

      <div className="ab-mapper-hint">
        <GripVertical size={11} className="text-zinc-600 shrink-0" />
        <span>Click a slot to pick a file, or drag from the file list on the left</span>
      </div>

      {mode === "rgb" ? (
        <div className="ab-mapper-slots">
          {(["L", "R", "G", "B"] as ChannelSlot[]).map((slot, i) => (
            <div key={slot}>
              <DropZoneSlot
                slot={slot}
                label={CHANNEL_META[slot].label}
                color={CHANNEL_META[slot].color}
                bg={CHANNEL_META[slot].bg}
                border={CHANNEL_META[slot].border}
                file={channels[slot]}
                onDrop={assignChannel}
                onClear={clearChannel}
                allFiles={files}
              />
              {slot !== "B" && slot !== "L" && i < 3 && (
                <button
                  className="ab-mapper-swap"
                  onClick={() => {
                    const slots: ChannelSlot[] = ["L", "R", "G", "B"];
                    swapChannels(slots[i], slots[i + 1]);
                  }}
                  title={`Swap ${slot} and ${(["L", "R", "G", "B"] as ChannelSlot[])[i + 1]}`}
                >
                  <ArrowLeftRight size={10} />
                </button>
              )}
            </div>
          ))}
        </div>
      ) : (
        <div className="ab-mapper-slots">
          <DropZoneSlot
            slot="science"
            label={CALIB_META.science.label}
            color={CALIB_META.science.color}
            bg={CALIB_META.science.bg}
            border={CALIB_META.science.border}
            file={calibFrames.science}
            onDrop={assignChannel}
            onClear={clearChannel}
            allFiles={files}
            count={calibFrames.science ? 1 : 0}
          />
          {CALIB_FRAME_SLOTS.map((slot) => {
            const frames = calibFrames[slot];
            const ofKind = framesOfKind[slot];
            const allIn = withAllFramesOfKind(calibFrames, slot, files) === calibFrames;
            return (
              <DropZoneSlot
                key={slot}
                slot={slot}
                label={CALIB_META[slot].label}
                color={CALIB_META[slot].color}
                bg={CALIB_META[slot].bg}
                border={CALIB_META[slot].border}
                file={frames[0] ?? null}
                onDrop={assignChannel}
                onClear={clearChannel}
                allFiles={files}
                multi
                count={frames.length}
                frames={frames}
                onToggle={toggleCalibSlotFrame}
              >
                {ofKind > 0 && (
                  <button
                    type="button"
                    onClick={() => addAllFramesOfKind(slot)}
                    disabled={allIn}
                    className="mt-1 ml-3 text-[10px] text-zinc-400 hover:text-zinc-200 transition-colors disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:text-zinc-400"
                  >
                    {`Add all ${FRAME_KIND_NAMES[slot]} frames (${ofKind})`}
                  </button>
                )}
              </DropZoneSlot>
            );
          })}
        </div>
      )}

      {!hideButton && (
        <div className="ab-mapper-actions">
          {mode === "rgb" ? (
            <RunButton
              label={
                channels.L
                  ? `Compose LRGB (${assignedRgbCount}/3 + L)`
                  : `Compose RGB (${assignedRgbCount}/3 channels)`
              }
              runningLabel="Composing..."
              running={isLoading}
              disabled={!canCompose}
              accent="violet"
              onClick={handleCompose}
            />
          ) : (
            <RunButton
              label="Calibrate"
              runningLabel="Calibrating..."
              running={isLoading}
              disabled={!calibFrames.science}
              accent="violet"
              onClick={handleCalibrate}
            />
          )}
        </div>
      )}
    </div>
  );
}

export default memo(SmartChannelMapper);
