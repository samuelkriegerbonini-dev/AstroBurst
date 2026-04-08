import { useState, useCallback, useMemo } from "react";
import { BarChart3, Check, X, Loader2 } from "lucide-react";
import { Slider, RunButton, SectionHeader } from "../ui";
import { analyzeSubframes, type SubframeMetrics, type SubframeAnalysisResult } from "../../services/analysis";

interface SubframeSelectorPanelProps {
  files: string[];
  onSelectionChange?: (accepted: string[], rejected: string[]) => void;
}

const ICON = <BarChart3 size={14} className="text-teal-400" />;

export default function SubframeSelectorPanel({ files, onSelectionChange }: SubframeSelectorPanelProps) {
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<SubframeAnalysisResult | null>(null);
  const [error, setError] = useState("");
  const [overrides, setOverrides] = useState<Record<string, boolean>>({});

  const [maxFwhm, setMaxFwhm] = useState(8.0);
  const [maxEcc, setMaxEcc] = useState(0.7);
  const [minSnr, setMinSnr] = useState(5.0);
  const [minStars, setMinStars] = useState(5);

  const [sortBy, setSortBy] = useState<keyof SubframeMetrics>("weight");
  const [sortAsc, setSortAsc] = useState(false);

  const handleAnalyze = useCallback(async () => {
    if (files.length === 0) return;
    setLoading(true);
    setError("");
    setResult(null);
    setOverrides({});
    try {
      const res = await analyzeSubframes(files, { maxFwhm, maxEccentricity: maxEcc, minSnr, minStars });
      setResult(res);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [files, maxFwhm, maxEcc, minSnr, minStars]);

  const toggleOverride = useCallback((path: string) => {
    setOverrides((prev) => {
      const current = prev[path];
      const sub = result?.subframes.find((s) => s.file_path === path);
      const original = sub?.accepted ?? true;
      if (current === undefined) {
        return { ...prev, [path]: !original };
      }
      const next = { ...prev };
      delete next[path];
      return next;
    });
  }, [result]);

  const effectiveSubframes = useMemo(() => {
    if (!result) return [];
    return result.subframes.map((s) => ({
      ...s,
      accepted: overrides[s.file_path] !== undefined ? overrides[s.file_path] : s.accepted,
    }));
  }, [result, overrides]);

  const sorted = useMemo(() => {
    const arr = [...effectiveSubframes];
    arr.sort((a, b) => {
      const va = a[sortBy] as number;
      const vb = b[sortBy] as number;
      if (typeof va === "boolean" || typeof vb === "boolean") {
        return sortAsc ? Number(va) - Number(vb) : Number(vb) - Number(va);
      }
      return sortAsc ? va - vb : vb - va;
    });
    return arr;
  }, [effectiveSubframes, sortBy, sortAsc]);

  const acceptedCount = effectiveSubframes.filter((s) => s.accepted).length;
  const rejectedCount = effectiveSubframes.length - acceptedCount;

  const handleApply = useCallback(() => {
    if (!onSelectionChange) return;
    const accepted = effectiveSubframes.filter((s) => s.accepted).map((s) => s.file_path);
    const rejected = effectiveSubframes.filter((s) => !s.accepted).map((s) => s.file_path);
    onSelectionChange(accepted, rejected);
  }, [effectiveSubframes, onSelectionChange]);

  const handleSort = useCallback((col: keyof SubframeMetrics) => {
    if (sortBy === col) {
      setSortAsc((prev) => !prev);
    } else {
      setSortBy(col);
      setSortAsc(false);
    }
  }, [sortBy]);

  if (files.length === 0) {
    return (
      <div className="flex items-center justify-center py-12 text-zinc-600 text-xs">
        Load FITS files to analyze subframe quality.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <SectionHeader icon={ICON} title="Subframe Selector" />

      <div className="text-[10px] text-zinc-500">
        {files.length} file{files.length !== 1 ? "s" : ""} available for analysis.
      </div>

      <div className="grid grid-cols-2 gap-2">
        <Slider label="Max FWHM" value={maxFwhm} min={1} max={20} step={0.5} accent="teal"
                format={(v) => v.toFixed(1)} onChange={setMaxFwhm} />
        <Slider label="Max Eccentricity" value={maxEcc} min={0.1} max={1.0} step={0.05} accent="teal"
                format={(v) => v.toFixed(2)} onChange={setMaxEcc} />
        <Slider label="Min SNR" value={minSnr} min={1} max={50} step={1} accent="teal"
                format={(v) => v.toFixed(0)} onChange={setMinSnr} />
        <Slider label="Min Stars" value={minStars} min={1} max={50} step={1} accent="teal"
                format={(v) => `${v}`} onChange={(v) => setMinStars(Math.round(v))} />
      </div>

      <RunButton
        label="Analyze Subframes"
        runningLabel="Analyzing..."
        running={loading}
        accent="teal"
        onClick={handleAnalyze}
      />

      {error && <div className="text-[9px] text-red-400">{error}</div>}

      {result && (
        <>
          <div className="flex items-center justify-between text-[10px]">
            <span className="text-zinc-400">
              {result.elapsed_ms}ms | {acceptedCount} accepted, {rejectedCount} rejected
            </span>
            {onSelectionChange && (
              <button
                onClick={handleApply}
                className="px-2 py-1 rounded text-[9px] font-medium bg-teal-600/20 text-teal-300 hover:bg-teal-600/30 transition-colors"
              >
                Apply Selection
              </button>
            )}
          </div>

          <div className="overflow-x-auto rounded-lg border border-zinc-800/50">
            <table className="w-full text-[9px]">
              <thead>
              <tr className="bg-zinc-900/50 text-zinc-500 uppercase tracking-wider">
                <th className="px-2 py-1.5 text-left w-6"></th>
                <ThSort label="File" col="file_name" current={sortBy} asc={sortAsc} onClick={handleSort} />
                <ThSort label="Stars" col="star_count" current={sortBy} asc={sortAsc} onClick={handleSort} />
                <ThSort label="FWHM" col="median_fwhm" current={sortBy} asc={sortAsc} onClick={handleSort} />
                <ThSort label="Ecc" col="median_eccentricity" current={sortBy} asc={sortAsc} onClick={handleSort} />
                <ThSort label="SNR" col="median_snr" current={sortBy} asc={sortAsc} onClick={handleSort} />
                <ThSort label="Weight" col="weight" current={sortBy} asc={sortAsc} onClick={handleSort} />
              </tr>
              </thead>
              <tbody>
              {sorted.map((sub) => {
                const isOverridden = overrides[sub.file_path] !== undefined;
                return (
                  <tr
                    key={sub.file_path}
                    className={`border-t border-zinc-800/30 transition-colors cursor-pointer hover:bg-zinc-800/30 ${
                      sub.accepted ? "" : "opacity-40"
                    } ${isOverridden ? "ring-1 ring-inset ring-amber-500/20" : ""}`}
                    onClick={() => toggleOverride(sub.file_path)}
                  >
                    <td className="px-2 py-1">
                      {sub.accepted
                        ? <Check size={10} className="text-emerald-400" />
                        : <X size={10} className="text-red-400" />
                      }
                    </td>
                    <td className="px-2 py-1 text-zinc-300 font-mono truncate max-w-[120px]" title={sub.file_name}>
                      {sub.file_name}
                    </td>
                    <td className="px-2 py-1 text-right font-mono text-zinc-400">{sub.star_count}</td>
                    <td className={`px-2 py-1 text-right font-mono ${sub.median_fwhm > maxFwhm ? "text-red-400" : "text-zinc-300"}`}>
                      {sub.median_fwhm.toFixed(2)}
                    </td>
                    <td className={`px-2 py-1 text-right font-mono ${sub.median_eccentricity > maxEcc ? "text-red-400" : "text-zinc-300"}`}>
                      {sub.median_eccentricity.toFixed(3)}
                    </td>
                    <td className={`px-2 py-1 text-right font-mono ${sub.median_snr < minSnr ? "text-red-400" : "text-zinc-300"}`}>
                      {sub.median_snr.toFixed(1)}
                    </td>
                    <td className="px-2 py-1 text-right">
                      <div className="flex items-center justify-end gap-1">
                        <div className="w-12 h-1.5 bg-zinc-800 rounded-full overflow-hidden">
                          <div
                            className="h-full rounded-full bg-teal-500"
                            style={{ width: `${(sub.weight * 100).toFixed(0)}%` }}
                          />
                        </div>
                        <span className="font-mono text-zinc-400 w-8 text-right">{(sub.weight * 100).toFixed(0)}%</span>
                      </div>
                    </td>
                  </tr>
                );
              })}
              </tbody>
            </table>
          </div>
        </>
      )}
    </div>
  );
}

function ThSort({
                  label,
                  col,
                  current,
                  asc,
                  onClick,
                }: {
  label: string;
  col: keyof SubframeMetrics;
  current: keyof SubframeMetrics;
  asc: boolean;
  onClick: (col: keyof SubframeMetrics) => void;
}) {
  const active = current === col;
  return (
    <th
      className={`px-2 py-1.5 text-right cursor-pointer select-none hover:text-zinc-300 transition-colors ${active ? "text-teal-400" : ""}`}
      onClick={() => onClick(col)}
    >
      {label}
      {active && <span className="ml-0.5">{asc ? "▲" : "▼"}</span>}
    </th>
  );
}
