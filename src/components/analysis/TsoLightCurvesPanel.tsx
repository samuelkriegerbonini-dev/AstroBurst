import { lazy, memo, Suspense, useEffect, useId, useMemo, useRef, useSyncExternalStore } from "react";
import { Clock, Loader2 } from "lucide-react";
import { ErrorAlert, WarningList } from "../ui";
import { fileStore } from "../../hooks/useFileStore";
import { tsoStore, useTsoChosen, useTsoSeriesIndex, useTsoSet, type TsoSectionProps } from "../../hooks/useTsoStore";
import { isSpectrumSeries } from "../../utils/fileKinds";
import { tsoGroupKey } from "../../utils/tsoLightCurves";
import {
  completenessText,
  layoutText,
  levelText,
  seriesFactsText,
  seriesOptionText,
  setPaths,
  siblingSegments,
  type LoadedSeriesFile,
} from "../../utils/tsoSet";
import { fileBaseName } from "../../utils/values";

const TsoCurvesSection = lazy(() => import("./TsoCurvesSection"));
const TsoWhtltSection = lazy(() => import("./TsoWhtltSection"));
const TsoDepthSpectrum = lazy(() => import("./TsoDepthSpectrum"));

const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const LINE_CLASS = "text-[10px] font-mono text-zinc-400 break-words";
const INCOMPLETE_CLASS = "text-[10px] font-mono text-amber-300 break-words";
const CAPTION_CLASS = "text-[9px] text-zinc-500";

export interface TsoLightCurvesPanelProps {
  filePath: string | null;
  fileKey: string | null;
}

function TsoLightCurvesPanel({ filePath, fileKey }: TsoLightCurvesPanelProps) {
  const seriesId = useId();
  const files = useSyncExternalStore(fileStore.subscribe, fileStore.getFiles);
  const loaded = useMemo<LoadedSeriesFile[]>(
    () => files.filter((file) => isSpectrumSeries(file)).map((file) => ({ path: file.path, name: file.name })),
    [files],
  );
  const siblings = useMemo(() => {
    if (filePath === null) return [];
    const activeName = loaded.find((file) => file.path === filePath)?.name ?? fileBaseName(filePath);
    return siblingSegments(filePath, activeName, loaded);
  }, [filePath, loaded]);
  const stored = useTsoChosen(fileKey);
  const chosen = useMemo(() => siblings.filter((sibling) => stored.includes(sibling.path)).map((sibling) => sibling.path), [siblings, stored]);
  const paths = useMemo(() => (filePath === null ? [] : setPaths(filePath, chosen)), [filePath, chosen]);
  const pathsKey = paths.join("\n");
  const pathsRef = useRef(paths);
  pathsRef.current = paths;
  useEffect(() => {
    const opened = pathsRef.current;
    if (fileKey === null || opened.length === 0) return;
    void tsoStore.open(fileKey, opened);
  }, [fileKey, pathsKey]);
  const set = useTsoSet(fileKey);
  const index = useTsoSeriesIndex(fileKey);

  if (filePath === null || fileKey === null) return null;

  const result = set.result;
  const shownIndex = result ? Math.min(index, result.series.length - 1) : 0;
  const series = result?.series[shownIndex] ?? null;
  const group = result && series ? tsoGroupKey(result.set_id, series.key) : null;
  const sectionProps: TsoSectionProps | null = result && series && group ? { fileKey, paths, set: result, series, group } : null;

  const tick = (path: string, ticked: boolean) => {
    const next = siblings.filter((sibling) => (sibling.path === path ? ticked : chosen.includes(sibling.path))).map((sibling) => sibling.path);
    tsoStore.setChosen(fileKey, next);
  };

  return (
    <div className="ab-panel overflow-hidden" data-tso-panel>
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Clock size={12} className="text-amber-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Time-series spectra</span>
        </div>
        {set.loading && <Loader2 size={12} className="animate-spin text-amber-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <ErrorAlert message={set.error} />

        {siblings.length > 0 && (
          <div className="space-y-1" data-tso-segments>
            <div className={SECTION_CLASS}>Segments of this exposure in Files</div>
            {siblings.map((sibling) => (
              <label key={sibling.path} className="flex items-center gap-1.5 text-[10px] font-mono text-zinc-300 cursor-pointer">
                <input
                  type="checkbox"
                  aria-label={`Stitch ${sibling.name}`}
                  checked={chosen.includes(sibling.path)}
                  onChange={(e) => tick(sibling.path, e.target.checked)}
                />
                <span className="break-all">{sibling.name}</span>
              </label>
            ))}
            <div className={CAPTION_CLASS}>
              Ticked segments are read with this file as one series; drop the other segment files in Files to list them here.
            </div>
          </div>
        )}

        {result && (
          <div className="space-y-1">
            {result.files.map((file) => (
              <div key={file.path} className={LINE_CLASS} data-tso-file>
                {`${file.file_name}, ${levelText(file.level)}, ${layoutText(file.layout)}`}
              </div>
            ))}
            {result.series.length > 1 && (
              <div className="flex flex-col gap-0.5">
                <label htmlFor={seriesId} className={SECTION_CLASS}>
                  Series
                </label>
                <select
                  id={seriesId}
                  aria-label="Time series"
                  value={shownIndex}
                  onChange={(e) => tsoStore.setSeriesIndex(fileKey, Number(e.target.value))}
                  className={SELECT_CLASS}
                >
                  {result.series.map((option, i) => (
                    <option key={i} value={i}>
                      {seriesOptionText(option)}
                    </option>
                  ))}
                </select>
              </div>
            )}
            {series && (
              <div className={LINE_CLASS} data-tso-facts>
                {seriesFactsText(series)}
              </div>
            )}
            {series?.completeness.map((exposure) => (
              <div key={exposure.exposure} className={exposure.complete === false ? INCOMPLETE_CLASS : LINE_CLASS} data-tso-exposure>
                {completenessText(exposure)}
              </div>
            ))}
            <WarningList warnings={result.warnings} />
            {result.notes.length > 0 && (
              <details className="text-[10px] font-mono text-zinc-500" data-tso-notes>
                <summary className="cursor-pointer select-none">{`notes (${result.notes.length})`}</summary>
                <ul className="mt-0.5 flex flex-col gap-0.5">
                  {result.notes.map((note, i) => (
                    <li key={i} className="break-words">
                      {note}
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        )}
      </div>

      {sectionProps && (
        <>
          <Suspense fallback={null}>
            <TsoCurvesSection {...sectionProps} />
          </Suspense>
          <Suspense fallback={null}>
            <TsoWhtltSection {...sectionProps} />
          </Suspense>
          <Suspense fallback={null}>
            <TsoDepthSpectrum {...sectionProps} />
          </Suspense>
        </>
      )}
    </div>
  );
}

export default memo(TsoLightCurvesPanel);
