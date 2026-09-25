import { memo, useEffect, useId, useState } from "react";
import type { VelocityConvention } from "../../shared/types/spectral";
import { conventionLabel } from "../../utils/spectralAxis";
import {
  LINE_FAMILIES,
  REST_LINES,
  familyLabel,
  formatRedshift,
  formatSystemicKms,
  parseRedshiftInput,
  parseSystemicInput,
  resyncRedshiftText,
  resyncSystemicText,
  velocityKmsFromRedshift,
  type LineFamily,
  type LineListAvailability,
} from "../../utils/lineList";

interface LineListControlsProps {
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
}

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs font-mono text-zinc-200 focus:border-violet-500/50 w-full disabled:opacity-40";
const INVALID_INPUT_CLASS = "border-red-500/60";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const ACCENT_COLOR = "#22d3ee";

function inputClass(valid: boolean): string {
  return valid ? INPUT_CLASS : `${INPUT_CLASS} ${INVALID_INPUT_CLASS}`;
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
          <input
            id={redshiftId}
            type="number"
            step="any"
            value={zText}
            aria-invalid={zValid ? undefined : true}
            onChange={(e) => handleRedshiftText(e.target.value)}
            className={inputClass(zValid)}
          />
        </div>
        <div className="flex-1 flex flex-col gap-0.5">
          <label htmlFor={systemicId} className={LABEL_CLASS}>
            {`v_sys, ${conventionLabel(convention)} (km/s)`}
          </label>
          <input
            id={systemicId}
            type="number"
            step="any"
            value={vText}
            aria-invalid={vValid ? undefined : true}
            onChange={(e) => handleSystemicText(e.target.value)}
            className={inputClass(vValid)}
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
                color: enabled ? ACCENT_COLOR : "#71717a",
              }}
            >
              {familyLabel(family)}
            </button>
          );
        })}
      </div>

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
