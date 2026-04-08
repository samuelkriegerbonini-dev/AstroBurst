import { useState, useCallback, useMemo } from "react";
import { Loader2, BarChart3, Check, X } from "lucide-react";
import type { WizardState } from "../wizard";
import { stackFrames } from "../../../services/stacking";
import { analyzeSubframes, type SubframeMetrics, type SubframeAnalysisResult } from "../../../services/analysis";
import { getOutputDir } from "../../../infrastructure/tauri";
import { RunButton, Slider } from "../../ui";

interface StackStepProps {
  state: WizardState;
  dispatch: React.Dispatch<any>;
  onStacked: (channelId: string, path: string) => void;
}

export default function StackStep({ state, dispatch, onStacked }: StackStepProps) {
  const [loading, setLoading] = useState<Record<string, boolean>>({});
  const [results, setResults] = useState<Record<string, any>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [analyzing, setAnalyzing] = useState<Record<string, boolean>>({});
  const [analyzeErrors, setAnalyzeErrors] = useState<Record<string, string>>({});
  const [overrides, setOverrides] = useState<Record<string, Record<string, boolean>>>({});
  const [maxFwhm, setMaxFwhm] = useState(8.0);
  const [maxEcc, setMaxEcc] = useState(0.7);
  const [minSnr, setMinSnr] = useState(5.0);
  const [minStars, setMinStars] = useState(5);

  const stackableBins = useMemo(
    () => state.bins.filter((b) => b.files.length > 1),
    [state.bins],
  );

  const singleBins = useMemo(
    () => state.bins.filter((b) => b.files.length === 1),
    [state.bins],
  );

  const getEffectiveFiles = useCallback((binId: string, files: string[]) => {
    const excluded = state.excludedFiles[binId] ?? [];
    if (excluded.length === 0) return files;
    const set = new Set(excluded);
    return files.filter((f) => !set.has(f));
  }, [state.excludedFiles]);

  const handleAnalyze = useCallback(async (binId: string, files: string[]) => {
    if (files.length === 0) return;
    setAnalyzing((prev) => ({ ...prev, [binId]: true }));
    setAnalyzeErrors((prev) => ({ ...prev, [binId]: "" }));
    setOverrides((prev) => ({ ...prev, [binId]: {} }));
    try {
      const res = await analyzeSubframes(files, { maxFwhm, maxEccentricity: maxEcc, minSnr, minStars });
      dispatch({ type: "SET_SUBFRAME_RESULT", binId, result: res });
    } catch (e: any) {
      setAnalyzeErrors((prev) => ({ ...prev, [binId]: e?.message ?? String(e) }));
    } finally {
      setAnalyzing((prev) => ({ ...prev, [binId]: false }));
    }
  }, [maxFwhm, maxEcc, minSnr, minStars, dispatch]);

  const toggleOverride = useCallback((binId: string, filePath: string) => {
    setOverrides((prev) => {
      const binOverrides = { ...(prev[binId] ?? {}) };
      const sub = state.subframeResults[binId]?.subframes.find((s) => s.file_path === filePath);
      const original = sub?.accepted ?? true;
      if (binOverrides[filePath] === undefined) {
        binOverrides[filePath] = !original;
      } else {
        delete binOverrides[filePath];
      }
      return { ...prev, [binId]: binOverrides };
    });
  }, [state.subframeResults]);

  const applySelection = useCallback((binId: string) => {
    const result = state.subframeResults[binId];
    if (!result) return;
    const binOv = overrides[binId] ?? {};
    const rejected = result.subframes
      .filter((s) => {
        const ov = binOv[s.file_path];
        return ov !== undefined ? !ov : !s.accepted;
      })
      .map((s) => s.file_path);
    dispatch({ type: "SET_EXCLUDED_FILES", binId, files: rejected });
  }, [state.subframeResults, overrides, dispatch]);

  const handleStack = useCallback(async (binId: string, allFiles: string[]) => {
    const files = getEffectiveFiles(binId, allFiles);
    if (!files || files.length < 2) {
      setErrors((prev) => ({ ...prev, [binId]: `Need at least 2 files after exclusions, got ${files?.length ?? 0}` }));
      return;
    }
    setLoading((prev) => ({ ...prev, [binId]: true }));
    setErrors((prev) => ({ ...prev, [binId]: "" }));
    try {
      const result = await stackFrames(files, await getOutputDir(), {
        sigmaLow: 3.0,
        sigmaHigh: 3.0,
        align: true,
        name: `stacked_${binId}`,
      });
      setResults((prev) => ({ ...prev, [binId]: result }));
      if (result.fits_path) {
        onStacked(binId, result.fits_path);
      }
    } catch (e: any) {
      const msg = e?.message ?? String(e);
      console.error(`[AstroBurst] Stack failed for ${binId}:`, msg);
      setErrors((prev) => ({ ...prev, [binId]: msg }));
    } finally {
      setLoading((prev) => ({ ...prev, [binId]: false }));
    }
  }, [onStacked, getEffectiveFiles]);

  const handleStackAll = useCallback(async () => {
    const bins = stackableBins.slice();
    const promises = bins.map((bin) => handleStack(bin.id, bin.files));
    await Promise.allSettled(promises);
  }, [stackableBins, handleStack]);

  if (stackableBins.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-zinc-600 text-xs gap-2">
        <span>No channels have multiple files to stack.</span>
        {singleBins.length > 0 && (
          <span className="text-[10px] text-zinc-700">
            {singleBins.length} channel(s) with single files will be used directly.
          </span>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center justify-between">
        <span className="text-xs text-zinc-400">
          {stackableBins.length} channel(s) to stack
        </span>
        <RunButton
          label="Stack All"
          runningLabel="Stacking..."
          running={Object.values(loading).some(Boolean)}
          accent="blue"
          onClick={handleStackAll}
          small
        />
      </div>

      {stackableBins.map((bin) => {
        const isLoading = loading[bin.id];
        const result = results[bin.id];
        const error = errors[bin.id];
        const isStacked = !!state.stackedPaths[bin.id] || !!result;
        const isAnalyzing = analyzing[bin.id];
        const analyzeError = analyzeErrors[bin.id];
        const subResult = state.subframeResults[bin.id];
        const binOv = overrides[bin.id] ?? {};
        const excluded = state.excludedFiles[bin.id] ?? [];
        const effectiveCount = getEffectiveFiles(bin.id, bin.files).length;

        return (
          <div key={bin.id} className="flex flex-col gap-1.5 p-2 rounded-lg border"
               style={{
                 borderColor: isStacked ? `${bin.color}40` : "rgba(63,63,70,0.3)",
                 background: isStacked ? `${bin.color}08` : "rgba(24,24,27,0.3)",
               }}>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-1.5">
                <span className="w-2 h-2 rounded-full" style={{ background: bin.color }} />
                <span className="text-[10px] font-medium text-zinc-300">{bin.shortLabel}</span>
                <span className="text-[9px] text-zinc-600">{effectiveCount}/{bin.files.length} frames</span>
              </div>
              <div className="flex items-center gap-1">
                <button
                  onClick={() => handleAnalyze(bin.id, bin.files)}
                  disabled={isAnalyzing}
                  className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] bg-teal-600/20 text-teal-400 hover:bg-teal-600/30 disabled:opacity-40 transition-all"
                >
                  {isAnalyzing ? <Loader2 size={9} className="animate-spin" /> : <BarChart3 size={9} />}
                  Analyze
                </button>
                <button
                  onClick={() => handleStack(bin.id, bin.files)}
                  disabled={isLoading}
                  className="flex items-center gap-1 px-2 py-0.5 rounded text-[9px] bg-blue-600/20 text-blue-400 hover:bg-blue-600/30 disabled:opacity-40 transition-all"
                >
                  {isLoading ? <Loader2 size={9} className="animate-spin" /> : null}
                  {isLoading ? "Stacking..." : isStacked ? "Re-stack" : "Stack"}
                </button>
              </div>
            </div>

            {analyzeError && <div className="text-[9px] text-red-400">{analyzeError}</div>}

            {subResult && (
              <div className="flex flex-col gap-1">
                <div className="flex items-center justify-between text-[9px] text-zinc-500">
                  <span>{subResult.elapsed_ms}ms | {subResult.accepted} accepted, {subResult.rejected} rejected</span>
                  <button
                    onClick={() => applySelection(bin.id)}
                    className="px-1.5 py-0.5 rounded text-[8px] bg-teal-600/20 text-teal-300 hover:bg-teal-600/30 transition-colors"
                  >
                    Apply
                  </button>
                </div>
                <div className="overflow-x-auto rounded border border-zinc-800/50 max-h-32 overflow-y-auto">
                  <table className="w-full text-[8px]">
                    <thead>
                    <tr className="bg-zinc-900/50 text-zinc-500 uppercase tracking-wider sticky top-0">
                      <th className="px-1.5 py-1 w-5"></th>
                      <th className="px-1.5 py-1 text-left">File</th>
                      <th className="px-1.5 py-1 text-right">Stars</th>
                      <th className="px-1.5 py-1 text-right">FWHM</th>
                      <th className="px-1.5 py-1 text-right">SNR</th>
                      <th className="px-1.5 py-1 text-right">Wt</th>
                    </tr>
                    </thead>
                    <tbody>
                    {subResult.subframes.map((sub) => {
                      const ov = binOv[sub.file_path];
                      const accepted = ov !== undefined ? ov : sub.accepted;
                      return (
                        <tr
                          key={sub.file_path}
                          className={`border-t border-zinc-800/30 cursor-pointer hover:bg-zinc-800/30 ${accepted ? "" : "opacity-40"}`}
                          onClick={() => toggleOverride(bin.id, sub.file_path)}
                        >
                          <td className="px-1.5 py-0.5">
                            {accepted ? <Check size={8} className="text-emerald-400" /> : <X size={8} className="text-red-400" />}
                          </td>
                          <td className="px-1.5 py-0.5 text-zinc-300 font-mono truncate max-w-[80px]" title={sub.file_name}>
                            {sub.file_name}
                          </td>
                          <td className="px-1.5 py-0.5 text-right font-mono text-zinc-400">{sub.star_count}</td>
                          <td className="px-1.5 py-0.5 text-right font-mono text-zinc-300">{sub.median_fwhm.toFixed(2)}</td>
                          <td className="px-1.5 py-0.5 text-right font-mono text-zinc-300">{sub.median_snr.toFixed(1)}</td>
                          <td className="px-1.5 py-0.5 text-right font-mono text-zinc-400">{(sub.weight * 100).toFixed(0)}%</td>
                        </tr>
                      );
                    })}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {excluded.length > 0 && !subResult && (
              <div className="text-[8px] text-amber-400/60">{excluded.length} file(s) excluded</div>
            )}

            {result && (
              <div className="text-[9px] text-zinc-500">
                {result.frame_count} frames, {result.rejected_pixels ?? 0} rejected px, {result.elapsed_ms ?? "?"}ms
              </div>
            )}
            {error && <div className="text-[9px] text-red-400">{error}</div>}
          </div>
        );
      })}

      {singleBins.length > 0 && (
        <div className="text-[9px] text-zinc-600 pt-1 border-t border-zinc-800/30">
          {singleBins.map((b) => b.shortLabel).join(", ")} have single files (no stacking needed).
        </div>
      )}
    </div>
  );
}
