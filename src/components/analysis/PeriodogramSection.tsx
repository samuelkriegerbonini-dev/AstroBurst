import { memo, useEffect, useId, useMemo, useRef, useState } from "react";
import ProfilePlot from "../regions/ProfilePlot";
import { DecimalField, ErrorAlert, SMALL_BUTTON_CLASS } from "../ui";
import { lightCurveStore, useLightCurveView } from "../../hooks/useLightCurveStore";
import { lightCurvePeriodogram, type PeriodogramSearch } from "../../services/periodogram";
import { formatSignificant } from "../../utils/formatSignificant";
import { periodogramEntry } from "../../utils/lightCurveDoc";
import {
  fapText,
  foldFromDraft,
  periodogramAliasWarning,
  periodogramRangeText,
  periodRangeFromDraft,
  periodStep,
} from "../../utils/lightCurveInputs";
import { periodogramInput, type LightCurveModel } from "../../utils/lightCurveViewModel";
import { measurementLog } from "../../utils/measurementLog";
import type { LightCurveViewState } from "../../shared/types/lightCurve";

export interface PeriodogramSectionProps {
  group: string;
  model: LightCurveModel;
  csvName: string;
}

const PERIODOGRAM_PLOT_HEIGHT_PX = 130;
const FOLDED_PLOT_HEIGHT_PX = 150;

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const LABEL_CLASS = "text-[9px] text-zinc-500";
const NOTICE_CLASS = "text-[9px] text-amber-400/90";
const CARD_CLASS = "bg-zinc-900/80 rounded px-2 py-1.5";
const POWER_COLOR = "#67e8f9";
const FOLDED_COLOR = "#fbbf24";
const MIN_SEARCH_POINTS = 4;
const TIME_AXIS_REASON = "A period search needs a time axis.";
const FEW_POINTS_REASON = "A period search needs at least 4 points.";
const STALE_SEARCH = "The curve changed since this search; search again.";
const FAP_LINE = "Baluev 2008 upper limit for the highest peak, white Gaussian noise";

type Fold = LightCurveViewState["fold"];

interface FoldDraft {
  period: string;
  epoch: string;
}

interface SearchRecord {
  result: PeriodogramSearch;
  signature: string;
  minFrequency: number | null;
  maxFrequency: number | null;
}

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function draftFromFold(fold: Fold, offset: number): FoldDraft {
  if (fold === null) return { period: "", epoch: "" };
  return { period: String(fold.periodDays), epoch: String(Number((fold.t0 - offset).toFixed(8))) };
}

function sameFold(a: Fold, b: Fold): boolean {
  if (a === null || b === null) return a === b;
  return a.periodDays === b.periodDays && a.t0 === b.t0;
}

function PeriodogramSection({ group, model, csvName }: PeriodogramSectionProps) {
  const minPeriodId = useId();
  const maxPeriodId = useId();
  const foldPeriodId = useId();
  const foldEpochId = useId();
  const view = useLightCurveView(group);
  const input = useMemo(() => periodogramInput(model), [model]);
  const [minPeriodText, setMinPeriodText] = useState("");
  const [maxPeriodText, setMaxPeriodText] = useState("");
  const [foldDraft, setFoldDraft] = useState<FoldDraft>(() => draftFromFold(view.fold, model.offset));
  const [record, setRecord] = useState<SearchRecord | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sequence = useRef(0);
  const docKey = `${group}|${model.doc.id}`;
  const offset = model.offset;

  useEffect(() => {
    sequence.current += 1;
    setRunning(false);
  }, [docKey]);

  useEffect(() => {
    setFoldDraft((current) => {
      const typed = foldFromDraft(current.period, current.epoch, offset);
      return typed.apply && sameFold(typed.fold, view.fold) ? current : draftFromFold(view.fold, offset);
    });
  }, [view.fold, offset]);

  const range = periodRangeFromDraft(minPeriodText, maxPeriodText);
  const signature = `${docKey}|${model.counts.kept}|${model.normalised}`;
  const timeAxis = model.doc.timeScale !== "index";
  const reason = !timeAxis ? TIME_AXIS_REASON : input.value.length < MIN_SEARCH_POINTS ? FEW_POINTS_REASON : range.error;
  const foldError = foldFromDraft(foldDraft.period, foldDraft.epoch, offset).error;

  const search = () => {
    if (reason !== null) return;
    const { minFrequency, maxFrequency } = range;
    const seq = ++sequence.current;
    setRunning(true);
    setError(null);
    lightCurvePeriodogram(input.time, input.value, input.err, { minFrequency, maxFrequency }).then(
      (result) => {
        if (seq !== sequence.current) return;
        setRecord({ result, signature, minFrequency, maxFrequency });
        setRunning(false);
      },
      (e: unknown) => {
        if (seq !== sequence.current) return;
        setError(messageOf(e));
        setRunning(false);
      },
    );
  };

  const editFold = (draft: FoldDraft) => {
    setFoldDraft(draft);
    const typed = foldFromDraft(draft.period, draft.epoch, offset);
    if (typed.apply && !sameFold(typed.fold, view.fold)) lightCurveStore.patchView(group, { fold: typed.fold });
  };

  const foldAtPeak = (result: PeriodogramSearch) => {
    const window = view.window;
    const t0 = window !== null ? (window.start + window.end) / 2 : input.time.length > 0 ? input.time[0] : null;
    if (t0 !== null) lightCurveStore.patchView(group, { fold: { periodDays: result.best_period, t0 } });
  };

  const logSearch = (searched: SearchRecord) =>
    measurementLog.append(
      periodogramEntry({
        doc: model.doc,
        valueName: model.yLabel,
        result: searched.result,
        minFrequency: searched.minFrequency,
        maxFrequency: searched.maxFrequency,
        fold: view.fold,
      }),
    );

  const searchResult = (searched: SearchRecord) => {
    const { result } = searched;
    const aliasWarning = periodogramAliasWarning(result);
    return (
      <>
        {searched.signature !== signature && (
          <div className={NOTICE_CLASS} data-stale-search>
            {STALE_SEARCH}
          </div>
        )}
        <ProfilePlot
          series={[{ x: result.frequency, y: result.power, color: POWER_COLOR, label: "power", mode: "line" }]}
          xLabel="frequency (cycles per day)"
          yLabel="power"
          height={PERIODOGRAM_PLOT_HEIGHT_PX}
          toolbar
          logToggle={false}
          wheelZoom="modifier"
          csvName={`${csvName}_periodogram`}
          referenceLines={[{ axis: "x", value: result.best_frequency, label: `P = ${formatSignificant(result.best_period, 6)} d` }]}
        />
        <div className={LABEL_CLASS} data-search-range>
          {periodogramRangeText(result)}
        </div>
        {aliasWarning !== null && (
          <div className={NOTICE_CLASS} data-alias-warning>
            {aliasWarning}
          </div>
        )}
        <div className="grid grid-cols-3 gap-1.5 text-[10px]">
          <div className={CARD_CLASS} data-card="period">
            <div className="text-zinc-500">Best period</div>
            <div className="text-amber-300 font-mono">{`${formatSignificant(result.best_period, 6)} d`}</div>
            <div className={LABEL_CLASS}>
              {`frequency ${formatSignificant(result.best_frequency, 6)} per day, power ${formatSignificant(result.best_power, 3)}`}
            </div>
          </div>
          <div className={CARD_CLASS} data-card="fap">
            <div className="text-zinc-500">False-alarm probability</div>
            <div className="text-zinc-300 font-mono">{fapText(result.false_alarm_probability)}</div>
            <div className={LABEL_CLASS}>{FAP_LINE}</div>
          </div>
          <div className={CARD_CLASS} data-card="resolution">
            <div className="text-zinc-500">Grid resolution</div>
            <div className="text-zinc-300 font-mono">{`${formatSignificant(periodStep(result.best_period, result.frequency_step), 2)} d`}</div>
            <div className={LABEL_CLASS}>
              {`one step of ${result.n_frequencies} frequencies; ${result.n_points} points${result.weighted ? ", weighted by 1 / err^2" : ", equal weights"}`}
            </div>
          </div>
        </div>
        <details className="text-[9px] text-zinc-500" data-periodogram-notes>
          <summary className="cursor-pointer select-none">{`notes (${result.notes.length})`}</summary>
          <ul className="mt-1 space-y-0.5 list-disc pl-4">
            {result.notes.map((note, k) => (
              <li key={k}>{note}</li>
            ))}
          </ul>
        </details>
        <div className="flex flex-wrap gap-1.5">
          <button type="button" onClick={() => foldAtPeak(result)} className={SMALL_BUTTON_CLASS}>
            Fold at this period
          </button>
          <button type="button" onClick={() => logSearch(searched)} className={SMALL_BUTTON_CLASS}>
            Log period search
          </button>
        </div>
      </>
    );
  };

  return (
    <details data-periodogram className="rounded border border-zinc-800/60 px-2 py-1">
      <summary className="cursor-pointer select-none text-[10px] text-zinc-400">Period search and fold</summary>
      <div className="space-y-2 pt-1.5">
        <div className="grid grid-cols-3 gap-1.5 items-end">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={minPeriodId} className={LABEL_CLASS}>
              shortest period d
            </label>
            <DecimalField
              id={minPeriodId}
              optional
              placeholder="automatic"
              value={minPeriodText}
              onChange={setMinPeriodText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={maxPeriodId} className={LABEL_CLASS}>
              longest period d
            </label>
            <DecimalField
              id={maxPeriodId}
              optional
              placeholder="automatic"
              value={maxPeriodText}
              onChange={setMaxPeriodText}
              className={INPUT_CLASS}
            />
          </div>
          <button
            type="button"
            onClick={search}
            disabled={reason !== null || running}
            title={reason ?? undefined}
            className={SMALL_BUTTON_CLASS}
          >
            {running ? "Searching..." : "Search period"}
          </button>
        </div>
        {reason !== null && (
          <div className={NOTICE_CLASS} data-search-hint>
            {reason}
          </div>
        )}
        <ErrorAlert message={error} />
        {record !== null && searchResult(record)}
        {timeAxis && (
          <>
            <div className="grid grid-cols-3 gap-1.5 items-end">
              <div className="flex flex-col gap-0.5">
                <label htmlFor={foldPeriodId} className={LABEL_CLASS}>
                  fold period d
                </label>
                <DecimalField
                  id={foldPeriodId}
                  optional
                  value={foldDraft.period}
                  onChange={(text) => editFold({ ...foldDraft, period: text })}
                  className={INPUT_CLASS}
                />
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={foldEpochId} className={LABEL_CLASS}>
                  fold epoch
                </label>
                <DecimalField
                  id={foldEpochId}
                  optional
                  value={foldDraft.epoch}
                  onChange={(text) => editFold({ ...foldDraft, epoch: text })}
                  className={INPUT_CLASS}
                />
              </div>
              {view.fold !== null && (
                <button type="button" onClick={() => lightCurveStore.patchView(group, { fold: null })} className={SMALL_BUTTON_CLASS}>
                  Unfold
                </button>
              )}
            </div>
            {foldError !== null && (
              <div className={NOTICE_CLASS} data-fold-hint>
                {foldError}
              </div>
            )}
          </>
        )}
        {model.folded !== null && view.fold !== null && (
          <ProfilePlot
            series={[
              { x: model.folded.phase, y: model.folded.y, yErr: model.folded.yErr, color: FOLDED_COLOR, label: model.doc.label, mode: "points" },
            ]}
            xLabel={`phase (period ${formatSignificant(view.fold.periodDays, 6)} d)`}
            yLabel={model.yLabel}
            invertY={model.invertY}
            height={FOLDED_PLOT_HEIGHT_PX}
            toolbar
            logToggle={false}
            wheelZoom="modifier"
            csvName={`${csvName}_folded`}
          />
        )}
      </div>
    </details>
  );
}

export default memo(PeriodogramSection);
