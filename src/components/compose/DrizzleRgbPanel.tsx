import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import { Layers, Maximize2, Info, Palette, ChevronDown, ChevronRight, RefreshCw } from "lucide-react";
import { Slider, Toggle, RunButton, ResultGrid, SectionHeader } from "../ui";
import ProgressBar from "../file/ProgressBar";
import type { ProcessedFile } from "../../shared/types";

type ChannelKey = "r" | "g" | "b";

interface DrizzleRgbPanelProps {
  files?: ProcessedFile[];
  onDrizzleRgb?: (
    rPaths: string[] | null,
    gPaths: string[] | null,
    bPaths: string[] | null,
    options: Record<string, any>,
  ) => void;
  result?: any;
  isLoading?: boolean;
  progress?: number;
  progressStage?: string;
}

const ICON = (
  <div className="relative">
    <Layers size={14} className="text-indigo-400" />
    <Palette size={8} className="text-pink-400 absolute -bottom-0.5 -right-0.5" />
  </div>
);

export default function DrizzleRgbPanel({
  files = [],
  onDrizzleRgb,
  result = null,
  isLoading = false,
  progress = 0,
  progressStage = "",
}: DrizzleRgbPanelProps) {
  const [rPaths, setRPaths] = useState<string[]>([]);
  const [gPaths, setGPaths] = useState<string[]>([]);
  const [bPaths, setBPaths] = useState<string[]>([]);
  const [scale, setScale] = useState(2.0);
  const [pixfrac, setPixfrac] = useState(0.7);
  const [kernel, setKernel] = useState("square");
  const [sigmaLow, setSigmaLow] = useState(3.0);
  const [sigmaHigh, setSigmaHigh] = useState(3.0);
  const [align, setAlign] = useState(true);
  const [alignmentMethod, setAlignmentMethod] = useState("fft");
  const [wbMode, setWbMode] = useState("auto");
  const [scnrEnabled, setScnrEnabled] = useState(false);
  const [scnrMethod, setScnrMethod] = useState("average");
  const [scnrAmount, setScnrAmount] = useState(0.5);
  const [saveFits, setSaveFits] = useState(false);
  const [expandedChannels, setExpandedChannels] = useState({ r: true, g: false, b: false });
  const [elapsed, setElapsed] = useState("0");
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (isLoading) {
      setElapsed("0");
      const start = Date.now();
      timerRef.current = setInterval(() => {
        setElapsed(((Date.now() - start) / 1000).toFixed(1));
      }, 100);
    } else {
      if (timerRef.current) clearInterval(timerRef.current);
    }
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [isLoading]);

  const toggleFile = useCallback((channel: ChannelKey, path: string) => {
    const setter = channel === "r" ? setRPaths : channel === "g" ? setGPaths : setBPaths;
    setter((prev) => prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path]);
  }, []);

  const selectAllForChannel = useCallback(
    (channel: ChannelKey) => {
      const allPaths = files.map((f) => f.path);
      const setter = channel === "r" ? setRPaths : channel === "g" ? setGPaths : setBPaths;
      setter(allPaths);
    },
    [files]
  );

  const clearChannel = useCallback((channel: ChannelKey) => {
    const setter = channel === "r" ? setRPaths : channel === "g" ? setGPaths : setBPaths;
    setter([]);
  }, []);

  const toggleExpand = useCallback((channel: ChannelKey) => {
    setExpandedChannels((prev) => ({ ...prev, [channel]: !prev[channel] }));
  }, []);

  const handleAutoAssign = useCallback(() => {
    const patterns: Record<string, RegExp[]> = {
      r: [/[_-]r[._-]/i, /ha|h.?alpha|red|f656|f658/i, /[_-]R\./],
      g: [/[_-]g[._-]/i, /oiii|o3|green|f502|f501/i, /[_-]G\./],
      b: [/[_-]b[._-]/i, /sii|s2|blue|f673/i, /[_-]B\./],
    };

    const rMatches: string[] = [];
    const gMatches: string[] = [];
    const bMatches: string[] = [];

    for (const f of files) {
      const name = f.name || f.path || "";
      if (patterns.r.some((p) => p.test(name))) rMatches.push(f.path);
      else if (patterns.g.some((p) => p.test(name))) gMatches.push(f.path);
      else if (patterns.b.some((p) => p.test(name))) bMatches.push(f.path);
    }

    if (rMatches.length > 0) setRPaths(rMatches);
    if (gMatches.length > 0) setGPaths(gMatches);
    if (bMatches.length > 0) setBPaths(bMatches);

    if (rMatches.length === 0 && gMatches.length === 0 && bMatches.length === 0) {
      const third = Math.ceil(files.length / 3);
      setRPaths(files.slice(0, third).map((f) => f.path));
      setGPaths(files.slice(third, third * 2).map((f) => f.path));
      setBPaths(files.slice(third * 2).map((f) => f.path));
    }
  }, [files]);

  const canDrizzle = useMemo(() => {
    const channelsWithFrames = [rPaths.length >= 2, gPaths.length >= 2, bPaths.length >= 2].filter(Boolean).length;
    return channelsWithFrames >= 2;
  }, [rPaths, gPaths, bPaths]);

  const totalFrames = useMemo(() => rPaths.length + gPaths.length + bPaths.length, [rPaths, gPaths, bPaths]);

  const estimatedOutputRes = useMemo(() => {
    if (result) return `${result.output_dims[0]}×${result.output_dims[1]}`;
    const firstFile = files[0];
    if (firstFile?.result?.dimensions) {
      return `~${Math.ceil(firstFile.result.dimensions[0] * scale)}×${Math.ceil(firstFile.result.dimensions[1] * scale)}`;
    }
    return null;
  }, [result, files, scale]);

  const handleDrizzle = useCallback(() => {
    if (!canDrizzle || !onDrizzleRgb) return;
    onDrizzleRgb(
      rPaths.length >= 2 ? rPaths : null,
      gPaths.length >= 2 ? gPaths : null,
      bPaths.length >= 2 ? bPaths : null,
      { scale, pixfrac, kernel, sigmaLow, sigmaHigh, align, alignmentMethod, wbMode, scnrEnabled, scnrMethod, scnrAmount, saveFits }
    );
  }, [canDrizzle, onDrizzleRgb, rPaths, gPaths, bPaths, scale, pixfrac, kernel, sigmaLow, sigmaHigh, align, alignmentMethod, wbMode, scnrEnabled, scnrMethod, scnrAmount, saveFits]);

  const ChannelAccordion = ({ label, color, channel, paths, expanded }: {
    label: string;
    color: string;
    channel: ChannelKey;
    paths: string[];
    expanded: boolean;
  }) => (
    <div className="border border-zinc-800/50 rounded overflow-hidden">
      <button onClick={() => toggleExpand(channel)} className="w-full flex items-center justify-between px-2 py-1.5 bg-zinc-900/50 hover:bg-zinc-900 transition-colors">
        <div className="flex items-center gap-2">
          <div className="w-3 h-3 rounded-full border-2" style={{ backgroundColor: color + "33", borderColor: color }} />
          <span className="text-[11px] font-medium text-zinc-300">{label}</span>
          {paths.length > 0 && (
            <span className="text-[10px] text-zinc-500 bg-zinc-800 px-1.5 py-0.5 rounded">{paths.length} frames</span>
          )}
        </div>
        {expanded ? <ChevronDown size={12} className="text-zinc-500" /> : <ChevronRight size={12} className="text-zinc-500" />}
      </button>
      {expanded && (
        <div className="px-2 py-1.5 bg-zinc-950/50 space-y-1">
          <div className="flex items-center justify-between">
            <span className="text-[9px] text-zinc-600">Select frames for {label} channel</span>
            <div className="flex gap-2">
              <button onClick={() => selectAllForChannel(channel)} className="text-[9px] text-zinc-500 hover:text-zinc-300">All</button>
              <button onClick={() => clearChannel(channel)} className="text-[9px] text-zinc-500 hover:text-zinc-300">Clear</button>
            </div>
          </div>
          <div className="max-h-24 overflow-y-auto space-y-0.5 custom-scrollbar">
            {files.map((f) => (
              <label key={f.path || f.id} className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer hover:text-zinc-300 py-0.5">
                <input type="checkbox" checked={paths.includes(f.path)} onChange={() => toggleFile(channel, f.path)} className="w-3 h-3" style={{ accentColor: color }} />
                <span className="truncate">{f.name || f.path}</span>
              </label>
            ))}
            {files.length === 0 && (
              <div className="text-[10px] text-zinc-600 py-2 text-center">No FITS files loaded</div>
            )}
          </div>
        </div>
      )}
    </div>
  );

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <div className="flex items-center justify-between">
        <SectionHeader icon={ICON} title="Drizzle RGB" subtitle={totalFrames > 0 ? `${totalFrames} total` : undefined} />
        {files.length >= 2 && (
          <button onClick={handleAutoAssign} className="text-[10px] text-zinc-500 hover:text-zinc-300 flex items-center gap-1" title="Auto-assign channels by filter detection">
            <RefreshCw size={10} />
            Auto
          </button>
        )}
      </div>

      <div className="space-y-1">
        <ChannelAccordion label="Red" color="#ef4444" channel="r" paths={rPaths} expanded={expandedChannels.r} />
        <ChannelAccordion label="Green" color="#22c55e" channel="g" paths={gPaths} expanded={expandedChannels.g} />
        <ChannelAccordion label="Blue" color="#3b82f6" channel="b" paths={bPaths} expanded={expandedChannels.b} />
      </div>

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Scale</label>
          <select value={scale} onChange={(e) => setScale(parseFloat(e.target.value))} className="ab-select">
            <option value={1.5}>1.5× (Subtle)</option>
            <option value={2}>2.0× (Standard)</option>
            <option value={3}>3.0× (Aggressive)</option>
          </select>
        </div>

        <Slider label="Pixfrac" value={pixfrac} min={0.1} max={1.0} step={0.05} accent="indigo" format={(v) => v.toFixed(2)} onChange={setPixfrac} />

        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Kernel</label>
          <select value={kernel} onChange={(e) => setKernel(e.target.value)} className="ab-select">
            <option value="square">Square (Variable Pixel)</option>
            <option value="gaussian">Gaussian</option>
            <option value="lanczos3">Lanczos-3</option>
          </select>
        </div>

        <div className="flex items-center gap-2">
          <label className="text-[10px] text-zinc-500 w-14">Sigma</label>
          <div className="flex-1 flex items-center gap-1">
            <input type="number" min="1" max="10" step="0.5" value={sigmaLow} onChange={(e) => setSigmaLow(parseFloat(e.target.value))} className="w-12 bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-[10px] text-zinc-300 outline-none text-center" />
            <span className="text-[9px] text-zinc-600">low</span>
            <input type="number" min="1" max="10" step="0.5" value={sigmaHigh} onChange={(e) => setSigmaHigh(parseFloat(e.target.value))} className="w-12 bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-[10px] text-zinc-300 outline-none text-center" />
            <span className="text-[9px] text-zinc-600">high</span>
          </div>
        </div>

        <Toggle label="Sub-pixel alignment" checked={align} accent="indigo" onChange={setAlign} />

        {align && (
          <div className="flex items-center justify-between">
            <label className="text-xs text-zinc-400">Method</label>
            <select value={alignmentMethod} onChange={(e) => setAlignmentMethod(e.target.value)} className="ab-select">
              <option value="fft">Phase Correlation (FFT)</option>
              <option value="zncc">ZNCC (Spatial)</option>
            </select>
          </div>
        )}
      </div>

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">White Balance</label>
          <select value={wbMode} onChange={(e) => setWbMode(e.target.value)} className="ab-select">
            <option value="auto">Auto (Median)</option>
            <option value="none">None</option>
          </select>
        </div>

        <Toggle label="SCNR (Green Removal)" checked={scnrEnabled} accent="indigo" onChange={setScnrEnabled} />

        {scnrEnabled && (
          <div className="pl-4 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <label className="text-xs text-zinc-400">Method</label>
              <select value={scnrMethod} onChange={(e) => setScnrMethod(e.target.value)} className="ab-select">
                <option value="average">Average Neutral</option>
                <option value="maximum">Maximum Neutral</option>
              </select>
            </div>
            <Slider label="Amount" value={scnrAmount} min={0} max={1} step={0.1} accent="indigo" format={(v) => `${(v * 100).toFixed(0)}%`} onChange={setScnrAmount} />
          </div>
        )}

        <Toggle label="Save FITS output" checked={saveFits} accent="indigo" onChange={setSaveFits} />
      </div>

      {estimatedOutputRes && (
        <div className="flex items-center gap-1.5 text-[10px] text-zinc-500">
          <Maximize2 size={9} />
          Output: {estimatedOutputRes}
        </div>
      )}

      {isLoading ? (
        <div className="space-y-1.5">
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-indigo-300">{progressStage || `Processing ${totalFrames} frames…`}</span>
            <span className="text-[10px] text-zinc-500 font-mono">{elapsed}s</span>
          </div>
          <ProgressBar value={progress} variant="blue" indeterminate={progress <= 0} />
        </div>
      ) : (
        <RunButton label={`Drizzle RGB (${scale}×)`} runningLabel="Processing..." running={false} disabled={!canDrizzle} accent="indigo" onClick={handleDrizzle} />
      )}

      {!canDrizzle && !isLoading && totalFrames > 0 && (
        <div className="flex items-center gap-1.5 text-[10px] text-amber-400/70">
          <Info size={9} />
          Requires at least 2 channels with 2+ frames each
        </div>
      )}

      {result && !isLoading && (
        <div className="flex flex-col gap-3 animate-fade-in border-t border-zinc-800/50 pt-3">
          {result.previewUrl && (
            <img src={result.previewUrl} alt="Drizzle RGB result" className="w-full rounded border border-zinc-700" />
          )}
          <ResultGrid columns={3} items={[
            { label: "R frames", value: result.frame_count_r || 0 },
            { label: "G frames", value: result.frame_count_g || 0 },
            { label: "B frames", value: result.frame_count_b || 0 },
          ]} />
          <ResultGrid columns={2} items={[
            { label: "Input", value: result.input_dims ? `${result.input_dims[0]}×${result.input_dims[1]}` : "--" },
            { label: "Output", value: result.output_dims ? `${result.output_dims[0]}×${result.output_dims[1]}` : "--" },
          ]} />
          <div className="text-[10px] text-zinc-500">
            {result.elapsed_ms} ms · {result.scale || scale}× scale · {result.rejected_pixels?.toLocaleString() || 0} rejected
          </div>
        </div>
      )}
    </div>
  );
}
