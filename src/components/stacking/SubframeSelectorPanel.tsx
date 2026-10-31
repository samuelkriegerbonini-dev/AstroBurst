import { useState, useCallback, useMemo, useId } from "react";
import { BarChart3, Check, X } from "lucide-react";
import { Slider, RunButton, SectionHeader } from "../ui";
import { analyzeSubframes, type SubframeMetrics, type SubframeAnalysisResult } from "../../services/analysis";
import type { SubframeNoiseMetrics } from "../../shared/types/statistics";
import {
  DEFAULT_SUBFRAME_SETTINGS,
  SUBFRAME_STALE_NOTE,
  subframeFilesChanged,
  subframeSettingsChanged,
  subframeStaleNote,
  type SubframeSettings,
  type SubframeThresholds,
} from "../../utils/subframeSettings";

interface SubframeSelectorPanelProps {
  files: string[];
  leftOutLine?: string | null;
  onSelectionChange?: (accepted: string[], rejected: string[], weightsByPath?: Record<string, number>) => void;
}

type SubframeRow = SubframeMetrics & Partial<SubframeNoiseMetrics>;
type SubframeColumn = keyof SubframeRow;

const ICON = <BarChart3 size={14} className="text-teal-400" />;

function formatNoise(sigma: number | undefined): string {
  if (sigma == null || !Number.isFinite(sigma)) return "--";
  if (sigma === 0) return "0";
  return Math.abs(sigma) < 1e-3 || Math.abs(sigma) >= 1e6 ? sigma.toExponential(2) : sigma.toPrecision(4);
}

function LeftOutLine({ text }: { text: string | null | undefined }) {
  if (!text) return null;
  return <p data-testid="subframe-left-out" className="text-[10px] text-zinc-500 leading-snug">{text}</p>;
}

export default function SubframeSelectorPanel({ files, leftOutLine, onSelectionChange }: SubframeSelectorPanelProps) {
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<SubframeAnalysisResult | null>(null);
  const [analysedWith, setAnalysedWith] = useState<SubframeSettings | null>(null);
  const [analysedFiles, setAnalysedFiles] = useState<readonly string[] | null>(null);
  const [error, setError] = useState("");
  const [overrides, setOverrides] = useState<Record<string, boolean>>({});

  const [maxFwhm, setMaxFwhm] = useState(DEFAULT_SUBFRAME_SETTINGS.maxFwhm);
  const [maxEcc, setMaxEcc] = useState(DEFAULT_SUBFRAME_SETTINGS.maxEccentricity);
  const [minSnr, setMinSnr] = useState(DEFAULT_SUBFRAME_SETTINGS.minSnr);
  const [minStars, setMinStars] = useState(DEFAULT_SUBFRAME_SETTINGS.minStars);

  const [fwhmWeight, setFwhmWeight] = useState(DEFAULT_SUBFRAME_SETTINGS.fwhmWeight);
  const [eccWeight, setEccWeight] = useState(DEFAULT_SUBFRAME_SETTINGS.eccentricityWeight);
  const [snrWeight, setSnrWeight] = useState(DEFAULT_SUBFRAME_SETTINGS.snrWeight);
  const [noiseWeight, setNoiseWeight] = useState(DEFAULT_SUBFRAME_SETTINGS.noiseWeight);

  const [sortBy, setSortBy] = useState<SubframeColumn>("weight");
  const [sortAsc, setSortAsc] = useState(false);

  const settings = useMemo<SubframeSettings>(() => ({
    maxFwhm,
    maxEccentricity: maxEcc,
    minSnr,
    minStars,
    fwhmWeight,
    eccentricityWeight: eccWeight,
    snrWeight,
    noiseWeight,
  }), [maxFwhm, maxEcc, minSnr, minStars, fwhmWeight, eccWeight, snrWeight, noiseWeight]);

  const staleNote = subframeStaleNote(
    analysedWith !== null && subframeSettingsChanged(analysedWith, settings),
    analysedFiles !== null && subframeFilesChanged(analysedFiles, files),
  );
  const stale = staleNote !== null;

  const handleAnalyze = useCallback(async () => {
    if (files.length === 0) return;
    const analysing = [...files];
    setLoading(true);
    setError("");
    setResult(null);
    setAnalysedWith(null);
    setAnalysedFiles(null);
    setOverrides({});
    try {
      const res = await analyzeSubframes(analysing, settings);
      setResult(res);
      setAnalysedWith(settings);
      setAnalysedFiles(analysing);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [files, settings]);

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

  const effectiveSubframes = useMemo<SubframeRow[]>(() => {
    if (!result) return [];
    return result.subframes.map((s) => ({
      ...s,
      accepted: overrides[s.file_path] !== undefined ? overrides[s.file_path] : s.accepted,
    }));
  }, [result, overrides]);

  const sorted = useMemo(() => {
    const arr = [...effectiveSubframes];
    arr.sort((a, b) => {
      const va = a[sortBy];
      const vb = b[sortBy];
      if (typeof va === "boolean" || typeof vb === "boolean") {
        return sortAsc ? Number(va) - Number(vb) : Number(vb) - Number(va);
      }
      if (typeof va === "string" || typeof vb === "string") {
        const cmp = String(va).localeCompare(String(vb));
        return sortAsc ? cmp : -cmp;
      }
      const na = va ?? Number.NEGATIVE_INFINITY;
      const nb = vb ?? Number.NEGATIVE_INFINITY;
      return sortAsc ? na - nb : nb - na;
    });
    return arr;
  }, [effectiveSubframes, sortBy, sortAsc]);

  const handleApply = useCallback(() => {
    if (!onSelectionChange || stale) return;
    const accepted = effectiveSubframes.filter((s) => s.accepted).map((s) => s.file_path);
    const rejected = effectiveSubframes.filter((s) => !s.accepted).map((s) => s.file_path);
    onSelectionChange(
      accepted,
      rejected,
      Object.fromEntries(effectiveSubframes.filter((s) => s.accepted).map((s) => [s.file_path, s.weight])),
    );
  }, [effectiveSubframes, onSelectionChange, stale]);

  const handleSort = useCallback((col: SubframeColumn) => {
    if (sortBy === col) {
      setSortAsc((prev) => !prev);
    } else {
      setSortBy(col);
      setSortAsc(false);
    }
  }, [sortBy]);

  if (files.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 py-12 text-zinc-600 text-xs">
        Load FITS files to analyze subframe quality.
        <LeftOutLine text={leftOutLine} />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <SectionHeader icon={ICON} title="Subframe Selector" />

      <div className="text-[10px] text-zinc-500">
        {files.length} file{files.length !== 1 ? "s" : ""} available for analysis.
      </div>
      <LeftOutLine text={leftOutLine} />

      <div className="grid grid-cols-2 gap-2">
        <Slider label="Max FWHM" value={maxFwhm} min={1} max={20} step={0.5} accent="teal"
                format={(v) => v.toFixed(1)} onChange={setMaxFwhm} />
        <Slider label="Max Eccentricity" value={maxEcc} min={0.1} max={1.0} step={0.05} accent="teal"
                format={(v) => v.toFixed(2)} onChange={setMaxEcc} />
        <Slider label="Min Star SNR" value={minSnr} min={0} max={200} step={1} accent="teal"
                format={(v) => v.toFixed(0)} onChange={setMinSnr} />
        <Slider label="Min Stars" value={minStars} min={1} max={50} step={1} accent="teal"
                format={(v) => `${v}`} onChange={(v) => setMinStars(Math.round(v))} />
      </div>

      <div className="text-[10px] text-zinc-500 uppercase tracking-wider">Scoring weights</div>
      <div className="grid grid-cols-2 gap-2">
        <Slider label="FWHM weight" value={fwhmWeight} min={0} max={3} step={0.1} accent="teal"
                format={(v) => v.toFixed(1)} onChange={setFwhmWeight} />
        <Slider label="Ecc weight" value={eccWeight} min={0} max={3} step={0.1} accent="teal"
                format={(v) => v.toFixed(1)} onChange={setEccWeight} />
        <Slider label="SNR weight" value={snrWeight} min={0} max={3} step={0.1} accent="teal"
                format={(v) => v.toFixed(1)} onChange={setSnrWeight} />
        <Slider label="Noise weight" value={noiseWeight} min={0} max={3} step={0.1} accent="teal"
                format={(v) => v.toFixed(1)} onChange={setNoiseWeight} />
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
        <SubframeResultsView
          elapsedMs={result.elapsed_ms}
          rows={sorted}
          thresholds={analysedWith ?? settings}
          stale={stale}
          staleNote={staleNote ?? undefined}
          overrides={overrides}
          sortBy={sortBy}
          sortAsc={sortAsc}
          onSort={handleSort}
          onToggle={toggleOverride}
          onApply={onSelectionChange ? handleApply : undefined}
        />
      )}
    </div>
  );
}

interface SubframeResultsViewProps {
  elapsedMs: number;
  rows: SubframeRow[];
  thresholds: SubframeThresholds;
  stale: boolean;
  staleNote?: string;
  overrides: Record<string, boolean>;
  sortBy: SubframeColumn;
  sortAsc: boolean;
  onSort: (col: SubframeColumn) => void;
  onToggle: (path: string) => void;
  onApply?: () => void;
}

export function SubframeResultsView({
  elapsedMs,
  rows,
  thresholds,
  stale,
  staleNote = SUBFRAME_STALE_NOTE,
  overrides,
  sortBy,
  sortAsc,
  onSort,
  onToggle,
  onApply,
}: SubframeResultsViewProps) {
  const staleId = useId();
  const acceptedCount = rows.filter((s) => s.accepted).length;
  const rejectedCount = rows.length - acceptedCount;
  return (
    <>
      <div className="flex items-center justify-between text-[10px]">
        <span className="text-zinc-400">
          {elapsedMs}ms | {acceptedCount} accepted, {rejectedCount} rejected
        </span>
        {onApply && (
          <button
            type="button"
            onClick={onApply}
            disabled={stale}
            aria-describedby={stale ? staleId : undefined}
            className="px-2 py-1 rounded text-[9px] font-medium bg-teal-600/20 text-teal-300 hover:bg-teal-600/30 transition-colors disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-teal-600/20"
          >
            Apply Selection
          </button>
        )}
      </div>
      <div id={staleId} role="status" className="text-[10px] text-amber-300 empty:hidden">
        {stale ? staleNote : ""}
      </div>

      <div className={`overflow-x-auto rounded-lg border border-zinc-800/50 transition-opacity ${stale ? "opacity-60" : ""}`}>
        <table className="w-full text-[9px]">
          <thead>
          <tr className="bg-zinc-900/50 text-zinc-500 uppercase tracking-wider">
            <th className="px-2 py-1.5 text-left w-6"></th>
            <ThSort label="File" col="file_name" current={sortBy} asc={sortAsc} onClick={onSort} />
            <ThSort label="Stars" col="star_count" current={sortBy} asc={sortAsc} onClick={onSort} />
            <ThSort label="FWHM" col="median_fwhm" current={sortBy} asc={sortAsc} onClick={onSort} />
            <ThSort label="Ecc" col="median_eccentricity" current={sortBy} asc={sortAsc} onClick={onSort} />
            <ThSort label="SNR" col="median_snr" current={sortBy} asc={sortAsc} onClick={onSort} />
            <ThSort label="Noise" col="noise_sigma" current={sortBy} asc={sortAsc} onClick={onSort} />
            <ThSort label="Weight" col="weight" current={sortBy} asc={sortAsc} onClick={onSort} />
          </tr>
          </thead>
          <tbody>
          {rows.map((sub) => {
            const isOverridden = overrides[sub.file_path] !== undefined;
            return (
              <tr
                key={sub.file_path}
                className={`border-t border-zinc-800/30 transition-colors cursor-pointer hover:bg-zinc-800/30 ${
                  sub.accepted ? "" : "opacity-40"
                } ${isOverridden ? "ring-1 ring-inset ring-amber-500/20" : ""}`}
                onClick={() => onToggle(sub.file_path)}
              >
                <td className="px-2 py-1">
                  <SubframeStatusButton name={sub.file_name} accepted={sub.accepted} onToggle={() => onToggle(sub.file_path)} />
                </td>
                <td className="px-2 py-1 text-zinc-300 font-mono truncate max-w-[120px]" title={sub.file_name}>
                  {sub.file_name}
                </td>
                <td className="px-2 py-1 text-right font-mono text-zinc-400">{sub.star_count}</td>
                <td className={`px-2 py-1 text-right font-mono ${sub.median_fwhm > thresholds.maxFwhm ? "text-red-400" : "text-zinc-300"}`}>
                  {sub.median_fwhm.toFixed(2)}
                </td>
                <td className={`px-2 py-1 text-right font-mono ${sub.median_eccentricity > thresholds.maxEccentricity ? "text-red-400" : "text-zinc-300"}`}>
                  {sub.median_eccentricity.toFixed(3)}
                </td>
                <td className={`px-2 py-1 text-right font-mono ${sub.median_snr < thresholds.minSnr ? "text-red-400" : "text-zinc-300"}`}>
                  {sub.median_snr.toFixed(1)}
                </td>
                <td
                  className="px-2 py-1 text-right font-mono text-zinc-300"
                  title={sub.noise_fraction != null ? `k-sigma MRS noise, ${(sub.noise_fraction * 100).toFixed(0)}% of pixels used` : "k-sigma MRS noise"}
                >
                  {formatNoise(sub.noise_sigma)}
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
  );
}

export function SubframeStatusButton({ name, accepted, onToggle }: { name: string; accepted: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={accepted}
      aria-label={`Frame ${name}: ${accepted ? "accepted, press to reject" : "rejected, press to accept"}`}
      onClick={(e) => {
        e.stopPropagation();
        onToggle();
      }}
      className="flex items-center rounded p-0.5 cursor-pointer"
    >
      {accepted ? <Check size={10} className="text-emerald-400" /> : <X size={10} className="text-red-400" />}
    </button>
  );
}

export function ThSort({
                  label,
                  col,
                  current,
                  asc,
                  onClick,
                }: {
  label: string;
  col: SubframeColumn;
  current: SubframeColumn;
  asc: boolean;
  onClick: (col: SubframeColumn) => void;
}) {
  const active = current === col;
  return (
    <th aria-sort={active ? (asc ? "ascending" : "descending") : "none"} className={`px-2 py-1.5 text-right ${active ? "text-teal-400" : ""}`}>
      <button
        type="button"
        onClick={() => onClick(col)}
        className="uppercase cursor-pointer select-none hover:text-zinc-300 transition-colors"
      >
        {label}
        {active && <span aria-hidden="true" className="ml-0.5">{asc ? "▲" : "▼"}</span>}
      </button>
    </th>
  );
}
