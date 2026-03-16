import { useState, useCallback, useRef, useEffect } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Slider, Toggle, RunButton, ErrorAlert, SectionHeader } from "../ui";
import { runCalibrationPipeline } from "../../services/stacking.service";

interface FileGroup {
  label: string;
  paths: string[];
}

interface ChannelFilesInput {
  label: string;
  paths: string[];
}

interface ChannelPreview {
  label: string;
  pixels_b64: string;
  width: number;
  height: number;
}

interface PipelineResponse {
  stats: {
    darks_combined: number;
    flats_combined: number;
    bias_combined: number;
    channels: { label: string; lights_input: number; mean: number; stddev: number }[];
  };
  channel_previews: ChannelPreview[];
  rgb_preview: string | null;
}

const CHANNEL_LABELS = ["R", "G", "B"];
const CHANNEL_COLORS: Record<string, string> = { R: "#ef4444", G: "#22c55e", B: "#3b82f6" };

const ICON = (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-blue-400">
    <path d="M4 4h16v16H4z" />
    <path d="M4 12h16M12 4v16" opacity="0.3" />
  </svg>
);

interface PipelinePanelProps {
  files?: any[];
  onPreviewUpdate?: (url: string | null | undefined) => void;
  calibration?: any;
  stackConfig?: any;
}

export default function PipelinePanel(_props: PipelinePanelProps) {
  const [result, setResult] = useState<PipelineResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState("");

  const [channels, setChannels] = useState<FileGroup[]>(
    CHANNEL_LABELS.map((l) => ({ label: l, paths: [] }))
  );
  const [darks, setDarks] = useState<string[]>([]);
  const [flats, setFlats] = useState<string[]>([]);
  const [bias, setBias] = useState<string[]>([]);
  const [sigmaLow, setSigmaLow] = useState(2.5);
  const [sigmaHigh, setSigmaHigh] = useState(3.0);
  const [normalize, setNormalize] = useState(true);
  const [activePreview, setActivePreview] = useState<string | null>(null);

  const rgbCanvasRef = useRef<HTMLCanvasElement>(null);

  const pickFiles = useCallback(async (title: string): Promise<string[]> => {
    const selected = await open({
      multiple: true,
      filters: [{ name: "FITS", extensions: ["fits", "fit", "fts"] }],
      title,
    });
    if (!selected) return [];
    if (Array.isArray(selected)) return selected.map(f => typeof f === 'string' ? f : f.path);
    return [selected];
  }, []);

  const addToChannel = useCallback(
    async (index: number) => {
      const paths = await pickFiles(`Select ${CHANNEL_LABELS[index]} lights`);
      if (paths.length === 0) return;
      setChannels((prev) => {
        const next = [...prev];
        next[index] = { ...next[index], paths: [...next[index].paths, ...paths] };
        return next;
      });
    },
    [pickFiles]
  );

  const addCalibration = useCallback(
    async (type: "dark" | "flat" | "bias") => {
      const paths = await pickFiles(`Select ${type} frames`);
      if (paths.length === 0) return;
      switch (type) {
        case "dark": setDarks((p) => [...p, ...paths]); break;
        case "flat": setFlats((p) => [...p, ...paths]); break;
        case "bias": setBias((p) => [...p, ...paths]); break;
      }
    },
    [pickFiles]
  );

  const handleRun = async () => {
    const channelInputs: ChannelFilesInput[] = channels
      .filter((c) => c.paths.length > 0)
      .map((c) => ({ label: c.label, paths: c.paths }));

    if (channelInputs.length === 0) return;

    setLoading(true);
    setError(null);
    setProgress("Building calibration masters...");
    try {
      const res = await runCalibrationPipeline({
        channels: channelInputs,
        dark_paths: darks,
        flat_paths: flats,
        bias_paths: bias,
        sigma_low: sigmaLow,
        sigma_high: sigmaHigh,
        normalize,
      }) as PipelineResponse;
      setResult(res);
      setProgress("");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setProgress("");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (!result?.rgb_preview || !rgbCanvasRef.current) return;
    const firstCh = result.channel_previews[0];
    if (!firstCh) return;

    const { width: w, height: h } = firstCh;
    const canvas = rgbCanvasRef.current;
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const raw = atob(result.rgb_preview);
    const imgData = ctx.createImageData(w, h);
    for (let i = 0; i < w * h; i++) {
      imgData.data[i * 4] = raw.charCodeAt(i * 3);
      imgData.data[i * 4 + 1] = raw.charCodeAt(i * 3 + 1);
      imgData.data[i * 4 + 2] = raw.charCodeAt(i * 3 + 2);
      imgData.data[i * 4 + 3] = 255;
    }
    ctx.putImageData(imgData, 0, 0);
  }, [result]);

  const totalLights = channels.reduce((s, c) => s + c.paths.length, 0);

  const CalibRow = ({ label, count, onAdd, onClear }: { label: string; count: number; onAdd: () => void; onClear: () => void }) => (
    <div className="flex items-center justify-between">
      <span className="text-xs text-zinc-400">{label}: {count}</span>
      <div className="flex gap-1">
        <button onClick={onAdd} className="text-[10px] text-zinc-500 hover:text-zinc-300 bg-zinc-800 px-2 py-0.5 rounded">+ Add</button>
        {count > 0 && <button onClick={onClear} className="text-[10px] text-red-400 hover:text-red-300 bg-zinc-800 px-2 py-0.5 rounded">Clear</button>}
      </div>
    </div>
  );

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Calibration Pipeline" />

      <div className="flex flex-col gap-2">
        {channels.map((ch, i) => (
          <div key={ch.label} className="flex items-center justify-between rounded border border-zinc-800/50 px-2 py-1.5">
            <div className="flex items-center gap-2">
              <span className="inline-block h-3 w-3 rounded-full" style={{ backgroundColor: CHANNEL_COLORS[ch.label] }} />
              <span className="text-xs text-zinc-300">{ch.label}</span>
              <span className="text-[10px] text-zinc-500">{ch.paths.length} files</span>
            </div>
            <div className="flex gap-1">
              <button onClick={() => addToChannel(i)} className="text-[10px] text-zinc-500 hover:text-zinc-300 bg-zinc-800 px-2 py-0.5 rounded">+ Add</button>
              {ch.paths.length > 0 && (
                <button
                  onClick={() => setChannels((prev) => { const next = [...prev]; next[i] = { ...next[i], paths: [] }; return next; })}
                  className="text-[10px] text-red-400 hover:text-red-300 bg-zinc-800 px-2 py-0.5 rounded"
                >Clear</button>
              )}
            </div>
          </div>
        ))}
      </div>

      <div className="flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
        <span className="text-xs text-zinc-500 uppercase tracking-wider">Calibration (optional)</span>
        <CalibRow label="Darks" count={darks.length} onAdd={() => addCalibration("dark")} onClear={() => setDarks([])} />
        <CalibRow label="Flats" count={flats.length} onAdd={() => addCalibration("flat")} onClear={() => setFlats([])} />
        <CalibRow label="Bias" count={bias.length} onAdd={() => addCalibration("bias")} onClear={() => setBias([])} />
      </div>

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <span className="text-xs text-zinc-500 uppercase tracking-wider">Stacking</span>
        <Slider label="Sigma Low" value={sigmaLow} min={1} max={5} step={0.1} accent="sky" format={(v) => v.toFixed(1)} onChange={setSigmaLow} />
        <Slider label="Sigma High" value={sigmaHigh} min={1} max={5} step={0.1} accent="sky" format={(v) => v.toFixed(1)} onChange={setSigmaHigh} />
        <Toggle label="Normalize before stack" checked={normalize} accent="sky" onChange={setNormalize} />
      </div>

      <RunButton
        label={`Run Pipeline (${totalLights} lights)`}
        runningLabel={progress || "Processing..."}
        running={loading}
        disabled={totalLights === 0}
        accent="sky"
        onClick={handleRun}
      />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in border-t border-zinc-800/50 pt-3">
          <span className="text-xs font-semibold text-zinc-400">Results</span>

          <div className="flex gap-1">
            {result.channel_previews.map((ch) => (
              <button
                key={ch.label}
                onClick={() => setActivePreview(ch.label)}
                className={`ab-pill ${activePreview === ch.label ? "data-active" : ""}`}
                data-active={activePreview === ch.label || undefined}
              >
                {ch.label}
              </button>
            ))}
            {result.rgb_preview && (
              <button
                onClick={() => setActivePreview("RGB")}
                className={`ab-pill ${activePreview === "RGB" ? "data-active" : ""}`}
                data-active={activePreview === "RGB" || undefined}
              >
                RGB
              </button>
            )}
          </div>

          {activePreview === "RGB" && result.rgb_preview && (
            <canvas ref={rgbCanvasRef} className="w-full rounded border border-zinc-700" style={{ imageRendering: "auto" }} />
          )}

          <div className="flex flex-col gap-1 text-[10px] text-zinc-500">
            {result.stats.channels.map((ch) => (
              <div key={ch.label}>
                {ch.label}: {ch.lights_input} lights, mean={ch.mean.toFixed(1)} std={ch.stddev.toFixed(1)}
              </div>
            ))}
            {result.stats.darks_combined > 0 && <div>Master dark: {result.stats.darks_combined} frames</div>}
            {result.stats.flats_combined > 0 && <div>Master flat: {result.stats.flats_combined} frames</div>}
          </div>
        </div>
      )}
    </div>
  );
}
