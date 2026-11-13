import { memo, useCallback, useEffect, useId, useMemo, useState } from "react";
import ProfilePlot, { type PlotBand, type PlotHit, type ProfileSeries } from "../regions/ProfilePlot";
import { DecimalField, SMALL_BUTTON_CLASS, Toggle } from "../ui";
import TransitFitSection from "./TransitFitSection";
import PeriodogramSection from "./PeriodogramSection";
import { lightCurveStore, useLightCurves, useLightCurveView } from "../../hooks/useLightCurveStore";
import {
  activeChoice,
  binText,
  lightCurveModel,
  modelSeries,
  quickLookOf,
  seriesChoices,
  type LightCurveModel,
  type ModelBand,
  type ModelCounts,
} from "../../utils/lightCurveViewModel";
import { EXCLUDABLE_FLAGS, quickLookEntry } from "../../utils/lightCurveDoc";
import { QUICK_LOOK_LABEL } from "../../utils/lightCurveAnalysis";
import { measurementLog } from "../../utils/measurementLog";
import { MISSING_VALUE, formatWithUncertainty } from "../../utils/formatUncertainty";
import { formatSignificant } from "../../utils/formatSignificant";
import {
  binFromDraft,
  clipFromDraft,
  draftFromBin,
  draftFromMargin,
  draftFromWindow,
  marginFromDraft,
  sameWindow,
  windowFromDraft,
  type BinKind,
  type WindowDraft,
} from "../../utils/lightCurveInputs";
import { jdOffsetLabel } from "../../utils/differentialPhotometry";
import { isFiniteNumber } from "../../utils/values";
import {
  DEFAULT_LIGHT_CURVE_VIEW,
  type LightCurveDoc,
  type LightCurveQuantity,
  type LightCurveViewState,
} from "../../shared/types/lightCurve";

export interface LightCurveViewProps {
  group: string;
  onSelectPoint?: (doc: LightCurveDoc, pointRef: number) => void;
  csvName?: string;
  selectedPointRef?: number | null;
}

const LIGHT_CURVE_COLORS: readonly string[] = ["#fbbf24", "#67e8f9", "#a5b4fc", "#86efac", "#f9a8d4", "#fdba74"];
export const LIGHT_CURVE_PLOT_HEIGHT_PX = 170;
export const NO_POINTS_HINT = "No point of this curve has a value to plot.";

const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const LABEL_CLASS = "text-[9px] text-zinc-500";
const CARD_CLASS = "bg-zinc-900/80 rounded px-2 py-1.5";
const CARD_LINE_CLASS = "text-[9px] text-zinc-500";
const CELL_CLASS = "text-right px-1.5 py-0.5";
const LABEL_CELL_CLASS = "text-left px-1.5 py-0.5";

const LEFT_OUT_COLOR = "rgba(161,161,170,0.45)";
const WINDOW_BAND_COLOR = "rgba(251,191,36,0.14)";
const MARGIN_BAND_COLOR = "rgba(161,161,170,0.08)";

const NATIVE_DISPLAY_TEXT: Readonly<Record<LightCurveQuantity, string>> = {
  diff_mag: "magnitude",
  flux: "flux",
  rel_flux: "relative flux",
};
const BASELINE_OPTIONS: readonly (readonly [LightCurveViewState["baselineKind"], string])[] = [
  ["mean", "mean"],
  ["median", "median"],
  ["linear", "straight line"],
];
const BIN_OPTIONS: readonly (readonly [BinKind, string])[] = [
  ["none", "no bins"],
  ["points", "by points"],
  ["minutes", "by minutes"],
];
const BINNED_KEPT_ALPHA = "66";
const LOG_TITLE = "Add the depth, its two errors, the scatter and the window to the measurement log";
const NO_WINDOW_TITLE = "Set a transit window first";
const NO_FORMAL_ERROR = "not every point has an error";

interface ClipDraft {
  sigma: string;
  window: string;
}

function draftFromClip(clipSigma: number | null, clipWindowPoints: number): ClipDraft {
  return {
    sigma: clipSigma === null ? "" : String(clipSigma),
    window: clipWindowPoints === DEFAULT_LIGHT_CURVE_VIEW.clipWindowPoints ? "" : String(clipWindowPoints),
  };
}

function hasPlottableValue(model: LightCurveModel): boolean {
  return model.y.some((v) => v !== null && Number.isFinite(v));
}

function hasWindow(model: LightCurveModel): boolean {
  return model.bands.some((band) => band.kind === "window");
}

function significant(value: number | null | undefined, digits: number): string {
  return isFiniteNumber(value) ? formatSignificant(value, digits) : MISSING_VALUE;
}

function plotBand(band: ModelBand): PlotBand {
  return band.kind === "window"
    ? { x0: band.x0, x1: band.x1, color: WINDOW_BAND_COLOR, label: "in transit" }
    : { x0: band.x0, x1: band.x1, color: MARGIN_BAND_COLOR };
}

function excludableCount(doc: LightCurveDoc): number {
  return doc.flags.filter((words) => words.some((word) => EXCLUDABLE_FLAGS.includes(word))).length;
}

function leftOutText(counts: ModelCounts): string | null {
  const parts: [number, string][] = [
    [counts.flagged, "flagged"],
    [counts.manual, "by hand"],
    [counts.clipped, "clipped"],
    [counts.missing, "without a value"],
  ];
  const shown = parts.filter(([n]) => n > 0).map(([n, what]) => `${n} ${what}`);
  return shown.length === 0 ? null : `left out: ${shown.join(", ")}`;
}

function formalLine(model: LightCurveModel): string | null {
  if (model.scatter === null) return null;
  if (model.scatter.meanFormalPpm === null) return NO_FORMAL_ERROR;
  return `scatter / formal = ${significant(model.scatter.ratio, 3)}`;
}

function binnedScatterLine(model: LightCurveModel, windowed: boolean): string {
  const name = windowed ? "out-of-transit scatter" : "scatter";
  const scatter = model.binnedScatter;
  return scatter ? `${name} ${formatSignificant(scatter.ppm, 4)} ppm, N = ${scatter.n}` : `${name} ${MISSING_VALUE}`;
}

function modelCards(model: LightCurveModel, binLabel: string | null) {
  const { scatter, depth, counts, binned } = model;
  const windowed = hasWindow(model);
  const formal = formalLine(model);
  const leftOut = leftOutText(counts);
  return (
    <div className="grid grid-cols-2 gap-1.5 text-[10px]">
      <div className={CARD_CLASS} data-card="scatter">
        <div className="text-zinc-500">{windowed ? "Out-of-transit scatter" : "Scatter (all kept points)"}</div>
        <div className="text-amber-300 font-mono">{scatter ? `${formatSignificant(scatter.ppm, 4)} ppm` : MISSING_VALUE}</div>
        {scatter && (
          <div className={CARD_LINE_CLASS}>{`${formatSignificant(scatter.mmag, 4)} mmag, N = ${scatter.n}, sample standard deviation`}</div>
        )}
      </div>
      <div className={CARD_CLASS} data-card="formal">
        <div className="text-zinc-500">Mean formal error</div>
        <div className="text-zinc-300 font-mono">
          {isFiniteNumber(scatter?.meanFormalPpm) ? `${formatSignificant(scatter.meanFormalPpm, 4)} ppm` : MISSING_VALUE}
        </div>
        {formal && <div className={CARD_LINE_CLASS}>{formal}</div>}
      </div>
      {windowed && (
        <div className={CARD_CLASS} data-card="depth">
          <div className="text-zinc-500">Depth (box)</div>
          <div className="text-amber-300 font-mono">{formatWithUncertainty(depth?.depth, depth?.err)}</div>
          <div className={CARD_LINE_CLASS}>{`scatter ${significant(depth?.errScatter, 2)}, formal ${significant(depth?.errFormal, 2)}`}</div>
          {depth && <div className={CARD_LINE_CLASS}>{`N in ${depth.nIn}, out ${depth.nOut}`}</div>}
          <div className={CARD_LINE_CLASS}>{QUICK_LOOK_LABEL}</div>
        </div>
      )}
      <div className={CARD_CLASS} data-card="points">
        <div className="text-zinc-500">Points</div>
        <div className="text-zinc-300 font-mono">{`${counts.kept} of ${counts.total}`}</div>
        {leftOut && <div className={CARD_LINE_CLASS}>{leftOut}</div>}
      </div>
      {binned && (
        <div className={CARD_CLASS} data-card="binned">
          <div className="text-zinc-500">{`Binned (${binLabel})`}</div>
          <div className="text-zinc-300 font-mono">{`${binned.x.length} bins`}</div>
          <div className={CARD_LINE_CLASS}>{binnedScatterLine(model, windowed)}</div>
        </div>
      )}
    </div>
  );
}

function curveTable(models: readonly LightCurveModel[], windowed: boolean) {
  return (
    <div className="max-h-56 overflow-auto border border-zinc-800/60 rounded">
      <table className="w-full text-[9px] font-mono" data-curve-table>
        <thead className="sticky top-0 bg-zinc-900 text-zinc-500">
          <tr>
            <th className={LABEL_CELL_CLASS}>series</th>
            <th className={CELL_CLASS}>N</th>
            <th className={CELL_CLASS}>scatter ppm</th>
            {windowed && <th className={CELL_CLASS}>depth</th>}
          </tr>
        </thead>
        <tbody className="text-zinc-300">
          {models.map((m) => (
            <tr key={m.doc.id}>
              <td className={LABEL_CELL_CLASS}>{m.doc.label}</td>
              <td className={CELL_CLASS}>{m.counts.kept}</td>
              <td className={CELL_CLASS}>{significant(m.scatter?.ppm, 4)}</td>
              {windowed && <td className={CELL_CLASS}>{formatWithUncertainty(m.depth?.depth, m.depth?.err)}</td>}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function LightCurveView({ group, onSelectPoint, csvName, selectedPointRef }: LightCurveViewProps) {
  const seriesId = useId();
  const displayId = useId();
  const startId = useId();
  const endId = useId();
  const marginId = useId();
  const baselineId = useId();
  const binId = useId();
  const binSizeId = useId();
  const clipSigmaId = useId();
  const clipWindowId = useId();
  const docs = useLightCurves(group);
  const view = useLightCurveView(group);
  const choices = useMemo(() => seriesChoices(docs), [docs]);
  const active = activeChoice(choices, view.selection);

  const models = useMemo((): LightCurveModel[] => {
    if (!active) return [];
    return active.docIds.flatMap((id) => {
      const doc = docs.find((d) => d.id === id);
      return doc ? [lightCurveModel(doc, view)] : [];
    });
  }, [docs, active, view]);

  const first = models[0] ?? null;
  const offset = first?.offset ?? 0;
  const indexScale = first?.doc.timeScale === "index";

  const [windowDraft, setWindowDraft] = useState<WindowDraft>(() => draftFromWindow(view.window, offset));
  const [marginText, setMarginText] = useState(() => draftFromMargin(view.marginDays, indexScale));
  const [binDraft, setBinDraft] = useState(() => draftFromBin(view.bin));
  const [clipDraft, setClipDraft] = useState<ClipDraft>(() => draftFromClip(view.clipSigma, view.clipWindowPoints));

  useEffect(() => {
    setWindowDraft((current) => {
      const typed = windowFromDraft(current, offset);
      return typed.apply && sameWindow(typed.window, view.window) ? current : draftFromWindow(view.window, offset);
    });
  }, [view.window, offset]);

  useEffect(() => {
    setMarginText((current) => {
      const typed = marginFromDraft(current, indexScale);
      return typed.apply && typed.marginDays === view.marginDays ? current : draftFromMargin(view.marginDays, indexScale);
    });
  }, [view.marginDays, indexScale]);

  useEffect(() => {
    setBinDraft((current) => {
      return binText(binFromDraft(current.kind, current.size).bin) === binText(view.bin) ? current : draftFromBin(view.bin);
    });
  }, [view.bin]);

  useEffect(() => {
    setClipDraft((current) => {
      const typed = clipFromDraft(current.sigma, current.window);
      return typed.apply && typed.clipSigma === view.clipSigma && typed.clipWindowPoints === view.clipWindowPoints
        ? current
        : draftFromClip(view.clipSigma, view.clipWindowPoints);
    });
  }, [view.clipSigma, view.clipWindowPoints]);

  const binLabel = binText(view.bin);

  const plot = useMemo(() => {
    const series: ProfileSeries[] = [];
    const owners: (number | null)[] = [];
    const keptIndex: number[] = [];
    models.forEach((m, k) => {
      const { kept, excluded } = modelSeries(m);
      const color = LIGHT_CURVE_COLORS[k % LIGHT_CURVE_COLORS.length];
      keptIndex.push(series.length);
      owners.push(k);
      series.push({
        x: kept.x,
        y: kept.y,
        yErr: kept.yErr,
        color: m.binned === null ? color : `${color}${BINNED_KEPT_ALPHA}`,
        label: m.doc.label,
        mode: "points",
      });
      if (excluded !== null) {
        owners.push(k);
        series.push({ x: excluded.x, y: excluded.y, color: LEFT_OUT_COLOR, label: `${m.doc.label} (left out)`, mode: "points" });
      }
      if (m.binned === null) return;
      owners.push(null);
      series.push({ x: m.binned.x, y: m.binned.y, yErr: m.binned.yErr, color, label: `${m.doc.label} (${binLabel})`, mode: "both" });
    });
    return { series, owners, keptIndex };
  }, [models, binLabel]);

  const bands = useMemo(() => (first ? first.bands.map(plotBand) : []), [first]);
  const flaggedPoints = useMemo(() => (first ? excludableCount(first.doc) : 0), [first]);
  const notes = useMemo(() => [...new Set(models.flatMap((m) => m.notes))], [models]);
  const quickLooks = useMemo(
    () =>
      models.flatMap((m) => {
        const quickLook = quickLookOf(m, view);
        return quickLook ? [quickLook] : [];
      }),
    [models, view],
  );

  const selected = useMemo(() => {
    if (typeof selectedPointRef !== "number" || !first) return null;
    const index = first.doc.pointRef.indexOf(selectedPointRef);
    return index >= 0 ? { seriesIndex: plot.keptIndex[0], index } : null;
  }, [first, selectedPointRef, plot]);

  const patch = useCallback((change: Partial<LightCurveViewState>) => lightCurveStore.patchView(group, change), [group]);

  const selectPoint = useCallback(
    (hit: PlotHit) => {
      const owner = plot.owners[hit.seriesIndex];
      const model = owner === null || owner === undefined ? undefined : models[owner];
      if (!model) return;
      if (view.clickExcludes) {
        const pointState = model.state[hit.index];
        if (pointState !== "kept" && pointState !== "manual") return;
        const excluded = view.excluded.includes(hit.index)
          ? view.excluded.filter((position) => position !== hit.index)
          : [...view.excluded, hit.index].sort((a, b) => a - b);
        patch({ excluded });
        return;
      }
      const pointRef = model.doc.pointRef[hit.index];
      if (pointRef !== undefined) onSelectPoint?.(model.doc, pointRef);
    },
    [models, plot, view.clickExcludes, view.excluded, patch, onSelectPoint],
  );

  const editWindow = useCallback(
    (draft: WindowDraft) => {
      setWindowDraft(draft);
      const typed = windowFromDraft(draft, offset);
      if (typed.apply && !sameWindow(typed.window, view.window)) patch({ window: typed.window });
    },
    [offset, view.window, patch],
  );

  const editMargin = useCallback(
    (text: string) => {
      setMarginText(text);
      const typed = marginFromDraft(text, indexScale);
      if (typed.apply && typed.marginDays !== view.marginDays) patch({ marginDays: typed.marginDays });
    },
    [indexScale, view.marginDays, patch],
  );

  const editBin = useCallback(
    (draft: { kind: BinKind; size: string }) => {
      setBinDraft(draft);
      const typed = binFromDraft(draft.kind, draft.size);
      if (binText(typed.bin) !== binText(view.bin)) patch({ bin: typed.bin });
    },
    [view.bin, patch],
  );

  const editClip = useCallback(
    (draft: ClipDraft) => {
      setClipDraft(draft);
      const typed = clipFromDraft(draft.sigma, draft.window);
      if (typed.apply && (typed.clipSigma !== view.clipSigma || typed.clipWindowPoints !== view.clipWindowPoints)) {
        patch({ clipSigma: typed.clipSigma, clipWindowPoints: typed.clipWindowPoints });
      }
    },
    [view.clipSigma, view.clipWindowPoints, patch],
  );

  const logQuickLook = useCallback(() => {
    quickLooks.forEach((quickLook) => measurementLog.append(quickLookEntry(quickLook)));
  }, [quickLooks]);

  if (!active || !first) return null;

  const windowInput = windowFromDraft(windowDraft, offset);
  const draftMessage =
    windowInput.error ??
    windowInput.hint ??
    marginFromDraft(marginText, indexScale).error ??
    binFromDraft(binDraft.kind, binDraft.size).error ??
    clipFromDraft(clipDraft.sigma, clipDraft.window).error;
  const canLog = quickLooks.length > 0;
  const handExcluded = view.excluded.length;

  const chooseSeries = (key: string) => {
    const nextFirstId = choices.find((choice) => choice.key === key)?.docIds[0];
    const nextFirst = docs.find((doc) => doc.id === nextFirstId);
    const otherLength = nextFirst !== undefined && nextFirst.time.length !== first.doc.time.length;
    patch(otherLength ? { selection: key, excluded: [] } : { selection: key });
  };

  return (
    <div className="space-y-2" data-light-curve-view={group}>
      <div className="grid grid-cols-2 gap-1.5">
        <div className="flex flex-col gap-0.5">
          <label htmlFor={seriesId} className={LABEL_CLASS}>
            series
          </label>
          <select id={seriesId} value={active.key} onChange={(e) => chooseSeries(e.target.value)} className={SELECT_CLASS}>
            {choices.map((choice) => (
              <option key={choice.key} value={choice.key}>
                {choice.label}
              </option>
            ))}
          </select>
        </div>
        <div className="flex flex-col gap-0.5">
          <label htmlFor={displayId} className={LABEL_CLASS}>
            display
          </label>
          <select
            id={displayId}
            value={view.display}
            onChange={(e) => patch({ display: e.target.value as LightCurveViewState["display"] })}
            className={SELECT_CLASS}
          >
            <option value="native">{NATIVE_DISPLAY_TEXT[first.doc.quantity]}</option>
            <option value="normalised">normalised flux</option>
          </select>
        </div>
      </div>
      <div className="grid grid-cols-4 gap-1.5">
        <div className="flex flex-col gap-0.5">
          <label htmlFor={startId} className={LABEL_CLASS}>
            transit start
          </label>
          <DecimalField
            id={startId}
            optional
            value={windowDraft.start}
            onChange={(text) => editWindow({ ...windowDraft, start: text })}
            className={INPUT_CLASS}
          />
        </div>
        <div className="flex flex-col gap-0.5">
          <label htmlFor={endId} className={LABEL_CLASS}>
            transit end
          </label>
          <DecimalField
            id={endId}
            optional
            value={windowDraft.end}
            onChange={(text) => editWindow({ ...windowDraft, end: text })}
            className={INPUT_CLASS}
          />
        </div>
        <div className="flex flex-col gap-0.5">
          <label htmlFor={marginId} className={LABEL_CLASS}>
            {indexScale ? "margin points" : "margin min"}
          </label>
          <DecimalField id={marginId} optional min={0} value={marginText} onChange={editMargin} className={INPUT_CLASS} />
        </div>
        <div className="flex flex-col gap-0.5">
          <label htmlFor={baselineId} className={LABEL_CLASS}>
            baseline
          </label>
          <select
            id={baselineId}
            value={view.baselineKind}
            onChange={(e) => patch({ baselineKind: e.target.value as LightCurveViewState["baselineKind"] })}
            className={SELECT_CLASS}
          >
            {BASELINE_OPTIONS.map(([kind, text]) => (
              <option key={kind} value={kind}>
                {text}
              </option>
            ))}
          </select>
        </div>
      </div>
      <div className="grid grid-cols-4 gap-1.5">
        <div className="flex flex-col gap-0.5">
          <label htmlFor={binId} className={LABEL_CLASS}>
            bin
          </label>
          <select
            id={binId}
            value={binDraft.kind}
            onChange={(e) => editBin({ kind: e.target.value as BinKind, size: binDraft.size })}
            className={SELECT_CLASS}
          >
            {BIN_OPTIONS.map(([kind, text]) => (
              <option key={kind} value={kind} disabled={kind === "minutes" && indexScale}>
                {text}
              </option>
            ))}
          </select>
        </div>
        <div className="flex flex-col gap-0.5">
          <label htmlFor={binSizeId} className={LABEL_CLASS}>
            bin size
          </label>
          <DecimalField
            id={binSizeId}
            optional
            disabled={binDraft.kind === "none"}
            value={binDraft.size}
            onChange={(text) => editBin({ kind: binDraft.kind, size: text })}
            className={INPUT_CLASS}
          />
        </div>
        <div className="flex flex-col gap-0.5">
          <label htmlFor={clipSigmaId} className={LABEL_CLASS}>
            clip sigma
          </label>
          <DecimalField
            id={clipSigmaId}
            optional
            placeholder="off"
            value={clipDraft.sigma}
            onChange={(text) => editClip({ ...clipDraft, sigma: text })}
            className={INPUT_CLASS}
          />
        </div>
        <div className="flex flex-col gap-0.5">
          <label htmlFor={clipWindowId} className={LABEL_CLASS}>
            clip window
          </label>
          <DecimalField
            id={clipWindowId}
            optional
            integer
            placeholder="11"
            value={clipDraft.window}
            onChange={(text) => editClip({ ...clipDraft, window: text })}
            className={INPUT_CLASS}
          />
        </div>
      </div>
      <div className="text-[9px] text-zinc-600">
        {`Window in the units of the time axis (${first.xLabel}); the baseline is every kept point outside the window and its margin.`}
      </div>
      {draftMessage && (
        <div className="text-[9px] text-amber-400/90" data-window-hint>
          {draftMessage}
        </div>
      )}
      {flaggedPoints > 0 && (
        <Toggle
          label={`Leave out flagged points (${flaggedPoints})`}
          checked={view.excludeFlagged}
          accent="amber"
          onChange={(checked) => patch({ excludeFlagged: checked })}
        />
      )}
      <Toggle
        label="Click a point to leave it out"
        checked={view.clickExcludes}
        accent="amber"
        onChange={(checked) => patch({ clickExcludes: checked })}
      />
      {handExcluded > 0 && (
        <button type="button" onClick={() => patch({ excluded: [] })} className={SMALL_BUTTON_CLASS}>
          {`Put ${handExcluded} ${handExcluded === 1 ? "point" : "points"} back`}
        </button>
      )}
      {models.some(hasPlottableValue) ? (
        <ProfilePlot
          series={plot.series}
          xLabel={first.xLabel}
          yLabel={first.yLabel}
          height={LIGHT_CURVE_PLOT_HEIGHT_PX}
          invertY={first.invertY}
          toolbar
          logToggle={first.doc.quantity === "flux" && !first.normalised}
          onSelect={selectPoint}
          selected={selected}
          csvName={csvName ?? "lightcurve"}
          xTickFormat={first.doc.timeScale === "index" ? undefined : jdOffsetLabel}
          wheelZoom="modifier"
          bands={bands}
        />
      ) : (
        <div className="text-[10px] text-zinc-600">{NO_POINTS_HINT}</div>
      )}
      {active.docIds.length === 1 ? modelCards(first, binLabel) : curveTable(models, hasWindow(first))}
      {notes.map((note) => (
        <div key={note} className="text-[9px] text-amber-400/90" data-curve-note>
          {note}
        </div>
      ))}
      <button
        type="button"
        onClick={logQuickLook}
        disabled={!canLog}
        title={canLog ? LOG_TITLE : NO_WINDOW_TITLE}
        className={SMALL_BUTTON_CLASS}
      >
        Log quick look
      </button>
      {active.docIds.length === 1 && <TransitFitSection group={group} model={first} csvName={csvName ?? "lightcurve"} />}
      {active.docIds.length === 1 && <PeriodogramSection group={group} model={first} csvName={csvName ?? "lightcurve"} />}
    </div>
  );
}

export default memo(LightCurveView);
