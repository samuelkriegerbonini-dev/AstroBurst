import { useState, useCallback, useMemo } from "react";
import { Grid3X3, CheckCircle2, Wand2 } from "lucide-react";
import { Slider, Toggle, RunButton, ResultGrid, ErrorAlert, SectionHeader } from "../ui";
import { drizzleRgbStack } from "../../services/stacking";
import { getOutputDir } from "../../infrastructure/tauri";
import type { ProcessedFile } from "../../shared/types";
import type { DrizzleRgbResult } from "../../shared/types/stacking";

type Channel = "r" | "g" | "b";

interface DrizzleRgbPanelProps {
  files: ProcessedFile[];
  onResult?: (result: DrizzleRgbResult) => void;
}

const ICON = <Grid3X3 size={14} className="text-rose-400" />;

const CHANNEL_STYLES: Record<Channel, { active: string; idle: string }> = {
  r: { active: "bg-red-500/25 text-red-300 ring-1 ring-red-500/50", idle: "text-zinc-600 hover:text-red-400" },
  g: { active: "bg-green-500/25 text-green-300 ring-1 ring-green-500/50", idle: "text-zinc-600 hover:text-green-400" },
  b: { active: "bg-blue-500/25 text-blue-300 ring-1 ring-blue-500/50", idle: "text-zinc-600 hover:text-blue-400" },
};

const AUTO_PATTERNS: [Channel, RegExp][] = [
  ["r", /(?:[_.-](?:r|red)[_.-]|[_-](?:r|red)$|^(?:r|red)[_-])/i],
  ["g", /(?:[_.-](?:g|green)[_.-]|[_-](?:g|green)$|^(?:g|green)[_-])/i],
  ["b", /(?:[_.-](?:b|blue)[_.-]|[_-](?:b|blue)$|^(?:b|blue)[_-])/i],
];

function autoDetectChannel(name: string): Channel | null {
  const stem = name.replace(/\.(fits?|fts|fit)$/i, "");
  for (const [ch, re] of AUTO_PATTERNS) {
    if (re.test(stem)) return ch;
  }
  return null;
}

const KERNELS = [
  { value: "square", label: "Square" },
  { value: "gaussian", label: "Gaussian" },
  { value: "lanczos3", label: "Lanczos3" },
] as const;

export default function DrizzleRgbPanel({ files = [], onResult }: DrizzleRgbPanelProps) {
  const [assignments, setAssignments] = useState<Record<string, Channel>>({});
  const [scale, setScale] = useState(2.0);
  const [pixfrac, setPixfrac] = useState(0.7);
  const [kernel, setKernel] = useState<"square" | "gaussian" | "lanczos3">("square");
  const [align, setAlign] = useState(true);
  const [saveFits, setSaveFits] = useState(true);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<DrizzleRgbResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const channelPaths = useMemo(() => {
    const out: Record<Channel, string[]> = { r: [], g: [], b: [] };
    for (const f of files) {
      const ch = assignments[f.path];
      if (ch) out[ch].push(f.path);
    }
    return out;
  }, [files, assignments]);

  const readyChannels = (["r", "g", "b"] as Channel[]).filter((ch) => channelPaths[ch].length >= 2);
  const shortChannels = (["r", "g", "b"] as Channel[]).filter((ch) => channelPaths[ch].length === 1);
  const canRun = readyChannels.length >= 2;

  const toggleAssignment = useCallback((path: string, ch: Channel) => {
    setAssignments((prev) => {
      const next = { ...prev };
      if (next[path] === ch) {
        delete next[path];
      } else {
        next[path] = ch;
      }
      return next;
    });
  }, []);

  const autoAssign = useCallback(() => {
    setAssignments((prev) => {
      const next = { ...prev };
      for (const f of files) {
        if (next[f.path]) continue;
        const detected = autoDetectChannel(f.name);
        if (detected) next[f.path] = detected;
      }
      return next;
    });
  }, [files]);

  const clearAssignments = useCallback(() => setAssignments({}), []);

  const handleRun = useCallback(async () => {
    if (!canRun) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await drizzleRgbStack(
        channelPaths.r,
        channelPaths.g,
        channelPaths.b,
        await getOutputDir(),
        { scale, pixfrac, kernel, align, saveFits },
      );
      setResult(res);
      onResult?.(res);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsRunning(false);
    }
  }, [canRun, channelPaths, scale, pixfrac, kernel, align, saveFits, onResult]);

  const totalAssigned = channelPaths.r.length + channelPaths.g.length + channelPaths.b.length;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <div className="flex items-center justify-between">
        <SectionHeader
          icon={ICON}
          title="Drizzle RGB"
          subtitle={totalAssigned > 0 ? `${totalAssigned} frames assigned` : "Assign frames to channels"}
        />
        <div className="flex gap-2 items-center">
          <button onClick={autoAssign} className="flex items-center gap-1 text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors" title="Detect R/G/B from filenames">
            <Wand2 size={10} />
            Auto
          </button>
          <button onClick={clearAssignments} className="text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors">Clear</button>
        </div>
      </div>

      {files.length === 0 && (
        <div className="text-[11px] text-zinc-500 italic">Load FITS frames first, then assign each one to R, G or B.</div>
      )}

      <div className="flex flex-col gap-1 max-h-[200px] overflow-y-auto">
        {files.map((f) => {
          const assigned = assignments[f.path];
          return (
            <div
              key={f.id}
              className={`flex items-center gap-2 px-2.5 py-1.5 rounded text-[11px] transition-all ${
                assigned ? "bg-zinc-800/60 text-zinc-200" : "text-zinc-500 hover:bg-zinc-800/40"
              }`}
            >
              <span className="truncate flex-1">{f.name}</span>
              <div className="flex gap-1 shrink-0">
                {(["r", "g", "b"] as Channel[]).map((ch) => (
                  <button
                    key={ch}
                    onClick={() => toggleAssignment(f.path, ch)}
                    className={`w-6 h-5 rounded text-[10px] font-bold uppercase transition-all ${
                      assigned === ch ? CHANNEL_STYLES[ch].active : CHANNEL_STYLES[ch].idle
                    }`}
                  >
                    {ch}
                  </button>
                ))}
              </div>
            </div>
          );
        })}
      </div>

      <div className="grid grid-cols-3 gap-1.5">
        {(["r", "g", "b"] as Channel[]).map((ch) => (
          <div key={ch} className="ab-metric-card flex flex-col items-center p-2 rounded">
            <span className={`text-[9px] uppercase ${ch === "r" ? "text-red-400/70" : ch === "g" ? "text-green-400/70" : "text-blue-400/70"}`}>
              {ch} frames
            </span>
            <span className={`text-sm font-mono ${channelPaths[ch].length >= 2 ? "text-zinc-200" : "text-zinc-600"}`}>
              {channelPaths[ch].length}
            </span>
          </div>
        ))}
      </div>

      {shortChannels.length > 0 && (
        <div className="text-[10px] text-amber-400/90 bg-amber-900/20 border border-amber-800/30 rounded px-2.5 py-1.5">
          Channel{shortChannels.length > 1 ? "s" : ""} {shortChannels.map((c) => c.toUpperCase()).join(", ")} need at
          least 2 frames for drizzle and will be skipped.
        </div>
      )}

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <span className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">Drizzle Parameters</span>
        <Slider label="Scale" value={scale} min={1.0} max={3.0} step={0.5} accent="rose" format={(v) => `${v.toFixed(1)}x`} onChange={setScale} />
        <Slider label="Pixfrac" value={pixfrac} min={0.1} max={1.0} step={0.05} accent="rose" format={(v) => v.toFixed(2)} onChange={setPixfrac} />
        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Kernel</label>
          <select
            value={kernel}
            onChange={(e) => setKernel(e.target.value as typeof kernel)}
            className="ab-select"
            disabled={isRunning}
          >
            {KERNELS.map((k) => (
              <option key={k.value} value={k.value}>{k.label}</option>
            ))}
          </select>
        </div>
        <Toggle label="Align frames and channels" checked={align} accent="rose" onChange={setAlign} />
        <Toggle label="Save FITS alongside PNG" checked={saveFits} accent="rose" onChange={setSaveFits} />
      </div>

      <RunButton
        label={`Drizzle ${readyChannels.map((c) => c.toUpperCase()).join("+") || "RGB"}`}
        runningLabel="Drizzling..."
        running={isRunning}
        disabled={!canRun}
        accent="rose"
        onClick={handleRun}
      />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            Drizzle RGB Complete
          </div>
          <ResultGrid columns={3} items={[
            { label: "Output", value: result.output_dims ? `${result.output_dims[1]}×${result.output_dims[0]}` : "--" },
            { label: "Scale", value: result.scale ? `${result.scale.toFixed(1)}x` : "--" },
            { label: "Rejected", value: result.rejected_pixels?.toLocaleString() ?? "0" },
            { label: "R frames", value: result.frame_count_r },
            { label: "G frames", value: result.frame_count_g },
            { label: "B frames", value: result.frame_count_b },
          ]} />
          {result.fits_path && (
            <div className="text-[10px] text-zinc-500 truncate" title={result.fits_path}>
              FITS: {result.fits_path}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
