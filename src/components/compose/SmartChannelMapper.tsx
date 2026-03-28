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
} from "lucide-react";
import { RunButton, SectionHeader } from "../ui";

export interface ChannelFile {
  id: string;
  path: string;
  name: string;
  filter?: string;
  instrument?: string;
  exptime?: number;
  previewUrl?: string;
}

export type ChannelSlot = "L" | "R" | "G" | "B";
export type CalibSlot = "science" | "bias" | "dark" | "flat";
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
}

interface SmartChannelMapperProps {
  mode: MapperMode;
  files: ChannelFile[];
  onComposeRgb?: (
    assignments: ChannelAssignment,
    options: Record<string, any>,
  ) => void;
  onCalibrate?: (assignments: CalibAssignment) => void;
  isLoading?: boolean;
  composeOptions?: Record<string, any>;
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
}

const CHANNEL_META: Record<ChannelSlot, { color: string; label: string; bg: string; border: string }> = {
  L: { color: "#e4e4e7", label: "Luminance", bg: "rgba(228,228,231,0.06)", border: "rgba(228,228,231,0.2)" },
  R: { color: "#ef4444", label: "Red", bg: "rgba(239,68,68,0.06)", border: "rgba(239,68,68,0.25)" },
  G: { color: "#22c55e", label: "Green", bg: "rgba(34,197,94,0.06)", border: "rgba(34,197,94,0.25)" },
  B: { color: "#3b82f6", label: "Blue", bg: "rgba(59,130,246,0.06)", border: "rgba(59,130,246,0.25)" },
};

const CALIB_META: Record<CalibSlot, { color: string; label: string; bg: string; border: string; multi: boolean }> = {
  science: { color: "#f59e0b", label: "Science", bg: "rgba(245,158,11,0.06)", border: "rgba(245,158,11,0.25)", multi: false },
  bias: { color: "#a78bfa", label: "Bias", bg: "rgba(167,139,250,0.06)", border: "rgba(167,139,250,0.2)", multi: true },
  dark: { color: "#60a5fa", label: "Dark", bg: "rgba(96,165,250,0.06)", border: "rgba(96,165,250,0.2)", multi: true },
  flat: { color: "#fbbf24", label: "Flat", bg: "rgba(251,191,36,0.06)", border: "rgba(251,191,36,0.2)", multi: true },
};

const JWST_FILTER_WAVELENGTH: Record<string, number> = {
  F070W: 700, F090W: 900, F115W: 1150, F140M: 1400, F150W: 1500,
  F162M: 1620, F164N: 1640, F150W2: 1500, F182M: 1820, F187N: 1870,
  F200W: 2000, F210M: 2100, F212N: 2120, F250M: 2500, F277W: 2770,
  F300M: 3000, F322W2: 3220, F323N: 3230, F335M: 3350, F356W: 3560,
  F360M: 3600, F405N: 4050, F410M: 4100, F430M: 4300, F444W: 4440,
  F460M: 4600, F466N: 4660, F470N: 4700, F480M: 4800,
};

const PALETTE_PRESETS = [
  { id: "SHO", label: "SHO (Hubble)", desc: "SII\u2192R  Ha\u2192G  OIII\u2192B", icon: "\u2728" },
  { id: "HOO", label: "HOO", desc: "Ha\u2192R  OIII\u2192G+B", icon: "\ud83c\udf11" },
  { id: "HOS", label: "HOS", desc: "Ha\u2192R  OIII\u2192G  SII\u2192B", icon: "\ud83c\udf0c" },
  { id: "NaturalColor", label: "Natural Color", desc: "Ha\u2192R  OIII\u2192G+B (natural)", icon: "\ud83c\udf0d" },
  { id: "Custom", label: "Custom", desc: "Manual assignment", icon: "\ud83c\udfa8" },
] as const;

function getFilterWavelength(filter?: string): number | null {
  if (!filter) return null;
  const key = filter.toUpperCase().trim();
  return JWST_FILTER_WAVELENGTH[key] ?? null;
}

function autoMapByMetadata(files: ChannelFile[]): Partial<ChannelAssignment> {
  const withWavelength = files
    .map((f) => ({ file: f, wl: getFilterWavelength(f.filter) }))
    .filter((x): x is { file: ChannelFile; wl: number } => x.wl !== null)
    .sort((a, b) => a.wl - b.wl);

  if (withWavelength.length === 0) return {};

  if (withWavelength.length >= 3) {
    const sorted = [...withWavelength].sort((a, b) => b.wl - a.wl);
    return {
      R: sorted[0].file,
      G: sorted[Math.floor(sorted.length / 2)].file,
      B: sorted[sorted.length - 1].file,
    };
  }

  if (withWavelength.length === 2) {
    return {
      R: withWavelength[1].file,
      B: withWavelength[0].file,
    };
  }

  return {};
}

function autoMapByFilename(files: ChannelFile[]): Partial<ChannelAssignment> {
  const result: Partial<ChannelAssignment> = {};
  const patterns: Record<ChannelSlot, RegExp[]> = {
    L: [/[_-]l[._-]/i, /luminance|lum|clear/i],
    R: [/[_-]r[._-]/i, /ha|h.?alpha|red/i, /f444w|f410m|f356w/i],
    G: [/[_-]g[._-]/i, /oiii|o3|green/i, /f200w|f277w/i],
    B: [/[_-]b[._-]/i, /sii|s2|blue/i, /f115w|f090w|f150w/i],
  };

  for (const slot of (["L", "R", "G", "B"] as ChannelSlot[])) {
    for (const f of files) {
      if (Object.values(result).some((v) => v?.id === f.id)) continue;
      const name = f.name || f.path || "";
      if (patterns[slot].some((p) => p.test(name))) {
        result[slot] = f;
        break;
      }
    }
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
  onSelectFile: (slot: string) => void;
  allFiles: ChannelFile[];
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
  onSelectFile,
  allFiles,
}: DropZoneSlotProps) {
  const [isDragOver, setIsDragOver] = useState(false);
  const [showDropdown, setShowDropdown] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

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
    <div className="ab-channel-slot-wrapper" ref={dropdownRef}>
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
            {file.previewUrl && (
              <img src={file.previewUrl} alt="" className="ab-channel-thumb" />
            )}
            <div className="ab-channel-file-detail">
              <span className="ab-channel-filename" title={file.name}>{file.name}</span>
              <div className="ab-channel-meta-row">
                {file.filter && <FilterChip filter={file.filter} color={color} />}
                {file.exptime != null && (
                  <span className="ab-channel-exptime">{file.exptime}s</span>
                )}
              </div>
            </div>
            <button
              className="ab-channel-clear"
              onClick={(e) => {
                e.stopPropagation();
                onClear(slot);
              }}
              title="Remove"
            >
              <X size={12} />
            </button>
          </div>
        ) : (
          <button
            className="ab-channel-browse"
            onClick={() => setShowDropdown((v) => !v)}
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
                onClick={() => {
                  onDrop(slot, f);
                  setShowDropdown(false);
                }}
              >
                <span className="truncate">{f.name}</span>
                {f.filter && <FilterChip filter={f.filter} color={color} />}
              </button>
            ))
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
}: SmartChannelMapperProps) {
  const [channels, setChannels] = useState<ChannelAssignment>({
    L: null, R: null, G: null, B: null,
  });
  const [calibFrames, setCalibFrames] = useState<CalibAssignment>({
    science: null, bias: [], dark: [], flat: [],
  });
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
  }, [mode]);

  const clearChannel = useCallback((slot: string) => {
    if (mode === "rgb") {
      setChannels((prev) => ({ ...prev, [slot]: null }));
    } else {
      const s = slot as CalibSlot;
      setCalibFrames((prev) =>
        s === "science" ? { ...prev, science: null } : { ...prev, [s]: [] },
      );
    }
  }, [mode]);

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
                onSelectFile={() => {}}
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
          {(["science", "bias", "dark", "flat"] as CalibSlot[]).map((slot) => (
            <DropZoneSlot
              key={slot}
              slot={slot}
              label={CALIB_META[slot].label}
              color={CALIB_META[slot].color}
              bg={CALIB_META[slot].bg}
              border={CALIB_META[slot].border}
              file={slot === "science" ? calibFrames.science : (calibFrames[slot] as ChannelFile[])[0] ?? null}
              onDrop={assignChannel}
              onClear={clearChannel}
              onSelectFile={() => {}}
              allFiles={files}
            />
          ))}
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
