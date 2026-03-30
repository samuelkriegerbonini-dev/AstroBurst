import { useState, useCallback } from "react";
import { spccCalibrate } from "../../services/processing.service";
import { RunButton, ResultGrid, ErrorAlert, SectionHeader } from "../ui";

interface SpccResult {
  r_factor?: number;
  g_factor?: number;
  b_factor?: number;
  stars_matched?: number;
  stars_total?: number;
  avg_color_index?: number;
  white_reference?: string;
  catalog_name?: string;
  elapsed_ms?: number;
}

interface SpccPanelProps {
  rPath: string | null;
  gPath: string | null;
  bPath: string | null;
  wcsPath?: string | null;
  onFactorsReady?: (r: number, g: number, b: number) => void;
}

const WHITE_REFS = [
  { value: "average_spiral", label: "Average Spiral Galaxy" },
  { value: "g2v", label: "G2V (Solar)" },
  { value: "photopic", label: "Photopic (Human Eye)" },
];

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-cyan-400">
    <circle cx="12" cy="12" r="5" />
    <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
  </svg>
);

export default function SpccPanel({ rPath, gPath, bPath, wcsPath, onFactorsReady }: SpccPanelProps) {
  const [whiteRef, setWhiteRef] = useState("average_spiral");
  const [minSnr, setMinSnr] = useState(20);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<SpccResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const canRun = rPath && gPath && bPath;

  const handleRun = useCallback(async () => {
    if (!rPath || !gPath || !bPath) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await spccCalibrate(rPath, gPath, bPath, {
        wcsPath: wcsPath ?? undefined,
        whiteReference: whiteRef,
        minSnr,
      }) as SpccResult;
      setResult(res);
      if (res?.r_factor != null && res?.g_factor != null && res?.b_factor != null) {
        onFactorsReady?.(res.r_factor, res.g_factor, res.b_factor);
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsRunning(false);
    }
  }, [rPath, gPath, bPath, wcsPath, whiteRef, minSnr, onFactorsReady]);

  return (
    <div className="flex flex-col gap-3 p-3 border border-cyan-800/20 bg-cyan-900/5 rounded-lg">
      <SectionHeader icon={ICON} title="SPCC" subtitle="Spectrophotometric Color Calibration" />

      {!canRun && (
        <div className="text-[10px] text-zinc-500 italic">
          Assign R, G, B channels above. Plate Solve required for WCS.
        </div>
      )}

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">White Reference</label>
        <select
          value={whiteRef}
          onChange={(e) => setWhiteRef(e.target.value)}
          className="ab-select"
          disabled={isRunning}
        >
          {WHITE_REFS.map((wr) => (
            <option key={wr.value} value={wr.value}>{wr.label}</option>
          ))}
        </select>
      </div>

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">Min SNR</label>
        <input
          type="number"
          value={minSnr}
          min={5}
          max={100}
          step={5}
          onChange={(e) => setMinSnr(Number(e.target.value))}
          className="w-16 text-right text-xs bg-zinc-800/50 border border-zinc-700 rounded px-2 py-1 text-zinc-200 font-mono"
          disabled={isRunning}
        />
      </div>

      <RunButton
        label="Run SPCC"
        runningLabel="Calibrating..."
        running={isRunning}
        disabled={!canRun}
        accent="cyan"
        onClick={handleRun}
      />

      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in">
          <div className="grid grid-cols-3 gap-1.5">
            <div className="ab-metric-card flex flex-col items-center p-2 rounded">
              <span className="text-[9px] text-red-400/70 uppercase">R Factor</span>
              <span className="text-sm font-mono text-zinc-200">{result.r_factor?.toFixed(4)}</span>
            </div>
            <div className="ab-metric-card flex flex-col items-center p-2 rounded">
              <span className="text-[9px] text-green-400/70 uppercase">G Factor</span>
              <span className="text-sm font-mono text-zinc-200">{result.g_factor?.toFixed(4)}</span>
            </div>
            <div className="ab-metric-card flex flex-col items-center p-2 rounded">
              <span className="text-[9px] text-blue-400/70 uppercase">B Factor</span>
              <span className="text-sm font-mono text-zinc-200">{result.b_factor?.toFixed(4)}</span>
            </div>
          </div>

          <ResultGrid items={[
            { label: "Stars", value: `${result.stars_matched}/${result.stars_total}` },
            { label: "Avg Bp-Rp", value: result.avg_color_index?.toFixed(3) },
            { label: "Catalog", value: result.catalog_name },
            { label: "Reference", value: result.white_reference },
            { label: "Time", value: result.elapsed_ms ? `${(result.elapsed_ms / 1000).toFixed(1)}s` : null },
          ]} />

          <div className="text-[10px] text-cyan-400/80 bg-cyan-900/20 border border-cyan-800/20 rounded px-2.5 py-1.5">
            Factors applied to Manual WB. Re-compose to see the result.
          </div>
        </div>
      )}
    </div>
  );
}
