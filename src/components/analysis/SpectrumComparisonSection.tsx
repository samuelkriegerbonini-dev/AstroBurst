import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Layers } from "lucide-react";
import ProfilePlot from "../regions/ProfilePlot";
import { getCubeSpectrum, getCubeSpectrumRegion } from "../../services/cube";
import { spectrumComparisonStore, useSpectrumComparison } from "../../hooks/useSpectrumComparisonStore";
import { measurementLog, spectrumCompareEntry } from "../../utils/measurementLog";
import type { ContinuumWindows } from "../../shared/types/cube";
import type {
  CorrectionFrame,
  RadialVelocityCorrectionResult,
  SpectralAxisInfo,
  SpectralAxisMode,
  VelocityConvention,
} from "../../shared/types/spectral";
import type { RegionDoc } from "../../utils/regionStore";
import {
  COMPARE_DEBOUNCE_MS,
  NORMALISE_MODES,
  comparisonCsv,
  comparisonCsvFileName,
  comparisonSeries,
  entriesFrom,
  limitCandidates,
  pruneEntries,
  type ComparisonCsvInput,
  type NormaliseMode,
} from "../../utils/spectrumCompare";
import {
  comparisonAxis,
  comparisonCandidates,
  fileBaseName,
  linkedAnnulus,
  saveCsvDialog,
  vacuumUmAxis,
  type SpectrumView,
} from "../../utils/spectrumExport";
import { windowsAreValid } from "../../utils/spectrumRange";

interface SpectrumComparisonSectionProps {
  filePath: string;
  regionDoc: RegionDoc;
  axis: SpectralAxisInfo | null;
  mode: SpectralAxisMode;
  restUm: number | null;
  convention: VelocityConvention;
  correction: CorrectionFrame;
  correctionResult: RadialVelocityCorrectionResult | null;
  pixelCoord: { x: number; y: number } | null;
  view: SpectrumView;
  bunit: string | null;
  channelCount: number;
  windows: ContinuumWindows | null;
}

const ACCENT = "#22d3ee";
const NOTICE_MS = 2500;
const NO_CANDIDATE_HINT = "draw an included circle, box, ellipse or polygon region, or click a pixel";
const WINDOW_HINT = "brush continuum windows A and B first";
const NOTHING_HINT = "nothing to compare yet";
const STALE_TITLE = "waiting for the refetch";
const SELECT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 focus:border-violet-500/50 disabled:opacity-40";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] font-mono text-zinc-200 focus:border-violet-500/50 w-16 disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const SMALL_BUTTON_CLASS = "px-2 py-0.5 rounded border transition-colors text-zinc-500 hover:text-zinc-300 disabled:opacity-40";

function SpectrumComparisonSection({
  filePath,
  regionDoc,
  axis,
  mode,
  restUm,
  convention,
  correction,
  correctionResult,
  pixelCoord,
  view,
  bunit,
  channelCount,
  windows,
}: SpectrumComparisonSectionProps) {
  const doc = useSpectrumComparison(filePath);
  const normaliseId = useId();
  const stepId = useId();
  const [saveBusy, setSaveBusy] = useState(false);
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);
  const noticeTimerRef = useRef<number | null>(null);

  const regions = regionDoc.regions;
  const limited = useMemo(() => limitCandidates(comparisonCandidates(regions), regions), [regions]);
  const candidates = limited.compared;
  const pixelKey = useMemo(
    () => (doc.includePixel && pixelCoord ? `${pixelCoord.x},${pixelCoord.y}` : null),
    [doc.includePixel, pixelCoord],
  );
  const axisCol = useMemo(
    () => comparisonAxis(axis, mode, restUm, convention, correction, correctionResult, channelCount),
    [axis, mode, restUm, convention, correction, correctionResult, channelCount],
  );
  const axisValues = useMemo(
    () => axisCol.values ?? Array.from({ length: channelCount }, (_, i) => i),
    [axisCol, channelCount],
  );
  const vacuum = useMemo(() => vacuumUmAxis(axis, restUm, convention, channelCount), [axis, restUm, convention, channelCount]);
  const entries = useMemo(() => pruneEntries(doc.entries, regions, channelCount), [doc.entries, regions, channelCount]);
  const hidden = useMemo(() => new Set(doc.hidden), [doc.hidden]);
  const windowsValid = windowsAreValid(windows, channelCount);
  const usableWindows = windowsValid ? windows : null;
  const result = useMemo(
    () => comparisonSeries(entries, hidden, axisValues, view, doc.normalise, usableWindows, doc.offsetStep, bunit, limited.omitted),
    [entries, hidden, axisValues, view, doc.normalise, usableWindows, doc.offsetStep, bunit, limited.omitted],
  );
  const stale = doc.loading || doc.regionVersion !== regionDoc.version || doc.pixelKey !== pixelKey;

  const showNotice = useCallback((text: string, error: boolean) => {
    if (noticeTimerRef.current !== null) window.clearTimeout(noticeTimerRef.current);
    setNotice({ text, error });
    noticeTimerRef.current = window.setTimeout(() => setNotice(null), NOTICE_MS);
  }, []);

  useEffect(() => {
    return () => {
      if (noticeTimerRef.current !== null) window.clearTimeout(noticeTimerRef.current);
    };
  }, []);

  const runFetch = useCallback(async () => {
    const seq = spectrumComparisonStore.begin(filePath, regionDoc.version, pixelKey);
    const pixel = pixelKey !== null && pixelCoord ? pixelCoord : null;
    const regionJobs = Promise.allSettled(
      candidates.map((region) => getCubeSpectrumRegion(filePath, region.shape, linkedAnnulus(regions, region))),
    );
    const pixelJob = pixel ? Promise.allSettled([getCubeSpectrum(filePath, pixel.x, pixel.y)]) : null;
    const settled = await regionJobs;
    const pixelSettled = pixelJob ? (await pixelJob)[0] : null;
    const pixelEntry = pixel && pixelSettled ? { coord: pixel, result: pixelSettled } : null;
    spectrumComparisonStore.commit(filePath, seq, entriesFrom(candidates, regions, settled, pixelEntry));
  }, [filePath, regionDoc.version, pixelKey, pixelCoord, candidates, regions]);

  useEffect(() => {
    if (!doc.enabled) return;
    if (doc.loading) {
      if (doc.pendingRegionVersion === regionDoc.version && doc.pendingPixelKey === pixelKey) return;
      spectrumComparisonStore.invalidate(filePath);
      return;
    }
    if (doc.regionVersion === regionDoc.version && doc.pixelKey === pixelKey) return;
    const timer = window.setTimeout(runFetch, COMPARE_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [
    filePath,
    doc.enabled,
    doc.loading,
    doc.regionVersion,
    doc.pixelKey,
    doc.pendingRegionVersion,
    doc.pendingPixelKey,
    regionDoc.version,
    pixelKey,
    runFetch,
  ]);

  useEffect(() => {
    return () => spectrumComparisonStore.invalidate(filePath);
  }, [filePath]);

  const toggleHidden = useCallback(
    (id: string) => {
      const next = doc.hidden.includes(id) ? doc.hidden.filter((h) => h !== id) : [...doc.hidden, id];
      spectrumComparisonStore.patch(filePath, { hidden: next });
    },
    [filePath, doc.hidden],
  );

  const handleSave = useCallback(async () => {
    if (saveBusy) return;
    setSaveBusy(true);
    try {
      const csvInput: ComparisonCsvInput = {
        fileName: fileBaseName(filePath),
        view,
        mode: doc.normalise,
        windows: usableWindows,
        axis: axisCol,
        axisKnown: axis !== null,
        vacuum,
        channelCount,
        entries,
        result,
        specsys: axis?.specsys ?? null,
        axisMode: mode,
        correction,
        correctionResult,
        exportedAtUtc: new Date().toISOString(),
      };
      const csv = comparisonCsv(csvInput);
      const path = await saveCsvDialog(csv, comparisonCsvFileName(filePath), "Save comparison CSV");
      if (path) showNotice(`saved: ${path}`, false);
      if (path) measurementLog.append(spectrumCompareEntry(filePath, csvInput, path));
    } catch (e) {
      showNotice(`export failed: ${e instanceof Error ? e.message : String(e)}`, true);
    } finally {
      setSaveBusy(false);
    }
  }, [saveBusy, filePath, view, doc.normalise, usableWindows, axisCol, axis, vacuum, channelCount, entries, result, mode, correction, correctionResult, showNotice]);

  const candidateCount = candidates.length;
  const skippedCount = limited.omitted.length;
  const regionHint =
    skippedCount > 0
      ? `first ${candidateCount} of ${candidateCount + skippedCount} regions`
      : `${candidateCount} region${candidateCount === 1 ? "" : "s"}`;
  const enableHint = `${regionHint}${pixelCoord ? " + pixel" : ""}`;
  const canEnable = candidateCount > 0 || pixelCoord !== null;
  const nothingToCompare = candidateCount === 0 && !pixelCoord;

  return (
    <div className="w-full self-stretch px-3 pb-2 flex flex-col gap-2" style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}>
      <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
        <span className="flex items-center gap-1.5">
          <Layers size={12} style={{ color: ACCENT }} />
          <span className="text-[11px] font-semibold text-zinc-400 uppercase tracking-wider">Compare</span>
        </span>
        {!doc.enabled ? (
          <>
            <button
              onClick={() => spectrumComparisonStore.patch(filePath, { enabled: true })}
              disabled={!canEnable}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all disabled:opacity-40"
              style={{ background: "rgba(34,211,238,0.08)", border: "1px solid rgba(34,211,238,0.2)", color: ACCENT }}
            >
              Compare spectra
            </button>
            <span className="text-zinc-600">{canEnable ? enableHint : NO_CANDIDATE_HINT}</span>
          </>
        ) : (
          <>
            <button onClick={() => void runFetch()} disabled={doc.loading} className={SMALL_BUTTON_CLASS} style={{ borderColor: "var(--ab-border)" }}>
              Refresh
            </button>
            <button
              onClick={() => spectrumComparisonStore.patch(filePath, { enabled: false })}
              className={SMALL_BUTTON_CLASS}
              style={{ borderColor: "var(--ab-border)" }}
            >
              Close
            </button>
            {doc.loading && (
              <div className="w-3 h-3 rounded-full animate-spin" style={{ border: "1.5px solid transparent", borderTopColor: ACCENT }} />
            )}
            {stale && <span className="text-zinc-600">refreshing…</span>}
          </>
        )}
      </div>

      {doc.enabled && (
        <>
          {nothingToCompare && entries.length === 0 && <span className="text-[10px] font-mono text-zinc-600">{NOTHING_HINT}</span>}

          {entries.length > 0 && (
            <div className="flex flex-col gap-0.5 text-[10px] font-mono">
              {entries.map((entry) => (
                <label key={entry.id} className="flex items-center gap-1.5 cursor-pointer">
                  <input type="checkbox" checked={!hidden.has(entry.id)} onChange={() => toggleHidden(entry.id)} />
                  <span className="inline-block w-2.5 h-2.5 rounded-sm shrink-0" style={{ background: entry.color }} />
                  <span className="text-zinc-300 truncate">{entry.label}</span>
                  {entry.region && (
                    <span className="text-zinc-600">
                      {entry.region.npix.toFixed(1)} px{entry.region.bg_subtracted ? ` · bg ${entry.region.n_bg} px` : ""}
                    </span>
                  )}
                  {entry.error !== null && <span className="text-red-400/80 truncate">{entry.error}</span>}
                </label>
              ))}
            </div>
          )}

          <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
            {pixelCoord && (
              <label className="flex items-center gap-1 text-zinc-500 cursor-pointer">
                <input
                  type="checkbox"
                  checked={doc.includePixel}
                  onChange={(e) => spectrumComparisonStore.patch(filePath, { includePixel: e.target.checked })}
                />
                pixel ({pixelCoord.x}, {pixelCoord.y})
              </label>
            )}
            <label htmlFor={normaliseId} className={LABEL_CLASS}>
              Normalise
            </label>
            <select
              id={normaliseId}
              value={doc.normalise}
              onChange={(e) => spectrumComparisonStore.patch(filePath, { normalise: e.target.value as NormaliseMode })}
              className={SELECT_CLASS}
            >
              {NORMALISE_MODES.map((m) => (
                <option key={m} value={m} disabled={m === "window" && !windowsValid} title={m === "window" && !windowsValid ? WINDOW_HINT : undefined}>
                  {m}
                </option>
              ))}
            </select>
            {doc.normalise === "window" && !windowsValid && <span className="text-zinc-600">{WINDOW_HINT}</span>}
            {doc.normalise === "offset" && (
              <>
                <label htmlFor={stepId} className={LABEL_CLASS}>
                  step
                </label>
                <input
                  id={stepId}
                  type="number"
                  value={doc.offsetStep ?? ""}
                  placeholder={result.step !== null ? String(result.step) : "auto"}
                  onChange={(e) => {
                    const value = Number(e.target.value);
                    spectrumComparisonStore.patch(filePath, {
                      offsetStep: e.target.value.trim() === "" || !Number.isFinite(value) ? null : value,
                    });
                  }}
                  className={INPUT_CLASS}
                />
              </>
            )}
          </div>

          {result.series.length > 0 && (
            <ProfilePlot
              key={filePath}
              series={result.series}
              xLabel={axisCol.label}
              yLabel={result.yLabel}
              toolbar
              logToggle
              csvName="spectra"
            />
          )}

          {result.omitted.length > 0 && (
            <div className="flex flex-col gap-0.5 text-[10px] font-mono text-zinc-600">
              {result.omitted.map((omission) => (
                <span key={omission.id} className="break-words">
                  {omission.label}: {omission.reason}
                </span>
              ))}
            </div>
          )}

          <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
            <button
              onClick={() => void handleSave()}
              disabled={saveBusy || stale || result.plotted.length === 0}
              title={stale ? STALE_TITLE : undefined}
              className={SMALL_BUTTON_CLASS}
              style={{ borderColor: "var(--ab-border)" }}
            >
              Save CSV
            </button>
            {notice && <span className={notice.error ? "text-red-400/80 truncate" : "text-zinc-600 truncate"}>{notice.text}</span>}
          </div>
        </>
      )}
    </div>
  );
}

export default memo(SpectrumComparisonSection);
