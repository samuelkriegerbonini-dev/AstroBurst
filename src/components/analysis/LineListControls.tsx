import { memo, useEffect, useId, useState } from "react";
import type { VelocityConvention } from "../../shared/types/spectral";
import { conventionLabel } from "../../utils/spectralAxis";
import {
  BAND_SPECIES,
  DEFAULT_BAND_SPECIES,
  LINE_FAMILIES,
  MAX_REDSHIFT,
  REST_BANDS,
  REST_LINES,
  familyLabel,
  formatRedshift,
  formatSystemicKms,
  parseRedshiftInput,
  parseSystemicInput,
  resyncRedshiftText,
  resyncSystemicText,
  toggleBandSpecies,
  velocityKmsFromRedshift,
  type LineFamily,
  type LineListAvailability,
} from "../../utils/lineList";
import { DecimalField } from "../ui";

export interface LineListControlsProps {
  visible: boolean;
  onVisibleChange: (visible: boolean) => void;
  redshift: number;
  onRedshiftChange: (z: number) => void;
  convention: VelocityConvention;
  families: readonly LineFamily[];
  onFamiliesChange: (families: LineFamily[]) => void;
  frameLabel: string;
  availability: LineListAvailability;
  clickHint: string;
  pickedLabel: string | null;
  drawnCount: number;
  bandSpecies?: readonly string[];
  onBandSpeciesChange?: (species: string[]) => void;
  bandDrawnCount?: number;
}

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs font-mono text-zinc-200 focus:border-violet-500/50 w-full disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const ACCENT_COLOR = "#22d3ee";
const REDSHIFT_RANGE_ERROR = `Use a redshift above -1 and at most ${MAX_REDSHIFT}`;
const SYSTEMIC_RANGE_ERROR = `Use a velocity whose redshift is above -1 and at most ${MAX_REDSHIFT}`;
const BAND_NOTE = "band origins and atomic lines from NIST; the shaded widths of molecular bands are display guides";

function bandSourcesOf(species: string): string {
  return REST_BANDS.filter((band) => band.species === species)
    .map((band) => band.source)
    .join(" | ");
}

function LineListControls({
  visible,
  onVisibleChange,
  redshift,
  onRedshiftChange,
  convention,
  families,
  onFamiliesChange,
  frameLabel,
  availability,
  clickHint,
  pickedLabel,
  drawnCount,
  bandSpecies = [],
  onBandSpeciesChange,
  bandDrawnCount = 0,
}: LineListControlsProps) {
  const redshiftId = useId();
  const systemicId = useId();
  const [zText, setZText] = useState(() => formatRedshift(redshift));
  const [vText, setVText] = useState(() => formatSystemicKms(velocityKmsFromRedshift(redshift, convention)));

  useEffect(() => {
    setZText((current) => resyncRedshiftText(current, redshift));
    setVText((current) => resyncSystemicText(current, redshift, convention));
  }, [redshift, convention]);

  const zValid = parseRedshiftInput(zText) !== null;
  const vValid = parseSystemicInput(vText, convention) !== null;

  const handleRedshiftText = (text: string) => {
    setZText(text);
    const z = parseRedshiftInput(text);
    if (z === null) return;
    onRedshiftChange(z);
    setVText(formatSystemicKms(velocityKmsFromRedshift(z, convention)));
  };

  const handleSystemicText = (text: string) => {
    setVText(text);
    const z = parseSystemicInput(text, convention);
    if (z === null) return;
    onRedshiftChange(z);
    setZText(formatRedshift(z));
  };

  const toggleFamily = (family: LineFamily) => {
    onFamiliesChange(LINE_FAMILIES.filter((f) => (f === family ? !families.includes(f) : families.includes(f))));
  };

  return (
    <div className="flex flex-col gap-1.5 px-3 pb-2" style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}>
      <div className="flex items-center gap-2">
        <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
          <input type="checkbox" checked={visible} onChange={(e) => onVisibleChange(e.target.checked)} />
          Rest lines
        </label>
        {visible && availability.ok && (
          <span className="ml-auto text-[9px] font-mono text-zinc-600">
            {drawnCount} of {REST_LINES.length} in range
          </span>
        )}
      </div>

      <div className="flex gap-2">
        <div className="flex-1 flex flex-col gap-0.5">
          <label htmlFor={redshiftId} className={LABEL_CLASS}>
            Redshift z
          </label>
          <DecimalField
            id={redshiftId}
            value={zText}
            error={zValid ? null : REDSHIFT_RANGE_ERROR}
            onChange={handleRedshiftText}
            className={INPUT_CLASS}
          />
        </div>
        <div className="flex-1 flex flex-col gap-0.5">
          <label htmlFor={systemicId} className={LABEL_CLASS}>
            {`v_sys, ${conventionLabel(convention)} (km/s)`}
          </label>
          <DecimalField
            id={systemicId}
            value={vText}
            error={vValid ? null : SYSTEMIC_RANGE_ERROR}
            onChange={handleSystemicText}
            className={INPUT_CLASS}
          />
        </div>
      </div>

      <div className="flex items-center gap-1.5 text-[10px] font-mono">
        {LINE_FAMILIES.map((family) => {
          const enabled = families.includes(family);
          return (
            <button
              key={family}
              type="button"
              aria-pressed={enabled}
              onClick={() => toggleFamily(family)}
              className="px-2 py-0.5 rounded border transition-colors"
              style={{
                borderColor: enabled ? ACCENT_COLOR : "var(--ab-border)",
                color: enabled ? ACCENT_COLOR : "var(--ab-text-3)",
              }}
            >
              {familyLabel(family)}
            </button>
          );
        })}
      </div>

      {onBandSpeciesChange && (
        <>
          <div className="flex items-center gap-2">
            <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
              <input
                type="checkbox"
                checked={bandSpecies.length > 0}
                onChange={(e) => onBandSpeciesChange(e.target.checked ? [...DEFAULT_BAND_SPECIES] : [])}
              />
              Bands
            </label>
            {bandSpecies.length > 0 && availability.ok && (
              <span className="ml-auto text-[9px] font-mono text-zinc-600">
                {bandDrawnCount} of {REST_BANDS.length} in range
              </span>
            )}
          </div>
          {bandSpecies.length > 0 && (
            <>
              <div role="group" aria-label="Band species" className="flex flex-wrap items-center gap-1.5 text-[10px] font-mono">
                {BAND_SPECIES.map((species) => {
                  const enabled = bandSpecies.includes(species);
                  return (
                    <button
                      key={species}
                      type="button"
                      aria-pressed={enabled}
                      title={bandSourcesOf(species)}
                      onClick={() => onBandSpeciesChange(toggleBandSpecies(bandSpecies, species))}
                      className="px-2 py-0.5 rounded border transition-colors"
                      style={{
                        borderColor: enabled ? ACCENT_COLOR : "var(--ab-border)",
                        color: enabled ? ACCENT_COLOR : "var(--ab-text-3)",
                      }}
                    >
                      {species}
                    </button>
                  );
                })}
              </div>
              <p className="text-[9px] text-zinc-500">{BAND_NOTE}</p>
            </>
          )}
        </>
      )}

      <p className="text-[9px] font-mono text-zinc-500">{frameLabel}</p>
      {availability.ok
        ? clickHint !== "" && <p className="text-[9px] text-zinc-600">{clickHint}</p>
        : <p className="text-[9px] text-amber-300/80">{availability.reason}</p>}
      {pickedLabel !== null && (
        <p className="text-[9px] font-mono" style={{ color: ACCENT_COLOR }}>
          {pickedLabel}
        </p>
      )}
    </div>
  );
}

export default memo(LineListControls);
