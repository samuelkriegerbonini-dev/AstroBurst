import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BarChart3, CheckCircle2, XCircle } from "lucide-react";
import ProfilePlot, { type ProfileSeries } from "../regions/ProfilePlot";
import { ErrorAlert, RunButton } from "../ui";
import { compareWithRate } from "../../services/ramp";
import {
  ampFaintRows,
  compareVerdict,
  compareVerdictTitle,
  formatCompareBin,
  formatConfusion,
  formatShiftCheck,
  histogramSeries,
  histogramSummary,
  parseUncalRows,
  quickFlaggedShare,
} from "../../utils/rampLabels";
import type { Histogram, RateComparison } from "../../shared/types/ramp";
import type { CubeResult } from "./SpectroscopyPanel";

interface RampComparePanelProps {
  qslopePath: string;
  ratePath: string;
  dimensions: [number, number] | null;
  onCubeResult?: (result: CubeResult) => void;
}

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 font-mono focus:border-violet-500/50 w-16 disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const SMALL_BUTTON_CLASS =
  "px-2 py-0.5 rounded text-[9px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40";
const HIST_HEIGHT = 120;
const REL_HIST_COLOR = "rgba(45,212,191,0.9)";
const DELTA_HIST_COLOR = "rgba(167,139,250,0.9)";
const RATIO_LABEL = "Quick / rate ratio";
const FAINT_BIN_LABELS = ["[-inf,0.05)", "[0.05,0.3)"];
const REL_HIST_LABEL = "delta / rate";
const DELTA_HIST_LABEL = "delta DN/s";

function fileName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function signedDelta(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return "n/a";
  return `${v >= 0 ? "+" : ""}${v.toFixed(4)}`;
}

function histogramPlot(h: Histogram, color: string, label: string): ProfileSeries[] {
  const { x, y } = histogramSeries(h);
  return x.length > 0 ? [{ x, y, color, label, mode: "line" }] : [];
}

function RampComparePanel({ qslopePath, ratePath, dimensions, onCubeResult }: RampComparePanelProps) {
  const [y0Text, setY0Text] = useState("");
  const [y1Text, setY1Text] = useState("");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cmp, setCmp] = useState<RateComparison | null>(null);
  const seqRef = useRef(0);

  useEffect(() => {
    const seq = seqRef;
    return () => {
      seq.current++;
    };
  }, []);

  const rows = useMemo(() => parseUncalRows(y0Text, y1Text), [y0Text, y1Text]);

  const handleRun = useCallback(async () => {
    if (rows.error) return;
    const seq = ++seqRef.current;
    setRunning(true);
    setError(null);
    setCmp(null);
    try {
      const res = await compareWithRate(qslopePath, ratePath, undefined, rows.rows);
      if (seq === seqRef.current) setCmp(res);
    } catch (e) {
      if (seq === seqRef.current) {
        setCmp(null);
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (seq === seqRef.current) setRunning(false);
    }
  }, [qslopePath, ratePath, rows.error, rows.rows]);

  const verdict = useMemo(() => (cmp ? compareVerdict(cmp) : null), [cmp]);
  const ampRows = useMemo(() => (cmp ? ampFaintRows(cmp) : []), [cmp]);
  const relSeries = useMemo(() => (cmp ? histogramPlot(cmp.rel_hist, REL_HIST_COLOR, REL_HIST_LABEL) : []), [cmp]);
  const deltaSeries = useMemo(() => (cmp ? histogramPlot(cmp.delta_hist, DELTA_HIST_COLOR, DELTA_HIST_LABEL) : []), [cmp]);

  const publishRatio = useCallback(() => {
    if (!cmp?.ratioPreviewUrl) return;
    onCubeResult?.({ label: RATIO_LABEL, previewUrl: cmp.ratioPreviewUrl, fitsPath: cmp.ratio_fits_path, dimensions });
  }, [cmp, dimensions, onCubeResult]);

  return (
    <div className="rounded-lg border border-zinc-800/60 bg-zinc-950/40 p-2.5 space-y-2">
      <div className="flex items-center gap-2">
        <BarChart3 size={11} className="text-teal-400" />
        <span className="text-[10px] font-semibold text-zinc-300 uppercase tracking-wider">Compare with rate</span>
      </div>
      <div className="text-[9px] font-mono text-zinc-500 break-all" title={ratePath}>
        rate: {fileName(ratePath)}
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <label className="flex items-center gap-1" title="Optional uncal row window [y0, y1) to reproduce a band; empty = full frame">
          <span className={LABEL_CLASS}>uncal y0</span>
          <input type="text" inputMode="numeric" value={y0Text} onChange={(e) => setY0Text(e.target.value)} className={INPUT_CLASS} />
        </label>
        <label className="flex items-center gap-1">
          <span className={LABEL_CLASS}>y1</span>
          <input type="text" inputMode="numeric" value={y1Text} onChange={(e) => setY1Text(e.target.value)} className={INPUT_CLASS} />
        </label>
      </div>
      {rows.error && <div className="text-[9px] text-amber-300">{rows.error}</div>}
      <RunButton label="Compare" runningLabel="Comparing..." running={running} disabled={rows.error !== null} small onClick={handleRun} />
      <ErrorAlert message={error} />

      {cmp && verdict && (
        <div className="space-y-2">
          <div
            role="status"
            className={`rounded px-2 py-1.5 text-[10px] border ${
              verdict.passed
                ? "text-emerald-300 bg-emerald-900/20 border-emerald-700/30"
                : "text-red-300 bg-red-900/20 border-red-700/30"
            }`}
          >
            <div className="flex items-center gap-1.5 font-semibold">
              {verdict.passed ? <CheckCircle2 size={11} /> : <XCircle size={11} />}
              {compareVerdictTitle(cmp, verdict)}
            </div>
            {verdict.reasons.length > 0 && (
              <ul className="mt-1 list-disc pl-4 font-mono text-[9px] space-y-0.5">
                {verdict.reasons.map((r) => (
                  <li key={r}>{r}</li>
                ))}
              </ul>
            )}
          </div>

          <div className="text-[10px] font-mono text-zinc-300">{formatShiftCheck(cmp.zero_shift)}</div>
          <div className="text-[9px] font-mono text-zinc-500">
            good pixels {cmp.good_pixels}, quick-flagged among them {quickFlaggedShare(cmp)}
            {cmp.science_rows ? `, science rows ${cmp.science_rows[0]}..${cmp.science_rows[1]}` : ", full frame"}
            {`, ${cmp.elapsed_ms} ms`}
          </div>

          <div>
            <div className={LABEL_CLASS}>Bins (quick - official, DN/s)</div>
            <div className="mt-0.5 overflow-x-auto">
              {cmp.bins.map((b) => (
                <div key={formatCompareBin(b)} className="text-[9px] font-mono text-zinc-300 whitespace-pre">
                  {formatCompareBin(b)}
                </div>
              ))}
            </div>
          </div>

          {ampRows.length > 0 && (
            <div>
              <div className={LABEL_CLASS}>Faint median delta per amplifier (G2)</div>
              <table className="mt-0.5 text-[9px] font-mono text-zinc-300">
                <thead>
                  <tr className="text-zinc-500">
                    <th className="text-left pr-3 font-normal">amp</th>
                    <th className="text-left pr-3 font-normal">science rows</th>
                    {FAINT_BIN_LABELS.map((l) => (
                      <th key={l} className="text-right pr-3 font-normal">
                        {l}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {ampRows.map((r) => (
                    <tr key={r.amplifier}>
                      <td className="pr-3">{r.amplifier}</td>
                      <td className="pr-3">{r.rows}</td>
                      {r.deltas.map((d, i) => (
                        <td key={FAINT_BIN_LABELS[i]} className="text-right pr-3">
                          {signedDelta(d)}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          {relSeries.length > 0 && (
            <div>
              <div className={LABEL_CLASS}>delta / rate, official &gt;= 1 DN/s</div>
              <div role="img" aria-label={histogramSummary(cmp.rel_hist, REL_HIST_LABEL)}>
                <ProfilePlot series={relSeries} xLabel="delta / rate" yLabel="pixels" height={HIST_HEIGHT} logToggle={false} />
              </div>
            </div>
          )}
          {deltaSeries.length > 0 && (
            <div>
              <div className={LABEL_CLASS}>delta, official &lt; 0.05 DN/s</div>
              <div role="img" aria-label={histogramSummary(cmp.delta_hist, DELTA_HIST_LABEL)}>
                <ProfilePlot series={deltaSeries} xLabel="delta (DN/s)" yLabel="pixels" height={HIST_HEIGHT} logToggle={false} />
              </div>
            </div>
          )}

          <div className="space-y-0.5 text-[9px] font-mono text-zinc-400">
            <div>JUMP_DET (report only): {formatConfusion(cmp.jump)}</div>
            <div>
              SATURATED, rule {cmp.saturated_rule}: {formatConfusion(cmp.saturated)}
            </div>
            <div>SATURATED, any group: {formatConfusion(cmp.saturated_any_group)}</div>
          </div>

          <button type="button" className={SMALL_BUTTON_CLASS} disabled={!cmp.ratioPreviewUrl || !onCubeResult} onClick={publishRatio}>
            Show ratio image
          </button>
        </div>
      )}
    </div>
  );
}

export default memo(RampComparePanel);
