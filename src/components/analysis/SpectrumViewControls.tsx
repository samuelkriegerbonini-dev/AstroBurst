import { memo, useCallback, useState } from "react";
import type { FluxDisplayChoice, Spectrum1d, WavelengthDisplayUnit, WavelengthFrameChoice } from "../../shared/types/spectrum1d";
import {
  effectiveFrameChoice,
  useSpectrumLineOverlay,
  useSpectrumView,
  type SpectrumViewState,
  type YLimitsMode,
} from "../../hooks/useSpectrumViewStore";
import { decimalFieldState } from "../../utils/decimalText";
import DecimalField from "../ui/DecimalField";
import { AIR_FRAME_FORMULA_NOTE, documentFrameLine, documentFrameOrigin } from "../../utils/spectrumDocument";
import type { SpectrumPlotModel } from "../../utils/spectrumPlotModel";
import { WAVELENGTH_DISPLAY_UNITS, WAVELENGTH_UNIT_LABEL, fluxChoiceLabel, fluxDisplayChoices } from "../../utils/spectrumUnits";

interface SpectrumViewControlsProps {
  viewKey: string;
  spectrum: Spectrum1d | null;
  model: SpectrumPlotModel | null;
  disabled?: boolean;
}

const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const BUTTON_CLASS = "px-2 py-0.5 rounded border transition-colors disabled:opacity-40";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 focus:border-violet-500/50";
const WAVELENGTH_FRAME_CHOICES: readonly WavelengthFrameChoice[] = ["air", "vacuum"];
const Y_LIMITS_CHOICES: readonly YLimitsMode[] = ["full", "robust"];
const Y_LIMITS_TITLES: Record<YLimitsMode, string> = {
  full: "y limits from the smallest to the largest visible sample",
  robust: "y limits from the 1st to the 99th percentile of the visible samples",
};
const RESET_ZOOM_TITLE = "Show the whole axis (the wheel zooms, Shift and drag pans)";
const BIN_FIELD_CLASS = `${SELECT_CLASS} w-10`;
const UNBINNED_CHANNELS_NOTE = "the line range, the continuum windows and Measure line use the unbinned channels";

function pressedStyle(pressed: boolean) {
  return {
    borderColor: pressed ? "var(--ab-violet)" : "var(--ab-border)",
    color: pressed ? "var(--ab-violet)" : "var(--ab-text-3)",
  };
}

function samples(count: number): string {
  return count === 1 ? "1 sample" : `${count} samples`;
}

function plotViewNotes(model: SpectrumPlotModel, view: SpectrumViewState, overlayPublished: boolean): string[] {
  const notes: string[] = [];
  const { fluxState } = model;
  if (model.offScale > 0) notes.push(`${samples(model.offScale)} off scale`);
  if (view.yLimits === "full" && model.farOutliers > 0) notes.push(`y range set by ${samples(model.farOutliers)} far from the rest: try robust limits`);
  if (view.logY && fluxState.invertY) notes.push("log y is not used for AB magnitudes");
  else if (view.logY && !model.logY) notes.push("log y needs a sample above 0: shown linear");
  if (model.logHidden > 0) {
    notes.push(`${samples(model.logHidden)} at or below 0 ${model.logHidden === 1 ? "is" : "are"} not drawn on a log axis`);
  }
  if (model.logY && overlayPublished) notes.push("the measured line overlay is not drawn on a log axis");
  if (fluxState.note !== null) notes.push(`flux shown in the file unit: ${fluxState.note}`);
  if (fluxState.scale === null && overlayPublished) notes.push("the measured line overlay is drawn only in the file unit or a multiple of it");
  if (model.bins !== null) notes.push(model.bins.note, UNBINNED_CHANNELS_NOTE);
  return notes;
}

function hasVacuumAxis(spectrum: Spectrum1d): boolean {
  const kind = spectrum.axis.kind;
  return kind === "wave" || kind === "awav" || kind === "freq";
}

function SpectrumViewControls({ viewKey, spectrum, model, disabled = false }: SpectrumViewControlsProps) {
  const [view, patchView] = useSpectrumView(viewKey);
  const lineOverlay = useSpectrumLineOverlay(viewKey);
  const frameChoice = effectiveFrameChoice(view, spectrum);
  const chooseFrame = useCallback(
    (choice: WavelengthFrameChoice) => {
      if (choice === frameChoice) return;
      patchView({ frameChoice: choice });
    },
    [frameChoice, patchView],
  );
  const [binDraft, setBinDraft] = useState<string | null>(null);

  const plotModel = model !== null && model.n > 0 ? model : null;
  if (!spectrum && !plotModel) return null;
  const wavelengthUnit = plotModel?.wavelengthUnit ?? null;
  const fluxChoices = spectrum ? fluxDisplayChoices(spectrum.flux_unit, hasVacuumAxis(spectrum)) : [];
  const fluxChoiceShown = fluxChoices.length > 1;
  const frameOrigin = spectrum ? documentFrameOrigin(spectrum) : null;
  const binChannels = spectrum && plotModel && plotModel.n >= 2 ? plotModel.n : null;
  const commitBin = () => {
    if (binDraft === null || binChannels === null) return;
    const { value, error } = decimalFieldState(binDraft, { integer: true, min: 1, max: binChannels });
    if (error !== null || value === null) return;
    setBinDraft(null);
    if (value !== view.binFactor) patchView({ binFactor: value });
  };
  const handleBinKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") commitBin();
    else if (e.key === "Escape") setBinDraft(null);
  };

  return (
    <>
      {spectrum && (
        <div data-document-frame className="px-3 pb-2 flex flex-col gap-1 text-[10px] font-mono">
          <div className="flex items-center gap-2 flex-wrap">
            <span className={LABEL_CLASS}>Frame</span>
            {frameChoice !== null &&
              WAVELENGTH_FRAME_CHOICES.map((choice) => (
                <button
                  key={choice}
                  data-frame-choice={choice}
                  aria-pressed={frameChoice === choice}
                  disabled={disabled}
                  onClick={() => chooseFrame(choice)}
                  className={BUTTON_CLASS}
                  style={pressedStyle(frameChoice === choice)}
                >
                  {choice}
                </button>
              ))}
            <span className="text-zinc-500">{documentFrameLine(spectrum, frameChoice)}</span>
          </div>
          {frameOrigin !== null && (
            <span data-frame-origin className="text-zinc-600">
              frame from: {frameOrigin}
            </span>
          )}
          {frameChoice === "air" && <span className="text-zinc-600">{AIR_FRAME_FORMULA_NOTE}</span>}
        </div>
      )}
      {plotModel && (
        <div className="px-3 pb-2 flex flex-col gap-1 text-[10px] font-mono">
          <div className="flex items-center gap-2 flex-wrap">
            <button
              data-view-log
              aria-pressed={view.logY}
              onClick={() => patchView({ logY: !view.logY })}
              className={BUTTON_CLASS}
              style={pressedStyle(view.logY)}
            >
              log y
            </button>
            <span className={LABEL_CLASS}>Limits</span>
            {Y_LIMITS_CHOICES.map((yLimits) => (
              <button
                key={yLimits}
                data-view-limits={yLimits}
                aria-pressed={view.yLimits === yLimits}
                title={Y_LIMITS_TITLES[yLimits]}
                onClick={() => patchView({ yLimits })}
                className={BUTTON_CLASS}
                style={pressedStyle(view.yLimits === yLimits)}
              >
                {yLimits}
              </button>
            ))}
            {binChannels !== null && (
              <>
                <span className={LABEL_CLASS}>Bin</span>
                <DecimalField
                  integer
                  min={1}
                  max={binChannels}
                  value={binDraft ?? String(view.binFactor)}
                  onChange={setBinDraft}
                  onBlur={commitBin}
                  onKeyDown={handleBinKey}
                  aria-label="Bin factor"
                  data-view-bin
                  className={BIN_FIELD_CLASS}
                />
              </>
            )}
            {view.xDomain !== null && (
              <button
                data-view-reset-zoom
                title={RESET_ZOOM_TITLE}
                onClick={() => patchView({ xDomain: null })}
                className={BUTTON_CLASS}
                style={pressedStyle(false)}
              >
                reset zoom
              </button>
            )}
          </div>
          {(wavelengthUnit !== null || fluxChoiceShown) && (
            <div data-view-units className="flex items-center gap-2 flex-wrap">
              <span className={LABEL_CLASS}>Units</span>
              {wavelengthUnit !== null && (
                <select
                  aria-label="Wavelength unit"
                  data-view-wavelength-unit
                  value={wavelengthUnit}
                  onChange={(e) => patchView({ wavelengthUnit: e.target.value as WavelengthDisplayUnit })}
                  className={SELECT_CLASS}
                >
                  {WAVELENGTH_DISPLAY_UNITS.map((unit) => (
                    <option key={unit} value={unit}>
                      {WAVELENGTH_UNIT_LABEL[unit]}
                    </option>
                  ))}
                </select>
              )}
              {spectrum && fluxChoiceShown && (
                <select
                  aria-label="Flux unit"
                  data-view-flux-unit
                  value={fluxChoices.includes(view.fluxUnit) ? view.fluxUnit : "native"}
                  onChange={(e) => patchView({ fluxUnit: e.target.value as FluxDisplayChoice })}
                  className={SELECT_CLASS}
                >
                  {fluxChoices.map((choice) => (
                    <option key={choice} value={choice}>
                      {fluxChoiceLabel(choice, spectrum.flux_unit)}
                    </option>
                  ))}
                </select>
              )}
            </div>
          )}
          {plotViewNotes(plotModel, view, lineOverlay !== null).map((note) => (
            <span key={note} className="text-zinc-500">
              {note}
            </span>
          ))}
        </div>
      )}
    </>
  );
}

export default memo(SpectrumViewControls);
