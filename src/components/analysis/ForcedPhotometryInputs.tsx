import { memo, useCallback, useId, useState, useSyncExternalStore } from "react";
import { FileUp, X } from "lucide-react";
import { DecimalNumberField, SMALL_BUTTON_CLASS, Toggle } from "../ui";
import { depthStore } from "../../hooks/useDepthStore";
import { forcedPhotometryStore } from "../../hooks/useForcedPhotometryStore";
import { FORCED_MAX_BANDS } from "../../shared/types/forcedPhotometry";
import {
  bandDisplayName,
  noiseFactorOffer,
  type ForcedFields,
  type ForcedPositionsParse,
  type ForcedRadiusUnit,
  type ForcedZeroPointChoice,
} from "../../utils/forcedPhotometryRequest";
import { noiseOfferText } from "./forcedPhotometryInputsModel";

export interface ForcedInputFile {
  name: string;
  path: string;
}

export interface ForcedPhotometryInputsProps {
  fields: ForcedFields;
  positions: ForcedPositionsParse;
  files: readonly ForcedInputFile[];
  disabled?: boolean;
}

export interface BandNoiseOfferProps {
  path: string;
  label: string;
  fCorr: number | null;
  radius: number | null;
  unit: ForcedRadiusUnit;
  disabled?: boolean;
}

export const MAX_SHOWN_POSITION_ERRORS = 5;

const RADIUS_UNITS: readonly ForcedRadiusUnit[] = ["arcsec", "px"];
const ZERO_POINT_SYSTEMS: readonly { value: ForcedZeroPointChoice["system"]; label: string }[] = [
  { value: "ab", label: "AB" },
  { value: "vega", label: "Vega" },
];
const TYPED_ZERO_POINT: Omit<ForcedZeroPointChoice, "zp"> = Object.freeze({ system: "ab", perSecond: false });
const TEXTAREA_ROWS = 4;

const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const LABEL_CLASS = "text-[9px] text-zinc-500";
const HINT_CLASS = "text-[10px] text-zinc-600";
const TEXTAREA_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-cyan-500/50 w-full resize-y";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-cyan-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono";
const ERRORS_CLASS =
  "text-[9px] text-amber-300/90 bg-amber-900/15 border border-amber-800/30 rounded px-2 py-1 space-y-0.5 max-h-24 overflow-auto";

const POSITIONS_LABEL = "RA Dec [id] per line, or CSV with ra, dec and id columns";
const POSITIONS_PLACEHOLDER = "53.13874 -28.08539 star1";
const POSITIONS_HINT =
  "Decimal values are ICRS degrees; sexagesimal RA is hours unless written with d. Each position is measured where it is given: nothing is recentred.";
const BANDS_EMPTY =
  "Open the image of each band in Files, then add it here. Every band keeps its own pixel grid: nothing is resampled.";
const BANDS_HINT =
  "Label names the band in the table. Dataset names the filter set, for example DECAM, when the header does not. Noise factor is f_corr of the Depth section, measured with the same aperture.";
const ANNULUS_CAPTION = "same unit as the radius; empty: no local sky is subtracted, as survey coadds need";
const GROWTH_LABEL = "Correct to total flux with the growth curve of the stars of each band";
const GAIN_TITLE = "Photon noise of the source; used only when a sky annulus is measured";
const PER_SECOND_TITLE =
  "The zero point is for one count per second: the backend divides the counts by the exposure time of the header";
const ZERO_POINT_CAPTIONS: Record<ForcedZeroPointChoice["system"], string> = {
  ab: "Used only when the header of this band gives no calibration: flux in Jy = image unit x 10^((8.90 - zero point) / 2.5).",
  vega: "Used only when the header of this band gives no calibration. A Vega zero point gives no Jy and no AB magnitude: the band stays out of the colours and of the SED.",
};

function positionsCount(positions: ForcedPositionsParse): string {
  const n = positions.positions.length;
  const count = n === 1 ? "1 position" : `${n} positions`;
  return positions.total > n ? `${count} of ${positions.total} read` : count;
}

export function BandNoiseOffer({ path, label, fCorr, radius, unit, disabled = false }: BandNoiseOfferProps) {
  const latestDepth = () => depthStore.latest(path);
  const depth = useSyncExternalStore(depthStore.subscribe, latestDepth, latestDepth);
  const offer = noiseFactorOffer(depth, radius, unit);
  const text = noiseOfferText(offer, fCorr);
  if (text === null) return null;
  return (
    <div data-forced-noise-offer={offer.kind} className="flex items-center gap-1.5 text-[9px] text-zinc-400">
      <span>{text}</span>
      {offer.kind === "usable" && fCorr !== offer.fCorr && (
        <button
          type="button"
          data-action="use-depth-fcorr"
          aria-label={`Use the Depth noise factor for ${label}`}
          disabled={disabled}
          onClick={() => forcedPhotometryStore.updateBand(path, { fCorr: offer.fCorr })}
          className={SMALL_BUTTON_CLASS}
        >
          Use
        </button>
      )}
    </div>
  );
}

function ForcedPhotometryInputs({ fields, positions, files, disabled = false }: ForcedPhotometryInputsProps) {
  const positionsId = useId();
  const bandsId = useId();
  const radiusId = useId();
  const annulusId = useId();
  const growthId = useId();
  const referenceId = useId();
  const limitId = useId();
  const gainId = useId();
  const [loadError, setLoadError] = useState<string | null>(null);
  const store = forcedPhotometryStore;

  const loadList = useCallback(async () => {
    setLoadError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: false,
        filters: [{ name: "Position lists", extensions: ["csv", "txt", "tsv", "dat"] }],
        title: "Load a position list",
      });
      const path = Array.isArray(picked) ? picked[0] : picked;
      if (!path) return;
      const { readTextFile } = await import("@tauri-apps/plugin-fs");
      forcedPhotometryStore.setFields({ positionsText: await readTextFile(path) });
    } catch (e: unknown) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const chosen = new Set(fields.bands.map((b) => b.path));
  const addable = files.filter((f) => !chosen.has(f.path));
  const full = fields.bands.length >= FORCED_MAX_BANDS;
  const errors = positions.errors;
  const hiddenErrors = errors.length - MAX_SHOWN_POSITION_ERRORS;

  return (
    <div data-forced-inputs className="space-y-2">
      <div data-forced-block="positions" className="space-y-1">
        <div className={SECTION_CLASS}>Positions</div>
        <div className="flex flex-col gap-0.5">
          <label htmlFor={positionsId} className={LABEL_CLASS}>
            {POSITIONS_LABEL}
          </label>
          <textarea
            id={positionsId}
            data-field="positions"
            rows={TEXTAREA_ROWS}
            spellCheck={false}
            placeholder={POSITIONS_PLACEHOLDER}
            value={fields.positionsText}
            disabled={disabled}
            onChange={(e) => store.setFields({ positionsText: e.target.value })}
            className={TEXTAREA_CLASS}
          />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            data-action="load-positions"
            disabled={disabled}
            onClick={() => void loadList()}
            className={SMALL_BUTTON_CLASS}
          >
            <FileUp size={10} />
            Load CSV
          </button>
          <span data-forced-position-count className="text-[9px] text-zinc-500 font-mono">
            {positionsCount(positions)}
          </span>
        </div>
        {loadError !== null && (
          <div role="alert" className="text-[9px] text-red-400/90">
            {`Could not load the list: ${loadError}`}
          </div>
        )}
        {errors.length > 0 && (
          <div data-forced-position-errors className={ERRORS_CLASS}>
            {errors.slice(0, MAX_SHOWN_POSITION_ERRORS).map((line, i) => (
              <div key={i}>{line}</div>
            ))}
            {hiddenErrors > 0 && <div>{`and ${hiddenErrors} more`}</div>}
          </div>
        )}
        {fields.positionsText.trim() === "" && <div className={HINT_CLASS}>{POSITIONS_HINT}</div>}
      </div>

      <div data-forced-block="bands" className="space-y-1">
        <div className={SECTION_CLASS}>{`Bands (${fields.bands.length} of ${FORCED_MAX_BANDS})`}</div>
        {fields.bands.map((band, i) => {
          const rowId = `${bandsId}-${i}`;
          const zeroPoint = band.zeroPoint ?? null;
          return (
            <div key={band.path} data-forced-band={band.path} className="border border-zinc-800/60 rounded px-1.5 py-1 space-y-1">
              <div className="flex items-center justify-between gap-1">
                <span className="text-[10px] text-zinc-300 font-mono truncate" title={band.path}>
                  {band.fileName}
                </span>
                <button
                  type="button"
                  data-action="remove-band"
                  aria-label={`Remove ${band.fileName}`}
                  disabled={disabled}
                  onClick={() => store.removeBand(band.path)}
                  className={SMALL_BUTTON_CLASS}
                >
                  <X size={10} />
                </button>
              </div>
              <div className="grid grid-cols-3 gap-1">
                <div className="flex flex-col gap-0.5">
                  <label htmlFor={`${rowId}-label`} className={LABEL_CLASS}>
                    Label
                  </label>
                  <input
                    id={`${rowId}-label`}
                    type="text"
                    data-field="label"
                    placeholder="from the filter"
                    value={band.label}
                    disabled={disabled}
                    onChange={(e) => store.updateBand(band.path, { label: e.target.value })}
                    className={INPUT_CLASS}
                  />
                </div>
                <div className="flex flex-col gap-0.5">
                  <label htmlFor={`${rowId}-dataset`} className={LABEL_CLASS}>
                    Dataset
                  </label>
                  <input
                    id={`${rowId}-dataset`}
                    type="text"
                    data-field="dataset"
                    placeholder="from the header"
                    value={band.dataset}
                    disabled={disabled}
                    onChange={(e) => store.updateBand(band.path, { dataset: e.target.value })}
                    className={INPUT_CLASS}
                  />
                </div>
                <div className="flex flex-col gap-0.5">
                  <label htmlFor={`${rowId}-fcorr`} className={LABEL_CLASS}>
                    Noise factor
                  </label>
                  <DecimalNumberField
                    id={`${rowId}-fcorr`}
                    data-field="fCorr"
                    optional
                    min={0.01}
                    max={100}
                    placeholder="none"
                    value={band.fCorr}
                    disabled={disabled}
                    onValueChange={(fCorr) => store.updateBand(band.path, { fCorr })}
                    className={INPUT_CLASS}
                  />
                </div>
              </div>
              <div data-forced-zero-point className="flex items-center gap-1">
                <label htmlFor={`${rowId}-zp`} className={LABEL_CLASS}>
                  Zero point
                </label>
                <DecimalNumberField
                  id={`${rowId}-zp`}
                  data-field="zp"
                  optional
                  min={-100}
                  max={100}
                  placeholder="from the header"
                  value={zeroPoint?.zp ?? null}
                  disabled={disabled}
                  onValueChange={(zp) =>
                    store.updateBand(band.path, { zeroPoint: zp === null ? null : { ...(zeroPoint ?? TYPED_ZERO_POINT), zp } })
                  }
                  className={INPUT_CLASS}
                />
                <select
                  data-field="zpSystem"
                  aria-label="Zero point system"
                  value={zeroPoint?.system ?? TYPED_ZERO_POINT.system}
                  disabled={disabled || zeroPoint === null}
                  onChange={(e) =>
                    zeroPoint !== null &&
                    store.updateBand(band.path, {
                      zeroPoint: { ...zeroPoint, system: e.target.value as ForcedZeroPointChoice["system"] },
                    })
                  }
                  className={SELECT_CLASS}
                >
                  {ZERO_POINT_SYSTEMS.map((system) => (
                    <option key={system.value} value={system.value}>
                      {system.label}
                    </option>
                  ))}
                </select>
                <input
                  id={`${rowId}-zp-per-second`}
                  type="checkbox"
                  data-field="zpPerSecond"
                  title={PER_SECOND_TITLE}
                  checked={zeroPoint?.perSecond ?? TYPED_ZERO_POINT.perSecond}
                  disabled={disabled || zeroPoint === null}
                  onChange={(e) =>
                    zeroPoint !== null && store.updateBand(band.path, { zeroPoint: { ...zeroPoint, perSecond: e.target.checked } })
                  }
                />
                <label htmlFor={`${rowId}-zp-per-second`} title={PER_SECOND_TITLE} className={LABEL_CLASS}>
                  per second
                </label>
              </div>
              {zeroPoint !== null && <div className={HINT_CLASS}>{ZERO_POINT_CAPTIONS[zeroPoint.system]}</div>}
              <BandNoiseOffer
                path={band.path}
                label={band.label || band.fileName}
                fCorr={band.fCorr}
                radius={fields.radius}
                unit={fields.radiusUnit}
                disabled={disabled}
              />
            </div>
          );
        })}
        {addable.length > 0 && (
          <div className="flex flex-wrap items-center gap-1">
            {addable.map((file) => (
              <button
                key={file.path}
                type="button"
                data-action="add-band"
                disabled={disabled || full}
                onClick={() => store.addBand(file)}
                className={SMALL_BUTTON_CLASS}
              >
                {`+ ${bandDisplayName(file)}`}
              </button>
            ))}
            {addable.length >= 2 && (
              <button
                type="button"
                data-action="add-all-bands"
                disabled={disabled || full}
                onClick={() => addable.forEach((file) => store.addBand(file))}
                className={SMALL_BUTTON_CLASS}
              >
                Add all
              </button>
            )}
          </div>
        )}
        {fields.bands.length === 0 && files.length === 0 && <div className={HINT_CLASS}>{BANDS_EMPTY}</div>}
        {fields.bands.length > 0 && <div className={HINT_CLASS}>{BANDS_HINT}</div>}
      </div>

      <div data-forced-block="aperture" className="space-y-1">
        <div className={SECTION_CLASS}>Aperture</div>
        <div className="flex items-center gap-1">
          <label htmlFor={radiusId} className={LABEL_CLASS}>
            Radius
          </label>
          <DecimalNumberField
            id={radiusId}
            data-field="radius"
            min={0.001}
            max={3600}
            value={fields.radius}
            disabled={disabled}
            onValueChange={(radius) => store.setFields({ radius })}
            className={INPUT_CLASS}
          />
          <select
            data-field="radiusUnit"
            aria-label="Radius unit"
            value={fields.radiusUnit}
            disabled={disabled}
            onChange={(e) => store.setFields({ radiusUnit: e.target.value as ForcedRadiusUnit })}
            className={SELECT_CLASS}
          >
            {RADIUS_UNITS.map((unit) => (
              <option key={unit} value={unit}>
                {unit}
              </option>
            ))}
          </select>
        </div>
        <div className="flex items-center gap-1">
          <label htmlFor={annulusId} className={LABEL_CLASS}>
            Sky annulus
          </label>
          <DecimalNumberField
            id={annulusId}
            data-field="annulusInner"
            optional
            min={0.001}
            max={3600}
            placeholder="inner"
            value={fields.annulusInner}
            disabled={disabled}
            onValueChange={(annulusInner) => store.setFields({ annulusInner })}
            className={INPUT_CLASS}
          />
          <DecimalNumberField
            data-field="annulusOuter"
            aria-label="Sky annulus outer"
            optional
            min={0.001}
            max={3600}
            placeholder="outer"
            value={fields.annulusOuter}
            disabled={disabled}
            onValueChange={(annulusOuter) => store.setFields({ annulusOuter })}
            className={INPUT_CLASS}
          />
        </div>
        <div className={LABEL_CLASS}>{ANNULUS_CAPTION}</div>
        <div className="flex items-center gap-1.5">
          <input
            id={growthId}
            type="checkbox"
            data-field="growthCorrection"
            checked={fields.growthCorrection}
            disabled={disabled}
            onChange={(e) => store.setFields({ growthCorrection: e.target.checked })}
          />
          <label htmlFor={growthId} className="text-xs text-zinc-400">
            {GROWTH_LABEL}
          </label>
        </div>
        {fields.growthCorrection && (
          <div className="flex items-center gap-1">
            <label htmlFor={referenceId} className={LABEL_CLASS}>
              Reference radius (arcsec)
            </label>
            <DecimalNumberField
              id={referenceId}
              data-field="referenceRadiusArcsec"
              optional
              min={0.001}
              max={600}
              placeholder="3.5"
              value={fields.referenceRadiusArcsec}
              disabled={disabled}
              onValueChange={(referenceRadiusArcsec) => store.setFields({ referenceRadiusArcsec })}
              className={INPUT_CLASS}
            />
          </div>
        )}
      </div>

      <div data-forced-block="options" className="space-y-1">
        <div className={SECTION_CLASS}>Options</div>
        <div className="flex items-center gap-1">
          <label htmlFor={limitId} className={LABEL_CLASS}>
            Limit (sigma)
          </label>
          <DecimalNumberField
            id={limitId}
            data-field="limitSigma"
            optional
            min={0.1}
            max={100}
            placeholder="3"
            value={fields.limitSigma}
            disabled={disabled}
            onValueChange={(limitSigma) => store.setFields({ limitSigma })}
            className={INPUT_CLASS}
          />
        </div>
        <Toggle
          label="Exclude flagged pixels"
          checked={fields.excludeDq}
          disabled={disabled}
          onChange={(excludeDq) => store.setFields({ excludeDq })}
        />
        <div className="flex items-center gap-1">
          <label htmlFor={gainId} className={LABEL_CLASS}>
            Gain (e-/ADU)
          </label>
          <DecimalNumberField
            id={gainId}
            data-field="gain"
            optional
            min={0.01}
            placeholder="none"
            title={GAIN_TITLE}
            value={fields.gain}
            disabled={disabled}
            onValueChange={(gain) => store.setFields({ gain })}
            className={INPUT_CLASS}
          />
        </div>
      </div>
    </div>
  );
}

export default memo(ForcedPhotometryInputs);
