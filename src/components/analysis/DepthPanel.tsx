import { memo, useEffect, useId, useState } from "react";
import { DecimalNumberField, ErrorAlert, RunButton, SMALL_BUTTON_CLASS, WarningList } from "../ui";
import { depthKey, depthStore, useDepth } from "../../hooks/useDepthStore";
import { useRegionKey } from "../../hooks/useRegionKey";
import { useHeaderSeeing, useImagePsf, useResolutionSettings } from "../../hooks/useResolutionSettings";
import { useMeasurementProvenance } from "../../hooks/useMeasurementLog";
import { useTimedFlag } from "../../hooks/useTimedNotice";
import type { ImageDepth } from "../../shared/types/depth";
import { APP_VERSION } from "../../utils/constants";
import { summedPixelUnit } from "../../utils/csvProvenance";
import { depthCsv, depthCsvFileName } from "../../utils/depthExport";
import { DEPTH_LAYER_ID, DEPTH_LAYER_KIND, createDepthPainter } from "../../utils/depthOverlay";
import { measurementLog } from "../../utils/measurementLog";
import { imageDepthEntry } from "../../utils/measurementLogQa";
import { overlayStore } from "../../utils/overlayStore";
import { psfChoice, significant } from "../../utils/resolutionText";
import { saveCsvDialog } from "../../utils/spectrumExport";
import { DEFAULT_DEPTH_FIELDS, depthOptions, type DepthFields, type DepthRadiusUnit } from "./depthPanelModel";

export interface DepthPanelProps {
  filePath: string | null;
  measureKey?: string | null;
}

const RADIUS_UNITS: readonly DepthRadiusUnit[] = ["arcsec", "px"];
const COPIED_FEEDBACK_MS = 1500;
const PIXEL_SPREAD_SHOWN_ABOVE = 1.05;
const MAX_SEED = 4294967295;

const INTRO =
  "How faint a source this image can show: the scatter of empty apertures placed between the sources, compared with the errors the file states.";
const FORMAL_SIGMA_TITLE = "Median over the apertures of the error the error layer gives";
const F_CORR_TITLE =
  "Empty-aperture scatter over the formal error: the errors of the file are this many times too small for an aperture of this size";
const CLIPPED_SIGMA_TITLE = "Standard deviation after 3 sigma clipping; the limit uses the MAD value";
const PSF_LIMIT_TITLE = "No correlated-noise correction; needs an error layer and a PSF FWHM";

const LABEL_CLASS = "text-[9px] text-zinc-500";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-cyan-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono";
const CHECK_LABEL_CLASS = "flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer";

function Card({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="bg-zinc-900/80 rounded px-2 py-1.5" title={title ?? value}>
      <div className="text-[9px] text-zinc-500 uppercase">{label}</div>
      <div className="text-[10px] font-mono text-zinc-200 break-words">{value}</div>
    </div>
  );
}

function microJansky(jansky: number | null): string {
  return significant(jansky === null ? null : jansky * 1e6);
}

function headlineValue(depth: ImageDepth, sumUnit: string): string {
  if (depth.limit_ab !== null) return `${depth.limit_ab.toFixed(2)} AB mag (${microJansky(depth.limit_jy)} uJy)`;
  if (depth.limit_jy !== null) return `${microJansky(depth.limit_jy)} uJy`;
  return `${significant(depth.limit_native)} ${sumUnit} (not calibrated)`;
}

function psfLimitValue(depth: ImageDepth, sumUnit: string): string {
  if (depth.psf_limit_formal_ab !== null) {
    return `${depth.psf_limit_formal_ab.toFixed(2)} AB mag at FWHM ${significant(depth.psf_fwhm_arcsec)} arcsec`;
  }
  if (depth.psf_limit_formal_native !== null) return `${significant(depth.psf_limit_formal_native)} ${sumUnit}`;
  return "--";
}

function pixelSigmaValue(depth: ImageDepth, unit: string): string {
  if (depth.sigma_pix_median_native === null) return "--";
  const base = `${significant(depth.sigma_pix_median_native)} ${unit}`;
  const spread = depth.sigma_pix_spread;
  return spread !== null && spread > PIXEL_SPREAD_SHOWN_ABOVE ? `${base}, x${spread.toFixed(2)} across the image` : base;
}

export function DepthResultView({ depth }: { depth: ImageDepth }) {
  const unit = depth.bunit ?? "image units";
  const sumUnit = summedPixelUnit(depth.bunit) ?? "image units x pix";
  const cards: { label: string; value: string; title?: string }[] = [
    { label: "empty-aperture sigma", value: `${significant(depth.sigma_empty_native)} ${sumUnit}` },
    {
      label: "formal sigma",
      value: depth.sigma_formal_native === null ? "--" : `${significant(depth.sigma_formal_native)} ${sumUnit}`,
      title: FORMAL_SIGMA_TITLE,
    },
    { label: "f_corr", value: depth.f_corr === null ? "--" : depth.f_corr.toFixed(2), title: F_CORR_TITLE },
    { label: "clipped sigma", value: `${significant(depth.sigma_clipped_native)} ${sumUnit}`, title: CLIPPED_SIGMA_TITLE },
    { label: "apertures", value: `${depth.n_used} used, ${depth.n_placed} placed, ${depth.n_requested} requested` },
    { label: "masked", value: `${(100 * depth.masked_fraction).toFixed(1)} percent` },
    { label: "point-source limit (formal)", value: psfLimitValue(depth, sumUnit), title: PSF_LIMIT_TITLE },
    { label: "pixel sigma", value: pixelSigmaValue(depth, unit) },
    { label: "calibration", value: depth.calibration_origin ?? "none" },
    { label: "error layer", value: depth.err_origin ?? "none" },
    { label: "mask", value: depth.dq_source ?? "none" },
  ];
  return (
    <div data-depth-result className="space-y-1.5">
      <div className="bg-zinc-900/80 rounded px-2 py-1.5">
        <div className="text-[9px] text-zinc-500 uppercase">{`${depth.n_sigma} sigma depth`}</div>
        <div className="text-sm font-mono text-zinc-100 break-words">{headlineValue(depth, sumUnit)}</div>
      </div>
      <div className="text-[10px] text-zinc-400 break-words">{depth.definition}</div>
      <div className="text-[10px] text-zinc-500">{`scatter known to ${(100 * depth.sigma_empty_rel_err).toFixed(1)} percent`}</div>
      <div className="grid grid-cols-2 gap-1.5">
        {cards.map((card) => (
          <Card key={card.label} label={card.label} value={card.value} title={card.title} />
        ))}
      </div>
      <WarningList warnings={depth.warnings} />
      <details className="text-[10px] text-zinc-500">
        <summary className="cursor-pointer">{`notes (${depth.notes.length})`}</summary>
        <ul className="mt-1 space-y-0.5 list-disc pl-4">
          {depth.notes.map((note) => (
            <li key={note}>{note}</li>
          ))}
        </ul>
      </details>
    </div>
  );
}

export function DepthPanelInner({ filePath, measureKey }: DepthPanelProps) {
  const [fields, setFields] = useState<DepthFields>(DEFAULT_DEPTH_FIELDS);
  const [showApertures, setShowApertures] = useState(true);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [copied, flagCopied] = useTimedFlag(COPIED_FEEDBACK_MS);
  const key = filePath ? depthKey(filePath, measureKey) : null;
  const state = useDepth(key);
  const overlayKey = useRegionKey();
  const provenance = useMeasurementProvenance();
  const settings = useResolutionSettings();
  const imagePsf = useImagePsf(filePath);
  const headerSeeing = useHeaderSeeing(settings.psfSource === "header" ? filePath : null);
  const resolutionFwhm = psfChoice(settings, imagePsf.psf, headerSeeing).fwhmArcsec;
  const radiusId = useId();
  const aperturesId = useId();
  const sigmaId = useId();
  const seedId = useId();
  const detectId = useId();
  const psfId = useId();
  const depth = state.depth;

  useEffect(() => {
    if (!overlayKey || !showApertures || !depth) return;
    overlayStore.add(overlayKey, {
      id: DEPTH_LAYER_ID,
      kind: DEPTH_LAYER_KIND,
      visible: true,
      paint: createDepthPainter(depth.apertures, depth.radius_px),
    });
    return () => overlayStore.remove(overlayKey, DEPTH_LAYER_ID);
  }, [overlayKey, showApertures, depth]);

  const update = (patch: Partial<DepthFields>) => setFields((current) => ({ ...current, ...patch }));

  const measure = () => {
    if (key === null || !filePath) return;
    void depthStore.measure(key, filePath, depthOptions(fields));
  };

  const csvText = (shown: ImageDepth) =>
    depthCsv(shown, { fileName: provenance.file, appVersion: APP_VERSION, exportedUtc: new Date().toISOString() });

  const copyCsv = (shown: ImageDepth) => {
    navigator.clipboard?.writeText(csvText(shown));
    flagCopied();
  };

  const saveCsv = async (shown: ImageDepth) => {
    setSaveError(null);
    try {
      await saveCsvDialog(csvText(shown), depthCsvFileName(filePath), "Save the empty apertures");
    } catch (e: unknown) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="ab-panel overflow-hidden" data-depth-panel>
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Depth</span>
      </div>

      <div className="px-3 py-2 space-y-2">
        <p className="text-[10px] text-zinc-500">{INTRO}</p>

        <div className="grid grid-cols-2 gap-2">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={radiusId} className={LABEL_CLASS}>
              Aperture radius
            </label>
            <div className="flex items-center gap-1">
              <DecimalNumberField
                id={radiusId}
                data-field="radius"
                optional
                min={0.001}
                max={1000}
                placeholder="1.0"
                value={fields.radius}
                onValueChange={(radius) => update({ radius })}
                className={INPUT_CLASS}
              />
              <select
                data-field="radiusUnit"
                aria-label="Aperture radius unit"
                value={fields.radiusUnit}
                onChange={(e) => update({ radiusUnit: e.target.value as DepthRadiusUnit })}
                className={SELECT_CLASS}
              >
                {RADIUS_UNITS.map((unit) => (
                  <option key={unit} value={unit}>
                    {unit}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={aperturesId} className={LABEL_CLASS}>
              Apertures
            </label>
            <DecimalNumberField
              id={aperturesId}
              data-field="nApertures"
              integer
              min={10}
              max={5000}
              value={fields.nApertures}
              onValueChange={(nApertures) => nApertures !== null && update({ nApertures })}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={sigmaId} className={LABEL_CLASS}>
              Significance (sigma)
            </label>
            <DecimalNumberField
              id={sigmaId}
              data-field="nSigma"
              min={0.1}
              max={100}
              value={fields.nSigma}
              onValueChange={(nSigma) => nSigma !== null && update({ nSigma })}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={seedId} className={LABEL_CLASS}>
              Seed
            </label>
            <DecimalNumberField
              id={seedId}
              data-field="seed"
              integer
              min={0}
              max={MAX_SEED}
              value={fields.seed}
              onValueChange={(seed) => seed !== null && update({ seed })}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={detectId} className={LABEL_CLASS}>
              Source mask threshold (sigma)
            </label>
            <DecimalNumberField
              id={detectId}
              data-field="detectNsigma"
              min={1}
              max={10}
              value={fields.detectNsigma}
              onValueChange={(detectNsigma) => detectNsigma !== null && update({ detectNsigma })}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={psfId} className={LABEL_CLASS}>
              PSF FWHM (arcsec, optional)
            </label>
            <DecimalNumberField
              id={psfId}
              data-field="psfFwhmArcsec"
              optional
              min={0.001}
              max={60}
              value={fields.psfFwhmArcsec}
              onValueChange={(psfFwhmArcsec) => update({ psfFwhmArcsec })}
              className={INPUT_CLASS}
            />
            <button
              type="button"
              data-action="use-resolution-psf"
              disabled={resolutionFwhm === null}
              onClick={() => update({ psfFwhmArcsec: resolutionFwhm })}
              className={SMALL_BUTTON_CLASS}
            >
              Use the Resolution PSF
            </button>
          </div>
        </div>

        <label className={CHECK_LABEL_CLASS}>
          <input type="checkbox" data-field="useMask" checked={fields.useMask} onChange={(e) => update({ useMask: e.target.checked })} />
          Use the mask of the file
        </label>
        <label className={CHECK_LABEL_CLASS}>
          <input type="checkbox" data-field="showApertures" checked={showApertures} onChange={(e) => setShowApertures(e.target.checked)} />
          Show apertures on the image
        </label>

        <RunButton label="Measure depth" runningLabel="Measuring…" running={state.loading} disabled={!filePath} small onClick={measure} />

        <ErrorAlert message={state.error} />

        {depth && (
          <>
            <DepthResultView depth={depth} />
            <div className="flex flex-wrap items-center gap-1.5">
              <button
                type="button"
                data-action="copy-depth-csv"
                disabled={state.loading}
                onClick={() => copyCsv(depth)}
                className={SMALL_BUTTON_CLASS}
              >
                {copied ? "copied" : "Copy CSV"}
              </button>
              <button
                type="button"
                data-action="save-depth-csv"
                disabled={state.loading}
                onClick={() => void saveCsv(depth)}
                className={SMALL_BUTTON_CLASS}
              >
                Save CSV
              </button>
              <button
                type="button"
                data-action="log-depth"
                disabled={state.loading}
                onClick={() => measurementLog.append(imageDepthEntry(provenance, depth))}
                className={SMALL_BUTTON_CLASS}
              >
                Log
              </button>
            </div>
            <ErrorAlert message={saveError} compact />
          </>
        )}
      </div>
    </div>
  );
}

export default memo(DepthPanelInner);
