import { useState, useCallback, useRef, useEffect } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useBackend } from "../../hooks/useBackend";

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

interface PipelinePanelProps {
  files?: any[];
  onPreviewUpdate?: (url: string | null | undefined) => void;
  calibration?: any;
  stackConfig?: any;
}

export default function PipelinePanel(_props: PipelinePanelProps) {
  const { runCalibrationPipeline } = useBackend();
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
        next[index] = {
          ...next[index],
          paths: [...next[index].paths, ...paths],
        };
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
        case "dark":
          setDarks((p) => [...p, ...paths]);
          break;
        case "flat":
          setFlats((p) => [...p, ...paths]);
          break;
        case "bias":
          setBias((p) => [...p, ...paths]);
          break;
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

  return (
    <div className="flex flex-col gap-3 p-3 text-xs">
      <div className="font-semibold uppercase tracking-wide text-neutral-400">
        Calibration Pipeline
      </div>

      <div className="flex flex-col gap-2">
        {channels.map((ch, i) => (
          <div
            key={ch.label}
            className="flex items-center justify-between rounded border border-neutral-700 px-2 py-1.5"
          >
            <div className="flex items-center gap-2">
              <span
                className="inline-block h-3 w-3 rounded-full"
                style={{
                  backgroundColor:
                    ch.label === "R"
                      ? "#ef4444"
                      : ch.label === "G"
                      ? "#22c55e"
                      : "#3b82f6",
                }}
              />
              <span>{ch.label}</span>
              <span className="text-neutral-500">{ch.paths.length} files</span>
            </div>
            <div className="flex gap-1">
              <button
                onClick={() => addToChannel(i)}
                className="rounded bg-neutral-800 px-2 py-0.5 hover:bg-neutral-700"
              >
                + Add
              </button>
              {ch.paths.length > 0 && (
                <button
                  onClick={() =>
                    setChannels((prev) => {
                      const next = [...prev];
                      next[i] = { ...next[i], paths: [] };
                      return next;
                    })
                  }
                  className="rounded bg-neutral-800 px-2 py-0.5 text-red-400 hover:bg-neutral-700"
                >
                  Clear
                </button>
              )}
            </div>
          </div>
        ))}
      </div>

      <div className="border-t border-neutral-800 pt-2 text-neutral-400">
        Calibration (optional)
      </div>

      <div className="flex flex-col gap-1">
        <div className="flex items-center justify-between">
          <span>Darks: {darks.length}</span>
          <div className="flex gap-1">
            <button
              onClick={() => addCalibration("dark")}
              className="rounded bg-neutral-800 px-2 py-0.5 hover:bg-neutral-700"
            >
              + Add
            </button>
            {darks.length > 0 && (
              <button
                onClick={() => setDarks([])}
                className="rounded bg-neutral-800 px-2 py-0.5 text-red-400 hover:bg-neutral-700"
              >
                Clear
              </button>
            )}
          </div>
        </div>
        <div className="flex items-center justify-between">
          <span>Flats: {flats.length}</span>
          <div className="flex gap-1">
            <button
              onClick={() => addCalibration("flat")}
              className="rounded bg-neutral-800 px-2 py-0.5 hover:bg-neutral-700"
            >
              + Add
            </button>
            {flats.length > 0 && (
              <button
                onClick={() => setFlats([])}
                className="rounded bg-neutral-800 px-2 py-0.5 text-red-400 hover:bg-neutral-700"
              >
                Clear
              </button>
            )}
          </div>
        </div>
        <div className="flex items-center justify-between">
          <span>Bias: {bias.length}</span>
          <div className="flex gap-1">
            <button
              onClick={() => addCalibration("bias")}
              className="rounded bg-neutral-800 px-2 py-0.5 hover:bg-neutral-700"
            >
              + Add
            </button>
            {bias.length > 0 && (
              <button
                onClick={() => setBias([])}
                className="rounded bg-neutral-800 px-2 py-0.5 text-red-400 hover:bg-neutral-700"
              >
                Clear
              </button>
            )}
          </div>
        </div>
      </div>

      <div className="border-t border-neutral-800 pt-2 text-neutral-400">
        Stacking
      </div>

      <label className="flex items-center justify-between">
        <span>Sigma low</span>
        <input
          type="number"
          min={1}
          max={5}
          step={0.1}
          value={sigmaLow}
          onChange={(e) => setSigmaLow(Number(e.target.value))}
          className="w-16 rounded bg-neutral-800 px-2 py-1 text-right"
        />
      </label>
      <label className="flex items-center justify-between">
        <span>Sigma high</span>
        <input
          type="number"
          min={1}
          max={5}
          step={0.1}
          value={sigmaHigh}
          onChange={(e) => setSigmaHigh(Number(e.target.value))}
          className="w-16 rounded bg-neutral-800 px-2 py-1 text-right"
        />
      </label>
      <label className="flex items-center gap-2">
        <input
          type="checkbox"
          checked={normalize}
          onChange={(e) => setNormalize(e.target.checked)}
        />
        <span>Normalize before stack</span>
      </label>

      <button
        onClick={handleRun}
        disabled={loading || totalLights === 0}
        className="rounded bg-blue-600 px-3 py-1.5 font-medium hover:bg-blue-500 disabled:opacity-50"
      >
        {loading ? progress || "Processing..." : `Run Pipeline (${totalLights} lights)`}
      </button>

      {error && <div className="text-red-400">{error}</div>}

      {result && (
        <div className="flex flex-col gap-2 border-t border-neutral-800 pt-2">
          <div className="font-semibold text-neutral-400">Results</div>

          <div className="flex gap-1">
            {result.channel_previews.map((ch) => (
              <button
                key={ch.label}
                onClick={() => setActivePreview(ch.label)}
                className={`rounded px-2 py-0.5 ${
                  activePreview === ch.label
                    ? "bg-blue-600"
                    : "bg-neutral-800 hover:bg-neutral-700"
                }`}
              >
                {ch.label}
              </button>
            ))}
            {result.rgb_preview && (
              <button
                onClick={() => setActivePreview("RGB")}
                className={`rounded px-2 py-0.5 ${
                  activePreview === "RGB"
                    ? "bg-blue-600"
                    : "bg-neutral-800 hover:bg-neutral-700"
                }`}
              >
                RGB
              </button>
            )}
          </div>

          {activePreview === "RGB" && result.rgb_preview && (
            <canvas
              ref={rgbCanvasRef}
              className="w-full rounded border border-neutral-700"
              style={{ imageRendering: "auto" }}
            />
          )}

          <div className="flex flex-col gap-1 text-neutral-500">
            {result.stats.channels.map((ch) => (
              <div key={ch.label}>
                {ch.label}: {ch.lights_input} lights, mean={ch.mean.toFixed(1)}{" "}
                std={ch.stddev.toFixed(1)}
              </div>
            ))}
            {result.stats.darks_combined > 0 && (
              <div>Master dark: {result.stats.darks_combined} frames</div>
            )}
            {result.stats.flats_combined > 0 && (
              <div>Master flat: {result.stats.flats_combined} frames</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
