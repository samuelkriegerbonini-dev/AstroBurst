import { useState, useCallback } from "react";
import { Loader2, Workflow, Play, CheckCircle2, AlertCircle, ChevronDown, ChevronRight, Settings2 } from "lucide-react";
import { useBackend } from "../../hooks/useBackend";
import type { ProcessedFile } from "../../utils/types";
import type { CalibrationState, StackConfig } from "../preview/StackingTab";

interface PipelinePanelProps {
  files: ProcessedFile[];
  onPreviewUpdate?: (url: string | null | undefined) => void;
  calibration?: CalibrationState;
  stackConfig?: StackConfig;
}

function ConfigSummary({
                         calibration,
                         stackConfig,
                       }: {
  calibration?: CalibrationState;
  stackConfig?: StackConfig;
}) {
  const hasCal = calibration && (calibration.hasBias || calibration.hasDark || calibration.hasFlat);
  const hasConfig = stackConfig !== undefined;

  if (!hasCal && !hasConfig) return null;

  return (
    <div className="bg-zinc-900/60 rounded-lg border border-zinc-800/40 px-3 py-2.5 space-y-2">
      <div className="flex items-center gap-1.5 text-[10px] text-zinc-400 font-medium uppercase tracking-wider">
        <Settings2 size={10} />
        Workflow Summary
      </div>

      {hasCal && (
        <div className="flex items-center gap-2 text-[10px]">
          <span className="text-violet-400">Calibration:</span>
          <div className="flex gap-1.5">
            {calibration!.hasBias && (
              <span className="px-1.5 py-0.5 rounded bg-violet-500/10 text-violet-300 border border-violet-500/20">Bias</span>
            )}
            {calibration!.hasDark && (
              <span className="px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-300 border border-blue-500/20">Dark</span>
            )}
            {calibration!.hasFlat && (
              <span className="px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-300 border border-amber-500/20">Flat</span>
            )}
          </div>
        </div>
      )}

      {hasConfig && (
        <div className="flex items-center gap-3 text-[10px] font-mono text-zinc-500 flex-wrap">
          <span>
            Sigma <span className="text-amber-400/80">{stackConfig!.sigmaLow.toFixed(1)}</span>/<span className="text-amber-400/80">{stackConfig!.sigmaHigh.toFixed(1)}</span>
          </span>
          <span>
            Iter <span className="text-amber-400/80">{stackConfig!.maxIterations}</span>
          </span>
          <span className={stackConfig!.align ? "text-emerald-400/80" : "text-zinc-600"}>
            Align {stackConfig!.align ? "ON" : "OFF"}
          </span>
        </div>
      )}

      {calibration?.calibratedFitsPath && (
        <div className="text-[10px] text-emerald-400/60 truncate">
          Calibrated: {calibration.calibratedFitsPath.split(/[/\\]/).pop()}
        </div>
      )}
    </div>
  );
}

function ResultSummary({ data }: { data: any }) {
  const [expanded, setExpanded] = useState(false);

  if (!data) return null;

  const entries = Object.entries(data);
  const previewKeys = ["png_path", "collapsed_path", "corrected_png", "fits_path"];
  const statsKeys = ["elapsed_ms", "dimensions", "frame_count", "rejected_pixels"];

  const stats = entries.filter(([k]) => statsKeys.includes(k));
  const paths = entries.filter(([k]) => previewKeys.includes(k));
  const rest = entries.filter(([k]) => !statsKeys.includes(k) && !previewKeys.includes(k));

  return (
    <div className="bg-zinc-800/40 rounded-lg px-3 py-2 text-[10px] font-mono space-y-1">
      {stats.map(([k, v]) => (
        <div key={k} className="flex justify-between">
          <span className="text-zinc-500">{k}</span>
          <span className="text-zinc-300">
            {Array.isArray(v) ? v.join(" x ") : typeof v === "number" && k.includes("ms") ? `${(Number(v) / 1000).toFixed(1)}s` : String(v)}
          </span>
        </div>
      ))}
      {paths.map(([k, v]) => (
        <div key={k} className="flex justify-between">
          <span className="text-zinc-500">{k}</span>
          <span className="text-emerald-400 truncate ml-2 max-w-[160px]">{String(v).split("/").pop()}</span>
        </div>
      ))}
      {rest.length > 0 && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex items-center gap-1 text-zinc-500 hover:text-zinc-300 transition-colors mt-1"
        >
          {expanded ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
          {rest.length} more fields
        </button>
      )}
      {expanded && (
        <pre className="text-[9px] text-zinc-500 whitespace-pre-wrap max-h-[100px] overflow-y-auto mt-1">
          {JSON.stringify(Object.fromEntries(rest), null, 2)}
        </pre>
      )}
    </div>
  );
}

export default function PipelinePanel({ files = [], onPreviewUpdate, calibration, stackConfig }: PipelinePanelProps) {
  const { runPipeline } = useBackend();
  const [inputPath, setInputPath] = useState<string | null>(null);
  const [frameStep, setFrameStep] = useState(5);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);

  const handleRun = useCallback(async () => {
    if (!inputPath) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await runPipeline(inputPath, "./output", frameStep);
      setResult(res);
      const url = res?.previewUrl || res?.collapsedPreviewUrl;
      if (url) {
        onPreviewUpdate?.(url);
      }
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setIsRunning(false);
    }
  }, [inputPath, frameStep, runPipeline, onPreviewUpdate]);

  const totalResults = result?.results?.length || 0;
  const failedCount = result?.failed || 0;
  const successCount = totalResults - failedCount;

  return (
    <div className="flex flex-col gap-3">
      <ConfigSummary calibration={calibration} stackConfig={stackConfig} />

      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4">
        <h4 className="text-xs font-semibold text-cyan-400 uppercase tracking-wider flex items-center gap-1.5 mb-3">
          <Workflow size={12} />
          Pipeline Runner
        </h4>
        <p className="text-[10px] text-zinc-500 mb-3">
          Runs the full pipeline: calibrate, stack, and process in one step.
        </p>

        <div className="space-y-3">
          <div>
            <label className="text-[10px] text-zinc-400 block mb-1.5">Input File</label>
            <select
              value={inputPath || ""}
              onChange={(e) => setInputPath(e.target.value || null)}
              className="w-full bg-zinc-900 border border-zinc-700/50 rounded-md px-3 py-2 text-xs text-zinc-200 outline-none focus:border-cyan-500/50"
            >
              <option value="">Select file...</option>
              {calibration?.calibratedFitsPath && (
                <option value={calibration.calibratedFitsPath}>
                  {calibration.calibratedFitsPath.split(/[/\\]/).pop()} (calibrated)
                </option>
              )}
              {files.map((f) => (
                <option key={f.id} value={f.path}>
                  {f.name}
                </option>
              ))}
            </select>
          </div>

          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="text-[10px] text-zinc-400">Frame Step (for cubes)</label>
              <span className="text-[10px] font-mono text-zinc-500">{frameStep}</span>
            </div>
            <input
              type="range"
              min={1}
              max={20}
              step={1}
              value={frameStep}
              onChange={(e) => setFrameStep(parseInt(e.target.value))}
              className="w-full accent-cyan-500"
            />
          </div>
        </div>
      </div>

      <button
        onClick={handleRun}
        disabled={!inputPath || isRunning}
        className="flex items-center justify-center gap-2 rounded-lg px-4 py-2.5 text-sm font-medium transition-all disabled:opacity-30 disabled:cursor-not-allowed"
        style={{
          background: "rgba(6,182,212,0.15)",
          color: "#67e8f9",
          border: "1px solid rgba(6,182,212,0.25)",
        }}
      >
        {isRunning ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} />}
        {isRunning ? "Running Pipeline..." : "Run Pipeline"}
      </button>

      {error && (
        <div className="flex items-start gap-2 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 text-xs text-red-300">
          <AlertCircle size={14} className="shrink-0 mt-0.5" />
          {error}
        </div>
      )}

      {result && (
        <div className="bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5 space-y-2">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            Pipeline Complete
          </div>

          <div className="grid grid-cols-3 gap-2 text-[10px]">
            <div className="bg-zinc-900/60 rounded px-2 py-1.5 text-center">
              <div className="text-zinc-500">Time</div>
              <div className="text-zinc-200 font-mono">
                {result.elapsed_ms ? `${(result.elapsed_ms / 1000).toFixed(1)}s` : "--"}
              </div>
            </div>
            <div className="bg-zinc-900/60 rounded px-2 py-1.5 text-center">
              <div className="text-zinc-500">Success</div>
              <div className="text-emerald-300 font-mono">{successCount}</div>
            </div>
            <div className="bg-zinc-900/60 rounded px-2 py-1.5 text-center">
              <div className="text-zinc-500">Failed</div>
              <div className={`font-mono ${failedCount > 0 ? "text-red-400" : "text-zinc-500"}`}>
                {failedCount}
              </div>
            </div>
          </div>

          {result.results && result.results.length > 0 && (
            <div className="space-y-1.5 mt-1">
              {result.results.map((item: any, idx: number) => {
                const isOk = item.Ok !== undefined;
                const data = isOk ? item.Ok : item.Err;
                return (
                  <div key={idx}>
                    <div className={`text-[10px] font-medium mb-0.5 ${isOk ? "text-zinc-400" : "text-red-400"}`}>
                      Step {idx + 1} {isOk ? "" : "(failed)"}
                    </div>
                    {isOk && typeof data === "object" ? (
                      <ResultSummary data={data} />
                    ) : (
                      <div className="text-[10px] font-mono text-red-300 bg-red-900/20 rounded px-2 py-1">
                        {typeof data === "string" ? data : JSON.stringify(data)}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
