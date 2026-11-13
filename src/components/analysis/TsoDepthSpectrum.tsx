import { memo, useEffect, useId, useMemo, useRef, useState } from "react";
import ProfilePlot from "../regions/ProfilePlot";
import { DecimalField, ErrorAlert, RunButton, SMALL_BUTTON_CLASS, Toggle, WarningList } from "../ui";
import { useLightCurves, useLightCurveView } from "../../hooks/useLightCurveStore";
import { useTimedNotice } from "../../hooks/useTimedNotice";
import { useTransitFit } from "../../hooks/useTransitFitStore";
import { tsoStore, useTsoCurves, type TsoSectionProps } from "../../hooks/useTsoStore";
import { tsoTransitSpectrum } from "../../services/tsoTransit";
import type { TransitFitResult } from "../../shared/types/transit";
import type { TsoLightCurves } from "../../shared/types/tso";
import type { TsoTransitSpectrum } from "../../shared/types/tsoTransit";
import { APP_VERSION } from "../../utils/constants";
import { QUICK_LOOK_LABEL } from "../../utils/lightCurveAnalysis";
import { lightCurveModel } from "../../utils/lightCurveViewModel";
import { BAND_SPECIES, DEFAULT_BAND_SPECIES, toggleBandSpecies } from "../../utils/lineList";
import { transitFitInput, transitFitSignature } from "../../utils/transitFitSetup";
import { DEPTH_COLOR, bandBins, depthBandMarks, depthPlot, depthPlotBands, depthTableRows } from "../../utils/tsoDepthView";
import { WHITE_DOC_ID, depthSpectrum, tsoTimeName } from "../../utils/tsoLightCurves";
import {
  BASELINE_NAMES,
  EMPTY_TRANSIT_DRAFT,
  FIT_COLOR,
  QUICK_SERIES_LABEL,
  TRANSIT_FIT_LABEL,
  draftFromWhiteLight,
  fitFromDraft,
  fitPlotSeries,
  fitTableRows,
  fixedLine,
  isBlankDraft,
  leftOutRows,
  onDifferentScales,
  whiteLightValues,
  type TransitDraft,
} from "../../utils/tsoTransitSpectrum";
import { measurementLog } from "../../utils/measurementLog";
import { tsoDepthSpectrumEntries } from "../../utils/measurementLogTso";
import { saveCsvDialog } from "../../utils/spectrumExport";
import { tsoDepthSpectrumCsv, tsoDepthSpectrumCsvFileName } from "../../utils/tsoExports";
import { tsoTransitSpectrumCsv, tsoTransitSpectrumCsvFileName, tsoTransitSpectrumEntries } from "../../utils/tsoTransitExports";

const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const CAPTION_CLASS = "text-[9px] text-zinc-500";
const LINE_CLASS = "text-[10px] text-zinc-400";
const CHIP_COLOR = "#22d3ee";
const CELL_CLASS = "text-right px-1.5 py-0.5";
const PIXEL_LOSS_CELL_CLASS = "text-right px-1.5 py-0.5 text-amber-300";
const NOTICE_CLASS = "text-[9px] text-emerald-400/90 break-all";
const NOTICE_MS = 6000;
const LABEL_CLASS = "text-[9px] text-zinc-500";
const AMBER_LINE_CLASS = "text-[9px] text-amber-400/90";
const STALE_CLASS = "text-[10px] text-amber-300";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const REMARK_CLASS = "text-left px-1.5 py-0.5";
const FLAGGED_REMARK_CLASS = "text-left px-1.5 py-0.5 text-amber-300";
const LIST_CLASS = "text-[10px] font-mono text-zinc-500";
const QUICK_Y_LABEL = "depth (1 - F_in / F_out)";
const FIT_Y_LABEL = "(Rp/Rs)^2";
const BOTH_Y_LABEL = "depth";
const NO_WHITE_FIT_TITLE = "No white-light fit yet: run the transit fit on the white light in the view above.";
const UNUSABLE_WHITE_FIT_TITLE =
  "The white-light fit cannot be taken (an eclipse fit, no convergence or a value at its limit): fit the white light again in the view above.";
const NO_LEFT_OUT: readonly number[] = Object.freeze([]);

interface BandsState {
  group: string;
  species: readonly string[];
  binsAsked: boolean;
}

interface ExportNotice {
  group: string;
  kind: "saved" | "logged" | "fitSaved" | "fitLogged";
  text: string;
}

type DraftText = "periodText" | "t0Text" | "aRsText" | "bText" | "rpText" | "u1Text" | "u2Text";

interface FitRun {
  loading: boolean;
  error: string | null;
  spectrum: TsoTransitSpectrum | null;
  basis: TsoLightCurves | null;
}

interface SeriesShown {
  quick: boolean;
  fit: boolean;
}

interface FitBlockState {
  group: string;
  draft: TransitDraft;
  fit: FitRun;
  shown: SeriesShown;
}

const SERIES_SWITCHES: readonly { member: keyof SeriesShown; text: string; color: string }[] = [
  { member: "quick", text: "quick look", color: DEPTH_COLOR },
  { member: "fit", text: "model fit", color: FIT_COLOR },
];

function fitBlockStart(group: string): FitBlockState {
  return {
    group,
    draft: EMPTY_TRANSIT_DRAFT,
    fit: { loading: false, error: null, spectrum: null, basis: null },
    shown: { quick: true, fit: true },
  };
}

function withText(draft: TransitDraft, member: DraftText, text: string): TransitDraft {
  return { ...draft, [member]: text, fromWhiteLight: false };
}

function orbitFields(timeName: string): { member: DraftText; label: string; aria: string }[] {
  return [
    { member: "periodText", label: "period (d)", aria: "Orbital period in days" },
    { member: "t0Text", label: `t0 (${timeName})`, aria: "Mid-transit time" },
    { member: "aRsText", label: "a / Rs", aria: "Scaled semi-major axis a over Rs" },
    { member: "bText", label: "b", aria: "Impact parameter" },
    { member: "rpText", label: "Rp / Rs start", aria: "Starting radius ratio Rp over Rs" },
    { member: "u1Text", label: "u1", aria: "Limb darkening u1" },
    { member: "u2Text", label: "u2", aria: "Limb darkening u2" },
  ];
}

function TsoDepthSpectrum({ group, paths, set, series }: TsoSectionProps) {
  const docs = useLightCurves(group);
  const view = useLightCurveView(group);
  const curves = useTsoCurves(group);
  const [stored, setStored] = useState<BandsState>(() => ({ group, species: [...DEFAULT_BAND_SPECIES], binsAsked: false }));
  const bands = stored.group === group ? stored : { group, species: [...DEFAULT_BAND_SPECIES], binsAsked: false };
  if (bands !== stored) setStored(bands);
  const result = curves.result && curves.result.bins.length > 0 ? curves.result : null;
  const spectrum = useMemo(() => (result ? depthSpectrum(docs, view, result) : null), [docs, view, result]);
  const marks = useMemo(
    () => depthBandMarks(bands.species, series.wavelength_min_um, series.wavelength_max_um),
    [bands.species, series.wavelength_min_um, series.wavelength_max_um],
  );
  const [saveError, setSaveError] = useState<{ group: string; message: string } | null>(null);
  const [fitSaveError, setFitSaveError] = useState<{ group: string; message: string } | null>(null);
  const [notice, showNotice] = useTimedNotice<ExportNotice>(NOTICE_MS);
  const fieldsId = useId();
  const record = useTransitFit(group, WHITE_DOC_ID);
  const [fitStored, setFitStored] = useState<FitBlockState>(() => fitBlockStart(group));
  const fitBlock = fitStored.group === group ? fitStored : fitBlockStart(group);
  if (fitBlock !== fitStored) setFitStored(fitBlock);
  const { draft, fit, shown } = fitBlock;
  const sequence = useRef(0);
  const handed = useRef<TransitFitResult | null>(null);
  const white = docs.find((doc) => doc.id === WHITE_DOC_ID) ?? null;
  const handOver = useMemo(() => whiteLightValues(record.result), [record.result]);
  const whiteSignature = useMemo(
    () => (white ? transitFitSignature(transitFitInput(lightCurveModel(white, view))) : null),
    [white, view],
  );
  const whiteStale = record.result !== null && record.signature !== whiteSignature;
  const leftOut = useMemo(() => (result ? leftOutRows(white, view, result.row_index) : NO_LEFT_OUT), [white, view, result]);
  const input = useMemo(() => fitFromDraft(draft, leftOut), [draft, leftOut]);
  const handOverInputs = useRef({ handOver, whiteStale, draft });
  handOverInputs.current = { handOver, whiteStale, draft };

  useEffect(() => {
    handed.current = null;
    sequence.current += 1;
  }, [group]);

  useEffect(() => {
    const { handOver: values, whiteStale: stale, draft: current } = handOverInputs.current;
    if (values === null || stale || !isBlankDraft(current) || record.result === handed.current) return;
    handed.current = record.result;
    setFitStored((previous) => ({ ...previous, draft: draftFromWhiteLight(values, previous.draft) }));
  }, [group, record.result]);

  const title = <div className={SECTION_CLASS}>Depth spectrum</div>;

  if (!spectrum || !result) {
    return (
      <div className="px-3 py-2 space-y-2" data-tso-depth>
        {title}
        <div className={CAPTION_CLASS}>Add wavelength bins and compute the light curves to get a depth spectrum.</div>
      </div>
    );
  }

  const header = (
    <>
      {title}
      <div className="text-[9px] text-zinc-500" data-tso-quick-look>
        {QUICK_LOOK_LABEL}
      </div>
    </>
  );

  const quick = spectrum.window === null ? null : spectrum;
  const quickPlot = quick ? depthPlot(quick) : null;
  const current = fit.spectrum !== null && fit.basis === result ? fit.spectrum : null;
  const fitSeries = current ? fitPlotSeries(current) : null;
  const bothSeries = quickPlot !== null && fitSeries !== null;
  const drawQuick = quickPlot !== null && (shown.quick || fitSeries === null);
  const drawFit = fitSeries !== null && (shown.fit || quickPlot === null);
  const plotSeries = [
    ...(drawQuick ? quickPlot.series.map((line) => (fitSeries ? { ...line, label: QUICK_SERIES_LABEL } : line)) : []),
    ...(drawFit ? [fitSeries] : []),
  ];
  const yLabel = drawQuick ? (drawFit ? BOTH_Y_LABEL : QUICK_Y_LABEL) : FIT_Y_LABEL;
  const showBands = quick !== null || drawFit;
  const fitTable = current ? fitTableRows(current, quick) : [];
  const warningLines = fitTable.flatMap((row) => row.warnings.map((warning) => `${row.band}: ${warning}`));
  const bins = bandBins(marks);
  const hasDepth = spectrum.white?.depth != null || spectrum.rows.some((depthRow) => depthRow.depth !== null);
  const shownNotice = notice?.group === group ? notice : null;

  const saveDepths = async () => {
    setSaveError(null);
    const context = { software: `AstroBurst ${APP_VERSION}`, exportedUtc: new Date().toISOString(), files: set.files };
    let path: string | null;
    try {
      path = await saveCsvDialog(
        tsoDepthSpectrumCsv(spectrum, result, view, context),
        tsoDepthSpectrumCsvFileName(result, set.files[0]?.file_name ?? ""),
        "Save depth spectrum",
      );
    } catch (e) {
      setSaveError({ group, message: `Could not save the file: ${e instanceof Error ? e.message : String(e)}` });
      return;
    }
    if (path === null) return;
    for (const draft of tsoDepthSpectrumEntries(spectrum, result, view, set.files, path)) measurementLog.append(draft);
    showNotice({ group, kind: "saved", text: `Saved ${path}` });
  };

  const logDepths = () => {
    setSaveError(null);
    const drafts = tsoDepthSpectrumEntries(spectrum, result, view, set.files, null);
    for (const draft of drafts) measurementLog.append(draft);
    const text = drafts.length === 1 ? "Added 1 row to the measurement log" : `Added ${drafts.length} rows to the measurement log`;
    showNotice({ group, kind: "logged", text });
  };

  const fitHasDepth = current !== null && current.bins.some((bin) => bin.depth !== null);

  const saveFit = async (fitted: TsoTransitSpectrum) => {
    setFitSaveError(null);
    const context = { software: `AstroBurst ${APP_VERSION}`, exportedUtc: new Date().toISOString(), files: set.files };
    let path: string | null;
    try {
      path = await saveCsvDialog(
        tsoTransitSpectrumCsv(fitted, quick, context),
        tsoTransitSpectrumCsvFileName(fitted, set.files[0]?.file_name ?? ""),
        "Save fitted spectrum",
      );
    } catch (e) {
      setFitSaveError({ group, message: `Could not save the file: ${e instanceof Error ? e.message : String(e)}` });
      return;
    }
    if (path === null) return;
    for (const entry of tsoTransitSpectrumEntries(fitted, set.files, path)) measurementLog.append(entry);
    showNotice({ group, kind: "fitSaved", text: `Saved ${path}` });
  };

  const logFit = (fitted: TsoTransitSpectrum) => {
    setFitSaveError(null);
    const entries = tsoTransitSpectrumEntries(fitted, set.files, null);
    for (const entry of entries) measurementLog.append(entry);
    const text = entries.length === 1 ? "Added 1 row to the measurement log" : `Added ${entries.length} rows to the measurement log`;
    showNotice({ group, kind: "fitLogged", text });
  };

  const editDraft = (change:(previous: TransitDraft) => TransitDraft) =>
    setFitStored((previous) => ({ ...previous, draft: change(previous.draft) }));

  const takeWhiteLight = () => {
    if (handOver === null) return;
    handed.current = record.result;
    editDraft((previous) => draftFromWhiteLight(handOver, previous));
  };

  const runFit = () => {
    const request = tsoStore.getRequest(group);
    const fitRequest = input.fit;
    if (fitRequest === null || request === null) return;
    const seq = ++sequence.current;
    setFitStored((previous) => ({ ...previous, fit: { ...previous.fit, loading: true, error: null } }));
    tsoTransitSpectrum([...paths], series.key, request, fitRequest).then(
      (answer) => {
        if (seq !== sequence.current) return;
        setFitStored((previous) => ({ ...previous, fit: { loading: false, error: null, spectrum: answer, basis: result } }));
      },
      (e: unknown) => {
        if (seq !== sequence.current) return;
        const error = e instanceof Error ? e.message : String(e);
        setFitStored((previous) => ({ ...previous, fit: { ...previous.fit, loading: false, error } }));
      },
    );
  };

  const flipSeries = (member: keyof SeriesShown) =>
    setFitStored((previous) => {
      const next = { ...previous.shown, [member]: !previous.shown[member] };
      return next.quick || next.fit ? { ...previous, shown: next } : previous;
    });

  const fitSection = (
    <div className="space-y-2" data-tso-fit>
      <div className={SECTION_CLASS}>Model fit</div>
      <div className={CAPTION_CLASS} data-tso-fit-label>
        {TRANSIT_FIT_LABEL}
      </div>
      {series.time_scale === "index" ? (
        <div className={LINE_CLASS} data-tso-fit-no-time>
          A transit fit needs a time axis: this series is on integration numbers.
        </div>
      ) : (
        <>
          {isBlankDraft(draft) && (
            <div className={CAPTION_CLASS} data-tso-fit-hint>
              Fit the white light in the light-curve view above, then take its values here, or type the orbit.
            </div>
          )}
          {draft.fromWhiteLight && (
            <div className={CAPTION_CLASS} data-tso-fit-origin>
              Values taken from the white-light fit.
            </div>
          )}
          <div className="space-y-1">
            <button
              type="button"
              className={SMALL_BUTTON_CLASS}
              disabled={handOver === null}
              title={handOver !== null ? undefined : record.result === null ? NO_WHITE_FIT_TITLE : UNUSABLE_WHITE_FIT_TITLE}
              onClick={takeWhiteLight}
            >
              Take the white-light fit
            </button>
            {whiteStale && (
              <div className={AMBER_LINE_CLASS} data-tso-fit-white-stale>
                The white light changed since its transit fit: fit it again in the view above before taking its values.
              </div>
            )}
          </div>
          <div className="grid grid-cols-4 gap-1.5">
            {orbitFields(tsoTimeName(series.time_scale)).map(({ member, label, aria }) => (
              <div key={member} className="flex flex-col gap-0.5">
                <label htmlFor={`${fieldsId}-${member}`} className={LABEL_CLASS}>
                  {label}
                </label>
                <DecimalField
                  id={`${fieldsId}-${member}`}
                  optional
                  aria-label={aria}
                  value={draft[member]}
                  onChange={(text) => editDraft((previous) => withText(previous, member, text))}
                  className={INPUT_CLASS}
                />
              </div>
            ))}
          </div>
          <div className={CAPTION_CLASS}>
            Period, t0, a / Rs and b stay fixed in every bin; Rp / Rs is fitted in every bin from this starting value.
          </div>
          <Toggle
            label="Fit u1 and u2 in every bin"
            checked={draft.limbFree}
            onChange={(on) => editDraft((previous) => ({ ...previous, limbFree: on }))}
          />
          <Toggle
            label="Scale errors by sqrt(reduced chi2)"
            checked={draft.scaleErrors}
            onChange={(on) => editDraft((previous) => ({ ...previous, scaleErrors: on }))}
          />
          <div className="flex flex-col gap-0.5">
            <label htmlFor={`${fieldsId}-baseline`} className={LABEL_CLASS}>
              Baseline
            </label>
            <select
              id={`${fieldsId}-baseline`}
              aria-label="Baseline polynomial"
              value={String(draft.baselineOrder)}
              onChange={(e) => editDraft((previous) => ({ ...previous, baselineOrder: Number(e.target.value) as TransitDraft["baselineOrder"] }))}
              className={SELECT_CLASS}
            >
              {BASELINE_NAMES.map((name, order) => (
                <option key={name} value={String(order)}>
                  {name}
                </option>
              ))}
            </select>
          </div>
          {leftOut.length > 0 && (
            <div className={CAPTION_CLASS} data-tso-fit-left-out>
              {`${leftOut.length} integration(s) left out in the light-curve view are left out of every bin.`}
            </div>
          )}
          <RunButton label="Fit every bin" runningLabel="Fitting every bin" running={fit.loading} disabled={input.fit === null} onClick={runFit} />
          {input.error !== null && !isBlankDraft(draft) && (
            <div className={AMBER_LINE_CLASS} data-field-error>
              {input.error}
            </div>
          )}
          <ErrorAlert message={fit.error} />
          {fit.spectrum !== null && current === null && (
            <div className={STALE_CLASS} data-tso-fit-stale>
              The light curves changed since this fit: fit again.
            </div>
          )}
          {current !== null && (
            <>
              <div className={LINE_CLASS} data-tso-fit-fixed>
                {fixedLine(current)}
              </div>
              <WarningList warnings={current.warnings} />
              <div className="overflow-x-auto" data-tso-fit-table>
                <table className="w-full text-[9px] font-mono">
                  <thead className="text-zinc-500">
                    <tr>
                      <th className="text-left px-1.5 py-0.5">band</th>
                      <th className={CELL_CLASS}>(Rp/Rs)^2</th>
                      <th className={CELL_CLASS}>error</th>
                      <th className={CELL_CLASS}>Rp/Rs</th>
                      <th className={CELL_CLASS}>error</th>
                      <th className={CELL_CLASS}>chi2 / dof</th>
                      <th className={CELL_CLASS}>in transit / points</th>
                      <th className={CELL_CLASS}>quick look</th>
                      <th className={REMARK_CLASS}>remark</th>
                    </tr>
                  </thead>
                  <tbody className="text-zinc-300">
                    {fitTable.map((row) => (
                      <tr key={row.key}>
                        <td className="text-left px-1.5 py-0.5 whitespace-nowrap">{row.band}</td>
                        <td className={CELL_CLASS}>{row.depth}</td>
                        <td className={CELL_CLASS}>{row.depthErr}</td>
                        <td className={CELL_CLASS}>{row.rp}</td>
                        <td className={CELL_CLASS}>{row.rpErr}</td>
                        <td className={CELL_CLASS}>{row.chi2}</td>
                        <td className={CELL_CLASS}>{row.points}</td>
                        <td className={CELL_CLASS}>{row.quick}</td>
                        <td className={row.flagged || row.failed ? FLAGGED_REMARK_CLASS : REMARK_CLASS}>{row.remark}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {warningLines.length > 0 && (
                <details className={LIST_CLASS} data-tso-fit-warnings>
                  <summary className="cursor-pointer select-none">{`fit warnings (${warningLines.length})`}</summary>
                  <ul className="mt-0.5 flex flex-col gap-0.5">
                    {warningLines.map((line, i) => (
                      <li key={i} className="break-words text-amber-300/90">
                        {line}
                      </li>
                    ))}
                  </ul>
                </details>
              )}
              {current.notes.length > 0 && (
                <details className={LIST_CLASS} data-tso-fit-notes>
                  <summary className="cursor-pointer select-none">{`notes (${current.notes.length})`}</summary>
                  <ul className="mt-0.5 flex flex-col gap-0.5">
                    {current.notes.map((note, i) => (
                      <li key={i} className="break-words">
                        {note}
                      </li>
                    ))}
                  </ul>
                </details>
              )}
              <div className="space-y-1" data-tso-fit-actions>
                <div className="flex items-center gap-1">
                  <button
                    type="button"
                    className={SMALL_BUTTON_CLASS}
                    disabled={!fitHasDepth || fit.loading}
                    onClick={() => void saveFit(current)}
                  >
                    Save CSV
                  </button>
                  <button type="button" className={SMALL_BUTTON_CLASS} disabled={!fitHasDepth || fit.loading} onClick={() => logFit(current)}>
                    Log
                  </button>
                </div>
                <ErrorAlert message={fitSaveError?.group === group ? fitSaveError.message : null} />
                {shownNotice?.kind === "fitSaved" && (
                  <div role="status" className={NOTICE_CLASS} data-tso-fit-saved>
                    {shownNotice.text}
                  </div>
                )}
                {shownNotice?.kind === "fitLogged" && (
                  <div role="status" className={NOTICE_CLASS} data-tso-fit-logged>
                    {shownNotice.text}
                  </div>
                )}
              </div>
            </>
          )}
        </>
      )}
    </div>
  );

  return (
    <div className="px-3 py-2 space-y-2" data-tso-depth>
      {header}
      {quick === null && (
        <div className={LINE_CLASS} data-tso-no-window>
          Set a transit window in the light-curve view above: the depth of every bin is then drawn against wavelength.
        </div>
      )}
      <WarningList warnings={spectrum.warnings} />
      {showBands && (
        <div className="space-y-1" data-tso-bands>
          <div className="flex flex-wrap items-center gap-1.5 text-[10px] font-mono">
            <span className="text-zinc-400">Bands</span>
            {BAND_SPECIES.map((name) => {
              const enabled = bands.species.includes(name);
              return (
                <button
                  key={name}
                  type="button"
                  aria-pressed={enabled}
                  onClick={() => setStored({ group, species: toggleBandSpecies(bands.species, name), binsAsked: false })}
                  className="px-2 py-0.5 rounded border transition-colors"
                  style={{
                    borderColor: enabled ? CHIP_COLOR : "var(--ab-border)",
                    color: enabled ? CHIP_COLOR : "var(--ab-text-3)",
                  }}
                >
                  {name}
                </button>
              );
            })}
          </div>
          <p className={CAPTION_CLASS}>
            Shaded ranges are where each species absorbs (vacuum, observed frame); they are a guide, not an identification.
          </p>
        </div>
      )}
      {bothSeries && (
        <div className="flex flex-wrap items-center gap-1.5 text-[10px] font-mono" data-tso-series-switches>
          <span className="text-zinc-400">Show</span>
          {SERIES_SWITCHES.map(({ member, text, color }) => (
            <button
              key={member}
              type="button"
              aria-pressed={shown[member]}
              onClick={() => flipSeries(member)}
              className="px-2 py-0.5 rounded border transition-colors"
              style={{
                borderColor: shown[member] ? color : "var(--ab-border)",
                color: shown[member] ? color : "var(--ab-text-3)",
              }}
            >
              {text}
            </button>
          ))}
        </div>
      )}
      {bothSeries && current !== null && onDifferentScales(current) && (
        <div className={AMBER_LINE_CLASS} data-tso-scale-note>
          The two series are on different scales here (grazing transit): switch one off to read the other.
        </div>
      )}
      {plotSeries.length > 0 && (
        <ProfilePlot
          series={plotSeries}
          referenceLines={drawQuick ? quickPlot.referenceLines : []}
          bands={depthPlotBands(marks)}
          xLabel="wavelength (um)"
          yLabel={yLabel}
          height={170}
          toolbar
          logToggle={false}
          csvName="tso_depth_spectrum"
        />
      )}
      {quick !== null && quickPlot === null && !drawFit && (
        <div className={LINE_CLASS}>No bin has a depth: the transit window holds no point, or no point is left outside it.</div>
      )}
      {quick !== null && (
        <>
          <div className="overflow-x-auto" data-tso-depth-table>
            <table className="w-full text-[9px] font-mono">
              <thead className="text-zinc-500">
                <tr>
                  <th className="text-left px-1.5 py-0.5">band</th>
                  <th className={CELL_CLASS}>depth</th>
                  <th className={CELL_CLASS}>error</th>
                  <th className={CELL_CLASS}>N in / out</th>
                  <th className={CELL_CLASS}>NPIXELS in / out</th>
                </tr>
              </thead>
              <tbody className="text-zinc-300">
                {depthTableRows(spectrum).map((row) => (
                  <tr key={row.key}>
                    <td className="text-left px-1.5 py-0.5 whitespace-nowrap">{row.band}</td>
                    <td className={CELL_CLASS}>{row.depth}</td>
                    <td className={CELL_CLASS}>{row.error}</td>
                    <td className={CELL_CLASS}>{row.counts}</td>
                    <td className={row.pixelLoss ? PIXEL_LOSS_CELL_CLASS : CELL_CLASS}>{row.pixels}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="space-y-1" data-tso-depth-actions>
            <div className="flex items-center gap-1">
              <button type="button" className={SMALL_BUTTON_CLASS} disabled={!hasDepth} onClick={() => void saveDepths()}>
                Save CSV
              </button>
              <button type="button" className={SMALL_BUTTON_CLASS} disabled={!hasDepth} onClick={logDepths}>
                Log
              </button>
            </div>
            <ErrorAlert message={saveError?.group === group ? saveError.message : null} />
            {shownNotice?.kind === "saved" && (
              <div role="status" className={NOTICE_CLASS} data-tso-saved>
                {shownNotice.text}
              </div>
            )}
            {shownNotice?.kind === "logged" && (
              <div role="status" className={NOTICE_CLASS} data-tso-logged>
                {shownNotice.text}
              </div>
            )}
          </div>
        </>
      )}
      {showBands && (
        <>
          <button
            type="button"
            disabled={bins.length === 0}
            onClick={() => {
              tsoStore.patchDraft(group, { extraBins: bins });
              setStored({ group, species: bands.species, binsAsked: true });
            }}
            className="px-2 py-0.5 rounded border border-cyan-600/30 bg-cyan-600/10 text-[10px] text-cyan-300 hover:bg-cyan-600/20 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          >
            Use the shown bands as bins
          </button>
          {bands.binsAsked && (
            <div className={CAPTION_CLASS} data-tso-bands-hint>
              Compute the light curves again to get one curve per band.
            </div>
          )}
        </>
      )}
      {fitSection}
    </div>
  );
}

export default memo(TsoDepthSpectrum);
