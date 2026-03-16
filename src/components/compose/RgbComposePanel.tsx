import { useState, useCallback } from "react";
import { Palette, RefreshCw, Sparkles, Sun } from "lucide-react";
import { Slider, Toggle, RunButton, ResultGrid, SectionHeader } from "../ui";
import type { ProcessedFile } from "../../shared/types";
import type { PaletteSuggestion } from "../../context/PreviewContext";

interface RgbComposePanelProps {
  files?: ProcessedFile[];
  onCompose?: (
    lPath: string | null,
    rPath: string | null,
    gPath: string | null,
    bPath: string | null,
    options: Record<string, any>,
  ) => void;
  result?: any;
  isLoading?: boolean;
  narrowbandPalette?: PaletteSuggestion | null;
}

function PaletteBadge({ paletteName }: { paletteName: string }) {
  return (
    <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium bg-amber-500/15 text-amber-400 border border-amber-500/20">
      <Sparkles size={10} />
      {paletteName} Palette (auto-detected)
    </span>
  );
}

const ICON = <Palette size={14} className="text-pink-400" />;

export default function RgbComposePanel({
  files = [],
  onCompose,
  result = null,
  isLoading = false,
  narrowbandPalette,
}: RgbComposePanelProps) {
  const [lFile, setLFile] = useState("");
  const [rFile, setRFile] = useState("");
  const [gFile, setGFile] = useState("");
  const [bFile, setBFile] = useState("");
  const [autoStretch, setAutoStretch] = useState(true);
  const [linkedStf, setLinkedStf] = useState(false);
  const [align, setAlign] = useState(true);
  const [wbMode, setWbMode] = useState("auto");
  const [scnrEnabled, setScnrEnabled] = useState(false);
  const [scnrMethod, setScnrMethod] = useState("average");
  const [scnrAmount, setScnrAmount] = useState(0.5);
  const [lrgbLightness, setLrgbLightness] = useState(1.0);
  const [lrgbChrominance, setLrgbChrominance] = useState(1.0);

  const assignedCount = [rFile, gFile, bFile].filter(Boolean).length;
  const hasL = lFile !== "";
  const canCompose = assignedCount >= 2;

  const handleCompose = useCallback(() => {
    if (!canCompose || !onCompose) return;
    onCompose(lFile || null, rFile || null, gFile || null, bFile || null, {
      autoStretch,
      linkedStf,
      align,
      wbMode,
      scnrEnabled,
      scnrMethod,
      scnrAmount,
      lrgbLightness: hasL ? lrgbLightness : undefined,
      lrgbChrominance: hasL ? lrgbChrominance : undefined,
    });
  }, [lFile, rFile, gFile, bFile, autoStretch, linkedStf, align, wbMode, scnrEnabled, scnrMethod, scnrAmount, lrgbLightness, lrgbChrominance, hasL, onCompose, canCompose]);

  const handleAutoAssign = useCallback(() => {
    if (
      narrowbandPalette &&
      narrowbandPalette.is_complete &&
      narrowbandPalette.r_file &&
      narrowbandPalette.g_file &&
      narrowbandPalette.b_file
    ) {
      setRFile(narrowbandPalette.r_file.file_path);
      setGFile(narrowbandPalette.g_file.file_path);
      setBFile(narrowbandPalette.b_file.file_path);
      return;
    }

    const remaining = [...files];
    const patterns: Record<string, RegExp[]> = {
      l: [/[_-]l[._-]/i, /luminance|lum|clear/i, /[_-]L\./],
      r: [/[_-]r[._-]/i, /ha|h.?alpha|red/i, /[_-]R\./],
      g: [/[_-]g[._-]/i, /oiii|o3|green/i, /[_-]G\./],
      b: [/[_-]b[._-]/i, /sii|s2|blue/i, /[_-]B\./],
    };

    let lMatch = "", rMatch = "", gMatch = "", bMatch = "";
    for (const f of remaining) {
      const name = f.name || f.path || "";
      if (!lMatch && patterns.l.some((p) => p.test(name))) lMatch = f.path;
      else if (!rMatch && patterns.r.some((p) => p.test(name))) rMatch = f.path;
      else if (!gMatch && patterns.g.some((p) => p.test(name))) gMatch = f.path;
      else if (!bMatch && patterns.b.some((p) => p.test(name))) bMatch = f.path;
    }

    if (!rMatch && !gMatch && !bMatch && remaining.length >= 2) {
      rMatch = remaining[0]?.path || "";
      gMatch = remaining[1]?.path || "";
      bMatch = remaining[2]?.path || "";
    }

    if (lMatch) setLFile(lMatch);
    if (rMatch) setRFile(rMatch);
    if (gMatch) setGFile(gMatch);
    if (bMatch) setBFile(bMatch);
  }, [files, narrowbandPalette]);

  const ChannelSelect = ({ label, color, value, onChange, icon }: {
    label: string;
    color: string;
    value: string;
    onChange: (v: string) => void;
    icon?: React.ReactNode;
  }) => (
    <div className="flex items-center gap-2">
      {icon || (
        <div className="w-3 h-3 rounded-full border" style={{ backgroundColor: color + "33", borderColor: color }} />
      )}
      <span className="text-[10px] text-zinc-400 w-3 font-bold">{label}</span>
      <select value={value} onChange={(e) => onChange(e.target.value)} className="flex-1 ab-select">
        <option value="">— none —</option>
        {files.map((f) => (
          <option key={f.path || f.id} value={f.path}>{f.name || f.path}</option>
        ))}
      </select>
    </div>
  );

  const composeLabel = hasL
    ? `Compose LRGB (${assignedCount}/3 + L)`
    : `Compose RGB (${assignedCount}/3 channels)`;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <SectionHeader icon={ICON} title="RGB Compose" />
          {narrowbandPalette?.is_complete && narrowbandPalette.palette_name && (
            <PaletteBadge paletteName={narrowbandPalette.palette_name} />
          )}
        </div>
        {files.length >= 2 && (
          <button onClick={handleAutoAssign} className="text-[10px] text-zinc-500 hover:text-zinc-300 flex items-center gap-1" title="Auto-assign channels by filename">
            <RefreshCw size={10} />
            Auto
          </button>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <ChannelSelect label="L" color="#f5f5f5" value={lFile} onChange={setLFile} icon={<Sun size={12} className="text-zinc-400" />} />

        {hasL && (
          <div className="pl-5 pb-1 space-y-2 border-l border-zinc-800 ml-1.5">
            <Slider label="Lightness" value={lrgbLightness} min={0} max={1} step={0.05} accent="violet" format={(v) => `${(v * 100).toFixed(0)}%`} onChange={setLrgbLightness} />
            <Slider label="Chrominance" value={lrgbChrominance} min={0} max={1} step={0.05} accent="violet" format={(v) => `${(v * 100).toFixed(0)}%`} onChange={setLrgbChrominance} />
          </div>
        )}

        <ChannelSelect label="R" color="#ef4444" value={rFile} onChange={setRFile} />
        <ChannelSelect label="G" color="#22c55e" value={gFile} onChange={setGFile} />
        <ChannelSelect label="B" color="#3b82f6" value={bFile} onChange={setBFile} />
      </div>

      <div className="flex flex-col gap-1.5 border-t border-zinc-800/50 pt-3">
        <Toggle label="Auto STF" checked={autoStretch} accent="violet" onChange={setAutoStretch} />
        <Toggle label="Linked STF" checked={linkedStf} accent="violet" onChange={setLinkedStf} />
        <Toggle label="Align" checked={align} accent="violet" onChange={setAlign} />

        <div className="flex items-center justify-between pt-1">
          <label className="text-xs text-zinc-400">White Balance</label>
          <select value={wbMode} onChange={(e) => setWbMode(e.target.value)} className="ab-select">
            <option value="auto">Auto (Median)</option>
            <option value="none">None</option>
            <option value="manual">Manual</option>
          </select>
        </div>

        <Toggle label="SCNR (Green Removal)" checked={scnrEnabled} accent="violet" onChange={setScnrEnabled} />

        {scnrEnabled && (
          <div className="pl-4 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <label className="text-xs text-zinc-400">Method</label>
              <select value={scnrMethod} onChange={(e) => setScnrMethod(e.target.value)} className="ab-select">
                <option value="average">Average Neutral</option>
                <option value="maximum">Maximum Neutral</option>
              </select>
            </div>
            <Slider label="Amount" value={scnrAmount} min={0} max={1} step={0.1} accent="violet" format={(v) => `${(v * 100).toFixed(0)}%`} onChange={setScnrAmount} />
          </div>
        )}
      </div>

      <RunButton label={composeLabel} runningLabel="Composing..." running={isLoading} disabled={!canCompose} accent="violet" onClick={handleCompose} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          {result.previewUrl && (
            <img src={result.previewUrl} alt="RGB composite" className="w-full rounded border border-zinc-700" />
          )}
          <ResultGrid columns={3} items={[
            { label: "R median", value: result.stats_r?.median?.toFixed(0) },
            { label: "G median", value: result.stats_g?.median?.toFixed(0) },
            { label: "B median", value: result.stats_b?.median?.toFixed(0) },
          ]} />
          {(result.offset_g || result.offset_b) && (
            <div className="text-[10px] text-zinc-500">
              Offsets — G: [{result.offset_g?.[0]}, {result.offset_g?.[1]}] B: [{result.offset_b?.[0]}, {result.offset_b?.[1]}]
            </div>
          )}
          {result.resampled && (
            <div className="text-[10px] text-amber-400/80">⚡ Auto-resampled (mixed SW/LW resolution)</div>
          )}
          {result.lrgb_applied && (
            <div className="text-[10px] text-zinc-400">
              ☀ LRGB applied (L: {(lrgbLightness * 100).toFixed(0)}%, C: {(lrgbChrominance * 100).toFixed(0)}%)
            </div>
          )}
          <div className="text-[10px] text-zinc-500">{result.elapsed_ms} ms</div>
        </div>
      )}
    </div>
  );
}
