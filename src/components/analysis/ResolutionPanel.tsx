import { memo, useState, type ReactNode } from "react";
import { DecimalNumberField, ErrorAlert, SMALL_BUTTON_CLASS, WarningList } from "../ui";
import ResolutionCard from "./ResolutionCard";
import {
  imagePsfStore,
  resolutionSettingsStore,
  useHeaderSeeing,
  useImagePsf,
  useResolutionSettings,
} from "../../hooks/useResolutionSettings";
import type { ResolutionPsfSource, ResolutionSettings } from "../../shared/types/resolution";
import { imagePsfText, psfChoice, significant } from "../../utils/resolutionText";

export interface ResolutionPanelProps {
  filePath: string | null;
  measureKey?: string | null;
}

type RequiredNumberSetting = "h0" | "omegaM" | "eps" | "qResolved" | "minPixelsPerFwhm" | "scaleOfInterestKpc";
type OptionalNumberSetting = "redshift" | "psfFwhmArcsec";

const INTRO =
  'Whether a source is resolved depends on the PSF, on its signal-to-noise and on the sampling. These settings feed every "Resolved or not" card.';
const NO_HEADER_CARD = "no SEEING, PSF_FWHM or PSFFWHM card";
const PSF_SOURCES: readonly ResolutionPsfSource[] = ["image", "typed", "header"];
const PSF_SOURCE_LABEL: Record<ResolutionPsfSource, string> = {
  image: "Stars of this image",
  typed: "Typed value",
  header: "Header card",
};

const FIELDSET_CLASS = "space-y-1.5 border-t border-zinc-800/50 pt-2";
const LEGEND_CLASS = "text-[10px] font-semibold text-zinc-300 uppercase tracking-wider";
const LABEL_CLASS = "text-[10px] text-zinc-400";
const VALUE_CLASS = "text-[10px] font-mono text-zinc-200 break-words";
const NOTE_CLASS = "text-[10px] text-zinc-500";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 font-mono focus:border-sky-500/50 w-20 disabled:opacity-40";
const BADGE_CLASS = "text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30";

function writeRequired(key: RequiredNumberSetting) {
  return (value: number | null) => {
    if (value === null) return;
    const patch: Partial<Pick<ResolutionSettings, RequiredNumberSetting>> = {};
    patch[key] = value;
    resolutionSettingsStore.update(patch);
  };
}

function writeOptional(key: OptionalNumberSetting) {
  return (value: number | null) => {
    const patch: Partial<Pick<ResolutionSettings, OptionalNumberSetting>> = {};
    patch[key] = value;
    resolutionSettingsStore.update(patch);
  };
}

function writePsfSource(text: string) {
  const source = PSF_SOURCES.find((candidate) => candidate === text);
  if (source !== undefined) resolutionSettingsStore.update({ psfSource: source });
}

function fieldRow(label: string, control: ReactNode): React.JSX.Element {
  return (
    <label className="flex items-center justify-between gap-2">
      <span className={LABEL_CLASS}>{label}</span>
      {control}
    </label>
  );
}

function textRow(label: string, text: string): React.JSX.Element {
  return (
    <div className="flex items-baseline gap-2">
      <span className={LABEL_CLASS}>{label}</span>
      <span className={VALUE_CLASS}>{text}</span>
    </div>
  );
}

export function ResolutionPanelInner({ filePath }: ResolutionPanelProps): React.JSX.Element {
  const settings = useResolutionSettings();
  const image = useImagePsf(filePath);
  const headerCards = useHeaderSeeing(filePath);
  const choice = psfChoice(settings, image.psf, headerCards);
  const [sourceFwhm, setSourceFwhm] = useState<number | null>(null);
  const [sourceR50, setSourceR50] = useState<number | null>(null);
  const [sourceSnr, setSourceSnr] = useState<number | null>(null);

  const psf = image.psf;
  const shownCards = psf?.header_values ?? headerCards;
  const headerText =
    shownCards.length === 0 ? NO_HEADER_CARD : shownCards.map((card) => `${card.key} ${significant(card.value_arcsec)} arcsec`).join(", ");
  const psfUsed = choice.missing === null ? `${significant(choice.fwhmArcsec)} arcsec, ${choice.reference}` : choice.missing;
  const onMeasure = () => {
    if (filePath !== null) void imagePsfStore.measure(filePath);
  };

  return (
    <div className="ab-panel overflow-hidden" data-resolution-panel>
      <div className="flex items-center px-3 py-2 border-b border-zinc-800/50">
        <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Resolution</span>
      </div>
      <div className="px-3 py-2 space-y-2">
        <p className={NOTE_CLASS}>{INTRO}</p>
        <fieldset className={FIELDSET_CLASS}>
          <legend className={LEGEND_CLASS}>PSF</legend>
          {fieldRow(
            "PSF source",
            <select
              data-field="psfSource"
              value={settings.psfSource}
              onChange={(event) => writePsfSource(event.target.value)}
              className="bg-zinc-900 border border-zinc-700/50 rounded px-1 py-0.5 text-[10px] text-zinc-200"
            >
              {PSF_SOURCES.map((source) => (
                <option key={source} value={source}>
                  {PSF_SOURCE_LABEL[source]}
                </option>
              ))}
            </select>,
          )}
          {fieldRow(
            "PSF FWHM (arcsec)",
            <DecimalNumberField
              data-field="psfFwhmArcsec"
              value={settings.psfFwhmArcsec}
              onValueChange={writeOptional("psfFwhmArcsec")}
              min={0.001}
              max={60}
              optional
              disabled={settings.psfSource !== "typed"}
              className={INPUT_CLASS}
            />,
          )}
          <button
            type="button"
            data-action="measure-psf"
            className={SMALL_BUTTON_CLASS}
            disabled={filePath === null || image.loading}
            onClick={onMeasure}
          >
            {image.loading ? "Measuring the PSF…" : "Measure PSF from stars"}
          </button>
          {psf !== null && (
            <>
              {textRow("Image PSF", imagePsfText(psf))}
              <div className="flex items-center gap-1.5">
                <span className={NOTE_CLASS}>{psf.model}</span>
                {psf.undersampled && <span className={BADGE_CLASS}>undersampled</span>}
              </div>
              <WarningList warnings={psf.warnings} />
              {psf.notes.length > 0 && (
                <details className="text-[9px] text-zinc-500">
                  <summary className="cursor-pointer">{`notes (${psf.notes.length})`}</summary>
                  <ul className="list-disc pl-4 space-y-0.5">
                    {psf.notes.map((note, index) => (
                      <li key={index}>{note}</li>
                    ))}
                  </ul>
                </details>
              )}
            </>
          )}
          {filePath !== null && textRow("Header", headerText)}
          <ErrorAlert message={image.error} />
          {textRow("PSF used", psfUsed)}
        </fieldset>
        <fieldset className={FIELDSET_CLASS}>
          <legend className={LEGEND_CLASS}>Physical scale</legend>
          {fieldRow(
            "Redshift (optional)",
            <DecimalNumberField
              data-field="redshift"
              value={settings.redshift}
              onValueChange={writeOptional("redshift")}
              min={0.0001}
              max={20}
              optional
              className={INPUT_CLASS}
            />,
          )}
          {fieldRow(
            "H0 (km/s/Mpc)",
            <DecimalNumberField data-field="h0" value={settings.h0} onValueChange={writeRequired("h0")} min={20} max={200} className={INPUT_CLASS} />,
          )}
          {fieldRow(
            "Omega_m",
            <DecimalNumberField
              data-field="omegaM"
              value={settings.omegaM}
              onValueChange={writeRequired("omegaM")}
              min={0.01}
              max={0.99}
              className={INPUT_CLASS}
            />,
          )}
          {settings.redshift === null && <p className={NOTE_CLASS}>Type a redshift to see sizes in kpc.</p>}
        </fieldset>
        <fieldset className={FIELDSET_CLASS}>
          <legend className={LEGEND_CLASS}>Thresholds</legend>
          {fieldRow(
            "PSF FWHM uncertainty (fraction)",
            <DecimalNumberField data-field="eps" value={settings.eps} onValueChange={writeRequired("eps")} min={0} max={0.5} className={INPUT_CLASS} />,
          )}
          {fieldRow(
            "Resolved from (resolution elements)",
            <DecimalNumberField
              data-field="qResolved"
              value={settings.qResolved}
              onValueChange={writeRequired("qResolved")}
              min={0.1}
              max={100}
              className={INPUT_CLASS}
            />,
          )}
          {fieldRow(
            "Sampling limit (px per FWHM)",
            <DecimalNumberField
              data-field="minPixelsPerFwhm"
              value={settings.minPixelsPerFwhm}
              onValueChange={writeRequired("minPixelsPerFwhm")}
              min={0.5}
              max={10}
              className={INPUT_CLASS}
            />,
          )}
          {fieldRow(
            "Scale of interest (kpc)",
            <DecimalNumberField
              data-field="scaleOfInterestKpc"
              value={settings.scaleOfInterestKpc}
              onValueChange={writeRequired("scaleOfInterestKpc")}
              min={0.001}
              max={1000}
              className={INPUT_CLASS}
            />,
          )}
          <button type="button" data-action="reset-resolution" className={SMALL_BUTTON_CLASS} onClick={() => resolutionSettingsStore.reset()}>
            Reset to defaults
          </button>
        </fieldset>
        <fieldset className={FIELDSET_CLASS}>
          <legend className={LEGEND_CLASS}>Check a size</legend>
          {fieldRow(
            "Source FWHM (arcsec)",
            <DecimalNumberField data-field="sourceFwhm" value={sourceFwhm} onValueChange={setSourceFwhm} min={0.001} optional className={INPUT_CLASS} />,
          )}
          {fieldRow(
            "Half-light radius (arcsec)",
            <DecimalNumberField data-field="sourceR50" value={sourceR50} onValueChange={setSourceR50} min={0.001} optional className={INPUT_CLASS} />,
          )}
          {fieldRow(
            "Signal-to-noise",
            <DecimalNumberField data-field="sourceSnr" value={sourceSnr} onValueChange={setSourceSnr} optional className={INPUT_CLASS} />,
          )}
          {filePath === null ? (
            <p className={NOTE_CLASS}>Open an image to check a size.</p>
          ) : (
            <ResolutionCard filePath={filePath} fwhmArcsec={sourceFwhm} halfLightRadiusArcsec={sourceR50} snr={sourceSnr} context="manual" />
          )}
        </fieldset>
      </div>
    </div>
  );
}

export default memo(ResolutionPanelInner);
