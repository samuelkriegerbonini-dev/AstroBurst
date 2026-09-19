import { useState, useEffect, useRef, useCallback, useMemo, memo } from "react";
import { Grid3X3, Copy, Check, Loader2 } from "lucide-react";
import { computeStatistics, computeStatisticsComposite } from "../../services/statistics";
import type {
  ChannelStatistics,
  ChannelStatisticsBody,
  DataRange,
  StatisticsUnit,
} from "../../shared/types/statistics";
import type { RegionShape } from "../../shared/types/regions";
import { useDqContext } from "../../context/PreviewContext";
import { useCompositePreview } from "../../context/CompositeContext";
import { useRegionKey } from "../../hooks/useRegionKey";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { Toggle, RunButton, ErrorAlert } from "../ui";
import {
  STATISTIC_ROWS,
  UNIT_LABELS,
  convertScaleValue,
  convertStatistics,
  fitsSixteenBit,
  formatStatistic,
  statisticsToCsv,
} from "../../utils/statisticsUnits";

interface StatisticsPanelProps {
  filePath: string | null;
}

interface ChannelResult {
  label: string;
  body: ChannelStatisticsBody;
}

interface PanelResult {
  channels: ChannelResult[];
  unit: string | null;
  masked: boolean;
  dqExcluded: number | null;
  elapsedMs: number;
  regionKind: RegionShape["shape"] | null;
}

const UNITS: readonly StatisticsUnit[] = ["raw", "normalized", "16bit"];
const COPIED_FEEDBACK_MS = 1500;

function channelRange(body: ChannelStatisticsBody): DataRange {
  return { min: body.data_min, max: body.data_max };
}

function StatisticsPanel({ filePath }: StatisticsPanelProps) {
  const { excludeDq } = useDqContext();
  const { isShowingComposite } = useCompositePreview();
  const regionKey = useRegionKey();
  const doc = useRegionDoc(regionKey);
  const selectedShape = useMemo<RegionShape | null>(
    () => doc.regions.find((r) => r.id === doc.selectedId)?.shape ?? null,
    [doc.regions, doc.selectedId],
  );

  const [unit, setUnit] = useState<StatisticsUnit>("raw");
  const [useRegion, setUseRegion] = useState(false);
  const [noise, setNoise] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<PanelResult | null>(null);
  const [copied, setCopied] = useState(false);

  const run = useCallback(async () => {
    if (!filePath && !isShowingComposite) return;
    setLoading(true);
    setError(null);
    try {
      if (isShowingComposite) {
        const res = await computeStatisticsComposite(noise);
        setResult({
          channels: [
            { label: "R", body: res.r },
            { label: "G", body: res.g },
            { label: "B", body: res.b },
          ],
          unit: null,
          masked: false,
          dqExcluded: null,
          elapsedMs: res.elapsed_ms,
          regionKind: null,
        });
      } else if (filePath) {
        const region = useRegion ? selectedShape : null;
        const res = await computeStatistics(filePath, { excludeDq, region, noise });
        setResult({
          channels: [{ label: "K", body: res }],
          unit: res.unit,
          masked: res.masked,
          dqExcluded: res.dq_excluded,
          elapsedMs: res.elapsed_ms,
          regionKind: region?.shape ?? null,
        });
      }
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [filePath, isShowingComposite, noise, useRegion, selectedShape, excludeDq]);

  const runRef = useRef(run);
  runRef.current = run;

  useEffect(() => {
    setResult(null);
    setError(null);
    if (filePath || isShowingComposite) void runRef.current();
  }, [filePath, isShowingComposite]);

  const sixteenBitAvailable = useMemo(
    () => result !== null && result.channels.every((c) => fitsSixteenBit(channelRange(c.body))),
    [result],
  );

  useEffect(() => {
    if (unit === "16bit" && result && !sixteenBitAvailable) setUnit("raw");
  }, [unit, result, sixteenBitAvailable]);

  const converted = useMemo<ChannelStatistics[]>(
    () => (result ? result.channels.map((c) => convertStatistics(c.body.statistics, unit, channelRange(c.body))) : []),
    [result, unit],
  );

  const noiseSigmas = useMemo<number[]>(
    () =>
      result
        ? result.channels.map((c) => {
            const sigma = c.body.noise?.sigma;
            return sigma == null ? NaN : convertScaleValue(sigma, unit, channelRange(c.body));
          })
        : [],
    [result, unit],
  );

  const handleCopy = useCallback(() => {
    if (!result) return;
    const columns = result.channels.map((c, i) => ({ label: c.label, stats: converted[i] }));
    const lines = [statisticsToCsv(columns, "raw")];
    if (result.channels.some((c) => c.body.noise)) {
      lines.push(["noise_sigma", ...noiseSigmas.map((s) => (Number.isFinite(s) ? String(s) : ""))].join(","));
      lines.push(["noise_fraction", ...result.channels.map((c) => String(c.body.noise?.fraction ?? ""))].join(","));
    }
    lines.push(`unit,${unit}`);
    navigator.clipboard?.writeText(lines.join("\n"));
    setCopied(true);
    window.setTimeout(() => setCopied(false), COPIED_FEEDBACK_MS);
  }, [result, converted, noiseSigmas, unit]);

  const hasTarget = Boolean(filePath) || isShowingComposite;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Grid3X3 size={12} className="text-sky-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Statistics</span>
        </div>
        {loading && <Loader2 size={12} className="animate-spin text-sky-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-xs text-zinc-400">Units</span>
          <div className="flex rounded overflow-hidden border border-zinc-800">
            {UNITS.map((u) => {
              const disabled = u === "16bit" && !sixteenBitAvailable;
              const active = unit === u;
              return (
                <button
                  key={u}
                  type="button"
                  disabled={disabled}
                  onClick={() => setUnit(u)}
                  title={disabled ? "16-bit units need data inside [0, 1]" : `Show values as ${UNIT_LABELS[u]}`}
                  className={`px-2 py-0.5 text-[9px] font-mono transition-colors ${
                    active ? "bg-sky-600/30 text-sky-200" : "text-zinc-400 hover:text-zinc-200"
                  } ${disabled ? "opacity-40 cursor-not-allowed" : ""}`}
                >
                  {UNIT_LABELS[u]}
                </button>
              );
            })}
          </div>
        </div>

        <Toggle
          label={selectedShape ? `Selected region only (${selectedShape.shape})` : "Selected region only"}
          checked={useRegion}
          disabled={!selectedShape || isShowingComposite}
          accent="sky"
          onChange={setUseRegion}
        />
        <Toggle label="Evaluate noise (k-sigma MRS)" checked={noise} accent="sky" onChange={setNoise} />

        <RunButton
          label="Compute"
          runningLabel="Computing..."
          running={loading}
          disabled={!hasTarget}
          accent="sky"
          small
          onClick={() => void run()}
        />

        <ErrorAlert message={error} />

        {result && (
          <>
            <div className="flex items-center justify-between text-[9px] text-zinc-500 font-mono">
              <span>
                {result.elapsedMs}ms
                {result.regionKind ? ` | ${result.regionKind} region` : " | full frame"}
                {result.masked ? ` | DQ excluded: ${result.dqExcluded ?? 0}` : ""}
                {result.unit ? ` | BUNIT ${result.unit}` : ""}
              </span>
              <button
                type="button"
                onClick={handleCopy}
                className="flex items-center gap-1 px-1.5 py-0.5 rounded text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/60 transition-colors"
                title="Copy the table as CSV"
              >
                {copied ? <Check size={10} className="text-emerald-400" /> : <Copy size={10} />}
                <span>{copied ? "Copied" : "CSV"}</span>
              </button>
            </div>

            <div className="overflow-x-auto rounded-lg border border-zinc-800/50">
              <table className="w-full text-[9px]">
                <thead>
                  <tr className="bg-zinc-900/50 text-zinc-500 uppercase tracking-wider">
                    <th className="px-2 py-1 text-left">Statistic</th>
                    {result.channels.map((c) => (
                      <th key={c.label} className="px-2 py-1 text-right">
                        {c.label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {STATISTIC_ROWS.map((row) => (
                    <tr key={row.key} className="border-t border-zinc-800/30">
                      <td className="px-2 py-0.5 text-zinc-400">{row.label}</td>
                      {converted.map((s, i) => (
                        <td key={result.channels[i].label} className="px-2 py-0.5 text-right font-mono text-zinc-200">
                          {formatStatistic(s[row.key], row.kind, unit)}
                        </td>
                      ))}
                    </tr>
                  ))}
                  {result.channels.some((c) => c.body.noise) && (
                    <>
                      <tr className="border-t border-zinc-700/60">
                        <td className="px-2 py-0.5 text-sky-300">noise sigma</td>
                        {noiseSigmas.map((s, i) => (
                          <td key={result.channels[i].label} className="px-2 py-0.5 text-right font-mono text-sky-200">
                            {formatStatistic(s, "scale", unit)}
                          </td>
                        ))}
                      </tr>
                      <tr className="border-t border-zinc-800/30">
                        <td className="px-2 py-0.5 text-sky-300">noise pixels</td>
                        {result.channels.map((c) => (
                          <td key={c.label} className="px-2 py-0.5 text-right font-mono text-sky-200">
                            {c.body.noise ? formatStatistic(c.body.noise.fraction, "fraction", unit) : "--"}
                          </td>
                        ))}
                      </tr>
                    </>
                  )}
                </tbody>
              </table>
            </div>
          </>
        )}

        {!result && !error && !loading && (
          <div className="text-[10px] text-zinc-600">
            Exact per-pixel statistics (no histogram approximation, negatives and zeros included). avgDev, MAD
            and sqrt(BWMV) are robust scale estimates; the noise row uses the k-sigma multiresolution estimator.
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(StatisticsPanel);
