import { useId } from "react";
import { Slider } from "../ui";
import type { RejectionMethod } from "../../shared/types/stacking";
import {
  REJECTION_LIMITS,
  REJECTION_OPTIONS,
  boundRejectionSettings,
  boundRejectionValue,
  rejectionFrameHint,
  type FixedRejection,
  type RejectionParameter,
  type RejectionSettings,
} from "../../utils/stackingRejection";

interface RejectionControlsProps {
  settings: RejectionSettings;
  onChange: (change: Partial<RejectionSettings>) => void;
  frameCount: number | null;
  accent: string;
  label?: string;
  fixed?: FixedRejection;
  disabled?: boolean;
}

interface ParameterRow {
  parameter: RejectionParameter;
  label: string;
  format: (value: number) => string;
  methods: readonly RejectionMethod[];
}

const SIGMA_METHODS: readonly RejectionMethod[] = ["sigma_clip", "winsorized_sigma_clip", "linear_fit_clip"];
const oneDecimal = (value: number) => value.toFixed(1);
const twoDecimals = (value: number) => value.toFixed(2);

const PARAMETER_ROWS: readonly ParameterRow[] = [
  { parameter: "sigmaLow", label: "Sigma Low", format: oneDecimal, methods: SIGMA_METHODS },
  { parameter: "sigmaHigh", label: "Sigma High", format: oneDecimal, methods: SIGMA_METHODS },
  { parameter: "maxIterations", label: "Max Iterations", format: String, methods: SIGMA_METHODS },
  { parameter: "winsorCutoff", label: "Winsorization Cutoff", format: (value) => `${value.toFixed(1)}σ`, methods: ["winsorized_sigma_clip"] },
  { parameter: "percentileLow", label: "Percentile Low", format: twoDecimals, methods: ["percentile_clip"] },
  { parameter: "percentileHigh", label: "Percentile High", format: twoDecimals, methods: ["percentile_clip"] },
  { parameter: "minmaxLow", label: "Reject Low", format: String, methods: ["min_max"] },
  { parameter: "minmaxHigh", label: "Reject High", format: String, methods: ["min_max"] },
];

export default function RejectionControls({
  settings,
  onChange,
  frameCount,
  accent,
  label = "Rejection",
  fixed,
  disabled = false,
}: RejectionControlsProps) {
  const selectId = useId();
  const shown = boundRejectionSettings({ ...settings, ...fixed?.values });
  const hint = frameCount === null ? null : rejectionFrameHint(shown.rejection, frameCount, shown.minmaxLow, shown.minmaxHigh);
  const rows = PARAMETER_ROWS.filter((row) => row.methods.includes(shown.rejection));

  const changeParameter = (parameter: RejectionParameter, value: number) => {
    const change: Partial<RejectionSettings> = {};
    change[parameter] = boundRejectionValue(parameter, value);
    onChange(change);
  };

  return (
    <>
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={selectId} className="text-xs text-zinc-400">{label}</label>
        <select
          id={selectId}
          value={shown.rejection}
          onChange={(e) => onChange({ rejection: e.target.value as RejectionMethod })}
          disabled={disabled}
          className="ab-select"
        >
          {REJECTION_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
        </select>
      </div>
      {hint && (
        <p data-testid="rejection-hint" className={`text-[10px] leading-snug ${hint.severity === "warn" ? "text-amber-400/80" : "text-zinc-500"}`}>{hint.text}</p>
      )}
      {rows.map((row) => {
        const value = shown[row.parameter];
        if (fixed?.values[row.parameter] !== undefined) {
          return (
            <div key={row.parameter} data-testid="rejection-fixed" className="flex items-center justify-between">
              <span className="text-xs text-zinc-400">{row.label}</span>
              <span className="text-[11px] text-zinc-500">{`${row.format(value)}, ${fixed.note}`}</span>
            </div>
          );
        }
        const limit = REJECTION_LIMITS[row.parameter];
        return (
          <Slider
            key={row.parameter}
            label={row.label}
            value={value}
            min={limit.min}
            max={limit.max}
            step={limit.step}
            accent={accent}
            format={row.format}
            disabled={disabled}
            onChange={(v) => changeParameter(row.parameter, v)}
          />
        );
      })}
    </>
  );
}
