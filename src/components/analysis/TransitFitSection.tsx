import { memo, useId, useMemo, useRef, useState } from "react";
import ProfilePlot from "../regions/ProfilePlot";
import { CopyCsvButton, DecimalField, ErrorAlert, SMALL_BUTTON_CLASS, Toggle } from "../ui";
import { useLightCurveView } from "../../hooks/useLightCurveStore";
import { transitFitStore, useTransitFit } from "../../hooks/useTransitFitStore";
import { fitTransit } from "../../services/transit";
import { parseDecimalText } from "../../utils/decimalText";
import { jdOffsetLabel } from "../../utils/differentialPhotometry";
import { formatSignificant } from "../../utils/formatSignificant";
import { MISSING_VALUE } from "../../utils/formatUncertainty";
import { docTimeLabel } from "../../utils/lightCurveDoc";
import type { LightCurveModel } from "../../utils/lightCurveViewModel";
import { measurementLog } from "../../utils/measurementLog";
import { saveCsvDialog } from "../../utils/spectrumExport";
import {
  timeAveragingRows,
  transitFitCsvText,
  transitFitEntry,
  transitFitLabel,
  transitPlotSeries,
  transitResultRows,
  type TransitResultRow,
} from "../../utils/transitFitReport";
import {
  setupWithPeriod,
  setupWithStart,
  transitDraftLabel,
  transitDraftNames,
  transitFitInput,
  transitFitRequest,
  transitFitSignature,
  transitStartFromQuickLook,
  type TransitDraftName,
  type TransitFitSetup,
  type TransitParameterDraft,
} from "../../utils/transitFitSetup";
import { fileBaseName } from "../../utils/values";
import type { LightCurveSource } from "../../shared/types/lightCurve";
import type { TransitFitRequest, TransitFitResult, TransitKind } from "../../shared/types/transit";

export interface TransitFitSectionProps {
  group: string;
  model: LightCurveModel;
  csvName: string;
}

export const TRANSIT_FIT_PLOT_HEIGHT_PX = 170;
export const TRANSIT_RESIDUAL_PLOT_HEIGHT_PX = 110;

const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const LABEL_CLASS = "text-[9px] text-zinc-500";
const NOTICE_CLASS = "text-[9px] text-amber-400/90";
const CELL_CLASS = "text-right px-1.5 py-0.5";
const LABEL_CELL_CLASS = "text-left px-1.5 py-0.5";

const KIND_OPTIONS: readonly (readonly [TransitKind, string])[] = [
  ["transit", "transit (planet in front)"],
  ["eclipse", "eclipse (planet behind)"],
];
const BASELINE_OPTIONS: readonly (readonly [TransitFitSetup["baselineOrder"], string])[] = [
  [0, "constant"],
  [1, "straight line"],
  [2, "parabola"],
];
const START_TITLE = "Fill the mid time, Rp/Rs, a/Rs and b from the transit window, the box depth and the period";
const NO_WINDOW_TITLE = "Set a transit window in the quick look first";
const NO_PERIOD_TITLE = "Type the period first (the window must be shorter than half of it)";
const TIME_AXIS_REASON = "A transit fit needs a time axis.";
const STALE_FIT = "The curve or its kept points changed since this fit; fit again.";
const DATA_COLOR = "#fbbf24";
const MODEL_COLOR = "#67e8f9";
const CARD_CLASS = "bg-zinc-900/80 rounded px-2 py-1.5";
const CAPTION_CLASS = "text-[9px] text-zinc-600";
const TABLE_CLASS = "w-full text-[9px] font-mono";
const TIME_AVERAGING_CAPTION =
  "Residuals binned by 1, 2, 4, ... points. White noise falls as 1 / sqrt(points); a ratio well above 1 at long bins is correlated noise that the errors of the fit do not include.";
const LOG_TITLE = "Add the fitted values, their errors and the notes of the fit to the measurement log";

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function csvFileName(source: LightCurveSource): string | null {
  if (source.kind === "frames") return source.reference;
  if (source.kind === "table") return fileBaseName(source.path);
  return source.paths.length > 0 ? source.paths.map(fileBaseName).join("; ") : null;
}

function withUnit(row: TransitResultRow | undefined): string {
  if (row === undefined) return MISSING_VALUE;
  return row.unit === "" ? row.value : `${row.value} ${row.unit}`;
}

function optionalSignificant(value: number | null, digits: number): string {
  return value === null ? MISSING_VALUE : formatSignificant(value, digits);
}

function withDraft(setup: TransitFitSetup, name: TransitDraftName, draft: TransitParameterDraft): TransitFitSetup {
  return { ...setup, [name]: draft };
}

function captionOf(kind: TransitKind, xLabel: string): string {
  const common = `Times in the units of the time axis (${xLabel}). A free parameter starts from its value; a fixed one keeps it.`;
  return kind === "eclipse"
    ? `${common} Rp/Rs, a/Rs and b are usually fixed to the values of the transit fit.`
    : `${common} u1 = u2 = 0 is a uniform disc: limb darkening is typed or fitted, never taken from a stellar model.`;
}

function TransitFitSection({ group, model, csvName }: TransitFitSectionProps) {
  const kindId = useId();
  const periodId = useId();
  const baselineId = useId();
  const rowsId = useId();
  const docId = model.doc.id;
  const record = useTransitFit(group, docId);
  const view = useLightCurveView(group);
  const setup = record.setup;
  const input = useMemo(() => transitFitInput(model), [model]);
  const signature = useMemo(() => transitFitSignature(input), [input]);
  const built = useMemo(() => transitFitRequest(input, setup, model.offset), [input, setup, model.offset]);
  const series = useMemo(() => {
    if (record.result === null || record.request === null) return null;
    const { time, flux, err } = record.request;
    return transitPlotSeries(record.result, { time, flux, err, pointIndex: [] }, model.offset);
  }, [record.result, record.request, model.offset]);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const sequence = useRef(0);
  const doc = model.doc;
  const fluxUnit = doc.quantity === "flux" ? doc.unit : null;

  const storeSetup = (next: TransitFitSetup) => transitFitStore.setSetup(group, docId, next);

  const fitCsv = (result: TransitFitResult, request: TransitFitRequest): string =>
    transitFitCsvText(
      result,
      { time: request.time, flux: request.flux, err: request.err, pointIndex: [] },
      { fileName: csvFileName(doc.source), series: doc.label, timeName: docTimeLabel(doc, 0), timeScale: doc.timeScale, fluxUnit, exportedUtc: new Date().toISOString() },
    );

  const saveFit = async (text: string) => {
    setSavedPath(null);
    setError(null);
    try {
      setSavedPath(await saveCsvDialog(text, `${csvName}_transit_fit.csv`, "Save the transit fit"));
    } catch (e: unknown) {
      setError(messageOf(e));
    }
  };

  const logFit = (result: TransitFitResult) => measurementLog.append(transitFitEntry(doc, result, docTimeLabel(doc, 0)));

  const fitResults = (result: TransitFitResult, request: TransitFitRequest) => {
    const rows = transitResultRows(result, model.offset, model.xLabel, fluxUnit);
    const copyText = () => {
      setError(null);
      return fitCsv(result, request);
    };
    return (
      <>
        <div className="grid grid-cols-2 gap-1.5 text-[10px]">
          <div className={CARD_CLASS} data-card="fit-depth">
            <div className="text-zinc-500">{result.kind === "eclipse" ? "Eclipse depth" : "(Rp/Rs)^2"}</div>
            <div className="text-amber-300 font-mono">{withUnit(rows.find((row) => row.key === "depth"))}</div>
            <div className={LABEL_CLASS}>least squares, 1 sigma from the covariance</div>
            {result.error_scale !== 1 && (
              <div className={LABEL_CLASS}>{`errors multiplied by ${formatSignificant(result.error_scale, 3)}`}</div>
            )}
          </div>
          <div className={CARD_CLASS} data-card="fit-scatter">
            <div className="text-zinc-500">Residual scatter</div>
            <div className="text-zinc-300 font-mono">{`${formatSignificant(result.rms_ppm, 4)} ppm`}</div>
            <div className={LABEL_CLASS}>
              {`N = ${result.n_points}, ${result.n_in_transit} in ${result.kind === "eclipse" ? "eclipse" : "transit"}`}
            </div>
          </div>
          <div className={CARD_CLASS} data-card="fit-chi2">
            <div className="text-zinc-500">Reduced chi2</div>
            <div className="text-zinc-300 font-mono">{result.weighted ? formatSignificant(result.reduced_chi2, 3) : MISSING_VALUE}</div>
            <div className={LABEL_CLASS}>
              {result.weighted
                ? `chi2 ${formatSignificant(result.chi2, 5)} for ${result.dof} degrees of freedom`
                : "equal weights: the points carry no errors, so there is no chi2 to judge"}
            </div>
          </div>
          <div className={CARD_CLASS} data-card="fit-beta">
            <div className="text-zinc-500">Time-averaging beta</div>
            <div className="text-zinc-300 font-mono">{formatSignificant(result.time_averaging.beta, 3)}</div>
            <div className={LABEL_CLASS}>largest binned rms over the white-noise expectation; 1 is white noise</div>
          </div>
        </div>
        <table className={TABLE_CLASS} data-fit-results>
          <thead className="bg-zinc-900 text-zinc-500">
            <tr>
              <th className={LABEL_CELL_CLASS}>quantity</th>
              <th className={CELL_CLASS}>value</th>
              <th className={LABEL_CELL_CLASS}>unit</th>
              <th className={LABEL_CELL_CLASS}>fit</th>
            </tr>
          </thead>
          <tbody className="text-zinc-300">
            {rows.map((row) => (
              <tr key={row.key} data-fit-row={row.key}>
                <td className={LABEL_CELL_CLASS}>{row.label}</td>
                <td className={CELL_CLASS}>{row.value}</td>
                <td className={LABEL_CELL_CLASS}>{row.unit}</td>
                <td className={LABEL_CELL_CLASS}>{row.fit}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <details className="text-[9px] text-zinc-500" data-fit-time-averaging>
          <summary className="cursor-pointer select-none">Time-averaging test</summary>
          <table className={`${TABLE_CLASS} mt-1`}>
            <thead className="bg-zinc-900 text-zinc-500">
              <tr>
                <th className={CELL_CLASS}>bin points</th>
                <th className={CELL_CLASS}>bin min</th>
                <th className={CELL_CLASS}>bins</th>
                <th className={CELL_CLASS}>rms ppm</th>
                <th className={CELL_CLASS}>white noise ppm</th>
                <th className={CELL_CLASS}>ratio</th>
              </tr>
            </thead>
            <tbody className="text-zinc-300">
              {timeAveragingRows(result).map((bin) => (
                <tr key={bin.points}>
                  <td className={CELL_CLASS}>{bin.points}</td>
                  <td className={CELL_CLASS}>{optionalSignificant(bin.minutes, 3)}</td>
                  <td className={CELL_CLASS}>{bin.bins}</td>
                  <td className={CELL_CLASS}>{formatSignificant(bin.rmsPpm, 4)}</td>
                  <td className={CELL_CLASS}>{formatSignificant(bin.expectedPpm, 4)}</td>
                  <td className={CELL_CLASS}>{optionalSignificant(bin.ratio, 3)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className={CAPTION_CLASS}>{TIME_AVERAGING_CAPTION}</div>
        </details>
        <details className="text-[9px] text-zinc-500" data-fit-notes>
          <summary className="cursor-pointer select-none">{`notes (${result.notes.length})`}</summary>
          <ul className="mt-1 space-y-0.5 list-disc pl-4">
            {result.notes.map((note, k) => (
              <li key={k}>{note}</li>
            ))}
          </ul>
        </details>
        <div className="flex flex-wrap gap-1.5">
          <button type="button" onClick={() => void saveFit(fitCsv(result, request))} className={SMALL_BUTTON_CLASS}>
            Save fit CSV
          </button>
          <CopyCsvButton label="Copy fit CSV" text={copyText} className={SMALL_BUTTON_CLASS} icon={null} onError={setError} />
          <button type="button" onClick={() => logFit(result)} title={LOG_TITLE} className={SMALL_BUTTON_CLASS}>
            Log transit fit
          </button>
        </div>
        {savedPath !== null && (
          <div className={LABEL_CLASS} data-fit-saved>
            {`Saved ${savedPath}`}
          </div>
        )}
      </>
    );
  };

  const start = transitStartFromQuickLook(view.window, model.depth?.depth ?? null, parseDecimalText(setup.period));
  const startTitle = start !== null ? START_TITLE : view.window === null ? NO_WINDOW_TITLE : NO_PERIOD_TITLE;
  const fold = setup.period.trim() === "" ? view.fold : null;
  const fitReason = model.doc.timeScale === "index" ? TIME_AXIS_REASON : built.error;

  const runFit = () => {
    const request = fitReason === null ? built.request : null;
    if (request === null) return;
    const seq = ++sequence.current;
    setRunning(true);
    setError(null);
    setSavedPath(null);
    fitTransit(request).then(
      (result) => {
        if (seq !== sequence.current) return;
        transitFitStore.setResult(group, docId, { request, result, signature, fittedUtc: new Date().toISOString() });
        setRunning(false);
      },
      (e: unknown) => {
        if (seq !== sequence.current) return;
        setError(messageOf(e));
        setRunning(false);
      },
    );
  };

  return (
    <details data-transit-fit className="rounded border border-zinc-800/60 px-2 py-1">
      <summary className="cursor-pointer select-none text-[10px] text-zinc-400">Transit fit</summary>
      <div className="space-y-2 pt-1.5">
        <div className={LABEL_CLASS} data-fit-label>
          {`Fit: ${transitFitLabel(setup.kind)}`}
        </div>
        <div className={LABEL_CLASS}>The quick-look depth above stays the first look. The period is typed and never fitted.</div>
        <div className="grid grid-cols-4 gap-1.5 items-end">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={kindId} className={LABEL_CLASS}>
              fit
            </label>
            <select
              id={kindId}
              value={setup.kind}
              onChange={(e) => storeSetup({ ...setup, kind: e.target.value as TransitKind })}
              className={SELECT_CLASS}
            >
              {KIND_OPTIONS.map(([kind, text]) => (
                <option key={kind} value={kind}>
                  {text}
                </option>
              ))}
            </select>
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={periodId} className={LABEL_CLASS}>
              period d
            </label>
            <DecimalField
              id={periodId}
              min={0}
              placeholder="days"
              value={setup.period}
              onChange={(text) => storeSetup({ ...setup, period: text })}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={baselineId} className={LABEL_CLASS}>
              baseline
            </label>
            <select
              id={baselineId}
              value={String(setup.baselineOrder)}
              onChange={(e) => storeSetup({ ...setup, baselineOrder: Number(e.target.value) as TransitFitSetup["baselineOrder"] })}
              className={SELECT_CLASS}
            >
              {BASELINE_OPTIONS.map(([order, text]) => (
                <option key={order} value={String(order)}>
                  {text}
                </option>
              ))}
            </select>
          </div>
          <Toggle
            label="Scale errors by sqrt(reduced chi2)"
            checked={setup.scaleErrors}
            accent="amber"
            onChange={(checked) => storeSetup({ ...setup, scaleErrors: checked })}
          />
        </div>
        <table className="w-full text-[9px] font-mono" data-fit-parameters>
          <thead className="bg-zinc-900 text-zinc-500">
            <tr>
              <th className={LABEL_CELL_CLASS}>parameter</th>
              <th className={CELL_CLASS}>value</th>
              <th className={CELL_CLASS}>fit</th>
            </tr>
          </thead>
          <tbody className="text-zinc-300">
            {transitDraftNames(setup.kind).map((name) => {
              const label = transitDraftLabel(name, setup.kind);
              const draft = setup[name];
              return (
                <tr key={name} data-fit-parameter={name}>
                  <td id={`${rowsId}-${name}`} className={LABEL_CELL_CLASS}>
                    {label}
                  </td>
                  <td className={CELL_CLASS}>
                    <DecimalField
                      aria-label={label}
                      value={draft.text}
                      onChange={(text) => storeSetup(withDraft(setup, name, { ...draft, text }))}
                      className={INPUT_CLASS}
                    />
                  </td>
                  <td className={CELL_CLASS}>
                    <Toggle
                      label="free"
                      checked={draft.free}
                      accent="teal"
                      describedBy={`${rowsId}-${name}`}
                      onChange={(free) => storeSetup(withDraft(setup, name, { ...draft, free }))}
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <div className="text-[9px] text-zinc-600">{captionOf(setup.kind, model.xLabel)}</div>
        <div className="flex flex-wrap gap-1.5">
          <button
            type="button"
            onClick={() => {
              if (start !== null) storeSetup(setupWithStart(setup, start, model.offset));
            }}
            disabled={start === null}
            title={startTitle}
            className={SMALL_BUTTON_CLASS}
          >
            Start from the quick look
          </button>
          {fold !== null && (
            <button type="button" onClick={() => storeSetup(setupWithPeriod(setup, fold.periodDays))} className={SMALL_BUTTON_CLASS}>
              Use fold period
            </button>
          )}
          <button
            type="button"
            onClick={runFit}
            disabled={fitReason !== null || running}
            title={fitReason ?? undefined}
            className={SMALL_BUTTON_CLASS}
          >
            {running ? "Fitting..." : "Fit"}
          </button>
        </div>
        {fitReason !== null && (
          <div className={NOTICE_CLASS} data-fit-hint>
            {fitReason}
          </div>
        )}
        <ErrorAlert message={error} />
        {record.result !== null && record.request !== null && (
          <>
            {record.signature !== signature && (
              <div className={NOTICE_CLASS} data-stale-fit>
                {STALE_FIT}
              </div>
            )}
            {record.result.warnings.map((warning, k) => (
              <div key={k} className={NOTICE_CLASS} data-fit-warning>
                {warning}
              </div>
            ))}
            {series !== null && (
              <>
                <ProfilePlot
                  series={[
                    { x: series.time, y: series.flux, yErr: series.fluxErr ?? undefined, color: DATA_COLOR, label: "data / baseline", mode: "points" },
                    { x: series.curveTime, y: series.curveFlux, color: MODEL_COLOR, label: "model", mode: "line" },
                  ]}
                  xLabel={model.xLabel}
                  yLabel="flux / fitted baseline"
                  height={TRANSIT_FIT_PLOT_HEIGHT_PX}
                  toolbar
                  logToggle={false}
                  wheelZoom="modifier"
                  xTickFormat={jdOffsetLabel}
                  csvName={`${csvName}_transit_fit`}
                />
                <ProfilePlot
                  series={[{ x: series.time, y: series.residualPpm, color: DATA_COLOR, label: "residual", mode: "points" }]}
                  xLabel={model.xLabel}
                  yLabel="residual (ppm of the baseline)"
                  height={TRANSIT_RESIDUAL_PLOT_HEIGHT_PX}
                  toolbar
                  logToggle={false}
                  wheelZoom="modifier"
                  xTickFormat={jdOffsetLabel}
                  referenceLines={[{ axis: "y", value: 0 }]}
                  csvName={`${csvName}_transit_residuals`}
                />
              </>
            )}
            {fitResults(record.result, record.request)}
          </>
        )}
      </div>
    </details>
  );
}

export default memo(TransitFitSection);
