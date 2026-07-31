import { useState, useCallback } from "react";
import { Grid3X3, CheckCircle2, Layers } from "lucide-react";
import { RunButton, ResultGrid, ErrorAlert, SectionHeader } from "../ui";
import { debayerFits, debayerBatch } from "../../services/processing";
import type { DebayerResult, DebayerBatchResult } from "../../services/processing";
import { useDoneFilesContext } from "../../context/PreviewContext";

interface DebayerPanelProps {
  selectedFile: { path: string; name?: string } | null;
  outputDir: string;
  onPreviewUpdate?: (url: string | null | undefined) => void;
}

const PATTERNS = [
  { value: "", label: "Auto (from header)" },
  { value: "RGGB", label: "RGGB" },
  { value: "BGGR", label: "BGGR" },
  { value: "GRBG", label: "GRBG" },
  { value: "GBRG", label: "GBRG" },
] as const;

const METHODS = [
  { value: "bilinear", label: "Bilinear (full resolution)" },
  { value: "superpixel", label: "Super-pixel (half resolution)" },
] as const;

const ICON = <Grid3X3 size={14} className="text-orange-400" />;

export default function DebayerPanel({ selectedFile, outputDir, onPreviewUpdate }: DebayerPanelProps) {
  const { doneFiles } = useDoneFilesContext();
  const [pattern, setPattern] = useState("");
  const [method, setMethod] = useState<"bilinear" | "superpixel">("bilinear");
  const [isRunning, setIsRunning] = useState(false);
  const [isBatchRunning, setIsBatchRunning] = useState(false);
  const [result, setResult] = useState<DebayerResult | null>(null);
  const [batchResult, setBatchResult] = useState<DebayerBatchResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleRun = useCallback(async () => {
    if (!selectedFile?.path) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    setBatchResult(null);
    try {
      const res = await debayerFits(selectedFile.path, outputDir, {
        method,
        pattern: pattern || undefined,
      });
      setResult(res);
      onPreviewUpdate?.(res.previewUrl);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsRunning(false);
    }
  }, [selectedFile, outputDir, method, pattern, onPreviewUpdate]);

  const handleBatch = useCallback(async () => {
    if (doneFiles.length === 0) return;
    setIsBatchRunning(true);
    setError(null);
    setBatchResult(null);
    try {
      const res = await debayerBatch(
        doneFiles.map((f) => f.path),
        outputDir,
        { method, pattern: pattern || undefined },
      );
      setBatchResult(res);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsBatchRunning(false);
    }
  }, [doneFiles, outputDir, method, pattern]);

  const fileName = selectedFile?.name || selectedFile?.path?.split(/[/\\]/).pop();

  return (
    <div className="flex flex-col gap-3 p-3">
      <SectionHeader
        icon={ICON}
        title="Debayer (OSC)"
        subtitle="Reconstruct R/G/B from a Bayer color camera frame"
      />

      {!selectedFile && (
        <div className="text-[10px] text-zinc-500 italic">Load a CFA FITS (one-shot color camera) first.</div>
      )}

      {fileName && (
        <div className="text-[10px] text-zinc-500 truncate" title={selectedFile?.path}>
          Input: <span className="text-zinc-300">{fileName}</span>
        </div>
      )}

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">Bayer Pattern</label>
        <select
          value={pattern}
          onChange={(e) => setPattern(e.target.value)}
          className="ab-select"
          disabled={isRunning || isBatchRunning}
        >
          {PATTERNS.map((p) => (
            <option key={p.value} value={p.value}>{p.label}</option>
          ))}
        </select>
      </div>

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">Method</label>
        <select
          value={method}
          onChange={(e) => setMethod(e.target.value as typeof method)}
          className="ab-select"
          disabled={isRunning || isBatchRunning}
        >
          {METHODS.map((m) => (
            <option key={m.value} value={m.value}>{m.label}</option>
          ))}
        </select>
      </div>

      <RunButton
        label="Debayer Current File"
        runningLabel="Debayering..."
        running={isRunning}
        disabled={!selectedFile?.path || isBatchRunning}
        accent="amber"
        onClick={handleRun}
      />

      <RunButton
        label={`Debayer All Loaded (${doneFiles.length})`}
        runningLabel="Debayering batch..."
        running={isBatchRunning}
        disabled={doneFiles.length === 0 || isRunning}
        accent="amber"
        small
        icon={<Layers size={12} />}
        onClick={handleBatch}
      />

      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            Debayer Complete ({result.pattern}, {result.method})
          </div>
          <ResultGrid columns={2} items={[
            { label: "Output", value: result.dimensions ? `${result.dimensions[0]}×${result.dimensions[1]}` : "--" },
            { label: "Time", value: `${result.elapsed_ms}ms` },
          ]} />
          <div className="flex flex-col gap-1">
            {[
              { ch: "R", path: result.r_path, color: "text-red-300" },
              { ch: "G", path: result.g_path, color: "text-green-300" },
              { ch: "B", path: result.b_path, color: "text-blue-300" },
            ].map((c) => (
              <div key={c.ch} className="flex items-center gap-2 text-[10px]">
                <span className={`font-bold w-3 ${c.color}`}>{c.ch}</span>
                <span className="text-zinc-500 truncate" title={c.path}>{c.path.split(/[/\\]/).pop()}</span>
              </div>
            ))}
          </div>
          <div className="text-[9px] text-zinc-600">
            Use these three FITS files in the Compose wizard or stack each channel separately.
          </div>
        </div>
      )}

      {batchResult && (
        <div className="flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            Batch Complete — {batchResult.succeeded} ok, {batchResult.failed} failed
          </div>
          {batchResult.results.filter((r) => r.error).slice(0, 4).map((r) => (
            <div key={r.path} className="text-[9px] text-amber-400/90 truncate" title={`${r.path}: ${r.error}`}>
              {r.path.split(/[/\\]/).pop()}: {r.error}
            </div>
          ))}
          <div className="text-[9px] text-zinc-600">
            R/G/B FITS files were written next to your output folder, named &lt;frame&gt;_R/_G/_B.fits.
          </div>
        </div>
      )}
    </div>
  );
}
